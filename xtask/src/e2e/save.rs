//! `xtask e2e save` (step 4 of `docs/plans/completed/stream-from-storage-m8.md`):
//! a save of the mix holds no song-length buffer in the page's JS heap.
//!
//! The scenario plants a stored session of 1 minute and one of 10 minutes,
//! each a 16-bit WAV source and two 16-bit stem WAVs, in the layout of
//! `old_session`.  For each format, WAV, then FLAC, then MP3, it restores
//! the 1-minute session and saves the mix as heard through the mix lane's
//! Save menu, then the same for the 10-minute session.  Each restore is a
//! reload, so each save has a new page and new workers.
//!
//! The measure (D9 of the M8 plan): Chrome runs with
//! `--enable-precise-memory-info` and `--js-flags=--expose-gc`, so
//! `performance.memory.usedJSHeapSize` reads V8's used heap and the backing
//! stores of `ArrayBuffer`s at each call.  An injected script
//! (`xtask/e2e/save_spy.js`) reads the value before the save, at each
//! write of the page to browser storage, and once at its end, so it sees
//! the whole save (review round speed 5 of M8).  Each reading collects
//! garbage and waits for wasm-bindgen's finalizers first.  The measure is
//! the largest value less the value before.  Each save's measure must be
//! within [`heap_budget`], and the 10-minute save's at most
//! [`PAGE_BOUND_MB`] above the 1-minute save's.  The end of a save is the
//! click of its download link, which the script keeps instead of downloading:
//! every build makes it.  A calibration comes first: a 64 MiB `Uint8Array`
//! must raise the measure by at least 60 MiB.
//!
//! The worker's part: each encode logs a `memory: encode` line with the
//! worker's WebAssembly memory before and after, and the file's size.
//!
//! Checks (D10): for each format, the 10-minute save's measure is at most
//! [`PAGE_BOUND_MB`] above the 1-minute save's.  A FLAC encode of 10
//! minutes grows the worker's memory by at most 1 MiB more than one of 1
//! minute.  An MP3 encode grows it by at most [`MP3_FACTOR`] times its MP3
//! plus 1 MiB (the MP3 exception, TD-59).  The 1-minute WAV is the 16-bit
//! mix of the planted stems, computed here.  The 1-minute FLAC decodes to
//! the same samples, and the MP3 to the same frames within one MP3 frame
//! and the encoder's delay.
//!
//! Checks of D11 (a save that does not wait for the UI frame): for each
//! format, a 10-minute save grows the page's WebAssembly memory by at most
//! [`WASM_BOUND_BYTES`] more than a 1-minute save.  The 10-minute WAV save
//! takes fewer animation frames than a quarter of its blocks, with the
//! animation frames throttled to about 4 a second.  Each save's longest
//! slice, and its longest end of a read, which the page logs, are at most
//! [`SLICE_LIMIT_MS`].  A seventh save saves the 10-minute mix as a WAV
//! while the song plays, with animation frames of [`LONG_FRAME_MS`], as
//! CI's paints in software.  It fails on an underrun, on frames of another
//! time, and when the save takes more than [`PLAYING_SAVE_MS`].  The main
//! thread's long tasks are logged, not checked.
//!
//! At the end, `saves/` holds only the offered files.  It prints every
//! measure, and runs every check also after one failed, so a run of an
//! older build shows its numbers.

use super::old_session::PLANT;
use super::playback::{PlaybackLine, playback_line};
use super::{Opts, Page, in_page, number_after, sha256_hex};
use anyhow::{Result, bail};
use serde_json::{Value, json};

/// The lengths of the two sessions, in seconds.
const SECONDS: [u32; 2] = [60, 600];
/// The planted stems.  `old_session`'s plant makes stem `k` from a sine of
/// `0.002 x (k + 1)` radians a frame.
const STEMS: [&str; 2] = ["bass", "drums"];
/// The formats, in the order of the saves, with their items' rows in the
/// Save menu.
const FORMATS: [(&str, usize); 3] = [("wav", 0), ("flac", 1), ("mp3", 2)];
/// D10: how much more the 10-minute save's JS heap measure may be than
/// the 1-minute save's, in MB.
pub(super) const PAGE_BOUND_MB: f64 = 4.0;
/// D10: how much more a FLAC encode of 10 minutes may grow the worker's
/// memory than one of 1 minute, in bytes.
pub(super) const FLAC_BOUND_BYTES: f64 = ONE_MIB;
/// D10: a worker's MP3 encode grows its memory by at most this many times
/// its MP3, plus [`ONE_MIB`].
pub(super) const MP3_FACTOR: f64 = 4.0;
const ONE_MIB: f64 = 1_048_576.0;
/// D11: how much more a 10-minute save may grow the page's WebAssembly
/// memory than a 1-minute save, in bytes.  The save's reader holds 3 spans
/// of 65,536 frames of each lane (`SAVE_AHEAD`), whatever the song's
/// length.  So the two saves need the same memory.
pub(super) const WASM_BOUND_BYTES: f64 = ONE_MIB;
/// The limit of the `e2e save` step in CI's `browser-tests` job
/// (`timeout-minutes`), in seconds.  It is the longest that every wait of a
/// run can take (finding 3-1 of review round 3 of M8).  So a run that
/// waits in vain still ends on a check of its own.  A step that CI stops
/// writes no `save.log`, but each note of the run also goes to the step's
/// output.  Since review round speed 4 of M8, a run takes about 3 minutes
/// on the maintainer's Mac (174 to 176 s), in Chrome and with
/// `--swiftshader`, and 3.3 minutes on CI (run 40894815433).
pub(super) const STEP_LIMIT_S: u64 = 40 * 60;
/// The longest that a save of `seconds` of audio may take from its click to its
/// offer, in seconds: 6 times or more the slowest save of its length on the
/// maintainer's Mac with `--swiftshader`, after D11 of M8 (2026-10-29).  Those
/// were FLAC saves of 3.1 s for 60 s of audio, and of 28.7 s for 600 s; the
/// worker's encode takes most of each.  Before D11 of M8, a save took one UI
/// frame for each block, and CI took 34 s for 60 s of audio.  Since D11, a save
/// does not wait for the UI frame, and CI ran the worker's encode faster than
/// the Mac (`e2e flac`).
pub(super) const fn save_cap_s(seconds: u32) -> u64 {
    if seconds <= 60 { 60 } else { 180 }
}
/// The longest that a save may show no progress, in seconds: no write of
/// the page to browser storage and no growth of the save files.  An MP3
/// encode writes nothing until its encoder finishes.  The MP3 save of 10
/// minutes took 16.6 s in all on the maintainer's Mac with `--swiftshader`.
pub(super) const STALL_S: u64 = 60;
/// How often the wait for an offer reads the save's progress, in seconds.
const POLL_S: u64 = 1;
/// The longest wait, in seconds, for a restore and for its stem overviews.
/// In CI run 40788465048, the first restores of the 1-minute and the
/// 10-minute sessions took 1.0 s and 5.0 s, and a later 1-minute restore
/// 0.6 s.  So 30 s leaves 6 times the longest.
pub(super) const RESTORE_WAIT_S: u64 = 30;
/// The longest wait, in seconds, for the app to start.  It started within
/// 0.6 s in CI run 40788465048.
pub(super) const START_WAIT_S: u64 = 60;
/// The longest wait, in seconds, for the plant of a session.  In CI run
/// 40788465048, the plant of the 10-minute session took 4.4 s (1.2 s to
/// 5.6 s).  So 60 s leaves 13 times that.
pub(super) const PLANT_WAIT_S: u64 = 60;
/// The longest wait, in seconds, for the read of a 1-minute file.  In CI run
/// 40788465048, the read of the WAV's SHA-256 took less than 0.1 s.  The
/// FLAC and the MP3 (about 1.4 MB each) are read whole, as base64, which
/// that run did not reach.  So 30 s leaves a wide margin.
pub(super) const READ_WAIT_S: u64 = 30;
/// The longest wait, in seconds, for the removal of the sessions and the
/// save files.  In CI run 40788465048, the first clean-up and the plant of
/// the 1-minute session took 0.4 s together.  The last clean-up removes
/// about 330 MB, which that run did not reach.
pub(super) const CLEAN_WAIT_S: u64 = 30;
/// The longest wait, in seconds, for a short script: a session switch, the
/// calibration, a heap reading, a reading of the progress or the list of
/// save files.
pub(super) const SCRIPT_WAIT_S: u64 = 15;
/// The idle time of each save, in seconds: 3 s before it, 0.5 s with the
/// menu open and 1 s after it, rounded up.
const SAVE_IDLE_S: u64 = 5;
/// The longest wait, in seconds, for a `playback:` line, which the page
/// logs every 2 s while it plays.
pub(super) const PLAY_WAIT_S: u64 = 30;
/// D11: the longest slice of a save's task, in milliseconds, as the page
/// logs it at the save's end.  cpal queues two buffers of 3,072 frames.
/// When one ends, only the other is queued: about 70 ms at 44.1 kHz, and
/// 64 ms at 48 kHz.  So a save that holds the main thread for about that
/// long can starve the audio output.  The bound is about half of it.  A
/// slice is its budget, 4 ms or up to 16 ms after a long wait, and one
/// block of work, about 0.5 ms on the maintainer's Mac.
pub(super) const SLICE_LIMIT_MS: f64 = 35.0;
/// The time of each animation frame during the save during playback, in
/// milliseconds.  In CI run 40838459678, each paint in software was a
/// long task of about 121 to 125 ms.  The page repaints in each frame
/// while it plays, so each task waited for a frame, and the save of 10
/// minutes did not end within 180 s.  With these frames, the build of that
/// run failed in the same way on the maintainer's Mac (review round speed
/// 3 of M8).  The spy holds each frame only for the rest of this time, so
/// a runner that paints in software gets frames of about this time too,
/// not of its paint and this time (review round speed 4).
pub(super) const LONG_FRAME_MS: f64 = 125.0;
/// The longest that the save during playback may take, from its click to
/// its offer, in milliseconds.  It is about twice the time of the save on
/// the maintainer's Mac with [`LONG_FRAME_MS`] (56.25 s), and of the
/// estimate for CI (review round speed 4 of M8).
pub(super) const PLAYING_SAVE_MS: u64 = 120_000;
/// The saves of a run, and the first restores of its sessions.
const SAVES: u64 = (FORMATS.len() * SECONDS.len()) as u64;
const FIRST_RESTORES: u64 = SECONDS.len() as u64;
/// The caps of all the saves of a run together.
const fn caps_s() -> u64 {
    let mut sum = 0;
    let mut i = 0;
    while i < SECONDS.len() {
        sum += save_cap_s(SECONDS[i]);
        i += 1;
    }
    sum * FORMATS.len() as u64
}
/// The longest that all the waits of a run can take together (finding 2-2 of
/// review round 2 of M8).  These are the start, the two calibrations, the two
/// clean-ups, and the plant of each session.  Each first restore waits for a
/// session switch, the restore and the stem overviews.  Each save waits for a
/// session switch, the restore, two heap readings and two readings of the
/// WebAssembly memory, and idles.  It waits up to its cap and one reading of
/// its progress for its offer.  The 1-minute save of each format reads its
/// file.  The save during playback waits for a session switch, the restore, two
/// `playback:` lines, its cap, one reading of its progress, the read of its
/// long tasks and the read of its frames, and idles.  The run ends with the
/// list of save files.  It does not count the WebDriver commands of a click, a
/// screenshot, a mark or the start of the long frames, which return at once.
pub(super) const WAITS_S: u64 = START_WAIT_S
    + 2 * SCRIPT_WAIT_S
    + 2 * CLEAN_WAIT_S
    + FIRST_RESTORES * PLANT_WAIT_S
    + FIRST_RESTORES * (SCRIPT_WAIT_S + 2 * RESTORE_WAIT_S)
    + SAVES * (SCRIPT_WAIT_S + RESTORE_WAIT_S + 5 * SCRIPT_WAIT_S + SAVE_IDLE_S)
    + caps_s()
    + FORMATS.len() as u64 * READ_WAIT_S
    + 4 * SCRIPT_WAIT_S
    + RESTORE_WAIT_S
    + SAVE_IDLE_S
    + 2 * PLAY_WAIT_S
    + save_cap_s(SECONDS[1])
    + SCRIPT_WAIT_S;
