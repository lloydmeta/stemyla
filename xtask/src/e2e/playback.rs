//! The page's `playback:` line (D9 of `docs/plans/active/stream-from-storage-m6.md`),
//! which the page logs every 2 s while it plays.

use super::number_after;

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
}
