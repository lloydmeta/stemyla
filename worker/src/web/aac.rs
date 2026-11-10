//! The AAC-LC encode of a save (M4 of `plans/active/m4a.md`), with the
//! browser's WebCodecs `AudioEncoder`.  The `m4a` crate makes the M4A.
//!
//! WebCodecs goes through the inline JS module below, not through the
//! WebCodecs types of `web-sys`.  Those types need
//! `--cfg=web_sys_unstable_apis`, which would change the build of every
//! wasm crate.
//!
//! The encode has these parts:
//!
//! * The probe.  WebCodecs does not give the encoder's delay, and it
//!   differs between encoders.  So each encode first encodes an impulse,
//!   decodes it with `AudioDecoder`, and measures the delay.
//! * The lead-in.  Safari encodes the first 1024 input frames as silence.
//!   So the encode puts [`LEAD_IN`] frames of silence before the source.
//!   The skip of the `elst` entry and of `iTunSMPB` is the lead-in and the
//!   delay together.
//! * The queues.  The encode waits while the encoder's queue holds more
//!   than [`QUEUE_MOST`] blocks.  It also waits while more than
//!   [`OUT_MOST`] frames are sent and not yet back as packets.  So neither
//!   the encoder's copies of the input nor the JS packet queue grow with the
//!   song.
//! * The order.  The packet count is known only at the end, so the M4A has
//!   its `moov` last (`Order::MoovLast`).  The encode reserves the 36 bytes
//!   of `ftyp` and the `mdat` header, puts each packet, puts the `moov`,
//!   then puts the final head in place of the reserve.
//!
//! Each packet goes from the encoder's JS callback to the sink as a JS
//! array.  So WebAssembly memory holds no packet, and each write of the
//! storage path is a whole array (MEM-5).  The encode keeps only the packet
//! sizes, 4 bytes for each packet of 1024 frames.

use js_sys::{Array, Uint8Array};
use m4a::{AacConfig, Edit, Gapless, Layout, MAX_CONFIG_LEN, Order, SampleEntry};
use stem_protocol::AAC_ENCODER;
use wasm_bindgen::prelude::*;

use super::encode::{EncodeOut, Failure, Input};
use super::js_text;

