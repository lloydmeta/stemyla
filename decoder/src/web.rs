//! The browser glue of the decode worker: its messages, its queue of
//! requests, and the session files in the private file system (OPFS).
//!
//! The worker takes one request at a time, in the order sent, and ends
//! each with exactly one of `decoded`, `undecodable` and
//! `decode-unstored` (D6, D9).  It makes its working set ([`Buffers`])
//! once, when it starts (D12).
//!
//! It takes the requests `decode`, `overviews` and `classic`
//! (`stem_protocol::decode::WorkerRequest`).  It reads each source, a
//! kept `mix.f32` and each lane file with `getFile` and `FileReaderSync`,
//! so a running separation's reads see no lock (D7).  It
//! writes each part file through one `FileSystemSyncAccessHandle`, each
//! write a new whole `Uint8Array` (MEM-5), and closes each handle before
//! it replies.  The JS objects stay in tables on the JS side; Rust holds
//! their numbers (D7a).

use crate::pass::{self, Buffers, PartFile, Session, StoredFile};
use crate::source::{BlockSource, ReadFailure};
use engine::decode::MediaSource;
use js_sys::Uint8Array;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use stem_protocol::decode::{DecodeReply, WorkerRequest};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

#[wasm_bindgen(inline_js = r#"
const files = new Map();
const parts = new Map();
let next_id = 1;
async function folder(path, create) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
// The file at `path` as a `File`, kept in the table: [its number, its size].
export async function file_open(path) {
  const [d, n] = split(path);
  const f = await (await (await folder(d, false)).getFileHandle(n)).getFile();
  const id = next_id++;
  files.set(id, f);
  return [id, f.size];
}
// Bytes `start..end` of a kept file, read synchronously (workers only).
export function file_read(id, start, end) {
  const f = files.get(id);
  if (!f) throw new Error(`no open file ${id}`);
  return new Uint8Array(new FileReaderSync().readAsArrayBuffer(f.slice(start, end)));
}
export function files_forget() { files.clear(); }
// An empty file `rel` in the folder `session`, which must exist, so a
// forgotten session does not come back.  The folders in `rel` are made.
export async function part_open(session, rel) {
  const [d, n] = split(rel);
  let dir = await folder(session, false);
  for (const p of d.split("/").filter((s) => s)) dir = await dir.getDirectoryHandle(p, { create: true });
  const h = await (await dir.getFileHandle(n, { create: true })).createSyncAccessHandle();
  try { await h.truncate(0); } catch (e) { await h.close(); throw e; }
  const id = next_id++;
  parts.set(id, h);
  return id;
}
// `write` and `read` are synchronous on a sync access handle, so the
// decoder's sink can write without `await`.  A browser whose handle
// returns a promise is refused, not written to.
export function part_write(id, bytes, at) {
  const n = parts.get(id).write(bytes, { at });
  if (n !== bytes.byteLength) throw new Error(`wrote ${n} of ${bytes.byteLength} bytes`);
}
// The reads of whole blocks in the second pass share one array, so they
// make no garbage.  The decode worker copies each read into WebAssembly
// memory before the next.  A shorter read, at the end of a file, gets a
// new array.  Each read gets a whole array, never a view, as each write
// does (MEM-5).
let read_block = new Uint8Array(0);
export function part_read(id, at, len) {
  if (read_block.length < len) read_block = new Uint8Array(len);
  const b = read_block.length === len ? read_block : new Uint8Array(len);
  const n = parts.get(id).read(b, { at });
  if (n !== len) throw new Error(`read ${n} of ${len} bytes`);
  return b;
}
export async function part_close(id) {
  const h = parts.get(id);
  parts.delete(id);
  await h.flush();
  await h.close();
}
export async function remove_at(path) {
  try {
    const [d, n] = split(path);
    await (await folder(d, false)).removeEntry(n);
  } catch (_) {}
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn file_open(path: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    fn file_read(id: u32, start: f64, end: f64) -> Result<Uint8Array, JsValue>;
    fn files_forget();
    #[wasm_bindgen(catch)]
    async fn part_open(session: &str, rel: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    fn part_write(id: u32, bytes: &Uint8Array, at: f64) -> Result<(), JsValue>;
    #[wasm_bindgen(catch)]
    fn part_read(id: u32, at: f64, len: u32) -> Result<Uint8Array, JsValue>;
    #[wasm_bindgen(catch)]
    async fn part_close(id: u32) -> Result<(), JsValue>;
    async fn remove_at(path: &str);
}

thread_local! {
    /// The requests not yet taken, in the order sent.
    static QUEUE: RefCell<VecDeque<JsValue>> = const { RefCell::new(VecDeque::new()) };
    /// Whether a request runs now.
    static RUNNING: Cell<bool> = const { Cell::new(false) };
    /// The working set, made once (D12).  A request takes it while it
    /// runs.
    static BUFFERS: RefCell<Option<Buffers>> = const { RefCell::new(None) };
}

/// The text of a JS error: its name and message, such as
/// "NotFoundError: ...".
fn js_text(e: &JsValue) -> String {
    match e.dyn_ref::<js_sys::Error>() {
        Some(err) => format!("{}: {}", err.name(), err.message()),
        None => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    }
}

/// Megabytes of this worker's WebAssembly memory, which only ever grows.
fn wasm_mb() -> f64 {
    let mem: js_sys::WebAssembly::Memory = wasm_bindgen::memory().unchecked_into();
    mem.buffer()
        .unchecked_into::<js_sys::ArrayBuffer>()
        .byte_length() as f64
        / 1e6
}

fn send(reply: DecodeReply) {
    let (o, transfer) = reply.to_js();
    // Under test the page is the test runner, so messages are kept instead.
    #[cfg(test)]
    {
        let _ = transfer;
        tests::SENT.with_borrow_mut(|sent| sent.push(o.into()));
    }
    #[cfg(not(test))]
    {
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        if let Err(e) = scope.post_message_with_transfer(&o, &transfer) {
            web_sys::console::error_2(&"decode-worker: a message to the page was lost:".into(), &e);
        }
    }
}

fn log(text: String) {
    send(DecodeReply::Log(text));
}

/// Makes the working set, if it is not made yet.
fn warm_up() {
    BUFFERS.with_borrow_mut(|bufs| {
        bufs.get_or_insert_with(Buffers::default);
    });
}

/// Makes the working set, installs the message handler and tells the
/// page that the worker listens.
pub fn start() {
    console_error_panic_hook::set_once();
    warm_up();
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| enqueue(e.data()));
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    scope.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
    send(DecodeReply::Alive);
}

/// Queues the request `msg`, and starts the queue if it is not running.
fn enqueue(msg: JsValue) {
    QUEUE.with_borrow_mut(|queue| queue.push_back(msg));
    if !RUNNING.replace(true) {
        wasm_bindgen_futures::spawn_local(drain());
    }
}

/// Runs the queued requests one at a time, in the order sent.
async fn drain() {
    while let Some(msg) = QUEUE.with_borrow_mut(VecDeque::pop_front) {
        let reply = handle(&msg).await;
        send(reply);
    }
    RUNNING.set(false);
}

/// Runs one request, and gives the reply that ends it.  It logs the
/// worker's memory before it opens a file and before the reply.
async fn handle(msg: &JsValue) -> DecodeReply {
    let req = match WorkerRequest::from_js(msg) {
        Ok(req) => req,
        Err(bad) => {
            return DecodeReply::Unstored {
                text: format!("the decode request could not be read: {}", bad.text),
            };
        }
    };
    warm_up();
    let mut bufs = BUFFERS.with_borrow_mut(Option::take).unwrap_or_default();
    let kind = req.kind();
    log(format!(
        "decoder: memory: decoder wasm {:.0} MB at the {kind}'s start",
        wasm_mb()
    ));
    let reply = match &req {
        WorkerRequest::Decode(req) => {
            let mut opfs = Opfs {
                session: req.session.clone(),
            };
            pass::run(&mut opfs, req, &mut bufs).await
        }
        WorkerRequest::Overviews(req) => {
            let mut opfs = Opfs {
                session: req.session.clone(),
            };
            crate::lanes::overviews(&mut opfs, req, &mut bufs).await
        }
        WorkerRequest::Classic(req) => {
            let mut opfs = Opfs {
                session: req.session.clone(),
            };
            crate::lanes::classic(&mut opfs, req, &mut bufs).await
        }
    };
    files_forget();
    BUFFERS.with_borrow_mut(|slot| *slot = Some(bufs));
    log(format!(
        "decoder: memory: decoder wasm {:.0} MB at the {kind}'s end",
        wasm_mb()
    ));
    reply
}

/// The files of one session in OPFS.
struct Opfs {
    session: String,
}

/// A part file: the number of its sync access handle.
struct OpfsPart(u32);

/// A stored file: the number of its `File`, and its size.
struct OpfsStored {
    id: u32,
    size: u64,
}

impl PartFile for OpfsPart {
    fn write_at(&mut self, at: u64, bytes: &[u8]) -> Result<(), String> {
        // A new array of its own, never a view (MEM-5).
        let whole = Uint8Array::new_with_length(bytes.len() as u32);
        whole.copy_from(bytes);
        part_write(self.0, &whole, at as f64).map_err(|e| js_text(&e))
    }

    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> Result<(), String> {
        let bytes = part_read(self.0, at as f64, buf.len() as u32).map_err(|e| js_text(&e))?;
        bytes.copy_to(buf);
        Ok(())
    }
}

/// Fills `buf` with the bytes of kept file `id` from byte `at`.
fn read_file(id: u32, at: u64, buf: &mut [u8]) -> Result<(), String> {
    let end = at + buf.len() as u64;
    let bytes = file_read(id, at as f64, end as f64).map_err(|e| js_text(&e))?;
    if bytes.length() as usize != buf.len() {
        return Err(format!("gave {} of {} bytes", bytes.length(), buf.len()));
    }
    bytes.copy_to(buf);
    Ok(())
}

impl StoredFile for OpfsStored {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> Result<(), String> {
        read_file(self.id, at, buf)
    }
}

/// Opens the file at `path`: its number and its size.
async fn open_file(path: &str) -> Result<(u32, u64), String> {
    let opened = file_open(path)
        .await
        .map_err(|e| format!("{path} could not be opened: {}", js_text(&e)))?;
    let pair: js_sys::Array = opened.unchecked_into();
    let number = |i| pair.get(i).as_f64().unwrap_or(0.0);
    Ok((number(0) as u32, number(1) as u64))
}

impl Session for Opfs {
    type Part = OpfsPart;
    type Stored = OpfsStored;

    async fn source(
        &mut self,
        path: &str,
        failure: ReadFailure,
    ) -> Result<Box<dyn MediaSource>, String> {
        let (id, size) = open_file(path).await?;
        #[cfg(test)]
        tests::after_source_open(path).await;
        let read = move |at: u64, buf: &mut [u8]| read_file(id, at, buf);
        Ok(Box::new(BlockSource::new(size, read, failure)))
    }

    async fn stored(&mut self, path: &str) -> Result<OpfsStored, String> {
        let (id, size) = open_file(path).await?;
        Ok(OpfsStored { id, size })
    }

    async fn create(&mut self, path: &str) -> Result<OpfsPart, String> {
        let rel = path
            .strip_prefix(&self.session)
            .and_then(|rest| rest.strip_prefix('/'))
            .ok_or_else(|| format!("{path} is not in the session {}", self.session))?;
        let id = part_open(&self.session, rel)
            .await
            .map_err(|e| format!("{path} could not be made: {}", js_text(&e)))?;
        Ok(OpfsPart(id.as_f64().unwrap_or(0.0) as u32))
    }

    async fn close(&mut self, part: OpfsPart) -> Result<(), String> {
        part_close(part.0).await.map_err(|e| js_text(&e))
    }

    async fn remove(&mut self, path: &str) {
        remove_at(path).await;
    }
}

#[cfg(test)]
mod tests;
