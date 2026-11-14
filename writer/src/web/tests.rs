//! Browser tests of the write worker on OPFS, run in a dedicated worker
//! (`cargo run -p xtask -- web-test`).  The tests share one worker and its
//! origin's private file system, so each starts from an empty one.  The
//! helpers plant files through sync access handles, so a test run leaves
//! no swap file in Safari (TD-103).

#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use super::*;
use stem_protocol::write::kind;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

thread_local! {
    /// Every message the worker would have posted to the page.
    pub(super) static SENT: RefCell<Vec<JsValue>> = const { RefCell::new(Vec::new()) };
}

#[wasm_bindgen(inline_js = r#"
async function folder(path, create) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
export async function wipe() {
  const root = await navigator.storage.getDirectory();
  for await (const name of root.keys()) await root.removeEntry(name, { recursive: true });
}
export async function make_folder(path) { await folder(path, true); }
export async function plant(path, bytes) {
  const [d, n] = split(path);
  const h = await (await (await folder(d, true)).getFileHandle(n, { create: true })).createSyncAccessHandle();
  try { h.truncate(0); h.write(bytes.slice(), { at: 0 }); h.flush(); } finally { h.close(); }
}
// The bytes of the file at `path`, or null if there is none.
export async function read_at(path) {
  try {
    const [d, n] = split(path);
    const f = await (await (await folder(d, false)).getFileHandle(n)).getFile();
    return new Uint8Array(await f.arrayBuffer());
  } catch (e) { if (e.name === "NotFoundError") return null; throw e; }
}
export async function exists(path) {
  try { await folder(path, false); return true; }
  catch (e) { if (e.name === "NotFoundError") return false; throw e; }
}
export function blob_of(bytes) { return new Blob([bytes]); }
export function post(msg, transfer) { return structuredClone(msg, { transfer }); }
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
// Records what each FileSystemSyncAccessHandle.write gets: "whole", or
// "view" for a view into a larger buffer.
let record = null;
let real_write = null;
export function start_sync_spy() {
  if (real_write) throw new Error("a spy is already on the sync write");
  const proto = FileSystemSyncAccessHandle.prototype;
  real_write = proto.write;
  record = [];
  proto.write = function (data, options) {
    record.push(ArrayBuffer.isView(data) && data.byteOffset === 0 && data.byteLength === data.buffer.byteLength ? "whole" : "view");
    return real_write.call(this, data, options);
  };
}
export function stop_sync_spy() {
  FileSystemSyncAccessHandle.prototype.write = real_write;
  real_write = null;
  return record.join(" ");
}
// The next `createSyncAccessHandle` is refused, as when another writer
// holds the file; the returned function restores it.
export function refuse_sync_handle() {
  const proto = FileSystemFileHandle.prototype;
  const real = proto.createSyncAccessHandle;
  proto.createSyncAccessHandle = function () {
    proto.createSyncAccessHandle = real;
    return Promise.reject(new DOMException("refused by the test", "NoModificationAllowedError"));
  };
}
// The next `FileSystemSyncAccessHandle.prototype.write` throws, as a full
// disk refuses it, once the handle is open.
export function refuse_next_sync_write() {
  const proto = FileSystemSyncAccessHandle.prototype;
  const real = proto.write;
  proto.write = function () {
    proto.write = real;
    throw new DOMException("refused by the test", "QuotaExceededError");
  };
}
// "lloyd" at `at` in a buffer of 16 bytes, as a view of 5 bytes.
export function lloyd_in_a_larger_buffer(at) {
  const b = new Uint8Array(16);
  b.set(new TextEncoder().encode("lloyd"), at);
  return b.subarray(at, at + 5);
}
"#)]
extern "C" {
    async fn wipe();
    async fn make_folder(path: &str);
    async fn plant(path: &str, bytes: &Uint8Array);
    #[wasm_bindgen(js_name = read_at)]
    async fn read_at_js(path: &str) -> JsValue;
    #[wasm_bindgen(js_name = exists)]
    async fn exists_js(path: &str) -> JsValue;
    fn blob_of(bytes: &Uint8Array) -> web_sys::Blob;
    fn post(msg: &js_sys::Object, transfer: &js_sys::Array) -> JsValue;
    async fn sleep(ms: u32);
    fn start_sync_spy();
    fn stop_sync_spy() -> String;
    fn refuse_sync_handle();
    fn refuse_next_sync_write();
    fn lloyd_in_a_larger_buffer(at: u32) -> Uint8Array;
}

