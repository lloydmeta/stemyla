//! Spectrogram textures for the lanes, and the classic (HPSS) drums job.
//!
//! A spectrogram reads only the windows that its columns show (D5 of the
//! M7 plan), through a [`Pcm`] of the lane's audio, so no lane makes a
//! song-length signal.

use crate::Lane;
use eframe::egui::{self, Color32, ColorImage, TextureOptions};
use engine::Song;
use engine::hpss::{BlockHpss, HpssConfig, SpectrogramColumns};
use engine::mixdown::Pcm;

const ROWS: usize = 128;
const MAX_COLS: usize = 1024;
const MAX_HZ: f32 = 8_000.0;

/// Dark blue → purple → orange → yellow, by dB between -90 and -10.
fn colour(db: f32) -> Color32 {
    let t = ((db + 90.0) / 80.0).clamp(0.0, 1.0);
    let stops = [
        (0.0, [8.0, 8.0, 24.0]),
        (0.35, [80.0, 20.0, 110.0]),
        (0.7, [230.0, 90.0, 50.0]),
        (1.0, [250.0, 240.0, 150.0]),
    ];
    for w in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if t <= t1 {
            let f = (t - t0) / (t1 - t0);
            let c = |i: usize| (c0[i] + (c1[i] - c0[i]) * f) as u8;
            return Color32::from_rgb(c(0), c(1), c(2));
        }
    }
    Color32::from_rgb(250, 240, 150)
}

/// Makes the spectrogram texture of each lane that lacks one, in lane
/// order, one [`SpectrogramJob`] at a time.  A job goes on in the next frame
/// when its time ends or its source waits.
#[derive(Default)]
pub struct Spectrograms {
    /// The job of the lane at this index of the lanes.
    job: Option<(usize, SpectrogramJob)>,
}

impl Spectrograms {
    /// Makes textures while `more` says there is time left.  `source` gives
    /// the audio of a lane, or `None` when it cannot be read yet; that lane
    /// waits, and the next lane goes first.
    pub fn step(
        &mut self,
        ctx: &egui::Context,
        lanes: &mut [Lane],
        source: impl Fn(&Lane) -> Option<Box<dyn Pcm>>,
        mut more: impl FnMut() -> bool,
    ) {
        loop {
            if self.job.is_none() {
                let next = lanes
                    .iter()
                    .enumerate()
                    .filter(|(_, lane)| lane.spectrogram.is_none())
                    .find_map(|(i, lane)| source(lane).map(|pcm| (i, pcm)));
                let Some((i, pcm)) = next else {
                    return;
                };
                self.job = Some((i, SpectrogramJob::new(pcm)));
            }
            let Some((_, job)) = &mut self.job else {
                return;
            };
            if !job.run_while(&mut more) {
                // A source that waits asks for a repaint when its read ends.
                if !job.is_waiting() {
                    ctx.request_repaint();
                }
                return;
            }
            if let Some((i, job)) = self.job.take()
                && let Some(lane) = lanes.get_mut(i)
            {
                lane.spectrogram = Some(texture(ctx, &lane.name, job.finish()));
            }
            if !more() {
                ctx.request_repaint();
                return;
            }
        }
    }

    /// Drops the job of the lane at `lane`, whose audio changed.
    pub fn forget(&mut self, lane: usize) {
        if self.job.as_ref().is_some_and(|(i, _)| *i == lane) {
            self.job = None;
        }
    }
}

/// The spectrogram texture of the lane `name`, from its image.
pub fn texture(
    ctx: &egui::Context,
    name: &str,
    (db, cols, rows): (Vec<f32>, usize, usize),
) -> egui::TextureHandle {
    let pixels: Vec<Color32> = db.iter().map(|v| colour(*v)).collect();
    let img = ColorImage::new([cols, rows], pixels);
    ctx.load_texture(format!("spec-{name}"), img, TextureOptions::LINEAR)
}

/// The most columns that a spectrogram job reads in one request.
const BATCH_COLUMNS: usize = 32;

