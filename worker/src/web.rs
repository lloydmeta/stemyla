use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use js_sys::{Float32Array, Uint8Array};
use roformer::demix::{Demixer, Segment};
use roformer::separate::{analyse, apply_masks};
use roformer::weights::{Source, Store, WeightError};
use roformer::{CHUNK, DEPTH, FEATURES, OVERLAP, STEMS};
use roformer_gpu::Gpu;
use roformer_gpu::net::{Job, Net, Step};
use stem_protocol::{
    self as protocol, ModelSource, Reply, Request, SegmentStem, Settings, ToWorker, kind,
};
use wasm_bindgen::prelude::*;
use web_sys::{Blob, DedicatedWorkerGlobalScope, MessageEvent};

const CACHE_NAME: &str = "bs_roformer_sw.safetensors";

/// Where "Download model" fetches the weights: a Hugging Face mirror of
/// the BS-RoFormer SW checkpoint converted to f16 safetensors (same source
/// checkpoint, SHA-256 24e7d35e..., as our own conversion).  The weights
/// carry no stated licence, so this app links to them rather than hosting
/// a copy.  `?model_url=` overrides it (the hash check is then skipped).
const MODEL_URL: &str =
    "https://huggingface.co/lumabeat/bs-roformer-sw/resolve/main/bs_roformer_sw.safetensors";
/// SHA-256 of that file as Hugging Face lists it.
const MODEL_SHA256: &str = "6c8303a829575d03f21562ea185be7b6b23e922052883dec1b9518ca00a920fc";
const DOWNLOAD_NAME: &str = "download.part";

/// Default upper bound on any per-step intermediate, in f32 elements
/// (32 MiB): one band per time-attention step at full chunk length (about
/// 4 GFLOP), so each GPU step stays short.  `?max_elems=` overrides it.
const BUDGET: usize = 8 << 20;

#[wasm_bindgen(inline_js = r#"
export async function cache_get(name) {
  const root = await navigator.storage.getDirectory();
  let fh;
  try { fh = await root.getFileHandle(name); }
  catch (e) { if (e.name === "NotFoundError") return null; throw e; }
  const f = await fh.getFile();
  return new Uint8Array(await f.arrayBuffer());
}
export async function cache_file(name) {
  const root = await navigator.storage.getDirectory();
  try { return await (await root.getFileHandle(name)).getFile(); }
  catch (e) { if (e.name === "NotFoundError") return null; throw e; }
}
export function blob_range(blob, start, end) {
  return new Uint8Array(new FileReaderSync().readAsArrayBuffer(blob.slice(start, end)));
}
let queue_tail = Promise.resolve();
export function queue_turn() {
  let release;
  const held = new Promise((r) => { release = r; });
  const turn = queue_tail.then(() => release);
  queue_tail = turn.then(() => held);
  return turn;
}
export function storage_lock(waiting) {
  const name = "stemyla-model-storage";
  return new Promise((granted, failed) => {
    const hold = () => new Promise((release) => granted(release));
    navigator.locks
      .request(name, { ifAvailable: true }, (lock) => {
        if (lock) return hold();
        waiting();
        navigator.locks.request(name, hold).catch(failed);
      })
      .catch(failed);
  });
}
export async function opfs_has(name) {
  try { const root = await navigator.storage.getDirectory(); await root.getFileHandle(name); return true; }
  catch (e) { return e.name !== "NotFoundError"; }
}
async function write_file(fh, body) {
  const w = await fh.createWritable();
  try {
    await body(w);
    await w.close();
  } catch (e) {
    try { await w.abort(); } catch (_) {}
    throw e;
  }
}
export async function cache_put(name, bytes) {
  const part = name + ".part";
  try {
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle(part, { create: true });
    await write_file(fh, (w) => w.write(bytes));
    if (await opfs_commit(part, name)) return true;
  } catch (e) {}
  await opfs_remove(part);
  return false;
}
export async function opfs_commit(from, to) {
  try {
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle(from);
    await fh.move(root, to);
    return true;
  } catch (e) { return false; }
}
export async function gpu_info(pref) {
  if (!navigator.gpu) return "no WebGPU";
  const a = await navigator.gpu.requestAdapter({ powerPreference: pref });
  if (!a) return "no WebGPU adapter";
  const i = a.info || {};
  return [i.vendor, i.architecture, i.description].filter((s) => s).join(" ") || "unknown adapter";
}
export async function blob_bytes(blob) {
  return new Uint8Array(await blob.arrayBuffer());
}
export async function download_to_opfs(url, name, progress) {
  const ctl = new AbortController();
  let idle;
  const arm = () => {
    clearTimeout(idle);
    idle = setTimeout(() => ctl.abort(new Error("no data from the server for 60 s")), 60000);
  };
  arm();
  try { return await download_with(url, name, progress, ctl.signal, arm); }
  finally { clearTimeout(idle); }
}
async function download_with(url, name, progress, signal, arm) {
  const r = await fetch(url, { signal });
  if (!r.ok) throw new Error(`HTTP ${r.status} from ${url}`);
  const total = Number(r.headers.get("content-length")) || 0;
  const root = await navigator.storage.getDirectory();
  const fh = await root.getFileHandle(name, { create: true });
  const reader = r.body.getReader();
  let done = 0, last = 0;
  try {
    await write_file(fh, async (w) => {
      for (;;) {
        const { value, done: end } = await reader.read();
        if (end) break;
        arm();
        await w.write(value);
        done += value.length;
        if (done - last >= 4e6) { progress(done, total); last = done; }
      }
    });
  } catch (e) {
    reader.cancel().catch(() => {});
    throw e;
  }
  progress(done, total);
  return done;
}
export async function opfs_remove(name) {
  try { const root = await navigator.storage.getDirectory(); await root.removeEntry(name); return true; }
  catch (e) { return e.name === "NotFoundError"; }
}
export async function sha256_hex(bytes) {
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(d, (b) => b.toString(16).padStart(2, "0")).join("");
}
export async function fetch_bytes(url) {
  try {
    const r = await fetch(url);
    if (!r.ok) return null;
    return new Uint8Array(await r.arrayBuffer());
  } catch (e) { return null; }
}
export function perf_now() { return performance.now(); }
"#)]
extern "C" {
    /// `performance.now()`: milliseconds at 100 µs resolution (5 µs when
    /// cross-origin isolated), against `Date.now()`'s whole milliseconds.
    fn perf_now() -> f64;
    /// The file's bytes, or null if there is no such file.
    #[wasm_bindgen(catch)]
    async fn cache_get(name: &str) -> Result<JsValue, JsValue>;
    /// The file as a `File` (read from storage only when sliced and read),
    /// or null if there is no such file.
    #[wasm_bindgen(catch)]
    async fn cache_file(name: &str) -> Result<JsValue, JsValue>;
    /// Bytes `start..end` of a `Blob`, read synchronously (workers only),
    /// so a tensor can be read while the model is built.
    #[wasm_bindgen(catch)]
    fn blob_range(blob: &Blob, start: f64, end: f64) -> Result<Uint8Array, JsValue>;
    /// Resolves, in the order asked, with a function that ends this turn.
    async fn queue_turn() -> JsValue;
    /// Resolves with a function that releases the lock, once no other tab
    /// of this site holds it.
    #[wasm_bindgen(catch)]
    async fn storage_lock(waiting: &js_sys::Function) -> Result<JsValue, JsValue>;
    /// Whether the file exists (or cannot be told apart from existing).
    async fn opfs_has(name: &str) -> JsValue;
    /// Stores `bytes`, a `Uint8Array` or a `Blob`.
    async fn cache_put(name: &str, bytes: &JsValue) -> JsValue;
    async fn gpu_info(pref: &str) -> JsValue;
    #[wasm_bindgen(catch)]
    async fn blob_bytes(blob: &JsValue) -> Result<JsValue, JsValue>;
    async fn fetch_bytes(url: &str) -> JsValue;
    #[wasm_bindgen(catch)]
    async fn download_to_opfs(
        url: &str,
        name: &str,
        progress: &js_sys::Function,
    ) -> Result<JsValue, JsValue>;
    /// Whether the file is gone (removed, or never there).
    async fn opfs_remove(name: &str) -> JsValue;
    async fn opfs_commit(from: &str, to: &str) -> JsValue;
    #[wasm_bindgen(catch)]
    async fn sha256_hex(bytes: &Uint8Array) -> Result<JsValue, JsValue>;
}