// All the waits together end within the step's limit.
const _: () = assert!(WAITS_S < STEP_LIMIT_S);
/// The calibration array, and the least rise of the measure that it must
/// make.
const CALIBRATION_BYTES: u64 = 64 << 20;
const CALIBRATION_LEAST: f64 = (60 << 20) as f64;
/// Frames of one MP3 frame, and the most frames that an MP3's length may
/// differ by from its WAV's: one MP3 frame and the encoder's delay.
const MP3_FRAME: usize = 1152;
pub(super) const MP3_FRAMES_TOLERANCE: usize = 2 * MP3_FRAME;

/// The id of the session of `seconds`.
fn session_id(seconds: u32) -> String {
    format!("{}", 1_792_640_100_000u64 + u64::from(seconds))
}

/// Makes session `args[0]` current, for the next reload.
const MAKE_CURRENT: &str = "const [id] = args;
    const root = await navigator.storage.getDirectory();
    const sessions = await root.getDirectoryHandle('sessions', { create: true });
    const w = await (await sessions.getFileHandle('current', { create: true })).createWritable();
    await w.write(id); await w.close();
    return id;";

/// Removes the sessions and the save files that the scenario made.
const CLEAN: &str = "const root = await navigator.storage.getDirectory();
    for (const name of ['sessions', 'saves']) {
      try { await root.removeEntry(name, { recursive: true }); }
      catch (e) { if (e.name !== 'NotFoundError') throw e; }
    }
    return true;";

/// The names in `saves/`, sorted, or null.
const SAVE_NAMES: &str = "try {
      const d = await (await navigator.storage.getDirectory()).getDirectoryHandle('saves');
      const out = [];
      for await (const name of d.keys()) out.push(name);
      return out.sort();
    } catch (e) { if (e.name === 'NotFoundError') return null; throw e; }";

/// The measure of the calibration: the rise of the JS heap measure for a
/// 64 MiB array.
const CALIBRATE: &str = "const [bytes] = args;
    const before = await window.__e2e_heap();
    window.__e2e_calibration = new Uint8Array(bytes);
    window.__e2e_calibration.fill(1);
    const after = await window.__e2e_heap();
    window.__e2e_calibration = null;
    await window.__e2e_heap();
    return before === null || after === null ? null : after - before;";

/// The last offered file: its SHA-256 (hex), its first bytes as text, and
/// its bytes as base64 when `args[0]`.
const LAST_FILE: &str = "const [whole] = args;
    const b = window.__e2e_last;
    if (!b) return null;
    const bytes = new Uint8Array(await b.arrayBuffer());
    const d = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const sha256 = Array.from(d, (x) => x.toString(16).padStart(2, '0')).join('');
    const head = String.fromCharCode(...bytes.subarray(0, 4));
    let base64 = null;
    if (whole) {
      let s = '';
      for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
      base64 = btoa(s);
    }
    return { sha256, head, size: bytes.length, base64 };";

/// One save: its format and length, its JS heap measure, what was offered,
/// and the worker's memory line.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Save {
    pub format: String,
    pub seconds: u32,
    /// The largest JS heap value during the save, less the value before,
    /// in bytes.
    pub heap: Option<f64>,
    /// The offered file's name, size and type.
    pub offered: Option<(String, f64, String)>,
    /// The worker's memory growth during the encode, and the file's size,
    /// in bytes.
    pub worker: Option<(f64, f64)>,
    /// The growth of the page's WebAssembly memory during the save, in
    /// bytes.
    pub wasm: Option<f64>,
    /// The page's animation frames (`requestAnimationFrame` callbacks)
    /// from the click of the menu item to the offer.
    pub frames: Option<u64>,
    /// The save's longest slice, from the page's line, in milliseconds.
    pub slice_ms: Option<f64>,
    /// The longest end of a read in the save's reader, from the page's
    /// line, in milliseconds.
    pub read_end_ms: Option<f64>,
    /// The JS heap readings of the save.
    pub readings: Option<u64>,
}

/// The worker's growth and the file's size, from a `memory: encode` line:
/// `memory: encode <codec> <what>: wasm <before> to <after> bytes, file
/// <bytes> bytes`.
pub(super) fn encode_line(line: &str) -> Option<(f64, f64)> {
    let at = line.find("memory: encode ")?;
    let rest = &line[at..];
    let before = number_after(rest, ": wasm ")?;
    let after = number_after(rest, " to ")?;
    let file = number_after(rest, ", file ")?;
    Some((after - before, file))
}

/// The save of `format` and `seconds` among `saves`.
fn find<'a>(saves: &'a [Save], format: &str, seconds: u32) -> Option<&'a Save> {
    saves
        .iter()
        .find(|s| s.format == format && s.seconds == seconds)
}

const MB: f64 = 1e6;

/// The checks of D10 on the saves, each with what it checks.
pub(super) fn save_checks(saves: &[Save]) -> Vec<(bool, String)> {
    let mut checks = Vec::new();
    for (format, _) in FORMATS {
        let heap = |seconds| find(saves, format, seconds).and_then(|s| s.heap);
        let (short, long) = (heap(SECONDS[0]), heap(SECONDS[1]));
        let growth = short.zip(long).map(|(s, l)| (l - s) / MB);
        let wasm = |seconds| find(saves, format, seconds).and_then(|s| s.wasm);
        let (short_wasm, long_wasm) = (wasm(SECONDS[0]), wasm(SECONDS[1]));
        checks.push((
            short_wasm
                .zip(long_wasm)
                .is_some_and(|(s, l)| l - s <= WASM_BOUND_BYTES),
            format!(
                "{format}: the 10-minute save grows the page's WebAssembly memory by at most {WASM_BOUND_BYTES} bytes more than the 1-minute save ({short_wasm:?} and {long_wasm:?} bytes)"
            ),
        ));
        checks.push((
            growth.is_some_and(|g| g <= PAGE_BOUND_MB),
            format!(
                "{format}: the 10-minute save's JS heap measure is at most {PAGE_BOUND_MB} MB above the 1-minute save's ({:?} MB and {:?} MB: {growth:?} MB more)",
                short.map(|v| v / MB),
                long.map(|v| v / MB)
            ),
        ));
        for seconds in SECONDS {
            let offered = find(saves, format, seconds).and_then(|s| s.offered.clone());
            checks.push((
                offered
                    .as_ref()
                    .is_some_and(|(name, _, _)| name.ends_with(&format!(".{format}"))),
                format!("{format}, {seconds} s: a .{format} file is offered ({offered:?})"),
            ));
        }
    }
    // Only the WAV: a FLAC's or an MP3's offer waits for the worker's
    // encode too, which takes about 28 s and 16 s for 10 minutes.
    let frames = find(saves, "wav", SECONDS[1]).and_then(|s| s.frames);
    checks.push((
        not_frame_bound(frames, SECONDS[1]),
        format!(
            "wav, 600 s: the save takes fewer animation frames, throttled to about 4 a second, than a quarter of its {} blocks ({frames:?})",
            blocks(SECONDS[1])
        ),
    ));
    for save in saves {
        let what = format!("{}, {} s", save.format, save.seconds);
        checks.push(slice_check(&what, save.slice_ms));
        checks.push(read_end_check(&what, save.read_end_ms));
    }
    for save in saves {
        let what = format!("{}, {} s", save.format, save.seconds);
        let least = full_writes(save.seconds) + 2;
        checks.push((
            save.readings.is_some_and(|r| r >= least),
            format!(
                "{what}: the JS heap is read at each write and at the end, {least} times or more ({:?})",
                save.readings
            ),
        ));
        checks.push((
            save.heap.is_some_and(|h| h <= heap_budget()),
            format!(
                "{what}: the JS heap measure is at most {:.2} MB, the save's arrays and a margin ({:?} MB)",
                heap_budget() / MB,
                save.heap.map(|h| h / MB)
            ),
        ));
    }
    let worker = |format, seconds| find(saves, format, seconds).and_then(|s| s.worker);
    let (flac_short, flac_long) = (worker("flac", SECONDS[0]), worker("flac", SECONDS[1]));
    let flac = flac_short.zip(flac_long).map(|((s, _), (l, _))| l - s);
    checks.push((
        flac.is_some_and(|g| g <= FLAC_BOUND_BYTES),
        format!(
            "flac: the worker's growth for 10 minutes is at most {FLAC_BOUND_BYTES} bytes above that for 1 minute ({flac_short:?}, {flac_long:?})"
        ),
    ));
    for seconds in SECONDS {
        let mp3 = worker("mp3", seconds);
        checks.push((
            mp3.is_some_and(|(growth, file)| file > 0.0 && growth <= MP3_FACTOR * file + ONE_MIB),
            format!(
                "mp3, {seconds} s: the worker grows by at most {MP3_FACTOR} times its MP3 plus 1 MiB ({mp3:?}, a factor of {:?})",
                mp3.map(|(g, f)| g / f)
            ),
        ));
    }
    checks
}

/// The 16-bit samples of the mix of the planted stems at `seconds`, as
/// `old_session`'s plant makes them, interleaved.  Each stem's sample is
/// `round(3000 sin(0.002 (k + 1) i))` on the left and half of it, rounded,
/// on the right, rounded as JavaScript rounds (half up).  The mix of two
/// stems of 16-bit samples is their sum, which never reaches full scale.
pub(super) fn planted_mix(seconds: u32) -> Vec<i16> {
    let frames = seconds as usize * 44_100;
    let js_round = |x: f64| (x + 0.5).floor();
    let mut out = Vec::with_capacity(2 * frames);
    for i in 0..frames {
        let (mut left, mut right) = (0.0, 0.0);
        for k in 1..=STEMS.len() {
            let v = js_round(3000.0 * (i as f64 * 0.002 * k as f64).sin());
            left += v;
            right += js_round(v / 2.0);
        }
        out.push(left as i16);
        out.push(right as i16);
    }
    out
}

/// The WAV that a save of the planted mix of `seconds` gives.
pub(super) fn planted_wav(seconds: u32) -> Vec<u8> {
    let samples = planted_mix(seconds);
    let to_f32 = |s: &i16| f32::from(*s) / 32_768.0;
    let left: Vec<f32> = samples.iter().step_by(2).map(to_f32).collect();
    let right: Vec<f32> = samples.iter().skip(1).step_by(2).map(to_f32).collect();
    engine::wav::encode_pcm16(&left, &right, 44_100)
}

