//! Browser tests of the decode worker on OPFS, run in a dedicated worker
//! (`cargo run -p xtask -- web-test`).  The tests share one worker and its
//! origin's private file system, so each starts from an empty one.

#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use super::*;
use engine::Song;
use engine::decode::decode_bytes;
use engine::overview::lane_columns;
use std::alloc::{GlobalAlloc, Layout, System};
use stem_protocol::decode::Source;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

thread_local! {
    /// Every message the worker would have posted to the page.
    pub(super) static SENT: RefCell<Vec<JsValue>> = const { RefCell::new(Vec::new()) };
    /// The source that `after_source_open` removes.
    static REMOVE_AFTER_OPEN: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Heap bytes allocated and not yet freed.
    static LIVE: Cell<usize> = const { Cell::new(0) };
    /// The most `LIVE` has been since a test set it.
    static PEAK: Cell<usize> = const { Cell::new(0) };
}

/// Keeps `LIVE` and `PEAK`.  Only `alloc` and `dealloc` are written, so
/// the default `realloc` holds the old and the new buffer at once.
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

/// Removes the source at `path` after it is opened, if a test asked for
/// it: as when the page forgets the song while the decode runs.
pub(super) async fn after_source_open(path: &str) {
    let target = REMOVE_AFTER_OPEN.with_borrow(Clone::clone);
    if target.as_deref() == Some(path) {
        remove_at(path).await;
    }
}

