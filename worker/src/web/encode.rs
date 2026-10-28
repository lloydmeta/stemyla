//! The MP3 and FLAC encoders of a save (M8 of
//! `plans/active/stream-from-storage.md`).
//!
//! An encode reads its audio from one of two sources ([`Encode`]): two
//! transferred arrays (the memory path of a save), or a 16-bit stereo WAV
//! file in the `saves` folder of browser storage (the storage path).  It
//! reads either source one block at a time.  It converts each 16-bit sample
//! of a WAV to `s / 32768`, which is exact, so [`pcm16`] gives the same 16
//! bits back.
//!
//! On the memory path, the reply carries the file: the MP3's bytes, or a
//! `Blob` of the FLAC's encoded frames.  On the storage path, the encoder
//! writes the file to `out` through one `FileSystemSyncAccessHandle`, each
//! write a whole new `Uint8Array` (MEM-5).  It replies with the path
//! ([`Encoded::Stored`]).  At the end of a stored encode, the worker
//! removes the WAV file, also when the encode fails.  A failed stored
//! encode also removes its output, and replies with the reason
//! ([`Encoded::Failed`]), not with `error`.  So the page knows which save
//! failed.  A malformed request, or a file outside `saves/`, gives `error`,
//! and the worker removes nothing.
//!
//! A stored FLAC writes its stream header first, with the frame count still
//! unknown, then each frame as it is encoded, then the header again.  It
//! holds no list of encoded parts.
//!
//! The MP3 exception (D4 of the M8 plan, TD-59).  `rusty_mp3` 1.0.0 keeps
//! every encoded frame until `finish()` at 256 kbps or less on MPEG-1 (its
//! bit reservoir).  So an MP3 encode holds the MP3's frames in WebAssembly
//! memory, about 3.5 times the MP3's size at its peak (TD-59).  The worker
//! writes the packets to the output file after `finish()`, as `next_packet`
//! gives them, so the Info frame is first at every rate.
//!
//! The encodes do not wait for the worker's turn: they run beside a
//! separation.  After each encode, the worker logs a `memory: encode` line
//! with its WebAssembly memory before and after (D9 of the M8 plan).

use flacenc::component::{BitRepr, Stream, StreamInfo};
use flacenc::error::Verify;
use flacenc::source::{Context, Fill, FrameBuf};
use js_sys::{Float32Array, Uint8Array};
use stem_protocol::{Encode, Encoded, Reply};
use wasm_bindgen::prelude::*;
use web_sys::Blob;

use super::mix::mix_file;
use super::{blob_range, js_text, log, pcm16, send};

#[wasm_bindgen(inline_js = r#"
async function folder_and_name(path) {
  const parts = path.split("/").filter((s) => s);
  const name = parts.pop();
  let d = await navigator.storage.getDirectory();
  for (const p of parts) d = await d.getDirectoryHandle(p);
  return [d, name];
}
export async function out_open(path) {
  const [d, name] = await folder_and_name(path);
  const h = await (await d.getFileHandle(name, { create: true })).createSyncAccessHandle();
  try { await h.truncate(0); }
  catch (e) { try { await h.close(); } catch (_) {} throw e; }
  return h;
}
// `write` returns the bytes written.  Old browsers return a promise, so
// it is awaited, as `flush` and `close` are.
export async function out_write(h, bytes, at) {
  const n = await h.write(bytes, { at });
  if (n !== bytes.byteLength) throw new Error(`wrote ${n} of ${bytes.byteLength} bytes`);
}
export async function out_close(h) {
  try { await h.flush(); }
  finally { await h.close(); }
}
export async function out_drop(h) {
  try { await h.close(); } catch (_) {}
}
export async function save_remove(path) {
  try { const [d, name] = await folder_and_name(path); await d.removeEntry(name); return true; }
  catch (e) { return e.name === "NotFoundError"; }
}
// The worker's JS heap after a garbage collection, where Chrome gives
// `performance.memory` here and `gc` is exposed; else null.
export function js_heap() {
  const m = globalThis.performance && performance.memory;
  if (!m) return null;
  if (typeof globalThis.gc === "function") globalThis.gc();
  return m.usedJSHeapSize;
}
"#)]
extern "C" {
    /// A sync access handle on the file at `path`, made if missing, and
    /// emptied.
    #[wasm_bindgen(catch)]
    async fn out_open(path: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn out_write(h: &JsValue, bytes: &Uint8Array, at: f64) -> Result<(), JsValue>;
    /// Flushes and closes the handle.
    #[wasm_bindgen(catch)]
    async fn out_close(h: &JsValue) -> Result<(), JsValue>;
    /// Closes the handle, and ignores a failure.
    async fn out_drop(h: &JsValue);
    /// Removes the file at `path`: whether it is gone.
    async fn save_remove(path: &str) -> JsValue;
    fn js_heap() -> Option<f64>;
}

/// Frames of each channel that an MP3 encode reads at a time: 16 MP3
/// frames, 72 KB for each channel of floats.
pub(super) const MP3_BLOCK: usize = 1152 * 16;

/// Frames of each channel in one FLAC frame, and that a FLAC encode reads
/// at a time.  RFC 9639's streamable subset allows 4,608 at 48 kHz or
/// less, so every player reads the file.
pub(super) const FLAC_BLOCK: usize = 4096;

/// Bytes of the header of a 16-bit stereo WAV that the page writes.
const WAV_HEADER_BYTES: usize = 44;
/// Bytes of one frame of a 16-bit stereo WAV.
const WAV_FRAME_BYTES: usize = 4;
/// The folder of the save files, at the root of the private file system.
const SAVES: &str = "saves/";

/// The two encoders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Codec {
    Mp3,
    Flac,
}

