//! Where the player reads the frames of each lane (M6, D1).
//!
//! A [`StemSource`] gives the frames of one lane from a position.  A lane in
//! memory is a [`MemorySource`]: two [`Samples`], shared, not copied.  A lane
//! in a session file is a `readahead::StoredSource`, which copies only from
//! the read-ahead buffer.  The audio callback calls [`StemSource::read`],
//! so no source waits, allocates or frees there (RT-1).
//!
//! [`next_run`] gives the frames that the player reads next from a
//! position.  The player asks [`StemSource::ready`] for that run before it
//! plays from a position that it has not played from yet, and the reader
//! reads the same run first.

use crate::song::{Samples, Song};
use std::sync::Arc;

/// The frames of every stored lane that the reader reads together.
pub const BLOCK_FRAMES: u64 = 32_768;

/// What the player tells its sources at the end of each `process`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    /// The player's own audible position, and its read position.
    Reading { heard: u64, read: u64 },
    /// The run that the reader reads first: the target of a seek that holds
    /// or fades out, or the position that the player has not played from
    /// since a jump; or none.
    Seek(Option<u64>),
}

/// The frames of one lane of a song.
pub trait StemSource: Send + Sync {
    /// Fills `left` and `right` (the same length) with the frames from
    /// `from`.  A frame at or after the lane's end is 0.0.  A frame that
    /// the source does not hold now is 0.0, and the source counts an
    /// underrun.  It never blocks, allocates or frees (RT-1).
    fn read(&self, from: u64, left: &mut [f32], right: &mut [f32]);

    /// Whether [`Self::read`] of `frames` frames from `from` gives real
    /// frames now.
    fn ready(&self, from: u64, frames: usize) -> bool;

    /// Stores what the player tells the source.  Atomics only (RT-1).
    fn hint(&self, hint: Hint);
}

/// A lane in memory.  Clones of its [`Samples`] share their buffers (MEM-1).
pub struct MemorySource {
    left: Samples,
    right: Samples,
}

impl MemorySource {
    pub fn new(left: Samples, right: Samples) -> Self {
        Self { left, right }
    }

    /// Whether this source reads from these buffers, not a copy of them.
    pub fn shares(&self, left: &Samples, right: &Samples) -> bool {
        self.left.shares(left) && self.right.shares(right)
    }
}

fn copy_from(samples: &[f32], from: u64, out: &mut [f32]) {
    let start = usize::try_from(from)
        .unwrap_or(usize::MAX)
        .min(samples.len());
    let n = (samples.len() - start).min(out.len());
    out[..n].copy_from_slice(&samples[start..start + n]);
    out[n..].fill(0.0);
}

impl StemSource for MemorySource {
    fn read(&self, from: u64, left: &mut [f32], right: &mut [f32]) {
        copy_from(&self.left, from, left);
        copy_from(&self.right, from, right);
    }

    fn ready(&self, _from: u64, _frames: usize) -> bool {
        true
    }

    fn hint(&self, _hint: Hint) {}
}

/// A lane that plays silence: a stored lane when no read-ahead buffer
/// holds it (D2 of the M7 plan).  It never falls back to samples.
pub struct SilentSource;

impl StemSource for SilentSource {
    fn read(&self, _from: u64, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);
    }

    fn ready(&self, _from: u64, _frames: usize) -> bool {
        true
    }

    fn hint(&self, _hint: Hint) {}
}

/// One memory source for each stem of `song`, in its order, with no copy.
pub fn memory_sources(song: &Song) -> Vec<Arc<dyn StemSource>> {
    song.stems
        .iter()
        .map(|stem| {
            Arc::new(MemorySource::new(stem.left.clone(), stem.right.clone()))
                as Arc<dyn StemSource>
        })
        .collect()
}

/// A loop `(s, e)` is in force when `s < e'`, with `e' = min(e, frames)`,
/// as the player reads it.  Gives `(s, e')`.
pub fn loop_in_force(looping: Option<(u64, u64)>, frames: u64) -> Option<(u64, u64)> {
    let (s, e) = looping?;
    let e = e.min(frames);
    (s < e).then_some((s, e))
}

/// Up to three runs of frames, `start..end` each; an empty run is unused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Run(pub [(u64, u64); 3]);

impl Run {
    /// The runs that hold frames.
    pub fn parts(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.0.iter().copied().filter(|(start, end)| start < end)
    }

    /// Whether `ready` holds for each part.
    pub fn ready(&self, ready: impl Fn(u64, usize) -> bool) -> bool {
        self.parts()
            .all(|(start, end)| ready(start, (end - start) as usize))
    }
}

