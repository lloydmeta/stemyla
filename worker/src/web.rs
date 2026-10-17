use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use js_sys::{Array, Float32Array, Object, Reflect, Uint8Array};
use roformer::demix::Demixer;
use roformer::separate::{analyse, apply_masks};
use roformer::{CHUNK, DEPTH, FEATURES, OVERLAP, STEMS};
use roformer_gpu::Gpu;
use roformer_gpu::net::{Job, Net, Step};
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

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
  try {
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle(name);
    const f = await fh.getFile();
    return new Uint8Array(await f.arrayBuffer());
  } catch (e) { return null; }
}
export async function cache_put(name, bytes) {
  try {
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle(name, { create: true });
    const w = await fh.createWritable();
    await w.write(bytes);
    await w.close();
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
  const r = await fetch(url);
  if (!r.ok) throw new Error(`HTTP ${r.status} from ${url}`);
  const total = Number(r.headers.get("content-length")) || 0;
  const root = await navigator.storage.getDirectory();
  const fh = await root.getFileHandle(name, { create: true });
  const w = await fh.createWritable();
  const reader = r.body.getReader();
  let done = 0, last = 0;
  for (;;) {
    const { value, done: end } = await reader.read();
    if (end) break;
    await w.write(value);
    done += value.length;
    if (done - last >= 4e6) { progress(done, total); last = done; }
  }
  await w.close();
  progress(done, total);
  return done;
}
export async function opfs_remove(name) {
  try { const root = await navigator.storage.getDirectory(); await root.removeEntry(name); return true; } catch (e) { return false; }
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
    async fn cache_get(name: &str) -> JsValue;
    async fn cache_put(name: &str, bytes: &Uint8Array) -> JsValue;
    async fn gpu_info(pref: &str) -> JsValue;
    async fn blob_bytes(blob: &JsValue) -> JsValue;
    async fn fetch_bytes(url: &str) -> JsValue;
    #[wasm_bindgen(catch)]
    async fn download_to_opfs(
        url: &str,
        name: &str,
        progress: &js_sys::Function,
    ) -> Result<JsValue, JsValue>;
    async fn opfs_remove(name: &str) -> JsValue;
    async fn sha256_hex(bytes: &Uint8Array) -> JsValue;
}

struct State {
    gpu: Option<Rc<Gpu>>,
    /// "high-performance" or "low-power" (`?gpu=integrated`).
    power: &'static str,
    net: Option<Rc<Net>>,
    busy: bool,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State { gpu: None, power: "high-performance", net: None, busy: false }) };
}

fn scope() -> DedicatedWorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn now_s() -> f64 {
    perf_now() / 1000.0
}

fn post(fields: &[(&str, JsValue)], transfer: &[JsValue]) {
    let o = Object::new();
    for (k, v) in fields {
        let _ = Reflect::set(&o, &JsValue::from_str(k), v);
    }
    let t: Array = transfer.iter().collect();
    let _ = scope().post_message_with_transfer(&o, &t);
}

fn post_model(ok: bool, text: &str) {
    post(
        &[
            ("type", "model".into()),
            ("ok", ok.into()),
            ("text", text.into()),
        ],
        &[],
    );
}