struct State {
    gpu: Option<Rc<Gpu>>,
    /// "high-performance" or "low-power" (`?gpu=integrated`).
    power: &'static str,
    net: Option<Rc<Net>>,
    /// Whether `ensure_kernels` has set this GPU's kernels.
    kernels_chosen: bool,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State { gpu: None, power: "high-performance", net: None, kernels_chosen: false }) };
}

fn scope() -> DedicatedWorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn now_s() -> f64 {
    perf_now() / 1000.0
}

fn send(reply: Reply) {
    let (o, transfer) = reply.to_js();
    // Under test the page is the test runner, so messages are kept instead.
    #[cfg(test)]
    {
        let _ = transfer;
        tests::SENT.with_borrow_mut(|s| s.push(o.into()));
    }
    #[cfg(not(test))]
    {
        // Not through `log`, which sends too.
        if let Err(e) = scope().post_message_with_transfer(&o, &transfer) {
            web_sys::console::error_2(&"stem-worker: a message to the page was lost:".into(), &e);
        }
    }
}

/// `ok`: whether the request succeeded.  `loaded` and `stored` say what
/// the worker holds afterwards, which a failed request leaves as it was
/// (a model loaded earlier stays loaded; a stored file kept after a GPU
/// failure can still be removed).
async fn post_model(ok: bool, text: &str) {
    let loaded = STATE.with_borrow(|s| s.net.is_some());
    let stored = opfs_has(CACHE_NAME).await.as_bool().unwrap_or(false);
    send(Reply::Model {
        ok,
        loaded,
        stored,
        text: text.into(),
    });
}

fn log(text: &str) {
    send(Reply::Log(text.into()));
}

/// A thrown JS value as text: "NotReadableError: ..." for an exception.
fn js_text(e: &JsValue) -> String {
    match e.dyn_ref::<js_sys::Error>() {
        Some(err) => format!("{}: {}", err.name(), err.message()),
        None => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    }
}

pub fn start() {
    console_error_panic_hook::set_once();
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| {
        wasm_bindgen_futures::spawn_local(handle(e.data()));
    });
    scope().set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
    send(Reply::Alive);
}

/// Adapter description and the limits the kernels are written to.
async fn gpu_text(gpu: &Gpu) -> String {
    let pref = STATE.with_borrow(|s| s.power);
    let name = gpu_info(pref).await.as_string().unwrap_or_default();
    let l = gpu.device.limits();
    let usable: Vec<&str> = roformer_gpu::TILES
        .iter()
        .zip(gpu.usable.get())
        .filter(|(_, u)| *u)
        .map(|(t, _)| t.name)
        .collect();
    let policy = gpu.policy.get().name();
    let attn = gpu.attention.get().name();
    let fused = if gpu.fused_ok.get() {
        "checked OK"
    } else {
        "not used"
    };
    format!(
        "{name}; own WGSL kernels at WebGPU default limits: invocations/workgroup {}, workgroup memory {} KiB, storage binding {} MiB; matmul {policy}, checked OK: {}; attention {attn}, fused attention {fused}",
        l.max_compute_invocations_per_workgroup,
        l.max_compute_workgroup_storage_size / 1024,
        l.max_storage_buffer_binding_size >> 20,
        usable.join(" "),
    )
}

/// The WebGPU device, created on first use.  `{gpu: "integrated"}` in the
/// first message asks the browser for the low-power (integrated) adapter.
async fn gpu(s: &Settings) -> Result<Rc<Gpu>, String> {
    if let Some(g) = STATE.with_borrow(|s| s.gpu.clone()) {
        return Ok(g);
    }
    let integrated = s.gpu.as_deref() == Some("integrated");
    let power = if integrated {
        wgpu::PowerPreference::LowPower
    } else {
        wgpu::PowerPreference::HighPerformance
    };
    // Recorded before anything names the adapter by it: `choose_kernels`
    // keys this GPU's stored choice on it, and with the default still in
    // place an integrated GPU took the discrete GPU's choice.
    STATE.with_borrow_mut(|st| {
        st.power = if integrated {
            "low-power"
        } else {
            "high-performance"
        };
    });
    // `?profile=1` opens the device with `timestamp-query` (when offered)
    // so every step reports GPU time per kernel; otherwise no features.
    let g = Rc::new(if s.profile {
        let g = Gpu::new_profiling(power).await?;
        if !g.profiling() {
            log("profile: this adapter has no timestamp-query; no GPU times");
        }
        g
    } else {
        Gpu::new(power).await?
    });
    // Every matmul kernel against the CPU on small products; one that
    // fails on this GPU is never used (the fallback failing is fatal).
    for p in g.check_kernels().await? {
        log(&p);
    }
    // The fused attention kernel against the CPU; if it fails, attention
    // runs split everywhere.
    if let Some(p) = g.check_attention().await {
        log(&p);
    }
    // Overrides from the URL; this GPU's own choice comes later, in
    // `ensure_kernels`.
    apply_policy(&g, s);
    apply_attention(&g, s);
    STATE.with_borrow_mut(|st| st.gpu = Some(g.clone()));
    Ok(g)
}