impl Codec {
    pub(super) fn kind(self) -> &'static str {
        match self {
            Codec::Mp3 => stem_protocol::kind::MP3,
            Codec::Flac => stem_protocol::kind::FLAC,
        }
    }

    fn reply(self, name: String, file: Encoded) -> Reply {
        match self {
            Codec::Mp3 => Reply::Mp3 { name, file },
            Codec::Flac => Reply::Flac { name, file },
        }
    }
}

/// The audio of an encode, read one block at a time.
enum Input {
    Arrays([Float32Array; 2]),
    /// A 16-bit stereo WAV file of `frames` frames, as a `File` of its
    /// state at the open.
    Wav {
        file: Blob,
        frames: usize,
    },
}

impl Input {
    /// Opens the WAV file at `path`, and checks that it holds `frames`
    /// frames.
    #[cfg_attr(mutants, mutants::skip)]
    async fn wav(path: &str, frames: usize) -> Result<Self, String> {
        let file = mix_file(path)
            .await
            .map_err(|e| format!("the WAV file could not be opened: {}", js_text(&e)))?;
        // In f64: the bytes of a long song do not fit a wasm32 usize.
        let want = WAV_HEADER_BYTES as f64 + WAV_FRAME_BYTES as f64 * frames as f64;
        if file.size() != want {
            return Err(format!(
                "the WAV file holds {} bytes, not the {want} bytes of {frames} frames",
                file.size()
            ));
        }
        Ok(Self::Wav { file, frames })
    }

    #[cfg_attr(mutants, mutants::skip)]
    fn frames(&self) -> usize {
        match self {
            Self::Arrays([left, right]) => left.length().min(right.length()) as usize,
            Self::Wav { frames, .. } => *frames,
        }
    }

    /// Reads the frames from `at` into `left` and `right`, which have the
    /// same length.
    #[cfg_attr(mutants, mutants::skip)]
    fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> Result<(), String> {
        let n = left.len();
        match self {
            Self::Arrays([l, r]) => {
                let (start, end) = (at as u32, (at + n) as u32);
                l.subarray(start, end).copy_to(left);
                r.subarray(start, end).copy_to(right);
            }
            Self::Wav { file, .. } => {
                let byte =
                    |frame: usize| WAV_HEADER_BYTES as f64 + WAV_FRAME_BYTES as f64 * frame as f64;
                let bytes = blob_range(file, byte(at), byte(at + n))
                    .map_err(|e| format!("the WAV file could not be read: {}", js_text(&e)))?;
                if bytes.length() as usize != WAV_FRAME_BYTES * n {
                    return Err(format!(
                        "the WAV file gave {} bytes of frames {at} to {}, not {}",
                        bytes.length(),
                        at + n,
                        WAV_FRAME_BYTES * n
                    ));
                }
                #[expect(
                    clippy::disallowed_methods,
                    reason = "MEM-1: one block of the WAV file, not the song"
                )]
                let bytes = bytes.to_vec();
                let sample = |k: usize| f32::from(i16::from_le_bytes([bytes[k], bytes[k + 1]]));
                for (f, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    *l = sample(WAV_FRAME_BYTES * f) / 32768.0;
                    *r = sample(WAV_FRAME_BYTES * f + 2) / 32768.0;
                }
            }
        }
        Ok(())
    }
}

