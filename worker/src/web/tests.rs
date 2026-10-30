//! Browser tests of the worker's storage and message handling, run in a
//! dedicated worker (`cargo run -p xtask -- web-test`).  Only
//! `a_read_back_is_copied_into_wasm_memory_once` and
//! `an_f16_variant_the_browser_rejects_keeps_f32_weights` open a GPU.
//! Every other case either fails before the GPU opens or never reaches it.
//!
//! The tests share one worker and its origin's private file system, so
//! each starts from an empty one, and they run one at a time.

#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use super::*;
use encode::{FLAC_BLOCK, MP3_BLOCK};
use js_sys::{Object, Reflect};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use stem_protocol::{Encode, Encoded};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

thread_local! {
    /// Every message the worker would have posted to the page.
    pub(super) static SENT: RefCell<Vec<JsValue>> = const { RefCell::new(Vec::new()) };
    /// Heap bytes allocated and not yet freed.
    static LIVE: Cell<usize> = const { Cell::new(0) };
    /// The most `LIVE` has been since a test set it.
    static PEAK: Cell<usize> = const { Cell::new(0) };
}

/// Keeps `LIVE` and `PEAK` for every test in this binary.  Only `alloc`
/// and `dealloc` are written, so the default `realloc` holds the old and
/// the new buffer at once, as `System`'s may.
struct PeakAllocator;

// SAFETY: every call is forwarded unchanged to `System`; counting touches
// only `const` thread-locals, which do not allocate.
unsafe impl GlobalAlloc for PeakAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = LIVE.try_with(|live| {
            let now = live.get() + layout.size();
            live.set(now);
            let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
        });
        // SAFETY: the caller's contract is the same as `System`'s.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(layout.size())));
        // SAFETY: the caller's contract is the same as `System`'s.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: PeakAllocator = PeakAllocator;

#[wasm_bindgen(inline_js = r#"
export async function wipe() {
  const root = await navigator.storage.getDirectory();
  for await (const name of root.keys()) await root.removeEntry(name, { recursive: true });
}
export async function plant(name, bytes) {
  const root = await navigator.storage.getDirectory();
  const w = await (await root.getFileHandle(name, { create: true })).createWritable();
  await w.write(bytes);
  await w.close();
}
export async function files() {
  const root = await navigator.storage.getDirectory();
  const out = [];
  for await (const [name, h] of root.entries()) if (h.kind === "file") out.push(name);
  return out.sort().join(" ");
}
export async function plant_dir(name) {
  const root = await navigator.storage.getDirectory();
  await root.getDirectoryHandle(name, { create: true });
}
export function blob_url(bytes) { return URL.createObjectURL(new Blob([bytes])); }
export function blob(bytes) { return new Blob([bytes]); }
export function revoke(url) { URL.revokeObjectURL(url); }
// Makes WebGPU look absent (as on a device without it) until the
// returned function is called.
export function hide_gpu() {
  const proto = Object.getPrototypeOf(navigator);
  const own = Object.getOwnPropertyDescriptor(proto, "gpu");
  Object.defineProperty(proto, "gpu", { get: () => undefined, configurable: true });
  return () => {
    if (own) Object.defineProperty(proto, "gpu", own);
    else delete proto.gpu;
  };
}
// The next fetch answers with "lloyd" and "acme" as views into larger
// buffers, as a stream's chunks may be.
export function next_fetch_sends_views() {
  const real = globalThis.fetch;
  globalThis.fetch = async () => {
    globalThis.fetch = real;
    const view = (text) => {
      const b = new Uint8Array(64);
      const t = new TextEncoder().encode(text);
      b.set(t, 8);
      return b.subarray(8, 8 + t.length);
    };
    const body = new ReadableStream({
      start(c) { c.enqueue(view("lloyd")); c.enqueue(view("acme")); c.close(); },
    });
    return new Response(body);
  };
}
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
// Records what each OPFS write gets, in call order: "blob", "whole" (a
// typed array that is its whole buffer) or "view".  A simpler copy of
// the spy in app/src/store.rs.
let record = null;
let real_write = null;
export function start_spy() {
  if (real_write) throw new Error("a spy is already on write");
  const proto = FileSystemWritableFileStream.prototype;
  real_write = proto.write;
  record = [];
  proto.write = function (data) {
    if (data instanceof Blob) record.push("blob");
    else if (ArrayBuffer.isView(data))
      record.push(data.byteOffset === 0 && data.byteLength === data.buffer.byteLength ? "whole" : "view");
    else record.push(typeof data);
    return real_write.call(this, data);
  };
}
// Restores `write` and returns the record, joined by spaces.
export function stop_spy() {
  FileSystemWritableFileStream.prototype.write = real_write;
  real_write = null;
  return record.join(" ");
}
async function folder_at(path, create) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split_at(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
export async function plant_folder(path) { await folder_at(path, true); }
// Removes the file at `path`: whether it is gone.
export async function remove_at(path) {
  try {
    const [d, n] = split_at(path);
    await (await folder_at(d, false)).removeEntry(n);
    return true;
  } catch (e) { return e.name === "NotFoundError"; }
}
export async function plant_at(path, bytes) {
  const [d, n] = split_at(path);
  const w = await (await (await folder_at(d, true)).getFileHandle(n, { create: true })).createWritable();
  await w.write(bytes);
  await w.close();
}
// The bytes of the file at `path`, or null if there is none.
export async function read_at(path) {
  try {
    const [d, n] = split_at(path);
    const f = await (await (await folder_at(d, false)).getFileHandle(n)).getFile();
    return new Uint8Array(await f.arrayBuffer());
  } catch (e) { if (e.name === "NotFoundError") return null; throw e; }
}
// The names of the entries in the folder `path`, sorted and joined by
// spaces, or null if there is no such folder.
export async function names_in(path) {
  try {
    const out = [];
    for await (const name of (await folder_at(path, false)).keys()) out.push(name);
    return out.sort().join(" ");
  } catch (e) { if (e.name === "NotFoundError") return null; throw e; }
}
// Records what each FileSystemSyncAccessHandle.write gets: "whole" or
// "view".  Call `fail_at` (from 1; 0 for none) is refused, as a full
// disk refuses it.
let sync_record = null;
let sync_real = null;
export function start_sync_spy(fail_at) {
  if (sync_real) throw new Error("a spy is already on the sync write");
  const proto = FileSystemSyncAccessHandle.prototype;
  sync_real = proto.write;
  sync_record = [];
  proto.write = function (data, options) {
    if (sync_record.length + 1 === fail_at) {
      sync_record.push("refused");
      throw new DOMException("refused by the test", "QuotaExceededError");
    }
    sync_record.push(ArrayBuffer.isView(data) && data.byteOffset === 0 && data.byteLength === data.buffer.byteLength ? "whole" : "view");
    return sync_real.call(this, data, options);
  };
}
// The next `createSyncAccessHandle` call `n` (from 1) is refused, as when
// another writer holds the file; the returned function restores it.
export function refuse_sync_handle(n) {
  const proto = FileSystemFileHandle.prototype;
  const real = proto.createSyncAccessHandle;
  let calls = 0;
  proto.createSyncAccessHandle = function () {
    calls++;
    if (calls === n) return Promise.reject(new DOMException("refused by the test", "NoModificationAllowedError"));
    return real.call(this);
  };
  return () => { proto.createSyncAccessHandle = real; };
}
export function stop_sync_spy() {
  FileSystemSyncAccessHandle.prototype.write = sync_real;
  sync_real = null;
  return sync_record.join(" ");
}
// Records the bytes of each blob that FileReaderSync reads as an
// ArrayBuffer, in call order.
let read_record = null;
let read_real = null;
export function start_read_spy() {
  if (read_real) throw new Error("a spy is already on readAsArrayBuffer");
  const proto = FileReaderSync.prototype;
  read_real = proto.readAsArrayBuffer;
  read_record = [];
  proto.readAsArrayBuffer = function (blob) {
    read_record.push(blob.size);
    return read_real.call(this, blob);
  };
}
// Restores readAsArrayBuffer and returns the bytes of each read.
export function stop_read_spy() {
  FileReaderSync.prototype.readAsArrayBuffer = read_real;
  read_real = null;
  return read_record;
}
// Records, in one sequence, each FileReaderSync read ("r<bytes>") and
// each FileSystemSyncAccessHandle write ("w<bytes>@<at>", with " view"
// for a view).
let order_record = null;
let order_read = null;
let order_write = null;
export function start_order_spy() {
  if (order_read || order_write) throw new Error("a spy is already on the reads or writes");
  const rp = FileReaderSync.prototype;
  const wp = FileSystemSyncAccessHandle.prototype;
  order_read = rp.readAsArrayBuffer;
  order_write = wp.write;
  order_record = [];
  rp.readAsArrayBuffer = function (blob) {
    order_record.push(`r${blob.size}`);
    return order_read.call(this, blob);
  };
  wp.write = function (data, options) {
    const whole = ArrayBuffer.isView(data) && data.byteOffset === 0 && data.byteLength === data.buffer.byteLength;
    order_record.push(`w${data.byteLength}@${options && options.at}${whole ? "" : " view"}`);
    return order_write.call(this, data, options);
  };
}
// Restores both and returns the record, joined by spaces.
export function stop_order_spy() {
  FileReaderSync.prototype.readAsArrayBuffer = order_read;
  FileSystemSyncAccessHandle.prototype.write = order_write;
  order_read = null;
  order_write = null;
  return order_record.join(" ");
}
// The next fetch answers with one byte, and keeps how it was asked to
// treat redirects for `last_redirect`.
let redirect = null;
export function record_next_fetch() {
  const real = globalThis.fetch;
  globalThis.fetch = async (_url, init) => {
    globalThis.fetch = real;
    redirect = init.redirect;
    return new Response(new Uint8Array(1));
  };
}
export function last_redirect() { return redirect; }
// The next fetch answers with 10 of 1000 bytes, then the connection breaks.
export function break_next_fetch() {
  const real = globalThis.fetch;
  globalThis.fetch = async () => {
    globalThis.fetch = real;
    const body = new ReadableStream({
      start(c) { c.enqueue(new Uint8Array(10)); c.error(new TypeError("network error")); },
    });
    return new Response(body, { headers: { "content-length": "1000" } });
  };
}
"#)]
extern "C" {
    async fn wipe();
    #[wasm_bindgen(js_name = plant)]
    async fn plant_js(name: &str, bytes: &Uint8Array);
    /// Names of the files at the root, sorted, space-separated.
    async fn files() -> JsValue;
    /// Makes a directory at the root, where a file of that name cannot be
    /// written.
    async fn plant_dir(name: &str);
    fn blob_url(bytes: &[u8]) -> String;
    /// A `Blob` holding a copy of `bytes`, as a picked file is.
    fn blob(bytes: &[u8]) -> Blob;
    fn revoke(url: &str);
    async fn sleep(ms: u32);
    fn next_fetch_sends_views();
    fn break_next_fetch();
    fn record_next_fetch();
    fn last_redirect() -> Option<String>;
    fn hide_gpu() -> js_sys::Function;
    fn start_spy();
    fn stop_spy() -> String;
    async fn plant_folder(path: &str);
    #[wasm_bindgen(js_name = remove_at)]
    async fn remove_at_js(path: &str) -> JsValue;
    #[wasm_bindgen(js_name = plant_at)]
    async fn plant_at_js(path: &str, bytes: &Uint8Array);
    #[wasm_bindgen(js_name = read_at)]
    async fn read_at_js(path: &str) -> JsValue;
    #[wasm_bindgen(js_name = names_in)]
    async fn names_in_js(path: &str) -> JsValue;
    fn start_sync_spy(fail_at: u32);
    fn stop_sync_spy() -> String;
    fn refuse_sync_handle(n: u32) -> js_sys::Function;
    fn start_read_spy();
    fn stop_read_spy() -> Vec<f64>;
    fn start_order_spy();
    fn stop_order_spy() -> String;
}

/// Writes a file at `path`, with its folders.
async fn plant_at(path: &str, bytes: &[u8]) {
    plant_at_js(path, &Uint8Array::from(bytes)).await;
}

/// Whether the file at `path` is gone after its removal.
async fn opfs_remove_at(path: &str) -> bool {
    remove_at_js(path).await.as_bool() == Some(true)
}

/// The bytes of the file at `path`, or `None` if there is none.
async fn read_at(path: &str) -> Option<Vec<u8>> {
    let got = read_at_js(path).await;
    (!got.is_null()).then(|| got.unchecked_into::<Uint8Array>().to_vec())
}

/// The entries of the folder `path`, sorted and joined by spaces, or
/// `None` if there is no such folder.
async fn names_in(path: &str) -> Option<String> {
    names_in_js(path).await.as_string()
}

/// Writes a file at the root (from a copy: OPFS reads the bytes after
/// this returns, and a view of wasm memory may be gone by then).
async fn plant(name: &str, bytes: &[u8]) {
    plant_js(name, &Uint8Array::from(bytes)).await;
}

/// Waits until `done` holds, or 10 s (the assertions after it then fail).
async fn until(done: impl Fn() -> bool) {
    for _ in 0..1000 {
        if done() {
            return;
        }
        sleep(10).await;
    }
}

async fn fresh() {
    wipe().await;
    SENT.with_borrow_mut(Vec::clear);
}

async fn listing() -> String {
    files().await.as_string().unwrap_or_default()
}

fn field(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

/// The messages of type `kind` posted since the last [`fresh`].
fn sent(kind: &str) -> Vec<JsValue> {
    SENT.with_borrow(|s| {
        s.iter()
            .filter(|m| field(m, "type").as_string().as_deref() == Some(kind))
            .cloned()
            .collect()
    })
}

/// The one `model` reply, as (ok, loaded, stored, text).
fn model_reply() -> (bool, bool, bool, String) {
    let replies = sent("model");
    assert_eq!(replies.len(), 1, "one model reply");
    let m = &replies[0];
    (
        field(m, "ok").as_bool().expect("ok"),
        field(m, "loaded").as_bool().expect("loaded"),
        field(m, "stored").as_bool().expect("stored"),
        field(m, "text").as_string().expect("text"),
    )
}

/// Whether no test has opened a GPU (none of these cases needs one).
fn no_gpu() -> bool {
    STATE.with_borrow(|s| s.gpu.is_none())
}

fn message(fields: &[(&str, JsValue)]) -> JsValue {
    let o = Object::new();
    for (k, v) in fields {
        Reflect::set(&o, &JsValue::from_str(k), v).expect("set");
    }
    o.into()
}

fn of_type(kind: &str) -> JsValue {
    message(&[("type", kind.into())])
}

/// A safetensors header that claims 16 bytes of JSON and has 1.
const TRUNCATED: [u8; 9] = [16, 0, 0, 0, 0, 0, 0, 0, b'{'];

#[wasm_bindgen_test]
async fn turns_are_served_in_the_order_asked() {
    let order = Rc::new(RefCell::new(Vec::new()));
    let first = Held::turn().await;
    for n in [2, 3] {
        let order = order.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _turn = Held::turn().await;
            order.borrow_mut().push(format!("{n} starts"));
            sleep(30).await;
            order.borrow_mut().push(format!("{n} ends"));
        });
    }
    sleep(50).await;
    assert!(
        order.borrow().is_empty(),
        "nothing runs while turn 1 is held"
    );
    drop(first);
    until(|| order.borrow().len() == 4).await;
    assert_eq!(
        *order.borrow(),
        ["2 starts", "2 ends", "3 starts", "3 ends"],
        "each turn waits for the one before it to end"
    );
}

