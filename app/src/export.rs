//! Saving a stem or the mix as heard, one block at a time (decision 0014).
//!
//! An [`ExportJob`] reads a [`Pcm`] source in blocks and writes each block
//! to a [`Sink`].  The page steps a job for a few milliseconds in each UI
//! frame, since the main thread also feeds the Web Audio output.  The sinks
//! on the web write into JS arrays, so no song-sized buffer is made in
//! WebAssembly memory (MEM-1, MEM-4).

use crate::DERIVED;
use engine::Song;
use engine::mixdown::{MixRecipe, Pcm};

/// Frames that a job reads at a time.
pub const BLOCK: usize = 16_384;

/// Where a job writes its blocks.
pub trait Sink {
    /// Takes the frames from `at`; both channels have the same length.
    fn write(&mut self, at: usize, left: &[f32], right: &[f32]);
}

/// Reads a source in blocks and writes them, in order, to a sink.
pub struct ExportJob<S: Sink> {
    source: Box<dyn Pcm>,
    sink: S,
    at: usize,
    clipped: usize,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl<S: Sink> ExportJob<S> {
    pub fn new(source: Box<dyn Pcm>, sink: S) -> Self {
        let block = BLOCK.min(source.frames());
        Self {
            source,
            sink,
            at: 0,
            clipped: 0,
            left: vec![0.0; block],
            right: vec![0.0; block],
        }
    }

    /// Reads and writes the next block.  True when the source is done.
    pub fn step(&mut self) -> bool {
        let n = self.left.len().min(self.source.frames() - self.at);
        if n > 0 {
            let (left, right) = (&mut self.left[..n], &mut self.right[..n]);
            self.clipped += self.source.read(self.at, left, right);
            self.sink.write(self.at, left, right);
            self.at += n;
        }
        self.is_done()
    }

    /// Steps while `more` says there is time left, and at least once.  True
    /// when the source is done.
    pub fn run_while(&mut self, mut more: impl FnMut() -> bool) -> bool {
        while !self.step() {
            if !more() {
                return false;
            }
        }
        true
    }

    pub fn is_done(&self) -> bool {
        self.at == self.source.frames()
    }

    /// The part of the source done, from 0 to 1.
    pub fn fraction(&self) -> f32 {
        match self.source.frames() {
            0 => 1.0,
            frames => self.at as f32 / frames as f32,
        }
    }

    /// Samples at full scale so far.
    pub fn clipped(&self) -> usize {
        self.clipped
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn into_sink(self) -> S {
        self.sink
    }
}

/// The waveform of a lane: the lowest and highest sample of both channels
/// in each column, as [`crate::lane_columns`] gives them.
pub struct PeaksSink {
    column: usize,
    columns: Vec<(f32, f32)>,
}

impl PeaksSink {
    /// For a source of `frames` frames, in `count` columns at most.
    pub fn new(frames: usize, count: usize) -> Self {
        let column = frames.div_ceil(count).max(1);
        Self {
            column,
            columns: vec![(f32::INFINITY, f32::NEG_INFINITY); frames.div_ceil(column)],
        }
    }

    /// The columns, and the scale that makes the loudest fill the lane.
    pub fn finish(self) -> (Vec<(f32, f32)>, f32) {
        let loudest = self
            .columns
            .iter()
            .map(|(lo, hi)| lo.abs().max(hi.abs()))
            .fold(0.0, f32::max);
        let scale = if loudest > 1e-6 { 1.0 / loudest } else { 1.0 };
        (self.columns, scale)
    }
}

impl Sink for PeaksSink {
    fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
        for (i, (l, r)) in left.iter().zip(right).enumerate() {
            let (lo, hi) = &mut self.columns[(at + i) / self.column];
            *lo = lo.min(*l).min(*r);
            *hi = hi.max(*l).max(*r);
        }
    }
}

/// A test for [`ExportJob::run_while`]: true until `ms` milliseconds have
/// passed since the call.  Natively it is always true, so a job runs to its
/// end in one frame: native audio has its own thread.
pub fn time_left(ms: f64) -> impl FnMut() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let clock = web_sys::window().and_then(|w| w.performance());
        let now = move || clock.as_ref().map_or(0.0, |c| c.now());
        let end = now() + ms;
        move || now() < end
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ms;
        || true
    }
}

/// What a mix holds, by the names of its stems.
#[derive(Debug, PartialEq)]
enum Contents {
    Nothing,
    /// Every source stem, and no derived lane.
    All,
    Without(Vec<String>),
    Of(Vec<String>),
}

