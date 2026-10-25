//! Audio to save, read one block at a time: one stem, or a mix of stems.
//!
//! A saved file is made block by block, so neither a stem nor a mix needs a
//! second song-sized buffer in the page's WebAssembly memory (MEM-1).

use crate::params::SharedParams;
use crate::song::{Song, Stem};
use std::sync::Arc;

/// The gain of each stem in a mix, fixed when it is made.  So a control
/// moved while a mix is saved does not change half of the file.
#[derive(Clone, Debug, PartialEq)]
pub struct MixRecipe {
    gains: Vec<f32>,
}

impl MixRecipe {
    pub fn new(gains: Vec<f32>) -> Self {
        Self { gains }
    }

    /// The mix the player plays: each stem at its effective gain (Mute,
    /// Solo and volume).
    pub fn as_heard(params: &SharedParams) -> Self {
        Self::new(
            (0..params.stems.len())
                .map(|i| params.effective_gain(i))
                .collect(),
        )
    }

    pub fn gains(&self) -> &[f32] {
        &self.gains
    }

    /// The stems with a gain above 0, in order.
    pub fn audible(&self) -> Vec<usize> {
        (0..self.gains.len())
            .filter(|&i| self.gains[i] > 0.0)
            .collect()
    }

    pub fn is_silent(&self) -> bool {
        self.gains.iter().all(|&g| g <= 0.0)
    }
}

/// Stereo audio that can be read one block at a time.
pub trait Pcm {
    fn frames(&self) -> usize;
    fn sample_rate(&self) -> u32;
    /// Fills `left` and `right` with the frames from `at`, and returns how
    /// many samples are at full scale (±1 or beyond).  Both slices have the
    /// same length, and `at` plus that length is at most `frames`.
    fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> usize;
}

/// A song's stems at the gains of a recipe, summed and clamped to ±1, as
/// the player sums them.
pub struct Mix {
    song: Arc<Song>,
    audible: Vec<usize>,
    volumes: Vec<f32>,
}

impl Mix {
    /// Panics if the recipe does not have one gain for each stem.
    pub fn new(song: Arc<Song>, recipe: &MixRecipe) -> Self {
        assert_eq!(
            recipe.gains().len(),
            song.stems.len(),
            "a mix needs one gain for each stem"
        );
        let audible = recipe.audible();
        let volumes = audible.iter().map(|&i| recipe.gains()[i]).collect();
        Self {
            song,
            audible,
            volumes,
        }
    }

    fn channel(&self, side: impl Fn(&Stem) -> &[f32], at: usize, out: &mut [f32]) {
        let stems: Vec<&[f32]> = self
            .audible
            .iter()
            .map(|&i| side(&self.song.stems[i]))
            .collect();
        out.copy_from_slice(&dsp::mix_range(&stems, &self.volumes, at, out.len()));
    }
}

impl Pcm for Mix {
    fn frames(&self) -> usize {
        self.song.frames
    }

    fn sample_rate(&self) -> u32 {
        self.song.sample_rate
    }

    fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> usize {
        self.channel(|s| &s.left, at, left);
        self.channel(|s| &s.right, at, right);
        at_full_scale(left) + at_full_scale(right)
    }
}

/// One stem as it is: not scaled and not clamped.  The stem is shared with
/// the song, not copied.
pub struct StemPcm {
    stem: Stem,
    sample_rate: u32,
}

impl StemPcm {
    pub fn new(stem: Stem, sample_rate: u32) -> Self {
        Self { stem, sample_rate }
    }
}