/// The bytes of the file at `path`, or `None`.
async fn read_at(path: &str) -> Option<Vec<u8>> {
    let got = read_at_js(path).await;
    (!got.is_null()).then(|| got.unchecked_into::<Uint8Array>().to_vec())
}

async fn exists(path: &str) -> bool {
    exists_js(path).await.as_bool() == Some(true)
}

/// `n` bytes, each its index plus `seed`, modulo 251.
fn numbered(n: usize, seed: usize) -> Vec<u8> {
    (0..n).map(|i| ((i + seed) % 251) as u8).collect()
}

/// An empty file system and no replies kept.
async fn fresh() {
    wipe().await;
    SENT.with_borrow_mut(Vec::clear);
}

/// Sends `requests` as the page does (a structured clone with the
/// transfers), and gives the replies, read back, once each has one.
async fn run(requests: Vec<WriteRequest>) -> Vec<(Outcome, Option<Vec<u8>>)> {
    let want = SENT.with_borrow(Vec::len) + requests.len();
    for request in &requests {
        let (msg, transfer) = request.to_js();
        enqueue(post(&msg, &transfer));
    }
    for _ in 0..2_000 {
        if SENT.with_borrow(Vec::len) >= want {
            break;
        }
        sleep(5).await;
    }
    SENT.with_borrow(|sent| {
        sent.iter()
            .skip(want - requests.len())
            .map(|msg| match WriteReply::from_js(msg) {
                Ok(WriteReply::Done { outcome, bytes }) => (outcome, bytes.map(|b| b.to_vec())),
                other => (Outcome::Failed(format!("not a done: {other:?}")), None),
            })
            .collect()
    })
}

fn file(within: &str, rel: &str, body: Body) -> WriteRequest {
    WriteRequest::File {
        within: within.into(),
        rel: rel.into(),
        body,
    }
}

fn block(within: &str, rel: &str, at: u64, bytes: &[u8]) -> WriteRequest {
    WriteRequest::Block {
        within: within.into(),
        rel: rel.into(),
        at,
        bytes: Uint8Array::from(bytes),
    }
}

/// A whole file from a typed array, and from a `Blob` of more than one
/// slice of 8 MiB, reads back as sent; each makes the folders of `rel`.
#[wasm_bindgen_test]
async fn a_whole_file_reads_back_from_an_array_and_from_a_large_blob() {
    fresh().await;
    make_folder("sessions/1").await;
    let small = numbered(300, 1);
    let large = numbered((9 << 20) + 7, 2);
    let replies = run(vec![
        file(
            "sessions/1",
            "stems/bass.wav.page.part",
            Body::Bytes(Uint8Array::from(&small[..])),
        ),
        file(
            "",
            "sessions/1/src/0.wav",
            Body::Blob(blob_of(&Uint8Array::from(&large[..]))),
        ),
    ])
    .await;
    assert_eq!(
        replies,
        [(Outcome::Written, None), (Outcome::Written, None)]
    );
    assert!(
        read_at("sessions/1/stems/bass.wav.page.part").await == Some(small),
        "the array"
    );
    let got = read_at("sessions/1/src/0.wav").await.unwrap_or_default();
    assert!(
        got == large,
        "the blob: {} of {} bytes",
        got.len(),
        large.len()
    );
}