#[wasm_bindgen(inline_js = r#"
async function folder_at(path, create) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split_at(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
export async function wipe() {
  const root = await navigator.storage.getDirectory();
  for await (const name of root.keys()) await root.removeEntry(name, { recursive: true });
}
export async function plant_folder(path) { await folder_at(path, true); }
export async function plant_at(path, bytes) {
  const [d, n] = split_at(path);
  const w = await (await (await folder_at(d, true)).getFileHandle(n, { create: true })).createWritable();
  await w.write(bytes);
  await w.close();
}
// The bytes of the file at `path`, or null if there is none.
export async function read_back(path) {
  try {
    const [d, n] = split_at(path);
    const f = await (await (await folder_at(d, false)).getFileHandle(n)).getFile();
    return new Uint8Array(await f.arrayBuffer());
  } catch (e) { if (e.name === "NotFoundError") return null; throw e; }
}
// The path of every file below `path`, sorted and joined by spaces.
export async function files_below(path) {
  const out = [];
  async function walk(d, at) {
    for await (const [name, h] of d.entries()) {
      if (h.kind === "file") out.push(at + name);
      else await walk(h, at + name + "/");
    }
  }
  await walk(await folder_at(path, false), "");
  return out.sort().join(" ");
}
// Records what each FileSystemSyncAccessHandle.write gets: "whole" or
// "view".
let writes = null;
let real_write = null;
export function start_write_spy() {
  if (real_write) throw new Error("a spy is already on the sync write");
  const proto = FileSystemSyncAccessHandle.prototype;
  real_write = proto.write;
  writes = [];
  proto.write = function (data, options) {
    writes.push((ArrayBuffer.isView(data) && data.byteOffset === 0 && data.byteLength === data.buffer.byteLength ? "whole" : "view") + ":" + data.byteLength);
    return real_write.call(this, data, options);
  };
}
export function stop_write_spy() {
  FileSystemSyncAccessHandle.prototype.write = real_write;
  real_write = null;
  return writes.join(" ");
}
// Records the bytes of each read: FileReaderSync.readAsArrayBuffer of a
// blob, and FileSystemSyncAccessHandle.read into a buffer.  For each
// sync read, `sync_reads` also records "whole" or "view", its bytes, and
// which ArrayBuffer it got (#0 is the first that a read got).
let reads = null;
let real_reads = null;
let sync_reads = null;
export function start_read_spies() {
  if (real_reads) throw new Error("the read spies are already on");
  const file = FileReaderSync.prototype;
  const sync = FileSystemSyncAccessHandle.prototype;
  real_reads = [file.readAsArrayBuffer, sync.read];
  reads = [];
  sync_reads = [];
  const buffers = [];
  file.readAsArrayBuffer = function (blob) {
    reads.push(blob.size);
    return real_reads[0].call(this, blob);
  };
  sync.read = function (buffer, options) {
    reads.push(buffer.byteLength);
    const whole = ArrayBuffer.isView(buffer) && buffer.byteOffset === 0 && buffer.byteLength === buffer.buffer.byteLength;
    const backing = ArrayBuffer.isView(buffer) ? buffer.buffer : buffer;
    if (!buffers.includes(backing)) buffers.push(backing);
    sync_reads.push((whole ? "whole" : "view") + ":" + buffer.byteLength + ":#" + buffers.indexOf(backing));
    return real_reads[1].call(this, buffer, options);
  };
}
export function sync_read_calls() {
  return sync_reads.join(" ");
}
// Makes each FileSystemSyncAccessHandle.read read no bytes, until stopped.
let real_read = null;
export function start_short_reads() {
  if (real_read) throw new Error("short reads are already on");
  real_read = FileSystemSyncAccessHandle.prototype.read;
  FileSystemSyncAccessHandle.prototype.read = function () { return 0; };
}
export function stop_short_reads() {
  FileSystemSyncAccessHandle.prototype.read = real_read;
  real_read = null;
}
export function decoder_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function stop_read_spies() {
  FileReaderSync.prototype.readAsArrayBuffer = real_reads[0];
  FileSystemSyncAccessHandle.prototype.read = real_reads[1];
  real_reads = null;
  return reads;
}
"#)]
extern "C" {
    async fn wipe();
    async fn plant_folder(path: &str);
    async fn plant_at(path: &str, bytes: &[u8]);
    async fn read_back(path: &str) -> JsValue;
    async fn files_below(path: &str) -> JsValue;
    fn start_write_spy();
    fn stop_write_spy() -> String;
    fn start_read_spies();
    fn stop_read_spies() -> Vec<f64>;
    fn sync_read_calls() -> String;
    fn start_short_reads();
    fn stop_short_reads();
    async fn decoder_sleep(ms: u32);
}

const SESSION: &str = "sessions/1792640000000";

/// An empty private file system with the session folder, and no message
/// sent yet.
async fn fresh() {
    wipe().await;
    plant_folder(SESSION).await;
    SENT.with_borrow_mut(Vec::clear);
    REMOVE_AFTER_OPEN.with_borrow_mut(|path| *path = None);
}

/// A request for one source, `src/0.<ext>`, in the mix.
fn one_source(ext: &str) -> DecodeRequest {
    DecodeRequest {
        session: SESSION.into(),
        sources: vec![Source {
            file: format!("src/0.{ext}"),
            index: 0,
            ext: ext.into(),
            in_mix: true,
        }],
        mix_frames: None,
    }
}

async fn decode(req: &DecodeRequest) -> DecodeReply {
    let (msg, _) = req.to_js();
    handle(&msg.into()).await
}

/// The texts of the log lines sent.
fn logs() -> Vec<String> {
    SENT.with_borrow(|sent| {
        sent.iter()
            .filter_map(|msg| match DecodeReply::from_js(msg) {
                Ok(DecodeReply::Log(text)) => Some(text),
                _ => None,
            })
            .collect()
    })
}

async fn file(path: &str) -> Option<Vec<u8>> {
    let bytes = read_back(&format!("{SESSION}/{path}")).await;
    bytes.dyn_into::<Uint8Array>().ok().map(|b| b.to_vec())
}

/// A tone of `frames` frames, different for each `seed`.
fn tone(frames: usize, seed: usize) -> Vec<f32> {
    let step = 0.01 * (seed + 1) as f32;
    (0..frames)
        .map(|i| 0.5 * (step * i as f32).sin() + 0.01)
        .collect()
}

fn wav(frames: usize, rate: u32) -> Vec<u8> {
    engine::wav::encode_pcm16(&tone(frames, 0), &tone(frames, 1), rate)
}

/// A 16-bit stereo FLAC, compressed by `flacenc`, as
/// `engine/src/test_flac.rs` makes it.
fn flac(frames: usize, rate: u32) -> Vec<u8> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    const BLOCK: usize = 4096;
    let step = |x: f32| (f64::from(x) * 32768.0).round().clamp(-32768.0, 32767.0) as i32;
    let (left, right) = (tone(frames, 0), tone(frames, 1));
    let interleaved: Vec<i32> = left
        .iter()
        .zip(&right)
        .flat_map(|(l, r)| [step(*l), step(*r)])
        .collect();
    let source = flacenc::source::MemSource::from_samples(&interleaved, 2, 16, rate as usize);
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .expect("the default config");
    let mut stream =
        flacenc::encode_with_fixed_block_size(&config, source, BLOCK).expect("encoded");
    // flacenc states the short last block as the minimum (TD-52).
    stream
        .stream_info_mut()
        .set_block_sizes(BLOCK, BLOCK)
        .expect("block sizes");
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).expect("written");
    sink.into_inner()
}

