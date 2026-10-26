//! The page's `playback:` line (D9 of `docs/plans/active/stream-from-storage-m6.md`),
//! which the page logs every 2 s while it plays.

use super::{Opts, Page, in_page, number_after};
use anyhow::Result;
use serde_json::json;

/// One `playback:` line.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlaybackLine {
    /// The lanes that play from storage, and all the lanes.
    pub stored: u64,
    pub lanes: u64,
    pub read_ahead_mib: f64,
    /// The callbacks that missed frames.
    pub underruns: u64,
    pub missed_lane_frames: u64,
    pub held_frames: u64,
    pub failed_lanes: u64,
    pub reads: u64,
    pub p50_ms: f64,
    pub p99_ms: f64,
}

/// The `playback:` line in `line`, if it is one.
pub(super) fn playback_line(line: &str) -> Option<PlaybackLine> {
    let line = &line[line.find("playback: lanes from storage ")?..];
    let count = |key: &str| number_after(line, key).map(|n| n as u64);
    Some(PlaybackLine {
        stored: count("lanes from storage ")?,
        lanes: count(" of ")?,
        read_ahead_mib: number_after(line, "read-ahead ")?,
        underruns: count("underruns ")?,
        missed_lane_frames: count("missed lane frames ")?,
        held_frames: count("held ")?,
        failed_lanes: count("failed lanes ")?,
        reads: count("reads ")?,
        p50_ms: number_after(line, "read p50 ")?,
        p99_ms: number_after(line, "p99 ")?,
    })
}

/// The six stems that the scenario writes into the session, as the worker
/// names them.
const STEMS: [&str; 6] = ["bass", "drums", "other", "vocals", "guitar", "piano"];

/// Writes a 16-bit stereo WAV of the song's frames (from the `decoded` line)
/// for each stem in `arguments[0]`, and its `stem` line, into the current
/// session, as `store::save_stems` writes them.
const PLANT_STEMS: &str = "const root = await navigator.storage.getDirectory();
    const sessions = await root.getDirectoryHandle('sessions');
    const id = (await (await (await sessions.getFileHandle('current')).getFile()).text()).trim();
    const dir = await sessions.getDirectoryHandle(id);
    const metaFile = await dir.getFileHandle('meta');
    const meta = (await (await metaFile.getFile()).text()).trim();
    const decoded = meta.split('\\n').find((l) => l.startsWith('decoded\\t'));
    const frames = Number(decoded.split('\\t')[1]);
    const rate = 44100;
    const stems = await dir.getDirectoryHandle('stems', { create: true });
    let k = 0;
    for (const n of args[0]) {
      k++;
      const b = new DataView(new ArrayBuffer(44 + frames * 4));
      const s = (o, t) => [...t].forEach((c, i) => b.setUint8(o + i, c.charCodeAt(0)));
      s(0, 'RIFF'); b.setUint32(4, 36 + frames * 4, true); s(8, 'WAVEfmt ');
      b.setUint32(16, 16, true); b.setUint16(20, 1, true); b.setUint16(22, 2, true);
      b.setUint32(24, rate, true); b.setUint32(28, rate * 4, true); b.setUint16(32, 4, true); b.setUint16(34, 16, true);
      s(36, 'data'); b.setUint32(40, frames * 4, true);
      for (let i = 0; i < frames * 2; i++) b.setInt16(44 + 2 * i, Math.round(3000 * Math.sin(i * 0.002 * k)), true);
      const w = await (await stems.getFileHandle(`${n}.wav`, { create: true })).createWritable();
      await w.write(b.buffer); await w.close();
    }
    const w = await metaFile.createWritable();
    await w.write(meta + '\\n' + args[0].map((n) => `stem\\t${n}`).join('\\n') + '\\n'); await w.close();
    return frames;";