fn contents(song: &Song, recipe: &MixRecipe) -> Contents {
    let audible: Vec<&str> = recipe
        .audible()
        .into_iter()
        .map(|i| song.stems[i].name.as_str())
        .collect();
    let derived = |name: &&str| DERIVED.contains(name);
    let missing: Vec<&str> = song
        .stems
        .iter()
        .map(|s| s.name.as_str())
        .filter(|name| !derived(name) && !audible.contains(name))
        .collect();
    let owned = |names: Vec<&str>| names.into_iter().map(String::from).collect();
    if audible.is_empty() {
        Contents::Nothing
    } else if audible.iter().any(derived) {
        Contents::Of(owned(audible))
    } else if missing.is_empty() {
        Contents::All
    } else if missing.len() < audible.len() {
        Contents::Without(owned(missing))
    } else {
        Contents::Of(owned(audible))
    }
}

/// The subtitle of the mix lane: what it plays.
pub fn mix_label(song: &Song, recipe: &MixRecipe) -> String {
    match contents(song, recipe) {
        Contents::Nothing => "as heard: nothing".into(),
        Contents::All => "as heard: all stems".into(),
        Contents::Without(names) => format!("as heard: all stems except {}", names.join(", ")),
        Contents::Of(names) => format!("as heard: {}", names.join(", ")),
    }
}