/// The 1-minute files that the run reads back for [`file_checks`].
#[derive(Debug, Default, PartialEq)]
pub(super) struct Files {
    /// The SHA-256 of the WAV, in hex.
    pub wav_sha256: Option<String>,
    pub flac: Option<Vec<u8>>,
    pub mp3: Option<Vec<u8>>,
}

/// Whether the run reads back the file of a save of `format` and `seconds`,
/// and if so, whether whole (`true`, to decode it) or only its SHA-256.
pub(super) fn reads_back(format: &str, seconds: u32) -> Option<bool> {
    (seconds == SECONDS[0]).then_some(format != "wav")
}

impl Files {
    /// Keeps what [`LAST_FILE`] gave for the save of `format`.
    pub(super) fn keep(&mut self, format: &str, file: &Value) {
        let bytes = file["base64"]
            .as_str()
            .and_then(|b| crate::webdriver::base64_decode(b).ok());
        match format {
            "wav" => self.wav_sha256 = file["sha256"].as_str().map(String::from),
            "flac" => self.flac = bytes,
            _ => self.mp3 = bytes,
        }
    }
}

/// Whether `audio` holds the interleaved 16-bit `samples`, exactly.
pub(super) fn decodes_to(audio: &engine::decode::Audio, samples: &[i16]) -> bool {
    let frames = samples.len() / 2;
    audio.frames() == frames
        && audio.channels.len() == 2
        && (0..frames).all(|i| {
            (0..2).all(|c| audio.channels[c][i] == f32::from(samples[2 * i + c]) / 32_768.0)
        })
}

/// Whether an MP3 of `frames` frames has the length of a WAV of `wav`
/// frames, within [`MP3_FRAMES_TOLERANCE`].
pub(super) fn mp3_frames_ok(frames: Option<usize>, wav: usize) -> bool {
    frames.is_some_and(|n| n.abs_diff(wav) <= MP3_FRAMES_TOLERANCE)
}

/// The check of the calibration: the rise of the measure for a 64 MiB
/// array is at least [`CALIBRATION_LEAST`].
pub(super) fn calibration_check(rise: Option<f64>) -> (bool, String) {
    (
        rise.is_some_and(|r| r >= CALIBRATION_LEAST),
        format!("the measure sees a 64 MiB array ({rise:?} bytes)"),
    )
}

/// The y of the Save menu's item at `row`.
pub(super) fn item_y(o: &Opts, row: usize) -> f64 {
    o.item_y + o.item_dy * row as f64
}

/// The rise of the JS heap during a save: the largest reading less the
/// reading before.
pub(super) fn heap_rise(max: &Value, before: &Value) -> Option<f64> {
    max.as_f64().zip(before.as_f64()).map(|(m, b)| m - b)
}

/// The name, size and type of the offered file of an `e2e: offered` line.
pub(super) fn offered_file(line: &str) -> Option<(String, f64, String)> {
    let json: Value = serde_json::from_str(line.split("e2e: offered ").nth(1)?).ok()?;
    Some((
        json["name"].as_str()?.to_string(),
        json["size"].as_f64()?,
        json["mime"].as_str()?.to_string(),
    ))
}

/// The animation frames (`requestAnimationFrame` callbacks) of the page from
/// the click to the offer, from an `e2e: offered` line.
pub(super) fn offered_frames(line: &str) -> Option<u64> {
    let json: Value = serde_json::from_str(line.split("e2e: offered ").nth(1)?).ok()?;
    json["frames"].as_u64()
}

/// The milliseconds from the click to the offer, from an `e2e: offered`
/// line.
pub(super) fn offered_ms(line: &str) -> Option<u64> {
    let json: Value = serde_json::from_str(line.split("e2e: offered ").nth(1)?).ok()?;
    json["ms"].as_u64()
}

/// The offered file of a save, from the line that the wait found.  It is
/// an error when there is none, so the run ends (finding 1-4 of review
/// round 1 of M8).
pub(super) fn offer_of(
    line: Option<&str>,
    format: &str,
    seconds: u32,
) -> Result<(String, f64, String)> {
    match line.and_then(offered_file) {
        Some(file) => Ok(file),
        None => {
            bail!("{format}, {seconds} s: the save offered no file that it could read ({line:?})")
        }
    }
}

/// What the wait for an offer decides from the readings of a save's
/// progress so far.
#[derive(Debug, PartialEq)]
pub(super) enum Wait {
    /// The save goes on: wait more.
    Going,
    /// No reading changed for [`STALL_S`].
    Stalled,
    /// The save is past its cap ([`save_cap_s`]).
    OverCap,
}

/// Decides the wait for an offer at `now` (seconds since the click), from
/// `readings` of the save's progress, each its time and its value.  The
/// save stalls when no reading changed for `stall_s` since the last change,
/// or since the click (finding 1 of the CI run of 2026-10-28).
pub(super) fn judge(readings: &[(f64, u64)], now: f64, stall_s: f64, cap_s: f64) -> Wait {
    if now >= cap_s {
        return Wait::OverCap;
    }
    let last_change = readings
        .windows(2)
        .rev()
        .find(|w| w[1].1 != w[0].1)
        .map_or(0.0, |w| w[1].0);
    if now - last_change >= stall_s {
        Wait::Stalled
    } else {
        Wait::Going
    }
}

/// The wait's decision for a save of `seconds` of audio, with its stall
/// time ([`STALL_S`]) and its cap ([`save_cap_s`]).
pub(super) fn save_wait(readings: &[(f64, u64)], now: f64, seconds: u32) -> Wait {
    judge(readings, now, STALL_S as f64, save_cap_s(seconds) as f64)
}

/// Waits for the save to offer its file, from the log at `mark`: the
/// offer's line.  It reads the save's progress every [`POLL_S`] at most,
/// and ends the run when the save stalls or passes its cap.  So it takes
/// at most the cap and one reading of the progress ([`SCRIPT_WAIT_S`]).
fn wait_for_offer(p: &mut Page, mark: usize, format: &str, seconds: u32) -> Result<String> {
    let start = std::time::Instant::now();
    let cap = save_cap_s(seconds) as f64;
    let mut readings = Vec::new();
    loop {
        p.poll()?;
        if let Some(line) = p.log[mark..].iter().find(|l| l.contains("e2e: offered ")) {
            return Ok(line.clone());
        }
        let progress = in_page(
            p,
            "return await window.__e2e_progress();",
            json!([]),
            SCRIPT_WAIT_S,
        )?;
        let now = start.elapsed().as_secs_f64();
        readings.push((now, progress.as_u64().unwrap_or(0)));
        match save_wait(&readings, now, seconds) {
            Wait::Going => std::thread::sleep(std::time::Duration::from_secs_f64(
                (POLL_S as f64).min(cap - now),
            )),
            Wait::Stalled => bail!(
                "{format}, {seconds} s: the save made no progress for {STALL_S} s ({now:.0} s after the click)"
            ),
            Wait::OverCap => {
                bail!("{format}, {seconds} s: the save offered no file within its cap of {cap} s")
            }
        }
    }
}

/// The checks of the 1-minute files: the WAV is the planted mix, the FLAC
/// decodes to its samples, and the MP3 to its frames within the tolerance.
pub(super) fn file_checks(files: Files) -> Vec<(bool, String)> {
    let want = planted_wav(SECONDS[0]);
    let samples = planted_mix(SECONDS[0]);
    let frames = samples.len() / 2;
    let flac_ok = files
        .flac
        .and_then(|b| engine::decode_bytes(b, Some("flac")).ok())
        .is_some_and(|audio| decodes_to(&audio, &samples));
    let mp3_frames = files
        .mp3
        .and_then(|b| engine::decode_bytes(b, Some("mp3")).ok())
        .map(|audio| audio.frames());
    let wav_sha256 = files.wav_sha256;
    vec![
        (
            wav_sha256.as_deref() == Some(sha256_hex(&want).as_str()),
            format!(
                "wav, {} s: the file is the 16-bit mix of the planted stems (SHA-256 {:?})",
                SECONDS[0], wav_sha256
            ),
        ),
        (
            flac_ok,
            format!(
                "flac, {} s: the file decodes to the WAV's samples",
                SECONDS[0]
            ),
        ),
        (
            mp3_frames_ok(mp3_frames, frames),
            format!(
                "mp3, {} s: the file decodes to {mp3_frames:?} frames, within {MP3_FRAMES_TOLERANCE} of the WAV's {frames}",
                SECONDS[0]
            ),
        ),
    ]
}

/// The check of `saves/` at the end: only the offered files, two of each
/// format and the WAV of the save during playback, and so no WAV of a
/// FLAC or MP3 save.
pub(super) fn saves_folder_check(names: &Value) -> (bool, String) {
    let names: Vec<&str> = names
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let count = |ext: &str| names.iter().filter(|n| n.ends_with(ext)).count();
    (
        names.len() == 7 && count(".wav") == 3 && count(".flac") == 2 && count(".mp3") == 2,
        format!("saves/ holds the 7 offered files and nothing else: {names:?}"),
    )
}

/// What the save during playback gave.
#[derive(Debug, Default)]
pub(super) struct Playing {
    /// The save offered its file.
    pub offered: bool,
    /// A `playback:` line came after the offer.
    pub line_after_offer: bool,
    /// The `playback:` lines from the click of Play on: one before the
    /// save at least, and the first after the offer.
    pub lines: Vec<PlaybackLine>,
    /// The lengths of the main thread's long tasks from the click of the
    /// menu item to the offer, in milliseconds: logged, not checked.
    pub long_tasks: Option<Vec<f64>>,
    /// The save's longest slice, from the page's line, in milliseconds.
    pub slice_ms: Option<f64>,
    /// The longest end of a read in the save's reader, in milliseconds.
    pub read_end_ms: Option<f64>,
    /// The milliseconds from the click of the save's item to its offer.
    pub ms: Option<u64>,
    /// The mean time of the long frames, from just before Play to Pause,
    /// in milliseconds.
    pub frame_ms: Option<f64>,
    /// The share of the long frames whose hold went past the rest of
    /// [`LONG_FRAME_MS`] by over [`OVER_MS`].
    pub over_share: Option<f64>,
}

/// The largest share of the long frames whose hold may go past the rest of
/// [`LONG_FRAME_MS`] by over [`OVER_MS`].  The system can stop the page in
/// a hold on a loaded machine.  A spy that holds on top of the page's own
/// frame goes past in each frame (findings 5-2 and 6-1 of review rounds
/// speed 5 and 6 of M8).
pub(super) const OVER_SHARE: f64 = 0.1;
/// How far a hold may go past the rest of [`LONG_FRAME_MS`], in
/// milliseconds: a turn of the spy's loop, and room for the timer.
pub(super) const OVER_MS: f64 = 5.0;

/// The long frames of the spy, each its start and its hold in
/// milliseconds, from `__e2e_long_frames`.
pub(super) fn long_frames(frames: &Value) -> Option<Vec<(f64, f64)>> {
    let starts = frames["starts"].as_array()?;
    let holds = frames["holds"].as_array()?;
    if starts.len() != holds.len() {
        return None;
    }
    starts
        .iter()
        .zip(holds)
        .map(|(s, h)| Some((s.as_f64()?, h.as_f64()?)))
        .collect()
}