/// The output file of a stored encode, written through one sync access
/// handle.
struct OutFile {
    handle: JsValue,
    path: String,
    /// Bytes written from the start.
    at: f64,
}

impl OutFile {
    #[cfg_attr(mutants, mutants::skip)]
    async fn open(path: &str) -> Result<Self, String> {
        let handle = out_open(path)
            .await
            .map_err(|e| format!("the output file could not be opened: {}", js_text(&e)))?;
        Ok(Self {
            handle,
            path: path.into(),
            at: 0.0,
        })
    }

    /// Writes `bytes` after the bytes so far.
    #[cfg_attr(mutants, mutants::skip)]
    async fn append(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.write_at(bytes, self.at).await?;
        self.at += bytes.len() as f64;
        Ok(())
    }

    /// Writes `bytes` at `at`, as a new array of their own, never a view
    /// of WebAssembly memory (MEM-5).
    #[cfg_attr(mutants, mutants::skip)]
    async fn write_at(&self, bytes: &[u8], at: f64) -> Result<(), String> {
        let whole = Uint8Array::from(bytes);
        out_write(&self.handle, &whole, at)
            .await
            .map_err(|e| format!("the output file could not be written: {}", js_text(&e)))
    }

    #[cfg_attr(mutants, mutants::skip)]
    async fn close(&self) -> Result<(), String> {
        out_close(&self.handle)
            .await
            .map_err(|e| format!("the output file could not be closed: {}", js_text(&e)))
    }

    /// Closes the handle and removes the file.
    #[cfg_attr(mutants, mutants::skip)]
    async fn abandon(&self) {
        out_drop(&self.handle).await;
        save_remove(&self.path).await;
    }
}

/// Where a FLAC encode puts its stream header and frames.
enum FlacOut {
    /// Each part as a `Uint8Array` in a JS array, for one `Blob` at the
    /// end (the memory path).
    Parts(js_sys::Array),
    File(OutFile),
}

impl FlacOut {
    #[cfg_attr(mutants, mutants::skip)]
    async fn put(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Parts(parts) => {
                parts.push(&Uint8Array::from(bytes));
                Ok(())
            }
            Self::File(file) => file.append(bytes).await,
        }
    }

    /// Puts the final stream header in place of the first one.
    #[cfg_attr(mutants, mutants::skip)]
    async fn replace_header(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Parts(parts) => {
                parts.set(0, Uint8Array::from(bytes).into());
                Ok(())
            }
            Self::File(file) => file.write_at(bytes, 0.0).await,
        }
    }
}

/// Encodes one stem or mix (`{type: "mp3"}` or `{type: "flac"}`) and
/// replies with its file.  An error is a malformed request, or a failed
/// encode on the memory path, for an `error` reply.
#[cfg_attr(mutants, mutants::skip)]
pub(super) async fn encode(
    codec: Codec,
    name: Option<String>,
    source: Encode,
    rate: Option<u32>,
) -> Result<(), String> {
    let name = name.unwrap_or_else(|| format!("stem.{}", codec.kind()));
    let rate = rate.unwrap_or(44_100);
    let before = wasm_bytes();
    match source {
        Encode::Arrays { left, right } => {
            let input = Input::Arrays([left, right]);
            let (file, size) = match codec {
                Codec::Mp3 => {
                    let bytes = mp3_bytes(&input, rate)?;
                    let size = bytes.length() as f64;
                    (Encoded::Bytes(bytes), size)
                }
                Codec::Flac => {
                    let blob = flac_blob(&input, rate).await?;
                    let size = blob.size();
                    (Encoded::Blob(blob), size)
                }
            };
            log_memory(codec, &name, before, size);
            send(codec.reply(name, file));
        }
        Encode::Stored { wav, frames, out } => {
            for (field, path) in [("wav", &wav), ("out", &out)] {
                if !is_save_path(path) {
                    return Err(format!("{field} must be a file in {SAVES}, not {path:?}"));
                }
            }
            if wav == out {
                return Err("wav and out must be two files".into());
            }
            let written = stored(codec, &wav, frames, &out, rate).await;
            save_remove(&wav).await;
            let file = match written {
                Ok(size) => {
                    log_memory(codec, &out, before, size);
                    Encoded::Stored { path: out }
                }
                Err(text) => {
                    log_memory(codec, &out, before, 0.0);
                    Encoded::Failed { path: out, text }
                }
            };
            send(codec.reply(name, file));
        }
    }
    Ok(())
}

