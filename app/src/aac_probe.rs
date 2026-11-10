//! The page's probe of the browser's AAC encoder (M4 of
//! `plans/active/m4a.md`): it decides whether the Save menu offers
//! "M4A (AAC 256 kbps)".  It runs once at start, on the page, with no
//! message to the worker.  It passes when three things hold:
//! `AudioEncoder` exists, `isConfigSupported` passes for
//! `stem_protocol::AAC_ENCODER`, and an encode of 4096 frames, then a
//! flush, gives a packet within a time limit.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
// Whether this browser can encode AAC with `config`, as [ok, why].  Each
// failure gives false, with no throw.  It closes its AudioData and its
// encoder on each path.
export async function aac_probe(config, ms) {
  if (typeof AudioEncoder === "undefined") return [false, "no AudioEncoder"];
  try {
    const support = await AudioEncoder.isConfigSupported(config);
    if (!support || !support.supported) return [false, "isConfigSupported is false"];
  } catch (e) { return [false, `isConfigSupported: ${e}`]; }
  let enc = null, data = null, packets = 0, error = null, timer = null;
  try {
    enc = new AudioEncoder({ output: () => { packets++; }, error: (e) => { error = String(e); } });
    enc.configure(config);
    const frames = 4096;
    data = new AudioData({ format: "f32-planar", sampleRate: config.sampleRate, numberOfFrames: frames,
      numberOfChannels: config.numberOfChannels, timestamp: 0,
      data: new Float32Array(frames * config.numberOfChannels) });
    enc.encode(data);
    data.close();
    data = null;
    const flushed = enc.flush().then(() => "flushed", (e) => `flush: ${error || e}`);
    const late = new Promise((r) => { timer = setTimeout(() => r("late"), ms); });
    const end = await Promise.race([flushed, late]);
    if (end === "late") return [false, `no packet within ${ms} ms`];
    if (end !== "flushed") return [false, end];
    if (error) return [false, error];
    return packets > 0 ? [true, ""] : [false, "the test encode gave no packet"];
  } catch (e) { return [false, error || String(e)]; }
  finally {
    clearTimeout(timer);
    if (data) data.close();
    if (enc && enc.state !== "closed") enc.close();
  }
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = aac_probe)]
    async fn aac_probe_js(config: &JsValue, ms: u32) -> JsValue;
}

/// The time limit of the app's probe, in milliseconds.
pub const PROBE_MS: u32 = 5_000;

/// Whether this browser can save AAC: `Ok(())`, or why not.  It never
/// throws.
pub async fn aac_encoder(ms: u32) -> Result<(), String> {
    let config = stem_protocol::AAC_ENCODER.to_js(44_100);
    let result = js_sys::Array::from(&aac_probe_js(&config, ms).await);
    if result.get(0).as_bool() == Some(true) {
        Ok(())
    } else {
        Err(result.get(1).as_string().unwrap_or_default())
    }
}
