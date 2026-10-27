//! Lane audio: what a lane of the shown song plays and is drawn from, and
//! the sources that read it in blocks (D1 and D3 of the M7 plan).
//!
//! A lane's audio is samples in memory.  The jobs of the page (the mix
//! lane's waveform, a spectrogram, a save) read it through a [`Pcm`]:
//! [`LanesMix`] for a mix of lanes, [`LanePcm`] for one lane.

use engine::mixdown::{MixRecipe, Pcm, at_full_scale};
use engine::{Samples, Stem};
use std::borrow::Cow;

/// What a lane plays and is drawn from.
#[derive(Clone)]
pub enum LaneAudio {
    /// Samples in memory, shared with the song, not copied (MEM-1).
    Memory { left: Samples, right: Samples },
}

impl LaneAudio {
    /// The audio of a stem in memory.  It shares the stem's samples.
    pub fn of_stem(stem: &Stem) -> Self {
        LaneAudio::Memory {
            left: stem.left.clone(),
            right: stem.right.clone(),
        }
    }

    /// The frames from `at` of each side.  A lane in memory lends its
    /// samples.  Frames past the lane's end are silence.
    fn frames(&self, at: usize, frames: usize) -> (Cow<'_, [f32]>, Cow<'_, [f32]>) {
        match self {
            LaneAudio::Memory { left, right } => {
                (part_of(left, at, frames), part_of(right, at, frames))
            }
        }
    }
}

/// The `frames` samples of `samples` from `at`, with silence past its end.
fn part_of(samples: &[f32], at: usize, frames: usize) -> Cow<'_, [f32]> {
    if at + frames <= samples.len() {
        return Cow::Borrowed(&samples[at..at + frames]);
    }
    let have = &samples[at.min(samples.len())..];
    let mut part = Vec::with_capacity(frames);
    part.extend_from_slice(have);
    part.resize(frames, 0.0);
    Cow::Owned(part)
}

/// Lanes at the gains of a recipe, summed and clamped to ±1 with
/// `dsp::mix_range`, as the player sums them and as
/// `engine::mixdown::Mix` does.
pub struct LanesMix {
    frames: usize,
    sample_rate: u32,
    /// The lanes with a gain above 0, and their gains.
    audible: Vec<LaneAudio>,
    volumes: Vec<f32>,
}

impl LanesMix {
    /// The mix of `lanes`, of `frames` frames at `sample_rate`.  Panics if
    /// the recipe does not have one gain for each lane.
    pub fn new(lanes: &[LaneAudio], frames: usize, sample_rate: u32, recipe: &MixRecipe) -> Self {
        assert_eq!(
            recipe.gains().len(),
            lanes.len(),
            "a mix needs one gain for each lane"
        );
        let audible = recipe.audible();
        Self {
            frames,
            sample_rate,
            volumes: audible.iter().map(|&i| recipe.gains()[i]).collect(),
            audible: audible.into_iter().map(|i| lanes[i].clone()).collect(),
        }
    }

    /// Sums one side of a block: `sides` holds that side of each audible
    /// lane.
    fn channel(&self, sides: &[&[f32]], out: &mut [f32]) {
        out.copy_from_slice(&dsp::mix_range(sides, &self.volumes, 0, out.len()));
    }
}

