//! The browser glue of the write worker: its messages, its queue of
//! requests, and the files in the private file system (OPFS).
//!
//! The worker takes one request at a time, in the order sent, and ends
//! each with exactly one `write-done` (`stem_protocol::write`).  It writes
//! each file through a `FileSystemSyncAccessHandle`, which writes in place
//! and makes no swap file in Safari (TD-103).  It opens the handle, writes,
//! flushes and closes it for each request, also for each block of a file,
//! so it holds no handle between requests.  Each write gets a whole array
//! (MEM-5): a transferred array (a view is copied out first), or a slice of
//! a `Blob` that `arrayBuffer` gives.

use js_sys::Uint8Array;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use stem_protocol::write::{Body, Outcome, WriteReply, WriteRequest};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

#[wasm_bindgen(inline_js = r#"
async function folder(path, create, from) {
  let d = from || await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
// Safari 27 writes a typed array view's whole buffer, not just the view,
// so only whole arrays are written: a view is copied out first (MEM-5).
function whole(bytes) {
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? bytes : bytes.slice();
}
// The handle of the file `within/rel`, with the folders of `rel` made, as
// `st_write` of the page makes them; null when the folder `within` is gone.
async function file_of(within, rel) {
  let base;
  try { base = await folder(within, false); }
  catch (e) { if (within && e.name === "NotFoundError") return null; throw e; }
  const [d, n] = split("/" + rel);
  return await (await folder(d, true, base)).getFileHandle(n, { create: true });
}
// `write` returns the bytes written.  Old browsers return a promise, so
// it is awaited, as `truncate`, `flush` and `close` are.
async function put(h, bytes, at) {
  const n = await h.write(whole(bytes), { at });
  if (n !== bytes.byteLength) throw new Error(`wrote ${n} of ${bytes.byteLength} bytes`);
}
// Opens a sync access handle of `within/rel`, cuts the file to 0 bytes
// if `cut`, runs `body` with it, flushes and closes it, also on a failure.
async function with_handle(within, rel, cut, body) {
  try {
    const fh = await file_of(within, rel);
    if (!fh) return "gone";
    const h = await fh.createSyncAccessHandle();
    try {
      if (cut) await h.truncate(0);
      await body(h);
      await h.flush();
    } finally {
      await h.close();
    }
    return "written";
  } catch (e) {
    return (e && e.name) || "Error";
  }
}
// A `Blob` is read one slice at a time, so the worker holds at most one.
const SLICE = 8 << 20;
export async function write_whole(within, rel, body) {
  return with_handle(within, rel, true, async (h) => {
    if (!(body instanceof Blob)) return put(h, body, 0);
    for (let at = 0; at < body.size; at += SLICE) {
      const end = Math.min(at + SLICE, body.size);
      await put(h, new Uint8Array(await body.slice(at, end).arrayBuffer()), at);
    }
  });
}
export async function write_block(within, rel, at, bytes) {
  return with_handle(within, rel, at === 0, (h) => put(h, bytes, at));
}
export async function write_end(within, rel, size) {
  try {
    let base;
    try { base = await folder(within, false); }
    catch (e) { if (within && e.name === "NotFoundError") return "gone"; throw e; }
    const [d, n] = split("/" + rel);
    let dir;
    try { dir = await folder(d, false, base); }
    catch (e) { if (size === null && e.name === "NotFoundError") return "written"; throw e; }
    if (size === null) {
      try { await dir.removeEntry(n); }
      catch (e) { if (e.name !== "NotFoundError") throw e; }
      return "written";
    }
    const held = (await (await dir.getFileHandle(n)).getFile()).size;
    return held === size ? "written" : `the file holds ${held} of ${size} bytes`;
  } catch (e) {
    return (e && e.name) || "Error";
  }
}
"#)]
extern "C" {
    /// Writes the whole file `within/rel` from `body`, a `Blob` or a typed
    /// array: "written", "gone" when the folder `within` is gone, else the
    /// name of the error.
    async fn write_whole(within: &str, rel: &str, body: &JsValue) -> JsValue;
    /// Writes `bytes` at byte `at` of the file `within/rel`, which is cut
    /// to 0 bytes first when `at` is 0: as [`write_whole`] answers.
    async fn write_block(within: &str, rel: &str, at: f64, bytes: &Uint8Array) -> JsValue;
    /// Ends a block-written file `within/rel`: with a `size`, "written"
    /// if the file holds `size` bytes, else the error; with `null`, it
    /// removes the file (an abandoned write).  "gone" when the folder
    /// `within` is gone.
    async fn write_end(within: &str, rel: &str, size: &JsValue) -> JsValue;
}

thread_local! {
    /// The requests not yet taken, in the order sent.
    static QUEUE: RefCell<VecDeque<JsValue>> = const { RefCell::new(VecDeque::new()) };
    /// Whether a request runs now.
    static RUNNING: Cell<bool> = const { Cell::new(false) };
}

fn send(reply: WriteReply) {
    let (o, transfer) = reply.to_js();
    // Under test the page is the test runner, so the replies are kept.
    #[cfg(test)]
    {
        let _ = transfer;
        tests::SENT.with_borrow_mut(|sent| sent.push(o.into()));
    }
    #[cfg(not(test))]
    {
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        if let Err(e) = scope.post_message_with_transfer(&o, &transfer) {
            web_sys::console::error_2(&"write-worker: a message to the page was lost:".into(), &e);
        }
    }
}

/// Installs the message handler and tells the page that the worker
/// listens.
pub fn start() {
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| enqueue(e.data()));
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    scope.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
    send(WriteReply::Ready);
}

/// Queues the request `msg`, and starts the queue if it is not running.
/// One request at a time: a second sync access handle of a file that one
/// request holds would be refused.
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

/// The outcome of a write from the JS answer.
fn outcome(answer: &JsValue) -> Outcome {
    match answer.as_string().as_deref() {
        Some("written") => Outcome::Written,
        Some("gone") => Outcome::Gone,
        Some(name) => Outcome::Failed(name.to_string()),
        None => Outcome::Failed("Error".into()),
    }
}

/// Runs one request, and gives the reply that ends it.  A request that
/// cannot be read fails, so no request waits for ever.
async fn handle(msg: &JsValue) -> WriteReply {
    match WriteRequest::from_js(msg) {
        Err(bad) => WriteReply::Done {
            outcome: Outcome::Failed(bad.text),
            bytes: None,
        },
        Ok(WriteRequest::File { within, rel, body }) => {
            let body: JsValue = match body {
                Body::Blob(blob) => blob.into(),
                Body::Bytes(bytes) => bytes.into(),
            };
            WriteReply::Done {
                outcome: outcome(&write_whole(&within, &rel, &body).await),
                bytes: None,
            }
        }
        Ok(WriteRequest::Block {
            within,
            rel,
            at,
            bytes,
        }) => WriteReply::Done {
            outcome: outcome(&write_block(&within, &rel, at as f64, &bytes).await),
            bytes: Some(bytes),
        },
        Ok(WriteRequest::End { within, rel, size }) => {
            let size = size.map_or(JsValue::NULL, |n| JsValue::from(n as f64));
            WriteReply::Done {
                outcome: outcome(&write_end(&within, &rel, &size).await),
                bytes: None,
            }
        }
    }
}

#[cfg(test)]
mod tests;