/// The mean time of the long frames, from the first start to the last.
pub(super) fn mean_frame_ms(frames: &[(f64, f64)]) -> Option<f64> {
    let (first, last) = (frames.first()?.0, frames.last()?.0);
    (frames.len() >= 2).then(|| (last - first) / (frames.len() - 1) as f64)
}

/// The share of the long frames whose hold went past the rest of
/// [`LONG_FRAME_MS`] by over [`OVER_MS`].  The rest is what the page's own
/// time of the frame before left: its time from start to start less its
/// hold.  This uses the starts and the holds, not the spy's own sums.
pub(super) fn over_share(frames: &[(f64, f64)]) -> Option<f64> {
    let pairs = frames.windows(2);
    let count = pairs.len();
    let over = pairs
        .filter(|w| {
            let ((start, hold), (next, next_hold)) = (w[0], w[1]);
            let own = next - start - hold;
            next_hold > (LONG_FRAME_MS - own).max(0.0) + OVER_MS
        })
        .count();
    (count >= 1).then(|| over as f64 / count as f64)
}

/// The checks of the save during playback (D11): the song played from
/// storage before the save and after its offer, with no underrun and no
/// failed lane, as `e2e playback` counts them.
pub(super) fn playing_checks(playing: &Playing) -> Vec<(bool, String)> {
    let lines = &playing.lines;
    let underruns: Vec<u64> = lines.iter().map(|l| l.underruns).collect();
    vec![
        (
            playing.offered,
            "wav, 600 s, during playback: the save offers its file".into(),
        ),
        (
            playing.line_after_offer && lines.len() >= 2,
            format!(
                "wav, 600 s, during playback: a playback line before the save and after its offer ({} lines)",
                lines.len()
            ),
        ),
        (
            !lines.is_empty()
                && lines
                    .iter()
                    .all(|l| l.lanes > 0 && l.stored == l.lanes && l.failed_lanes == 0),
            "wav, 600 s, during playback: every lane plays from storage, and none fails".into(),
        ),
        (
            !lines.is_empty() && underruns.iter().all(|&u| u == 0),
            format!("wav, 600 s, during playback: no underrun ({underruns:?})"),
        ),
        slice_check("wav, 600 s, during playback", playing.slice_ms),
        read_end_check("wav, 600 s, during playback", playing.read_end_ms),
        (
            playing
                .frame_ms
                .is_some_and(|ms| ms >= 0.95 * LONG_FRAME_MS)
                && playing.over_share.is_some_and(|share| share <= OVER_SHARE),
            format!(
                "wav, 600 s, during playback: the animation frames take {} ms or more on average ({:?} ms), and at most {OVER_SHARE} of them hold past the rest of {LONG_FRAME_MS} ms ({:?})",
                0.95 * LONG_FRAME_MS,
                playing.frame_ms,
                playing.over_share
            ),
        ),
        (
            playing.ms.is_some_and(|ms| ms <= PLAYING_SAVE_MS),
            format!(
                "wav, 600 s, during playback, with frames of {LONG_FRAME_MS} ms: the save takes at most {PLAYING_SAVE_MS} ms ({:?} ms)",
                playing.ms
            ),
        ),
    ]
}

/// The longest slice of a save, in milliseconds, from the page's line
/// `save: <name>: longest slice <ms> ms`.
pub(super) fn slice_line(line: &str) -> Option<f64> {
    number_after(&line[line.find("save: ")?..], ": longest slice ")
}

/// The longest end of a read of a save, in milliseconds, from the page's
/// line `save: <name>: longest slice <ms> ms, longest read end <ms> ms`.
pub(super) fn read_end_line(line: &str) -> Option<f64> {
    number_after(&line[line.find("save: ")?..], ", longest read end ")
}

/// Finding 4-8 of review round speed 4 of M8: the check of the longest end
/// of a read of a save against [`SLICE_LIMIT_MS`].  The end of a read
/// decodes a span of each lane into its slot, outside the save's slices.
pub(super) fn read_end_check(what: &str, ms: Option<f64>) -> (bool, String) {
    (
        ms.is_some_and(|ms| ms <= SLICE_LIMIT_MS),
        format!("{what}: the longest end of a read is at most {SLICE_LIMIT_MS} ms ({ms:?} ms)"),
    )
}

/// The check of a save's longest slice against [`SLICE_LIMIT_MS`].
pub(super) fn slice_check(what: &str, slice_ms: Option<f64>) -> (bool, String) {
    (
        slice_ms.is_some_and(|ms| ms <= SLICE_LIMIT_MS),
        format!(
            "{what}: the save's longest slice is at most {SLICE_LIMIT_MS} ms ({slice_ms:?} ms)"
        ),
    )
}

/// The bytes of each write of a save file but the last, as the page writes
/// them: `app::export::SAVE_WRITE_BYTES` (a test keeps the two equal).
pub(super) const SAVE_WRITE_BYTES: u64 = 2 << 20;
/// The full writes of a save of `seconds` of audio: its 16-bit WAV in
/// blocks of [`SAVE_WRITE_BYTES`].
pub(super) fn full_writes(seconds: u32) -> u64 {
    (44 + 4 * 44_100 * u64::from(seconds)) / SAVE_WRITE_BYTES
}

/// The bytes of the reads in flight of a save in `e2e save`: 2 reads of
/// 65,536 frames (`app::lane_store::SAVE_AHEAD`, a test keeps them equal)
/// of 3 lanes of 16-bit WAV files, 4 bytes a frame.  A mix save reads the
/// 2 audible lanes, the stems; the third lane, the source, is a margin.
pub(super) const READS_IN_FLIGHT_BYTES: u64 = 2 * 65_536 * 4 * 3;
/// The margin of [`heap_budget`] for the rest of the page's JS objects
/// during a save, in bytes.
pub(super) const HEAP_MARGIN_BYTES: u64 = 2_000_000;

/// D10: the most that a save's JS heap measure may be, in bytes, whatever
/// the song's length (finding 5-1 of review round speed 5 of M8).  These
/// are the write's array, the browser's copy of the write in flight, the
/// reads in flight and a margin.  The end stage comes after the last read
/// ends, so its own array of the last write never lies beside the reads in
/// flight.  Its 3 arrays of 2 MiB (6.29 MB) take 0.52 MB of the margin.
pub(super) fn heap_budget() -> f64 {
    (2 * SAVE_WRITE_BYTES + READS_IN_FLIGHT_BYTES + HEAP_MARGIN_BYTES) as f64
}

/// The blocks of 16,384 frames that a save of `seconds` of audio reads.
pub(super) fn blocks(seconds: u32) -> u64 {
    (u64::from(seconds) * 44_100).div_ceil(16_384)
}

/// Whether the animation frames of the save of `format` and `seconds` are
/// throttled: only the 10-minute WAV save has its frames checked
/// ([`not_frame_bound`]).
pub(super) fn throttled(format: &str, seconds: u32) -> bool {
    format == "wav" && seconds == SECONDS[1]
}

/// D11: a save that does not wait for the UI frame takes fewer animation
/// frames than a quarter of its blocks.  Before D11, it took about one for
/// each block.  The spy throttles the frames of this save to about 4 a
/// second (`__e2e_mark`).  So the bound of 403 frames is also a time of
/// about 100 s (finding 3-1 of review round speed 3 of M8).  A frame-bound
/// save of 10 minutes needs over 400 s, and a save of D11 passes while it
/// takes less than about 100 s.
pub(super) fn not_frame_bound(frames: Option<u64>, seconds: u32) -> bool {
    frames.is_some_and(|f| f < blocks(seconds) / 4)
}

/// `xtask e2e save`.
pub(super) fn run(o: &Opts) -> Result<()> {
    let base = super::start_server(&o.dist, Vec::new(), &["console.js", "save_spy.js"])?;
    let mut p = Page::open(
        o,
        base,
        &[
            "--enable-precise-memory-info",
            "--js-flags=--expose-gc",
            "--autoplay-policy=no-user-gesture-required",
        ],
        "index.html",
    )?;
    let ready = p.wait_for(0, START_WAIT_S, |l| l.contains("separator: "))?;
    p.check(ready.is_some(), "the app started");
    let calibration =
        in_page(&mut p, CALIBRATE, json!([CALIBRATION_BYTES]), SCRIPT_WAIT_S)?.as_f64();
    p.note(&format!(
        "save: calibration: a {CALIBRATION_BYTES}-byte array raised the measure by {calibration:?} bytes"
    ));
    let (ok, what) = calibration_check(calibration);
    p.check(ok, &what);
    // Logged, not checked: a runner that paints in software can have long
    // tasks of its own (review round speed 2 of M8).
    let long = in_page(
        &mut p,
        "return await window.__e2e_long_calibrate();",
        json!([]),
        SCRIPT_WAIT_S,
    )?;
    p.note(&format!(
        "save: long tasks: a hold of 120 ms showed as a long task of {long:?} ms (null: no long tasks in this browser)"
    ));
    in_page(&mut p, CLEAN, json!([]), CLEAN_WAIT_S)?;
    for seconds in SECONDS {
        let frames = in_page(
            &mut p,
            PLANT,
            json!([session_id(seconds), seconds, STEMS]),
            PLANT_WAIT_S,
        )?;
        p.note(&format!("save: planted a session of {frames} frames"));
    }
    // The first restore of a session decodes it and makes its stem
    // overviews, which a save would overlap: each session is restored once
    // first.
    for seconds in SECONDS {
        let from = p.mark();
        restore(&mut p, seconds)?;
        let overviews = p.wait_for(from, RESTORE_WAIT_S, |l| l.contains(OVERVIEWS_END))?;
        if overviews.is_none() {
            bail!(
                "{seconds} s: the first restore made no stem overviews within {RESTORE_WAIT_S} s"
            );
        }
    }
    let mut saves = Vec::new();
    let mut files = Files::default();
    for (format, row) in FORMATS {
        for seconds in SECONDS {
            let save = save_once(&mut p, o, format, row, seconds)?;
            p.note(&format!("save: {save:?}"));
            if let Some(whole) = reads_back(format, seconds) {
                let file = in_page(&mut p, LAST_FILE, json!([whole]), READ_WAIT_S)?;
                p.note(&format!(
                    "save: {format}, {seconds} s: the file starts with {:?}, {} bytes",
                    file["head"], file["size"]
                ));
                files.keep(format, &file);
            }
            saves.push(save);
        }
    }
    let playing = save_playing(&mut p, o)?;
    p.note(&format!(
        "save: wav, 600 s, during playback: long tasks during the save {:?} ms (logged, not checked)",
        playing.long_tasks
    ));
    for (ok, what) in playing_checks(&playing) {
        p.check(ok, &what);
    }
    let names = in_page(&mut p, SAVE_NAMES, json!([]), SCRIPT_WAIT_S)?;
    for s in &saves {
        p.note(&format!(
            "save: {} {} s: JS heap measure {:?} MB; WebAssembly growth {:?} bytes; worker growth and file {:?}",
            s.format,
            s.seconds,
            s.heap.map(|h| h / MB),
            s.wasm,
            s.worker
        ));
    }
    for (ok, what) in save_checks(&saves).into_iter().chain(file_checks(files)) {
        p.check(ok, &what);
    }
    let (ok, what) = saves_folder_check(&names);
    p.check(ok, &what);
    in_page(&mut p, CLEAN, json!([]), CLEAN_WAIT_S)?;
    let errors = p.errors();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish()
}