/// `xtask e2e playback`: a stored song with stems plays from browser
/// storage (M6).
pub(super) fn run(o: &Opts) -> Result<()> {
    let stems = super::synth_stems(&o.out)?;
    let base = super::start_server(
        &o.dist,
        vec![("/stems/".into(), stems)],
        &["console.js", "audio_contexts.js"],
    )?;
    let mut p = Page::open(
        o,
        base,
        &["--autoplay-policy=no-user-gesture-required"],
        "index.html",
    )?;
    let ready = p.wait_for(0, 60, |l| l.contains("separator: "))?;
    p.check(ready.is_some(), "the app started");
    let names = ["drums", "bass", "guitar", "keys", "lead"];
    let urls: Vec<String> = names.iter().map(|n| format!("/stems/{n}.wav")).collect();
    let files: Vec<String> = names.iter().map(|n| format!("{n}.wav")).collect();
    let drop: Vec<(&str, &str, &str)> = urls
        .iter()
        .zip(&files)
        .map(|(u, f)| (u.as_str(), f.as_str(), "audio/wav"))
        .collect();
    let from = p.mark();
    p.drop_files(&drop)?;
    let kept = p.wait_for(from, 120, |l| {
        l.contains("song kept in browser storage as session")
    })?;
    p.check(kept.is_some(), "the synthetic stems open and are kept");
    let frames = in_page(&mut p, PLANT_STEMS, json!([STEMS]), 120)?;
    p.note(&format!("planted six stems of {frames} frames"));
    p.reload()?;
    let from = p.mark();
    let restored = p.wait_for(from, 120, |l| {
        l.contains("and its separated stems from this browser")
    })?;
    p.check(restored.is_some(), "the song restores with its stems");
    p.idle(2.0)?;
    p.screenshot("playback_restored")?;

    let played_from = p.mark();
    p.click(o.play_x, "Play")?;
    let eight = p.wait_for(played_from, 60, |l| {
        super::audio_stats(&[l.to_string()], 0)
            .first()
            .is_some_and(|(_, head)| *head >= 8.0 * RATE)
    })?;
    p.check(eight.is_some(), "it plays 8 s");
    // A click on the bass lane seeks to about 3 s.
    let x = LANE_LEFT + (LANE_RIGHT - LANE_LEFT) * SEEK_S / 30.0;
    let seek_from = p.mark();
    p.note(&format!(
        "click the bass lane at ({x:.0}, {BASS_Y}) to seek to about {SEEK_S} s"
    ));
    p.session.click(x, BASS_Y)?;
    p.idle(1.0)?;
    p.note(&format!(
        "click the Mute of the original lane at {ORIGINAL_MUTE:?}"
    ));
    p.session.click(ORIGINAL_MUTE.0, ORIGINAL_MUTE.1)?;
    p.idle(5.0)?;
    p.screenshot("playback_after_seek")?;
    let heads: Vec<f64> = super::audio_stats(&p.log, seek_from)
        .into_iter()
        .map(|(_, head)| head)
        .collect();
    p.check(
        passes_the_target(&heads, SEEK_S * RATE),
        &format!("the playhead passes the seek target: {heads:?}"),
    );
    let lines: Vec<PlaybackLine> = p.log[played_from..]
        .iter()
        .filter_map(|l| playback_line(l))
        .collect();
    if let Some(first) = lines.first() {
        p.note(&format!("first playback line after Play: {first:?}"));
    }
    if let Some(last) = lines.last() {
        p.note(&format!("last playback line: {last:?}"));
    }
    for (ok, what) in playback_checks(&lines, 7) {
        p.check(ok, &what);
    }
    let errors = p.errors();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish()
}

const RATE: f64 = 44_100.0;
/// Where the waveforms start and end in the viewport, in CSS pixels.
const LANE_LEFT: f64 = 217.0;
const LANE_RIGHT: f64 = 1_090.0;
/// The middle of the bass lane, the first stem.
const BASS_Y: f64 = 200.0;
/// The Mute of "original", the last lane.
const ORIGINAL_MUTE: (f64, f64) = (27.0, 601.0);
const SEEK_S: f64 = 3.0;

/// Whether the playheads after a seek to `target` land within 3 s of it
/// (a stats line comes each 2 s) and then pass it by at least 1 s: it played from there.
fn passes_the_target(heads: &[f64], target: f64) -> bool {
    let Some(landed) = heads
        .iter()
        .position(|h| (target - RATE..target + 3.0 * RATE).contains(h))
    else {
        return false;
    };
    heads[landed..]
        .iter()
        .any(|h| *h > heads[landed] + RATE && *h < target + 10.0 * RATE)
}