/// Makes the spectrogram image of one lane (D5 of the M7 plan).  It reads
/// only the frames of its columns' windows, a batch of columns at a time,
/// and holds no song-length signal.
///
/// A batch is at most [`BATCH_COLUMNS`] columns.  Windows that overlap or
/// touch are read as one range.  So a batch reads at most
/// `engine::source::BLOCK_FRAMES` frames, in one request of its ranges
/// (`Pcm::ready_ranges`).
pub struct SpectrogramJob {
    source: Box<dyn Pcm>,
    columns: SpectrogramColumns,
    /// The next column to make.
    next: usize,
    /// The last batch found the source not ready.
    waiting: bool,
    /// The ranges of the batch, and where each starts in the buffers.
    ranges: Vec<(usize, usize)>,
    starts: Vec<usize>,
    left: Vec<f32>,
    right: Vec<f32>,
    mid: Vec<f32>,
}

impl SpectrogramJob {
    /// The image of `source`: its mid, the mean of its two channels.
    pub fn new(source: Box<dyn Pcm>) -> Self {
        let columns = SpectrogramColumns::new(
            source.frames(),
            source.sample_rate(),
            MAX_HZ,
            ROWS,
            MAX_COLS,
        );
        let buffer = (engine::source::BLOCK_FRAMES as usize).min(source.frames());
        Self {
            source,
            columns,
            next: 0,
            waiting: false,
            ranges: Vec::with_capacity(BATCH_COLUMNS),
            starts: Vec::with_capacity(BATCH_COLUMNS),
            left: vec![0.0; buffer],
            right: vec![0.0; buffer],
            mid: vec![0.0; buffer],
        }
    }

    /// Makes columns while `more` says there is time left, and at least one
    /// batch.  True when the image is done.  It returns at once while the
    /// source is not ready.
    pub fn run_while(&mut self, mut more: impl FnMut() -> bool) -> bool {
        while !self.step() {
            if self.waiting || !more() {
                return false;
            }
        }
        true
    }

    /// Whether the last batch found the source not ready.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// The image, its columns and its rows.
    pub fn finish(self) -> (Vec<f32>, usize, usize) {
        self.columns.finish()
    }

    fn is_done(&self) -> bool {
        self.next == self.columns.columns()
    }

    /// Makes the next batch of columns, if its frames are ready.  True when
    /// the image is done.
    fn step(&mut self) -> bool {
        if self.is_done() {
            return true;
        }
        let end = self.batch_end();
        self.waiting = !self.source.ready_ranges(&self.ranges);
        if self.waiting {
            return false;
        }
        for (&(at, frames), &start) in self.ranges.iter().zip(&self.starts) {
            let (left, right) = (
                &mut self.left[start..start + frames],
                &mut self.right[start..start + frames],
            );
            self.source.read(at, left, right);
            for i in start..start + frames {
                self.mid[i] = 0.5 * (self.left[i] + self.right[i]);
            }
        }
        for c in self.next..end {
            let window = self.columns.window(c);
            let samples = if window.is_empty() {
                &[][..]
            } else {
                // The range that holds the window: the last that starts at
                // or before it.
                let k = self.ranges.partition_point(|&(at, _)| at <= window.start) - 1;
                let offset = self.starts[k] + window.start - self.ranges[k].0;
                &self.mid[offset..offset + window.len()]
            };
            self.columns.put(c, samples);
        }
        self.next = end;
        self.is_done()
    }

    /// Fills `ranges` and `starts` with the windows of the columns from
    /// `next`, and gives the end of the batch.
    fn batch_end(&mut self) -> usize {
        self.ranges.clear();
        self.starts.clear();
        let mut held = 0;
        let mut c = self.next;
        while c < self.columns.columns() && c - self.next < BATCH_COLUMNS {
            let window = self.columns.window(c);
            match self.ranges.last_mut() {
                _ if window.is_empty() => {}
                Some((at, frames)) if *at + *frames >= window.start => {
                    let grown = window.end - (*at + *frames);
                    if held + grown > self.left.len() {
                        break;
                    }
                    *frames += grown;
                    held += grown;
                }
                _ => {
                    if held + window.len() > self.left.len() {
                        break;
                    }
                    self.ranges.push((window.start, window.len()));
                    self.starts.push(held);
                    held += window.len();
                }
            }
            c += 1;
        }
        c
    }
}