#[wasm_bindgen_test]
async fn model_and_gpu_messages_wait_for_their_turn() {
    fresh().await;
    let held = Held::turn().await;
    wasm_bindgen_futures::spawn_local(handle(of_type("init")));
    wasm_bindgen_futures::spawn_local(handle(of_type("separate")));
    sleep(200).await;
    assert!(
        sent("model").is_empty() && sent("error").is_empty(),
        "nothing is answered while another message has the turn"
    );
    drop(held);
    until(|| !sent("model").is_empty() && !sent("error").is_empty()).await;
    assert_eq!(sent("model").len(), 1, "init answered");
    assert_eq!(sent("error").len(), 1, "separate answered");
    assert!(
        sent("status").is_empty(),
        "no other tab is waited for: {:?}",
        sent("status")
    );
}

#[wasm_bindgen_test]
async fn cache_get_tells_a_missing_file_from_a_present_one() {
    fresh().await;
    assert!(cache_get("model").await.expect("no error").is_null());
    plant("model", &[1, 2, 3]).await;
    let got = cache_get("model").await.expect("no error");
    assert_eq!(got.unchecked_into::<Uint8Array>().to_vec(), [1, 2, 3]);
}

#[wasm_bindgen_test]
async fn commit_replaces_the_destination_and_removes_the_source() {
    fresh().await;
    plant("model", &[9]).await;
    plant("model.part", &[1, 2, 3]).await;
    assert_eq!(
        opfs_commit("model.part", "model").await.as_bool(),
        Some(true)
    );
    assert_eq!(listing().await, "model");
    let got = cache_get("model").await.expect("no error");
    assert_eq!(got.unchecked_into::<Uint8Array>().to_vec(), [1, 2, 3]);
}

#[wasm_bindgen_test]
async fn commit_of_a_missing_file_fails_and_keeps_the_destination() {
    fresh().await;
    plant("model", &[9]).await;
    assert_eq!(
        opfs_commit("model.part", "model").await.as_bool(),
        Some(false)
    );
    assert_eq!(listing().await, "model");
}

#[wasm_bindgen_test]
async fn removing_a_missing_file_counts_as_removed() {
    fresh().await;
    assert_eq!(opfs_remove("nothing").await.as_bool(), Some(true));
    plant("something", &[1]).await;
    assert_eq!(opfs_remove("something").await.as_bool(), Some(true));
    assert_eq!(listing().await, "");
}

#[wasm_bindgen_test]
async fn cache_put_leaves_only_the_finished_file() {
    fresh().await;
    plant(CACHE_NAME, &[9]).await;
    let bytes = Uint8Array::from(&[4u8, 5, 6][..]);
    assert_eq!(cache_put(CACHE_NAME, &bytes).await.as_bool(), Some(true));
    assert_eq!(listing().await, CACHE_NAME);
    let got = cache_get(CACHE_NAME).await.expect("no error");
    assert_eq!(got.unchecked_into::<Uint8Array>().to_vec(), [4, 5, 6]);
}

#[wasm_bindgen_test]
async fn cache_put_stores_a_blob() {
    fresh().await;
    assert_eq!(
        cache_put(CACHE_NAME, &blob(&[4, 5, 6])).await.as_bool(),
        Some(true)
    );
    assert_eq!(listing().await, CACHE_NAME);
    assert_eq!(stored_bytes().await, [4, 5, 6]);
}

/// Every range of `body`, read through `file`, in an order that crosses
/// block boundaries, comes back to blocks already evicted, and ends in the
/// last, partial block.
fn reads_back(file: &ModelFile, body: &[u8]) {
    let n = body.len();
    assert_eq!(file.size(), n);
    for (s, e) in [
        (0, 8),
        (8, 8),
        (13, n / 2 + 1),
        (99, 101),
        (250, 1000),
        (0, 1),
        (n - 1, n),
        (40, n),
    ] {
        assert_eq!(file.read(s, e).expect("read")[..], body[s..e], "{s}..{e}");
        if let ModelFile::Blob(f) = file {
            assert!(f.held() <= f.keep, "{} blocks held of {}", f.held(), f.keep);
        }
    }
}

#[wasm_bindgen_test]
async fn a_stored_file_is_read_in_blocks() {
    fresh().await;
    let body: Vec<u8> = (0..2_000u32).map(|i| (i * 7) as u8).collect();
    plant(CACHE_NAME, &body).await;
    assert!(cache_file("missing").await.expect("no error").is_null());
    let stored = || async {
        cache_file(CACHE_NAME)
            .await
            .expect("no error")
            .unchecked_into()
    };
    reads_back(&ModelFile::blob(stored().await), &body);
    reads_back(&ModelFile::blob_in_blocks(stored().await, 100, 2), &body);
    reads_back(&ModelFile::blob_in_blocks(stored().await, 7, 1), &body);
    reads_back(&ModelFile::Bytes(Uint8Array::from(&body[..])), &body);
}

/// Reset removing the file mid-load, say.  (A file rewritten after it was
/// opened is a read error in Chrome, but Safari 27 reads the new bytes, so
/// the hashed download is built from the bytes it hashed.)
#[wasm_bindgen_test]
async fn a_stored_file_removed_after_it_was_opened_is_a_read_error() {
    fresh().await;
    plant(CACHE_NAME, &TRUNCATED).await;
    let file = ModelFile::blob(
        cache_file(CACHE_NAME)
            .await
            .expect("no error")
            .unchecked_into(),
    );
    opfs_remove(CACHE_NAME).await;
    let got = Store::from_source(&file).err().expect("not readable");
    assert!(
        matches!(LoadError::from(got), LoadError::Read(_)),
        "a read error, so a stored model is kept, not removed as unusable"
    );
}

#[wasm_bindgen_test]
/// Refused from its header, before it is copied into storage: a stale
/// partial file is left as it was, not overwritten by a copy.
async fn a_picked_file_that_is_not_a_model_is_refused_before_it_is_copied() {
    fresh().await;
    plant(CACHE_NAME, &[7]).await;
    plant(DOWNLOAD_NAME, &[9]).await;
    handle(message(&[
        ("type", "model".into()),
        ("file", blob(&TRUNCATED).into()),
    ]))
    .await;
    let (ok, _, stored, text) = model_reply();
    assert!(!ok && stored, "{text}");
    assert!(no_gpu(), "told unusable before the GPU opened");
    assert!(text.starts_with("not a BS-RoFormer SW model"), "{text}");
    assert_eq!(stored_bytes().await, [7]);
    let stale = cache_get(DOWNLOAD_NAME).await.expect("no error");
    assert_eq!(
        stale.unchecked_into::<Uint8Array>().to_vec(),
        [9],
        "not copied"
    );
}

/// A well-formed safetensors file with no tensors: it gets as far as the
/// GPU.
const EMPTY_MODEL: [u8; 10] = [2, 0, 0, 0, 0, 0, 0, 0, b'{', b'}'];

#[wasm_bindgen_test]
async fn a_picked_file_the_gpu_failed_to_load_is_not_stored() {
    fresh().await;
    plant(CACHE_NAME, &[7]).await;
    let show_gpu = hide_gpu();
    handle(message(&[
        ("type", "model".into()),
        ("file", blob(&EMPTY_MODEL).into()),
    ]))
    .await;
    show_gpu.call0(&JsValue::NULL).expect("restored");
    let (ok, loaded, stored, text) = model_reply();
    assert!(!ok && !loaded && stored, "{text}");
    assert!(text.starts_with("the model could not be loaded"), "{text}");
    assert_eq!(listing().await, CACHE_NAME, "no partial file left");
    assert_eq!(stored_bytes().await, [7]);
}

/// Chunks that are views of larger buffers, which Safari 27 would write
/// whole (see `whole` in the worker).  Chrome writes just the view, so the
/// read-back fails only in Safari without that copy; the spy on `write`
/// fails in any browser (MEM-5).
#[wasm_bindgen_test]
async fn a_download_holds_only_the_bytes_of_its_chunks() {
    fresh().await;
    next_fetch_sends_views();
    let progress = Closure::<dyn FnMut(f64, f64)>::new(|_: f64, _: f64| {});
    start_spy();
    let got = download_to_opfs(
        "/weights/m",
        DOWNLOAD_NAME,
        progress.as_ref().unchecked_ref(),
        false,
    )
    .await;
    assert_eq!(stop_spy(), "whole whole");
    assert_eq!(got.ok().and_then(|n| n.as_f64()), Some(9.0));
    let stored = cache_get(DOWNLOAD_NAME).await.expect("no error");
    assert_eq!(stored.unchecked_into::<Uint8Array>().to_vec(), b"lloydacme");
}

#[wasm_bindgen_test]
async fn opfs_write_writes_more_than_one_slice() {
    fresh().await;
    let body: Vec<u8> = (0..(9u32 << 20)).map(|i| (i % 251) as u8).collect();
    for (what, src) in [
        ("a blob", blob(&body).into()),
        ("bytes", JsValue::from(Uint8Array::from(&body[..]))),
    ] {
        start_spy();
        let written = opfs_write("model", &src).await;
        let record = stop_spy();
        assert_eq!(written.as_bool(), Some(true));
        let whole_slices = if what == "a blob" {
            "blob blob"
        } else {
            "whole whole"
        };
        assert_eq!(record, whole_slices, "{what}");
        let got = cache_get("model")
            .await
            .expect("no error")
            .unchecked_into::<Uint8Array>()
            .to_vec();
        let first_wrong = got.iter().zip(&body).position(|(a, b)| a != b);
        assert!(
            got.len() == body.len() && first_wrong.is_none(),
            "{what}: {} bytes of {}, first wrong at {first_wrong:?}",
            got.len(),
            body.len()
        );
    }
}

#[wasm_bindgen_test]
fn a_file_over_4_gb_is_refused() {
    assert!(reachable(2_000.0).is_ok());
    assert!(reachable(u32::MAX as f64).is_ok());
    assert!(matches!(
        reachable(5e9),
        Err(WeightError::Parse(e)) if e.contains("over the 4 GB")
    ));
}

async fn sha256(bytes: &[u8]) -> String {
    sha256_hex(&Uint8Array::from(bytes))
        .await
        .expect("hashed")
        .as_string()
        .expect("hex")
}

#[wasm_bindgen_test]
async fn a_download_is_used_only_if_its_hash_matches() {
    fresh().await;
    let body = b"the model".to_vec();
    plant(DOWNLOAD_NAME, &body).await;
    let hash = sha256(&body).await;
    let checked = verified_download(Some(&hash)).await.expect("matches");
    assert!(
        matches!(checked, ModelFile::Bytes(_)),
        "built from what was hashed"
    );
    // Rewritten after the check (Safari reads the new bytes through a
    // `File` opened before): what was hashed is still what is read.
    plant(DOWNLOAD_NAME, b"not the model").await;
    assert_eq!((&checked).read(0, body.len()).expect("read")[..], body[..]);

    let wrong = verified_download(Some(&sha256(b"another").await))
        .await
        .err()
        .expect("refused");
    assert!(
        wrong.starts_with("the downloaded model is not the expected file"),
        "{wrong}"
    );
    let unchecked = verified_download(None).await.expect("not checked");
    assert!(matches!(unchecked, ModelFile::Blob(_)));
}