/// The checks of the `playback:` lines after Play, for a song of `lanes`
/// lanes that all play from storage.
fn playback_checks(lines: &[PlaybackLine], lanes: u64) -> Vec<(bool, String)> {
    let first = lines.first();
    vec![
        (
            first.is_some_and(|l| l.stored == lanes && l.lanes == lanes),
            format!("the first playback line says lanes from storage {lanes} of {lanes}"),
        ),
        (
            lines.len() >= 3,
            format!("at least 3 playback lines ({})", lines.len()),
        ),
        (lines.iter().all(|l| l.underruns == 0), "underruns 0".into()),
        (
            lines.iter().all(|l| l.failed_lanes == 0),
            "failed lanes 0".into(),
        ),
        (
            lines
                .iter()
                .all(|l| l.read_ahead_mib > 0.0 && l.read_ahead_mib <= 32.0),
            "a read-ahead of at most 32 MiB".into(),
        ),
        (
            lines.last().is_some_and(|l| l.reads > 0),
            "the reader read blocks".into(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playback_line_gives_its_numbers() {
        let line = "[9.1s] console.info: [app] app/src/lib.rs:1500: playback: lanes from storage 7 of 7, read-ahead 24.0 MiB, underruns 0 (callbacks), missed lane frames 0, held 2048 frames, failed lanes 0, reads 31, read p50 3.5 ms, p99 12.0 ms";
        assert_eq!(
            playback_line(line),
            Some(PlaybackLine {
                stored: 7,
                lanes: 7,
                read_ahead_mib: 24.0,
                underruns: 0,
                missed_lane_frames: 0,
                held_frames: 2048,
                failed_lanes: 0,
                reads: 31,
                p50_ms: 3.5,
                p99_ms: 12.0,
            })
        );
        let some = "playback: lanes from storage 6 of 8, read-ahead 23.5 MiB, underruns 3 (callbacks), missed lane frames 1200, held 0 frames, failed lanes 1, reads 0, read p50 0.0 ms, p99 0.0 ms";
        let got = playback_line(some).expect("a line");
        assert_eq!((got.stored, got.lanes, got.underruns), (6, 8, 3));
        assert_eq!((got.missed_lane_frames, got.failed_lanes), (1200, 1));
        assert_eq!(
            playback_line("audio stats: 12 callbacks of 512 frames"),
            None
        );
        assert_eq!(playback_line("playback: lanes from storage 7 of"), None);
    }

    fn line(stored: u64, underruns: u64, failed: u64, mib: f64) -> PlaybackLine {
        PlaybackLine {
            stored,
            lanes: 7,
            read_ahead_mib: mib,
            underruns,
            missed_lane_frames: 0,
            held_frames: 0,
            failed_lanes: failed,
            reads: 3,
            p50_ms: 1.0,
            p99_ms: 2.0,
        }
    }

    fn failing(lines: &[PlaybackLine]) -> usize {
        playback_checks(lines, 7)
            .iter()
            .filter(|(ok, _)| !ok)
            .count()
    }

    #[test]
    fn the_playback_checks_pass_a_good_run_and_fail_bad_ones() {
        let good = vec![line(7, 0, 0, 24.0); 3];
        assert_eq!(failing(&good), 0);
        assert_eq!(failing(&[line(7, 0, 0, 24.0)]), 1, "too few lines");
        assert_eq!(failing(&[]), 3, "no lines");
        let mut memory = good.clone();
        memory[0].stored = 6;
        assert_eq!(failing(&memory), 1, "a lane from memory");
        let mut underrun = good.clone();
        underrun[2].underruns = 1;
        assert_eq!(failing(&underrun), 1);
        let mut failed = good.clone();
        failed[1].failed_lanes = 1;
        assert_eq!(failing(&failed), 1);
        let mut big = good.clone();
        big[1].read_ahead_mib = 33.0;
        assert_eq!(failing(&big), 1);
        let mut none_read = good;
        none_read[2].reads = 0;
        assert_eq!(failing(&none_read), 1);
    }

    #[test]
    fn the_playhead_passes_the_target_only_after_landing_near_it() {
        let target = 3.0 * RATE;
        assert!(passes_the_target(
            &[9.0 * RATE, 3.2 * RATE, 4.0 * RATE, 5.5 * RATE],
            target
        ));
        assert!(
            !passes_the_target(&[9.0 * RATE, 10.0 * RATE, 12.0 * RATE], target),
            "no seek"
        );
        assert!(
            !passes_the_target(&[3.1 * RATE, 3.1 * RATE], target),
            "it did not play on"
        );
        assert!(!passes_the_target(&[], target));
    }
}