/// HPSS on the song's mix (both channels): returns the percussive part.
pub fn classic_drums(song: &Song) -> (Vec<f32>, Vec<f32>) {
    let (l, r) = crate::mix_of_sources(song);
    percussive(&l, &r)
}

/// The percussive part of a stereo signal in memory, with D8's block HPSS
/// (D8 of the M7 plan), which gives `hpss`'s output bit for bit.
pub fn percussive(left: &[f32], right: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut block = BlockHpss::new(left.len(), &HpssConfig::default());
    let (mut l, mut r) = (
        Vec::with_capacity(left.len()),
        Vec::with_capacity(left.len()),
    );
    let mut sink = |a: &[f32], b: &[f32]| -> Result<(), std::convert::Infallible> {
        l.extend_from_slice(a);
        r.extend_from_slice(b);
        Ok(())
    };
    let chunk = engine::source::BLOCK_FRAMES as usize;
    for (a, b) in left.chunks(chunk).zip(right.chunks(chunk)) {
        let Ok(()) = block.put(a, b, &mut sink);
    }
    let Ok(()) = block.finish(&mut sink);
    (l, r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lanes::{LaneAudio, LanePcm, LanesMix};
    use engine::Stem;
    use engine::hpss::spectrogram_db;
    use engine::mixdown::MixRecipe;

    fn mid(left: &[f32], right: &[f32]) -> Vec<f32> {
        left.iter().zip(right).map(|(a, b)| 0.5 * (a + b)).collect()
    }
    use std::cell::Cell;
    use std::rc::Rc;

    fn song(frames: usize) -> Song {
        let wave = |k: usize| -> Vec<f32> {
            (0..frames)
                .map(|i| ((i * k) % 23) as f32 / 23.0 - 0.5)
                .collect()
        };
        Song {
            sample_rate: 44_100,
            frames,
            stems: ["bass", "lead", "drums"]
                .iter()
                .enumerate()
                .map(|(k, name)| Stem {
                    name: (*name).into(),
                    left: wave(k + 2).into(),
                    right: wave(k + 5).into(),
                })
                .collect(),
        }
    }

    /// The mid of `pcm`, read whole.
    fn mid_of(pcm: &dyn Pcm) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; pcm.frames()], vec![0.0; pcm.frames()]);
        pcm.read(0, &mut l, &mut r);
        mid(&l, &r)
    }

    fn bits(image: &(Vec<f32>, usize, usize)) -> (Vec<u32>, usize, usize) {
        (
            image.0.iter().map(|v| v.to_bits()).collect(),
            image.1,
            image.2,
        )
    }

    fn whole_image(source: Box<dyn Pcm>) -> (Vec<f32>, usize, usize) {
        let mut job = SpectrogramJob::new(source);
        assert!(job.run_while(|| true), "a job in memory is done at once");
        job.finish()
    }

    /// D5 of the M7 plan: a spectrogram job reads the windows of its
    /// columns, and gives the image of `spectrogram_db` of the whole mid,
    /// bit for bit.  The lengths give a `hop` below, at and over `n_fft`.
    #[test]
    fn a_spectrogram_job_gives_the_whole_image() {
        for frames in [1_000, 300_000, 1_048_576, 2_000_000] {
            let song = song(frames);
            let lanes: Vec<LaneAudio> = song.stems.iter().map(LaneAudio::of_stem).collect();
            let lane = LanePcm::new(lanes[1].clone(), frames, song.sample_rate);
            let want = spectrogram_db(&mid_of(&lane), 44_100, MAX_HZ, ROWS, MAX_COLS);
            assert!(
                bits(&whole_image(Box::new(lane))) == bits(&want),
                "a lane of {frames} frames"
            );
            let no_lead = MixRecipe::new(vec![1.0, 0.0, 0.75]);
            let heard = || LanesMix::new(&lanes, frames, song.sample_rate, &no_lead);
            let want = spectrogram_db(&mid_of(&heard()), 44_100, MAX_HZ, ROWS, MAX_COLS);
            assert!(
                bits(&whole_image(Box::new(heard()))) == bits(&want),
                "the mix as heard of {frames} frames"
            );
        }
    }

    /// A source whose reads are ready only when the test says so.  It
    /// counts the requests and the frames read.
    struct Held {
        inner: LanePcm,
        ready: Cell<bool>,
        requests: Cell<usize>,
        largest_read: Cell<usize>,
    }

    struct Shared(Rc<Held>);

    impl Pcm for Shared {
        fn frames(&self) -> usize {
            self.0.inner.frames()
        }
        fn sample_rate(&self) -> u32 {
            self.0.inner.sample_rate()
        }
        fn read(&self, at: usize, left: &mut [f32], right: &mut [f32]) -> usize {
            assert!(self.0.ready.get(), "a job reads only frames that are ready");
            self.0
                .largest_read
                .set(self.0.largest_read.get().max(left.len()));
            self.0.inner.read(at, left, right)
        }
        fn ready_ranges(&self, _: &[(usize, usize)]) -> bool {
            self.0.requests.set(self.0.requests.get() + 1);
            self.0.ready.get()
        }
    }

    #[test]
    fn a_spectrogram_job_waits_and_asks_for_32_windows_at_a_time() {
        // 2,000,000 frames: `hop` is 1,953, over `n_fft`, so no two windows
        // touch.
        let frames = 2_000_000;
        let song = song(frames);
        let lane = LaneAudio::of_stem(&song.stems[0]);
        let held = Rc::new(Held {
            inner: LanePcm::new(lane.clone(), frames, 44_100),
            ready: Cell::new(false),
            requests: Cell::new(0),
            largest_read: Cell::new(0),
        });
        let mut job = SpectrogramJob::new(Box::new(Shared(Rc::clone(&held))));
        let mut asked_for_time = 0;
        assert!(!job.run_while(|| {
            asked_for_time += 1;
            true
        }));
        assert!(job.is_waiting());
        assert_eq!(held.requests.get(), 1, "one request, then it returns");
        assert_eq!(asked_for_time, 0, "no busy loop while it waits");

        held.ready.set(true);
        held.requests.set(0);
        assert!(job.run_while(|| true));
        let image = job.finish();
        let columns = image.1;
        assert_eq!(
            held.requests.get(),
            columns.div_ceil(32),
            "one request for each 32 columns"
        );
        assert!(
            held.largest_read.get() <= 1_024,
            "it reads only the windows"
        );
        let want = spectrogram_db(
            &mid_of(&LanePcm::new(lane, frames, 44_100)),
            44_100,
            MAX_HZ,
            ROWS,
            MAX_COLS,
        );
        assert!(bits(&image) == bits(&want));
    }

    #[test]
    fn a_spectrogram_job_reads_touching_windows_as_one_range() {
        // 300,000 frames: `hop` is 292, so the windows overlap.
        let frames = 300_000;
        let song = song(frames);
        let held = Rc::new(Held {
            inner: LanePcm::new(LaneAudio::of_stem(&song.stems[0]), frames, 44_100),
            ready: Cell::new(true),
            requests: Cell::new(0),
            largest_read: Cell::new(0),
        });
        let mut job = SpectrogramJob::new(Box::new(Shared(Rc::clone(&held))));
        assert!(job.run_while(|| true));
        let columns = job.finish().1;
        assert_eq!(held.requests.get(), columns.div_ceil(32));
        // 32 columns: 31 hops and one window.
        assert_eq!(held.largest_read.get(), 31 * 292 + 1_024);
    }
}