/// This GPU's kernel choice (kept per GPU, or measured once) unless the
/// page URL sets both parts, then any override from the URL; once per
/// worker.  Called once the GPU has done real work (after the model
/// upload, or for `?bench` without a model after its matmul timings)
/// rather than when the device opens, so calibration neither shares the
/// GPU with the upload nor times a GPU that has only just woken.
async fn ensure_kernels(g: &Gpu, s: &Settings) {
    if STATE.with_borrow_mut(|st| std::mem::replace(&mut st.kernels_chosen, true)) {
        return;
    }
    let set = |v: &Option<String>| v.as_deref().is_some_and(|v| v != "auto");
    if !(set(&s.mm) && set(&s.attn)) {
        choose_kernels(g).await;
    }
    apply_policy(g, s);
    apply_attention(g, s);
}

fn budget(s: &Settings) -> usize {
    s.max_elems.unwrap_or(BUDGET)
}

async fn handle(msg: JsValue) {
    let kind = protocol::kind_of(&msg);
    // Model and GPU requests wait for their turn before they are read, so
    // even a malformed one is answered in the order sent.
    let queued = [
        kind::INIT,
        kind::MODEL,
        kind::DOWNLOAD,
        kind::FORGET_MODEL,
        kind::SEPARATE,
        kind::SELFTEST,
        kind::BENCH,
    ]
    .contains(&kind.as_str());
    let _turn = if queued {
        Some(Held::turn().await)
    } else {
        None
    };
    let request = ToWorker::from_js(&msg);
    // The request holds what it needs; a transferred mix is then held only
    // by it, and freed once copied in.
    drop(msg);
    let result = match request {
        Err(bad) => Err(bad.text),
        Ok(ToWorker { request, settings }) => {
            if queued {
                apply_overrides(&settings);
            }
            serve(request, &settings).await
        }
    };
    if let Err(text) = result {
        send(Reply::Error { of: kind, text });
    }
}

async fn serve(request: Request, s: &Settings) -> Result<(), String> {
    match request {
        Request::Init => with_storage(init(s)).await,
        Request::Model(source) => with_storage(load_model(source, s)).await,
        Request::Download => with_storage(download(s)).await,
        Request::ForgetModel => with_storage(forget_model()).await,
        Request::Separate { left, right } => {
            let (gpu, net) = loaded()?;
            run(&gpu, &net, [left, right], s).await
        }
        Request::Selftest { base } => selftest(base, s).await,
        Request::Bench => bench(s).await,
        Request::Mp3 {
            name,
            left,
            right,
            rate,
        } => mp3(name, [left, right], rate).await,
    }
}

/// Kernel overrides from the page URL, for a GPU opened earlier.
fn apply_overrides(s: &Settings) {
    if let Some(g) = STATE.with_borrow(|st| st.gpu.clone()) {
        apply_policy(&g, s);
        apply_attention(&g, s);
    }
}

/// A turn or lock, given back when dropped.
///
/// Messages run concurrently: each message's handler interleaves with the
/// others at every await.  Two model tasks at once would share
/// `download.part`, hold two 350 MB copies or save a model the user has
/// just removed, and a model upload during a separation would share the
/// GPU's error sink with it.  So every message that touches the model or
/// the GPU waits for its turn, in the order the page sent them.
struct Held(js_sys::Function);

impl Held {
    async fn turn() -> Self {
        Self(queue_turn().await.unchecked_into())
    }

    /// The stored files are shared by every tab of this site.
    async fn storage() -> Option<Self> {
        let waiting = Closure::<dyn FnMut()>::new(|| {
            send(Reply::Status(
                "Waiting for another tab of this site to finish with the model…".into(),
            ));
        });
        let lock = storage_lock(waiting.as_ref().unchecked_ref()).await;
        drop(waiting);
        match lock {
            Ok(release) => Some(Self(release.unchecked_into())),
            Err(e) => {
                log(&format!(
                    "no lock on browser storage ({}); other tabs may interfere",
                    js_text(&e)
                ));
                None
            }
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = self.0.call0(&JsValue::NULL);
    }
}

/// Runs a model task holding the storage lock, shared by every tab.
async fn with_storage(
    task: impl std::future::Future<Output = Result<(), String>>,
) -> Result<(), String> {
    let _storage = Held::storage().await;
    task.await
}

async fn forget_model() -> Result<(), String> {
    STATE.with_borrow_mut(|s| s.net = None);
    let removed = opfs_remove(CACHE_NAME).await.as_bool().unwrap_or(false);
    remove_leftovers().await;
    opfs_remove(&format!("{KERNEL_CHOICE}.part")).await;
    post_model(
        false,
        if removed {
            "model removed from this browser"
        } else {
            "the model could not be removed from browser storage; use Reset"
        },
    )
    .await;
    Ok(())
}

/// Partial files from a download or save that never finished (a phone
/// may kill the tab during the upload that comes before the commit).
async fn remove_leftovers() {
    opfs_remove(DOWNLOAD_NAME).await;
    opfs_remove(&format!("{CACHE_NAME}.part")).await;
}

/// Why the weights could not be put on the GPU: the file itself, reading
/// it, or the GPU.
enum LoadError {
    Weights(WeightError),
    /// Why the browser could not read the file.
    Read(String),
    Gpu(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Weights(e) => write!(f, "{e}"),
            Self::Read(e) | Self::Gpu(e) => write!(f, "{e}"),
        }
    }
}

impl From<String> for LoadError {
    fn from(e: String) -> Self {
        Self::Gpu(e)
    }
}

impl From<WeightError> for LoadError {
    fn from(e: WeightError) -> Self {
        match e {
            WeightError::Read(why) => Self::Read(why),
            _ => Self::Weights(e),
        }
    }
}

/// The model file, outside WebAssembly memory: [`Store`] copies in the
/// header and then one tensor at a time, so the worker's memory (which
/// never shrinks) does not grow by the whole file.  A `Blob` (the stored
/// file, or the one the user picked) is read from disk in blocks, so not
/// even the JavaScript heap holds the whole file.
enum ModelFile {
    Bytes(Uint8Array),
    Blob(BlobFile),
}

