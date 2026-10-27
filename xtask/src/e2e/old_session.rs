//! `xtask e2e old-session` (step 9 of
//! `docs/plans/active/stream-from-storage-m7.md`): a session of M4's
//! layout restores with the page's memory under a fixed bound.
//!
//! The scenario plants a session of M4's layout in browser storage: a
//! 16-bit WAV source in `src/`, a `meta` with `file`, `rate` and six `stem`
//! lines, six 16-bit stem WAVs, and no `decoded` line.  It plants one of 1
//! minute, then one of 5 minutes, and reloads for each.  The song restores
//! through the decode worker (M5) and the stem overviews (D7 of M7).  The
//! first `playback:` line after Play must say `lanes from storage 7 of 7`.
//!
//! After that line, the driver reads the page's WebAssembly memory with a
//! script, `(await window.wasmBindings.default()).memory.buffer.byteLength`:
//! Trunk's loader keeps the app's JS bindings in `window.wasmBindings`, and
//! their `default` (wasm-bindgen's `init`) gives the exports that it made
//! once, with `memory`.  So the same check runs on an older build, which
//! logs no memory line there.  The memory of each length must be under
//! [`OLD_SESSION_BOUND_MB`], and the 5-minute value at most
//! [`MAX_GROWTH_MB`] above the 1-minute value.

use super::playback::{PlaybackLine, playback_line};
use super::{Opts, Page, in_page};
use anyhow::Result;
use serde_json::json;
use std::time::Duration;

/// The bound of the page's WebAssembly memory after a restore, in MB: the
/// 1-minute value of the new code, rounded up to the next 8 MB, plus 16 MB
/// (step 9 of the M7 plan).  It covers the fixed part: the app, the
/// read-ahead buffer (24 MiB for this song) and the overviews.  The new
/// code measured 40.2 MB for 1 minute and 40.3 MB for 5 minutes in Chrome
/// on 2026-10-27, so 48 + 16.
pub(super) const OLD_SESSION_BOUND_MB: f64 = 64.0;

/// The most that the 5-minute value may be above the 1-minute value, in MB.
pub(super) const MAX_GROWTH_MB: f64 = 4.0;

/// The lengths of the two sessions, in seconds.
const SECONDS: [u32; 2] = [60, 300];

/// The six stems, as the worker names them.
const STEMS: [&str; 6] = ["bass", "drums", "other", "vocals", "guitar", "piano"];

/// Plants session `args[0]` of M4's layout, of `args[1]` seconds, and makes
/// it current.  Each WAV is made a block at a time, in JS.
const PLANT: &str = "const [id, seconds, stems] = args;
    const frames = seconds * 44100;
    const root = await navigator.storage.getDirectory();
    const sessions = await root.getDirectoryHandle('sessions', { create: true });
    const dir = await sessions.getDirectoryHandle(id, { create: true });
    async function wav(folder, name, k) {
      const w = await (await folder.getFileHandle(name, { create: true })).createWritable();
      const h = new DataView(new ArrayBuffer(44));
      const s = (o, t) => [...t].forEach((c, i) => h.setUint8(o + i, c.charCodeAt(0)));
      s(0, 'RIFF'); h.setUint32(4, 36 + frames * 4, true); s(8, 'WAVEfmt ');
      h.setUint32(16, 16, true); h.setUint16(20, 1, true); h.setUint16(22, 2, true);
      h.setUint32(24, 44100, true); h.setUint32(28, 44100 * 4, true); h.setUint16(32, 4, true); h.setUint16(34, 16, true);
      s(36, 'data'); h.setUint32(40, frames * 4, true);
      await w.write(h.buffer);
      const block = 131072;
      for (let at = 0; at < frames; at += block) {
        const n = Math.min(block, frames - at);
        const b = new Int16Array(2 * n);
        for (let i = 0; i < n; i++) {
          const v = Math.round(3000 * Math.sin((at + i) * 0.002 * k));
          b[2 * i] = v;
          b[2 * i + 1] = Math.round(v / 2);
        }
        await w.write(b);
      }
      await w.close();
    }
    await wav(await dir.getDirectoryHandle('src', { create: true }), '0.wav', 9);
    const stemDir = await dir.getDirectoryHandle('stems', { create: true });
    for (let k = 0; k < stems.length; k++) await wav(stemDir, `${stems[k]}.wav`, k + 1);
    const meta = 'name\\tlloyd\\nrate\\t44100\\nfile\\tlloyd.wav\\t0.wav\\n'
      + stems.map((n) => `stem\\t${n}`).join('\\n') + '\\n';
    let w = await (await dir.getFileHandle('meta', { create: true })).createWritable();
    await w.write(meta); await w.close();
    w = await (await sessions.getFileHandle('current', { create: true })).createWritable();
    await w.write(id); await w.close();
    return frames;";

/// The page's WebAssembly memory, in bytes (see the module doc).
const MEMORY: &str = "const done = arguments[arguments.length - 1];
    (async () => (await window.wasmBindings.default()).memory.buffer.byteLength)()
      .then(done, (e) => done(`error: ${e}`));";

/// What one restore gave: the first `playback:` line after Play, and the
/// page's WebAssembly memory after it, in MB.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Restore {
    pub seconds: u32,
    pub first_line: Option<PlaybackLine>,
    pub memory_mb: Option<f64>,
}

