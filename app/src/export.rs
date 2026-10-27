//! Saving a stem or the mix as heard, one block at a time (decision 0014).
//!
//! An [`ExportJob`] reads a [`Pcm`] source in blocks and writes each block
//! to a [`Sink`].  The page steps a job for a few milliseconds in each UI
//! frame, since the main thread also feeds the Web Audio output.  The sinks
//! on the web write into JS arrays, so no song-sized buffer is made in
//! WebAssembly memory (MEM-1, MEM-4).

use crate::DERIVED;
use engine::mixdown::{MixRecipe, Pcm};
use engine::overview::COLUMNS;

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
    /// The last step found the source not ready.
    waiting: bool,
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
            waiting: false,
            left: vec![0.0; block],
            right: vec![0.0; block],
        }
    }

    /// Reads and writes the next block.  True when the source is done.
    /// When the source is not ready, it does nothing and waits.
    pub fn step(&mut self) -> bool {
        let n = self.left.len().min(self.source.frames() - self.at);
        self.waiting = n > 0 && !self.source.ready(self.at, n);
        if n > 0 && !self.waiting {
            let (left, right) = (&mut self.left[..n], &mut self.right[..n]);
            self.clipped += self.source.read(self.at, left, right);
            self.sink.write(self.at, left, right);
            self.at += n;
        }
        self.is_done()
    }

    /// Steps while `more` says there is time left, and at least once.  True
    /// when the source is done.  It returns at once while the source is not
    /// ready, so a job that waits for storage costs nothing in a frame.
    pub fn run_while(&mut self, mut more: impl FnMut() -> bool) -> bool {
        while !self.step() {
            if self.waiting || !more() {
                return false;
            }
        }
        true
    }

    /// Whether the last step found the source not ready.  A source that
    /// reads storage asks for a repaint when its read ends.
    pub fn is_waiting(&self) -> bool {
        self.waiting
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

    /// Why a read of the source's storage failed, if one did.
    pub fn failed(&self) -> Option<String> {
        self.source.failed()
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
/// in each column, as [`engine::overview::lane_columns`] gives them.
pub struct PeaksSink {
    column: usize,
    columns: Vec<(f32, f32)>,
}

/// What a [`PeaksSink`] gives at the end.
#[derive(Debug, PartialEq)]
pub struct Waveform {
    pub columns: Vec<(f32, f32)>,
    /// Makes the loudest column fill the lane.
    pub scale: f32,
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

    pub fn finish(self) -> Waveform {
        let scale = engine::overview::overview_scale(&self.columns);
        Waveform {
            columns: self.columns,
            scale,
        }
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

/// Keeps the mix lane's waveform on the mix as heard.  A new mix starts a
/// waveform job, which a caller steps for a few ms in each frame.  The job
/// holds only its block and the columns (D6 of the M7 plan); a spectrogram
/// of the mix reads its own windows (`spectrogram::SpectrogramJob`).
pub struct MixFollower {
    shown: Option<MixRecipe>,
    job: Option<(MixRecipe, ExportJob<PeaksSink>)>,
}

impl MixFollower {
    /// For a mix lane that shows `shown`.
    pub fn new(shown: MixRecipe) -> Self {
        Self {
            shown: Some(shown),
            job: None,
        }
    }

    /// For a mix lane that shows no mix yet: its first `follow` makes the
    /// lane (D4 of the M7 plan).
    pub fn unshown() -> Self {
        Self {
            shown: None,
            job: None,
        }
    }

    /// The mix that the lane shows, if it shows one.
    pub fn shown(&self) -> Option<&MixRecipe> {
        self.shown.as_ref()
    }

    /// The part done of the waveform job, if one runs.
    pub fn progress(&self) -> Option<f32> {
        self.job.as_ref().map(|(_, job)| job.fraction())
    }

    /// Moves the lane towards `heard`, while `more` says there is time.
    /// It continues a job for `heard`, and starts again when `heard`
    /// changes; `source` gives the audio of a mix, for a new job.  It gives
    /// the new waveform when it is done.
    pub fn follow(
        &mut self,
        heard: &MixRecipe,
        source: impl FnOnce(&MixRecipe) -> Box<dyn Pcm>,
        more: impl FnMut() -> bool,
    ) -> Option<Waveform> {
        if self.shown.as_ref() == Some(heard) {
            self.job = None;
            return None;
        }
        // Drop a job for another mix before the new one is made, so that
        // two jobs never read at once.
        if self.job.as_ref().is_some_and(|(recipe, _)| recipe != heard) {
            self.job = None;
        }
        let (recipe, mut job) = match self.job.take() {
            Some((recipe, job)) if recipe == *heard => (recipe, job),
            _ => {
                let source = source(heard);
                let sink = PeaksSink::new(source.frames(), COLUMNS);
                (heard.clone(), ExportJob::new(source, sink))
            }
        };
        if !job.run_while(more) {
            self.job = Some((recipe, job));
            return None;
        }
        self.shown = Some(recipe);
        Some(job.into_sink().finish())
    }

    /// Whether the waveform job waits for its source.
    pub fn is_waiting(&self) -> bool {
        self.job.as_ref().is_some_and(|(_, job)| job.is_waiting())
    }
}

/// Notes for the status line, such as that a mix clips, one for each file
/// that the worker encodes.  The worker can hold more than one file, also
/// two of the same name, and replies in the order of the requests.  So the
/// notes of a name wait in a queue, and each reply takes the oldest.
#[derive(Debug, Default)]
pub struct SaveNotes(Vec<(String, Option<String>)>);

impl SaveNotes {
    /// Adds the note of a file sent to the worker, or no note.
    pub fn push(&mut self, name: &str, note: Option<String>) {
        self.0.push((name.into(), note));
    }

    /// Takes the oldest note of `name`, or gives "".
    pub fn take(&mut self, name: &str) -> String {
        match self.0.iter().position(|(n, _)| n == name) {
            Some(i) => self.0.remove(i).1.unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Drops every note: after an encode fails, its reply never names it.
    pub fn clear(&mut self) {
        self.0.clear();
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

/// A file format that a stem or a mix saves as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Wav,
    Flac,
    Mp3,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Wav, Format::Flac, Format::Mp3];

    pub fn extension(self) -> &'static str {
        match self {
            Format::Wav => "wav",
            Format::Flac => "flac",
            Format::Mp3 => "mp3",
        }
    }

    /// The item of the Save menu.
    pub fn label(self) -> &'static str {
        match self {
            Format::Wav => "WAV (16-bit, lossless)",
            Format::Flac => "FLAC (16-bit, lossless)",
            Format::Mp3 => "MP3 (192 kbps)",
        }
    }

    /// Whether the worker encodes it.  The page makes a WAV itself.
    pub fn needs_worker(self) -> bool {
        self != Format::Wav
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{JsChannels, JsSink, JsWav};

/// Sinks that write into JS arrays, outside WebAssembly memory.
#[cfg(target_arch = "wasm32")]
mod web {
    use super::{Format, Sink};
    use engine::wav::{FRAME_BYTES, HEADER_BYTES, header, write_pcm16};
    use js_sys::{Float32Array, Uint8Array};

    /// Both channels at the source's length, for the worker's encoders,
    /// which take them transferred (MEM-2).
    pub struct JsChannels {
        pub left: Float32Array,
        pub right: Float32Array,
    }

    impl JsChannels {
        pub fn new(frames: usize) -> Self {
            Self {
                left: Float32Array::new_with_length(frames as u32),
                right: Float32Array::new_with_length(frames as u32),
            }
        }
    }

    impl Sink for JsChannels {
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            let (start, end) = (at as u32, (at + left.len()) as u32);
            self.left.subarray(start, end).copy_from(left);
            self.right.subarray(start, end).copy_from(right);
        }
    }

    /// A whole 16-bit WAV file.  Each block is made in a small reused
    /// buffer, then copied into place.
    pub struct JsWav {
        bytes: Uint8Array,
        block: Vec<u8>,
    }

    impl JsWav {
        pub fn new(frames: usize, sample_rate: u32) -> Self {
            let bytes = Uint8Array::new_with_length((HEADER_BYTES + frames * FRAME_BYTES) as u32);
            bytes
                .subarray(0, HEADER_BYTES as u32)
                .copy_from(&header(frames, sample_rate));
            Self {
                bytes,
                block: Vec::new(),
            }
        }

        pub fn into_bytes(self) -> Uint8Array {
            self.bytes
        }
    }

    impl Sink for JsWav {
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            self.block.resize(left.len() * FRAME_BYTES, 0);
            write_pcm16(left, right, &mut self.block);
            let start = (HEADER_BYTES + at * FRAME_BYTES) as u32;
            self.bytes
                .subarray(start, start + self.block.len() as u32)
                .copy_from(&self.block);
        }
    }

    /// The sink of a format: the file itself for WAV, the channels for the
    /// worker otherwise.
    pub enum JsSink {
        Wav(JsWav),
        Flac(JsChannels),
        Mp3(JsChannels),
    }

    impl JsSink {
        pub fn new(format: Format, frames: usize, sample_rate: u32) -> Self {
            match format {
                Format::Wav => JsSink::Wav(JsWav::new(frames, sample_rate)),
                Format::Flac => JsSink::Flac(JsChannels::new(frames)),
                Format::Mp3 => JsSink::Mp3(JsChannels::new(frames)),
            }
        }
    }

    impl Sink for JsSink {
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            match self {
                JsSink::Wav(s) => s.write(at, left, right),
                JsSink::Flac(s) | JsSink::Mp3(s) => s.write(at, left, right),
            }
        }
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

fn contents(names: &[&str], recipe: &MixRecipe) -> Contents {
    let audible: Vec<&str> = recipe.audible().into_iter().map(|i| names[i]).collect();
    let derived = |name: &&str| DERIVED.contains(name);
    let missing: Vec<&str> = names
        .iter()
        .copied()
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

/// The subtitle of the mix lane: what it plays, of the lanes `names`.
pub fn mix_label(names: &[&str], recipe: &MixRecipe) -> String {
    match contents(names, recipe) {
        Contents::Nothing => "as heard: nothing".into(),
        Contents::All => "as heard: all stems".into(),
        Contents::Without(names) => format!("as heard: all stems except {}", names.join(", ")),
        Contents::Of(names) => format!("as heard: {}", names.join(", ")),
    }
}

/// The name of a saved mix of the lanes `names`, without its extension.
pub fn mix_file_name(base: &str, names: &[&str], recipe: &MixRecipe) -> String {
    match contents(names, recipe) {
        Contents::Nothing | Contents::All => format!("{base} - mix"),
        Contents::Without(names) => format!("{base} - mix without {}", names.join(", ")),
        Contents::Of(names) => format!("{base} - mix of {}", names.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::mixdown::{Mix, StemPcm};
    use engine::overview::lane_columns;
    use engine::{Song, Stem};
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
        assert!(
            (job.fraction() - (2 * BLOCK) as f32 / frames as f32).abs() < 1e-6,
            "run_while steps once before it asks for time"
        );
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
    fn a_job_adds_up_the_clipped_samples_of_every_block() {
        let frames = 2 * BLOCK + 3;
        let mut left = vec![0.0; frames];
        let mut right = vec![0.0; frames];
        left[0] = 1.5;
        right[BLOCK] = -1.0;
        right[2 * BLOCK + 2] = 2.0;
        let stem = Stem {
            name: "bass".into(),
            left: left.into(),
            right: right.into(),
        };
        let mut job = ExportJob::new(Box::new(StemPcm::new(stem, 44_100)), Recorder::default());
        assert!(job.run_while(|| true));
        assert_eq!(job.clipped(), 3);
    }

    /// A source whose frames are ready only when the test says so.  It
    /// counts how often a job asks.
    struct Held {
        inner: Mix,
        ready: std::cell::Cell<bool>,
        asked: std::cell::Cell<usize>,
    }

    impl Pcm for Held {
        fn frames(&self) -> usize {
            self.inner.frames()
        }
        fn sample_rate(&self) -> u32 {
            self.inner.sample_rate()
        }
        fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> usize {
            assert!(self.ready.get(), "a job reads only frames that are ready");
            self.inner.read(at, left, right)
        }
        fn ready(&self, _: usize, _: usize) -> bool {
            self.asked.set(self.asked.get() + 1);
            self.ready.get()
        }
    }

    #[test]
    fn a_job_waits_for_a_source_that_is_not_ready() {
        let frames = 2 * BLOCK + 5;
        let song = Arc::new(song(&["bass", "drums"], frames));
        let recipe = MixRecipe::new(vec![1.0, 0.5]);
        let held = std::rc::Rc::new(Held {
            inner: Mix::new(Arc::clone(&song), &recipe),
            ready: std::cell::Cell::new(false),
            asked: std::cell::Cell::new(0),
        });
        struct Shared(std::rc::Rc<Held>);
        impl Pcm for Shared {
            fn frames(&self) -> usize {
                self.0.frames()
            }
            fn sample_rate(&self) -> u32 {
                self.0.sample_rate()
            }
            fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> usize {
                self.0.read(at, left, right)
            }
            fn ready(&self, at: usize, frames: usize) -> bool {
                self.0.ready(at, frames)
            }
        }
        let mut job = ExportJob::new(Box::new(Shared(held.clone())), Recorder::default());
        let mut asked_for_time = 0;
        let more = || {
            asked_for_time += 1;
            true
        };
        assert!(!job.run_while(more), "a job that waits is not done");
        assert_eq!(
            held.asked.get(),
            1,
            "it asks once and returns: no busy loop"
        );
        assert_eq!(asked_for_time, 0, "it does not ask for time while it waits");
        assert!(job.is_waiting());
        assert_eq!(job.fraction(), 0.0, "no progress");
        assert!(!job.step(), "a step waits too");
        assert!(job.sink().starts.is_empty());

        held.ready.set(true);
        assert!(job.run_while(|| true));
        assert!(!job.is_waiting());
        let want = run_mix(&song, &recipe);
        let rec = job.into_sink();
        assert_eq!(rec.starts, [0, BLOCK, 2 * BLOCK]);
        assert_eq!((rec.left, rec.right), (want.left, want.right));
    }

    #[test]
    fn only_wav_is_made_without_the_worker() {
        assert!(!Format::Wav.needs_worker());
        assert!(Format::Flac.needs_worker());
        assert!(Format::Mp3.needs_worker());
        let extensions: Vec<&str> = Format::ALL.iter().map(|f| f.extension()).collect();
        assert_eq!(extensions, ["wav", "flac", "mp3"]);
    }

    /// The mixes of `song`, for a follower's new jobs.
    fn mix_of(song: &Arc<Song>) -> impl FnOnce(&MixRecipe) -> Box<dyn Pcm> + '_ {
        |recipe| Box::new(Mix::new(Arc::clone(song), recipe))
    }

    #[test]
    fn a_mix_follower_continues_its_job_and_starts_again_when_the_mix_changes() {
        let frames = 3 * BLOCK;
        let song = Arc::new(song(&["bass", "drums", "vocals"], frames));
        let all = MixRecipe::new(vec![1.0, 1.0, 1.0]);
        let no_vocals = MixRecipe::new(vec![1.0, 1.0, 0.0]);
        let no_drums = MixRecipe::new(vec![1.0, 0.0, 1.0]);
        let mut follower = MixFollower::new(all.clone());
        assert_eq!(follower.follow(&all, mix_of(&song), || true), None);
        assert_eq!(follower.progress(), None, "the shown mix needs no job");

        let one = 1.0 / 3.0;
        assert_eq!(follower.follow(&no_vocals, mix_of(&song), || false), None);
        assert_eq!(follower.progress(), Some(one));
        assert_eq!(follower.follow(&no_vocals, mix_of(&song), || false), None);
        assert_eq!(
            follower.progress(),
            Some(2.0 * one),
            "the same mix continues"
        );
        assert_eq!(follower.follow(&no_drums, mix_of(&song), || false), None);
        assert_eq!(follower.progress(), Some(one), "a new mix starts again");

        let done = follower
            .follow(&no_drums, mix_of(&song), || true)
            .expect("the waveform of the new mix");
        assert_eq!(follower.shown(), Some(&no_drums));
        assert_eq!(follower.progress(), None);
        let want = run_mix(&song, &no_drums);
        assert_eq!(
            (done.columns, done.scale),
            lane_columns(&want.left, &want.right)
        );

        let back = follower
            .follow(&all, mix_of(&song), || true)
            .expect("the waveform of every stem");
        let want = run_mix(&song, &all);
        assert_eq!(
            (back.columns, back.scale),
            lane_columns(&want.left, &want.right)
        );
        assert_eq!(follower.follow(&all, mix_of(&song), || true), None);
    }

    #[test]
    fn a_mix_follower_drops_its_job_when_the_mix_goes_back() {
        let song = Arc::new(song(&["bass", "drums", "vocals"], 3 * BLOCK));
        let all = MixRecipe::new(vec![1.0, 1.0, 1.0]);
        let no_vocals = MixRecipe::new(vec![1.0, 1.0, 0.0]);
        let mut follower = MixFollower::new(all.clone());
        assert_eq!(follower.follow(&no_vocals, mix_of(&song), || false), None);
        assert!(follower.progress().is_some());
        assert_eq!(follower.follow(&all, mix_of(&song), || false), None);
        assert_eq!(
            follower.progress(),
            None,
            "back to the shown mix drops the job"
        );
    }

    fn run_mix(song: &Arc<Song>, recipe: &MixRecipe) -> Recorder {
        let mut job = ExportJob::new(
            Box::new(Mix::new(Arc::clone(song), recipe)),
            Recorder::default(),
        );
        assert!(job.run_while(|| true));
        job.into_sink()
    }

    #[test]
    fn save_notes_wait_for_their_own_file() {
        let mut notes = SaveNotes::default();
        notes.push("lloyd - bass.flac", None);
        notes.push("lloyd - mix.mp3", Some("  it clips.".into()));
        assert_eq!(notes.take("lloyd - mix.mp3"), "  it clips.");
        assert_eq!(notes.take("lloyd - bass.flac"), "");
        assert_eq!(notes.take("lloyd - mix.mp3"), "", "a note is taken once");

        notes.push("lloyd - mix.flac", Some("  it clips.".into()));
        notes.push("lloyd - mix.flac", None);
        assert_eq!(
            notes.take("lloyd - mix.flac"),
            "  it clips.",
            "the first reply of a name gets the first save's note"
        );
        assert_eq!(notes.take("lloyd - mix.flac"), "");

        notes.push("lloyd - mix.flac", Some("  it clips.".into()));
        notes.clear();
        assert_eq!(notes.take("lloyd - mix.flac"), "");
    }

    #[test]
    fn a_peak_of_exactly_the_threshold_is_silence_as_in_lane_columns() {
        let (left, right) = ([1e-6_f32, 0.0], [0.0_f32, 0.0]);
        let mut sink = PeaksSink::new(2, COLUMNS);
        sink.write(0, &left, &right);
        let peaks = sink.finish();
        assert_eq!(peaks.scale, 1.0);
        assert_eq!((peaks.columns, peaks.scale), lane_columns(&left, &right));
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
            let want = lane_columns(&whole.left, &whole.right);
            let peaks = run(&song, &recipe, PeaksSink::new(frames, COLUMNS)).finish();
            assert_eq!((peaks.columns, peaks.scale), want, "{frames} frames");
        }
    }

    #[test]
    fn mix_labels_and_names() {
        let s = song(&["bass", "drums", "other", "vocals", "original"], 1);
        let case = |gains: [f32; 5]| {
            let r = MixRecipe::new(gains.into());
            let names: Vec<&str> = s.stems.iter().map(|s| s.name.as_str()).collect();
            (
                mix_label(&names, &r),
                mix_file_name(&s.stems[0].name, &names, &r),
            )
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
        assert_eq!(
            mix_label(&names, &recipe),
            "as heard: all stems except lead"
        );
    }
}

/// Browser tests of the JS sinks (`cargo run -p xtask -- web-test`).
#[cfg(all(test, target_arch = "wasm32"))]
mod web_tests {
    use super::*;
    use engine::mixdown::Mix;
    use engine::{Song, Stem};
    use std::sync::Arc;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    fn song(frames: usize) -> Arc<Song> {
        let channel = |k: usize| -> Vec<f32> {
            (0..frames)
                .map(|i| ((i * k) % 101) as f32 / 101.0 - 0.5)
                .collect()
        };
        Arc::new(Song {
            sample_rate: 44_100,
            frames,
            stems: ["bass", "drums", "vocals"]
                .iter()
                .enumerate()
                .map(|(k, name)| Stem {
                    name: (*name).into(),
                    left: channel(k + 2).into(),
                    right: channel(k + 7).into(),
                })
                .collect(),
        })
    }

    const GAINS: [f32; 3] = [1.0, 1.5, 0.0];

    fn run<S: Sink>(song: &Arc<Song>, sink: S) -> S {
        let recipe = MixRecipe::new(GAINS.into());
        let mut job = ExportJob::new(Box::new(Mix::new(Arc::clone(song), &recipe)), sink);
        assert!(job.run_while(|| true));
        job.into_sink()
    }

    fn mixed(song: &Song, side: fn(&Stem) -> &[f32]) -> Vec<f32> {
        let stems: Vec<&[f32]> = song.stems.iter().map(side).collect();
        dsp::mix(&stems, &GAINS)
    }

    #[wasm_bindgen_test]
    fn js_channels_hold_the_mix() {
        let song = song(2 * BLOCK + 3);
        let sink = run(&song, JsChannels::new(song.frames));
        let mut left = vec![0.0; song.frames];
        let mut right = vec![0.0; song.frames];
        sink.left.copy_to(&mut left);
        sink.right.copy_to(&mut right);
        assert_eq!(left, mixed(&song, |s| &s.left));
        assert_eq!(right, mixed(&song, |s| &s.right));
    }

    #[wasm_bindgen_test]
    fn js_wav_is_the_wav_of_the_mix() {
        let song = song(BLOCK + 11);
        let bytes = run(&song, JsWav::new(song.frames, 48_000)).into_bytes();
        let mut got = vec![0; bytes.length() as usize];
        bytes.copy_to(&mut got);
        let want = engine::wav::encode_pcm16(
            &mixed(&song, |s| &s.left),
            &mixed(&song, |s| &s.right),
            48_000,
        );
        assert_eq!(got, want);
    }

    #[wasm_bindgen_test]
    fn a_mix_export_does_not_grow_wasm_memory() {
        // 60 s: 10.6 MB for each channel of the mix.
        let song = song(60 * 44_100);
        for format in Format::ALL {
            let before = crate::separator::page_wasm_mb();
            let sink = run(&song, JsSink::new(format, song.frames, 44_100));
            let grew = crate::separator::page_wasm_mb() - before;
            let bytes = match sink {
                JsSink::Wav(wav) => wav.into_bytes().length() as usize,
                JsSink::Flac(c) | JsSink::Mp3(c) => 4 * c.left.length() as usize,
            };
            assert!(
                bytes >= 4 * song.frames,
                "{format:?}: the sink holds the song"
            );
            assert!(
                grew < 1.0,
                "MEM-1: the page's WebAssembly memory grew by {grew:.1} MB during a {format:?} export"
            );
        }
    }
}