/// Whether `path` names a file in the `saves` folder, so that a page bug
/// cannot remove a session file.
fn is_save_path(path: &str) -> bool {
    path.strip_prefix(SAVES)
        .is_some_and(|name| !name.is_empty() && !name.contains('/') && name != "." && name != "..")
}

/// Encodes the WAV file at `wav` into the file at `out`: the bytes written.
/// On a failure, it leaves no file at `out`.
#[cfg_attr(mutants, mutants::skip)]
async fn stored(
    codec: Codec,
    wav: &str,
    frames: usize,
    out: &str,
    rate: u32,
) -> Result<f64, String> {
    let input = Input::wav(wav, frames).await?;
    let mut file = match OutFile::open(out).await {
        Ok(file) => file,
        Err(e) => {
            save_remove(out).await;
            return Err(e);
        }
    };
    let written = match codec {
        Codec::Mp3 => mp3_into(&input, rate, &mut file).await,
        Codec::Flac => {
            let mut sink = FlacOut::File(file);
            let written = flac_into(&input, rate, &mut sink).await;
            let FlacOut::File(back) = sink else {
                unreachable!("a file stays a file")
            };
            file = back;
            written
        }
    };
    let closed = match written {
        Ok(()) => file.close().await,
        Err(e) => Err(e),
    };
    match closed {
        Ok(()) => Ok(file.at),
        Err(e) => {
            file.abandon().await;
            Err(e)
        }
    }
}

/// The MP3 encoder after it has taken every block of `input` and
/// finished.  An encoder at 192 kbps.
#[cfg_attr(mutants, mutants::skip)]
fn mp3_encoder(input: &Input, rate: u32) -> Result<rusty_mp3::Mp3Encoder, String> {
    let frames = input.frames();
    let mut enc = rusty_mp3::Mp3Encoder::new(rusty_mp3::Mp3EncoderConfig {
        bitrate_kbps: 192,
        vbr_quality: None,
    });
    let (mut l, mut r) = (vec![0.0; MP3_BLOCK], vec![0.0; MP3_BLOCK]);
    let mut pcm = Vec::with_capacity(2 * MP3_BLOCK);
    let mut at = 0;
    // At least one push, so an unsupported rate fails with no samples too.
    loop {
        let n = MP3_BLOCK.min(frames - at);
        input.read(at, &mut l[..n], &mut r[..n])?;
        pcm.clear();
        for (a, b) in l[..n].iter().zip(&r[..n]) {
            pcm.push(*a);
            pcm.push(*b);
        }
        enc.push_pcm_f32(&pcm, 2, rate)
            .map_err(|e| format!("MP3 encoder: {e:?}"))?;
        at += n;
        if at == frames {
            break;
        }
    }
    enc.finish();
    Ok(enc)
}

/// The MP3 of `input`, for the reply of the memory path.
#[cfg_attr(mutants, mutants::skip)]
fn mp3_bytes(input: &Input, rate: u32) -> Result<Uint8Array, String> {
    let mut enc = mp3_encoder(input, rate)?;
    let mut out = Vec::new();
    while let Ok(packet) = enc.next_packet() {
        out.extend_from_slice(&packet);
    }
    Ok(Uint8Array::from(out.as_slice()))
}

/// Writes the MP3 of `input` to `file`, one packet at a time, with no
/// copy of the file of its own.
#[cfg_attr(mutants, mutants::skip)]
async fn mp3_into(input: &Input, rate: u32, file: &mut OutFile) -> Result<(), String> {
    let mut enc = mp3_encoder(input, rate)?;
    while let Ok(packet) = enc.next_packet() {
        file.append(&packet).await?;
    }
    Ok(())
}