/// The name of a saved mix, without its extension.
pub fn mix_file_name(base: &str, song: &Song, recipe: &MixRecipe) -> String {
    match contents(song, recipe) {
        Contents::Nothing | Contents::All => format!("{base} - mix"),
        Contents::Without(names) => format!("{base} - mix without {}", names.join(", ")),
        Contents::Of(names) => format!("{base} - mix of {}", names.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Stem;
    use engine::mixdown::{Mix, StemPcm};
    use std::sync::Arc;

    fn song(names: &[&str], frames: usize) -> Song {
        Song {
            sample_rate: 44_100,
            frames,
            stems: names
                .iter()
                .enumerate()
                .map(|(k, name)| Stem {
                    name: (*name).into(),
                    left: (0..frames)
                        .map(|i| ((i * (k + 3)) % 17) as f32 / 17.0 - 0.5)
                        .collect::<Vec<_>>()
                        .into(),
                    right: (0..frames)
                        .map(|i| ((i * (k + 5)) % 13) as f32 / 13.0 - 0.5)
                        .collect::<Vec<_>>()
                        .into(),
                })
                .collect(),
        }
    }

    /// Records each block's start and copies its frames.
    #[derive(Default)]
    struct Recorder {
        starts: Vec<usize>,
        left: Vec<f32>,
        right: Vec<f32>,
    }

    impl Sink for Recorder {
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            assert_eq!(at, self.left.len(), "blocks arrive in order, once each");
            self.starts.push(at);
            self.left.extend_from_slice(left);
            self.right.extend_from_slice(right);
        }
    }

    #[test]
    fn a_job_in_steps_writes_the_whole_mix_once() {
        let frames = 2 * BLOCK + 5;
        let song = Arc::new(song(&["bass", "drums", "vocals"], frames));
        let gains = vec![1.0, 0.5, 0.0];
        let recipe = MixRecipe::new(gains.clone());
        let mut job = ExportJob::new(
            Box::new(Mix::new(Arc::clone(&song), &recipe)),
            Recorder::default(),
        );
        assert_eq!(job.fraction(), 0.0);
        assert!(!job.step());
        assert!((job.fraction() - BLOCK as f32 / frames as f32).abs() < 1e-6);
        assert!(!job.run_while(|| false), "one step when there is no time");
        assert!(job.run_while(|| true));
        assert!(job.step(), "a step after the end does nothing");
        assert_eq!(job.fraction(), 1.0);
        let rec = job.into_sink();
        assert_eq!(rec.starts, [0, BLOCK, 2 * BLOCK]);
        let side = |f: fn(&Stem) -> &[f32]| {
            let stems: Vec<&[f32]> = song.stems.iter().map(f).collect();
            dsp::mix(&stems, &gains)
        };
        assert_eq!(rec.left, side(|s| &s.left));
        assert_eq!(rec.right, side(|s| &s.right));
    }

    #[test]
    fn a_job_counts_clipped_samples() {
        let stem = Stem {
            name: "bass".into(),
            left: vec![1.5, 0.0, -1.0].into(),
            right: vec![0.5, 0.25, 0.0].into(),
        };
        let mut job = ExportJob::new(Box::new(StemPcm::new(stem, 44_100)), Recorder::default());
        assert!(job.run_while(|| true));
        assert_eq!(job.clipped(), 2);
        assert_eq!(job.sink().left, [1.5, 0.0, -1.0], "a stem is not clamped");
    }

    #[test]
    fn a_job_of_no_frames_is_done_at_once() {
        let song = Arc::new(song(&["bass"], 0));
        let mut job = ExportJob::new(
            Box::new(Mix::new(song, &MixRecipe::new(vec![1.0]))),
            Recorder::default(),
        );
        assert!(job.is_done());
        assert!(job.step());
        assert_eq!(job.fraction(), 1.0);
    }

    #[test]
    fn peaks_of_a_mix_match_lane_columns() {
        for frames in [0, 1, 4095, 4097, 3 * BLOCK + 17] {
            let song = Arc::new(song(&["bass", "drums"], frames));
            let recipe = MixRecipe::new(vec![0.75, 1.5]);
            fn run<S: Sink>(song: &Arc<Song>, recipe: &MixRecipe, sink: S) -> S {
                let mut job = ExportJob::new(Box::new(Mix::new(Arc::clone(song), recipe)), sink);
                assert!(job.run_while(|| true));
                job.into_sink()
            }
            let whole = run(&song, &recipe, Recorder::default());
            let peaks = run(&song, &recipe, PeaksSink::new(frames, crate::COLUMNS));
            assert_eq!(
                peaks.finish(),
                crate::lane_columns(&whole.left, &whole.right),
                "{frames} frames"
            );
        }
    }

    #[test]
    fn mix_labels_and_names() {
        let s = song(&["bass", "drums", "other", "vocals", "original"], 1);
        let case = |gains: [f32; 5]| {
            let r = MixRecipe::new(gains.into());
            (mix_label(&s, &r), mix_file_name(&s.stems[0].name, &s, &r))
        };
        assert_eq!(
            case([1.0, 0.5, 1.0, 1.0, 0.0]),
            ("as heard: all stems".into(), "bass - mix".into())
        );
        assert_eq!(
            case([1.0, 1.0, 1.0, 0.0, 0.0]),
            (
                "as heard: all stems except vocals".into(),
                "bass - mix without vocals".into()
            )
        );
        assert_eq!(
            case([1.0, 1.0, 0.0, 0.0, 0.0]),
            (
                "as heard: bass, drums".into(),
                "bass - mix of bass, drums".into()
            ),
            "a tie lists what is in the mix"
        );
        assert_eq!(
            case([0.0, 0.0, 0.0, 1.0, 0.0]),
            ("as heard: vocals".into(), "bass - mix of vocals".into())
        );
        assert_eq!(
            case([0.0; 5]),
            ("as heard: nothing".into(), "bass - mix".into())
        );
        assert_eq!(
            case([1.0, 1.0, 1.0, 1.0, 1.0]),
            (
                "as heard: bass, drums, other, vocals, original".into(),
                "bass - mix of bass, drums, other, vocals, original".into()
            ),
            "an audible derived lane is named"
        );
    }

    #[test]
    fn the_demo_mix_without_lead_is_the_sum_of_the_other_stems() {
        let demo = synth::generate();
        let names: Vec<&str> = demo.stems.iter().map(|(n, _)| n.as_str()).collect();
        let lead = names
            .iter()
            .position(|n| *n == "lead")
            .expect("a lead stem");
        let song = Arc::new(Song {
            sample_rate: synth::SAMPLE_RATE,
            frames: demo.truth.frames,
            stems: demo
                .stems
                .iter()
                .map(|(name, st)| Stem {
                    name: name.clone(),
                    left: st.left.clone().into(),
                    right: st.right.clone().into(),
                })
                .collect(),
        });
        let gains: Vec<f32> = (0..names.len())
            .map(|i| if i == lead { 0.0 } else { 1.0 })
            .collect();
        let recipe = MixRecipe::new(gains);
        let mut job = ExportJob::new(
            Box::new(Mix::new(Arc::clone(&song), &recipe)),
            Recorder::default(),
        );
        assert!(job.run_while(|| true));
        let others: Vec<&[f32]> = demo
            .stems
            .iter()
            .filter(|(n, _)| n != "lead")
            .map(|(_, st)| st.left.as_slice())
            .collect();
        assert_eq!(job.sink().left, dsp::mix(&others, &vec![1.0; others.len()]));
        assert_eq!(mix_label(&song, &recipe), "as heard: all stems except lead");
    }
}