/// A `Blob` read in aligned blocks of `block` bytes, keeping the `keep`
/// most recently used.  Each read from storage costs about 2 ms in Safari,
/// so one per tensor (1915 of them) added about 4 s to a load.  The model
/// asks for tensors in a different order from the file's (bands and layers
/// are interleaved in it, sorted as text), but within about 64 MB of it at
/// a time: 8 blocks of 8 MB read each byte of the file once, in 42 reads.
struct BlobFile {
    blob: Blob,
    block: usize,
    keep: usize,
    /// Block index and bytes, least recently used first.
    blocks: RefCell<Vec<(usize, Uint8Array)>>,
}

impl ModelFile {
    fn blob(blob: Blob) -> Self {
        Self::blob_in_blocks(blob, 8 << 20, 8)
    }

    fn blob_in_blocks(blob: Blob, block: usize, keep: usize) -> Self {
        Self::Blob(BlobFile {
            blob,
            block,
            keep,
            blocks: RefCell::new(Vec::new()),
        })
    }

    fn as_js(&self) -> &JsValue {
        match self {
            Self::Bytes(b) => b,
            Self::Blob(f) => &f.blob,
        }
    }
}

impl BlobFile {
    fn read(&self, start: usize, end: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(end - start);
        let mut blocks = self.blocks.borrow_mut();
        for b in start / self.block..end.div_ceil(self.block) {
            let from = b * self.block;
            let bytes = match blocks.iter().position(|(i, _)| *i == b) {
                Some(p) => blocks.remove(p).1,
                None => {
                    let to = (from + self.block).min(self.blob.size() as usize);
                    blob_range(&self.blob, from as f64, to as f64).map_err(|e| js_text(&e))?
                }
            };
            let part = bytes.subarray(
                (start.max(from) - from) as u32,
                (end.min(from + self.block) - from) as u32,
            );
            let n = out.len();
            out.resize(n + part.length() as usize, 0);
            part.copy_to(&mut out[n..]);
            blocks.push((b, bytes));
            if blocks.len() > self.keep {
                blocks.remove(0);
            }
        }
        Ok(out)
    }
}

impl Source for &ModelFile {
    fn size(&self) -> usize {
        match self {
            ModelFile::Bytes(b) => b.length() as usize,
            ModelFile::Blob(f) => f.blob.size() as usize,
        }
    }

    fn read(&self, start: usize, end: usize) -> Result<Cow<'_, [u8]>, String> {
        match self {
            ModelFile::Bytes(b) => Ok(Cow::Owned(b.subarray(start as u32, end as u32).to_vec())),
            ModelFile::Blob(f) => f.read(start, end).map(Cow::Owned),
        }
    }
}

/// Uploads the weights; returns the seconds taken and a description of
/// the GPU.
async fn build(file: ModelFile, s: &Settings) -> Result<(f64, String), LoadError> {
    let t0 = now_s();
    // Parsed before the GPU opens: a file that is not safetensors at all is
    // then reported (and an unusable stored copy removed) even on a device
    // without WebGPU, instead of being kept behind a GPU error.
    Store::from_source(&file)?;
    let g = gpu(s).await?;
    let net = Net::load_store(&g, Store::from_source(&file)?, budget(s))?;
    // Its cached blocks, or a checked download's bytes, are not needed for
    // the GPU checks and kernel timing that follow.
    drop(file);
    g.done().await;
    if let Some(e) = g.take_errors().into_iter().next() {
        return Err(LoadError::Gpu(format!(
            "the GPU rejected the model upload: {e}"
        )));
    }
    ensure_kernels(&g, s).await;
    log(&memory_text(&net, None));
    STATE.with_borrow_mut(|s| s.net = Some(Rc::new(net)));
    Ok((now_s() - t0, gpu_text(&g).await))
}

async fn init(s: &Settings) -> Result<(), String> {
    remove_leftovers().await;
    let unreadable = |e: &str| {
        format!(
            "could not read the model in browser storage ({e}); it was kept: reload the page to try again, or remove it"
        )
    };
    let cached = match cache_file(CACHE_NAME).await {
        Ok(cached) => cached,
        Err(e) => {
            post_model(false, &unreadable(&js_text(&e))).await;
            return Ok(());
        }
    };
    if cached.is_null() {
        post_model(false, "no model cached in this browser").await;
        return Ok(());
    }
    let file = ModelFile::blob(cached.unchecked_into());
    let size = (&file).size();
    match build(file, s).await {
        Ok((secs, gpu)) => {
            post_model(
                true,
                &format!("model loaded from browser storage in {secs:.1} s ({gpu})"),
            )
            .await
        }
        // A copy that was never fully written (left by an older version of
        // this page, or by the browser stopping mid-write) would otherwise
        // fail every start until the browser storage is cleared by hand.
        Err(LoadError::Weights(e)) => {
            let removed = opfs_remove(CACHE_NAME).await.as_bool().unwrap_or(false);
            let fate = if removed {
                "has been removed; download it again"
            } else {
                "could not be removed; use Reset, then download it again"
            };
            post_model(
                false,
                &format!(
                    "the model in browser storage is unusable ({} bytes, {e}) and {fate}",
                    size
                ),
            )
            .await;
        }
        Err(LoadError::Read(e)) => post_model(false, &unreadable(&e)).await,
        Err(e @ LoadError::Gpu(_)) => {
            post_model(
                false,
                &format!("the model in browser storage could not be loaded, and was kept: {e}"),
            )
            .await
        }
    }
    Ok(())
}

async fn load_model(source: Option<ModelSource>, s: &Settings) -> Result<(), String> {
    let Some(file) = source.map(|source| match source {
        ModelSource::File(f) => ModelFile::blob(f),
        ModelSource::Bytes(b) => ModelFile::Bytes(b),
    }) else {
        post_model(
            false,
            "could not read the model file: model message without file or bytes",
        )
        .await;
        return Ok(());
    };
    // A handle, not a copy: the picked file, or the bytes sent.
    let to_store = file.as_js().clone();
    match build(file, s).await {
        Ok((secs, gpu)) => {
            let cached = cache_put(CACHE_NAME, &to_store)
                .await
                .as_bool()
                .unwrap_or(false);
            let note = if cached {
                "saved in browser storage"
            } else {
                "could not be saved in browser storage"
            };
            post_model(
                true,
                &format!("model loaded in {secs:.1} s, {note} ({gpu})"),
            )
            .await;
            Ok(())
        }
        Err(e @ LoadError::Weights(_)) => {
            post_model(false, &format!("not a BS-RoFormer SW model: {e}")).await;
            Ok(())
        }
        Err(e @ LoadError::Read(_)) => {
            post_model(false, &format!("could not read the model file: {e}")).await;
            Ok(())
        }
        Err(e @ LoadError::Gpu(_)) => {
            post_model(false, &format!("the model could not be loaded: {e}")).await;
            Ok(())
        }
    }
}