/// MEM-5: each array that the worker writes is whole: a transferred
/// array, a view (copied out first), and each slice of a `Blob`.
#[wasm_bindgen_test]
async fn each_written_array_is_whole() {
    fresh().await;
    let large = numbered((9 << 20) + 7, 3);
    start_sync_spy();
    let replies = run(vec![
        file("", "a", Body::Bytes(Uint8Array::from(&[1u8, 2, 3][..]))),
        file("", "b", Body::Bytes(lloyd_in_a_larger_buffer(4))),
        file("", "c", Body::Blob(blob_of(&Uint8Array::from(&large[..])))),
    ])
    .await;
    let record = stop_sync_spy();
    assert!(
        replies.iter().all(|(o, _)| *o == Outcome::Written),
        "{replies:?}"
    );
    assert_eq!(
        record, "whole whole whole whole",
        "a, b, then c in 2 slices"
    );
    assert_eq!(read_at("b").await.as_deref(), Some(&b"lloyd"[..]));
}

/// A block lands at its offset, a later block keeps the bytes before it,
/// and a first block (at 0) cuts an old, longer file.  Each reply gives
/// its array back whole.
#[wasm_bindgen_test]
async fn blocks_land_at_their_offsets_and_the_first_cuts_an_old_file() {
    fresh().await;
    plant("saves/1-1.wav", &Uint8Array::from(&numbered(100, 4)[..])).await;
    let replies = run(vec![
        block("", "saves/1-1.wav", 0, b"lloyd"),
        block("", "saves/1-1.wav", 5, b"acme"),
        block("", "saves/1-1.wav", 3, b"YD"),
    ])
    .await;
    assert_eq!(
        replies,
        [
            (Outcome::Written, Some(b"lloyd".to_vec())),
            (Outcome::Written, Some(b"acme".to_vec())),
            (Outcome::Written, Some(b"YD".to_vec())),
        ]
    );
    assert_eq!(
        read_at("saves/1-1.wav").await.as_deref(),
        Some(&b"lloYDacme"[..])
    );
}

/// As `st_write`: a write into a folder `within` that is gone gives
/// `gone`, and makes no folder.
#[wasm_bindgen_test]
async fn a_write_into_a_gone_folder_is_gone_and_makes_none() {
    fresh().await;
    let replies = run(vec![
        file(
            "sessions/9",
            "stems/bass.wav.page.part",
            Body::Bytes(Uint8Array::from(&[1u8][..])),
        ),
        block("sessions/9", "mix.f32.part", 0, &[2]),
    ])
    .await;
    assert_eq!(
        replies,
        [(Outcome::Gone, None), (Outcome::Gone, Some(vec![2]))]
    );
    assert!(!exists("sessions/9").await, "no folder made");
}

/// A refused sync access handle gives the error's name, and leaves no
/// handle open: the next write of the same file succeeds.
#[wasm_bindgen_test]
async fn a_refused_handle_gives_its_error_and_leaves_none_open() {
    fresh().await;
    refuse_sync_handle();
    let refused = run(vec![block("", "saves/2-1.wav", 0, b"lloyd")]).await;
    let again = run(vec![block("", "saves/2-1.wav", 0, b"acme")]).await;
    assert_eq!(
        refused,
        [(
            Outcome::Failed("NoModificationAllowedError".into()),
            Some(b"lloyd".to_vec())
        )]
    );
    assert_eq!(again, [(Outcome::Written, Some(b"acme".to_vec()))]);
    assert_eq!(
        read_at("saves/2-1.wav").await.as_deref(),
        Some(&b"acme"[..])
    );
}

/// Finding 1-1 of review round 1: a write that fails after its handle
/// opened gives the error's name, and closes the handle: the next write
/// of the same file succeeds.
#[wasm_bindgen_test]
async fn a_write_that_fails_in_an_open_handle_closes_it() {
    fresh().await;
    refuse_next_sync_write();
    let refused = run(vec![block("", "saves/5-1.wav", 0, b"lloyd")]).await;
    let again = run(vec![block("", "saves/5-1.wav", 0, b"acme")]).await;
    assert_eq!(
        refused[0].0,
        Outcome::Failed("QuotaExceededError".into()),
        "the refused write"
    );
    assert_eq!(again[0].0, Outcome::Written, "the next write");
    assert_eq!(
        read_at("saves/5-1.wav").await.as_deref(),
        Some(&b"acme"[..])
    );
}