fn mp3(frames: usize, rate: u32) -> Vec<u8> {
    let (left, right) = (tone(frames, 0), tone(frames, 1));
    let pcm: Vec<f32> = left
        .iter()
        .zip(&right)
        .flat_map(|(l, r)| [*l, *r])
        .collect();
    let mut encoder = rusty_mp3::Mp3Encoder::new(rusty_mp3::Mp3EncoderConfig {
        bitrate_kbps: 192,
        vbr_quality: None,
    });
    encoder.push_pcm_f32(&pcm, 2, rate).expect("encoded");
    encoder.finish();
    let mut out = Vec::new();
    while let Ok(packet) = encoder.next_packet() {
        out.extend_from_slice(&packet);
    }
    out
}

fn interleaved(left: &[f32], right: &[f32]) -> Vec<u8> {
    left.iter()
        .zip(right)
        .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
        .flatten()
        .collect()
}

fn overview_bytes(cols: &[(f32, f32)]) -> Vec<u8> {
    cols.iter()
        .flat_map(|(lo, hi)| [lo.to_le_bytes(), hi.to_le_bytes()])
        .flatten()
        .collect()
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

#[wasm_bindgen_test]
async fn a_wav_a_flac_and_an_mp3_decode_into_session_files() {
    for (ext, bytes) in [
        ("wav", wav(30_007, 48_000)),
        ("flac", flac(30_007, 44_100)),
        ("mp3", mp3(30_007, 44_100)),
    ] {
        fresh().await;
        plant_at(&format!("{SESSION}/src/0.{ext}"), &bytes).await;
        let reply = decode(&one_source(ext)).await;
        let whole = decode_bytes(bytes, Some(ext)).unwrap();
        let channels = whole.channels.len() as u8;
        let song = Song::from_audio(vec![("source".into(), whole)]).unwrap();
        let stem = &song.stems[0];
        assert_eq!(
            reply,
            DecodeReply::Decoded {
                frames: song.frames as u64,
                new_mix: true,
                channels: vec![channels],
            },
            "{ext}"
        );
        let mix = file("mix.f32.decode.part").await.expect("the mix part");
        assert!(
            mix == interleaved(&stem.left, &stem.right),
            "{ext}: the mix"
        );
        let (cols, _) = lane_columns(&stem.left, &stem.right);
        let overview = file("overviews/0.f32.decode.part").await;
        assert!(
            overview == Some(overview_bytes(&cols)),
            "{ext}: its overview"
        );
        assert_eq!(
            files_below(SESSION).await.as_string().unwrap_or_default(),
            format!(
                "mix.f32.decode.part overviews/0.f32.decode.part overviews/mix.f32.decode.part src/0.{ext}"
            ),
            "{ext}"
        );
        let lines = logs();
        assert!(
            lines.len() == 2
                && lines[0].starts_with("decoder: memory: decoder wasm ")
                && lines[0].ends_with(" MB at the decode's start")
                && lines[1].ends_with(" MB at the decode's end"),
            "{ext}: {lines:?}"
        );
    }
}

#[wasm_bindgen_test]
async fn decode_writes_pass_whole_arrays() {
    fresh().await;
    // More than a block of 131,072 frames.
    plant_at(&format!("{SESSION}/src/0.wav"), &wav(200_003, 44_100)).await;
    start_write_spy();
    let reply = decode(&one_source("wav")).await;
    let writes = stop_write_spy();
    assert!(
        matches!(
            reply,
            DecodeReply::Decoded {
                frames: 200_003,
                ..
            }
        ),
        "{reply:?}"
    );
    let calls: Vec<(&str, usize)> = writes
        .split(' ')
        .filter_map(|call| call.split_once(':'))
        .map(|(kind, bytes)| (kind, bytes.parse().unwrap_or(0)))
        .collect();
    assert!(calls.iter().all(|(kind, _)| *kind == "whole"), "{writes}");
    // An overview holds at most 32 KiB; the mix of 200,003 frames is two
    // blocks, so more than one call writes it.
    let mix_writes = calls.iter().filter(|(_, bytes)| *bytes > 32 << 10).count();
    assert!(mix_writes >= 2, "more than one write of the mix: {writes}");
}

/// The peak of the heap above its start while `request` decodes.
async fn decode_peak(req: &DecodeRequest) -> (usize, DecodeReply) {
    warm_up();
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    let reply = decode(req).await;
    (PEAK.with(Cell::get) - before, reply)
}

#[wasm_bindgen_test]
async fn a_decode_keeps_the_worker_memory_flat() {
    const MIB: usize = 1 << 20;
    let (short, long) = (wav(2 * 44_100, 44_100), wav(20 * 44_100, 44_100));

    fresh().await;
    plant_at(&format!("{SESSION}/src/0.wav"), &short).await;
    let (short_peak, reply) = decode_peak(&one_source("wav")).await;
    assert!(matches!(reply, DecodeReply::Decoded { .. }), "{reply:?}");

    fresh().await;
    plant_at(&format!("{SESSION}/src/0.wav"), &long).await;
    start_read_spies();
    let (long_peak, reply) = decode_peak(&one_source("wav")).await;
    let reads = stop_read_spies();
    assert!(
        matches!(reply, DecodeReply::Decoded { frames, .. } if frames == 20 * 44_100),
        "{reply:?}"
    );
    assert!(
        long_peak.abs_diff(short_peak) < MIB,
        "the peak does not grow with the song: {short_peak} bytes for 2 s, {long_peak} for 20 s"
    );
    assert!(reads.len() > 10, "{} reads", reads.len());
    assert!(
        reads.iter().all(|bytes| *bytes <= MIB as f64),
        "each read is at most 1 MiB: {reads:?}"
    );
    // Findings 2-10 and 2-13 of review round 2: each read of the second
    // pass gets a whole array, and the reads of a whole block share one,
    // so a read makes no garbage.
    let sync = sync_read_calls();
    let calls: Vec<(&str, &str, &str)> = sync
        .split(' ')
        .filter_map(|call| {
            let mut parts = call.splitn(3, ':');
            Some((parts.next()?, parts.next()?, parts.next()?))
        })
        .collect();
    assert!(calls.iter().all(|(kind, _, _)| *kind == "whole"), "{sync}");
    let blocks: Vec<&str> = calls
        .iter()
        .filter(|(_, bytes, _)| *bytes == MIB.to_string())
        .map(|(_, _, buffer)| *buffer)
        .collect();
    assert!(blocks.len() >= 6, "the reads of whole blocks: {sync}");
    assert!(blocks.iter().all(|b| *b == blocks[0]), "one array: {sync}");

    // The control: a whole decode of the long file holds it.
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    let whole = decode_bytes(long, Some("wav")).unwrap();
    let whole_peak = PEAK.with(Cell::get) - before;
    drop(whole);
    assert!(
        whole_peak > short_peak + MIB,
        "the control peaks {whole_peak} bytes, against {short_peak}"
    );
}

#[wasm_bindgen_test]
async fn a_removed_source_ends_with_unstored() {
    fresh().await;
    let path = format!("{SESSION}/src/0.wav");
    plant_at(&path, &wav(30_000, 44_100)).await;
    REMOVE_AFTER_OPEN.with_borrow_mut(|remove| *remove = Some(path.clone()));
    let reply = decode(&one_source("wav")).await;
    assert!(
        matches!(&reply, DecodeReply::Unstored { text } if text.contains("src/0.wav: bytes")),
        "{reply:?}"
    );
    assert_eq!(
        files_below(SESSION).await.as_string().unwrap_or_default(),
        "",
        "no part file is left, and the source is gone"
    );
}

/// Finding 2-1 of review round 2: a short read of a session file ends with
/// `decode-unstored`, whose text gives the bytes read and the bytes asked
/// for, and leaves no part file.
#[wasm_bindgen_test]
async fn a_short_read_ends_with_unstored_and_its_sizes() {
    fresh().await;
    plant_at(&format!("{SESSION}/src/0.wav"), &wav(2_000, 44_100)).await;
    start_short_reads();
    let reply = decode(&one_source("wav")).await;
    stop_short_reads();
    assert!(
        matches!(&reply, DecodeReply::Unstored { text } if text.contains("read 0 of 16000 bytes")),
        "{reply:?}"
    );
    assert_eq!(
        files_below(SESSION).await.as_string().unwrap_or_default(),
        "src/0.wav",
        "no part file is left"
    );
}

#[wasm_bindgen_test]
async fn a_malformed_request_ends_with_unstored() {
    fresh().await;
    let msg = js_sys::Object::new();
    js_sys::Reflect::set(&msg, &"type".into(), &"decode".into()).unwrap();
    let reply = handle(&msg.into()).await;
    assert!(
        matches!(&reply, DecodeReply::Unstored { text } if text.contains("could not be read") && text.contains("session")),
        "{reply:?}"
    );
}

/// A request for `src/0.wav` of session `id`.
fn request_of(id: &str) -> DecodeRequest {
    DecodeRequest {
        session: format!("sessions/{id}"),
        ..one_source("wav")
    }
}

/// D9: the worker takes the requests one at a time, in the order sent, and
/// ends each with one reply.
#[wasm_bindgen_test]
async fn the_queue_takes_each_request_in_turn() {
    fresh().await;
    let sessions = [
        ("1", wav(2_000, 44_100)),
        ("2", b"not audio".to_vec()),
        ("3", wav(3_000, 44_100)),
    ];
    for (id, bytes) in &sessions {
        plant_folder(&format!("sessions/{id}")).await;
        plant_at(&format!("sessions/{id}/src/0.wav"), bytes).await;
    }
    warm_up();
    for (id, _) in &sessions {
        let (msg, _) = request_of(id).to_js();
        enqueue(msg.into());
    }
    let ended = |sent: &[JsValue]| {
        sent.iter()
            .filter_map(|m| DecodeReply::from_js(m).ok())
            .filter(|r| !matches!(r, DecodeReply::Log(_) | DecodeReply::Alive))
            .count()
    };
    for _ in 0..250 {
        if SENT.with_borrow(|sent| ended(sent)) == 3 {
            break;
        }
        decoder_sleep(20).await;
    }
    let replies: Vec<DecodeReply> = SENT.with_borrow(|sent| {
        sent.iter()
            .filter_map(|m| DecodeReply::from_js(m).ok())
            .collect()
    });
    let kinds: Vec<String> = replies
        .iter()
        .map(|r| match r {
            DecodeReply::Log(text) if text.ends_with("at the decode's start") => "start".into(),
            DecodeReply::Log(text) if text.ends_with("at the decode's end") => "end".into(),
            DecodeReply::Decoded { frames, .. } => format!("decoded {frames}"),
            other => other.kind().to_string(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "start",
            "end",
            "decoded 2000",
            "start",
            "end",
            "undecodable",
            "start",
            "end",
            "decoded 3000"
        ],
        "one request at a time, in the order sent"
    );
    assert!(!RUNNING.get(), "the queue stops when it is empty");
}