#[wasm_bindgen(inline_js = r#"
// Whether this browser has an AAC encoder for `config`.
export async function aac_supported(config) {
  if (typeof AudioEncoder === "undefined") return false;
  try { return !!(await AudioEncoder.isConfigSupported(config)).supported; }
  catch (_) { return false; }
}
// One encode: its encoder, its queue of packets, the description of its
// first output, its error, and the function that ends a wait.
export function aac_open(config) {
  const s = { enc: null, chunks: [], received: 0, description: null, error: null, wake: null };
  const wake = () => { const w = s.wake; s.wake = null; if (w) w(); };
  s.enc = new AudioEncoder({
    output: (chunk, meta) => {
      const bytes = new Uint8Array(chunk.byteLength);
      chunk.copyTo(bytes);
      s.chunks.push(bytes);
      s.received += 1;
      const d = meta && meta.decoderConfig && meta.decoderConfig.description;
      if (d && !s.description) {
        const view = ArrayBuffer.isView(d) ? new Uint8Array(d.buffer, d.byteOffset, d.byteLength) : new Uint8Array(d);
        s.description = view.slice();
      }
      wake();
    },
    error: (e) => { if (!s.error) s.error = String(e); wake(); },
  });
  try { s.enc.configure(config); }
  catch (e) { aac_close(s); throw e; }
  return s;
}
// Encodes `frames` frames of `planar` (the left plane, then the right).
// The AudioData is closed at once: the encoder keeps its own copy.
export function aac_encode(s, planar, frames, rate, timestamp) {
  if (s.error) throw new Error(s.error);
  const data = new AudioData({ format: "f32-planar", sampleRate: rate, numberOfFrames: frames,
    numberOfChannels: 2, timestamp, data: planar });
  try { s.enc.encode(data); } finally { data.close(); }
}
// Encodes `frames` frames of silence, made in JS.
export function aac_encode_silence(s, frames, rate, timestamp) {
  aac_encode(s, new Float32Array(2 * frames), frames, rate, timestamp);
}
// Waits while the encoder's queue holds more than `most` blocks, or until
// an error: whether the queue moved within `ms` at each wait.  The listener
// is added before the queue is read again, so the wait cannot miss the
// event.
export async function aac_wait_queue(s, most, ms) {
  while (!s.error && s.enc.encodeQueueSize > most) {
    const moved = await new Promise((r) => {
      let timer = null;
      const done = (ok) => {
        clearTimeout(timer);
        s.enc.removeEventListener("dequeue", on);
        s.wake = null;
        r(ok);
      };
      const on = () => done(true);
      s.wake = on;
      s.enc.addEventListener("dequeue", on);
      if (s.error || s.enc.encodeQueueSize <= most) { done(true); return; }
      timer = setTimeout(() => done(false), ms);
    });
    if (!moved) return false;
  }
  return true;
}
// Waits for the next output or error: whether one came within `ms`.
export function aac_wait_output(s, ms) {
  const seen = s.received;
  return new Promise((r) => {
    let timer = null;
    const done = () => { clearTimeout(timer); s.wake = null; r(s.received > seen || !!s.error); };
    s.wake = done;
    timer = setTimeout(done, ms);
  });
}
// Yields one task, so that the outputs of the encoder can arrive.  A
// message is a task with no clamp, as a nested setTimeout has.
export function aac_yield() {
  return new Promise((r) => {
    const ch = new MessageChannel();
    ch.port1.onmessage = () => { ch.port1.close(); r(); };
    ch.port2.postMessage(0);
  });
}
// The packets that came since the last take, as an array of Uint8Arrays.
export function aac_take(s) { const c = s.chunks; s.chunks = []; return c; }
export function aac_received(s) { return s.received; }
export function aac_error(s) { return s.error; }
export function aac_description(s) { return s.description; }
export async function aac_flush(s) {
  if (s.error) throw new Error(s.error);
  try { await s.enc.flush(); }
  catch (e) { throw new Error(s.error || String(e)); }
  if (s.error) throw new Error(s.error);
}
// Closes the encoder, if it is not closed.  After its error callback, a
// codec is closed, and a second close throws.
export function aac_close(s) {
  if (s.enc && s.enc.state !== "closed") s.enc.close();
}
// The probe: encodes 16384 frames with 1.0 at frame 4096 and 0 elsewhere,
// and gives the packets and the description.
export async function aac_probe(config) {
  const s = aac_open(config);
  try {
    const frames = 16384;
    const planar = new Float32Array(2 * frames);
    planar[4096] = 1;
    planar[frames + 4096] = 1;
    aac_encode(s, planar, frames, config.sampleRate, 0);
    await aac_flush(s);
    return [aac_take(s), s.description];
  } finally { aac_close(s); }
}
// Decodes `chunks` with `AudioDecoder` and the bare AudioSpecificConfig
// `asc`: the frame and the value of the largest absolute sample of the
// left channel, and the frames decoded.
export async function aac_peak(config, asc, chunks) {
  let error = null, frames = 0, at = -1, peak = 0;
  const dec = new AudioDecoder({
    output: (a) => {
      try {
        const left = new Float32Array(a.numberOfFrames);
        a.copyTo(left, { planeIndex: 0, format: "f32-planar" });
        for (let i = 0; i < left.length; i++) {
          if (Math.abs(left[i]) > peak) { peak = Math.abs(left[i]); at = frames + i; }
        }
        frames += left.length;
      } finally { a.close(); }
    },
    error: (e) => { if (!error) error = String(e); },
  });
  try {
    dec.configure({ codec: config.codec, sampleRate: config.sampleRate,
      numberOfChannels: config.numberOfChannels, description: asc });
    chunks.forEach((data, k) => dec.decode(new EncodedAudioChunk({ type: "key",
      timestamp: Math.round(k * 1024 * 1e6 / config.sampleRate), data })));
    await dec.flush();
  } catch (e) { throw new Error(error || String(e)); }
  finally { if (dec.state !== "closed") dec.close(); }
  if (error) throw new Error(error);
  return [at, peak, frames];
}
"#)]
extern "C" {
    async fn aac_supported(config: &JsValue) -> JsValue;
    #[wasm_bindgen(catch)]
    fn aac_open(config: &JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    fn aac_encode(
        s: &JsValue,
        planar: &[f32],
        frames: u32,
        rate: u32,
        timestamp: f64,
    ) -> Result<(), JsValue>;
    #[wasm_bindgen(catch)]
    fn aac_encode_silence(
        s: &JsValue,
        frames: u32,
        rate: u32,
        timestamp: f64,
    ) -> Result<(), JsValue>;
    async fn aac_wait_queue(s: &JsValue, most: u32, ms: u32) -> JsValue;
    async fn aac_wait_output(s: &JsValue, ms: u32) -> JsValue;
    async fn aac_yield();
    fn aac_take(s: &JsValue) -> Array;
    fn aac_received(s: &JsValue) -> f64;
    fn aac_error(s: &JsValue) -> Option<String>;
    fn aac_description(s: &JsValue) -> Option<Uint8Array>;
    #[wasm_bindgen(catch)]
    async fn aac_flush(s: &JsValue) -> Result<(), JsValue>;
    fn aac_close(s: &JsValue);
    #[wasm_bindgen(catch)]
    async fn aac_probe(config: &JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn aac_peak(
        config: &JsValue,
        asc: &Uint8Array,
        chunks: &Array,
    ) -> Result<JsValue, JsValue>;
}

/// Whether this browser has an AAC encoder for the config of a save at
/// `rate`.
#[cfg(test)]
pub(super) async fn aac_encoder_here(rate: u32) -> bool {
    aac_supported(&AAC_ENCODER.to_js(rate).into())
        .await
        .as_bool()
        == Some(true)
}

/// Frames of silence before the source.  Safari encodes the first 1024
/// input frames as silence, so 2048 keeps one packet of margin.  The skip
/// trims them.
pub(super) const LEAD_IN: usize = 2048;

/// Frames of each channel that an AAC encode reads at a time.
pub(super) const AAC_BLOCK: usize = 4096;

/// The most blocks in the encoder's queue before the encode waits.
pub(super) const QUEUE_MOST: u32 = 4;

/// The most frames sent and not yet back as packets before the encode
/// waits for an output.
pub(super) const OUT_MOST: usize = 32_768;

/// The frames in each AAC packet.
const PACKET_FRAMES: usize = 1024;

/// The frame of the probe's impulse.
const PROBE_AT: i64 = 4096;

/// The probe refuses a delay of this many frames or more.
const MOST_DELAY: i64 = 8192;

/// The probe refuses a peak below this.
const LEAST_PEAK: f64 = 0.25;

/// The encode reads two channels from `Input`, so a save is stereo by
/// construction.
const _: () = assert!(AAC_ENCODER.channels == 2);

/// The packets that the bound of a layout adds to the frames of the encode.
const BOUND_PACKETS: usize = 8;

/// Milliseconds that an encode waits for an output, or for its queue to
/// move, before it ends.
const OUTPUT_WAIT_MS: u32 = 10_000;

thread_local! {
    /// The wait for an output, which a test makes short.
    static OUTPUT_WAIT: std::cell::Cell<u32> = const { std::cell::Cell::new(OUTPUT_WAIT_MS) };
}

/// Sets the wait for an output, for a test of an encoder that gives none.
#[cfg(test)]
pub(super) fn set_output_wait_ms(ms: u32) {
    OUTPUT_WAIT.with(|wait| wait.set(ms));
}

/// The error of the encoder: an encoder failure, not a storage failure.
fn encoder_failure(text: impl std::fmt::Display) -> Failure {
    Failure::encoder(format!("AAC encoder: {text}"))
}

/// One encoder of the browser.  Dropping it closes the encoder, on every
/// path, errors included.
struct Session(JsValue);

impl Session {
    fn open(rate: u32) -> Result<Self, Failure> {
        aac_open(&AAC_ENCODER.to_js(rate))
            .map(Session)
            .map_err(|e| encoder_failure(js_text(&e)))
    }

    /// The error of the encoder, or of `e`.
    fn failure(&self, e: &JsValue) -> Failure {
        encoder_failure(aac_error(&self.0).unwrap_or_else(|| js_text(e)))
    }

    /// Puts each packet that came into `out`, and pushes its size.
    async fn drain(&self, out: &mut EncodeOut, sizes: &mut Vec<u32>) -> Result<(), Failure> {
        for chunk in aac_take(&self.0).iter() {
            let chunk = Uint8Array::from(chunk);
            sizes.push(chunk.length());
            out.put_array(&chunk).await?;
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        aac_close(&self.0);
    }
}

/// The delay of this browser's AAC encoder at `rate`, in frames: the
/// probe.  It encodes an impulse at frame 4096 with the save's exact
/// config, and decodes it with `AudioDecoder`.  The delay is the frame of
/// the largest absolute sample, minus 4096.
pub(super) async fn aac_delay(rate: u32) -> Result<usize, Failure> {
    let config = AAC_ENCODER.to_js(rate);
    let probe = aac_probe(&config)
        .await
        .map_err(|e| encoder_failure(format!("the probe failed: {}", js_text(&e))))?;
    let probe = Array::from(&probe);
    let chunks = Array::from(&probe.get(0));
    let description = probe.get(1).dyn_into::<Uint8Array>().ok();
    let description =
        description.ok_or_else(|| encoder_failure("the probe gave no description"))?;
    let description = description_bytes(&description);
    let asc = m4a::audio_specific_config(&description)
        .map_err(|e| encoder_failure(format!("the probe's description: {e}")))?;
    let peak = aac_peak(&config, &Uint8Array::from(asc), &chunks)
        .await
        .map_err(|e| encoder_failure(format!("the probe did not decode: {}", js_text(&e))))?;
    let peak = Array::from(&peak);
    let (at, value) = (
        peak.get(0).as_f64().unwrap_or(-1.0) as i64,
        peak.get(1).as_f64().unwrap_or(0.0),
    );
    if value < LEAST_PEAK {
        return Err(encoder_failure(format!(
            "the probe's impulse decoded with a peak of {value}, below {LEAST_PEAK}"
        )));
    }
    let delay = at - PROBE_AT;
    if !(0..MOST_DELAY).contains(&delay) {
        return Err(encoder_failure(format!(
            "the probe gave a delay of {delay} frames, not 0 to {}",
            MOST_DELAY - 1
        )));
    }
    Ok(delay as usize)
}

/// Encodes `input` as an AAC-LC M4A into `out`, with its `moov` last.  It
/// reserves the head, puts each packet as it comes, then puts the `moov`,
/// then the final head in place of the reserve.  It keeps only the packet
/// sizes, so WebAssembly memory never holds the samples or the file
/// (MEM-1, MEM-4).
pub(super) async fn aac_into(input: &Input, rate: u32, out: &mut EncodeOut) -> Result<(), Failure> {
    let frames = input.frames();
    if frames == 0 {
        return Err(encoder_failure("the stem has no samples"));
    }
    if !m4a::AAC_RATES.contains(&rate) {
        return Err(encoder_failure(format!(
            "{rate} Hz is not a rate of the AAC table"
        )));
    }
    let supported = aac_supported(&AAC_ENCODER.to_js(rate).into()).await;
    if supported.as_bool() != Some(true) {
        return Err(encoder_failure("this browser has no AAC encoder"));
    }
    let delay = aac_delay(rate).await?;
    let skip = LEAD_IN + delay;
    let bound_packets = (skip + frames).div_ceil(PACKET_FRAMES) + BOUND_PACKETS;
    let bound = layout(MAX_CONFIG_LEN, bound_packets, rate, skip, frames)?;
    out.reserve(bound.head_len()).await?;
    let mut sizes = Vec::with_capacity(bound_packets);
    let config = aac_packets(input, rate, out, &mut sizes).await?;
    let packets = sizes.len();
    if packets * PACKET_FRAMES < skip + frames {
        return Err(encoder_failure(format!(
            "the encoder gave too few packets: {packets} packets of 1024 frames for {} frames",
            skip + frames
        )));
    }
    let config_len =
        u8::try_from(config.len()).map_err(|_| encoder_failure("the config is too long"))?;
    let layout = layout(config_len, packets, rate, skip, frames)?;
    aac_end(&layout, &sizes, config, bound.head_len(), out).await
}

/// The layout of an AAC M4A of `packets` packets, whose `elst` entry and
/// `iTunSMPB` tag skip `skip` frames, then play `frames`.
fn layout(
    config_len: u8,
    packets: usize,
    rate: u32,
    skip: usize,
    frames: usize,
) -> Result<Layout, Failure> {
    let track = packets * PACKET_FRAMES;
    let codec = m4a::Codec::Aac {
        channels: 2,
        config_len,
    };
    let skip32 = u32::try_from(skip).map_err(|_| encoder_failure("the skip is too long"))?;
    let edit = Edit {
        skip: skip32,
        frames: frames as u64,
    };
    let padding = u32::try_from(track - skip - frames)
        .map_err(|_| encoder_failure("the padding is too long"))?;
    Layout::new(codec, track as u64, rate, Some(edit), Order::MoovLast)
        .and_then(|layout| {
            layout.with_gapless(Gapless {
                delay: skip32,
                padding,
                frames: frames as u64,
            })
        })
        .map_err(encoder_failure)
}

/// Encodes the lead-in, then each block of `input`, and puts each packet
/// into `out`: the AudioSpecificConfig of the encoder, checked.  The block
/// buffers and the encoder are freed when it returns, before the `moov`
/// is made.
async fn aac_packets(
    input: &Input,
    rate: u32,
    out: &mut EncodeOut,
    sizes: &mut Vec<u32>,
) -> Result<Vec<u8>, Failure> {
    let frames = input.frames();
    let session = Session::open(rate)?;
    let s = &session.0;
    let timestamp = |frame: usize| (frame as f64 * 1_000_000.0 / f64::from(rate)).round();
    aac_encode_silence(s, LEAD_IN as u32, rate, 0.0).map_err(|e| session.failure(&e))?;
    let mut sent = LEAD_IN;
    let mut planar = vec![0.0; 2 * AAC_BLOCK];
    for at in (0..frames).step_by(AAC_BLOCK) {
        let n = AAC_BLOCK.min(frames - at);
        let (left, right) = planar[..2 * n].split_at_mut(n);
        input.read(at, left, right)?;
        aac_encode(s, &planar[..2 * n], n as u32, rate, timestamp(LEAD_IN + at))
            .map_err(|e| session.failure(&e))?;
        sent += n;
        let wait_ms = OUTPUT_WAIT.with(|wait| wait.get());
        let moved = aac_wait_queue(s, QUEUE_MOST, wait_ms).await;
        if moved.as_bool() != Some(true) {
            return Err(encoder_failure("the encoder's queue did not move"));
        }
        aac_yield().await;
        session.drain(out, sizes).await?;
        while sent > PACKET_FRAMES * aac_received(s) as usize + OUT_MOST {
            let came = aac_wait_output(s, wait_ms).await;
            if let Some(error) = aac_error(s) {
                return Err(encoder_failure(error));
            }
            if came.as_bool() != Some(true) {
                return Err(encoder_failure("the encoder gave no output"));
            }
            session.drain(out, sizes).await?;
        }
        if let Some(error) = aac_error(s) {
            return Err(encoder_failure(error));
        }
    }
    drop(planar);
    aac_flush(s).await.map_err(|e| session.failure(&e))?;
    session.drain(out, sizes).await?;
    let description =
        aac_description(s).ok_or_else(|| encoder_failure("the encoder gave no description"))?;
    drop(session);
    checked_config(&description_bytes(&description), rate)
}

/// The bytes of a description, which Rust reads.
fn description_bytes(description: &Uint8Array) -> Vec<u8> {
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1: a description of some tens of bytes, not a song"
    )]
    let bytes = description.to_vec();
    bytes
}

/// The AudioSpecificConfig of `description`, which must be AAC-LC in
/// stereo at `rate`, with 1024 frames in each packet.
fn checked_config(description: &[u8], rate: u32) -> Result<Vec<u8>, Failure> {
    let config = m4a::audio_specific_config(description).map_err(encoder_failure)?;
    let fields = AacConfig::read(config).map_err(encoder_failure)?;
    if fields.object_type != 2 {
        return Err(encoder_failure(format!(
            "the encoder gave object type {}, not 2 (AAC-LC)",
            fields.object_type
        )));
    }
    if fields.channel_config != 2 {
        return Err(encoder_failure(format!(
            "the encoder gave channel configuration {}, not 2 (stereo)",
            fields.channel_config
        )));
    }
    if fields.rate != rate {
        return Err(encoder_failure(format!(
            "the encoder gave {} Hz, not {rate} Hz",
            fields.rate
        )));
    }
    if fields.frames_960 {
        return Err(encoder_failure(
            "the encoder gave 960 frames in each packet, not 1024",
        ));
    }
    Ok(config.into())
}

/// The end of an AAC encode: puts the `moov` of the packet table `sizes`,
/// then the final head in place of the reserve of `reserved` bytes.  The
/// sizes and the `moov` live together here, about 12 bytes for each
/// packet.
pub(super) async fn aac_end(
    layout: &Layout,
    sizes: &[u32],
    config: Vec<u8>,
    reserved: usize,
    out: &mut EncodeOut,
) -> Result<(), Failure> {
    let entry = SampleEntry::Aac { config };
    let tail = layout.tail(&entry, sizes).map_err(encoder_failure)?;
    out.put(&tail).await?;
    drop(tail);
    let head = layout.head(&entry, sizes).map_err(encoder_failure)?;
    if head.len() != reserved {
        return Err(encoder_failure(format!(
            "the head has {} bytes, not the {reserved} bytes of its reserve",
            head.len()
        )));
    }
    out.replace_head(&head).await?;
    Ok(())
}