/// The same-site check on `?model_url=` covers the address asked for, so an
/// unchecked download must not be redirected off the site.
#[wasm_bindgen_test]
async fn only_a_hash_checked_download_follows_redirects() {
    for (expected, redirect) in [(Some("0".repeat(64)), "follow"), (None, "error")] {
        fresh().await;
        record_next_fetch();
        download_from("/weights/m", expected.as_deref(), &Settings::default())
            .await
            .expect("answered");
        assert_eq!(last_redirect().as_deref(), Some(redirect), "{expected:?}");
    }
}

#[wasm_bindgen_test]
async fn a_checked_download_the_gpu_failed_to_load_is_kept() {
    for (checked, kept) in [(true, true), (false, false)] {
        fresh().await;
        let url = blob_url(&EMPTY_MODEL);
        let hash = sha256(&EMPTY_MODEL).await;
        let show_gpu = hide_gpu();
        download_from(&url, checked.then_some(hash.as_str()), &Settings::default())
            .await
            .expect("answered");
        show_gpu.call0(&JsValue::NULL).expect("restored");
        revoke(&url);
        let (ok, loaded, stored, text) = model_reply();
        assert!(!ok && !loaded, "{text}");
        assert!(
            text.starts_with("the downloaded model could not be loaded"),
            "{text}"
        );
        assert_eq!(stored, kept, "checked {checked}: {text}");
        assert_eq!(
            text.contains("it was saved in browser storage"),
            kept,
            "{text}"
        );
        let left = if kept { CACHE_NAME } else { "" };
        assert_eq!(listing().await, left, "checked {checked}");
    }
}

#[wasm_bindgen_test]
async fn download_writes_the_whole_body_and_reports_progress() {
    fresh().await;
    let body: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
    let url = blob_url(&body);
    let last = Rc::new(RefCell::new((0.0, 0.0)));
    let seen = last.clone();
    let progress = Closure::<dyn FnMut(f64, f64)>::new(move |done: f64, total: f64| {
        *seen.borrow_mut() = (done, total);
    });
    let got = download_to_opfs(&url, DOWNLOAD_NAME, progress.as_ref().unchecked_ref(), true).await;
    revoke(&url);
    assert_eq!(got.expect("downloaded").as_f64(), Some(100_000.0));
    assert_eq!(*last.borrow(), (100_000.0, 100_000.0), "last progress");
    let stored = cache_get(DOWNLOAD_NAME).await.expect("no error");
    assert_eq!(stored.unchecked_into::<Uint8Array>().to_vec(), body);
}

#[wasm_bindgen_test]
async fn a_failed_fetch_creates_no_file() {
    fresh().await;
    let url = blob_url(&[1, 2, 3]);
    revoke(&url);
    let progress = Closure::<dyn FnMut(f64, f64)>::new(|_: f64, _: f64| {});
    let got = download_to_opfs(&url, DOWNLOAD_NAME, progress.as_ref().unchecked_ref(), true).await;
    assert!(got.is_err(), "a revoked URL cannot be fetched");
    assert_eq!(listing().await, "");
}

#[wasm_bindgen_test]
async fn init_without_a_stored_model_says_so() {
    fresh().await;
    handle(of_type("init")).await;
    let (ok, loaded, stored, text) = model_reply();
    assert!(!ok && !loaded && !stored);
    assert_eq!(text, "no model cached in this browser");
}

#[wasm_bindgen_test]
async fn init_removes_an_empty_stored_model() {
    fresh().await;
    plant(CACHE_NAME, &[]).await;
    handle(of_type("init")).await;
    let (ok, loaded, stored, text) = model_reply();
    assert!(!ok && !loaded && !stored, "{text}");
    assert!(no_gpu(), "told unusable before the GPU opened");
    assert!(
        text.starts_with("the model in browser storage is unusable (0 bytes"),
        "{text}"
    );
    assert!(
        text.ends_with("has been removed; download it again"),
        "{text}"
    );
    assert_eq!(listing().await, "");
}

#[wasm_bindgen_test]
async fn init_removes_a_truncated_stored_model() {
    fresh().await;
    plant(CACHE_NAME, &TRUNCATED).await;
    handle(of_type("init")).await;
    let (ok, _, stored, text) = model_reply();
    assert!(!ok && !stored, "{text}");
    assert!(text.contains("(9 bytes, not a safetensors file"), "{text}");
    assert_eq!(listing().await, "");
}

#[wasm_bindgen_test]
async fn init_removes_partial_files_left_by_an_interrupted_save() {
    fresh().await;
    plant(DOWNLOAD_NAME, &[1, 2, 3]).await;
    plant(&format!("{CACHE_NAME}.part"), &[1, 2, 3]).await;
    handle(of_type("init")).await;
    assert_eq!(listing().await, "");
}

/// The stored model's bytes, which these tests plant as `[7]`.
async fn stored_bytes() -> Vec<u8> {
    let got = cache_get(CACHE_NAME).await.expect("no error");
    assert!(!got.is_null(), "the stored model is still there");
    got.unchecked_into::<Uint8Array>().to_vec()
}

#[wasm_bindgen_test]
async fn a_picked_file_that_is_not_a_model_leaves_the_stored_model_alone() {
    fresh().await;
    plant(CACHE_NAME, &[7]).await;
    let bytes = Uint8Array::from(&[1u8, 2, 3][..]);
    handle(message(&[
        ("type", "model".into()),
        ("bytes", bytes.into()),
    ]))
    .await;
    let (ok, loaded, stored, text) = model_reply();
    assert!(!ok && !loaded && stored, "{text}");
    assert!(no_gpu(), "told unusable before the GPU opened");
    assert!(text.starts_with("not a BS-RoFormer SW model"), "{text}");
    assert_eq!(listing().await, CACHE_NAME);
    assert_eq!(stored_bytes().await, [7]);
}

#[wasm_bindgen_test]
async fn a_download_that_is_not_a_model_leaves_only_the_stored_model() {
    fresh().await;
    plant(CACHE_NAME, &[7]).await;
    let url = blob_url(b"<!doctype html><title>not a model</title>");
    handle(message(&[
        ("type", "download".into()),
        ("model_url", url.as_str().into()),
    ]))
    .await;
    revoke(&url);
    let (ok, _, stored, text) = model_reply();
    assert!(!ok && stored, "{text}");
    assert!(
        text.starts_with("the downloaded file is not a usable BS-RoFormer SW model"),
        "{text}"
    );
    assert_eq!(listing().await, CACHE_NAME, "no partial download left");
    assert_eq!(stored_bytes().await, [7]);
}

#[wasm_bindgen_test]
async fn a_failed_download_keeps_the_stored_model() {
    fresh().await;
    plant(CACHE_NAME, &[7]).await;
    plant(DOWNLOAD_NAME, &[7]).await;
    let url = blob_url(&[1]);
    revoke(&url);
    handle(message(&[
        ("type", "download".into()),
        ("model_url", url.as_str().into()),
    ]))
    .await;
    let (ok, _, stored, text) = model_reply();
    assert!(!ok && stored, "{text}");
    assert!(text.starts_with("model download failed"), "{text}");
    assert_eq!(listing().await, CACHE_NAME);
}

#[wasm_bindgen_test]
async fn a_download_cut_off_midway_leaves_no_partial_file() {
    fresh().await;
    plant(CACHE_NAME, &[7]).await;
    break_next_fetch();
    handle(message(&[
        ("type", "download".into()),
        ("model_url", "/never-fetched".into()),
    ]))
    .await;
    let (ok, _, stored, text) = model_reply();
    assert!(!ok && stored, "{text}");
    assert!(text.contains("network error"), "{text}");
    assert_eq!(listing().await, CACHE_NAME);
}

#[wasm_bindgen_test]
async fn forget_model_removes_the_stored_model_and_partial_files() {
    fresh().await;
    plant(CACHE_NAME, &[7]).await;
    plant(DOWNLOAD_NAME, &[7]).await;
    plant(&format!("{KERNEL_CHOICE}.part"), &[7]).await;
    plant(KERNEL_CHOICE, &[7]).await;
    handle(of_type("forget_model")).await;
    let (ok, loaded, stored, text) = model_reply();
    assert!(!ok && !loaded && !stored, "{text}");
    assert_eq!(text, "model removed from this browser");
    assert_eq!(listing().await, KERNEL_CHOICE, "the kernel choice stays");
}

#[wasm_bindgen_test]
async fn errors_name_the_request_they_answer() {
    fresh().await;
    // 4 frames, then none: an unsupported rate fails either way.
    for frames in [4, 0] {
        let samples = Float32Array::new_with_length(frames);
        handle(message(&[
            ("type", "mp3".into()),
            ("name", "x.mp3".into()),
            ("left", samples.clone().into()),
            ("right", samples.into()),
            ("rate", 96_000.into()),
        ]))
        .await;
    }
    let mix = Float32Array::new_with_length(4);
    handle(message(&[
        ("type", "separate".into()),
        ("left", mix.clone().into()),
        ("right", mix.into()),
    ]))
    .await;
    let errors = sent("error");
    let of: Vec<String> = errors
        .iter()
        .map(|e| field(e, "of").as_string().unwrap_or_default())
        .collect();
    assert_eq!(of, ["mp3", "mp3", "separate"]);
    assert_eq!(
        field(&errors[2], "text").as_string().as_deref(),
        Some("load the model first")
    );
}

const TONE_RATE: u32 = 44_100;
/// 30 s at 44.1 kHz.
const TONE_FRAMES: usize = 30 * TONE_RATE as usize;

/// Both channels, as the memory path of a save sends them.
fn arrays(left: &Float32Array, right: &Float32Array, rate: Option<u32>) -> Encode {
    Encode::Arrays {
        left: left.clone(),
        right: right.clone(),
        rate,
    }
}

/// `frames` of a sine at `hz`, as the page sends a channel.
fn tone(hz: f32, frames: usize) -> Float32Array {
    let step = std::f32::consts::TAU * hz / TONE_RATE as f32;
    let samples: Vec<f32> = (0..frames).map(|i| 0.5 * (step * i as f32).sin()).collect();
    Float32Array::from(samples.as_slice())
}

/// The MP3 of `left` and `right` encoded in one push, as `mp3` did before
/// it read the channels in blocks.  It stops at the shorter channel.
fn encoded_in_one_push(left: &Float32Array, right: &Float32Array) -> Vec<u8> {
    let mut pcm = Vec::new();
    for (a, b) in left.to_vec().iter().zip(&right.to_vec()) {
        pcm.push(*a);
        pcm.push(*b);
    }
    let mut enc = rusty_mp3::Mp3Encoder::new(rusty_mp3::Mp3EncoderConfig {
        bitrate_kbps: 192,
        vbr_quality: None,
    });
    enc.push_pcm_f32(&pcm, 2, TONE_RATE).expect("encoded");
    enc.finish();
    let mut out = Vec::new();
    while let Ok(packet) = enc.next_packet() {
        out.extend_from_slice(&packet);
    }
    out
}

#[wasm_bindgen_test]
async fn an_mp3_is_encoded_in_blocks() {
    fresh().await;
    let (left, right) = (tone(220.0, TONE_FRAMES), tone(330.0, TONE_FRAMES));
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    encode(
        Codec::Mp3,
        Some("tone.mp3".into()),
        arrays(&left, &right, Some(TONE_RATE)),
    )
    .await
    .expect("encoded");
    let held = PEAK.with(Cell::get) - before;
    let replies = sent("mp3");
    assert_eq!(replies.len(), 1, "one mp3 reply");
    let bytes = Uint8Array::new(&field(&replies[0], "bytes")).to_vec();
    assert_eq!(
        bytes,
        encoded_in_one_push(&left, &right),
        "the MP3 is the same as one encoded in one push"
    );
    let one_channel = 4 * TONE_FRAMES;
    assert!(
        held < one_channel,
        "MEM-1: encoding 30 s held {held} bytes at its peak, more than one channel's \
         {one_channel}.  Fix: read the channels from JS in blocks."
    );
}

#[wasm_bindgen_test]
async fn an_mp3_of_unequal_channels_stops_at_the_shorter() {
    fresh().await;
    // The shorter channel ends inside the second block, on either side.
    let (long, short) = (TONE_FRAMES, MP3_BLOCK + 7);
    for (left_frames, right_frames) in [(long, short), (short, long)] {
        let (left, right) = (tone(220.0, left_frames), tone(330.0, right_frames));
        encode(
            Codec::Mp3,
            Some("tone.mp3".into()),
            arrays(&left, &right, Some(TONE_RATE)),
        )
        .await
        .expect("encoded");
        let reply = sent("mp3").pop().expect("an mp3 reply");
        let bytes = Uint8Array::new(&field(&reply, "bytes")).to_vec();
        assert_eq!(
            bytes,
            encoded_in_one_push(&left, &right),
            "left {left_frames} frames, right {right_frames}"
        );
    }
    assert_eq!(sent("mp3").len(), 2, "one mp3 reply for each encode");
}

/// The bytes of the FLAC that the last `flac` reply carries.
async fn flac_reply() -> Vec<u8> {
    let reply = sent("flac").pop().expect("a flac reply");
    let file: Blob = field(&reply, "file").dyn_into().expect("a Blob");
    assert_eq!(file.type_(), "audio/flac");
    let bytes = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
        .await
        .expect("read");
    Uint8Array::new(&bytes).to_vec()
}