impl Pcm for StemPcm {
    fn frames(&self) -> usize {
        self.stem.left.len().min(self.stem.right.len())
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> usize {
        left.copy_from_slice(&self.stem.left[at..at + left.len()]);
        right.copy_from_slice(&self.stem.right[at..at + right.len()]);
        at_full_scale(left) + at_full_scale(right)
    }
}

fn at_full_scale(samples: &[f32]) -> usize {
    samples.iter().filter(|s| s.abs() >= 1.0).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stem(name: &str, left: Vec<f32>, right: Vec<f32>) -> Stem {
        Stem {
            name: name.into(),
            left: left.into(),
            right: right.into(),
        }
    }

    fn wave(frames: usize, hz: f32, amp: f32) -> Vec<f32> {
        (0..frames)
            .map(|i| amp * (hz * i as f32 / 1000.0).sin())
            .collect()
    }

    fn song(frames: usize) -> Arc<Song> {
        Arc::new(Song {
            sample_rate: 44_100,
            frames,
            stems: vec![
                stem("bass", wave(frames, 3.0, 0.5), wave(frames, 5.0, 0.4)),
                stem("drums", wave(frames, 7.0, 0.6), wave(frames, 11.0, 0.3)),
                stem("vocals", wave(frames, 13.0, 0.7), wave(frames, 17.0, 0.6)),
            ],
        })
    }

    /// Reads the whole of `pcm` in blocks of `block` frames.
    fn read_all(pcm: &dyn Pcm, block: usize) -> (Vec<f32>, Vec<f32>, usize) {
        let (mut l, mut r) = (vec![0.0; pcm.frames()], vec![0.0; pcm.frames()]);
        let mut clipped = 0;
        let mut at = 0;
        while at < pcm.frames() {
            let n = block.min(pcm.frames() - at);
            clipped += pcm.read(at, &mut l[at..at + n], &mut r[at..at + n]);
            at += n;
        }
        (l, r, clipped)
    }

    #[test]
    fn a_mix_read_in_blocks_equals_dsp_mix() {
        let song = song(40_000);
        let gains = vec![0.75, 1.25, 0.0];
        let mix = Mix::new(Arc::clone(&song), &MixRecipe::new(gains.clone()));
        let want = |side: fn(&Stem) -> &[f32]| {
            let stems: Vec<&[f32]> = song.stems.iter().map(side).collect();
            dsp::mix(&stems, &gains)
        };
        let (want_l, want_r) = (want(|s| &s.left), want(|s| &s.right));
        for block in [1, 7, 16_384, 40_000, 50_000] {
            let (l, r, _) = read_all(&mix, block);
            assert_eq!(l, want_l, "left, block {block}");
            assert_eq!(r, want_r, "right, block {block}");
        }
    }

    #[test]
    fn a_mix_counts_samples_at_full_scale() {
        let song = Arc::new(Song {
            sample_rate: 44_100,
            frames: 3,
            stems: vec![
                stem("a", vec![0.9, -0.9, 0.5], vec![0.0, 0.0, 0.0]),
                stem("b", vec![0.9, -0.9, 0.25], vec![0.25, 0.0, 0.0]),
            ],
        });
        let mix = Mix::new(song, &MixRecipe::new(vec![1.0, 1.0]));
        let (l, r, clipped) = read_all(&mix, 2);
        assert_eq!(l, [1.0, -1.0, 0.75]);
        assert_eq!(r, [0.25, 0.0, 0.0]);
        assert_eq!(clipped, 2);
    }

    #[test]
    fn a_silent_recipe_reads_silence() {
        let mix = Mix::new(song(100), &MixRecipe::new(vec![0.0; 3]));
        let (l, r, clipped) = read_all(&mix, 64);
        assert!(l.iter().chain(&r).all(|&s| s == 0.0));
        assert_eq!(clipped, 0);
    }

    #[test]
    #[should_panic(expected = "one gain for each stem")]
    fn a_recipe_for_other_stems_panics() {
        Mix::new(song(10), &MixRecipe::new(vec![1.0]));
    }

    #[test]
    fn as_heard_follows_mute_solo_and_volume() {
        let p = SharedParams::new(4);
        p.set_volume(0, 0.5);
        p.set_muted(1, true);
        assert_eq!(MixRecipe::as_heard(&p).gains(), [0.5, 0.0, 1.0, 1.0]);
        p.set_soloed(2, true);
        let soloed = MixRecipe::as_heard(&p);
        assert_eq!(soloed.gains(), [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(soloed.audible(), [2]);
        assert!(!soloed.is_silent());
        p.set_muted(2, true);
        assert!(MixRecipe::as_heard(&p).is_silent());
    }

    #[test]
    fn a_stem_pcm_is_the_stem_unclamped() {
        let s = stem("bass", vec![1.5, -0.25, 0.5], vec![-2.0, 0.125, 1.0]);
        let pcm = StemPcm::new(s.clone(), 48_000);
        assert_eq!(pcm.frames(), 3);
        assert_eq!(pcm.sample_rate(), 48_000);
        let (l, r, clipped) = read_all(&pcm, 2);
        assert_eq!(l, *s.left);
        assert_eq!(r, *s.right);
        assert_eq!(clipped, 3);
    }

    #[test]
    fn a_stem_pcm_shares_the_stem() {
        let s = stem("bass", vec![0.5; 8], vec![0.25; 8]);
        let pcm = StemPcm::new(s.clone(), 44_100);
        assert!(pcm.stem.left.shares(&s.left));
        assert!(pcm.stem.right.shares(&s.right));
    }
}