/// The next run from `p`: the frames that the player reads next from a
/// position (D6 of the M6 plan).  `xf` is the player's crossfade.
///
/// * From `p` to `min(p + BLOCK_FRAMES, frames)`.
/// * With a loop in force and `s <= p < e'`, only to
///   `min(p + BLOCK_FRAMES, e' + xf, frames)`, because the loop crossfade
///   reads the `xf` frames after `e'`.
/// * If that run reaches `e'`, also the loop's first frames, from `s` to
///   `min(s + BLOCK_FRAMES, e' + xf, frames)`.
/// * With a loop in force and `p >= e'`, the first read wraps at once to
///   `w = dsp::wrap_loop(p, s, e')`, with the crossfade from `p`: the run
///   is `p` to `min(p + xf, frames)`, then the next run from `w`.
pub fn next_run(p: u64, frames: u64, looping: Option<(u64, u64)>, xf: u64) -> Run {
    let Some((s, e)) = loop_in_force(looping, frames) else {
        return Run([(p, (p + BLOCK_FRAMES).min(frames)), (0, 0), (0, 0)]);
    };
    let tail = (e + xf).min(frames);
    let inside = |p: u64| -> [(u64, u64); 2] {
        let end = (p + BLOCK_FRAMES).min(tail);
        let wrap = if end >= e {
            (s, (s + BLOCK_FRAMES).min(tail))
        } else {
            (0, 0)
        };
        [(p, end), wrap]
    };
    if p >= e {
        let w = dsp::wrap_loop(p as usize, s as usize, e as usize) as u64;
        let [first, wrap] = inside(w);
        return Run([(p, (p + xf).min(frames)), first, wrap]);
    }
    if p >= s {
        let [first, wrap] = inside(p);
        return Run([first, wrap, (0, 0)]);
    }
    let end = (p + BLOCK_FRAMES).min(frames);
    let wrap = if end >= e {
        (s, (s + BLOCK_FRAMES).min(tail))
    } else {
        (0, 0)
    };
    Run([(p, end), wrap, (0, 0)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Stem;

    #[test]
    fn a_silent_source_reads_silence() {
        let (mut l, mut r) = ([0.5f32; 4], [-0.5f32; 4]);
        assert!(SilentSource.ready(0, 4));
        SilentSource.read(3, &mut l, &mut r);
        assert_eq!((l, r), ([0.0; 4], [0.0; 4]));
    }

    #[test]
    fn memory_sources_share_the_stems() {
        let song = Song {
            sample_rate: 44_100,
            frames: 4,
            stems: vec![
                Stem {
                    name: "lloyd".into(),
                    left: vec![0.1, 0.2, 0.3, 0.4].into(),
                    right: vec![0.5, 0.6, 0.7, 0.8].into(),
                },
                Stem {
                    name: "acme".into(),
                    left: vec![1.0; 4].into(),
                    right: vec![2.0; 4].into(),
                },
            ],
        };
        let handles = |song: &Song| -> Vec<usize> {
            song.stems
                .iter()
                .flat_map(|s| [s.left.handles(), s.right.handles()])
                .collect()
        };
        assert_eq!(handles(&song), [1; 4]);
        let sources = memory_sources(&song);
        assert_eq!(sources.len(), 2);
        assert_eq!(handles(&song), [2; 4], "MEM-1: each source shares its stem");
        for (stem, source) in song.stems.iter().zip(&sources) {
            let (mut l, mut r) = ([9.0; 3], [9.0; 3]);
            source.read(2, &mut l, &mut r);
            assert_eq!(l, [stem.left[2], stem.left[3], 0.0]);
            assert_eq!(r, [stem.right[2], stem.right[3], 0.0]);
        }
        let stem = &song.stems[0];
        let shared = MemorySource::new(stem.left.clone(), stem.right.clone());
        assert!(shared.shares(&stem.left, &stem.right), "MEM-1: no copy");
        let copied = MemorySource::new(vec![0.1, 0.2, 0.3, 0.4].into(), stem.right.clone());
        assert!(!copied.shares(&stem.left, &stem.right));
    }

    #[test]
    fn a_memory_source_reads_zeros_past_its_end() {
        let source = MemorySource::new(vec![1.0; 3].into(), vec![2.0; 3].into());
        let (mut l, mut r) = ([9.0; 2], [9.0; 2]);
        source.read(u64::MAX, &mut l, &mut r);
        assert_eq!((l, r), ([0.0; 2], [0.0; 2]));
        assert!(source.ready(u64::MAX, 2));
    }

    const B: u64 = BLOCK_FRAMES;

    #[test]
    fn the_next_run_follows_the_loop() {
        let frames = 10 * B;
        // No loop: one block, cut at the song's end.
        assert_eq!(next_run(5, frames, None, 256).0[0], (5, 5 + B));
        assert_eq!(
            next_run(frames - 10, frames, None, 256).0[0],
            (frames - 10, frames)
        );
        // A loop that is not in force changes nothing.
        assert_eq!(
            next_run(5, frames, Some((B, B)), 256),
            next_run(5, frames, None, 256)
        );
        // Inside a short loop: to its end and the crossfade, then its start.
        let run = next_run(1_100, frames, Some((1_000, 2_000)), 256);
        assert_eq!(run.0, [(1_100, 2_256), (1_000, 2_256), (0, 0)]);
        // Inside a long loop, far from its end: one block.
        let run = next_run(B, frames, Some((0, 5 * B)), 256);
        assert_eq!(run.0, [(B, 2 * B), (0, 0), (0, 0)]);
        // Before the loop, reaching its end: the loop's first block too.
        let run = next_run(100, frames, Some((200, 300)), 256);
        assert_eq!(run.0, [(100, 100 + B), (200, 556), (0, 0)]);
        // Before the loop, reaching it, with a loop that holds more than a
        // block after its start: the wrap reads one block from its start.
        let run = next_run(B - 50, frames, Some((B - 10, 2 * B - 50)), 256);
        assert_eq!(run.0, [(B - 50, 2 * B - 50), (B - 10, 2 * B - 10), (0, 0)]);
        // Before the loop, not reaching it.
        let run = next_run(0, frames, Some((2 * B, 3 * B)), 256);
        assert_eq!(run.0, [(0, B), (0, 0), (0, 0)]);
        // After the loop's end: the crossfade from p, then from the wrap.
        let run = next_run(5 * B + 10, frames, Some((0, 2 * B)), 256);
        assert_eq!(
            run.0,
            [(5 * B + 10, 5 * B + 266), (B + 10, 2 * B + 10), (0, B)]
        );
        // A loop past the song's end is cut at the song's end.
        let run = next_run(frames - 100, frames, Some((frames - 200, frames + 50)), 256);
        assert_eq!(
            run.0,
            [(frames - 100, frames), (frames - 200, frames), (0, 0)]
        );
    }
}