/// A FLAC decoded by Symphonia: its rate, its channels, and whether its
/// MD5 matches the audio.
fn decode_flac(bytes: Vec<u8>) -> (u32, Vec<Vec<f32>>, Option<bool>) {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            Hint::new().with_extension("flac"),
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .expect("a FLAC that Symphonia reads");
    let track = format.default_track(TrackType::Audio).expect("a track");
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .expect("audio")
        .clone();
    let options = AudioDecoderOptions::default().verify(true);
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &options)
        .expect("a decoder");
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let mut planes: Vec<Vec<f32>> = Vec::new();
    while let Ok(Some(packet)) = format.next_packet() {
        let buf = decoder.decode(&packet).expect("every frame decodes");
        buf.copy_to_vecs_planar::<f32>(&mut planes);
        channels.resize(planes.len(), Vec::new());
        for (c, p) in channels.iter_mut().zip(&planes) {
            c.extend_from_slice(p);
        }
    }
    let rate = params.sample_rate.expect("a rate");
    (rate, channels, decoder.finalize().verify_ok)
}

/// `channel` as the 16-bit samples that a FLAC holds.
fn as_16_bits(channel: &Float32Array) -> Vec<f32> {
    channel
        .to_vec()
        .iter()
        .map(|x| pcm16(*x) as f32 / 32768.0)
        .collect()
}

async fn encode_flac(left: &Float32Array, right: &Float32Array, rate: u32) -> Vec<u8> {
    encode(
        Codec::Flac,
        Some("tone.flac".into()),
        arrays(left, right, Some(rate)),
    )
    .await
    .expect("encoded");
    flac_reply().await
}

#[wasm_bindgen_test]
async fn a_flac_decodes_to_the_stem() {
    fresh().await;
    let (left, right) = (tone(220.0, TONE_FRAMES), tone(330.0, TONE_FRAMES));
    let bytes = encode_flac(&left, &right, 48_000).await;
    assert!(bytes.len() < TONE_FRAMES * 4, "{} bytes", bytes.len());
    let (rate, channels, verified) = decode_flac(bytes);
    assert_eq!(rate, 48_000);
    // Against the input, not `pcm16`, so a wrong quantiser fails.
    for (got, sent) in channels.iter().zip([&left, &right]) {
        let sent = sent.to_vec();
        assert_eq!(got.len(), sent.len());
        for (g, s) in got.iter().zip(&sent) {
            assert!((g - s).abs() <= 0.5 / 32768.0 + 1e-7, "{g} for {s}");
        }
    }
    assert_eq!(verified, Some(true), "the MD5 matches the audio");
    let reply = sent("flac").pop().expect("a flac reply");
    assert_eq!(
        field(&reply, "name").as_string().as_deref(),
        Some("tone.flac")
    );
}

#[wasm_bindgen_test]
async fn a_flac_is_encoded_in_blocks() {
    fresh().await;
    let (left, right) = (tone(220.0, TONE_FRAMES), tone(330.0, TONE_FRAMES));
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    encode(Codec::Flac, None, arrays(&left, &right, Some(TONE_RATE)))
        .await
        .expect("encoded");
    let held = PEAK.with(Cell::get) - before;
    let one_channel = 4 * TONE_FRAMES;
    assert!(
        held < one_channel / 10,
        "MEM-1: encoding 30 s held {held} bytes at its peak, more than a tenth of one \
         channel's {one_channel}.  Fix: read the channels from JS in blocks, and put each \
         FLAC frame in JS."
    );
    assert_eq!(sent("flac").len(), 1, "one flac reply");
}

#[wasm_bindgen_test]
async fn a_flac_of_unequal_channels_stops_at_the_shorter() {
    fresh().await;
    // The shorter channel ends inside the second block, on either side.
    let (long, short) = (3 * FLAC_BLOCK, FLAC_BLOCK + 7);
    for (left_frames, right_frames) in [(long, short), (short, long)] {
        let (left, right) = (tone(220.0, left_frames), tone(330.0, right_frames));
        let (_, channels, verified) = decode_flac(encode_flac(&left, &right, TONE_RATE).await);
        let shorter = |c: &Float32Array| as_16_bits(&c.subarray(0, short as u32));
        assert_eq!(
            channels,
            [shorter(&left), shorter(&right)],
            "left {left_frames} frames, right {right_frames}"
        );
        assert_eq!(verified, Some(true));
    }
}

#[wasm_bindgen_test]
async fn a_flac_clips_at_full_scale_and_rounds() {
    fresh().await;
    let step = 1.0 / 32_768.0;
    let left = Float32Array::from(&[1.5_f32, -1.5, 1.0, -1.0, 0.5, 0.75 * step, 0.25 * step][..]);
    let right = Float32Array::from(&[0.0_f32; 7][..]);
    let (_, channels, _) = decode_flac(encode_flac(&left, &right, TONE_RATE).await);
    // Clipped at full scale, and rounded to the nearest step.
    assert_eq!(
        channels[0],
        [32_767.0 * step, -1.0, 32_767.0 * step, -1.0, 0.5, step, 0.0]
    );
}

/// flacenc has no escape codes and Rice parameters of 14 bits at most,
/// which at 24 bits made a square wave 1,270 times its PCM size.  At 16
/// bits, signals that are hard to predict still encode smaller than PCM.
#[wasm_bindgen_test]
async fn a_flac_of_worst_case_signals_is_no_larger_than_pcm() {
    fresh().await;
    let frames = 2 * TONE_RATE as usize;
    let mut seed = 7_u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let signals: [(&str, Vec<f32>); 4] = [
        ("noise", (0..frames).map(|_| noise()).collect()),
        (
            "square",
            (0..frames)
                .map(|i| if (i / 50) % 2 == 0 { 1.0 } else { -1.0 })
                .collect(),
        ),
        (
            "alternate",
            (0..frames)
                .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
                .collect(),
        ),
        (
            "clipped",
            (0..frames)
                .map(|i| (4.0 * (0.05 * i as f32).sin()).clamp(-1.0, 1.0))
                .collect(),
        ),
    ];
    for (name, samples) in signals {
        let left = Float32Array::from(samples.as_slice());
        let right = Float32Array::from(samples.iter().map(|x| -x).collect::<Vec<f32>>().as_slice());
        let bytes = encode_flac(&left, &right, TONE_RATE).await;
        assert!(bytes.len() < frames * 4, "{name}: {} bytes", bytes.len());
        let (_, channels, verified) = decode_flac(bytes);
        assert_eq!(channels, [as_16_bits(&left), as_16_bits(&right)], "{name}");
        assert_eq!(verified, Some(true), "{name}");
    }
}

#[wasm_bindgen_test]
async fn a_flac_of_no_samples_is_refused() {
    fresh().await;
    let empty = Float32Array::new_with_length(0);
    let error = encode(Codec::Flac, None, arrays(&empty, &empty, None))
        .await
        .expect_err("refused");
    assert!(error.contains("no samples"), "{error}");
    assert!(sent("flac").is_empty());
}

/// A save's WAV file and its outputs, as the page names them (D1 of M8).
const SAVE_WAV: &str = "saves/1792640000000-1.wav";
const SAVE_FLAC: &str = "saves/1792640000000-1.flac";
const SAVE_MP3: &str = "saves/1792640000000-1.mp3";

/// A save's 16-bit WAV of `frames` frames of two tones: its bytes, and its
/// channels as the floats `s / 32768` of its 16-bit samples.
fn save_wav(frames: usize) -> (Vec<u8>, Float32Array, Float32Array) {
    let (left, right) = (tone(220.0, frames).to_vec(), tone(330.0, frames).to_vec());
    let pairs: Vec<(i16, i16)> = left
        .iter()
        .zip(&right)
        .map(|(l, r)| (pcm16(*l) as i16, pcm16(*r) as i16))
        .collect();
    let as_16_bits = |c: &[f32]| {
        let floats: Vec<f32> = c.iter().map(|x| pcm16(*x) as f32 / 32768.0).collect();
        Float32Array::from(floats.as_slice())
    };
    (wav_file(&pairs), as_16_bits(&left), as_16_bits(&right))
}

/// A stored encode of the WAV at `wav`, of `frames` frames, into `out`.
async fn encode_stored(codec: Codec, wav: &str, frames: usize, out: &str) -> Result<(), String> {
    encode(
        codec,
        Some(format!("lloyd - mix.{}", codec.kind())),
        Encode::Stored {
            wav: wav.into(),
            frames,
            out: out.into(),
            rate: TONE_RATE,
        },
    )
    .await
}

/// The file of the last reply of `kind`, read as the page reads it.
fn last_file(kind: &str) -> Encoded {
    let reply = sent(kind).pop().expect("a reply");
    match Reply::from_js(&reply).expect("a reply the page reads") {
        Reply::Mp3 { file, .. } | Reply::Flac { file, .. } => file,
        other => panic!("{other:?}"),
    }
}

/// The bytes of the file that the arrays path gives for the same samples.
async fn arrays_file(codec: Codec, left: &Float32Array, right: &Float32Array) -> Vec<u8> {
    encode(codec, None, arrays(left, right, Some(TONE_RATE)))
        .await
        .expect("encoded");
    match last_file(codec.kind()) {
        Encoded::Bytes(bytes) => bytes.to_vec(),
        Encoded::Blob(_) => flac_reply().await,
        other => panic!("{other:?}"),
    }
}

/// D2 and D4 of M8: a stored FLAC is the FLAC of the WAV's 16-bit samples,
/// bit for bit, and the worker removes the WAV at the end.
#[wasm_bindgen_test]
async fn a_stored_flac_is_the_flac_of_its_16_bit_samples() {
    for frames in [3 * FLAC_BLOCK + 7, TONE_FRAMES] {
        fresh().await;
        let (wav, left, right) = save_wav(frames);
        plant_at(SAVE_WAV, &wav).await;
        encode_stored(Codec::Flac, SAVE_WAV, frames, SAVE_FLAC)
            .await
            .expect("no error reply");
        assert!(
            matches!(last_file("flac"), Encoded::Stored { path } if path == SAVE_FLAC),
            "the reply names the output file"
        );
        let stored = read_at(SAVE_FLAC).await.expect("the FLAC file");
        assert_eq!(
            names_in("saves").await.as_deref(),
            Some("1792640000000-1.flac"),
            "the worker removed the WAV"
        );
        let (_, channels, verified) = decode_flac(stored.clone());
        assert_eq!(verified, Some(true), "{frames} frames: the MD5 matches");
        assert_eq!(channels[0].len(), frames);
        assert!(
            stored == arrays_file(Codec::Flac, &left, &right).await,
            "{frames} frames: the stored FLAC is the arrays FLAC of the same 16-bit samples"
        );
        let memory = sent("log")
            .iter()
            .filter_map(|l| field(l, "text").as_string())
            .find(|t| t.starts_with(&format!("memory: encode flac {SAVE_FLAC}: wasm ")));
        assert!(
            memory
                .as_ref()
                .is_some_and(|m| m.contains(&format!("bytes, file {} bytes", stored.len()))),
            "D9: a memory line names the file's size: {memory:?}"
        );
    }
}

/// D2 of M8: a stored MP3 is the MP3 of the WAV's 16-bit samples, bit for
/// bit, and the worker removes the WAV at the end.
#[wasm_bindgen_test]
async fn a_stored_mp3_is_the_mp3_of_its_16_bit_samples() {
    fresh().await;
    let (wav, left, right) = save_wav(TONE_FRAMES);
    plant_at(SAVE_WAV, &wav).await;
    encode_stored(Codec::Mp3, SAVE_WAV, TONE_FRAMES, SAVE_MP3)
        .await
        .expect("no error reply");
    assert!(
        matches!(last_file("mp3"), Encoded::Stored { path } if path == SAVE_MP3),
        "the reply names the output file"
    );
    let stored = read_at(SAVE_MP3).await.expect("the MP3 file");
    assert_eq!(
        names_in("saves").await.as_deref(),
        Some("1792640000000-1.mp3"),
        "the worker removed the WAV"
    );
    assert!(
        stored == arrays_file(Codec::Mp3, &left, &right).await,
        "the stored MP3 is the arrays MP3 of the same 16-bit samples"
    );
}