fn log(text: &str) {
    post(&[("type", "log".into()), ("text", text.into())], &[]);
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

pub fn start() {
    console_error_panic_hook::set_once();
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| {
        wasm_bindgen_futures::spawn_local(handle(e.data()));
    });
    scope().set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
    post(&[("type", "alive".into())], &[]);
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
async fn gpu(msg: &JsValue) -> Result<Rc<Gpu>, String> {
    if let Some(g) = STATE.with_borrow(|s| s.gpu.clone()) {
        return Ok(g);
    }
    let integrated = get(msg, "gpu").as_string().as_deref() == Some("integrated");
    let power = if integrated {
        wgpu::PowerPreference::LowPower
    } else {
        wgpu::PowerPreference::HighPerformance
    };
    // Recorded before anything names the adapter by it: `choose_kernels`
    // keys this GPU's stored choice on it, and with the default still in
    // place an integrated GPU took the discrete GPU's choice.
    STATE.with_borrow_mut(|s| {
        s.power = if integrated {
            "low-power"
        } else {
            "high-performance"
        };
    });
    // `?profile=1` opens the device with `timestamp-query` (when offered)
    // so every step reports GPU time per kernel; otherwise no features.
    let g = Rc::new(if get(msg, "profile").is_truthy() {
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
    // This GPU's kernel choice (kept per GPU, or measured once), unless
    // the page URL sets both parts; then any override from the URL.
    let set = |k: &str| get(msg, k).as_string().is_some_and(|v| v != "auto");
    if !(set("mm") && set("attn")) {
        choose_kernels(&g).await;
    }
    apply_policy(&g, msg);
    apply_attention(&g, msg);
    STATE.with_borrow_mut(|s| s.gpu = Some(g.clone()));
    Ok(g)
}

fn budget(msg: &JsValue) -> usize {
    get(msg, "max_elems")
        .as_f64()
        .map_or(BUDGET, |v| v as usize)
}

async fn handle(msg: JsValue) {
    let kind = get(&msg, "type").as_string().unwrap_or_default();
    if let Some(g) = STATE.with_borrow(|s| s.gpu.clone()) {
        apply_policy(&g, &msg);
        apply_attention(&g, &msg);
    }
    let result = match kind.as_str() {
        "init" => init(&msg).await,
        "model" => load_model(&msg).await,
        "separate" => separate(&msg).await,
        "selftest" => selftest(&msg).await,
        "bench" => bench(&msg).await,
        "download" => download(&msg).await,
        "mp3" => mp3(&msg).await,
        "forget_model" => {
            STATE.with_borrow_mut(|s| s.net = None);
            opfs_remove(CACHE_NAME).await;
            post_model(false, "model removed from this browser");
            Ok(())
        }
        other => Err(format!("unknown message {other}")),
    };
    if let Err(text) = result {
        post(&[("type", "error".into()), ("text", text.into())], &[]);
    }
}

/// Uploads the weights; returns the seconds taken and a description of
/// the GPU.
async fn build(bytes: &[u8], msg: &JsValue) -> Result<(f64, String), String> {
    let t0 = now_s();
    let g = gpu(msg).await?;
    let net = Net::load(&g, bytes, budget(msg)).map_err(|e| e.to_string())?;
    g.done().await;
    if let Some(e) = g.take_errors().into_iter().next() {
        return Err(format!("the GPU rejected the model upload: {e}"));
    }
    STATE.with_borrow_mut(|s| s.net = Some(Rc::new(net)));
    Ok((now_s() - t0, gpu_text(&g).await))
}

async fn init(msg: &JsValue) -> Result<(), String> {
    let cached = cache_get(CACHE_NAME).await;
    if cached.is_null() {
        post_model(false, "no model cached in this browser");
        return Ok(());
    }
    let bytes = cached.unchecked_into::<Uint8Array>().to_vec();
    let (secs, gpu) = build(&bytes, msg).await?;
    post_model(
        true,
        &format!("model loaded from browser storage in {secs:.1} s ({gpu})"),
    );
    Ok(())
}

async fn load_model(msg: &JsValue) -> Result<(), String> {
    let file = get(msg, "file");
    let arr: Uint8Array = if !file.is_undefined() {
        blob_bytes(&file).await.unchecked_into()
    } else {
        get(msg, "bytes")
            .dyn_into()
            .map_err(|_| "model message without file or bytes".to_string())?
    };
    let bytes = arr.to_vec();
    match build(&bytes, msg).await {
        Ok((secs, gpu)) => {
            let cached = cache_put(CACHE_NAME, &arr).await.as_bool().unwrap_or(false);
            let note = if cached {
                "saved in browser storage"
            } else {
                "could not be saved in browser storage"
            };
            post_model(
                true,
                &format!("model loaded in {secs:.1} s, {note} ({gpu})"),
            );
            Ok(())
        }
        Err(e) => {
            post_model(false, &format!("not a BS-RoFormer SW model: {e}"));
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

async fn separate(msg: &JsValue) -> Result<(), String> {
    let (gpu, net) = loaded()?;
    if STATE.with_borrow(|s| s.busy) {
        return Err("a separation is already running".into());
    }
    STATE.with_borrow_mut(|s| s.busy = true);
    let r = run(&gpu, &net, msg).await;
    STATE.with_borrow_mut(|s| s.busy = false);
    r
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
    fn new(msg: &JsValue) -> Self {
        let limit = get(msg, "step_limit").as_f64().unwrap_or(5.0);
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

/// Progress notes for debugging, sent when the message has `debug: true`.
fn trace(on: bool, text: &str) {
    if on {
        log(text);
    }
}

async fn run(gpu: &Gpu, net: &Net, msg: &JsValue) -> Result<(), String> {
    let debug = get(msg, "debug").is_truthy();
    let left: Float32Array = get(msg, "left")
        .dyn_into()
        .map_err(|_| "left must be a Float32Array")?;
    let right: Float32Array = get(msg, "right")
        .dyn_into()
        .map_err(|_| "right must be a Float32Array")?;
    let chunk = get(msg, "chunk").as_f64().map_or(CHUNK, |v| v as usize);
    let overlap = get(msg, "overlap").as_f64().map_or(OVERLAP, |v| v as usize);
    let t0 = now_s();
    let mut d = Demixer::new([left.to_vec(), right.to_vec()], chunk, overlap, STEMS.len());
    let mut pace = Pace::new(msg);
    let total = d.num_chunks();
    post(
        &[
            ("type", "progress".into()),
            ("done", 0.into()),
            ("total", (total as u32).into()),
            ("secs", 0.into()),
        ],
        &[],
    );
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
        let done = d.chunk_index().min(total) as u32;
        log(&format!(
            "chunk {done}/{total}: {:.1} s; so far {}",
            now_s() - tc,
            pace.summary()
        ));
        post(
            &[
                ("type", "progress".into()),
                ("done", done.into()),
                ("total", (total as u32).into()),
                ("secs", (now_s() - t0).into()),
            ],
            &[],
        );
    }
    for (name, stem) in STEMS.iter().zip(d.finish()) {
        let l = Float32Array::from(stem[0].as_slice());
        let r = Float32Array::from(stem[1].as_slice());
        post(
            &[
                ("type", "stem".into()),
                ("name", (*name).into()),
                ("left", l.clone().into()),
                ("right", r.clone().into()),
            ],
            &[l.buffer().into(), r.buffer().into()],
        );
    }
    log(&format!("separation: {}", pace.summary()));
    log_profile(gpu, &pace);
    post(
        &[("type", "done".into()), ("secs", (now_s() - t0).into())],
        &[],
    );
    Ok(())
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
async fn selftest(msg: &JsValue) -> Result<(), String> {
    let (gpu, net) = loaded()?;
    let base = get(msg, "base")
        .as_string()
        .unwrap_or_else(|| "/selftest/".into());
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
    let mut pace = Pace::new(msg);
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
    let stages_only = get(msg, "selftest").as_string().as_deref() == Some("stages");
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
            gpu.take_profile();
            let (spec, feats) = analyse(&[audio[..n].to_vec(), audio[n..].to_vec()]);
            let job = Job::new(&gpu, &net, feats.len() / FEATURES);
            let mut pace = Pace::new(msg);
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
    post(
        &[
            ("type", "selftest".into()),
            ("ok", ok.into()),
            ("text", text.into()),
        ],
        &[],
    );
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
fn apply_policy(gpu: &Gpu, msg: &JsValue) {
    if let Some(p) = get(msg, "mm")
        .as_string()
        .as_deref()
        .and_then(roformer_gpu::MatMulPolicy::parse)
    {
        gpu.policy.set(p);
    }
}

/// `?attn=` in the page URL: `auto` (this GPU's choice), `split`,
/// `fused`, or `time` / `freq` for fused attention on that axis only.
/// Fused attention is used only if it passed the start-up check.
fn apply_attention(gpu: &Gpu, msg: &JsValue) {
    let Some(name) = get(msg, "attn").as_string() else {
        return;
    };
    if name == "auto" {
        return;
    }
    match roformer_gpu::AttentionPolicy::parse(&name) {
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
async fn bench(msg: &JsValue) -> Result<(), String> {
    let g = gpu(msg).await?;
    log(&format!("bench: GPU {}", gpu_text(&g).await));
    let t0 = now_s();
    let (warm, round) = if get(msg, "bench_quick").is_truthy() {
        (0.0, 0.0)
    } else {
        (0.3, 0.2)
    };
    let rounds = get(msg, "bench_rounds")
        .as_f64()
        .map_or(3, |r| r.max(1.0) as usize);
    let only = get(msg, "bench")
        .as_string()
        .and_then(|s| roformer_gpu::bench::parse_kernels(&s));
    let table =
        roformer_gpu::bench::measure(&g, warm, round, rounds, only.as_deref(), now_s, |l| log(&l))
            .await;
    for l in roformer_gpu::bench::summary(&table) {
        log(&l);
    }
    roformer_gpu::bench::measure_attention(&g, warm, round, rounds, now_s, |l| log(&l)).await;
    let text = format!(
        "bench finished in {:.0} s; results in the browser console",
        now_s() - t0
    );
    post(&[("type", "bench".into()), ("text", text.into())], &[]);
    Ok(())
}

/// Downloads the model into browser storage with progress messages
/// (`{type: "download", done, total}`), checks its hash, then loads it like
/// a picked file.
async fn download(msg: &JsValue) -> Result<(), String> {
    let url = get(msg, "model_url")
        .as_string()
        .unwrap_or_else(|| MODEL_URL.to_string());
    let progress = Closure::<dyn FnMut(f64, f64)>::new(|done: f64, total: f64| {
        post(
            &[
                ("type", "download".into()),
                ("done", done.into()),
                ("total", total.into()),
            ],
            &[],
        );
    });
    let t0 = now_s();
    let fetched = download_to_opfs(&url, DOWNLOAD_NAME, progress.as_ref().unchecked_ref()).await;
    drop(progress);
    if let Err(e) = fetched {
        opfs_remove(DOWNLOAD_NAME).await;
        post_model(
            false,
            &format!(
                "model download failed: {}",
                e.as_string().unwrap_or_else(|| format!("{e:?}"))
            ),
        );
        return Ok(());
    }
    let part = cache_get(DOWNLOAD_NAME).await;
    opfs_remove(DOWNLOAD_NAME).await;
    if part.is_null() {
        post_model(
            false,
            "model download failed: the browser did not keep the file",
        );
        return Ok(());
    }
    let arr: Uint8Array = part.unchecked_into();
    if url == MODEL_URL {
        let hash = sha256_hex(&arr).await.as_string().unwrap_or_default();
        if hash != MODEL_SHA256 {
            post_model(
                false,
                &format!(
                    "the downloaded model is not the expected file (SHA-256 {hash}); not used"
                ),
            );
            return Ok(());
        }
    }
    let secs = now_s() - t0;
    match build(&arr.to_vec(), msg).await {
        Ok((load, gpu)) => {
            let cached = cache_put(CACHE_NAME, &arr).await.as_bool().unwrap_or(false);
            let note = if cached {
                "saved in browser storage"
            } else {
                "could not be saved in browser storage"
            };
            post_model(
                true,
                &format!(
                    "model downloaded in {secs:.0} s and loaded in {load:.1} s, {note} ({gpu})"
                ),
            );
        }
        Err(e) => post_model(
            false,
            &format!("the downloaded file is not a usable BS-RoFormer SW model: {e}"),
        ),
    }
    Ok(())
}

/// Encodes one stem as a 192 kbps MP3 (`{type: "mp3", name, left, right,
/// rate}`) and sends the file back as `{type: "mp3", name, bytes}`.  Runs
/// here, not on the page, because the encoder does most of its work in one
/// final call (seconds for a whole song).
async fn mp3(msg: &JsValue) -> Result<(), String> {
    let name = get(msg, "name")
        .as_string()
        .unwrap_or_else(|| "stem.mp3".into());
    let left: Float32Array = get(msg, "left")
        .dyn_into()
        .map_err(|_| "left must be a Float32Array")?;
    let right: Float32Array = get(msg, "right")
        .dyn_into()
        .map_err(|_| "right must be a Float32Array")?;
    let rate = get(msg, "rate").as_f64().unwrap_or(44_100.0) as u32;
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
    post(
        &[
            ("type", "mp3".into()),
            ("name", name.into()),
            ("bytes", bytes.clone().into()),
        ],
        &[bytes.buffer().into()],
    );
    Ok(())
}

const KERNEL_CHOICE: &str = "matmul_kernels";

/// Sets this GPU's matmul kernels and attention path: the choice kept in
/// browser storage for this adapter and these kernels, or else several
/// seconds of `roformer_gpu::bench::calibrate`, whose result is then kept.
async fn choose_kernels(gpu: &Gpu) {
    let pref = STATE.with_borrow(|s| s.power);
    let key = format!(
        "{}|{:016x}",
        gpu_info(pref).await.as_string().unwrap_or_default(),
        roformer_gpu::kernels_id()
    );
    // One line per GPU; lines for other GPUs (the other adapter of a
    // two-GPU laptop) are kept.
    let mut others = Vec::new();
    let stored = cache_get(KERNEL_CHOICE).await;
    if !stored.is_null() {
        let text =
            String::from_utf8_lossy(&stored.unchecked_into::<Uint8Array>().to_vec()).to_string();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('\t') else {
                continue;
            };
            if k != key {
                others.push(line.to_string());
                continue;
            }
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
    }
    log("choosing kernels for this GPU (several seconds, once)…");
    let c = roformer_gpu::bench::calibrate(gpu, now_s, |l| log(&l)).await;
    gpu.policy.set(c.matmul);
    gpu.attention.set(c.attention);
    others.push(format!("{key}\t{}", c.name()));
    let text = others.join("\n") + "\n";
    cache_put(KERNEL_CHOICE, &Uint8Array::from(text.as_bytes())).await;
}