fn loaded() -> Result<(Rc<Gpu>, Rc<Net>), String> {
    STATE.with_borrow(|s| match (&s.gpu, &s.net) {
        (Some(g), Some(n)) => Ok((g.clone(), n.clone())),
        _ => Err("load the model first".to_string()),
    })
}

/// Keeps the GPU from being monopolised.  Every step is one short command
/// buffer (one sub-batch of one transformer, a few GFLOP), and the worker
/// waits for the GPU to finish it before submitting the next, so other
/// work, the screen included, gets the GPU in between.  If one step takes
/// longer than the limit (default 5 s, `?step_limit=` in the page URL), the
/// run stops instead of risking the operating system's GPU watchdog.  The
/// first step of each kind may take up to 60 s, since it also waits for
/// the browser to compile that step's kernels.  Any WebGPU error also
/// stops the run.
struct Pace {
    limit: f64,
    seen: HashSet<&'static str>,
    worst: f64,
    worst_what: String,
    steps: usize,
    /// Matmul work and wall time of the steps after the first of each
    /// kind (which include compiling), for a throughput figure.
    flops: f64,
    secs: f64,
    /// Wall time of every step, the first of each kind included.
    wall: f64,
}

impl Pace {
    fn new(s: &Settings) -> Self {
        let limit = s.step_limit.unwrap_or(5.0);
        Self {
            limit,
            seen: HashSet::new(),
            worst: 0.0,
            worst_what: String::new(),
            steps: 0,
            flops: 0.0,
            secs: 0.0,
            wall: 0.0,
        }
    }

    /// Submits `step`, waits for the GPU, and checks time and errors.
    async fn run(&mut self, gpu: &Gpu, job: &Job<'_>, step: &Step) -> Result<(), String> {
        let t0 = now_s();
        job.run(step);
        gpu.done().await;
        let dt = now_s() - t0;
        gpu.collect_profile().await?;
        let first = self.seen.insert(step.kind());
        self.steps += 1;
        self.wall += dt;
        if !first {
            self.flops += step.flops(job.frames);
            self.secs += dt;
        }
        if !first && dt > self.worst {
            self.worst = dt;
            self.worst_what = step.to_string();
        }
        let errors = gpu.take_errors();
        if !errors.is_empty() {
            return Err(format!(
                "the GPU rejected {} operation(s) during {step}, so the result would be wrong; first: {}",
                errors.len(),
                errors[0]
            ));
        }
        let limit = if first {
            self.limit.max(60.0)
        } else {
            self.limit
        };
        if dt > limit {
            return Err(format!(
                "one GPU step ({step}) took {dt:.1} s, over the {limit:.0} s limit; stopped so the computer stays responsive"
            ));
        }
        Ok(())
    }