/// MEM-5 and the order of D4 of M8: every write of a stored encode is a
/// whole array, and each read of the WAV is one block.  A FLAC writes each
/// frame before it reads the next block, so it keeps no parts, and its
/// stream header has the same length at the start and at the end.
#[wasm_bindgen_test]
async fn stored_encodes_write_whole_arrays_as_they_read() {
    fresh().await;
    let frames = 3 * FLAC_BLOCK + 7;
    let (wav, _, _) = save_wav(frames);
    plant_at(SAVE_WAV, &wav).await;
    start_order_spy();
    let encoded = encode_stored(Codec::Flac, SAVE_WAV, frames, SAVE_FLAC).await;
    let record = stop_order_spy();
    encoded.expect("no error reply");
    assert!(matches!(last_file("flac"), Encoded::Stored { .. }));
    let steps: Vec<&str> = record.split(' ').collect();
    assert!(!record.contains("view"), "MEM-5: {record}");
    let block = format!("r{}", 4 * FLAC_BLOCK);
    let reads: Vec<&str> = steps
        .iter()
        .copied()
        .filter(|s| s.starts_with('r'))
        .collect();
    assert_eq!(reads, [block.as_str(), &block, &block, "r28"], "{record}");
    // The header at 0, then a read and its frame's write for each block,
    // then the header at 0 again.
    assert_eq!(steps.len(), 2 + 2 * reads.len(), "{record}");
    assert_eq!(steps[0], "w42@0", "{record}");
    assert_eq!(steps[steps.len() - 1], "w42@0", "{record}");
    for pair in steps[1..steps.len() - 1].chunks(2) {
        assert!(
            pair[0].starts_with('r') && pair[1].starts_with('w'),
            "each read is followed by its frame's write: {record}"
        );
    }

    fresh().await;
    let (wav, _, _) = save_wav(TONE_FRAMES);
    plant_at(SAVE_WAV, &wav).await;
    start_order_spy();
    let encoded = encode_stored(Codec::Mp3, SAVE_WAV, TONE_FRAMES, SAVE_MP3).await;
    let record = stop_order_spy();
    encoded.expect("no error reply");
    assert!(matches!(last_file("mp3"), Encoded::Stored { .. }));
    assert!(!record.contains("view"), "MEM-5: {record}");
    let most = 4 * MP3_BLOCK;
    // Nit 1-14 of review round 1 of M8: an MPEG-1 Layer III frame holds
    // 144 x bitrate / rate bytes, and one more with padding.  So each write
    // is one packet, and a stored MP3 makes one write for each frame.
    let packet = 144 * 192_000 / TONE_RATE as usize + 1;
    let mut writes = 0;
    for step in record.split(' ') {
        if let Some(bytes) = step.strip_prefix('r') {
            let bytes: usize = bytes.parse().expect("a size");
            assert!(bytes <= most, "a read of {bytes} bytes, over one block");
        }
        if let Some(write) = step.strip_prefix('w') {
            let bytes: usize = write
                .split('@')
                .next()
                .and_then(|b| b.parse().ok())
                .expect("a size");
            assert!(
                bytes <= packet,
                "a write of {bytes} bytes, over one packet of {packet}"
            );
            writes += 1;
        }
    }
    assert!(
        writes >= TONE_FRAMES / 1152,
        "{writes} writes, fewer than the {} frames of the MP3",
        TONE_FRAMES / 1152
    );
}

/// The peak of the counting allocator during a stored encode of `seconds`
/// of a save's WAV, and the size of the file.
async fn stored_peak(codec: Codec, seconds: usize) -> (usize, usize) {
    fresh().await;
    let frames = seconds * TONE_RATE as usize;
    let (wav, _, _) = save_wav(frames);
    plant_at(SAVE_WAV, &wav).await;
    drop(wav);
    let out = if codec == Codec::Flac {
        SAVE_FLAC
    } else {
        SAVE_MP3
    };
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    encode_stored(codec, SAVE_WAV, frames, out)
        .await
        .expect("no error reply");
    let held = PEAK.with(Cell::get) - before;
    assert!(matches!(last_file(codec.kind()), Encoded::Stored { .. }));
    (held, read_at(out).await.expect("the file").len())
}

/// MEM-1: a stored FLAC encode holds the same memory whatever the song's
/// length.
#[wasm_bindgen_test]
async fn a_stored_flac_keeps_the_worker_memory_flat() {
    let (ten, _) = stored_peak(Codec::Flac, 10).await;
    let (sixty, _) = stored_peak(Codec::Flac, 60).await;
    assert!(
        sixty <= ten + 64 * 1024,
        "MEM-1: a stored FLAC of 60 s held {sixty} bytes at its peak, and of 10 s {ten}.  \
         Fix: read the WAV one block at a time, and write each frame at once."
    );
    let (thirty, _) = stored_peak(Codec::Flac, 30).await;
    let one_channel = 4 * TONE_FRAMES;
    assert!(
        thirty < one_channel / 10,
        "MEM-1: a stored FLAC of 30 s held {thirty} bytes at its peak, more than a tenth \
         of one channel's {one_channel}"
    );
}

/// The MP3 exception of D4 of M8 (TD-59): a stored MP3 encode holds the
/// MP3's frames until `finish()`, and no more than 4 times the MP3's size
/// plus 1 MiB.
#[wasm_bindgen_test]
async fn a_stored_mp3_holds_at_most_four_times_its_file() {
    let (held, size) = stored_peak(Codec::Mp3, 60).await;
    web_sys::console::log_1(
        &format!(
            "a stored MP3 of 60 s: {size} bytes of MP3, a peak of {held} bytes ({:.2} times the MP3)",
            held as f64 / size as f64
        )
        .into(),
    );
    assert!(
        held < 4 * size + (1 << 20),
        "MEM-1: a stored MP3 of 60 s ({size} bytes) held {held} bytes at its peak"
    );
}

/// D4 of M8: a stored encode that fails replies `failed` with its output
/// path, removes its WAV, and leaves no output file.  Only a malformed
/// request gives `error`.
#[wasm_bindgen_test]
async fn a_failed_stored_encode_says_why_and_leaves_no_file() {
    let frames = 3 * FLAC_BLOCK + 7;
    for (codec, out) in [(Codec::Flac, SAVE_FLAC), (Codec::Mp3, SAVE_MP3)] {
        let kind = codec.kind();
        for case in [
            "wrong size",
            "too long",
            "missing",
            "no handle",
            "write refused",
        ] {
            fresh().await;
            let (wav, _, _) = save_wav(frames);
            match case {
                "wrong size" => plant_at(SAVE_WAV, &wav[..wav.len() - 4]).await,
                // Finding 3-4 of review round 3 of M8: 4 bytes more.
                "too long" => plant_at(SAVE_WAV, &[wav.as_slice(), &[0; 4]].concat()).await,
                "missing" => plant_folder("saves").await,
                _ => plant_at(SAVE_WAV, &wav).await,
            }
            let restore = (case == "no handle").then(|| refuse_sync_handle(1));
            if case == "write refused" {
                start_sync_spy(2);
            }
            let encoded = encode_stored(codec, SAVE_WAV, frames, out).await;
            if case == "write refused" {
                stop_sync_spy();
            }
            if let Some(restore) = restore {
                restore.call0(&JsValue::NULL).expect("restored");
            }
            encoded.unwrap_or_else(|e| panic!("{kind}, {case}: an error reply: {e}"));
            match last_file(kind) {
                Encoded::Failed { path, text } => {
                    assert_eq!(path, out, "{kind}, {case}");
                    assert!(!text.is_empty(), "{kind}, {case}");
                }
                other => panic!("{kind}, {case}: {other:?}"),
            }
            assert_eq!(
                names_in("saves").await.as_deref(),
                Some(""),
                "{kind}, {case}: no WAV and no output stay"
            );
            assert!(sent("error").is_empty(), "{kind}, {case}");
        }
    }
}

/// D4 of M8: two stored encodes run at once, and the one that fails
/// leaves the other's WAV and output whole.
#[wasm_bindgen_test]
async fn a_failed_stored_encode_leaves_another_encode_whole() {
    fresh().await;
    let (wav, left, right) = save_wav(TONE_FRAMES);
    plant_at(SAVE_WAV, &wav).await;
    plant_at("saves/1792640000000-2.wav", &wav[..wav.len() - 4]).await;
    for (wav, out) in [
        (SAVE_WAV, SAVE_FLAC),
        ("saves/1792640000000-2.wav", "saves/1792640000000-2.flac"),
    ] {
        wasm_bindgen_futures::spawn_local(async move {
            encode_stored(Codec::Flac, wav, TONE_FRAMES, out)
                .await
                .expect("no error reply");
        });
    }
    until(|| sent("flac").len() == 2).await;
    let files: Vec<Encoded> = sent("flac")
        .iter()
        .map(|r| match Reply::from_js(r).expect("a reply") {
            Reply::Flac { file, .. } => file,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(
        files.iter().any(
            |f| matches!(f, Encoded::Failed { path, .. } if path == "saves/1792640000000-2.flac")
        ),
        "the short WAV fails: {files:?}"
    );
    assert!(
        files
            .iter()
            .any(|f| matches!(f, Encoded::Stored { path } if path == SAVE_FLAC)),
        "the other is stored: {files:?}"
    );
    assert_eq!(
        names_in("saves").await.as_deref(),
        Some("1792640000000-1.flac")
    );
    let stored = read_at(SAVE_FLAC).await.expect("the FLAC");
    assert!(stored == arrays_file(Codec::Flac, &left, &right).await);
}

/// D5 of M8: a stored request that names a file outside `saves/`, or a
/// malformed one, gives `error`, and the worker removes nothing.
#[wasm_bindgen_test]
async fn a_stored_encode_outside_saves_or_malformed_is_refused() {
    let session_wav = "sessions/1792640000000/stems/bass.wav";
    let (wav, _, _) = save_wav(7);
    let request = |wav: &str, out: Option<&str>| {
        let mut fields = vec![
            ("type", "flac".into()),
            ("wav", wav.into()),
            ("frames", 7.into()),
            ("rate", 44_100.into()),
        ];
        if let Some(out) = out {
            fields.push(("out", out.into()));
        }
        message(&fields)
    };
    let outside =
        |field: &str, path: &str| format!("{field} must be a file in saves/, not {path:?}");
    for (wav_path, out, why) in [
        (session_wav, Some(SAVE_FLAC), outside("wav", session_wav)),
        (
            SAVE_WAV,
            Some("sessions/1792640000000/stems/bass.flac"),
            outside("out", "sessions/1792640000000/stems/bass.flac"),
        ),
        (
            SAVE_WAV,
            Some("saves/../bass.flac"),
            outside("out", "saves/../bass.flac"),
        ),
        (SAVE_WAV, Some("saves/"), outside("out", "saves/")),
        (
            "saves/sub/1.wav",
            Some(SAVE_FLAC),
            outside("wav", "saves/sub/1.wav"),
        ),
        (
            SAVE_WAV,
            Some(SAVE_WAV),
            "wav and out must be two files".to_string(),
        ),
        (
            SAVE_WAV,
            None,
            "a stored encode names its out file".to_string(),
        ),
    ] {
        fresh().await;
        plant_at(wav_path, &wav).await;
        handle(request(wav_path, out)).await;
        let errors = sent("error");
        assert_eq!(errors.len(), 1, "{wav_path} to {out:?}");
        assert_eq!(field(&errors[0], "of").as_string().as_deref(), Some("flac"));
        // Nit 2-9 of review round 2 of M8: each case for its own reason.
        assert_eq!(
            field(&errors[0], "text").as_string().as_deref(),
            Some(why.as_str()),
            "{wav_path} to {out:?}"
        );
        assert!(sent("flac").is_empty(), "{wav_path} to {out:?}");
        assert_eq!(
            read_at(wav_path).await,
            Some(wav.clone()),
            "{wav_path} to {out:?}: the worker removed nothing"
        );
    }
}

#[wasm_bindgen_test]
async fn init_keeps_a_stored_model_that_only_the_gpu_failed_to_load() {
    fresh().await;
    plant(CACHE_NAME, &EMPTY_MODEL).await;
    let show_gpu = hide_gpu();
    handle(of_type("init")).await;
    show_gpu.call0(&JsValue::NULL).expect("restored");
    let (ok, loaded, stored, text) = model_reply();
    assert!(!ok && !loaded && stored, "{text}");
    assert!(
        text.starts_with("the model in browser storage could not be loaded, and was kept"),
        "{text}"
    );
    assert_eq!(listing().await, CACHE_NAME);
}

#[wasm_bindgen_test]
async fn a_picked_file_is_loaded_from_itself_when_storage_cannot_copy_it() {
    fresh().await;
    plant_dir(DOWNLOAD_NAME).await;
    let show_gpu = hide_gpu();
    handle(message(&[
        ("type", "model".into()),
        ("file", blob(&EMPTY_MODEL).into()),
    ]))
    .await;
    show_gpu.call0(&JsValue::NULL).expect("restored");
    let (ok, loaded, stored, text) = model_reply();
    assert!(!ok && !loaded && !stored, "{text}");
    assert!(
        text.starts_with("the model could not be loaded"),
        "built from the picked file, as far as the GPU: {text}"
    );
    assert_eq!(listing().await, "", "nothing stored");
}

#[wasm_bindgen_test]
async fn a_failed_load_says_when_it_unloaded_the_model_before_it() {
    fresh().await;
    assert_eq!(unloaded_note(false).await, "", "there was none");
    assert_eq!(
        unloaded_note(true).await,
        "; the model loaded before was unloaded to make room"
    );
    plant(CACHE_NAME, &[7]).await;
    assert!(
        unloaded_note(true)
            .await
            .ends_with("reload the page to load the stored one")
    );
}

/// A session folder, as the page makes it before the song shows.
const SESSION: &str = "sessions/1792640000000";

fn part_path(name: &str) -> String {
    format!("{SESSION}/stems/{name}.wav.part")
}

/// The 44-byte header of a 16-bit stereo WAV at 44.1 kHz of `frames`
/// frames, written out field by field.
fn wav_header(frames: u32) -> Vec<u8> {
    let data = 4 * frames;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&44_100u32.to_le_bytes());
    out.extend_from_slice(&176_400u32.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out
}

/// A WAV file of interleaved 16-bit `frames` (left, right).
fn wav_file(frames: &[(i16, i16)]) -> Vec<u8> {
    let mut out = wav_header(frames.len() as u32);
    for (l, r) in frames {
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&r.to_le_bytes());
    }
    out
}

const STEP: f32 = 1.0 / 32_768.0;

/// Seven frames of two stems: "bass" holds the rounding and clipping
/// cases, "drums" a plain half scale.
fn two_stems() -> [[Vec<f32>; 2]; 2] {
    [
        [
            vec![0.5 * STEP, -0.5 * STEP, 1.0, 1.5, -1.0, 0.25, 0.0],
            vec![
                0.0,
                0.1,
                -0.1,
                -1.5,
                32_767.0 * STEP,
                0.75 * STEP,
                -0.75 * STEP,
            ],
        ],
        [vec![0.5; 7], vec![-0.5; 7]],
    ]
}

/// The 16-bit frames of [`two_stems`].
fn two_stems_as_16_bits() -> [Vec<(i16, i16)>; 2] {
    [
        vec![
            (1, 0),
            (-1, 3277),
            (32_767, -3277),
            (32_767, -32_768),
            (-32_768, 32_767),
            (8192, 1),
            (0, -1),
        ],
        vec![(16_384, -16_384); 7],
    ]
}

/// [`two_stems`] cut into segments at `cuts` (frames 0..3, 3..5, 5..7).
fn segments_of(stems: &[[Vec<f32>; 2]], cuts: &[(usize, usize)]) -> Vec<Segment> {
    cuts.iter()
        .map(|&(from, to)| Segment {
            start: from,
            stems: stems
                .iter()
                .map(|[l, r]| [l[from..to].to_vec(), r[from..to].to_vec()])
                .collect(),
        })
        .collect()
}

const CUTS: [(usize, usize); 3] = [(0, 3), (3, 5), (5, 7)];

async fn write_two_stems() -> Result<(), StemError> {
    let mut w = StemWriter::open(SESSION, &["bass", "drums"], 7).await?;
    for seg in segments_of(&two_stems(), &CUTS) {
        w.put(&seg).await?;
    }
    assert_eq!(w.frames(), 7, "frames in each file");
    w.finish().await
}

#[wasm_bindgen_test]
async fn stems_are_written_as_16_bit_wavs() {
    fresh().await;
    plant_folder(SESSION).await;
    write_two_stems().await.expect("written");
    let [bass, drums] = two_stems_as_16_bits();
    assert_eq!(read_at(&part_path("bass")).await, Some(wav_file(&bass)));
    assert_eq!(read_at(&part_path("drums")).await, Some(wav_file(&drums)));
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some("bass.wav.part drums.wav.part"),
        "only the .part files"
    );
}