impl Pcm for LanesMix {
    fn frames(&self) -> usize {
        self.frames
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> usize {
        let parts: Vec<_> = self
            .audible
            .iter()
            .map(|lane| lane.frames(at, left.len()))
            .collect();
        let lefts: Vec<&[f32]> = parts.iter().map(|(l, _)| l.as_ref()).collect();
        let rights: Vec<&[f32]> = parts.iter().map(|(_, r)| r.as_ref()).collect();
        self.channel(&lefts, left);
        self.channel(&rights, right);
        at_full_scale(left) + at_full_scale(right)
    }
}

/// One lane as it is: not scaled and not clamped, as
/// `engine::mixdown::StemPcm`.
pub struct LanePcm {
    lane: LaneAudio,
    frames: usize,
    sample_rate: u32,
}

impl LanePcm {
    /// The lane `lane`, of `frames` frames at `sample_rate`.
    pub fn new(lane: LaneAudio, frames: usize, sample_rate: u32) -> Self {
        Self {
            lane,
            frames,
            sample_rate,
        }
    }
}

impl Pcm for LanePcm {
    fn frames(&self) -> usize {
        self.frames
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> usize {
        let (l, r) = self.lane.frames(at, left.len());
        left.copy_from_slice(&l);
        right.copy_from_slice(&r);
        at_full_scale(left) + at_full_scale(right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Song;
    use engine::mixdown::{Mix, StemPcm};
    use std::sync::Arc;

    /// A song of `frames` frames whose stems go above full scale when they
    /// are summed, so the clamp counts.
    pub(crate) fn song(frames: usize) -> Arc<Song> {
        let wave = |k: usize| -> Vec<f32> {
            (0..frames)
                .map(|i| ((i * k) % 29) as f32 / 14.0 - 1.0)
                .collect()
        };
        Arc::new(Song {
            sample_rate: 48_000,
            frames,
            stems: ["bass", "drums", "vocals", "original"]
                .iter()
                .enumerate()
                .map(|(k, name)| Stem {
                    name: (*name).into(),
                    left: wave(k + 2).into(),
                    right: wave(k + 7).into(),
                })
                .collect(),
        })
    }

    pub(crate) fn lanes_of(song: &Song) -> Vec<LaneAudio> {
        song.stems.iter().map(LaneAudio::of_stem).collect()
    }

    /// The bits of `pcm` read in blocks of `block` frames, and the samples
    /// at full scale.
    fn bits_of(pcm: &dyn Pcm, block: usize) -> (Vec<u32>, Vec<u32>, usize) {
        let (mut l, mut r) = (vec![0.0; pcm.frames()], vec![0.0; pcm.frames()]);
        let mut clipped = 0;
        let mut at = 0;
        while at < pcm.frames() {
            let n = block.min(pcm.frames() - at);
            clipped += pcm.read(at, &mut l[at..at + n], &mut r[at..at + n]);
            at += n;
        }
        let bits = |x: &[f32]| x.iter().map(|v| v.to_bits()).collect();
        (bits(&l), bits(&r), clipped)
    }

    #[test]
    fn a_lanes_mix_gives_the_bytes_of_mix() {
        let song = song(70_001);
        let lanes = lanes_of(&song);
        for gains in [
            vec![1.0, 1.0, 1.0, 0.0],
            vec![0.75, 1.5, 0.0, 0.25],
            vec![0.0; 4],
        ] {
            let recipe = MixRecipe::new(gains);
            let want = Mix::new(Arc::clone(&song), &recipe);
            let got = LanesMix::new(&lanes, song.frames, song.sample_rate, &recipe);
            assert_eq!(got.frames(), want.frames());
            assert_eq!(got.sample_rate(), 48_000);
            for block in [1, 1_000, 16_384, 32_768, 70_001] {
                assert!(
                    bits_of(&got, block) == bits_of(&want, block),
                    "{recipe:?} in blocks of {block}"
                );
            }
        }
    }

    #[test]
    fn a_lane_pcm_gives_the_bytes_of_stem_pcm() {
        let song = song(70_001);
        for stem in &song.stems {
            let want = StemPcm::new(stem.clone(), song.sample_rate);
            let got = LanePcm::new(LaneAudio::of_stem(stem), song.frames, song.sample_rate);
            assert_eq!(got.frames(), want.frames());
            assert_eq!(got.sample_rate(), 48_000);
            for block in [1, 1_000, 16_384, 32_768, 70_001] {
                assert!(
                    bits_of(&got, block) == bits_of(&want, block),
                    "{} in blocks of {block}",
                    stem.name
                );
            }
        }
    }

    #[test]
    fn lane_audio_shares_the_samples_of_its_stem() {
        let song = song(10);
        let LaneAudio::Memory { left, right } = LaneAudio::of_stem(&song.stems[0]);
        assert!(left.shares(&song.stems[0].left));
        assert!(right.shares(&song.stems[0].right));
    }

    #[test]
    #[should_panic(expected = "one gain for each lane")]
    fn a_recipe_for_other_lanes_panics() {
        let song = song(10);
        LanesMix::new(&lanes_of(&song), 10, 48_000, &MixRecipe::new(vec![1.0]));
    }
}
