//! `xtask e2e save` (step 4 of `docs/plans/active/stream-from-storage-m8.md`):
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
//! `performance.memory.usedJSHeapSize` reads V8's used heap and the
//! backing stores of `ArrayBuffer`s at each call.  An injected script
//! (`xtask/e2e/save_spy.js`) calls `gc()` and reads the value before the
//! save, every 250 ms during it, and once at its end.  The measure is the
//! largest value less the value before.  The end of a save is the click of
//! its download link, which the script keeps instead of downloading: every
//! build makes it.  A calibration comes first: a 64 MiB `Uint8Array` must
//! raise the measure by at least 60 MiB.
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
//! and the encoder's delay.  At the end, `saves/` holds only the offered
//! files.  It prints every measure, and runs every check also after one
//! failed, so a run of an older build shows its numbers.

use super::old_session::PLANT;
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
pub(super) const FLAC_BOUND_BYTES: f64 = 1_048_576.0;
/// D10: a worker's MP3 encode grows its memory by at most this many times
/// its MP3, plus 1 MiB.
pub(super) const MP3_FACTOR: f64 = 4.0;
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
            mp3.is_some_and(|(growth, file)| file > 0.0 && growth <= MP3_FACTOR * file + FLAC_BOUND_BYTES),
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

/// The checks of the 1-minute files: the WAV is the planted mix, the FLAC
/// decodes to its samples, and the MP3 to its frames within the tolerance.
pub(super) fn file_checks(
    wav_sha256: Option<&str>,
    flac: Option<Vec<u8>>,
    mp3: Option<Vec<u8>>,
) -> Vec<(bool, String)> {
    let want = planted_wav(SECONDS[0]);
    let samples = planted_mix(SECONDS[0]);
    let frames = samples.len() / 2;
    let flac_ok = flac
        .and_then(|b| engine::decode_bytes(b, Some("flac")).ok())
        .is_some_and(|audio| {
            audio.frames() == frames
                && audio.channels.len() == 2
                && (0..frames).all(|i| {
                    (0..2).all(|c| audio.channels[c][i] == f32::from(samples[2 * i + c]) / 32_768.0)
                })
        });
    let mp3_frames = mp3
        .and_then(|b| engine::decode_bytes(b, Some("mp3")).ok())
        .map(|audio| audio.frames());
    vec![
        (
            wav_sha256 == Some(sha256_hex(&want).as_str()),
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
            mp3_frames.is_some_and(|n| n.abs_diff(frames) <= MP3_FRAMES_TOLERANCE),
            format!(
                "mp3, {} s: the file decodes to {mp3_frames:?} frames, within {MP3_FRAMES_TOLERANCE} of the WAV's {frames}",
                SECONDS[0]
            ),
        ),
    ]
}