/// MEM-5: every write through a sync access handle gets a whole array.
#[wasm_bindgen_test]
async fn stem_writes_pass_whole_arrays() {
    fresh().await;
    plant_folder(SESSION).await;
    start_sync_spy(0);
    let written = write_two_stems().await;
    let record = stop_sync_spy();
    written.expect("written");
    // Two headers at the start, two stems in each of three segments, and
    // two headers at the end.
    assert_eq!(record, ["whole"; 10].join(" "));
}

#[wasm_bindgen_test]
async fn a_stem_writer_never_brings_back_a_forgotten_session() {
    fresh().await;
    let opened = StemWriter::open(SESSION, &["bass"], 7).await;
    match opened {
        Err(StemError::Storage(text)) => assert!(text.contains("NotFoundError"), "{text}"),
        Err(StemError::Refused(text)) => panic!("refused: {text}"),
        Ok(_) => panic!("opened a writer in a forgotten session"),
    }
    assert_eq!(names_in("").await.as_deref(), Some(""), "nothing made");
}

#[wasm_bindgen_test]
async fn a_refused_stem_write_removes_the_stem_files() {
    fresh().await;
    plant_folder(SESSION).await;
    let names = ["bass", "drums"];
    let mut w = StemWriter::open(SESSION, &names, 7).await.expect("opened");
    start_sync_spy(3);
    let mut put = Ok(());
    for seg in segments_of(&two_stems(), &CUTS) {
        put = w.put(&seg).await;
        if put.is_err() {
            break;
        }
    }
    let record = stop_sync_spy();
    assert_eq!(record, "whole whole refused");
    match put {
        Err(StemError::Storage(text)) => {
            assert!(text.starts_with("QuotaExceededError"), "{text}")
        }
        other => panic!("{:?}", other.err()),
    }
    w.abandon().await;
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some(""),
        "no stem file remains"
    );
    let again = StemWriter::open(SESSION, &names, 7)
        .await
        .expect("the handles were closed, so the files open again");
    again.abandon().await;
}

/// Finding 1-12 of review round 1: a `.part` file that the open made
/// goes again when the open fails before it holds the file.
#[wasm_bindgen_test]
async fn a_failed_open_removes_the_files_it_made() {
    fresh().await;
    plant_folder(SESSION).await;
    let restore = refuse_sync_handle(2);
    let opened = StemWriter::open(SESSION, &["bass", "drums"], 7).await;
    restore.call0(&JsValue::NULL).expect("restored");
    match opened {
        Err(StemError::Storage(text)) => {
            assert!(text.starts_with("NoModificationAllowedError"), "{text}")
        }
        Err(StemError::Refused(text)) => panic!("refused: {text}"),
        Ok(_) => panic!("opened"),
    }
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some(""),
        "no stem file remains"
    );
}

/// Finding 1-12: a second writer of the same files fails to open, and
/// leaves the first writer's file as it is.  Chrome gives the second open
/// `NoModificationAllowedError` and Safari `InvalidStateError`, so only
/// these two pass (finding 4-4 of review round 4 of M3).  In both, the
/// lock also refuses the removal, so
/// this test does not tell the `ours` rule apart (finding 2-1);
/// `a_failed_open_keeps_a_file_that_it_did_not_make` does.  It checks
/// that a held file stays.
#[wasm_bindgen_test]
async fn a_failed_open_leaves_a_file_that_another_writer_holds() {
    fresh().await;
    plant_folder(SESSION).await;
    let first = StemWriter::open(SESSION, &["bass"], 7)
        .await
        .expect("opened");
    let second = StemWriter::open(SESSION, &["bass"], 7).await;
    let left = names_in(&format!("{SESSION}/stems")).await;
    // Before any assert: a held lock would fail the tests after this one
    // (finding 3-3 of review round 3).
    first.abandon().await;
    assert!(
        matches!(&second, Err(StemError::Storage(text))
            if text.starts_with("NoModificationAllowedError")
                || text.starts_with("InvalidStateError")),
        "the file is locked: {:?}",
        second.err()
    );
    assert_eq!(
        left.as_deref(),
        Some("bass.wav.part"),
        "the first writer's file stays"
    );
}

/// Finding 2-1 of review round 2: a `.part` file that the open did not
/// make, and could not hold, is not the writer's to remove.
#[wasm_bindgen_test]
async fn a_failed_open_keeps_a_file_that_it_did_not_make() {
    fresh().await;
    plant_at(&part_path("bass"), &[9; 20]).await;
    let restore = refuse_sync_handle(1);
    let opened = StemWriter::open(SESSION, &["bass"], 7).await;
    restore.call0(&JsValue::NULL).expect("restored");
    assert!(
        matches!(opened, Err(StemError::Storage(ref text)) if text.starts_with("NoModificationAllowedError")),
        "the open fails"
    );
    assert_eq!(read_at(&part_path("bass")).await, Some(vec![9; 20]));
}

/// Finding 2-12 of review round 2: `finish` refuses stems shorter than
/// the song.
#[wasm_bindgen_test]
async fn a_writer_with_frames_missing_refuses_to_finish() {
    fresh().await;
    plant_folder(SESSION).await;
    let mut w = StemWriter::open(SESSION, &["bass"], 7)
        .await
        .expect("opened");
    w.put(&Segment {
        start: 0,
        stems: vec![[vec![0.5; 5], vec![0.5; 5]]],
    })
    .await
    .expect("written");
    let finished = w.finish().await;
    w.abandon().await;
    match finished {
        Err(StemError::Refused(text)) => {
            assert_eq!(text, "the stems hold 5 of the song's 7 frames")
        }
        Err(StemError::Storage(text)) => panic!("storage: {text}"),
        Ok(()) => panic!("finished with frames missing"),
    }
}

#[wasm_bindgen_test]
async fn a_part_file_from_an_earlier_separation_is_replaced() {
    fresh().await;
    plant_at(&part_path("bass"), &[9; 200]).await;
    let kept = format!("{SESSION}/stems/bass.wav");
    plant_at(&kept, &[1, 2, 3]).await;
    let mut w = StemWriter::open(SESSION, &["bass"], 2)
        .await
        .expect("opened");
    w.put(&Segment {
        start: 0,
        stems: vec![[vec![0.5, -0.5], vec![0.25, -0.25]]],
    })
    .await
    .expect("written");
    w.finish().await.expect("finished");
    assert_eq!(
        read_at(&part_path("bass")).await,
        Some(wav_file(&[(16_384, 8192), (-16_384, -8192)]))
    );
    assert_eq!(
        read_at(&kept).await,
        Some(vec![1, 2, 3]),
        "kept stem as it was"
    );
}

#[wasm_bindgen_test]
async fn a_song_too_long_for_a_wav_is_refused() {
    fresh().await;
    plant_folder(SESSION).await;
    // 36 + 4 x frames is the RIFF size, a u32.
    let longest = (u32::MAX as usize - 36) / 4;
    match StemWriter::open(SESSION, &["bass"], longest + 1).await {
        Err(StemError::Refused(text)) => assert!(text.contains("too long"), "{text}"),
        Err(StemError::Storage(text)) => panic!("storage: {text}"),
        Ok(_) => panic!("opened a writer for a song too long for a WAV"),
    }
    assert_eq!(
        names_in(SESSION).await.as_deref(),
        Some(""),
        "no file opened"
    );
    let fits = StemWriter::open(SESSION, &["bass"], longest)
        .await
        .expect("the longest song opens");
    fits.abandon().await;
}

/// The stems of the fake network: stem `s` is `0.1 x (s + 1)` of the
/// left channel and minus that of the right.
fn fake_stems_of(x: &Stereo) -> Vec<Stereo> {
    (0..STEMS.len())
        .map(|s| {
            let k = 0.1 * (s + 1) as f32;
            [
                x[0].iter().map(|v| v * k).collect(),
                x[1].iter().map(|v| -v * k).collect(),
            ]
        })
        .collect()
}

/// A network that gives [`fake_stems_of`], and fails at chunk `fail_at`
/// if set.
struct FakeNetwork {
    fail_at: Option<usize>,
}

impl Network for FakeNetwork {
    async fn stems(&mut self, index: usize, x: &Stereo) -> Result<Vec<Stereo>, String> {
        if self.fail_at == Some(index) {
            return Err(format!("the fake network failed at chunk {index}"));
        }
        Ok(fake_stems_of(x))
    }

    fn so_far(&self) -> String {
        "the fake network".into()
    }

    fn finish(&self) {}
}

const FAKE_FRAMES: usize = 10_000;
const FAKE_CHUNK: usize = 2048;

fn fake_mix() -> Stereo {
    mix_of(FAKE_FRAMES)
}

/// A mix of `frames` frames, the start of every longer one.
fn mix_of(frames: usize) -> Stereo {
    [
        (0..frames)
            .map(|i| 0.8 * (0.013 * i as f32).sin())
            .collect(),
        (0..frames)
            .map(|i| 0.6 * (0.029 * i as f32).cos())
            .collect(),
    ]
}

fn fake_demixer() -> Demixer<Stereo> {
    Demixer::new(fake_mix(), FAKE_CHUNK, 2, STEMS.len())
}

/// What a demixer of its own gives with the fake network: the whole
/// stems, whether each chunk gives a segment, and whether the
/// `take_ready` after the last chunk does.
struct FakeRun {
    stems: Vec<Stereo>,
    segment_after: Vec<bool>,
    segment_at_end: bool,
}

fn fake_run() -> FakeRun {
    let mut d = fake_demixer();
    let mut stems: Vec<Stereo> = (0..STEMS.len())
        .map(|_| [vec![0.0; FAKE_FRAMES], vec![0.0; FAKE_FRAMES]])
        .collect();
    let mut place = |seg: Option<Segment>| {
        let Some(seg) = seg else { return false };
        for (whole, [l, r]) in stems.iter_mut().zip(&seg.stems) {
            whole[0][seg.start..seg.start + l.len()].copy_from_slice(l);
            whole[1][seg.start..seg.start + r.len()].copy_from_slice(r);
        }
        true
    };
    let mut segment_after = Vec::new();
    while let Ok(Some(x)) = d.next_input() {
        d.accept(&fake_stems_of(&x));
        segment_after.push(place(d.take_ready()));
    }
    let segment_at_end = place(d.take_ready());
    FakeRun {
        stems,
        segment_after,
        segment_at_end,
    }
}

/// The types of the replies sent since the last [`fresh`], without the
/// log lines.
fn reply_types() -> Vec<String> {
    SENT.with_borrow(|s| {
        s.iter()
            .filter_map(|m| field(m, "type").as_string())
            .filter(|t| t != "log")
            .collect()
    })
}

/// The replies of the fake run's first `chunks` chunks: `progress` before
/// the first, then `per_segment` for each chunk that gives a segment and a
/// `progress` for each chunk.
fn chunk_replies(run: &FakeRun, chunks: usize, per_segment: &str) -> Vec<String> {
    let mut out = vec!["progress".to_string()];
    for segment in &run.segment_after[..chunks] {
        if *segment {
            out.push(per_segment.into());
        }
        out.push("progress".into());
    }
    out
}

/// The replies of the whole fake run.
fn whole_run(run: &FakeRun, per_segment: &str) -> Vec<String> {
    let mut out = chunk_replies(run, run.segment_after.len(), per_segment);
    if run.segment_at_end {
        out.push(per_segment.into());
    }
    out.push("done".into());
    out
}