/// The decode worker's line at the end of an `overviews` request.
const OVERVIEWS_END: &str = "at the end of the overviews request";

/// Makes the session of `seconds` current and reloads, until the page
/// shows the song with its stems, and its session is current.  The page
/// writes `sessions/current` itself, in a task of its own, after it shows a
/// song.  So the restore waits for the end of that write: else the page's
/// late write can follow the next restore's, and that restore shows the
/// session before (review round speed 5 of M8).
fn restore(p: &mut Page, seconds: u32) -> Result<()> {
    let id = session_id(seconds);
    in_page(p, MAKE_CURRENT, json!([id]), SCRIPT_WAIT_S)?;
    p.reload()?;
    let from = p.mark();
    let current = p.wait_for(from, RESTORE_WAIT_S, |l| current_session(l).is_some())?;
    let shown = current.as_deref().and_then(current_session);
    let restored = p.log[from..]
        .iter()
        .any(|l| l.contains("and its separated stems from this browser"));
    restore_check(seconds, &id, restored, shown).map_err(anyhow::Error::msg)
}

/// The session in a page's line `store: session <id> is current`.
pub(super) fn current_session(line: &str) -> Option<&str> {
    let rest = &line[line.find("store: session ")? + "store: session ".len()..];
    rest.strip_suffix(" is current")
        .filter(|id| !id.is_empty() && !id.contains(' '))
}

/// The check of a restore of the session `id` of `seconds`: the song
/// restored with its stems, and the session made current is `id`.  The
/// run of 2026-10-29 that saved the 10-minute session for the 1-minute one
/// fails it.
pub(super) fn restore_check(
    seconds: u32,
    id: &str,
    restored: bool,
    shown: Option<&str>,
) -> std::result::Result<(), String> {
    match shown {
        None => Err(format!(
            "{seconds} s: no session became current within {RESTORE_WAIT_S} s"
        )),
        Some(shown) if shown != id => Err(format!(
            "{seconds} s: the restore showed session {shown}, not {id}"
        )),
        Some(_) if !restored => Err(format!(
            "{seconds} s: session {id} is current, but the song did not restore with its stems"
        )),
        Some(_) => Ok(()),
    }
}

/// The page's WebAssembly memory, in bytes.
const WASM_MEMORY: &str = "return (await window.wasmBindings.default()).memory.buffer.byteLength;";

/// After a save: the page's WebAssembly memory in bytes, and the lengths
/// of the main thread's long tasks during the save, in milliseconds, or
/// null where the browser has no long tasks.
const AFTER_SAVE: &str =
    "return { wasm: (await window.wasmBindings.default()).memory.buffer.byteLength,
      long: window.__e2e_long_in_save() };";

/// The lengths of the long tasks during the last save, from a reply of
/// [`AFTER_SAVE`] or of `__e2e_long_in_save`, in milliseconds.  `None`
/// when the reply has none.
pub(super) fn long_tasks(reply: &Value) -> Option<Vec<f64>> {
    reply.as_array()?.iter().map(Value::as_f64).collect()
}

/// The longest slice of the first save logged in `log`, from its line.
pub(super) fn slice_of(log: &[String]) -> Option<f64> {
    log.iter().find_map(|l| slice_line(l))
}

/// The longest end of a read of the first save logged in `log`.
pub(super) fn read_end_of(log: &[String]) -> Option<f64> {
    log.iter().find_map(|l| read_end_line(l))
}

/// The growth of a memory from `before` to `after`, in bytes.
pub(super) fn growth(before: Option<f64>, after: Option<f64>) -> Option<f64> {
    before.zip(after).map(|(b, a)| a - b)
}

/// Clicks the mix lane's Save and the menu's item at `row`, and waits for
/// the offer of the save of `format` and `seconds`: its line.  With
/// `heap`, the JS heap readings start just before the item's click, and
/// come at each write.  With `throttle`, the
/// animation frames come about 4 a second until the offer.
fn click_save(
    p: &mut Page,
    o: &Opts,
    (format, row, seconds): (&str, usize, u32),
    (heap, throttle): (bool, bool),
) -> Result<String> {
    let mark = p.mark();
    p.click_at(o.save_x, o.save_y, "the mix lane's Save")?;
    p.idle(0.5)?;
    p.screenshot(&format!("{format}-{seconds}-menu"))?;
    let start = if heap {
        "window.__e2e_start_heap(); window.__e2e_mark(arguments[0]);"
    } else {
        "window.__e2e_mark(arguments[0]);"
    };
    p.session.execute(start, json!([throttle]))?;
    p.click_at(o.item_x, item_y(o, row), &format!("the {format} item"))?;
    let offered = wait_for_offer(p, mark, format, seconds)?;
    p.note(&format!(
        "save: {format}, {seconds} s: offered after {} animation frames and {} ms",
        offered_frames(&offered).map_or("?".into(), |f| f.to_string()),
        offered_ms(&offered).map_or("?".into(), |f| f.to_string())
    ));
    Ok(offered)
}

/// Restores the session of `seconds`, saves its mix as `format` through
/// the Save menu (its item at `row`), and measures the save.
fn save_once(p: &mut Page, o: &Opts, format: &str, row: usize, seconds: u32) -> Result<Save> {
    restore(p, seconds)?;
    p.idle(3.0)?;
    let before = in_page(
        p,
        "return await window.__e2e_heap();",
        json!([]),
        SCRIPT_WAIT_S,
    )?;
    let wasm_before = in_page(p, WASM_MEMORY, json!([]), SCRIPT_WAIT_S)?.as_f64();
    let mark = p.mark();
    let throttle = throttled(format, seconds);
    let offered = click_save(p, o, (format, row, seconds), (true, throttle))?;
    let after = in_page(p, AFTER_SAVE, json!([]), SCRIPT_WAIT_S)?;
    let wasm_after = after["wasm"].as_f64();
    p.note(&format!(
        "save: {format}, {seconds} s: long tasks during the save {:?} ms",
        long_tasks(&after["long"])
    ));
    let end = in_page(
        p,
        "const max = await window.__e2e_stop_heap(); return { max, readings: window.__e2e_heap_readings };",
        json!([]),
        SCRIPT_WAIT_S,
    )?;
    let heap = heap_rise(&end["max"], &before);
    let offered_line = offered;
    let offered = Some(offer_of(Some(&offered_line), format, seconds)?);
    let worker = p.log[mark..]
        .iter()
        .find(|l| l.contains(&format!("memory: encode {format} ")))
        .and_then(|l| encode_line(l));
    // V8 settles before the next page.
    p.idle(1.0)?;
    Ok(Save {
        format: format.into(),
        seconds,
        heap,
        offered,
        worker,
        wasm: growth(wasm_before, wasm_after),
        frames: offered_frames(&offered_line),
        slice_ms: slice_of(&p.log[mark..]),
        read_end_ms: read_end_of(&p.log[mark..]),
        readings: end["readings"].as_u64(),
    })
}

