//! Browser tests of the worker's storage and message handling, run in a
//! dedicated worker (`cargo run -p xtask -- web-test`).  No GPU is needed:
//! every case either fails before the GPU opens or never reaches it.
//!
//! The tests share one worker and its origin's private file system, so
//! each starts from an empty one, and they run one at a time.

use super::*;
use js_sys::{Object, Reflect};
use std::cell::RefCell;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

thread_local! {
    /// Every message the worker would have posted to the page.
    pub(super) static SENT: RefCell<Vec<JsValue>> = const { RefCell::new(Vec::new()) };
}

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
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
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
    fn blob_url(bytes: &[u8]) -> String;
    /// A `Blob` holding a copy of `bytes`, as a picked file is.
    fn blob(bytes: &[u8]) -> Blob;
    fn revoke(url: &str);
    async fn sleep(ms: u32);
    fn break_next_fetch();
    fn hide_gpu() -> js_sys::Function;
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
async fn a_picked_file_that_is_not_a_model_is_read_from_the_file() {
    fresh().await;
    plant(CACHE_NAME, &[7]).await;
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
    assert_eq!(listing().await, CACHE_NAME, "no partial file left");
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

#[wasm_bindgen_test]
async fn opfs_write_writes_more_than_one_slice() {
    fresh().await;
    let body: Vec<u8> = (0..(9u32 << 20)).map(|i| (i % 251) as u8).collect();
    for src in [
        blob(&body).into(),
        JsValue::from(Uint8Array::from(&body[..])),
    ] {
        assert_eq!(opfs_write("model", &src).await.as_bool(), Some(true));
        let got = cache_get("model").await.expect("no error");
        assert!(got.unchecked_into::<Uint8Array>().to_vec() == body);
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
    let got = download_to_opfs(&url, DOWNLOAD_NAME, progress.as_ref().unchecked_ref()).await;
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
    let got = download_to_opfs(&url, DOWNLOAD_NAME, progress.as_ref().unchecked_ref()).await;
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
    let samples = Float32Array::new_with_length(4);
    handle(message(&[
        ("type", "mp3".into()),
        ("name", "x.mp3".into()),
        ("left", samples.clone().into()),
        ("right", samples.into()),
        ("rate", 96_000.into()),
    ]))
    .await;
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
    assert_eq!(of, ["mp3", "separate"]);
    assert_eq!(
        field(&errors[1], "text").as_string().as_deref(),
        Some("load the model first")
    );
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