#[wasm_bindgen_test]
async fn a_stored_separation_sends_no_samples() {
    fresh().await;
    plant_folder(SESSION).await;
    let mut net = FakeNetwork { fail_at: None };
    separate_song(&mut net, fake_demixer(), Some(SESSION), now_s())
        .await
        .expect("separated");
    let run = fake_run();
    assert!(
        run.segment_after.iter().filter(|s| **s).count() > 1,
        "the fake run has several segments"
    );
    assert_eq!(reply_types(), whole_run(&run, "stored"));
    let last = sent("stored").pop().expect("stored");
    assert_eq!(field(&last, "frames").as_f64(), Some(FAKE_FRAMES as f64));
    assert_eq!(field(&last, "len").as_f64(), Some(FAKE_FRAMES as f64));
    let names: Vec<String> = js_sys::Array::from(&field(&last, "stems"))
        .iter()
        .filter_map(|n| n.as_string())
        .collect();
    assert_eq!(names, STEMS);
    for (name, [left, right]) in STEMS.iter().zip(run.stems) {
        let frames: Vec<(i16, i16)> = left
            .iter()
            .zip(&right)
            .map(|(l, r)| (pcm16(*l) as i16, pcm16(*r) as i16))
            .collect();
        assert!(
            read_at(&part_path(name)).await == Some(wav_file(&frames)),
            "{name}: the file holds the fake network's stem, rounded to 16 bits"
        );
    }
}

#[wasm_bindgen_test]
async fn a_separation_without_a_session_sends_segments() {
    fresh().await;
    let mut net = FakeNetwork { fail_at: None };
    separate_song(&mut net, fake_demixer(), None, now_s())
        .await
        .expect("separated");
    assert_eq!(reply_types(), whole_run(&fake_run(), "segment"));
    let last = sent("segment").pop().expect("segment");
    let first = js_sys::Array::from(&js_sys::Array::from(&field(&last, "stems")).get(0));
    let left: Float32Array = first.get(1).unchecked_into();
    let end = field(&last, "start").as_f64().unwrap_or(0.0) + left.length() as f64;
    assert_eq!(end, FAKE_FRAMES as f64, "the segments reach the song's end");
    assert_eq!(names_in("").await.as_deref(), Some(""), "no file written");
}

#[wasm_bindgen_test]
async fn a_storage_failure_ends_the_separation_with_unstored() {
    fresh().await;
    plant_folder(SESSION).await;
    // Six headers, then the six stems of the first segment: the 7th write
    // is the first segment's first stem.
    start_sync_spy(7);
    let mut net = FakeNetwork { fail_at: None };
    let ended = separate_song(&mut net, fake_demixer(), Some(SESSION), now_s()).await;
    let record = stop_sync_spy();
    assert_eq!(record, [["whole"; 6].join(" "), "refused".into()].join(" "));
    reply_to(kind::SEPARATE, ended);
    let run = fake_run();
    let first = run
        .segment_after
        .iter()
        .position(|s| *s)
        .expect("a segment");
    let mut want = chunk_replies(&run, first, "stored");
    want.push("unstored".into());
    assert_eq!(reply_types(), want, "no error, no done");
    let text = field(&sent("unstored")[0], "text")
        .as_string()
        .unwrap_or_default();
    assert!(text.starts_with("QuotaExceededError"), "{text}");
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some(""),
        "no stem file remains"
    );
}

/// Finding 1-b2 of review round 1: browser storage refuses a header
/// write of `finish`, after every segment is written.
#[wasm_bindgen_test]
async fn a_refused_header_at_the_end_sends_unstored() {
    fresh().await;
    plant_folder(SESSION).await;
    let run = fake_run();
    let segments =
        run.segment_after.iter().filter(|s| **s).count() + usize::from(run.segment_at_end);
    // Six headers at the open, six stems for each segment, then the third
    // of the six headers that `finish` writes.
    let refused = 6 + 6 * segments + 3;
    start_sync_spy(refused as u32);
    let mut net = FakeNetwork { fail_at: None };
    let ended = separate_song(&mut net, fake_demixer(), Some(SESSION), now_s()).await;
    let record = stop_sync_spy();
    assert_eq!(
        record.split(' ').collect::<Vec<_>>().len(),
        refused,
        "every write up to the refused one: {record}"
    );
    assert!(record.ends_with("refused"), "{record}");
    reply_to(kind::SEPARATE, ended);
    let mut want = whole_run(&run, "stored");
    want.pop();
    want.push("unstored".into());
    assert_eq!(reply_types(), want, "no done, no error");
    let text = field(&sent("unstored")[0], "text")
        .as_string()
        .unwrap_or_default();
    assert!(text.starts_with("QuotaExceededError"), "{text}");
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some(""),
        "no stem file remains"
    );
}

/// Finding 2-3 of review round 2: a stem file that cannot open ends the
/// separation with `unstored` alone, before any chunk.
#[wasm_bindgen_test]
async fn a_failed_open_sends_only_unstored() {
    fresh().await;
    let mut net = FakeNetwork { fail_at: None };
    let ended = separate_song(&mut net, fake_demixer(), Some(SESSION), now_s()).await;
    reply_to(kind::SEPARATE, ended);
    assert_eq!(reply_types(), ["unstored"], "no error, done or progress");
    let text = field(&sent("unstored")[0], "text")
        .as_string()
        .unwrap_or_default();
    assert!(
        text.starts_with("NotFoundError"),
        "the session is gone: {text}"
    );
    assert_eq!(names_in("").await.as_deref(), Some(""), "no file made");
}

#[wasm_bindgen_test]
async fn a_network_failure_removes_the_part_files() {
    fresh().await;
    plant_folder(SESSION).await;
    let run = fake_run();
    // After a stem file holds a segment.
    let fail_at = 1 + run
        .segment_after
        .iter()
        .position(|s| *s)
        .expect("a segment");
    let mut net = FakeNetwork {
        fail_at: Some(fail_at),
    };
    let ended = separate_song(&mut net, fake_demixer(), Some(SESSION), now_s()).await;
    reply_to(kind::SEPARATE, ended);
    let mut want = chunk_replies(&run, fail_at, "stored");
    want.push("error".into());
    assert!(want.contains(&"stored".to_string()), "{want:?}");
    assert_eq!(reply_types(), want, "no unstored, no done");
    let text = field(&sent("error")[0], "text")
        .as_string()
        .unwrap_or_default();
    assert_eq!(text, format!("the fake network failed at chunk {fail_at}"));
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some(""),
        "no stem file remains"
    );
}

/// MEM-1: proves each path in this crate's `clippy.toml` (see the `#[expect]`s).
#[wasm_bindgen_test]
fn mem1_lint_is_on_for_js_arrays() {
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let slice = [1.0_f32].to_vec();
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let bytes = js_sys::Uint8Array::from(&[2_u8][..]).to_vec();
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1 canary: clippy fails here if this path is off"
    )]
    let floats = js_sys::Float32Array::from(&[3.0_f32][..]).to_vec();
    assert_eq!((slice, bytes, floats), (vec![1.0], vec![2], vec![3.0]));
}

/// The mix file of the session [`SESSION`].
const MIX_PATH: &str = "sessions/1792640000000/mix.f32";

/// The bytes of a mix file of `mix`: interleaved 32-bit little-endian
/// floats.
fn mix_file(mix: &Stereo) -> Vec<u8> {
    mix[0]
        .iter()
        .zip(&mix[1])
        .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
        .flatten()
        .collect()
}

fn stored_mix(frames: usize) -> Mix {
    Mix::Stored {
        path: MIX_PATH.into(),
        frames,
    }
}

fn mix_arrays(mix: &Stereo) -> Mix {
    Mix::Arrays {
        left: Float32Array::from(&mix[0][..]),
        right: Float32Array::from(&mix[1][..]),
    }
}

/// Separates `mix` with the fake network and the fake chunk, as `run`
/// does with the GPU's.
async fn fake_separation(mix: Mix, session: Option<&str>) -> Result<(), String> {
    separate_mix(
        mix,
        session,
        FAKE_CHUNK,
        2,
        |_| FakeNetwork { fail_at: None },
        now_s(),
    )
    .await
}

/// The six `.part` files of [`SESSION`].
async fn part_files() -> Vec<Option<Vec<u8>>> {
    let mut out = Vec::new();
    for name in STEMS {
        out.push(read_at(&part_path(name)).await);
    }
    out
}

/// The stems of the `segment` replies since the last [`fresh`], put
/// together: for each stem, its left and right samples.
fn segment_stems() -> Vec<Stereo> {
    let mut out: Vec<Stereo> = (0..STEMS.len()).map(|_| [Vec::new(), Vec::new()]).collect();
    for seg in sent("segment") {
        let stems = js_sys::Array::from(&field(&seg, "stems"));
        for (s, whole) in out.iter_mut().enumerate() {
            let stem = js_sys::Array::from(&stems.get(s as u32));
            for c in 0..2 {
                let samples: Float32Array = stem.get(c as u32 + 1).unchecked_into();
                whole[c].extend(samples.to_vec());
            }
        }
    }
    out
}

#[wasm_bindgen_test]
async fn a_stored_mix_separates_as_the_mix_in_memory() {
    fresh().await;
    plant_at(MIX_PATH, &mix_file(&fake_mix())).await;
    fake_separation(stored_mix(FAKE_FRAMES), Some(SESSION))
        .await
        .expect("separated from the mix file");
    let run = fake_run();
    assert_eq!(reply_types(), whole_run(&run, "stored"));
    let from_file = part_files().await;
    assert!(from_file.iter().all(Option::is_some), "six stem files");

    fresh().await;
    plant_folder(SESSION).await;
    fake_separation(mix_arrays(&fake_mix()), Some(SESSION))
        .await
        .expect("separated from the arrays");
    assert!(
        part_files().await == from_file,
        "the mix file and the arrays give the same stem files"
    );

    fresh().await;
    plant_at(MIX_PATH, &mix_file(&fake_mix())).await;
    fake_separation(stored_mix(FAKE_FRAMES), None)
        .await
        .expect("separated from the mix file, with no session");
    assert_eq!(reply_types(), whole_run(&run, "segment"));
    let got = segment_stems();
    for (s, (g, w)) in got.iter().zip(&run.stems).enumerate() {
        for c in 0..2 {
            assert!(
                g[c].len() == w[c].len()
                    && g[c]
                        .iter()
                        .zip(&w[c])
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                "stem {s} channel {c}: the segments hold the stems of the mix in memory"
            );
        }
    }
}

/// The peak of the Rust heap above its live bytes at the start, during a
/// separation of `mix` into [`SESSION`] with the fake network.
async fn separation_peak(mix: Mix) -> usize {
    SENT.with_borrow_mut(Vec::clear);
    let start = LIVE.get();
    PEAK.set(start);
    fake_separation(mix, Some(SESSION))
        .await
        .expect("separated");
    PEAK.get() - start
}

#[wasm_bindgen_test]
async fn a_stored_mix_keeps_the_worker_memory_flat() {
    let short = 5_000;
    let long = 30_000;
    // One span: at most a chunk of stereo `f32` frames.
    let span = 8 * FAKE_CHUNK;
    let chunks = |frames| Demixer::new(mix_of(frames), FAKE_CHUNK, 2, STEMS.len()).num_chunks();
    assert!(chunks(long) >= 30, "the long mix has about 30 chunks");

    fresh().await;
    plant_at(MIX_PATH, &mix_file(&mix_of(short))).await;
    let short_peak = separation_peak(stored_mix(short)).await;

    fresh().await;
    plant_at(MIX_PATH, &mix_file(&mix_of(long))).await;
    start_read_spy();
    let long_peak = separation_peak(stored_mix(long)).await;
    let reads = stop_read_spy();
    assert_eq!(reply_types().last().map(String::as_str), Some("done"));
    assert!(
        long_peak.abs_diff(short_peak) < span,
        "the peak does not grow with the song: {short_peak} bytes for {short} frames, {long_peak} for {long}"
    );
    assert_eq!(
        reads.len(),
        chunks(long),
        "one read of the mix file for each chunk"
    );
    assert!(
        reads.iter().all(|&bytes| bytes <= span as f64),
        "each read is at most one span: {reads:?}"
    );

    // The control: the same long mix as arrays is copied into the heap.
    fresh().await;
    plant_folder(SESSION).await;
    let arrays = mix_arrays(&mix_of(long));
    let arrays_peak = separation_peak(arrays).await;
    assert!(
        arrays_peak > short_peak + span,
        "the arrays of the long mix raise the peak: {arrays_peak} bytes, against {short_peak}"
    );
}

#[wasm_bindgen_test]
async fn a_missing_mix_file_ends_with_unstored() {
    fresh().await;
    plant_folder(SESSION).await;
    let ended = fake_separation(stored_mix(FAKE_FRAMES), Some(SESSION)).await;
    reply_to(kind::SEPARATE, ended);
    assert_eq!(reply_types(), ["unstored"], "no error, done or progress");
    let text = field(&sent("unstored")[0], "text")
        .as_string()
        .unwrap_or_default();
    assert!(text.contains("NotFoundError"), "{text}");
    assert_eq!(
        names_in(SESSION).await.as_deref(),
        Some(""),
        "no stem file is made"
    );
}