/// D11: plays the 10-minute session, and saves its mix as a WAV during
/// playback.  It gives whether the save offered its file, and the
/// `playback:` lines from the click of Play to the first line after the
/// offer.  Then it pauses.
fn save_playing(p: &mut Page, o: &Opts) -> Result<Playing> {
    let seconds = SECONDS[1];
    restore(p, seconds)?;
    p.idle(3.0)?;
    p.session.execute(
        "window.__e2e_long_frame_ms = arguments[0];",
        json!([LONG_FRAME_MS]),
    )?;
    let played = p.mark();
    p.click(o.play_x, "Play")?;
    if p.wait_for(played, PLAY_WAIT_S, |l| playback_line(l).is_some())?
        .is_none()
    {
        bail!("wav, {seconds} s, during playback: no playback line within {PLAY_WAIT_S} s");
    }
    let mark = p.mark();
    let offered = click_save(p, o, ("wav", 0, seconds), (false, false))?;
    let after = p.mark();
    let last = p.wait_for(after, PLAY_WAIT_S, |l| playback_line(l).is_some())?;
    p.click(o.play_x, "Pause")?;
    let frames = in_page(
        p,
        "window.__e2e_long_frame_ms = 0; return window.__e2e_long_frames;",
        json!([]),
        SCRIPT_WAIT_S,
    )?;
    let long = long_frames(&frames).unwrap_or_default();
    let frame_ms = mean_frame_ms(&long);
    let over_share = over_share(&long);
    p.note(&format!(
        "save: wav, 600 s, during playback: {} animation frames of {frame_ms:?} ms on average, {over_share:?} of them past the rest",
        long.len()
    ));
    // The long tasks of the save have reached their observer by now.
    let long = long_tasks(&in_page(
        p,
        "return window.__e2e_long_in_save();",
        json!([]),
        SCRIPT_WAIT_S,
    )?);
    let lines = p.log[played..]
        .iter()
        .filter_map(|l| playback_line(l))
        .collect();
    Ok(Playing {
        offered: offered_file(&offered).is_some(),
        line_after_offer: last.is_some(),
        lines,
        long_tasks: long,
        slice_ms: slice_of(&p.log[mark..]),
        read_end_ms: read_end_of(&p.log[mark..]),
        ms: offered_ms(&offered),
        frame_ms,
        over_share,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(format: &str, seconds: u32, heap_mb: f64, worker: Option<(f64, f64)>) -> Save {
        Save {
            format: format.into(),
            seconds,
            heap: Some(heap_mb * MB),
            offered: Some((format!("lloyd - mix.{format}"), 1.0, String::new())),
            worker,
            wasm: Some(f64::from(seconds) * 100.0),
            frames: Some(160),
            slice_ms: Some(6.0),
            read_end_ms: Some(2.0),
            readings: Some(full_writes(seconds) + 2),
        }
    }

    /// A run that passes: flat page measures, a flat FLAC encode, and MP3
    /// encodes of 3.5 times their files.
    fn good() -> Vec<Save> {
        vec![
            save("wav", 60, 1.5, None),
            save("wav", 600, 1.6, None),
            save("flac", 60, 1.2, Some((65_536.0, 5e6))),
            save("flac", 600, 1.3, Some((131_072.0, 5e7))),
            save("mp3", 60, 1.1, Some((5e6, 1.44e6))),
            save("mp3", 600, 1.2, Some((5e7, 1.44e7))),
        ]
    }

    fn failing(saves: &[Save]) -> Vec<String> {
        save_checks(saves)
            .into_iter()
            .filter(|(ok, _)| !ok)
            .map(|(_, what)| what)
            .collect()
    }

    #[test]
    fn the_save_checks_pass_a_flat_run_and_fail_growth() {
        assert!(failing(&good()).is_empty(), "{:?}", failing(&good()));
        // The base build: the WAV sink grows by about 95 MB from 1 to 10
        // minutes, and JsChannels by about 190 MB.
        let mut base = good();
        base[1].heap = Some(97.0 * MB);
        base[3].heap = Some(191.0 * MB);
        let failed = failing(&base);
        assert_eq!(failed.len(), 4, "{failed:?}");
        assert!(failed[0].starts_with("wav:") && failed[1].starts_with("flac:"));
        assert!(failed[2].starts_with("wav, 600 s: the JS heap measure is at most"));
        assert!(failed[3].starts_with("flac, 600 s: the JS heap measure is at most"));
        let mut flac = good();
        flac[3].worker = Some((65_536.0 + 1_048_577.0, 5e7));
        assert_eq!(failing(&flac).len(), 1);
        let mut mp3 = good();
        mp3[5].worker = Some((4.0 * 1.44e7 + 1_048_577.0, 1.44e7));
        assert_eq!(failing(&mp3).len(), 1);
        let mut missing = good();
        missing[4].worker = None;
        missing[0].offered = None;
        assert_eq!(failing(&missing).len(), 2);
        let mut wrong = good();
        wrong[2].offered = Some(("lloyd - mix.wav".into(), 1.0, String::new()));
        assert_eq!(failing(&wrong).len(), 1);
    }

    /// D11: a 10-minute save may grow the page's WebAssembly memory by at
    /// most the bound more than a 1-minute save.
    #[test]
    fn the_wasm_check_bounds_the_growth_of_the_long_save() {
        let mut at_bound = good();
        at_bound[1].wasm = Some(6_000.0 + WASM_BOUND_BYTES);
        assert!(failing(&at_bound).is_empty(), "{:?}", failing(&at_bound));
        let mut over = good();
        over[3].wasm = Some(6_001.0 + WASM_BOUND_BYTES);
        let failed = failing(&over);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("flac: the 10-minute save grows"));
        let mut missing = good();
        missing[4].wasm = None;
        assert_eq!(failing(&missing).len(), 1);
        // A read-ahead that grows with the song: 2 blocks of 3 lanes for
        // each of the 1,615 blocks of 10 minutes.
        let mut growing = good();
        growing[1].wasm = Some(1_615.0 * 2.0 * 16_384.0 * 8.0 * 3.0);
        assert_eq!(failing(&growing).len(), 1);
    }

    fn playing(underruns: &[u64], line_after_offer: bool) -> Playing {
        let line = |u| {
            playback_line(&format!("playback: lanes from storage 3 of 3, read-ahead 6.0 MiB, underruns {u} (callbacks), missed lane frames 0, held 0 frames, failed lanes 0, lanes read 3, reads 9, read p50 1.0 ms, p99 2.0 ms")).expect("a line")
        };
        Playing {
            offered: true,
            line_after_offer,
            lines: underruns.iter().map(|&u| line(u)).collect(),
            long_tasks: Some(Vec::new()),
            slice_ms: Some(6.0),
            read_end_ms: Some(2.0),
            ms: Some(29_000),
            frame_ms: Some(130.0),
            over_share: Some(0.0),
        }
    }

    /// D11: the save during playback passes with no underrun, a line
    /// before it and one after its offer, and fails otherwise.
    #[test]
    fn the_playing_checks_count_underruns_and_lines() {
        let failing = |p: &Playing| playing_checks(p).iter().filter(|(ok, _)| !ok).count();
        assert_eq!(failing(&playing(&[0, 0, 0], true)), 0);
        assert_eq!(failing(&playing(&[0, 1, 1], true)), 1, "an underrun");
        assert_eq!(
            failing(&playing(&[0, 0], false)),
            1,
            "no line after the offer"
        );
        assert_eq!(failing(&playing(&[0], true)), 1, "no line before the save");
        assert_eq!(failing(&playing(&[], true)), 3);
        let mut no_offer = playing(&[0, 0], true);
        no_offer.offered = false;
        assert_eq!(failing(&no_offer), 1);
        let mut memory = playing(&[0, 0], true);
        memory.lines[1].stored = 2;
        assert_eq!(failing(&memory), 1, "a lane from memory");
        let mut failed = playing(&[0, 0], true);
        failed.lines[0].failed_lanes = 1;
        assert_eq!(failing(&failed), 1, "a failed lane");
        // Review round speed 3: CI's save during playback took over 180 s.
        let mut slow = playing(&[0, 0], true);
        slow.ms = Some(PLAYING_SAVE_MS);
        assert_eq!(failing(&slow), 0, "a save at the limit");
        slow.ms = Some(PLAYING_SAVE_MS + 1);
        assert_eq!(failing(&slow), 1, "a slow save");
        slow.ms = None;
        assert_eq!(failing(&slow), 1, "no time");
        // Review rounds speed 4 and 5: frames of `LONG_FRAME_MS` or more,
        // with no hold on top of the page's own frame.
        let mut frames = playing(&[0, 0], true);
        frames.frame_ms = Some(190.0);
        frames.over_share = Some(1.0);
        assert_eq!(failing(&frames), 1, "the old spy: 125 ms on a paint of 65");
        frames.frame_ms = Some(274.0);
        frames.over_share = Some(0.0);
        assert_eq!(failing(&frames), 0, "a slow paint, and no hold");
        frames.over_share = Some(OVER_SHARE);
        assert_eq!(failing(&frames), 0, "the most frames past");
        frames.over_share = Some(OVER_SHARE + 0.01);
        assert_eq!(failing(&frames), 1, "too many frames past");
        frames.over_share = None;
        assert_eq!(failing(&frames), 1, "no share read");
        frames.over_share = Some(0.0);
        frames.frame_ms = Some(118.0);
        assert_eq!(failing(&frames), 1, "frames of 118 ms");
        frames.frame_ms = Some(119.0);
        assert_eq!(failing(&frames), 0, "frames of 119 ms");
        frames.frame_ms = None;
        assert_eq!(failing(&frames), 1, "no frames");

        // Nit 1-9 of review round speed 1: a song with no lane fails.
        let mut none = playing(&[0, 0], true);
        none.lines[0].stored = 0;
        none.lines[0].lanes = 0;
        assert_eq!(failing(&none), 1, "no lane plays");
    }

    /// Review round speed 2: each save fails when its longest slice, as
    /// the page logs it, passes the limit, or when the page logged none.
    #[test]
    fn the_longest_slice_of_each_save_is_bounded() {
        let line = "[3.2s] console.info: [app] app/src/lib.rs:4660: save: lloyd - mix.wav: longest slice 5.3 ms";
        assert_eq!(slice_line(line), Some(5.3));
        assert_eq!(slice_line("save: lloyd - mix.wav: offered"), None);
        assert_eq!(slice_line("longest slice 5.3 ms"), None);
        assert!(slice_check("wav", Some(SLICE_LIMIT_MS)).0);
        assert!(!slice_check("wav", Some(SLICE_LIMIT_MS + 0.1)).0);
        assert!(!slice_check("wav", None).0);
        let mut held = good();
        held[3].slice_ms = Some(100.0);
        let failed = failing(&held);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("flac, 600 s: the save's longest slice"));
        let mut none = good();
        none[0].slice_ms = None;
        assert_eq!(failing(&none).len(), 1);
        let failing = |p: &Playing| playing_checks(p).iter().filter(|(ok, _)| !ok).count();
        let mut playing_held = playing(&[0, 0], true);
        playing_held.slice_ms = Some(101.0);
        assert_eq!(failing(&playing_held), 1, "a slice that holds 101 ms");
        // The long tasks are logged, not checked.
        playing_held.slice_ms = Some(6.0);
        playing_held.long_tasks = Some(vec![212.5]);
        assert_eq!(failing(&playing_held), 0);
        // Finding 4-8 of review round speed 4: the longest read end.
        let both = "[2.0s] console.info: [app] app/src/lib.rs:4683: save: lloyd - mix.wav: longest slice 4.1 ms, longest read end 2.5 ms";
        assert_eq!(slice_line(both), Some(4.1));
        assert_eq!(read_end_line(both), Some(2.5));
        assert_eq!(read_end_of(&[both.to_string()]), Some(2.5));
        assert_eq!(read_end_line("save: x: longest slice 4.1 ms"), None);
        assert!(read_end_check("wav", Some(SLICE_LIMIT_MS)).0);
        assert!(!read_end_check("wav", Some(SLICE_LIMIT_MS + 0.1)).0);
        assert!(!read_end_check("wav", None).0);
        let mut slow_end = good();
        slow_end[1].read_end_ms = Some(48.4);
        let failed = self::failing(&slow_end);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("wav, 600 s: the longest end of a read"));
        let mut playing_end = playing(&[0, 0], true);
        playing_end.read_end_ms = Some(36.0);
        assert_eq!(
            playing_checks(&playing_end)
                .iter()
                .filter(|(ok, _)| !ok)
                .count(),
            1
        );
        // Finding 5-1: a reading at each write, and a budget for each save.
        let mut fewer = good();
        fewer[4].readings = Some(full_writes(60) + 1);
        let failed = self::failing(&fewer);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("mp3, 60 s: the JS heap is read at each write"));
        let mut big = good();
        big[2].heap = Some(heap_budget() - MB);
        big[3].heap = Some(heap_budget() + 1.0);
        let failed = self::failing(&big);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("flac, 600 s: the JS heap measure is at most"));
        big[3].heap = Some(heap_budget());
        assert!(self::failing(&big).is_empty(), "at the budget");
        // Nit 3-9 of review round speed 3: the first save's line.
        let log: Vec<String> = [
            "[1.0s] click the wav item at (100, 168)",
            "[2.0s] console.info: [app] app/src/lib.rs:4683: save: lloyd - mix.wav: longest slice 4.1 ms",
            "[3.0s] console.info: [app] app/src/lib.rs:4683: save: lloyd - mix.flac: longest slice 9.0 ms",
        ]
        .map(String::from)
        .into();
        assert_eq!(slice_of(&log), Some(4.1));
        assert_eq!(slice_of(&log[2..]), Some(9.0));
        assert_eq!(slice_of(&log[..1]), None);
        let one = |line: &str| slice_of(&[line.to_string()]);
        let held = "save: lloyd - mix.wav: longest slice 100.0 ms";
        assert!(!slice_check("wav", one(held)).0, "a slice of 100 ms fails");
        let short = "save: lloyd - mix.wav: longest slice 5.3 ms";
        assert!(slice_check("wav", one(short)).0, "a slice of 5.3 ms passes");
        assert!(!slice_check("wav", slice_of(&log[..1])).0, "no line fails");
        assert_eq!(long_tasks(&json!([51.5, 70])), Some(vec![51.5, 70.0]));
        assert_eq!(long_tasks(&json!(null)), None);
        assert_eq!(long_tasks(&json!(["x"])), None);
    }

    /// Findings 3-16 to 3-18 of review round speed 3: the choice of the
    /// throttled save.
    #[test]
    fn only_the_10_minute_wav_save_is_throttled() {
        assert!(throttled("wav", 600));
        assert!(!throttled("wav", 60));
        assert!(!throttled("flac", 600));
        assert!(!throttled("mp3", 60));
    }

    /// Finding 1-4 of review round speed 1: the 10-minute WAV save fails
    /// when it takes about one animation frame for each block, as before
    /// D11.
    #[test]
    fn a_frame_bound_save_fails() {
        assert_eq!(blocks(600), 1_615);
        assert_eq!(blocks(60), 162);
        assert!(not_frame_bound(Some(160), 600));
        assert!(not_frame_bound(Some(402), 600));
        assert!(!not_frame_bound(Some(403), 600));
        assert!(!not_frame_bound(Some(1_763), 600), "the build before D11");
        assert!(!not_frame_bound(None, 600));
        let mut frame_bound = good();
        frame_bound[1].frames = Some(1_763);
        let failed = failing(&frame_bound);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("wav, 600 s: the save takes fewer"));
    }

    #[test]
    fn a_growth_is_after_less_before() {
        assert_eq!(growth(Some(1.0), Some(5.0)), Some(4.0));
        assert_eq!(growth(Some(5.0), Some(1.0)), Some(-4.0));
        assert_eq!(growth(None, Some(1.0)), None);
        assert_eq!(growth(Some(1.0), None), None);
    }

    /// The mutants of review round speed 1: `WAITS_S` is the sum of every
    /// wait of a run, counted here once more.
    #[test]
    fn the_waits_are_the_sum_of_each_wait() {
        let first_restores = 2 * (SCRIPT_WAIT_S + 2 * RESTORE_WAIT_S);
        let saves = 6 * (SCRIPT_WAIT_S + RESTORE_WAIT_S + 5 * SCRIPT_WAIT_S + SAVE_IDLE_S);
        let playing = 4 * SCRIPT_WAIT_S + RESTORE_WAIT_S + SAVE_IDLE_S + 2 * PLAY_WAIT_S + 180;
        // The start, the heap calibration and the long-task calibration.
        let counted = START_WAIT_S
            + 2 * SCRIPT_WAIT_S
            + 2 * CLEAN_WAIT_S
            + 2 * PLANT_WAIT_S
            + first_restores
            + saves
            + 3 * (60 + 180)
            + 3 * READ_WAIT_S
            + playing
            + SCRIPT_WAIT_S;
        assert_eq!(WAITS_S, counted);
        assert_eq!(WAITS_S, 2_330);
        const { assert!(STEP_LIMIT_S - WAITS_S < 300, "the limit is near the waits") };
    }

    #[test]
    fn an_encode_line_gives_the_growth_and_the_file() {
        let line = "[12.0s] console.info: [INFO stemyla] separator: memory: encode mp3 saves/1792640000000-3.mp3: wasm 1179648 to 6291456 bytes, file 1438548 bytes";
        assert_eq!(encode_line(line), Some((5_111_808.0, 1_438_548.0)));
        assert_eq!(encode_line("memory: worker wasm 57 MB"), None);
    }

    #[test]
    fn the_planted_mix_is_the_sum_of_the_stems() {
        let mix = planted_mix(1);
        assert_eq!(mix.len(), 2 * 44_100);
        assert_eq!(&mix[..2], &[0, 0]);
        // Frame 1000: round(3000 sin(2)) + round(3000 sin(4)).
        let v1 = (3000.0 * 2.0f64.sin()).round();
        let v2 = (3000.0 * 4.0f64.sin()).round();
        assert_eq!(mix[2000], (v1 + v2) as i16);
        let wav = planted_wav(1);
        assert_eq!(wav.len(), 44 + 4 * 44_100);
        assert_eq!(
            &wav[44 + 4000..44 + 4002],
            &((v1 + v2) as i16).to_le_bytes()
        );
    }

    #[test]
    fn the_saves_folder_holds_only_the_offered_files() {
        let names = |n: &[&str]| json!(n);
        let seven = [
            "1-1.wav", "2-1.wav", "3-1.flac", "4-1.flac", "5-1.mp3", "6-1.mp3", "7-1.wav",
        ];
        assert!(saves_folder_check(&names(&seven)).0);
        assert!(
            !saves_folder_check(&names(&seven[..6])).0,
            "no WAV of the save during playback"
        );
        let mut left = seven.to_vec();
        left.push("5-1.wav");
        assert!(!saves_folder_check(&names(&left)).0, "a WAV of an MP3 save");
        assert!(!saves_folder_check(&Value::Null).0);
    }

    /// Finding 1-17 of review round 1 of M8: an extra file in `saves/`, such
    /// as a scratch file, fails the check too.
    #[test]
    fn an_extra_file_in_saves_fails_the_check() {
        let names = json!([
            "1-1.wav", "2-1.wav", "3-1.flac", "4-1.flac", "5-1.mp3", "6-1.mp3", "7-1.wav", "x.tmp"
        ]);
        assert!(!saves_folder_check(&names).0);
    }

    /// An MP3 line with a file of 0 bytes fails, and a 1-minute FLAC that
    /// grows more than the 10-minute one passes (finding 1-17).
    #[test]
    fn the_worker_checks_read_the_file_and_the_direction_of_growth() {
        let mut empty = good();
        empty[4].worker = Some((0.0, 0.0));
        let failed = failing(&empty);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].starts_with("mp3, 60 s"), "{failed:?}");
        let mut shrinks = good();
        shrinks[2].worker = Some((3.0 * ONE_MIB, 5e6));
        shrinks[3].worker = Some((65_536.0, 5e7));
        assert!(failing(&shrinks).is_empty(), "{:?}", failing(&shrinks));
        // The MP3 slack is one MiB: at 4 times the file plus one MiB, it
        // passes; one byte more fails.
        let mut edge = good();
        edge[5].worker = Some((4.0 * 1.44e7 + ONE_MIB, 1.44e7));
        assert!(failing(&edge).is_empty());
        edge[5].worker = Some((4.0 * 1.44e7 + ONE_MIB + 1.0, 1.44e7));
        assert_eq!(failing(&edge).len(), 1);
    }

    /// The frames of a spy, each its start and its hold, when the page's
    /// own frame takes `paint` ms and the spy holds `hold(own)` ms, where
    /// `own` is the page's time of the frame before.
    fn spy_frames(paint: f64, hold: impl Fn(f64) -> f64) -> Vec<(f64, f64)> {
        let mut frames = Vec::new();
        let (mut start, mut own) = (0.0, 0.0);
        for _ in 0..100 {
            let held = hold(own);
            frames.push((start, held));
            start += held + paint;
            own = paint;
        }
        frames
    }

    /// Finding 6-1 of review round speed 6 of M8: the frames check uses the
    /// starts and the holds that the spy logs.  The spy of today holds the
    /// rest of `LONG_FRAME_MS`; a spy that takes its own time as 0, as the
    /// spy before round speed 4 did, holds the whole time on top of the
    /// page's paint, and fails.  A slow paint and no hold passes, and one
    /// hold that the system stopped is within the share.
    #[test]
    fn the_long_frames_check_reads_the_spys_holds() {
        let rest = |own: f64| (LONG_FRAME_MS - own).max(0.0);
        // The first frame holds all of `LONG_FRAME_MS`: no frame before.
        let near = |ms: Option<f64>, want: f64| ms.is_some_and(|ms| (ms - want).abs() < 2.0);
        let today = spy_frames(65.0, rest);
        assert!(
            near(mean_frame_ms(&today), LONG_FRAME_MS),
            "{:?}",
            mean_frame_ms(&today)
        );
        assert_eq!(over_share(&today), Some(0.0));
        let own_zero = spy_frames(65.0, |_| LONG_FRAME_MS);
        assert!(near(mean_frame_ms(&own_zero), 190.0));
        assert!(
            over_share(&own_zero).is_some_and(|s| s > 0.95),
            "{:?}",
            over_share(&own_zero)
        );
        let slow = spy_frames(274.0, rest);
        assert!(near(mean_frame_ms(&slow), 274.0));
        assert_eq!(over_share(&slow), Some(0.0));
        let mut stopped = today.clone();
        stopped[50].1 += 100.0;
        for later in &mut stopped[51..] {
            later.0 += 100.0;
        }
        assert_eq!(over_share(&stopped), Some(1.0 / 99.0), "one hold stopped");
        let edge = spy_frames(65.0, |own| rest(own) + OVER_MS);
        assert_eq!(over_share(&edge), Some(0.0), "at the edge");
        let past = spy_frames(65.0, |own| rest(own) + OVER_MS + 0.5);
        assert!(over_share(&past).is_some_and(|s| s > 0.95));
        assert_eq!(mean_frame_ms(&[(0.0, 0.0)]), None);
        assert_eq!(over_share(&[(0.0, 0.0)]), None);
        // Nit 7-7 and mutant 7-9 of review round speed 7: the mean, exactly,
        // of starts as a page gives them, far from 0.
        let three = [(0.0, 60.0), (125.0, 60.0), (250.0, 60.0)];
        assert_eq!(mean_frame_ms(&three), Some(125.0));
        let page: Vec<(f64, f64)> = (0..10)
            .map(|k| (30_000.0 + 120.0 * f64::from(k), 0.0))
            .collect();
        assert_eq!(mean_frame_ms(&page), Some(120.0));
        let logged = json!({"starts": [0.0, 125.0], "holds": [60.0, 60.0]});
        assert_eq!(long_frames(&logged), Some(vec![(0.0, 60.0), (125.0, 60.0)]));
        assert_eq!(long_frames(&json!({"starts": [0.0], "holds": []})), None);
    }

    /// Finding 5-1 of review round speed 5 of M8: the budget of a save's JS
    /// heap, and the writes of a save, from the page's write size.
    #[test]
    fn the_heap_budget_holds_the_saves_arrays_and_a_margin() {
        assert_eq!(full_writes(60), 5);
        assert_eq!(full_writes(600), 50);
        assert_eq!(READS_IN_FLIGHT_BYTES, 1_572_864);
        assert_eq!(heap_budget(), 7_767_168.0);
        // Finding 6-8 of review round speed 6: the reads of the page.
        let reader = include_str!("../../../app/src/lane_store.rs");
        assert!(
            reader.contains(
                "pub const SAVE_AHEAD: Ahead = Ahead {\n    span: 65_536,\n    depth: SAVE_DEPTH,\n"
            ),
            "the span and the depth of SAVE_AHEAD"
        );
        assert!(
            reader.contains("const SAVE_DEPTH: usize = 2;"),
            "the depth of SAVE_AHEAD"
        );
        let export = include_str!("../../../app/src/export.rs");
        assert!(
            export.contains("pub const SAVE_WRITE_BYTES: usize = 2 << 20;"),
            "the page writes blocks of {SAVE_WRITE_BYTES} bytes"
        );
        assert_eq!(SAVE_WRITE_BYTES, 2 << 20);
    }

    /// Review round speed 5 of M8: a restore must show its own session.
    /// In a run under load, the page of the 10-minute session wrote
    /// `sessions/current` after the next restore had, so the restore of the
    /// 1-minute session showed the 10-minute one, and its save saved that.
    #[test]
    fn a_restore_must_show_its_own_session() {
        let line = "[21.5s] console.info: [app] app/src/lib.rs:4129: store: session 1792640100600 is current";
        assert_eq!(current_session(line), Some("1792640100600"));
        assert_eq!(
            current_session("store: session 1792640100060 is current"),
            Some("1792640100060")
        );
        assert_eq!(
            current_session("song kept in browser storage as session 1"),
            None
        );
        assert_eq!(current_session("store: session  is current"), None);
        let (short, long) = (session_id(60), session_id(600));
        assert_eq!(restore_check(60, &short, true, Some(&short)), Ok(()));
        let wrong = restore_check(60, &short, true, Some(&long)).unwrap_err();
        assert!(
            wrong.contains(&format!("showed session {long}, not {short}")),
            "{wrong}"
        );
        assert!(restore_check(60, &short, true, None).is_err(), "no session");
        assert!(
            restore_check(60, &short, false, Some(&short)).is_err(),
            "no stems"
        );
        // Finding 6-11 of review round speed 6: the page logs the line.
        let app = include_str!("../../../app/src/lib.rs");
        assert!(app.contains(r#"log::info!("store: session {id} is current");"#));
    }

    #[test]
    fn the_sessions_have_their_own_ids() {
        assert_eq!(session_id(60), "1792640100060");
        assert_eq!(session_id(600), "1792640100600");
    }

    #[test]
    fn the_calibration_needs_60_mib() {
        assert!(calibration_check(Some(67_094_229.0)).0);
        assert!(calibration_check(Some(CALIBRATION_LEAST)).0);
        assert!(!calibration_check(Some(CALIBRATION_LEAST - 1.0)).0);
        assert!(!calibration_check(None).0);
    }

    #[test]
    fn the_items_and_the_heap_rise_are_read_from_their_parts() {
        let o = Opts::parse(&["save".into()]).expect("options");
        assert_eq!(item_y(&o, 0), 168.0);
        assert_eq!(item_y(&o, 2), 210.0);
        assert_eq!(
            heap_rise(&json!(5_000_000.0), &json!(3_500_000.0)),
            Some(1_500_000.0)
        );
        assert_eq!(heap_rise(&Value::Null, &json!(3.0)), None);
    }

    /// Finding 1-4 of review round 1 of M8: a save that offers nothing ends
    /// the run, and the waits fit CI's job.
    #[test]
    fn a_save_with_no_offer_ends_the_run() {
        let line = r#"[9.0s] console.log: e2e: offered {"name":"lloyd - mix.wav","size":44,"mime":"audio/wav"}"#;
        assert_eq!(
            offer_of(Some(line), "wav", 60).expect("an offer"),
            ("lloyd - mix.wav".into(), 44.0, "audio/wav".into())
        );
        let none = offer_of(None, "flac", 600).expect_err("no offer");
        assert!(none.to_string().contains("flac, 600 s"), "{none}");
        assert!(offer_of(Some("e2e: offered {}"), "mp3", 60).is_err());
    }

    #[test]
    fn the_run_reads_back_the_1_minute_files() {
        assert_eq!(reads_back("wav", 60), Some(false));
        assert_eq!(reads_back("flac", 60), Some(true));
        assert_eq!(reads_back("mp3", 60), Some(true));
        assert_eq!(reads_back("wav", 600), None);
        let mut files = Files::default();
        let file = json!({"sha256": "abc", "base64": "AQI="});
        files.keep("wav", &file);
        files.keep("flac", &file);
        assert_eq!(files.wav_sha256.as_deref(), Some("abc"));
        assert_eq!(files.flac.as_deref(), Some(&[1u8, 2][..]));
        assert_eq!(files.mp3, None, "the FLAC is kept as the FLAC");
        files.keep("mp3", &json!({"base64": "Aw=="}));
        assert_eq!(files.mp3.as_deref(), Some(&[3u8][..]));
    }

    fn audio(samples: &[i16]) -> engine::decode::Audio {
        let side = |c: usize| {
            samples
                .iter()
                .skip(c)
                .step_by(2)
                .map(|s| f32::from(*s) / 32_768.0)
                .collect()
        };
        engine::decode::Audio {
            sample_rate: 44_100,
            channels: vec![side(0), side(1)],
        }
    }

    /// Finding 1-17: the FLAC check compares every sample of both channels.
    #[test]
    fn a_decode_matches_only_the_same_samples() {
        let samples: Vec<i16> = (0..64).map(|i| (i * 37 % 200 - 100) as i16).collect();
        assert!(decodes_to(&audio(&samples), &samples));
        assert!(
            !decodes_to(&audio(&samples[..62]), &samples),
            "a frame short"
        );
        for k in [0, 1, 2, 33] {
            let mut wrong = samples.clone();
            wrong[k] += 1;
            assert!(!decodes_to(&audio(&wrong), &samples), "sample {k}");
        }
    }

    #[test]
    fn an_mp3_length_is_within_one_frame_and_the_delay() {
        assert!(mp3_frames_ok(Some(2_646_144), 2_646_000));
        assert!(mp3_frames_ok(
            Some(2_646_000 + MP3_FRAMES_TOLERANCE),
            2_646_000
        ));
        assert!(!mp3_frames_ok(
            Some(2_646_001 + MP3_FRAMES_TOLERANCE),
            2_646_000
        ));
        assert!(!mp3_frames_ok(
            Some(2_646_000 - MP3_FRAMES_TOLERANCE - 1),
            2_646_000
        ));
        assert!(!mp3_frames_ok(None, 2_646_000));
    }

    /// The file checks with the planted files: the WAV of the planted mix
    /// and the FLAC of its samples pass; a wrong SHA-256, a FLAC a frame
    /// short and a missing MP3 fail (finding 1-17).
    #[test]
    fn the_file_checks_pass_the_planted_files_and_fail_wrong_ones() {
        let samples = planted_mix(SECONDS[0]);
        let side = |c: usize| -> Vec<f32> {
            samples
                .iter()
                .skip(c)
                .step_by(2)
                .map(|s| f32::from(*s) / 32_768.0)
                .collect()
        };
        let (left, right) = (side(0), side(1));
        let flac = super::super::flac_16(&left, &right, 44_100).expect("a FLAC");
        // Findings 2-14 and 2-16 of review round 2 of M8: an MP3 of the
        // planted mix, and one of 50 s of it.
        let mp3 = |frames: usize| {
            let mut pcm = Vec::with_capacity(2 * frames);
            for (l, r) in left[..frames].iter().zip(&right[..frames]) {
                pcm.push(*l);
                pcm.push(*r);
            }
            let mut enc = rusty_mp3::Mp3Encoder::new(rusty_mp3::Mp3EncoderConfig {
                bitrate_kbps: 192,
                vbr_quality: None,
            });
            enc.push_pcm_f32(&pcm, 2, 44_100).expect("encoded");
            enc.finish();
            let mut out = Vec::new();
            while let Ok(packet) = enc.next_packet() {
                out.extend_from_slice(&packet);
            }
            out
        };
        let short = super::super::flac_16(&left[1..], &right[1..], 44_100).expect("a FLAC");
        let wav = sha256_hex(&planted_wav(SECONDS[0]));
        let oks = |files: Files| -> Vec<bool> {
            file_checks(files).into_iter().map(|(ok, _)| ok).collect()
        };
        assert_eq!(
            oks(Files {
                wav_sha256: Some(wav),
                flac: Some(flac),
                mp3: Some(mp3(left.len())),
            }),
            [true, true, true]
        );
        let short_mp3 = file_checks(Files {
            mp3: Some(mp3(50 * 44_100)),
            ..Files::default()
        });
        assert!(!short_mp3[2].0, "an MP3 of 50 s");
        assert!(
            short_mp3[2].1.contains("of the WAV's 2646000"),
            "{}",
            short_mp3[2].1
        );
        assert_eq!(
            oks(Files {
                wav_sha256: Some("0".repeat(64)),
                flac: Some(short),
                mp3: None,
            }),
            [false, false, false]
        );
    }

    /// Finding 3-1 of review round 3 of M8: CI gives the `e2e save` step the
    /// limit that the waits fit in.
    #[test]
    fn the_ci_step_limit_is_the_waits_limit() {
        let ci = include_str!("../../../.github/workflows/ci.yml");
        let step = ci
            .split("e2e save --browser chrome")
            .nth(1)
            .expect("the e2e save step");
        let minutes: u64 = step
            .lines()
            .take_while(|l| !l.trim_start().starts_with("- "))
            .find_map(|l| l.trim().strip_prefix("timeout-minutes:"))
            .and_then(|m| m.trim().parse().ok())
            .expect("the step's timeout-minutes");
        assert_eq!(minutes * 60, STEP_LIMIT_S);
    }

    /// The CI run of 2026-10-28: a save that goes on is not stopped by a
    /// fixed time, a save with no progress for the stall time is, and a
    /// save past its cap is.
    #[test]
    fn the_wait_follows_the_saves_progress() {
        let stall = 120.0;
        let going: Vec<(f64, u64)> = (0..300).map(|i| (2.0 * i as f64, i as u64)).collect();
        assert_eq!(
            judge(&going[..200], 398.0, stall, 600.0),
            Wait::Going,
            "a long save that goes on"
        );
        assert_eq!(judge(&going, 600.0, stall, 600.0), Wait::OverCap);
        assert_eq!(judge(&going, 599.9, stall, 600.0), Wait::Going);
        // No change since the click.
        let flat = [(0.0, 7), (60.0, 7), (119.0, 7)];
        assert_eq!(judge(&flat, 119.0, stall, 600.0), Wait::Going);
        assert_eq!(judge(&flat, 120.0, stall, 600.0), Wait::Stalled);
        // A change at 50 s, then none: it stalls at 170 s, not before.
        let stops = [(0.0, 1), (50.0, 2), (100.0, 2), (169.0, 2)];
        assert_eq!(judge(&stops, 169.0, stall, 600.0), Wait::Going);
        assert_eq!(judge(&stops, 170.0, stall, 600.0), Wait::Stalled);
        // An earlier change does not count when a later one came.
        let later = [(0.0, 1), (10.0, 2), (20.0, 2), (150.0, 3), (200.0, 3)];
        assert_eq!(judge(&later, 200.0, stall, 600.0), Wait::Going);
        assert_eq!(judge(&[], 10.0, stall, 600.0), Wait::Going);
    }

    /// The slowest saves on the maintainer's Mac with `--swiftshader`,
    /// after D11 of M8, in seconds (see [`save_cap_s`]).
    const SLOWEST_SAVE_S: [f64; 2] = [3.1, 28.7];

    #[test]
    fn the_caps_leave_room_for_the_slowest_save() {
        assert_eq!(save_cap_s(60), 60);
        assert_eq!(save_cap_s(600), 180);
        for (seconds, slowest) in SECONDS.into_iter().zip(SLOWEST_SAVE_S) {
            assert!(
                save_cap_s(seconds) as f64 >= 6.0 * slowest,
                "{seconds} s: a cap of {} s",
                save_cap_s(seconds)
            );
        }
        assert!(
            STALL_S < save_cap_s(600),
            "a stall ends the long save first"
        );
        assert_eq!(caps_s(), 3 * (60 + 180));
    }

    #[test]
    fn an_offer_line_gives_its_frames() {
        let line = r#"[9.0s] console.log: e2e: offered {"name":"lloyd - mix.wav","size":44,"mime":"audio/wav","frames":181}"#;
        assert_eq!(offered_frames(line), Some(181));
        assert_eq!(offered_frames("e2e: offered {}"), None);
        let timed = r#"e2e: offered {"name":"lloyd - mix.wav","size":44,"mime":"audio/wav","frames":12,"ms":2051}"#;
        assert_eq!(offered_ms(timed), Some(2_051));
        assert_eq!(offered_ms(line), None);
    }

    /// A save of 10 minutes stalls after `STALL_S` with no progress, well
    /// before its cap, and a save that goes on ends at its cap.
    #[test]
    fn a_save_wait_takes_its_stall_time_and_its_cap() {
        let flat = [(0.0, 5), (30.0, 5), (60.0, 5)];
        assert_eq!(save_wait(&flat, 60.0, 600), Wait::Stalled);
        assert_eq!(save_wait(&flat, 59.0, 600), Wait::Going);
        let going: Vec<(f64, u64)> = (0..91).map(|i| (2.0 * i as f64, i as u64)).collect();
        assert_eq!(save_wait(&going, 180.0, 600), Wait::OverCap);
        assert_eq!(save_wait(&going[..90], 179.0, 600), Wait::Going);
        assert_eq!(save_wait(&going[..30], 59.0, 60), Wait::Going);
        assert_eq!(save_wait(&going[..31], 60.0, 60), Wait::OverCap);
    }
}