/// Two requests for one file, sent at once, are done one at a time, in
/// order.  Two handles of one file at once would refuse the second
/// (`NoModificationAllowedError`).
#[wasm_bindgen_test]
async fn two_requests_for_one_file_are_done_in_order() {
    fresh().await;
    let first = numbered(4 << 20, 5);
    let replies = run(vec![
        file(
            "",
            "sessions/2/src/0.wav",
            Body::Bytes(Uint8Array::from(&first[..])),
        ),
        file(
            "",
            "sessions/2/src/0.wav",
            Body::Bytes(Uint8Array::from(&b"lloyd"[..])),
        ),
    ])
    .await;
    assert_eq!(
        replies,
        [(Outcome::Written, None), (Outcome::Written, None)]
    );
    assert_eq!(
        read_at("sessions/2/src/0.wav").await.as_deref(),
        Some(&b"lloyd"[..])
    );
}

/// A request that cannot be read still ends, with a failure.
#[wasm_bindgen_test]
async fn a_request_that_cannot_be_read_ends_with_a_failure() {
    fresh().await;
    let want = SENT.with_borrow(Vec::len) + 1;
    let bad = js_sys::Object::new();
    js_sys::Reflect::set(&bad, &"type".into(), &kind::BLOCK.into()).unwrap();
    enqueue(bad.into());
    for _ in 0..200 {
        if SENT.with_borrow(Vec::len) >= want {
            break;
        }
        sleep(5).await;
    }
    let reply = SENT.with_borrow(|sent| sent.last().map(WriteReply::from_js));
    assert!(
        matches!(
            reply,
            Some(Ok(WriteReply::Done {
                outcome: Outcome::Failed(_),
                bytes: None
            }))
        ),
        "{reply:?}"
    );
}

fn end(within: &str, rel: &str, size: Option<u64>) -> WriteRequest {
    WriteRequest::End {
        within: within.into(),
        rel: rel.into(),
        size,
    }
}

/// A `write-end` with the file's size ends it; one whose block was lost
/// fails, with the bytes that the file holds.
#[wasm_bindgen_test]
async fn a_lost_block_makes_the_end_fail() {
    fresh().await;
    let replies = run(vec![
        block("", "saves/3-1.wav", 0, b"lloyd"),
        block("", "saves/3-1.wav", 5, b"acme"),
        end("", "saves/3-1.wav", Some(9)),
        block("", "saves/3-2.wav", 0, b"lloyd"),
        end("", "saves/3-2.wav", Some(9)),
    ])
    .await;
    let ends: Vec<&Outcome> = replies.iter().map(|(o, _)| o).collect();
    assert_eq!(ends[2], &Outcome::Written, "the whole file");
    assert_eq!(
        ends[4],
        &Outcome::Failed("the file holds 5 of 9 bytes".into()),
        "a lost block"
    );
}

/// A `write-end` with no size ends an abandoned write: the partial file
/// is gone.  In a gone folder it is `gone`.
#[wasm_bindgen_test]
async fn an_abandoned_write_leaves_no_partial_file() {
    fresh().await;
    make_folder("sessions/4").await;
    let replies = run(vec![
        block("sessions/4", "mix.f32.part", 0, b"lloyd"),
        end("sessions/4", "mix.f32.part", None),
        end("sessions/9", "mix.f32.part", None),
    ])
    .await;
    assert_eq!(replies[1].0, Outcome::Written);
    assert_eq!(replies[2].0, Outcome::Gone);
    assert_eq!(
        read_at("sessions/4/mix.f32.part").await,
        None,
        "no partial file"
    );
    assert!(!exists("sessions/9").await, "no folder made");
}

/// MEM-1: proves each path in this crate's `clippy.toml` (see the
/// `#[expect]`s).
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