    async fn all(&mut self, gpu: &Gpu, job: &Job<'_>, steps: &[Step]) -> Result<(), String> {
        for s in steps {
            self.run(gpu, job, s).await?;
        }
        Ok(())
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

/// Where the worker's memory is, for the console: on iOS the whole tab is
/// stopped at a fixed limit (2 GB on an iPhone 17 Pro), and GPU buffers
/// count towards it.  The GPU's live bytes are every buffer not yet
/// destroyed, so they stay flat from chunk to chunk of the same length
/// unless one leaks.
fn memory_text(net: &Net, job: Option<&Job>) -> String {
    let mb = |b: u64| b as f64 / 1e6;
    let job = job.map_or(String::new(), |j| {
        format!(", chunk buffers {:.0} MB", mb(j.bytes()))
    });
    let live = STATE
        .with_borrow(|s| s.gpu.as_ref().map(|g| g.live_bytes()))
        .map_or(String::new(), |b| {
            format!(", GPU buffers live {:.0} MB", mb(b))
        });
    format!(
        "memory: worker wasm {:.0} MB, model on the GPU {:.0} MB{job}{live}",
        wasm_mb(),
        mb(net.bytes)
    )
}

/// Progress notes for debugging, sent when the message has `debug: true`.
fn trace(on: bool, text: &str) {
    if on {
        log(text);
    }
}

async fn run(gpu: &Gpu, net: &Net, mix: [Float32Array; 2], s: &Settings) -> Result<(), String> {
    let debug = s.debug;
    let chunk = s.chunk.unwrap_or(CHUNK);
    let overlap = s.overlap.unwrap_or(OVERLAP);
    let t0 = now_s();
    // The page transferred these and nothing else holds them: dropped once
    // copied, so the browser can free them while the song is separated.
    let mix = mix.map(|c| c.to_vec());
    let mut d = Demixer::new(mix, chunk, overlap, STEMS.len());
    let song_len = d.output_len();
    // The report after the song covers this separation only.
    gpu.reset_profile().await?;
    let mut pace = Pace::new(s);
    let total = d.num_chunks();
    send(Reply::Progress {
        done: 0,
        total: total as u32,
        secs: 0.0,
    });
    let mut job: Option<Job> = None;
    while let Some(x) = d.next_input() {
        let tc = now_s();
        let (spec, feats) = analyse(&x);
        let frames = feats.len() / FEATURES;
        trace(
            debug,
            &format!("chunk {}: stft {:.2} s", d.chunk_index(), now_s() - tc),
        );
        if job.as_ref().is_none_or(|j| j.frames != frames) {
            job = Some(Job::new(gpu, net, frames));
            log(&format!(
                "separating {total} chunks; {}",
                memory_text(net, job.as_ref())
            ));
        }
        let j = job.as_ref().expect("job");
        j.set_input(&feats);
        let mut out = Vec::with_capacity(STEMS.len());
        for step in j.steps() {
            pace.run(gpu, j, &step).await?;
            if let Step::Mask { last: true, .. } = step {
                out.push(apply_masks(&spec, &j.mask().await?));
                trace(
                    debug,
                    &format!(
                        "chunk {}: mask {} at {:.2} s",
                        d.chunk_index(),
                        out.len(),
                        now_s() - tc
                    ),
                );
            }
        }
        d.accept(&out);
        drop(out);
        if let Some(seg) = d.take_ready() {
            post_segment(seg, song_len);
        }
        let done = d.chunk_index().min(total) as u32;
        log(&format!(
            "chunk {done}/{total}: {:.1} s; so far {}; {}",
            now_s() - tc,
            pace.summary(),
            memory_text(net, job.as_ref())
        ));
        send(Reply::Progress {
            done,
            total: total as u32,
            secs: now_s() - t0,
        });
    }
    if let Some(seg) = d.take_ready() {
        post_segment(seg, song_len);
    }
    log(&format!("separation: {}", pace.summary()));
    log_profile(gpu, &pace);
    send(Reply::Done { secs: now_s() - t0 });
    Ok(())
}

/// Sends finished samples of every stem to the page, which assembles the
/// song-length stems; the worker keeps at most a chunk of running sums
/// instead of six whole songs (see `Demixer::take_ready`).
fn post_segment(seg: Segment, song_len: usize) {
    let stems = STEMS
        .iter()
        .zip(&seg.stems)
        .map(|(name, [left, right])| SegmentStem {
            name: (*name).into(),
            left: Float32Array::from(left.as_slice()),
            right: Float32Array::from(right.as_slice()),
        })
        .collect();
    send(Reply::Segment {
        start: seg.start,
        len: song_len,
        stems,
    });
}

/// Signal-to-error ratio in dB.
fn snr(got: &[f32], want: &[f32]) -> f64 {
    let (mut s, mut e) = (0.0f64, 0.0f64);
    for (a, b) in got.iter().zip(want) {
        s += (*b as f64).powi(2);
        e += (*a as f64 - *b as f64).powi(2);
    }
    if got.len() != want.len() || !e.is_finite() {
        return f64::NEG_INFINITY;
    }
    if e == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (s / e).log10()
    }
}

/// One tensor from a reference file as f32, with its shape.
fn tensor_f32(st: &safetensors::SafeTensors, name: &str) -> Result<(Vec<usize>, Vec<f32>), String> {
    let v = st
        .tensor(name)
        .map_err(|_| format!("reference file lacks {name}"))?;
    let data = match v.dtype() {
        safetensors::Dtype::F32 => v
            .data()
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        safetensors::Dtype::F16 => v
            .data()
            .chunks_exact(2)
            .map(|c| half::f16::from_le_bytes([c[0], c[1]]).to_f32())
            .collect(),
        d => return Err(format!("{name}: unexpected dtype {d:?}")),
    };
    Ok((v.shape().to_vec(), data))
}

/// Compares GPU results with the CPU port's reference files
/// (`xtask roformer-selftest`) served under `base`, stage by stage, then one
/// full chunk.  Each line reports agreement in dB (higher is better; the
/// references are stored as f16, so about 60 dB is the ceiling and anything
/// under 30 dB is wrong) and any WebGPU errors seen during that stage.
async fn selftest(base: Option<String>, s: &Settings) -> Result<(), String> {
    let (gpu, net) = loaded()?;
    let base = base.unwrap_or_else(|| "/selftest/".into());
    log(&format!("selftest: GPU {}", gpu_text(&gpu).await));
    let before = gpu.take_errors();
    if !before.is_empty() {
        log(&format!(
            "selftest: {} WebGPU error(s) before the test, first: {}",
            before.len(),
            before[0]
        ));
    }
    let mut worst = f64::INFINITY;

    let stages = fetch_bytes(&format!("{base}stages.safetensors")).await;
    if stages.is_null() {
        return Err(format!(
            "no reference files at {base} (run xtask roformer-selftest and serve them with --mount /selftest=<dir>)"
        ));
    }
    let stages = stages.unchecked_into::<Uint8Array>().to_vec();
    let st = safetensors::SafeTensors::deserialize(&stages).map_err(|e| e.to_string())?;
    let (_, feats) = tensor_f32(&st, "features")?;
    let t0 = now_s();
    let job = Job::new(&gpu, &net, feats.len() / FEATURES);
    let mut pace = Pace::new(s);
    job.set_input(&feats);
    pace.run(&gpu, &job, &Step::Split).await?;
    let (_, want) = tensor_f32(&st, "band_split")?;
    report(&mut worst, "band split", snr(&job.hidden().await?, &want));
    for (i, input) in [
        (0usize, "band_split"),
        (1, "layer0"),
        (10, "layer9"),
        (11, "layer10"),
    ] {
        let (_, x) = tensor_f32(&st, input)?;
        let (_, want) = tensor_f32(&st, &format!("layer{i}"))?;
        job.set_hidden(&x);
        pace.all(&gpu, &job, &job.layer_steps(i)).await?;
        report(
            &mut worst,
            &format!("layer {i}"),
            snr(&job.hidden().await?, &want),
        );
    }
    let (_, x) = tensor_f32(&st, "layer11")?;
    let (_, want) = tensor_f32(&st, "final_norm")?;
    job.set_hidden(&x);
    pace.run(&gpu, &job, &Step::FinalNorm).await?;
    report(&mut worst, "final norm", snr(&job.normed().await?, &want));
    let (_, x) = tensor_f32(&st, "final_norm")?;
    for (s, name) in STEMS.iter().enumerate() {
        let (_, want) = tensor_f32(&st, &format!("mask{s}"))?;
        job.set_normed(&x);
        pace.all(&gpu, &job, &job.mask_steps(s)).await?;
        report(
            &mut worst,
            &format!("mask {name}"),
            snr(&job.mask().await?, &want),
        );
    }
    let (_, want) = tensor_f32(&st, "final_norm")?;
    job.set_input(&feats);
    pace.run(&gpu, &job, &Step::Split).await?;
    for i in 0..DEPTH {
        pace.all(&gpu, &job, &job.layer_steps(i)).await?;
    }
    pace.run(&gpu, &job, &Step::FinalNorm).await?;
    report(
        &mut worst,
        "all 12 layers chained",
        snr(&job.normed().await?, &want),
    );
    log(&format!(
        "selftest: stages took {:.1} s; longest GPU step {:.2} s ({})",
        now_s() - t0,
        pace.worst,
        pace.worst_what
    ));
    drop(job);

    // One full 13.35 s chunk; every step is paced, so it stops early (with
    // an error) rather than holding the GPU if this GPU is too slow.
    let stages_only = s.selftest.as_deref() == Some("stages");
    if stages_only {
        log("selftest: skipping the full chunk (selftest=stages)");
    } else if worst > 30.0 {
        let chunk = fetch_bytes(&format!("{base}chunk.safetensors")).await;
        if chunk.is_null() {
            log("selftest: no chunk.safetensors; skipping the full chunk");
        } else {
            let bytes = chunk.unchecked_into::<Uint8Array>().to_vec();
            let ct = safetensors::SafeTensors::deserialize(&bytes).map_err(|e| e.to_string())?;
            let (ashape, audio) = tensor_f32(&ct, "audio")?;
            let (_, want) = tensor_f32(&ct, "stems")?;
            let n = ashape[1];
            let t0 = now_s();
            gpu.reset_profile().await?;
            let (spec, feats) = analyse(&[audio[..n].to_vec(), audio[n..].to_vec()]);
            let job = Job::new(&gpu, &net, feats.len() / FEATURES);
            let mut pace = Pace::new(s);
            job.set_input(&feats);
            let mix_energy: f64 = audio.iter().map(|v| (*v as f64).powi(2)).sum();
            let mut s = 0;
            for step in job.steps() {
                pace.run(&gpu, &job, &step).await?;
                if let Step::Mask { last: true, .. } = step {
                    let out = apply_masks(&spec, &job.mask().await?);
                    let w = &want[s * 2 * n..(s + 1) * 2 * n];
                    let got: Vec<f32> = out[0].iter().chain(out[1].iter()).copied().collect();
                    let err: f64 = got
                        .iter()
                        .zip(w)
                        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                        .sum();
                    let db_mix = 10.0 * (mix_energy / err).log10();
                    report(
                        &mut worst,
                        &format!(
                            "full chunk {} (vs stem {:.1} dB, vs mix)",
                            STEMS[s],
                            snr(&got, w)
                        ),
                        db_mix,
                    );
                    s += 1;
                }
            }
            log(&format!(
                "selftest: full 13.35 s chunk took {:.1} s; {}",
                now_s() - t0,
                pace.summary()
            ));
            log_profile(&gpu, &pace);
        }
    } else {
        log("selftest: skipping the full chunk (stages failed)");
    }
    let ok = worst > 30.0;
    let text = if ok {
        format!("selftest passed: the GPU matches the CPU reference (worst stage {worst:.1} dB)")
    } else {
        format!("selftest FAILED: worst stage {worst:.1} dB; see the lines above")
    };
    log(&text);
    send(Reply::Selftest { ok, text });
    Ok(())
}

/// With `?profile=1`: GPU time per kernel since the last report, and how
/// much of the paced steps' wall time the GPU spent busy.
fn log_profile(gpu: &Gpu, pace: &Pace) {
    if !gpu.profiling() {
        return;
    }
    let totals = gpu.take_profile();
    let busy: f64 = totals.values().map(|v| v.1).sum();
    for l in roformer_gpu::profile_report(&totals) {
        log(&l);
    }
    log(&format!(
        "profile: GPU busy {busy:.2} s of {:.2} s of step wall time ({:.0}% waiting, submitting or between dispatches); one pass per dispatch while profiling",
        pace.wall,
        100.0 * (1.0 - busy / pace.wall.max(1e-9))
    ));
}

fn report(worst: &mut f64, name: &str, db: f64) {
    log(&format!("selftest: {name}: {db:.1} dB"));
    *worst = worst.min(db);
}

impl Pace {
    /// "N GPU steps, longest ..., about X TFLOP/s" for log lines.
    fn summary(&self) -> String {
        let rate = if self.secs > 0.0 {
            self.flops / self.secs / 1e12
        } else {
            0.0
        };
        format!(
            "{} GPU steps, longest {:.2} s ({}), {rate:.2} TFLOP/s on average (steps after the first of each kind)",
            self.steps, self.worst, self.worst_what
        )
    }
}

/// `?mm=` in the page URL: which matmul kernel to use (`auto` or a name
/// from `roformer_gpu::TILES`).
fn apply_policy(gpu: &Gpu, s: &Settings) {
    if let Some(p) = s.mm.as_deref().and_then(roformer_gpu::MatMulPolicy::parse) {
        gpu.policy.set(p);
    }
}

/// `?attn=` in the page URL: `auto` (this GPU's choice), `split`,
/// `fused`, or `time` / `freq` for fused attention on that axis only.
/// Fused attention is used only if it passed the start-up check.
fn apply_attention(gpu: &Gpu, s: &Settings) {
    let Some(name) = s.attn.as_deref() else {
        return;
    };
    if name == "auto" {
        return;
    }
    match roformer_gpu::AttentionPolicy::parse(name) {
        Some(p) if p == roformer_gpu::AttentionPolicy::SPLIT || gpu.fused_ok.get() => {
            gpu.attention.set(p)
        }
        Some(_) => {
            log("fused attention failed the start-up check on this GPU; using split attention")
        }
        None => log(&format!(
            "unknown attention setting {name} (auto, split, fused, time or freq)"
        )),
    }
}

/// Times every matmul kernel on the shapes of a full chunk (`?bench`).
async fn bench(s: &Settings) -> Result<(), String> {
    let g = gpu(s).await?;
    // Without a model the GPU has done nothing yet: choose its kernels
    // after the matmul timings below (which do not depend on the choice)
    // rather than on a GPU that has only just woken.
    let loaded = STATE.with_borrow(|st| st.net.is_some());
    if loaded {
        ensure_kernels(&g, s).await;
    }
    log(&format!("bench: GPU {}", gpu_text(&g).await));
    let t0 = now_s();
    let (warm, round) = if s.bench_quick {
        (0.0, 0.0)
    } else {
        (0.3, 0.2)
    };
    let rounds = s.bench_rounds.map_or(3, |r| r.max(1.0) as usize);
    let only = s
        .bench
        .as_deref()
        .and_then(roformer_gpu::bench::parse_kernels);
    let table =
        roformer_gpu::bench::measure(&g, warm, round, rounds, only.as_deref(), now_s, |l| log(&l))
            .await;
    for l in roformer_gpu::bench::summary(&table) {
        log(&l);
    }
    if !loaded {
        ensure_kernels(&g, s).await;
    }
    roformer_gpu::bench::measure_attention(&g, warm, round, rounds, now_s, |l| log(&l)).await;
    let text = format!(
        "bench finished in {:.0} s; results in the browser console",
        now_s() - t0
    );
    send(Reply::Bench(text));
    Ok(())
}

/// Downloads the model into browser storage with progress messages
/// (`{type: "download", done, total}`), checks its hash, then loads it like
/// a picked file.
async fn download(s: &Settings) -> Result<(), String> {
    let url = s.model_url.clone().unwrap_or_else(|| MODEL_URL.to_string());
    let progress = Closure::<dyn FnMut(f64, f64)>::new(|done: f64, total: f64| {
        send(Reply::Download { done, total });
    });
    let t0 = now_s();
    let fetched = download_to_opfs(&url, DOWNLOAD_NAME, progress.as_ref().unchecked_ref()).await;
    drop(progress);
    if let Err(e) = fetched {
        opfs_remove(DOWNLOAD_NAME).await;
        post_model(false, &format!("model download failed: {}", js_text(&e))).await;
        return Ok(());
    }
    let file = match verified_download(&url).await {
        Ok(file) => file,
        Err(e) => {
            opfs_remove(DOWNLOAD_NAME).await;
            post_model(false, &e).await;
            return Ok(());
        }
    };
    let secs = now_s() - t0;
    let built = build(file, s).await;
    match built {
        Ok((load, gpu)) => {
            // The verified download becomes the stored model by a rename:
            // no second 350 MB write, and no moment at which the stored
            // model exists but is not yet complete.
            let cached = opfs_commit(DOWNLOAD_NAME, CACHE_NAME)
                .await
                .as_bool()
                .unwrap_or(false);
            let note = if cached {
                "saved in browser storage"
            } else {
                opfs_remove(DOWNLOAD_NAME).await;
                "could not be saved in browser storage"
            };
            post_model(
                true,
                &format!(
                    "model downloaded in {secs:.0} s and loaded in {load:.1} s, {note} ({gpu})"
                ),
            )
            .await;
        }
        Err(e) => {
            opfs_remove(DOWNLOAD_NAME).await;
            let what = match e {
                LoadError::Weights(_) => "the downloaded file is not a usable BS-RoFormer SW model",
                LoadError::Read(_) | LoadError::Gpu(_) => {
                    "the downloaded model could not be loaded"
                }
            };
            post_model(false, &format!("{what}: {e}")).await;
        }
    }
    Ok(())
}

/// The downloaded file, checked against the published hash when it came
/// from [`MODEL_URL`].  WebCrypto hashes only whole buffers, so a checked
/// file is read into JavaScript, and the model is built from those very
/// bytes: Safari's `File` from storage reads whatever the file holds when
/// it is read, not what was hashed.  An unchecked one is read from storage
/// a tensor at a time.
async fn verified_download(url: &str) -> Result<ModelFile, String> {
    let unreadable = |e: JsValue| {
        format!(
            "model download failed: could not read the file ({})",
            js_text(&e)
        )
    };
    let part = cache_file(DOWNLOAD_NAME).await.map_err(unreadable)?;
    if part.is_null() {
        return Err("model download failed: the browser did not keep the file".into());
    }
    let file: Blob = part.unchecked_into();
    if url != MODEL_URL {
        return Ok(ModelFile::blob(file));
    }
    let bytes: Uint8Array = blob_bytes(&file)
        .await
        .map_err(unreadable)?
        .unchecked_into();
    let hash = sha256_hex(&bytes)
        .await
        .map_err(|e| format!("could not check the downloaded model ({})", js_text(&e)))?
        .as_string()
        .unwrap_or_default();
    if hash != MODEL_SHA256 {
        return Err(format!(
            "the downloaded model is not the expected file (SHA-256 {hash}); not used"
        ));
    }
    Ok(ModelFile::Bytes(bytes))
}

/// Encodes one stem as a 192 kbps MP3 (`{type: "mp3", name, left, right,
/// rate}`) and sends the file back as `{type: "mp3", name, bytes}`.  Runs
/// here, not on the page, because the encoder does most of its work in one
/// final call (seconds for a whole song).
async fn mp3(
    name: Option<String>,
    [left, right]: [Float32Array; 2],
    rate: Option<u32>,
) -> Result<(), String> {
    let name = name.unwrap_or_else(|| "stem.mp3".into());
    let rate = rate.unwrap_or(44_100);
    let (l, r) = (left.to_vec(), right.to_vec());
    let mut pcm = Vec::with_capacity(2 * l.len());
    for (a, b) in l.iter().zip(&r) {
        pcm.push(*a);
        pcm.push(*b);
    }
    let mut enc = rusty_mp3::Mp3Encoder::new(rusty_mp3::Mp3EncoderConfig {
        bitrate_kbps: 192,
        vbr_quality: None,
    });
    enc.push_pcm_f32(&pcm, 2, rate)
        .map_err(|e| format!("MP3 encoder: {e:?}"))?;
    enc.finish();
    let mut out = Vec::new();
    while let Ok(packet) = enc.next_packet() {
        out.extend_from_slice(&packet);
    }
    let bytes = Uint8Array::from(out.as_slice());
    send(Reply::Mp3 { name, bytes });
    Ok(())
}

const KERNEL_CHOICE: &str = "matmul_kernels";
/// Stored kernel choices kept per GPU, one per kernel set and calibration
/// version (the newest), as many as the builds kept side by side.
const KEEP_PER_GPU: usize = 3;

/// Sets this GPU's matmul kernels and attention path: the choice kept in
/// browser storage for this adapter and these kernels, or else several
/// seconds of `roformer_gpu::bench::calibrate`, whose result is then kept
/// (see `roformer_gpu::bench::store_choice` for which lines stay).
async fn choose_kernels(gpu: &Gpu) {
    let pref = STATE.with_borrow(|s| s.power);
    // The key's separators (tab, newline, `|`) must not occur in the name,
    // or its stored line would never match and would pile up.
    let adapter = gpu_info(pref)
        .await
        .as_string()
        .unwrap_or_default()
        .replace(['\t', '\n', '\r', '|'], " ");
    let key = format!(
        "{adapter}|{:016x}|{}",
        roformer_gpu::kernels_id(),
        roformer_gpu::bench::CALIBRATION
    );
    let stored = cache_get(KERNEL_CHOICE).await.unwrap_or(JsValue::NULL);
    let text = if stored.is_null() {
        String::new()
    } else {
        String::from_utf8_lossy(&stored.unchecked_into::<Uint8Array>().to_vec()).to_string()
    };
    for v in text
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(k, v)| (k == key).then_some(v))
    {
        if let Some(c) = roformer_gpu::bench::Choice::parse(v)
            && gpu.usable.get()[c.matmul.pick(1000, 1000)]
            && gpu.usable.get()[c.matmul.pick(10, 10)]
            && (c.attention == roformer_gpu::AttentionPolicy::SPLIT || gpu.fused_ok.get())
        {
            gpu.policy.set(c.matmul);
            gpu.attention.set(c.attention);
            return;
        }
    }
    log("choosing kernels for this GPU (several seconds, once)…");
    let c = roformer_gpu::bench::calibrate(gpu, now_s, |l| log(&l)).await;
    gpu.policy.set(c.matmul);
    gpu.attention.set(c.attention);
    // A WebGPU error while timing (out of memory with the weights
    // resident, say) makes the timings meaningless: use the choice for
    // now, keep nothing, and report it here rather than at the next step.
    if let Some(e) = gpu.take_errors().into_iter().next() {
        log(&format!(
            "kernel calibration hit a WebGPU error ({e}); this choice is not kept"
        ));
        return;
    }
    let text = roformer_gpu::bench::store_choice(&text, &adapter, &key, &c.name(), KEEP_PER_GPU);
    cache_put(KERNEL_CHOICE, &Uint8Array::from(text.as_bytes())).await;
}

#[cfg(test)]
mod tests;