/// The checks of the scenario: each restore plays its 7 lanes from
/// storage, and its memory is under `bound`; the last memory is at most
/// [`MAX_GROWTH_MB`] above the first.
pub(super) fn old_session_checks(restores: &[Restore], bound: f64) -> Vec<(bool, String)> {
    let mut checks = Vec::new();
    for r in restores {
        let line = r.first_line.as_ref();
        checks.push((
            line.is_some_and(|l| l.stored == 7 && l.lanes == 7),
            format!(
                "{} s: the first playback line says lanes from storage 7 of 7 ({:?})",
                r.seconds,
                line.map(|l| (l.stored, l.lanes))
            ),
        ));
        checks.push((
            r.memory_mb.is_some_and(|mb| mb < bound),
            format!(
                "{} s: the page's memory {:?} MB is under {bound} MB",
                r.seconds, r.memory_mb
            ),
        ));
    }
    let growth = match (restores.first(), restores.last()) {
        (Some(a), Some(b)) if restores.len() > 1 => {
            b.memory_mb.zip(a.memory_mb).map(|(b, a)| b - a)
        }
        _ => None,
    };
    checks.push((
        growth.is_some_and(|g| g <= MAX_GROWTH_MB),
        format!(
            "the page's memory grows by {growth:?} MB with the song, at most {MAX_GROWTH_MB} MB"
        ),
    ));
    checks
}

/// `xtask e2e old-session`.
pub(super) fn run(o: &Opts) -> Result<()> {
    let base = super::start_server(&o.dist, Vec::new(), &["console.js", "audio_contexts.js"])?;
    let mut p = Page::open(
        o,
        base,
        &["--autoplay-policy=no-user-gesture-required"],
        "index.html",
    )?;
    let ready = p.wait_for(0, 60, |l| l.contains("separator: "))?;
    p.check(ready.is_some(), "the app started");
    let mut restores = Vec::new();
    for seconds in SECONDS {
        let id = format!("{}", 1_792_640_000_000u64 + u64::from(seconds));
        let frames = in_page(&mut p, PLANT, json!([id, seconds, STEMS]), 900)?;
        p.note(&format!(
            "planted a session of M4's layout of {frames} frames"
        ));
        p.reload()?;
        let from = p.mark();
        let restored = p.wait_for(from, 900, |l| {
            l.contains("and its separated stems from this browser")
        })?;
        p.check(
            restored.is_some(),
            &format!("{seconds} s: the song restores with its stems"),
        );
        p.idle(2.0)?;
        let played = p.mark();
        p.click(o.play_x, "Play")?;
        let first = p.wait_for(played, 60, |l| playback_line(l).is_some())?;
        let first_line = first.as_deref().and_then(playback_line);
        let bytes = p
            .session
            .execute_async(MEMORY, json!([]), Duration::from_secs(30))?;
        let memory_mb = bytes.as_f64().map(|b| b / 1e6);
        p.note(&format!(
            "old-session: the {seconds}-second song: page memory {memory_mb:?} MB after the first playback line"
        ));
        p.click(o.play_x, "Pause")?;
        restores.push(Restore {
            seconds,
            first_line,
            memory_mb,
        });
    }
    for (ok, what) in old_session_checks(&restores, OLD_SESSION_BOUND_MB) {
        p.check(ok, &what);
    }
    let errors = p.errors();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(stored: u64) -> PlaybackLine {
        PlaybackLine {
            stored,
            lanes: 7,
            read_ahead_mib: 24.0,
            underruns: 0,
            missed_lane_frames: 0,
            held_frames: 0,
            failed_lanes: 0,
            lanes_read: 6,
            reads: 3,
            p50_ms: 1.0,
            p99_ms: 2.0,
        }
    }

    fn restore(seconds: u32, stored: u64, mb: f64) -> Restore {
        Restore {
            seconds,
            first_line: Some(line(stored)),
            memory_mb: Some(mb),
        }
    }

    fn failing(restores: &[Restore]) -> usize {
        old_session_checks(restores, 100.0)
            .iter()
            .filter(|(ok, _)| !ok)
            .count()
    }

    #[test]
    fn the_old_session_checks_pass_a_good_run_and_fail_bad_ones() {
        let good = [restore(60, 7, 60.0), restore(300, 7, 63.9)];
        assert_eq!(failing(&good), 0);
        assert_eq!(
            failing(&[restore(60, 7, 60.0), restore(300, 7, 64.1)]),
            1,
            "it grows with the song"
        );
        assert_eq!(
            failing(&[restore(60, 7, 100.0), restore(300, 7, 100.0)]),
            2,
            "over the bound twice"
        );
        assert_eq!(
            failing(&[restore(60, 6, 60.0), restore(300, 7, 60.0)]),
            1,
            "a lane from memory"
        );
        let mut no_line = restore(300, 7, 60.0);
        no_line.first_line = None;
        assert_eq!(
            failing(&[restore(60, 7, 60.0), no_line]),
            1,
            "no playback line"
        );
        let mut no_memory = restore(300, 7, 60.0);
        no_memory.memory_mb = None;
        assert_eq!(
            failing(&[restore(60, 7, 60.0), no_memory]),
            2,
            "no memory read: no bound and no growth"
        );
        assert_eq!(
            failing(&[restore(60, 7, 60.0)]),
            1,
            "one restore: no growth"
        );
        // The control on an older build: the read-back of seven lanes of 5
        // minutes is about 740 MB.
        assert_eq!(failing(&[restore(60, 7, 200.0), restore(300, 7, 900.0)]), 3);
    }
}
