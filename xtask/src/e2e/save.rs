//! `xtask e2e save` (step 4 of `docs/plans/completed/stream-from-storage-m8.md`):
//! a save of the mix holds no song-length buffer in the page's JS heap.
//!
//! The scenario plants a stored session of 1 minute and one of 10 minutes,
//! each a 16-bit WAV source and two 16-bit stem WAVs, in the layout of
//! `old_session`.  For each format, WAV, then FLAC, then M4A (ALAC), then
//! MP3, then M4A (AAC) where the page offers it, it restores the 1-minute
//! session.  It saves the mix as heard through the mix lane's Save menu,
//! then does the same for the 10-minute session.  Each restore
//! is a reload, so each save has a new page and new workers.
//!
//! AAC (M4 of `docs/plans/completed/m4a.md`): each page logs the line of its
//! AAC probe.  The line of the first page decides whether the run saves AAC
//! ([`aac_rule`]).  On macOS the run fails if the page does not offer AAC,
//! and on Linux if it does.  Each save waits for the probe line of its own
//! page.  Safari gives no `performance.memory`, so a run in Safari prints
//! the calibration and each check of the JS heap as "not measured in
//! Safari" (TD-11).
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
//! [`PAGE_BOUND_MB`] above the 1-minute save's.  A FLAC, an ALAC or an AAC
//! encode of 10 minutes grows the worker's memory by at most 1 MiB more
//! than one of 1 minute.  An MP3 encode grows it by at most [`MP3_FACTOR`] times its MP3
//! plus 1 MiB (the MP3 exception, TD-59).  The 1-minute WAV is the 16-bit
//! mix of the planted stems, computed here.  The 1-minute FLAC decodes to
//! the same samples, and the MP3 to the same frames within one MP3 frame
//! and the encoder's delay.  Each ALAC M4A, of 1 and of 10 minutes, decodes
//! to the samples of the mix of its length, bit for bit (M3 of
//! `docs/plans/completed/m4a.md`).  Each AAC M4A passes [`aac_checks`].
//!
//! Checks of D11 (a save that does not wait for the UI frame): for each
//! format, a 10-minute save grows the page's WebAssembly memory by at most
//! [`WASM_BOUND_BYTES`] (3 MiB) more than a 1-minute save.  The 10-minute
//! WAV save takes fewer animation frames than a quarter of its blocks, with
//! the animation frames throttled to about 4 a second.  Each save's longest
//! slice, and its longest end of a read, which the page logs, are at most
//! [`SLICE_LIMIT_MS`].  The last save saves the 10-minute mix as a WAV while
//! the song plays, with animation frames of [`LONG_FRAME_MS`], as CI's
//! paints in software.  It fails on an underrun, on frames of another time,
//! and when the save takes more than [`PLAYING_SAVE_MS`].  The main thread's
//! long tasks are logged, not checked.
//!
//! At the end, `saves/` holds only the offered files.  It prints every
//! measure, and runs every check also after one failed, so a run of an
//! older build shows its numbers.

use super::old_session::PLANT;
use super::playback::{PlaybackLine, playback_line};
use super::{Opts, Page, in_page, number_after, sha256_hex};
use crate::drivers::Browser;
use anyhow::{Result, bail};
use serde_json::{Value, json};

/// The lengths of the two sessions, in seconds.
const SECONDS: [u32; 2] = [60, 600];
/// The planted stems.  `old_session`'s plant makes stem `k` from a sine of
/// `0.002 x (k + 1)` radians a frame.
const STEMS: [&str; 2] = ["bass", "drums"];
/// The formats, in the order of the saves: each kind, extension and row
/// of its item in the Save menu (the order of `Format::ALL`).  A save is
/// keyed by its kind, since an ALAC and an AAC save both have the
/// extension `m4a`.  The kind of a FLAC, ALAC, MP3 or AAC save is the kind
/// of the worker's `memory: encode` line.  The page makes a WAV itself, so
/// its kind `wav` has no such line.  A run saves AAC only where the page
/// offers it ([`aac_rule`]).
const FORMATS: [(&str, &str, usize); 5] = [
    ("wav", "wav", 0),
    ("flac", "flac", 1),
    ("alac", "m4a", 2),
    ("mp3", "mp3", 3),
    ("aac", "m4a", 4),
];
/// D10: how much more the 10-minute save's JS heap measure may be than
/// the 1-minute save's, in MB.
pub(super) const PAGE_BOUND_MB: f64 = 4.0;
/// D10: how much more a FLAC, an ALAC or an AAC encode of 10 minutes may
/// grow the worker's memory than one of 1 minute, in bytes.
pub(super) const ENCODE_BOUND_BYTES: f64 = ONE_MIB;
/// D10: a worker's MP3 encode grows its memory by at most this many times
/// its MP3, plus [`ONE_MIB`].
pub(super) const MP3_FACTOR: f64 = 4.0;
const ONE_MIB: f64 = 1_048_576.0;
/// D11: how much more a 10-minute save may grow the page's WebAssembly
/// memory than a 1-minute save, in bytes.  The save's reader holds 3 spans
/// of 65,536 frames of each lane (`SAVE_AHEAD`), whatever the song's
/// length.  So the two saves need the same memory.  But in Safari the
/// growth has two levels, whatever the length (TD-97).  Step A of
/// `plans/completed/tech-debt-rungs-m2-td97.md` saw the same requests in each
/// measured save, all at the high level.  So the allocator's layout is the
/// likely cause; the bound does not depend on the cause.  The largest pair
/// seen is +1,900,544 bytes (MP3 of CI run 43008561411).  The extremes of
/// single growths seen, 5,505,024 (step 9) and 7,602,176, are 2,097,152
/// bytes apart; 3 MiB is 1 MiB above that.  A minute of 16-bit stereo at
/// 44.1 kHz is 10,584,000 bytes.  At one level, the 9 minutes more fail
/// above 3,145,728 / 9 = 349,525 bytes a minute: a save that keeps about
/// 1/30 of the song's 16-bit audio.  In the worst layout of those extremes
/// (1-minute high, 10-minute low), they fail above (3,145,728 + 2,097,152)
/// / 9 = 582,542 bytes a minute: about 1/18 of it.
pub(super) const WASM_BOUND_BYTES: f64 = 3.0 * ONE_MIB;
/// The limit of the `e2e save` step in CI's `browser-tests` job
/// (`timeout-minutes`), in seconds.  It is the longest that every wait of a
/// run can take (finding 3-1 of review round 3 of M8).  So a run that
/// waits in vain still ends on a check of its own.  A step that CI stops
/// writes no `save.log`, but each note of the run also goes to the step's
/// output.  Since review round speed 4 of M8, a run takes about 3 minutes
/// on the maintainer's Mac (174 to 176 s), in Chrome and with
/// `--swiftshader`, and 3.3 minutes on CI (run 40894815433).  With the two
/// M4A saves of M3 of `docs/plans/completed/m4a.md`, a run took 2.8 minutes
/// (168 s) on an Apple M2 Pro in Chrome on 2026-11-10.
pub(super) const STEP_LIMIT_S: u64 = 60 * 60;
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
/// The longest wait, in seconds, for the read of a file ([`reads_back`]).
/// In CI run 40788465048, the read of the WAV's SHA-256 took less than
/// 0.1 s.  The FLAC and the MP3 (about 1.4 MB each) are read whole, as
/// base64, which that run did not reach.  The 10-minute M4A (23.7 MB) is
/// read whole too.  It took about 2 s on an Apple M2 Pro on 2026-11-10 (M3
/// of `docs/plans/completed/m4a.md`).  So 30 s leaves a wide margin.
pub(super) const READ_WAIT_S: u64 = 30;
/// The longest wait, in seconds, for the removal of the sessions and the
/// save files.  In CI run 40788465048, the first clean-up and the plant of
/// the 1-minute session took 0.4 s together.  The last clean-up removes
/// about 356 MB, which that run did not reach.  The two M4A saves of M3 of
/// `docs/plans/completed/m4a.md` add 26 MB of it.  The two AAC saves of M4
/// add up to 21.3 MB more: 1.9 MB and 19.4 MB in Safari, the larger, on
/// 2026-11-11.
pub(super) const CLEAN_WAIT_S: u64 = 30;
/// The longest wait, in seconds, for a short script: a session switch, the
/// calibration, a heap reading, a reading of the progress or the list of
/// save files.
pub(super) const SCRIPT_WAIT_S: u64 = 15;
/// The longest wait, in seconds, for the page's line of its AAC probe
/// (`export: AAC encoder: ...`).  The probe has a limit of 5 s of its own
/// (`app::aac_probe::PROBE_MS`).  In Chrome on an Apple M2 Pro on
/// 2026-11-10, the line came 0.3 s after the start.  So 15 s is 3 times
/// the probe's limit.
pub(super) const PROBE_WAIT_S: u64 = 15;
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
/// The files that a run reads back at most ([`reads_back`]): the 1-minute
/// file of each format, and the 10-minute ALAC and AAC.
const READS: u64 = FORMATS.len() as u64 + 2;
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
/// file, and so do the 10-minute ALAC and AAC saves.  The save during playback waits for a session switch, the restore, two
/// `playback:` lines, its cap, one reading of its progress, the read of its
/// long tasks and the read of its frames, and idles.  The run ends with the
/// list of save files.  The probe line of the first page, and of each save's
/// page, has a wait of its own.  It does not count the WebDriver commands of
/// a click, a screenshot, a mark or the start of the long frames, which return at once.
pub(super) const WAITS_S: u64 = START_WAIT_S
    + PROBE_WAIT_S
    + SAVES * PROBE_WAIT_S
    + 2 * SCRIPT_WAIT_S
    + 2 * CLEAN_WAIT_S
    + FIRST_RESTORES * PLANT_WAIT_S
    + FIRST_RESTORES * (SCRIPT_WAIT_S + 2 * RESTORE_WAIT_S)
    + SAVES * (SCRIPT_WAIT_S + RESTORE_WAIT_S + 5 * SCRIPT_WAIT_S + SAVE_IDLE_S)
    + caps_s()
    + READS * READ_WAIT_S
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