/// The FLAC of `input`, for the reply of the memory path: a `Blob` of its
/// parts, which stay outside WebAssembly memory.
#[cfg_attr(mutants, mutants::skip)]
async fn flac_blob(input: &Input, rate: u32) -> Result<Blob, String> {
    let mut out = FlacOut::Parts(js_sys::Array::new());
    flac_into(input, rate, &mut out).await?;
    let FlacOut::Parts(parts) = out else {
        unreachable!("parts stay parts")
    };
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("audio/flac");
    Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|e| format!("FLAC file: {e:?}"))
}

/// Encodes `input` as a 16-bit FLAC into `out`: the stream header, each
/// frame as soon as it is encoded, then the final stream header in place
/// of the first.  It reads one FLAC frame of `input` at a time, so
/// WebAssembly memory never holds the samples or the file (MEM-1, MEM-4).
#[cfg_attr(mutants, mutants::skip)]
async fn flac_into(input: &Input, rate: u32, out: &mut FlacOut) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Debug| format!("FLAC encoder: {e:?}");
    let frames = input.frames();
    if frames == 0 {
        return Err("FLAC encoder: the stem has no samples".into());
    }
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| fail(&e))?;
    let mut info = StreamInfo::new(rate as usize, 2, 16).map_err(|e| fail(&e))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    // The frame count, the sizes and the MD5 are not known yet.
    Stream::with_stream_info(info.clone())
        .write(&mut sink)
        .map_err(|e| fail(&e))?;
    let header_bytes = sink.as_slice().len();
    out.put(sink.as_slice()).await?;
    let mut block = FrameBuf::with_size(2, FLAC_BLOCK).map_err(|e| fail(&e))?;
    let mut md5 = Context::new(16, 2);
    let (mut l, mut r) = (vec![0.0; FLAC_BLOCK], vec![0.0; FLAC_BLOCK]);
    let mut interleaved = Vec::with_capacity(2 * FLAC_BLOCK);
    for (number, at) in (0..frames).step_by(FLAC_BLOCK).enumerate() {
        let n = FLAC_BLOCK.min(frames - at);
        input.read(at, &mut l[..n], &mut r[..n])?;
        interleaved.clear();
        for (a, b) in l[..n].iter().zip(&r[..n]) {
            interleaved.push(pcm16(*a));
            interleaved.push(pcm16(*b));
        }
        (&mut block, &mut md5)
            .fill_interleaved(&interleaved)
            .map_err(|e| fail(&e))?;
        let frame = flacenc::encode_fixed_size_frame(&config, &block, number, &info)
            .map_err(|e| fail(&e))?;
        info.update_frame_info(&frame);
        sink.clear();
        frame.write(&mut sink).map_err(|e| fail(&e))?;
        out.put(sink.as_slice()).await?;
    }
    info.set_md5_digest(&md5.md5_digest());
    // flacenc gives a short last block as the minimum, which RFC 9639 (8.2)
    // excludes; Symphonia then reads the stream as variable-blocksize, and
    // refuses every frame.
    info.set_block_sizes(FLAC_BLOCK, FLAC_BLOCK)
        .map_err(|e| fail(&e))?;
    sink.clear();
    Stream::with_stream_info(info)
        .write(&mut sink)
        .map_err(|e| fail(&e))?;
    if sink.as_slice().len() != header_bytes {
        return Err(format!(
            "FLAC encoder: the stream header grew from {header_bytes} to {} bytes",
            sink.as_slice().len()
        ));
    }
    out.replace_header(sink.as_slice()).await
}

/// Bytes of this worker's WebAssembly memory, which only ever grows.
fn wasm_bytes() -> f64 {
    super::wasm_bytes()
}

/// Logs `memory: encode <codec> <what>: wasm <before> to <after> bytes,
/// file <bytes> bytes`, and the JS heap where Chrome gives it here (D9 of
/// the M8 plan).  `xtask e2e save` reads it.
#[cfg_attr(mutants, mutants::skip)]
fn log_memory(codec: Codec, what: &str, before: f64, file: f64) {
    let heap = js_heap().map_or(String::new(), |h| format!(", JS heap {h} bytes after gc"));
    log(&format!(
        "memory: encode {} {what}: wasm {before} to {} bytes, file {file} bytes{heap}",
        codec.kind(),
        wasm_bytes()
    ));
}