#[wasm_bindgen_test]
async fn a_mix_file_of_the_wrong_size_ends_with_unstored() {
    for frames in [FAKE_FRAMES - 1, FAKE_FRAMES + 1] {
        fresh().await;
        plant_at(MIX_PATH, &mix_file(&fake_mix())).await;
        let ended = fake_separation(stored_mix(frames), Some(SESSION)).await;
        reply_to(kind::SEPARATE, ended);
        assert_eq!(
            reply_types(),
            ["unstored"],
            "{frames}: no error, done or progress"
        );
        let text = field(&sent("unstored")[0], "text")
            .as_string()
            .unwrap_or_default();
        assert_eq!(
            text,
            format!(
                "the mix file holds {} bytes, not the {} bytes of {frames} frames",
                8 * FAKE_FRAMES,
                8 * frames
            )
        );
        assert_eq!(
            names_in(SESSION).await.as_deref(),
            Some("mix.f32"),
            "{frames}: no stem file is made"
        );
    }
}

/// A source of [`fake_mix`] that fails its read number `fail_at` (from
/// 1), as a mix file that the browser cannot read.
struct FailingMix {
    mix: Stereo,
    reads: usize,
    fail_at: usize,
}

impl MixCheck for FailingMix {
    async fn unchanged(&self) -> Result<(), String> {
        Ok(())
    }
}

impl MixSource for FailingMix {
    type Error = String;

    fn frames(&self) -> usize {
        self.mix.frames()
    }

    fn read(&mut self, start: usize, end: usize) -> Result<Stereo, String> {
        self.reads += 1;
        if self.reads == self.fail_at {
            return Err(format!(
                "NotReadableError: read {} refused by the test",
                self.reads
            ));
        }
        self.mix.read(start, end).map_err(|never| match never {})
    }
}

#[wasm_bindgen_test]
async fn a_failed_mix_read_ends_with_unstored() {
    fresh().await;
    plant_folder(SESSION).await;
    let source = FailingMix {
        mix: fake_mix(),
        reads: 0,
        fail_at: 2,
    };
    let d = Demixer::new(source, FAKE_CHUNK, 2, STEMS.len());
    let mut net = FakeNetwork { fail_at: None };
    let ended = separate_song(&mut net, d, Some(SESSION), now_s()).await;
    reply_to(kind::SEPARATE, ended);
    let mut want = chunk_replies(&fake_run(), 1, "stored");
    want.push("unstored".into());
    assert_eq!(reply_types(), want, "no error, no done");
    let text = field(&sent("unstored")[0], "text")
        .as_string()
        .unwrap_or_default();
    assert_eq!(text, "NotReadableError: read 2 refused by the test");
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some(""),
        "no stem file remains"
    );
}

/// Finding 2-2 of review round 2 of M4: a read of a mix file that changed
/// or went after the open does not give the old samples.  Chrome refuses
/// the read (NotReadableError).  Safari reads the bytes on disk now, at the
/// old size (`docs/references/browser-quirks.md`).  So the worker also
/// checks the file at the end (`a_mix_file_written_again_after_the_last_read_ends_with_unstored`).
#[wasm_bindgen_test]
async fn a_mix_file_changed_after_the_open_gives_no_old_samples() {
    let old = fake_mix();
    for change in ["shortened", "removed"] {
        fresh().await;
        plant_at(MIX_PATH, &mix_file(&old)).await;
        let mut source = StoredMix::open(MIX_PATH, FAKE_FRAMES)
            .await
            .expect("opened");
        if change == "shortened" {
            plant_at(MIX_PATH, &[0; 16]).await;
        } else {
            assert!(opfs_remove_at(MIX_PATH).await, "removed");
        }
        match source.read(0, FAKE_CHUNK) {
            Err(error) => assert!(
                error.starts_with("the mix file could not be read: "),
                "{change}: {error}"
            ),
            Ok(got) => assert!(
                got[0] != old[0][..FAKE_CHUNK],
                "{change}: the read gave the old samples"
            ),
        }
    }
}

/// Finding 3-3 of review round 3 of M4: a mix file written again shorter
/// during the separation ends it with `unstored` (D6), in Chrome and in
/// Safari.  A read fails, so this case does not reach the end check;
/// `a_mix_file_written_again_after_the_last_read_ends_with_unstored` does.
#[wasm_bindgen_test]
async fn a_mix_file_changed_during_the_separation_ends_with_unstored() {
    fresh().await;
    plant_at(MIX_PATH, &mix_file(&fake_mix())).await;
    let source = StoredMix::open(MIX_PATH, FAKE_FRAMES)
        .await
        .expect("opened");
    plant_at(MIX_PATH, &[0; 16]).await;
    let d = Demixer::new(source, FAKE_CHUNK, 2, STEMS.len());
    let mut net = FakeNetwork { fail_at: None };
    let ended = separate_song(&mut net, d, Some(SESSION), now_s()).await;
    reply_to(kind::SEPARATE, ended);
    assert_unstored_alone().await;
}

/// Asserts that the separation's replies end with one `unstored`, with
/// no `done` and no `error`, and that no stem file remains.
async fn assert_unstored_alone() {
    let types = reply_types();
    assert_eq!(
        types.last().map(String::as_str),
        Some("unstored"),
        "{types:?}"
    );
    assert!(
        !types.iter().any(|t| t == "done" || t == "error"),
        "{types:?}"
    );
    assert_eq!(types.iter().filter(|t| *t == "unstored").count(), 1);
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some(""),
        "no stem file remains"
    );
}

/// The fake network, which writes the mix file again at its size during
/// chunk `last`, after the separation's last read of it.
struct RewritingNetwork {
    last: usize,
    calls: usize,
}

impl Network for RewritingNetwork {
    async fn stems(&mut self, _index: usize, x: &Stereo) -> Result<Vec<Stereo>, String> {
        self.calls += 1;
        if self.calls == self.last {
            // Over a second, so that Safari's `lastModified`, in whole
            // seconds, changes.
            sleep(1100).await;
            let other: Stereo = fake_mix().map(|c| c.iter().map(|v| 0.5 * v).collect());
            plant_at(MIX_PATH, &mix_file(&other)).await;
        }
        Ok(fake_stems_of(x))
    }

    fn so_far(&self) -> String {
        "the rewriting network".into()
    }

    fn finish(&self) {}
}

/// Finding 4-1 of review round 4 of M4: the check after the last chunk
/// (D6) sees a mix file written again at its size after the last read.
/// The separation then ends with `unstored`, in Chrome and in Safari.
#[wasm_bindgen_test]
async fn a_mix_file_written_again_after_the_last_read_ends_with_unstored() {
    fresh().await;
    plant_at(MIX_PATH, &mix_file(&fake_mix())).await;
    let source = StoredMix::open(MIX_PATH, FAKE_FRAMES)
        .await
        .expect("opened");
    let d = Demixer::new(source, FAKE_CHUNK, 2, STEMS.len());
    let mut net = RewritingNetwork {
        last: d.num_chunks(),
        calls: 0,
    };
    let ended = separate_song(&mut net, d, Some(SESSION), now_s()).await;
    reply_to(kind::SEPARATE, ended);
    assert_eq!(net.calls, net.last, "every chunk ran");
    let unstored = sent("unstored");
    let text = unstored
        .first()
        .map(|reply| field(reply, "text").as_string().unwrap_or_default())
        .unwrap_or_default();
    assert!(
        text.starts_with("the mix file changed during the separation"),
        "the end check: {text}"
    );
    assert_unstored_alone().await;
}

/// Finding 3-3: the check after the last chunk sees a mix file written
/// again at its size, after the last read.  It waits over a second, so
/// that Safari's `lastModified`, in whole seconds, changes.
#[wasm_bindgen_test]
async fn the_end_check_sees_a_mix_file_written_again_at_its_size() {
    fresh().await;
    plant_at(MIX_PATH, &mix_file(&fake_mix())).await;
    let source = StoredMix::open(MIX_PATH, FAKE_FRAMES)
        .await
        .expect("opened");
    assert_eq!(source.unchanged().await, Ok(()), "the control");
    sleep(1100).await;
    let other: Stereo = fake_mix().map(|c| c.iter().map(|v| 0.5 * v).collect());
    plant_at(MIX_PATH, &mix_file(&other)).await;
    let error = source.unchanged().await.expect_err("changed");
    assert!(
        error.starts_with("the mix file changed during the separation: 80000 bytes"),
        "{error}"
    );
}

/// Finding 2-2: a read that gives fewer bytes than its span is an error,
/// and the separation ends with `unstored`.
#[wasm_bindgen_test]
async fn a_short_read_of_the_mix_is_an_error() {
    let bytes = mix_file(&fake_mix());
    let mut short = StoredMix::unchecked(blob(&bytes[..8 * 100]), FAKE_FRAMES);
    assert_eq!(
        short.read(50, 150).expect_err("short"),
        "the mix file gave 400 bytes of frames 50 to 150, not 800"
    );
    fresh().await;
    plant_folder(SESSION).await;
    let source = StoredMix::unchecked(blob(&bytes[..8 * 100]), FAKE_FRAMES);
    let d = Demixer::new(source, FAKE_CHUNK, 2, STEMS.len());
    let mut net = FakeNetwork { fail_at: None };
    let ended = separate_song(&mut net, d, Some(SESSION), now_s()).await;
    reply_to(kind::SEPARATE, ended);
    assert_eq!(
        reply_types(),
        ["progress", "unstored"],
        "the first read fails"
    );
    assert_eq!(
        names_in(&format!("{SESSION}/stems")).await.as_deref(),
        Some(""),
        "no stem file remains"
    );
}

/// MEM-1: `Gpu::read` copies a read-back into WebAssembly memory once.
/// wgpu's web `BufferView` copies the whole mapping into wasm memory when
/// it is read as a slice, so a `to_vec` of that slice made a second copy.
/// `Job::mask` reads one mask of a chunk (18.9 MB at the default chunk)
/// this way, six times for each chunk.  A browser with no GPU adapter
/// (CI's headless Chrome can have none) logs a skip (TD-8).  Any other
/// error, such as an adapter whose device does not open, fails the test.
#[wasm_bindgen_test]
async fn a_read_back_is_copied_into_wasm_memory_once() {
    let gpu = match Gpu::new(wgpu::PowerPreference::default()).await {
        Ok(gpu) => gpu,
        Err(error) => match error.strip_prefix("no GPU adapter: ") {
            Some(e) => {
                console_log!("skipped: no GPU adapter: {e}");
                return;
            }
            None => panic!("the GPU did not open: {error}"),
        },
    };
    let len = 1 << 20;
    let data: Vec<f32> = (0..len).map(|i| i as f32).collect();
    let buffer = gpu.upload("read-back test", &data);
    let bytes = 4 * len;
    let start = LIVE.get();
    PEAK.set(start);
    let back = gpu.read(&buffer, 0, len).await.expect("read back");
    let grown = PEAK.get() - start;
    assert!(back == data, "the read-back holds the uploaded floats");
    assert!(
        grown <= bytes + bytes / 8,
        "a read-back of {bytes} bytes grows the heap by one copy, not {grown} bytes"
    );
}

/// Decision 0022 in a browser: an f16 variant that the browser rejects
/// (a planted WGSL error) fails the start-up check with one line for each
/// tile that names it, `f16_ok` stays false, and the default loads f32.
/// The error scopes catch the errors, so none reaches the device's error
/// sink, where it could fail a later check or the model upload.  A browser
/// with no GPU adapter logs a skip (TD-8).
#[wasm_bindgen_test]
async fn an_f16_variant_the_browser_rejects_keeps_f32_weights() {
    let gpu = match Gpu::new(wgpu::PowerPreference::default()).await {
        Ok(gpu) => gpu,
        Err(error) => match error.strip_prefix("no GPU adapter: ") {
            Some(e) => {
                console_log!("skipped: no GPU adapter: {e}");
                return;
            }
            None => panic!("the GPU did not open: {error}"),
        },
    };
    assert!(gpu.has_f16_unpack(), "WebGPU reports every downlevel flag");
    gpu.break_f16_variants();
    let problems = gpu.check_kernels().await.expect("the f32 kernels pass");
    gpu.done().await;
    assert_eq!(problems.len(), roformer_gpu::TILES.len(), "{problems:#?}");
    for p in &problems {
        assert!(
            p.contains("does not compile on this GPU")
                && p.contains("pars")
                && p.ends_with("; weights stay f32"),
            "{p}"
        );
    }
    assert!(!gpu.f16_ok.get());
    let (format, note) = roformer_gpu::weight_format(None, gpu.downlevel_flags(), gpu.f16_ok.get());
    assert_eq!(format, WeightFormat::F32);
    assert!(note.is_some_and(|n| n.contains("start-up check")));
    // The f32 kernels still run, with no error, as a separation would.
    let (m, n, k) = (70, 66, 64);
    let a = gpu.upload("a", &vec![0.5; m * k]);
    let b = gpu.upload_weight("b", &vec![0.25; n * k]);
    let c = gpu.buffer("c", m * n);
    let mut r = gpu.rec();
    r.matmul(&roformer_gpu::MatMul::new(
        (&a, roformer_gpu::View::rows(0, k)),
        (&b, roformer_gpu::View::t(0, k)),
        (&c, roformer_gpu::View::rows(0, n)),
        m,
        n,
        k,
    ));
    r.submit();
    let out = gpu.read(&c, 0, m * n).await.expect("read back");
    assert!(out.iter().all(|&x| x == 8.0), "{:?}", &out[..4]);
    let errors = gpu.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}