/// One save: its kind and length, its JS heap measure, what was offered,
/// and the worker's memory line.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Save {
    /// The kind of the save, from [`FORMATS`]: `wav`, `flac`, `alac`, `mp3`
    /// or `aac`.
    pub kind: String,
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

/// The save of `kind` and `seconds` among `saves`.
fn find<'a>(saves: &'a [Save], kind: &str, seconds: u32) -> Option<&'a Save> {
    saves
        .iter()
        .find(|s| s.kind == kind && s.seconds == seconds)
}

const MB: f64 = 1e6;

/// The formats of a run: each kind, extension and row of [`FORMATS`], with
/// AAC only when the run saves it (`aac`).
pub(super) fn formats(aac: bool) -> impl Iterator<Item = (&'static str, &'static str, usize)> {
    FORMATS
        .into_iter()
        .filter(move |(kind, _, _)| aac || *kind != "aac")
}

/// The checks of D10 on the saves, each with what it checks, for the
/// formats of a run that saves AAC when `aac`.  The checks of the JS heap
/// are in [`heap_checks`], since Safari cannot measure it.
pub(super) fn save_checks(saves: &[Save], aac: bool) -> Vec<(bool, String)> {
    let mut checks = Vec::new();
    for (kind, extension, _) in formats(aac) {
        let wasm = |seconds| find(saves, kind, seconds).and_then(|s| s.wasm);
        let (short_wasm, long_wasm) = (wasm(SECONDS[0]), wasm(SECONDS[1]));
        checks.push((
            short_wasm
                .zip(long_wasm)
                .is_some_and(|(s, l)| l - s <= WASM_BOUND_BYTES),
            format!(
                "{kind}: the 10-minute save grows the page's WebAssembly memory by at most {WASM_BOUND_BYTES} bytes more than the 1-minute save ({short_wasm:?} and {long_wasm:?} bytes)"
            ),
        ));
        for seconds in SECONDS {
            let offered = find(saves, kind, seconds).and_then(|s| s.offered.clone());
            checks.push((
                offered
                    .as_ref()
                    .is_some_and(|(name, _, _)| name.ends_with(&format!(".{extension}"))),
                format!("{kind}, {seconds} s: a .{extension} file is offered ({offered:?})"),
            ));
        }
    }
    // Only the WAV: the offer of a FLAC, an M4A or an MP3 waits for the
    // worker's encode too.
    let frames = find(saves, "wav", SECONDS[1]).and_then(|s| s.frames);
    checks.push((
        not_frame_bound(frames, SECONDS[1]),
        format!(
            "wav, 600 s: the save takes fewer animation frames, throttled to about 4 a second, than a quarter of its {} blocks ({frames:?})",
            blocks(SECONDS[1])
        ),
    ));
    for save in saves {
        let what = format!("{}, {} s", save.kind, save.seconds);
        checks.push(slice_check(&what, save.slice_ms));
        checks.push(read_end_check(&what, save.read_end_ms));
    }
    for save in saves {
        let what = format!("{}, {} s", save.kind, save.seconds);
        let least = full_writes(save.seconds) + 2;
        checks.push((
            save.readings.is_some_and(|r| r >= least),
            format!(
                "{what}: the JS heap is read at each write and at the end, {least} times or more ({:?})",
                save.readings
            ),
        ));
    }
    let worker = |kind, seconds| find(saves, kind, seconds).and_then(|s| s.worker);
    let encodes = ["flac", "alac", "aac"];
    for kind in encodes.into_iter().filter(|kind| aac || *kind != "aac") {
        let (short, long) = (worker(kind, SECONDS[0]), worker(kind, SECONDS[1]));
        let growth = short.zip(long).map(|((s, _), (l, _))| l - s);
        checks.push((
            growth.is_some_and(|g| g <= ENCODE_BOUND_BYTES),
            format!(
                "{kind}: the worker's growth for 10 minutes is at most {ENCODE_BOUND_BYTES} bytes above that for 1 minute ({short:?}, {long:?})"
            ),
        ));
    }
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

/// The checks of D10 on the JS heap of the saves: the 10-minute save's
/// measure against the 1-minute save's, and each measure against the
/// budget.  Chrome measures the JS heap.  Safari gives no
/// `performance.memory`, so a run in Safari prints each of these as not
/// measured (TD-11).
pub(super) fn heap_checks(saves: &[Save], aac: bool) -> Vec<(bool, String)> {
    let mut checks = Vec::new();
    for (kind, _, _) in formats(aac) {
        let heap = |seconds| find(saves, kind, seconds).and_then(|s| s.heap);
        let (short, long) = (heap(SECONDS[0]), heap(SECONDS[1]));
        let growth = short.zip(long).map(|(s, l)| (l - s) / MB);
        checks.push((
            growth.is_some_and(|g| g <= PAGE_BOUND_MB),
            format!(
                "{kind}: the 10-minute save's JS heap measure is at most {PAGE_BOUND_MB} MB above the 1-minute save's ({:?} MB and {:?} MB: {growth:?} MB more)",
                short.map(|v| v / MB),
                long.map(|v| v / MB)
            ),
        ));
    }
    for save in saves {
        let what = format!("{}, {} s", save.kind, save.seconds);
        checks.push((
            save.heap.is_some_and(|h| h <= heap_budget()),
            format!(
                "{what}: the JS heap measure is at most {:.2} MB, the save's arrays and a margin ({:?} MB)",
                heap_budget() / MB,
                save.heap.map(|h| h / MB)
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

/// The files that the run reads back for [`file_checks`]: the 1-minute
/// file of each format, and the 10-minute ALAC and AAC.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Files {
    /// The SHA-256 of the WAV, in hex.
    pub wav_sha256: Option<String>,
    pub flac: Option<Vec<u8>>,
    pub mp3: Option<Vec<u8>>,
    /// The ALAC M4A of 1 minute.
    pub m4a_short: Option<Vec<u8>>,
    /// The ALAC M4A of 10 minutes.
    pub m4a_long: Option<Vec<u8>>,
    /// The AAC M4A of 1 minute.
    pub aac_short: Option<Vec<u8>>,
    /// The AAC M4A of 10 minutes.
    pub aac_long: Option<Vec<u8>>,
}

/// Whether the run reads back the file of a save of `kind` and `seconds`,
/// and if so, whether whole (`true`, to decode it) or only its SHA-256.
/// It reads the 1-minute file of each format, and each ALAC and AAC whole.
pub(super) fn reads_back(kind: &str, seconds: u32) -> Option<bool> {
    (seconds == SECONDS[0] || kind == "alac" || kind == "aac").then_some(kind != "wav")
}

impl Files {
    /// Keeps what [`LAST_FILE`] gave for the save of `kind` and `seconds`.
    pub(super) fn keep(&mut self, kind: &str, seconds: u32, file: &Value) {
        let bytes = file["base64"]
            .as_str()
            .and_then(|b| crate::webdriver::base64_decode(b).ok());
        let short = seconds == SECONDS[0];
        match kind {
            "wav" => self.wav_sha256 = file["sha256"].as_str().map(String::from),
            "flac" => self.flac = bytes,
            "alac" if short => self.m4a_short = bytes,
            "alac" => self.m4a_long = bytes,
            "aac" if short => self.aac_short = bytes,
            "aac" => self.aac_long = bytes,
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

/// Whether `m4a` decodes to the planted mix of `seconds`, bit for bit.
pub(super) fn m4a_decodes_to_the_mix(m4a: Option<Vec<u8>>, seconds: u32) -> bool {
    m4a.and_then(|b| engine::decode_bytes(b, Some("m4a")).ok())
        .is_some_and(|audio| decodes_to(&audio, &planted_mix(seconds)))
}

/// The checks of the files: the 1-minute WAV is the planted mix, the
/// 1-minute FLAC decodes to its samples, and the 1-minute MP3 to its frames
/// within the tolerance.  Each ALAC M4A, of 1 and of 10 minutes, decodes
/// to the samples of the mix of its length, bit for bit.  When the run
/// saves AAC (`aac`), each AAC M4A passes [`aac_checks`].
pub(super) fn file_checks(files: Files, aac: bool) -> Vec<(bool, String)> {
    let aac_files = [(files.aac_short, SECONDS[0]), (files.aac_long, SECONDS[1])];
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
    let m4a_ok = [
        m4a_decodes_to_the_mix(files.m4a_short, SECONDS[0]),
        m4a_decodes_to_the_mix(files.m4a_long, SECONDS[1]),
    ];
    let wav_sha256 = files.wav_sha256;
    let mut checks = vec![
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
        (
            m4a_ok[0],
            format!(
                "alac, {} s: the file decodes to the WAV's samples, bit for bit",
                SECONDS[0]
            ),
        ),
        (
            m4a_ok[1],
            format!(
                "alac, {} s: the file decodes to the samples of the mix, bit for bit",
                SECONDS[1]
            ),
        ),
    ];
    if aac {
        for (bytes, seconds) in aac_files {
            let audio = bytes.and_then(|b| engine::decode_bytes(b, Some("m4a")).ok());
            checks.extend(aac_checks(audio.as_ref(), seconds));
        }
    }
    checks
}

/// The check of `saves/` at the end: only the offered files, two of each
/// format and the WAV of the save during playback, and so no WAV of a
/// FLAC, M4A or MP3 save.  A run that saves AAC has 11 files, 4 of them
/// `.m4a`, and a run without AAC has 9.
pub(super) fn saves_folder_check(names: &Value, aac: bool) -> (bool, String) {
    let names: Vec<&str> = names
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let count = |ext: &str| names.iter().filter(|n| n.ends_with(ext)).count();
    let (files, m4a) = if aac { (11, 4) } else { (9, 2) };
    (
        names.len() == files
            && count(".wav") == 3
            && count(".flac") == 2
            && count(".m4a") == m4a
            && count(".mp3") == 2,
        format!("saves/ holds the {files} offered files and nothing else: {names:?}"),
    )
}

/// The most relative error of an AAC save to the planted mix, over the
/// whole file and over its first 4096 frames.  On an Apple M2 Pro on
/// 2026-11-11, Chrome 157 gave at most 0.00028 and 0.00072, and Safari 27.2
/// 0.00026 and 0.00019.  The bounds are about 7 times the larger.  A decode
/// one frame off gives about 0.0032 over the whole, and a lost first 1024
/// frames about 0.5 over the start.
pub(super) const MOST_AAC_ERROR: f64 = 0.002;
pub(super) const MOST_AAC_START_ERROR: f64 = 0.005;
/// The lags of the cross-correlation of an AAC save with the planted mix,
/// either side of 0.  The mix repeats every 3,142 frames, so no other peak
/// is in this range.
pub(super) const AAC_LAGS: isize = 1024;
/// The frames of the cross-correlation: 10 s, from frame [`AAC_LAGS`].  `xtask` and
/// `engine` build at opt-level 0, and lags over a whole 10-minute file are
/// about 5.4 x 10^10 multiply-adds for each channel.
pub(super) const AAC_LAG_FRAMES: usize = 10 * 44_100;

/// The frames of the cross-correlation of an AAC save of `frames` frames:
/// [`AAC_LAG_FRAMES`] from frame [`AAC_LAGS`], and at least [`AAC_LAGS`]
/// frames before the end, so that each lag reads whole frames.
pub(super) fn aac_lag_window(frames: usize) -> std::ops::Range<usize> {
    let margin = AAC_LAGS as usize;
    margin..(margin + AAC_LAG_FRAMES).min(frames.saturating_sub(margin))
}

/// The channels of the planted mix of `seconds`, as floats.
fn planted_channels(seconds: u32) -> [Vec<f32>; 2] {
    let samples = planted_mix(seconds);
    let channel = |c: usize| {
        samples
            .iter()
            .skip(c)
            .step_by(2)
            .map(|s| f32::from(*s) / 32_768.0)
            .collect()
    };
    [channel(0), channel(1)]
}

/// The root of the mean square of `got - want` over the frames `range` of
/// `want`, with `got` read `lag` frames later, over the root of the mean
/// square of `want`.  `None` when no frame of the range is in both.
pub(super) fn error_at(
    got: &[f32],
    want: &[f32],
    lag: isize,
    range: std::ops::Range<usize>,
) -> Option<f64> {
    let (mut diff, mut power, mut n) = (0.0_f64, 0.0_f64, 0_usize);
    for i in range {
        let g = i.checked_add_signed(lag).and_then(|j| got.get(j)).copied();
        let (Some(w), Some(g)) = (want.get(i).copied(), g) else {
            continue;
        };
        diff += (f64::from(g) - f64::from(w)).powi(2);
        power += f64::from(w).powi(2);
        n += 1;
    }
    (n > 0 && power > 0.0).then(|| (diff / power).sqrt())
}

/// The lag in `-most..=most` where the normalised cross-correlation of
/// `got` with the frames `range` of `want` is largest.  Each lag's sum is
/// divided by the root of the energy of the frames of `got` that it reads.
/// So a copy of `want` peaks at lag 0 (Cauchy-Schwarz), also for the slow
/// sines of the planted mix, whose raw cross-correlation can peak at lag 1.
/// The caller keeps `range` at least `most` frames from each end, so that
/// each lag reads whole frames.
pub(super) fn best_lag(
    got: &[f32],
    want: &[f32],
    range: std::ops::Range<usize>,
    most: isize,
) -> isize {
    let mut best = (f64::MIN, 0);
    for lag in -most..=most {
        let (mut sum, mut energy) = (0.0_f64, 0.0_f64);
        for (i, w) in want.iter().enumerate().take(range.end).skip(range.start) {
            if let Some(g) = i.checked_add_signed(lag).and_then(|j| got.get(j)) {
                sum += f64::from(*g) * f64::from(*w);
                energy += f64::from(*g).powi(2);
            }
        }
        let score = if energy > 0.0 {
            sum / energy.sqrt()
        } else {
            f64::MIN
        };
        if score > best.0 {
            best = (score, lag);
        }
    }
    best.1
}

/// The checks of an AAC save of `seconds`, decoded through M2's path
/// (`engine::decode_bytes`), against the planted mix (Step 6 of
/// `docs/plans/completed/m4a-m4.md`).  It has the mix's frames exactly.  Its
/// cross-correlation over the first 10 s peaks at lag 0.  Its error is
/// below a bound over the whole and over the first 4096 frames.  The error
/// at lag 0 is less than at lags of -2, -1, 1 and 2, since the planted
/// sines are slow and the cross-correlation is almost flat near 0.
pub(super) fn aac_checks(
    audio: Option<&engine::decode::Audio>,
    seconds: u32,
) -> Vec<(bool, String)> {
    aac_checks_of(
        audio,
        &planted_channels(seconds),
        &format!("aac, {seconds} s"),
    )
}

/// The checks of [`aac_checks`] against the channels `want`.
pub(super) fn aac_checks_of(
    audio: Option<&engine::decode::Audio>,
    want: &[Vec<f32>; 2],
    what: &str,
) -> Vec<(bool, String)> {
    let frames = want[0].len();
    let Some(audio) = audio.filter(|a| a.channels.len() == 2) else {
        return vec![(
            false,
            format!("{what}: the file decodes as stereo through M2's path"),
        )];
    };
    let got = &audio.channels;
    let window = aac_lag_window(frames);
    let lag = best_lag(&got[0], &want[0], window.clone(), AAC_LAGS);
    let worst = |lag: isize, range: std::ops::Range<usize>| {
        (0..2)
            .map(|c| error_at(&got[c], &want[c], lag, range.clone()))
            .try_fold(0.0_f64, |most, e| e.map(|e| most.max(e)))
    };
    let whole = worst(0, 0..frames);
    let start = worst(0, 0..4096.min(frames));
    let near: Vec<(isize, Option<f64>)> = [-2, -1, 1, 2].map(|l| (l, worst(l, 0..frames))).into();
    vec![
        (
            audio.frames() == frames,
            format!(
                "{what}: the file decodes to the {frames} frames of the mix ({} frames)",
                audio.frames()
            ),
        ),
        (
            lag == 0,
            format!(
                "{what}: the cross-correlation over frames {window:?} peaks at lag 0 of -{AAC_LAGS} to {AAC_LAGS} (lag {lag})",
            ),
        ),
        (
            whole.is_some_and(|e| e < MOST_AAC_ERROR),
            format!("{what}: the error to the mix is below {MOST_AAC_ERROR} ({whole:?})"),
        ),
        (
            start.is_some_and(|e| e < MOST_AAC_START_ERROR),
            format!(
                "{what}: the error over the first 4096 frames is below {MOST_AAC_START_ERROR} ({start:?})"
            ),
        ),
        (
            whole.is_some_and(|w| near.iter().all(|(_, e)| e.is_some_and(|e| w < e))),
            format!(
                "{what}: the error at lag 0 ({whole:?}) is less than at lags of -2, -1, 1 and 2 ({near:?})"
            ),
        ),
    ]
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
    // Safari gives no `performance.memory`, so it cannot measure the JS
    // heap (TD-11).  Each such check prints as not measured.
    let measured = heap_measured(o.browser);
    let probe = p.wait_for(0, PROBE_WAIT_S, |l| probe_line(l).is_some())?;
    let (aac, why_not) = aac_rule(std::env::consts::OS, probe.as_deref().and_then(probe_line));
    p.note(&format!("save: the run saves AAC: {aac} ({probe:?})"));
    if let Some(why) = why_not {
        p.check(
            false,
            &format!("the page offers AAC where the browser encodes it: {why}"),
        );
    }
    let calibration =
        in_page(&mut p, CALIBRATE, json!([CALIBRATION_BYTES]), SCRIPT_WAIT_S)?.as_f64();
    p.note(&format!(
        "save: calibration: a {CALIBRATION_BYTES}-byte array raised the measure by {calibration:?} bytes"
    ));
    let (ok, what) = calibration_check(calibration);
    check_heap(&mut p, measured, ok, &what);
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
    for (kind, _, row) in formats(aac) {
        for seconds in SECONDS {
            let save = save_once(&mut p, o, (kind, row), seconds)?;
            p.note(&format!("save: {save:?}"));
            if let Some(whole) = reads_back(kind, seconds) {
                let file = in_page(&mut p, LAST_FILE, json!([whole]), READ_WAIT_S)?;
                p.note(&format!(
                    "save: {kind}, {seconds} s: the file starts with {:?}, {} bytes",
                    file["head"], file["size"]
                ));
                files.keep(kind, seconds, &file);
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
            s.kind,
            s.seconds,
            s.heap.map(|h| h / MB),
            s.wasm,
            s.worker
        ));
    }
    for (ok, what) in save_checks(&saves, aac)
        .into_iter()
        .chain(file_checks(files, aac))
    {
        p.check(ok, &what);
    }
    for (ok, what) in heap_checks(&saves, aac) {
        check_heap(&mut p, measured, ok, &what);
    }
    let (ok, what) = saves_folder_check(&names, aac);
    p.check(ok, &what);
    in_page(&mut p, CLEAN, json!([]), CLEAN_WAIT_S)?;
    let errors = p.errors();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish()
}

/// The page's line of its AAC probe: whether the page offers AAC.  The
/// page logs `export: AAC encoder: yes`, or `export: AAC encoder: no
/// (<why>)`.
pub(super) fn probe_line(line: &str) -> Option<bool> {
    let rest = &line[line.find("export: AAC encoder: ")? + "export: AAC encoder: ".len()..];
    if rest.starts_with("yes") {
        Some(true)
    } else if rest.starts_with("no") {
        Some(false)
    } else {
        None
    }
}

/// Whether a run on `os` (`std::env::consts::OS`) saves AAC, from the probe
/// line of its first page (`offered`), and why the run fails, if it does.
/// On macOS, Chrome and Safari encode AAC through the system, so a page
/// that does not offer AAC fails the run.  On Linux, Chromium has no AAC
/// encoder (TD-92), so a page that offers AAC fails the run: TD-92 then
/// needs an update.  Elsewhere, the run follows the page.  A page with no
/// probe line saves no AAC, and fails the run.
pub(super) fn aac_rule(os: &str, offered: Option<bool>) -> (bool, Option<String>) {
    match (os, offered) {
        (_, None) => (false, Some("the page logged no AAC probe line".into())),
        ("macos", Some(false)) => (
            false,
            Some("on macOS, Chrome and Safari encode AAC, but the page does not offer it".into()),
        ),
        ("linux", Some(true)) => (
            true,
            Some("on Linux, the page offers AAC, so TD-92 needs an update".into()),
        ),
        (_, Some(offered)) => (offered, None),
    }
}

/// A check of the JS heap: a check where the browser measures it, and a
/// note "not measured in Safari" where it does not, so that it does not
/// pass in silence (TD-11).
fn check_heap(p: &mut Page, measured: bool, ok: bool, what: &str) {
    match heap_report(measured, ok, what) {
        (Some(ok), what) => p.check(ok, &what),
        (None, note) => p.note(&note),
    }
}

/// Whether `browser` measures the JS heap: Chrome does, with
/// `performance.memory`, and Safari does not (TD-11).
pub(super) fn heap_measured(browser: Browser) -> bool {
    browser != Browser::Safari
}

/// What [`check_heap`] does with a check of the JS heap: a check of `ok`
/// (`Some`), or a note that the browser does not measure it (`None`).
pub(super) fn heap_report(measured: bool, ok: bool, what: &str) -> (Option<bool>, String) {
    if measured {
        (Some(ok), what.to_string())
    } else {
        (None, format!("not measured in Safari: {what}"))
    }
}

/// Whether a save of `kind` needs AAC that its page, with the probe line
/// `offered`, does not offer.
pub(super) fn aac_not_offered(kind: &str, offered: Option<bool>) -> bool {
    kind == "aac" && offered != Some(true)
}

/// The kind of the worker's `memory: encode` line of a save of `kind`.
/// The page makes a WAV itself, so a WAV save has none.
pub(super) fn worker_kind(kind: &str) -> Option<&str> {
    (kind != "wav").then_some(kind)
}

/// The decode worker's line at the end of an `overviews` request.
const OVERVIEWS_END: &str = "at the end of the overviews request";

/// Makes the session of `seconds` current and reloads, until the page
/// shows the song with its stems, and its session is current.  It gives
/// the mark of the log at the reload, where the new page's lines start.  The page
/// writes `sessions/current` itself, in a task of its own, after it shows a
/// song.  So the restore waits for the end of that write: else the page's
/// late write can follow the next restore's, and that restore shows the
/// session before (review round speed 5 of M8).
fn restore(p: &mut Page, seconds: u32) -> Result<usize> {
    let id = session_id(seconds);
    in_page(p, MAKE_CURRENT, json!([id]), SCRIPT_WAIT_S)?;
    p.reload()?;
    let from = p.mark();
    let current = p.wait_for(from, RESTORE_WAIT_S, |l| current_session(l).is_some())?;
    let shown = current.as_deref().and_then(current_session);
    let restored = p.log[from..]
        .iter()
        .any(|l| l.contains("and its separated stems from this browser"));
    restore_check(seconds, &id, restored, shown).map_err(anyhow::Error::msg)?;
    Ok(from)
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

/// Restores the session of `seconds`, saves its mix as `kind` through the
/// Save menu (its item at `row`), and measures the save.  The worker's
/// `memory: encode` line has the same kind, but the page makes a WAV
/// itself.  Each restore is a new page with a new probe, so the save waits
/// for the probe line of its page first.  An AAC save needs a page that
/// offers AAC.
fn save_once(p: &mut Page, o: &Opts, (kind, row): (&str, usize), seconds: u32) -> Result<Save> {
    let page = restore(p, seconds)?;
    let probe = p.wait_for(page, PROBE_WAIT_S, |l| probe_line(l).is_some())?;
    let offered = probe.as_deref().and_then(probe_line);
    if aac_not_offered(kind, offered) {
        bail!("aac, {seconds} s: the page of the save does not offer AAC ({probe:?})");
    }
    let encode_kind = worker_kind(kind);
    p.idle(3.0)?;
    let before = in_page(
        p,
        "return await window.__e2e_heap();",
        json!([]),
        SCRIPT_WAIT_S,
    )?;
    let wasm_before = in_page(p, WASM_MEMORY, json!([]), SCRIPT_WAIT_S)?.as_f64();
    let mark = p.mark();
    let throttle = throttled(kind, seconds);
    let offered = click_save(p, o, (kind, row, seconds), (true, throttle))?;
    let after = in_page(p, AFTER_SAVE, json!([]), SCRIPT_WAIT_S)?;
    let wasm_after = after["wasm"].as_f64();
    p.note(&format!(
        "save: {kind}, {seconds} s: long tasks during the save {:?} ms",
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
    let offered = Some(offer_of(Some(&offered_line), kind, seconds)?);
    let worker = encode_kind.and_then(|kind| {
        p.log[mark..]
            .iter()
            .find(|l| l.contains(&format!("memory: encode {kind} ")))
            .and_then(|l| encode_line(l))
    });
    // V8 settles before the next page.
    p.idle(1.0)?;
    Ok(Save {
        kind: kind.into(),
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
            kind: format.into(),
            seconds,
            heap: Some(heap_mb * MB),
            offered: Some((
                format!("lloyd - mix.{}", extension_of(format)),
                1.0,
                String::new(),
            )),
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
            save("alac", 60, 1.2, Some((65_536.0, 7e6))),
            save("alac", 600, 1.3, Some((131_072.0, 7e7))),
        ]
    }

    /// The extension of the save of `kind`, from [`FORMATS`].
    fn extension_of(kind: &str) -> &'static str {
        FORMATS
            .iter()
            .find(|(k, _, _)| *k == kind)
            .map_or("?", |(_, extension, _)| *extension)
    }

    /// A run that passes and saves AAC: [`good`], and two flat AAC saves.
    fn good_aac() -> Vec<Save> {
        let mut saves = good();
        saves.push(save("aac", 60, 1.2, Some((65_536.0, 2e6))));
        saves.push(save("aac", 600, 1.3, Some((131_072.0, 2e7))));
        saves
    }

    /// What each failed check of a run that saves AAC when `aac` says.
    fn failing_with(saves: &[Save], aac: bool) -> Vec<String> {
        save_checks(saves, aac)
            .into_iter()
            .chain(heap_checks(saves, aac))
            .filter(|(ok, _)| !ok)
            .map(|(_, what)| what)
            .collect()
    }

    fn failing(saves: &[Save]) -> Vec<String> {
        save_checks(saves, false)
            .into_iter()
            .chain(heap_checks(saves, false))
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
        let mut m4a = good();
        m4a[7].worker = Some((65_536.0 + 1_048_577.0, 7e7));
        let failed = failing(&m4a);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(
            failed[0].starts_with("alac: the worker's growth"),
            "{failed:?}"
        );
        let mut m4a_missing = good();
        m4a_missing[6].worker = None;
        assert_eq!(failing(&m4a_missing).len(), 1);
        let mut m4a_offer = good();
        m4a_offer[7].offered = Some(("lloyd - mix.mp3".into(), 1.0, String::new()));
        assert_eq!(failing(&m4a_offer).len(), 1);
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

    /// TD-97: in Safari, the growth of a save has two levels, about 1.9 MB
    /// apart, whatever the song's length.  The pairs of 1 and 10 minutes
    /// that failed the bound of 1 MiB pass: MP3 of CI run 43008561411, and
    /// AAC of step 9 of `plans/completed/tech-debt-rungs-m2.md`.  So does
    /// the pair of the extremes of single growths: 5,505,024 (the ALAC save
    /// of 10 minutes of step 9) and 7,602,176.  The Decision log of
    /// `plans/completed/tech-debt-rungs-m2-td97.md` lists their logs.  One
    /// byte more than the bound above the 1-minute save fails.
    #[test]
    fn the_two_level_pairs_of_safari_pass_and_one_byte_more_fails() {
        let pairs = [
            (5_636_096.0, 7_536_640.0),
            (5_767_168.0, 7_274_496.0),
            (5_505_024.0, 7_602_176.0),
        ];
        for (short, long) in pairs {
            let mut saves = good();
            saves[6].wasm = Some(short);
            saves[7].wasm = Some(long);
            assert!(
                failing(&saves).is_empty(),
                "{short} and {long}: {:?}",
                failing(&saves)
            );
            saves[7].wasm = Some(short + WASM_BOUND_BYTES + 1.0);
            let failed = failing(&saves);
            assert_eq!(failed.len(), 1, "{failed:?}");
            assert!(
                failed[0].starts_with("alac: the 10-minute save grows"),
                "{failed:?}"
            );
        }
    }

    /// MEM-4, with literal values: a 10-minute save that keeps a part of
    /// the song's 16-bit audio fails.  The first pair (1/30 kept at one
    /// level) pins the bound from above: a bound wide enough to pass it
    /// fails this test.  The second pair (1/18 kept in the worst layout of
    /// the extremes) documents that case.  It catches no bound that the
    /// first pair passes.  A minute of 16-bit stereo at 44.1 kHz is
    /// 10,584,000 bytes.  At one level (5,636,096 for both lengths), keeping
    /// 1/30 of the 9 minutes more adds 3,175,200 bytes.  The worst layout of
    /// the extremes of single growths seen has the 1-minute save high
    /// (7,602,176) and the 10-minute save low (5,505,024).  There, keeping
    /// 1/18 adds 5,292,000 bytes.  So the 10-minute save grows by 3,194,848
    /// bytes more.
    #[test]
    fn a_save_that_keeps_a_part_of_the_song_fails() {
        let pairs = [(5_636_096.0, 8_811_296.0), (7_602_176.0, 10_797_024.0)];
        for (short, long) in pairs {
            let mut saves = good();
            saves[6].wasm = Some(short);
            saves[7].wasm = Some(long);
            let failed = failing(&saves);
            assert_eq!(failed.len(), 1, "{short} and {long}: {failed:?}");
            assert!(
                failed[0].starts_with("alac: the 10-minute save grows"),
                "{failed:?}"
            );
        }
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
        let saves =
            10 * (PROBE_WAIT_S + SCRIPT_WAIT_S + RESTORE_WAIT_S + 5 * SCRIPT_WAIT_S + SAVE_IDLE_S);
        let playing = 4 * SCRIPT_WAIT_S + RESTORE_WAIT_S + SAVE_IDLE_S + 2 * PLAY_WAIT_S + 180;
        // The start, the heap calibration and the long-task calibration.
        let counted = START_WAIT_S
            + PROBE_WAIT_S
            + 2 * SCRIPT_WAIT_S
            + 2 * CLEAN_WAIT_S
            + 2 * PLANT_WAIT_S
            + first_restores
            + saves
            + 5 * (60 + 180)
            + 7 * READ_WAIT_S
            + playing
            + SCRIPT_WAIT_S;
        assert_eq!(WAITS_S, counted);
        assert_eq!(WAITS_S, 3_595);
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
        let nine = [
            "1-1.wav", "2-1.wav", "3-1.flac", "4-1.flac", "5-1.m4a", "6-1.m4a", "7-1.mp3",
            "8-1.mp3", "9-1.wav",
        ];
        assert!(saves_folder_check(&names(&nine), false).0);
        assert!(
            !saves_folder_check(&names(&nine[..8]), false).0,
            "no WAV of the save during playback"
        );
        let mut no_m4a = nine.to_vec();
        no_m4a.retain(|n| *n != "6-1.m4a");
        assert!(
            !saves_folder_check(&names(&no_m4a), false).0,
            "one M4A of two"
        );
        let mut left = nine.to_vec();
        left.push("5-1.wav");
        assert!(
            !saves_folder_check(&names(&left), false).0,
            "a WAV of an M4A save"
        );
        assert!(!saves_folder_check(&Value::Null, false).0);
    }

    /// Finding 1-17 of review round 1 of M8: an extra file in `saves/`, such
    /// as a scratch file, fails the check too.
    #[test]
    fn an_extra_file_in_saves_fails_the_check() {
        let names = json!([
            "1-1.wav", "2-1.wav", "3-1.flac", "4-1.flac", "5-1.m4a", "6-1.m4a", "7-1.mp3",
            "8-1.mp3", "9-1.wav", "x.tmp"
        ]);
        assert!(!saves_folder_check(&names, false).0);
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
    fn the_run_reads_back_the_1_minute_files_and_each_m4a() {
        for (kind, short, long) in [
            ("wav", Some(false), None),
            ("flac", Some(true), None),
            ("alac", Some(true), Some(true)),
            ("mp3", Some(true), None),
            ("aac", Some(true), Some(true)),
        ] {
            assert_eq!(reads_back(kind, 60), short, "{kind}, 60 s");
            assert_eq!(reads_back(kind, 600), long, "{kind}, 600 s");
        }
        let reads = FORMATS
            .iter()
            .flat_map(|(kind, _, _)| SECONDS.map(|s| reads_back(kind, s)))
            .filter(Option::is_some)
            .count();
        assert_eq!(reads as u64, READS, "WAITS_S counts each read");
        let mut files = Files::default();
        let file = json!({"sha256": "abc", "base64": "AQI="});
        files.keep("wav", 60, &file);
        files.keep("flac", 60, &file);
        assert_eq!(files.wav_sha256.as_deref(), Some("abc"));
        assert_eq!(files.flac.as_deref(), Some(&[1u8, 2][..]));
        assert_eq!(files.mp3, None, "the FLAC is kept as the FLAC");
        files.keep("mp3", 60, &json!({"base64": "Aw=="}));
        assert_eq!(files.mp3.as_deref(), Some(&[3u8][..]));
        files.keep("alac", 60, &json!({"base64": "BA=="}));
        files.keep("alac", 600, &json!({"base64": "BQ=="}));
        assert_eq!(files.m4a_short.as_deref(), Some(&[4u8][..]));
        assert_eq!(files.m4a_long.as_deref(), Some(&[5u8][..]));
        files.keep("aac", 60, &json!({"base64": "Bg=="}));
        files.keep("aac", 600, &json!({"base64": "Bw=="}));
        assert_eq!(files.aac_short.as_deref(), Some(&[6u8][..]));
        assert_eq!(files.aac_long.as_deref(), Some(&[7u8][..]));
        assert_eq!(
            files.m4a_short.as_deref(),
            Some(&[4u8][..]),
            "an AAC is kept as an AAC"
        );
        assert_eq!(
            files.mp3.as_deref(),
            Some(&[3u8][..]),
            "an M4A is kept as an M4A"
        );
    }

    /// An M4A decodes to the planted mix of its length, bit for bit, or
    /// fails.
    #[test]
    fn an_m4a_check_decodes_to_the_planted_mix() {
        let seconds = 2;
        let mix = planted_mix(seconds);
        let m4a = |samples: &[i16]| {
            m4a::alac_file(samples, 44_100, None, m4a::Order::FastStart).expect("an M4A")
        };
        assert!(m4a_decodes_to_the_mix(Some(m4a(&mix)), seconds));
        assert!(!m4a_decodes_to_the_mix(None, seconds), "no file");
        assert!(
            !m4a_decodes_to_the_mix(Some(m4a(&mix[2..])), seconds),
            "a frame short"
        );
        let mut changed = mix.clone();
        changed[1001] += 1;
        assert!(
            !m4a_decodes_to_the_mix(Some(m4a(&changed)), seconds),
            "one sample changed"
        );
        assert!(
            !m4a_decodes_to_the_mix(Some(vec![0; 64]), seconds),
            "not an M4A"
        );
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
        let m4a = m4a::alac_file(&samples, 44_100, None, m4a::Order::FastStart).expect("an M4A");
        let wav = sha256_hex(&planted_wav(SECONDS[0]));
        let oks = |files: Files| -> Vec<bool> {
            file_checks(files, false)
                .into_iter()
                .map(|(ok, _)| ok)
                .collect()
        };
        // The 10-minute M4A of a run is too slow to make here:
        // `an_m4a_check_decodes_to_the_planted_mix` passes that check.
        assert_eq!(
            oks(Files {
                wav_sha256: Some(wav),
                flac: Some(flac),
                mp3: Some(mp3(left.len())),
                m4a_short: Some(m4a.clone()),
                m4a_long: None,
                ..Files::default()
            }),
            [true, true, true, true, false]
        );
        let short_mp3 = file_checks(
            Files {
                mp3: Some(mp3(50 * 44_100)),
                ..Files::default()
            },
            false,
        );
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
                m4a_short: None,
                m4a_long: Some(m4a),
                ..Files::default()
            }),
            [false, false, false, false, false]
        );
    }

    /// The `timeout-minutes` of each `e2e save` step of a workflow, in
    /// order, or `None` for a step that has none.
    fn save_step_limits(ci: &str) -> Vec<Option<u64>> {
        ci.split("e2e save --browser ")
            .skip(1)
            .map(|step| {
                step.lines()
                    .skip(1)
                    .take_while(|l| !l.trim_start().starts_with("- "))
                    .find_map(|l| l.trim().strip_prefix("timeout-minutes:"))
                    .and_then(|m| m.trim().parse().ok())
            })
            .collect()
    }

    /// Finding 3-1 of review round 3 of M8: CI gives each `e2e save` step
    /// the limit that the waits fit in.  The Linux job has one, and the
    /// macOS job one for Safari and one for Chrome (TD-92).
    #[test]
    fn the_ci_step_limit_is_the_waits_limit() {
        let limits = save_step_limits(include_str!("../../../.github/workflows/ci.yml"));
        assert_eq!(limits.len(), 3, "{limits:?}");
        for limit in limits {
            assert_eq!(limit.map(|m| m * 60), Some(STEP_LIMIT_S));
        }
    }

    #[test]
    fn a_save_step_with_no_limit_is_found() {
        let ci = "      - run: cargo run --locked -p xtask -- e2e save --browser chrome\n\
                  \x20       timeout-minutes: 60\n\
                  \x20     - run: cargo run --locked -p xtask -- e2e save --browser safari\n\
                  \x20     - run: cargo run --locked -p xtask -- e2e flac --browser safari\n\
                  \x20       timeout-minutes: 60\n";
        assert_eq!(save_step_limits(ci), [Some(60), None]);
    }

    /// The time, in minutes, that the limit of a job keeps for its steps
    /// other than `e2e save`, by job key.  Each time is more than the
    /// longest measured run of those steps.  In `browser-tests`, they took
    /// 10.5, 15.3 and 19.7 minutes (runs 42777211457, 40894815433 and
    /// 42952206488).  In `macos`, they took 37.0 minutes in run
    /// 42952206488, with no cache.  Its two `web-test` steps now run up to
    /// 3 times (TD-98), 75 and 45 minutes at most, in place of the 14.2
    /// and about 1 measured: 141.8 minutes at most.
    const OTHER_STEPS_MINUTES: [(&str, u64); 2] = [("browser-tests", 20), ("macos", 145)];

    /// A job of a workflow with `e2e save` steps: its key, its
    /// `timeout-minutes`, and the limits of its save steps.
    #[derive(Debug, PartialEq, Eq)]
    struct SaveJob {
        key: String,
        limit: Option<u64>,
        steps: Vec<Option<u64>>,
    }

    /// Each job of the workflow `ci` that has an `e2e save` step, in order.
    fn save_jobs(ci: &str) -> Vec<SaveJob> {
        let Some((_, jobs)) = ci.split_once("\njobs:\n") else {
            return Vec::new();
        };
        let mut parts: Vec<(&str, Vec<&str>)> = Vec::new();
        for line in jobs.lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let key = line
                .strip_prefix("  ")
                .filter(|rest| !rest.starts_with(' '))
                .and_then(|rest| rest.strip_suffix(':'));
            match (key, parts.last_mut()) {
                (Some(key), _) => parts.push((key, Vec::new())),
                (None, Some((_, lines))) => lines.push(line),
                (None, None) => {}
            }
        }
        parts
            .into_iter()
            .map(|(key, lines)| SaveJob {
                key: key.to_string(),
                limit: lines
                    .iter()
                    .find_map(|l| l.strip_prefix("    timeout-minutes:"))
                    .and_then(|m| m.trim().parse().ok()),
                steps: save_step_limits(&lines.join("\n")),
            })
            .filter(|job| !job.steps.is_empty())
            .collect()
    }

    /// Why the limit of `job` is too short, or `None`.  The job limit
    /// must hold each save step at its limit, and the time of the job in
    /// [`OTHER_STEPS_MINUTES`] for the other steps.
    fn job_limit_problem(job: &SaveJob) -> Option<String> {
        let Some(&(_, others)) = OTHER_STEPS_MINUTES.iter().find(|(key, _)| *key == job.key) else {
            return Some(format!(
                "job {}: OTHER_STEPS_MINUTES gives no time for its other steps",
                job.key
            ));
        };
        let steps: Option<u64> = job.steps.iter().copied().sum();
        match (job.limit, steps) {
            (Some(limit), Some(steps)) if limit >= steps + others => None,
            _ => Some(format!(
                "job {}: timeout-minutes {:?} does not hold the save steps {:?} and {others} minutes for the other steps",
                job.key, job.limit, job.steps
            )),
        }
    }

    /// Review round 2 of M2 (2-5): each save step may take its whole
    /// limit, so the job limit must hold all of them.
    #[test]
    fn the_ci_job_limit_holds_its_save_steps() {
        let jobs = save_jobs(include_str!("../../../.github/workflows/ci.yml"));
        let keys: Vec<&str> = jobs.iter().map(|job| job.key.as_str()).collect();
        assert_eq!(keys, ["browser-tests", "macos"]);
        for job in &jobs {
            assert_eq!(job_limit_problem(job), None);
        }
    }

    /// A workflow with one job `key` of `limit` minutes, and a save step of
    /// `step` minutes for each of `steps`.  A comment line that ends with
    /// `:` is in the job.
    fn one_job(key: &str, limit: &str, steps: &[u64]) -> String {
        let saves: String = steps
            .iter()
            .map(|minutes| {
                format!(
                    "      - run: cargo run --locked -p xtask -- e2e save --browser chrome\n\
                     \x20       timeout-minutes: {minutes}\n"
                )
            })
            .collect();
        format!(
            "name: CI\n\
             jobs:\n\
             \x20 lint:\n\
             \x20   runs-on: ubuntu-latest\n\
             \x20   timeout-minutes: 5\n\
             \x20   steps:\n\
             \x20     - run: cargo fmt --all --check\n\
             \x20 # lloyd's browser job:\n\
             \x20 {key}:\n\
             \x20   runs-on: macos-latest\n\
             \x20 # the limit of beachape:\n\
             {limit}\
             \x20   steps:\n\
             \x20     - run: cargo run --locked -p xtask -- e2e smoke --browser chrome\n\
             {saves}"
        )
    }

    fn the_job(key: &str, limit: &str, steps: &[u64]) -> SaveJob {
        let jobs = save_jobs(&one_job(key, limit, steps));
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        jobs.into_iter().next().unwrap()
    }

    #[test]
    fn a_job_reads_its_own_limit_and_its_save_steps() {
        assert_eq!(
            the_job("browser", "    timeout-minutes: 120\n", &[60, 60]),
            SaveJob {
                key: "browser".to_string(),
                limit: Some(120),
                steps: vec![Some(60), Some(60)],
            }
        );
        assert_eq!(the_job("browser", "", &[60]).limit, None);
    }

    /// Review round 3 of M2 (3-4): each job keeps its own time for its
    /// other steps.
    #[test]
    fn a_job_limit_that_the_save_steps_can_use_up_fails() {
        let limit = |minutes: u64| format!("    timeout-minutes: {minutes}\n");
        let problem = job_limit_problem(&the_job("macos", &limit(135), &[60, 60]));
        assert!(
            problem
                .as_deref()
                .is_some_and(|p| p.starts_with("job macos: ")),
            "{problem:?}"
        );
        assert!(job_limit_problem(&the_job("macos", &limit(264), &[60, 60])).is_some());
        assert_eq!(
            job_limit_problem(&the_job("macos", &limit(265), &[60, 60])),
            None
        );
        assert!(job_limit_problem(&the_job("browser-tests", &limit(79), &[60])).is_some());
        assert_eq!(
            job_limit_problem(&the_job("browser-tests", &limit(80), &[60])),
            None
        );
        assert!(job_limit_problem(&the_job("browser-tests", "", &[60])).is_some());
        let no_step_limit = SaveJob {
            key: "macos".to_string(),
            limit: Some(180),
            steps: vec![Some(60), None],
        };
        assert!(job_limit_problem(&no_step_limit).is_some());
    }

    #[test]
    fn a_save_job_with_no_time_for_its_other_steps_fails() {
        let problem = job_limit_problem(&the_job("lloyd", "    timeout-minutes: 500\n", &[60]));
        assert_eq!(
            problem.as_deref(),
            Some("job lloyd: OTHER_STEPS_MINUTES gives no time for its other steps")
        );
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
        assert_eq!(caps_s(), 5 * (60 + 180));
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

    /// M4 of `docs/plans/completed/m4a.md`: a run that saves AAC checks its two
    /// AAC saves as the others: the offer of a `.m4a`, the page's memory,
    /// the JS heap, and the worker's growth.  A run without AAC needs none.
    #[test]
    fn the_save_checks_of_a_run_with_aac_check_each_aac_save() {
        let none = failing_with(&good_aac(), true);
        assert!(none.is_empty(), "{none:?}");
        assert!(failing_with(&good(), false).is_empty());
        let missing = failing_with(&good(), true);
        assert!(
            missing
                .iter()
                .any(|f| f.starts_with("aac: the worker's growth")),
            "{missing:?}"
        );
        assert!(
            missing
                .iter()
                .any(|f| f.starts_with("aac, 60 s: a .m4a file is offered")),
            "{missing:?}"
        );
        let mut grows = good_aac();
        grows[9].worker = Some((65_536.0 + 1_048_577.0, 2e7));
        let failed = failing_with(&grows, true);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(
            failed[0].starts_with("aac: the worker's growth"),
            "{failed:?}"
        );
        let mut offer = good_aac();
        offer[8].offered = Some(("lloyd - mix.mp3".into(), 1.0, String::new()));
        let failed = failing_with(&offer, true);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(
            failed[0].starts_with("aac, 60 s: a .m4a file"),
            "{failed:?}"
        );
        let mut heap = good_aac();
        heap[9].heap = Some(9.0 * MB);
        let failed = failing_with(&heap, true);
        assert_eq!(failed.len(), 2, "{failed:?}");
        assert!(
            failed[0].starts_with("aac: the 10-minute save's JS heap"),
            "{failed:?}"
        );
        assert!(
            failed[1].starts_with("aac, 600 s: the JS heap measure"),
            "{failed:?}"
        );
        let mut wasm = good_aac();
        wasm[9].wasm = Some(6_001.0 + WASM_BOUND_BYTES);
        let failed = failing_with(&wasm, true);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(
            failed[0].starts_with("aac: the 10-minute save grows"),
            "{failed:?}"
        );
        let mut slice = good_aac();
        slice[8].slice_ms = Some(SLICE_LIMIT_MS + 1.0);
        assert_eq!(failing_with(&slice, true).len(), 1);
    }

    #[test]
    fn the_saves_folder_holds_11_files_with_aac() {
        let eleven = json!([
            "1-1.wav", "2-1.wav", "3-1.flac", "4-1.flac", "5-1.m4a", "6-1.m4a", "7-1.mp3",
            "8-1.mp3", "9-1.m4a", "10-1.m4a", "11-1.wav"
        ]);
        let check = saves_folder_check(&eleven, true);
        assert!(check.0, "{}", check.1);
        assert!(check.1.contains("the 11 offered files"), "{}", check.1);
        assert!(
            !saves_folder_check(&eleven, false).0,
            "11 files in a run without AAC"
        );
        let nine = json!([
            "1-1.wav", "2-1.wav", "3-1.flac", "4-1.flac", "5-1.m4a", "6-1.m4a", "7-1.mp3",
            "8-1.mp3", "9-1.wav"
        ]);
        assert!(saves_folder_check(&nine, false).0);
        assert!(
            !saves_folder_check(&nine, true).0,
            "no AAC file in a run with AAC"
        );
        let three = json!([
            "1-1.wav", "2-1.wav", "3-1.flac", "4-1.flac", "5-1.m4a", "6-1.m4a", "7-1.mp3",
            "8-1.mp3", "9-1.m4a", "10-1.wav", "11-1.wav"
        ]);
        assert!(
            !saves_folder_check(&three, true).0,
            "a WAV in place of an AAC"
        );
    }

    #[test]
    fn the_probe_line_and_the_platform_decide_the_aac_saves() {
        let yes = "[0.3s] console.info: [app] app/src/lib.rs:4596: export: AAC encoder: yes";
        let no = "[0.3s] console.info: [app] export: AAC encoder: no (isConfigSupported is false)";
        assert_eq!(probe_line(yes), Some(true));
        assert_eq!(probe_line(no), Some(false));
        assert_eq!(probe_line("export: AAC encoder: "), None);
        assert_eq!(probe_line("separator: ready"), None);
        assert_eq!(aac_rule("macos", Some(true)), (true, None));
        let (saves, why) = aac_rule("macos", Some(false));
        assert!(!saves && why.is_some_and(|w| w.contains("on macOS")));
        assert_eq!(aac_rule("linux", Some(false)), (false, None));
        let (saves, why) = aac_rule("linux", Some(true));
        assert!(saves && why.is_some_and(|w| w.contains("TD-92")));
        assert_eq!(aac_rule("windows", Some(true)), (true, None));
        assert_eq!(aac_rule("windows", Some(false)), (false, None));
        for os in ["macos", "linux", "windows"] {
            let (saves, why) = aac_rule(os, None);
            assert!(
                !saves && why.is_some_and(|w| w.contains("no AAC probe line")),
                "{os}"
            );
        }
    }

    /// An AAC decode passes [`aac_checks_of`] when it is the mix, and fails
    /// a decode one frame late or early, a short one, a noisy one and a
    /// silent start.  The mix here is 2 s of the planted mix.
    #[test]
    fn the_aac_checks_find_a_shift_a_cut_and_a_lost_start() {
        let want = planted_channels(2);
        let audio = |channels: [Vec<f32>; 2]| engine::decode::Audio {
            sample_rate: 44_100,
            channels: channels.into(),
        };
        let failed = |audio: &engine::decode::Audio| -> Vec<String> {
            aac_checks_of(Some(audio), &want, "aac, 2 s")
                .into_iter()
                .filter(|(ok, _)| !ok)
                .map(|(_, what)| what)
                .collect()
        };
        let same = audio(want.clone());
        assert!(failed(&same).is_empty(), "{:?}", failed(&same));
        let late = audio(
            want.clone()
                .map(|c| [vec![0.0], c[..c.len() - 1].to_vec()].concat()),
        );
        let late_failed = failed(&late);
        assert!(
            late_failed
                .iter()
                .any(|f| f.contains("peaks at lag 0") && f.contains("(lag 1)")),
            "{late_failed:?}"
        );
        assert!(
            late_failed.iter().any(|f| f.contains("the error at lag 0")),
            "{late_failed:?}"
        );
        let early = audio(want.clone().map(|c| [c[1..].to_vec(), vec![0.0]].concat()));
        assert!(
            failed(&early).iter().any(|f| f.contains("(lag -1)")),
            "{:?}",
            failed(&early)
        );
        let short = audio(want.clone().map(|c| c[..c.len() - 1].to_vec()));
        assert!(
            failed(&short).iter().any(|f| f.contains("decodes to the")),
            "{:?}",
            failed(&short)
        );
        let silent_start = audio(want.clone().map(|c| {
            let mut c = c;
            c[..1024].fill(0.0);
            c
        }));
        assert!(
            failed(&silent_start)
                .iter()
                .any(|f| f.contains("over the first 4096 frames")),
            "{:?}",
            failed(&silent_start)
        );
        let mut seed = 1_u32;
        let noisy = audio(want.clone().map(|c| {
            c.iter()
                .map(|x| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    x + ((seed >> 8) as f32 / 16_777_216.0 - 0.5) * 0.2
                })
                .collect()
        }));
        assert!(
            failed(&noisy)
                .iter()
                .any(|f| f.contains("the error to the mix is below")),
            "{:?}",
            failed(&noisy)
        );
        let mono = engine::decode::Audio {
            sample_rate: 44_100,
            channels: vec![want[0].clone()],
        };
        assert_eq!(failed(&mono).len(), 1, "a mono decode fails at once");
        assert_eq!(aac_checks_of(None, &want, "aac, 2 s").len(), 1);
    }

    /// The file checks of a run that saves AAC have the five checks of each
    /// AAC file, and a run without AAC has none of them.
    #[test]
    fn the_file_checks_have_the_aac_checks_only_in_a_run_with_aac() {
        assert_eq!(file_checks(Files::default(), false).len(), 5);
        let with = file_checks(Files::default(), true);
        assert_eq!(with.len(), 7, "one failed check for each missing AAC file");
        assert!(with[5].1.starts_with("aac, 60 s") && with[6].1.starts_with("aac, 600 s"));
    }

    /// Review round 1 of M4: the lag window is 10 s from frame 1024, and
    /// keeps 1024 frames from the end of a short mix.
    #[test]
    fn the_aac_lag_window_is_10_s_from_frame_1024() {
        assert_eq!(aac_lag_window(26_460_000), 1024..1024 + 441_000);
        assert_eq!(aac_lag_window(12 * 44_100), 1024..1024 + 441_000);
        assert_eq!(aac_lag_window(88_200), 1024..88_200 - 1024);
        assert!(aac_lag_window(1000).is_empty());
    }

    /// Review round 1 of M4: the error of a silent mix is no number, and a
    /// decode 1.0001 times the mix has an error of 1e-4.
    #[test]
    fn error_at_is_the_relative_error_and_none_for_silence() {
        let silent = vec![0.0_f32; 64];
        assert_eq!(error_at(&silent, &silent, 0, 0..64), None);
        assert_eq!(error_at(&silent, &silent, 0, 0..0), None);
        let want = planted_channels(1)[0].clone();
        let got: Vec<f32> = want.iter().map(|x| x * 1.0001).collect();
        let e = error_at(&got, &want, 0, 0..want.len());
        assert!(e.is_some_and(|e| (e - 1e-4).abs() < 1e-6), "{e:?}");
    }

    /// Review round 1 of M4: a silent decode has no peak, and gives lag 0.
    #[test]
    fn best_lag_of_a_silent_decode_is_0() {
        let want = planted_channels(1)[0].clone();
        let silent = vec![0.0_f32; want.len()];
        assert_eq!(best_lag(&silent, &want, 1024..2048, 16), 0);
    }

    /// Review round 1 of M4: the near check reads the lags -2, -1, 1 and 2,
    /// and fails a decode 0.7 frames early, whose error is least at lag -1.
    #[test]
    fn the_near_check_reads_lags_either_side_and_fails_a_part_frame() {
        let want = planted_channels(2);
        let early = want.clone().map(|c| {
            (0..c.len())
                .map(|i| {
                    let next = c.get(i + 1).copied().unwrap_or(c[i]);
                    c[i] + 0.7 * (next - c[i])
                })
                .collect::<Vec<f32>>()
        });
        let audio = engine::decode::Audio {
            sample_rate: 44_100,
            channels: early.into(),
        };
        let checks = aac_checks_of(Some(&audio), &want, "aac, 2 s");
        let near = checks
            .iter()
            .find(|(_, what)| what.contains("is less than at lags"))
            .expect("the near check");
        assert!(!near.0, "{}", near.1);
        for lag in ["(-2, Some(", "(-1, Some(", "(1, Some(", "(2, Some("] {
            assert!(near.1.contains(lag), "{lag} in {}", near.1);
        }
    }

    /// Review round 1 of M4: the decisions of the browser code of the run.
    #[test]
    fn the_heap_and_the_aac_decisions_of_a_run() {
        assert!(heap_measured(Browser::Chrome));
        assert!(!heap_measured(Browser::Safari));
        assert_eq!(
            heap_report(true, false, "x"),
            (Some(false), "x".to_string())
        );
        assert_eq!(heap_report(true, true, "x"), (Some(true), "x".to_string()));
        assert_eq!(
            heap_report(false, false, "x"),
            (None, "not measured in Safari: x".to_string())
        );
        assert!(aac_not_offered("aac", Some(false)));
        assert!(aac_not_offered("aac", None));
        assert!(!aac_not_offered("aac", Some(true)));
        assert!(!aac_not_offered("alac", Some(false)));
        assert_eq!(worker_kind("wav"), None);
        assert_eq!(worker_kind("aac"), Some("aac"));
        assert_eq!(worker_kind("flac"), Some("flac"));
    }
}