/// The check of `saves/` at the end: only the offered files, two of each
/// format, and so no WAV of a FLAC or MP3 save.
pub(super) fn saves_folder_check(names: &Value) -> (bool, String) {
    let names: Vec<&str> = names
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let count = |ext: &str| names.iter().filter(|n| n.ends_with(ext)).count();
    (
        names.len() == 6 && count(".wav") == 2 && count(".flac") == 2 && count(".mp3") == 2,
        format!("saves/ holds the 6 offered files and nothing else: {names:?}"),
    )
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
    let ready = p.wait_for(0, 60, |l| l.contains("separator: "))?;
    p.check(ready.is_some(), "the app started");
    let calibration = in_page(&mut p, CALIBRATE, json!([CALIBRATION_BYTES]), 60)?.as_f64();
    p.note(&format!(
        "save: calibration: a {CALIBRATION_BYTES}-byte array raised the measure by {calibration:?} bytes"
    ));
    p.check(
        calibration.is_some_and(|c| c >= CALIBRATION_LEAST),
        &format!("the measure sees a 64 MiB array ({calibration:?} bytes)"),
    );
    in_page(&mut p, CLEAN, json!([]), 120)?;
    for seconds in SECONDS {
        let frames = in_page(
            &mut p,
            PLANT,
            json!([session_id(seconds), seconds, STEMS]),
            900,
        )?;
        p.note(&format!("save: planted a session of {frames} frames"));
    }
    // The first restore of a session decodes it and makes its stem
    // overviews, which a save would overlap: each session is restored once
    // first.
    for seconds in SECONDS {
        let from = p.mark();
        restore(&mut p, seconds)?;
        let overviews = p.wait_for(from, 900, |l| l.contains(OVERVIEWS_END))?;
        p.note(&format!(
            "save: {seconds} s: the first restore made the stem overviews: {}",
            overviews.is_some()
        ));
    }
    let mut saves = Vec::new();
    let (mut wav_sha256, mut flac, mut mp3) = (None, None, None);
    for (format, row) in FORMATS {
        for seconds in SECONDS {
            let save = save_once(&mut p, o, format, row, seconds)?;
            p.note(&format!("save: {save:?}"));
            if seconds == SECONDS[0] && save.offered.is_some() {
                let whole = format != "wav";
                let file = in_page(&mut p, LAST_FILE, json!([whole]), 300)?;
                p.note(&format!(
                    "save: {format}, {seconds} s: the file starts with {:?}, {} bytes",
                    file["head"], file["size"]
                ));
                let bytes = file["base64"]
                    .as_str()
                    .and_then(|b| crate::webdriver::base64_decode(b).ok());
                match format {
                    "wav" => wav_sha256 = file["sha256"].as_str().map(String::from),
                    "flac" => flac = bytes,
                    _ => mp3 = bytes,
                }
            }
            saves.push(save);
        }
    }
    let names = in_page(&mut p, SAVE_NAMES, json!([]), 60)?;
    for s in &saves {
        p.note(&format!(
            "save: {} {} s: JS heap measure {:?} MB; worker growth and file {:?}",
            s.format,
            s.seconds,
            s.heap.map(|h| h / MB),
            s.worker
        ));
    }
    for (ok, what) in
        save_checks(&saves)
            .into_iter()
            .chain(file_checks(wav_sha256.as_deref(), flac, mp3))
    {
        p.check(ok, &what);
    }
    let (ok, what) = saves_folder_check(&names);
    p.check(ok, &what);
    in_page(&mut p, CLEAN, json!([]), 120)?;
    let errors = p.errors();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish()
}

/// The decode worker's line at the end of an `overviews` request.
const OVERVIEWS_END: &str = "at the end of the overviews request";

/// Makes the session of `seconds` current and reloads, until the song shows
/// with its stems.
fn restore(p: &mut Page, seconds: u32) -> Result<()> {
    in_page(p, MAKE_CURRENT, json!([session_id(seconds)]), 60)?;
    p.reload()?;
    let from = p.mark();
    let restored = p.wait_for(from, 900, |l| {
        l.contains("and its separated stems from this browser")
    })?;
    if restored.is_none() {
        bail!("{seconds} s: the song did not restore");
    }
    Ok(())
}

/// Restores the session of `seconds`, saves its mix as `format` through
/// the Save menu (its item at `row`), and measures the save.
fn save_once(p: &mut Page, o: &Opts, format: &str, row: usize, seconds: u32) -> Result<Save> {
    restore(p, seconds)?;
    p.idle(3.0)?;
    let before = in_page(p, "return await window.__e2e_heap();", json!([]), 60)?;
    p.session.execute("window.__e2e_start_heap();", json!([]))?;
    let mark = p.mark();
    p.click_at(o.save_x, o.save_y, "the mix lane's Save")?;
    p.idle(0.5)?;
    p.screenshot(&format!("{format}-{seconds}-menu"))?;
    let item_y = o.item_y + o.item_dy * row as f64;
    p.click_at(o.item_x, item_y, &format!("the {format} item"))?;
    let offered = p.wait_for(mark, 900, |l| l.contains("e2e: offered "))?;
    let max = in_page(p, "return await window.__e2e_stop_heap();", json!([]), 60)?;
    let heap = max.as_f64().zip(before.as_f64()).map(|(m, b)| m - b);
    let offered = offered.and_then(|l| {
        let json: Value = serde_json::from_str(l.split("e2e: offered ").nth(1)?).ok()?;
        Some((
            json["name"].as_str()?.to_string(),
            json["size"].as_f64()?,
            json["mime"].as_str()?.to_string(),
        ))
    });
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
        assert_eq!(failed.len(), 2, "{failed:?}");
        assert!(failed[0].starts_with("wav:") && failed[1].starts_with("flac:"));
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
        let six = [
            "1-1.wav", "2-1.wav", "3-1.flac", "4-1.flac", "5-1.mp3", "6-1.mp3",
        ];
        assert!(saves_folder_check(&names(&six)).0);
        let mut left = six.to_vec();
        left.push("5-1.wav");
        assert!(!saves_folder_check(&names(&left)).0, "a WAV of an MP3 save");
        assert!(!saves_folder_check(&Value::Null).0);
    }
}
