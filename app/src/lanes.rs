//! Lane audio: what a lane of the shown song plays and is drawn from, and
//! the sources that read it in blocks (D1 and D3 of the M7 plan).
//!
//! The shown song is a [`Shown`]: the audio of each lane, not an
//! `engine::Song` with lane files beside it.  So the lanes drawn, played
//! and saved cannot differ.  A lane's audio is samples in memory, or a
//! stored lane: a lane file with its overview, read in blocks through a
//! [`LaneStore`].  Until step 7 of the M7 plan, a lane on the web can also
//! keep samples and the file that they were read from (M6).
//!
//! The jobs of the page (the mix lane's waveform, a spectrogram, a save)
//! read the lanes through a [`Pcm`]: [`LanesMix`] for a mix of lanes,
//! [`LanePcm`] for one lane.  A stored lane reads through a
//! [`BlockReader`] of the job, a block at a time.

use crate::lane_store::{BlockReader, LaneStore};
use crate::store_meta::LaneFile;
use engine::mixdown::{MixRecipe, Pcm, at_full_scale};
use engine::{Samples, Song, Stem};
use std::borrow::Cow;
use std::rc::Rc;

/// The waveform of a lane, as `engine::overview::lane_columns` gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Overview {
    pub columns: Vec<(f32, f32)>,
    /// Makes the loudest column fill the lane.
    pub scale: f32,
}

/// What a lane plays and is drawn from.
#[derive(Clone)]
pub enum LaneAudio {
    /// Samples in memory, shared with the song, not copied (MEM-1).
    Memory { left: Samples, right: Samples },
    /// Samples read back whole from a lane file, with that file, which
    /// playback reads (M6).  Step 7 of the M7 plan removes it.
    ReadBack {
        left: Samples,
        right: Samples,
        file: LaneFile,
    },
    /// A lane file, read in blocks from `store`, and its overview.
    Stored {
        file: LaneFile,
        overview: Rc<Overview>,
        store: Rc<dyn LaneStore>,
    },
}

impl LaneAudio {
    /// Samples in memory, with the file they were read from, if any.
    pub fn with_file(left: Samples, right: Samples, file: Option<LaneFile>) -> Self {
        match file {
            Some(file) => LaneAudio::ReadBack { left, right, file },
            None => LaneAudio::Memory { left, right },
        }
    }

    /// The audio of a stem in memory.  It shares the stem's samples.
    pub fn of_stem(stem: &Stem) -> Self {
        LaneAudio::Memory {
            left: stem.left.clone(),
            right: stem.right.clone(),
        }
    }

    /// The samples of the lane, if they are in memory.
    pub fn samples(&self) -> Option<(&Samples, &Samples)> {
        match self {
            LaneAudio::Memory { left, right } | LaneAudio::ReadBack { left, right, .. } => {
                Some((left, right))
            }
            LaneAudio::Stored { .. } => None,
        }
    }

    /// The lane file that the lane plays from, if it has one.
    pub fn file(&self) -> Option<&LaneFile> {
        match self {
            LaneAudio::Memory { .. } => None,
            LaneAudio::ReadBack { file, .. } | LaneAudio::Stored { file, .. } => Some(file),
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

/// One lane of the shown song: its name and its audio.
#[derive(Clone)]
pub struct ShownLane {
    pub name: String,
    pub audio: LaneAudio,
}

/// The shown song (D1 of the M7 plan): its rate, its frames and the audio
/// of each lane, in order.
pub struct Shown {
    pub sample_rate: u32,
    pub frames: usize,
    pub lanes: Vec<ShownLane>,
}

impl Shown {
    /// A song in memory: each stem becomes a lane that shares its samples
    /// (MEM-1).
    pub fn in_memory(song: Song) -> Self {
        let files = vec![None; song.stems.len()];
        Self::with_files(song, files)
    }

    /// A song read back from its lane files: each stem with the file it
    /// was read from, `None` for a lane that plays from memory (D8 of M6).
    /// Its samples move into the lanes, with no copy.
    pub fn with_files(song: Song, files: Vec<Option<LaneFile>>) -> Self {
        assert_eq!(files.len(), song.stems.len(), "a lane file for each lane");
        Self {
            sample_rate: song.sample_rate,
            frames: song.frames,
            lanes: song
                .stems
                .into_iter()
                .zip(files)
                .map(|(stem, file)| ShownLane {
                    name: stem.name,
                    audio: LaneAudio::with_file(stem.left, stem.right, file),
                })
                .collect(),
        }
    }

    /// The names of the lanes, in order.
    pub fn names(&self) -> Vec<&str> {
        self.lanes.iter().map(|l| l.name.as_str()).collect()
    }

    /// The index of the lane `name`.
    pub fn stem_index(&self, name: &str) -> Option<usize> {
        self.lanes.iter().position(|l| l.name == name)
    }

    pub fn duration_s(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }

    /// The audio of each lane, in order.
    pub fn audio(&self) -> Vec<LaneAudio> {
        self.lanes.iter().map(|l| l.audio.clone()).collect()
    }

    /// The lane file of each lane, `None` for a lane that plays from
    /// memory (D8 of M6).
    pub fn files(&self) -> Vec<Option<LaneFile>> {
        self.lanes.iter().map(|l| l.audio.file().cloned()).collect()
    }

    /// Whether a lane plays from a session file, so the session is
    /// protected while the song is shown (D1 of the M7 plan).
    pub fn has_stored_lane(&self) -> bool {
        self.lanes.iter().any(|l| l.audio.file().is_some())
    }

    /// The song as an `engine::Song` whose stems share the lanes' samples
    /// (MEM-1), for the work that reads a whole song in memory: the memory
    /// path of a separation, the mix file of a song decoded on the page,
    /// and classic drums.  `None` when a lane has no samples.
    pub fn as_song(&self) -> Option<Song> {
        let stems = self
            .lanes
            .iter()
            .map(|lane| {
                let (left, right) = lane.audio.samples()?;
                Some(Stem {
                    name: lane.name.clone(),
                    left: left.clone(),
                    right: right.clone(),
                })
            })
            .collect::<Option<Vec<Stem>>>()?;
        Some(Song {
            sample_rate: self.sample_rate,
            frames: self.frames,
            stems,
        })
    }
}

/// The lanes that one job reads.  A lane in memory lends its samples; the
/// stored lanes read through one [`BlockReader`] (D3 of the M7 plan).
struct LaneReads {
    parts: Vec<Part>,
    reader: Option<BlockReader>,
}

/// The two sides of some frames of a lane, lent or made.
type Sides<'a> = (Cow<'a, [f32]>, Cow<'a, [f32]>);

/// How a job reads one lane.
enum Part {
    Memory(Samples, Samples),
    /// The lane at this index of the reader.
    Stored(usize),
}

impl LaneReads {
    /// Reads of `lanes`, lanes of a song of `frames` frames.  Panics if two
    /// stored lanes have different stores.
    fn new(lanes: Vec<LaneAudio>, frames: usize) -> Self {
        let mut files = Vec::new();
        let mut store: Option<Rc<dyn LaneStore>> = None;
        let parts = lanes
            .into_iter()
            .map(|lane| match lane {
                LaneAudio::Memory { left, right } | LaneAudio::ReadBack { left, right, .. } => {
                    Part::Memory(left, right)
                }
                LaneAudio::Stored {
                    file, store: from, ..
                } => {
                    if let Some(store) = &store {
                        assert!(
                            std::ptr::addr_eq(Rc::as_ptr(store), Rc::as_ptr(&from)),
                            "the stored lanes of a job have one store"
                        );
                    }
                    store.get_or_insert(from);
                    files.push(file);
                    Part::Stored(files.len() - 1)
                }
            })
            .collect();
        let reader = store.map(|store| BlockReader::new(store, files, frames));
        Self { parts, reader }
    }

    fn ready_ranges(&self, ranges: &[(usize, usize)]) -> bool {
        self.reader.as_ref().is_none_or(|r| r.ready(ranges))
    }

    /// Gives `f` the `frames` frames from `at` of each lane, which are
    /// ready.  Frames past a lane's end are silence.
    fn with_parts<R>(
        &self,
        at: usize,
        frames: usize,
        f: impl FnOnce(&[&[f32]], &[&[f32]]) -> R,
    ) -> R {
        let held = self.reader.as_ref().map(BlockReader::held);
        let parts: Vec<Sides<'_>> = self
            .parts
            .iter()
            .map(|part| match part {
                Part::Memory(left, right) => {
                    (part_of(left, at, frames), part_of(right, at, frames))
                }
                Part::Stored(k) => {
                    let held = held.as_ref().expect("a reader of the stored lanes");
                    let (left, right) = held.frames(*k, at, frames);
                    (Cow::Borrowed(left), Cow::Borrowed(right))
                }
            })
            .collect();
        let lefts: Vec<&[f32]> = parts.iter().map(|(l, _)| l.as_ref()).collect();
        let rights: Vec<&[f32]> = parts.iter().map(|(_, r)| r.as_ref()).collect();
        f(&lefts, &rights)
    }
}

/// Lanes at the gains of a recipe, summed and clamped to ±1 with
/// `dsp::mix_range`, as the player sums them and as
/// `engine::mixdown::Mix` does.
pub struct LanesMix {
    frames: usize,
    sample_rate: u32,
    /// The lanes with a gain above 0, and their gains.
    audible: LaneReads,
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
            audible: LaneReads::new(
                audible.into_iter().map(|i| lanes[i].clone()).collect(),
                frames,
            ),
        }
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
        let n = left.len();
        self.audible.with_parts(at, n, |lefts, rights| {
            left.copy_from_slice(&dsp::mix_range(lefts, &self.volumes, 0, n));
            right.copy_from_slice(&dsp::mix_range(rights, &self.volumes, 0, n));
        });
        at_full_scale(left) + at_full_scale(right)
    }

    fn ready(&self, at: usize, frames: usize) -> bool {
        self.ready_ranges(&[(at, frames)])
    }

    fn ready_ranges(&self, ranges: &[(usize, usize)]) -> bool {
        self.audible.ready_ranges(ranges)
    }
}

/// One lane as it is: not scaled and not clamped, as
/// `engine::mixdown::StemPcm`.
pub struct LanePcm {
    lane: LaneReads,
    frames: usize,
    sample_rate: u32,
}

impl LanePcm {
    /// The lane `lane`, of `frames` frames at `sample_rate`.
    pub fn new(lane: LaneAudio, frames: usize, sample_rate: u32) -> Self {
        Self {
            lane: LaneReads::new(vec![lane], frames),
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
        self.lane.with_parts(at, left.len(), |lefts, rights| {
            left.copy_from_slice(lefts[0]);
            right.copy_from_slice(rights[0]);
        });
        at_full_scale(left) + at_full_scale(right)
    }

    fn ready(&self, at: usize, frames: usize) -> bool {
        self.ready_ranges(&[(at, frames)])
    }

    fn ready_ranges(&self, ranges: &[(usize, usize)]) -> bool {
        self.lane.ready_ranges(ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lane_store::MemoryStore;
    use engine::Song;
    use engine::mixdown::{Mix, StemPcm};
    use engine::readahead::LaneFormat;
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
    /// at full scale.  Each read is ready at once, as a store in memory
    /// completes its reads at once.
    fn bits_of(pcm: &dyn Pcm, block: usize) -> (Vec<u32>, Vec<u32>, usize) {
        let (mut l, mut r) = (vec![0.0; pcm.frames()], vec![0.0; pcm.frames()]);
        let mut clipped = 0;
        let mut at = 0;
        while at < pcm.frames() {
            let n = block.min(pcm.frames() - at);
            assert!(pcm.ready(at, n), "ready at once");
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
        let audio = LaneAudio::of_stem(&song.stems[0]);
        let (left, right) = audio.samples().expect("samples");
        assert!(left.shares(&song.stems[0].left));
        assert!(right.shares(&song.stems[0].right));
    }

    /// MEM-1: a song in memory becomes lanes that share its samples.  The
    /// count of handles shows that no buffer is copied.
    #[test]
    fn a_song_in_memory_becomes_lanes_with_no_copy() {
        // The clone shares the samples, and the `Arc` goes at the end of
        // the statement, so `song` holds the only handles.
        let song = Song::clone(&song(1_000));
        let kept: Vec<(Samples, Samples)> = song
            .stems
            .iter()
            .map(|s| (s.left.clone(), s.right.clone()))
            .collect();
        let names: Vec<String> = song.stems.iter().map(|s| s.name.clone()).collect();
        let shown = Shown::in_memory(song);
        assert_eq!((shown.sample_rate, shown.frames), (48_000, 1_000));
        assert_eq!(shown.names(), names);
        for ((left, right), lane) in kept.iter().zip(&shown.lanes) {
            let (l, r) = lane.audio.samples().expect("samples in memory");
            assert!(l.shares(left) && r.shares(right), "{}", lane.name);
            assert_eq!((left.handles(), right.handles()), (2, 2), "{}", lane.name);
            assert_eq!(lane.audio.file(), None);
        }
        assert_eq!(shown.stem_index("drums"), Some(1));
        assert_eq!(shown.stem_index("lloyd"), None);
        assert_eq!(shown.duration_s(), 1_000.0 / 48_000.0);
    }

    /// An f32 lane file of the two sides, as the decode worker writes one.
    pub(crate) fn f32_file(left: &[f32], right: &[f32]) -> Vec<u8> {
        left.iter()
            .zip(right)
            .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
            .flatten()
            .collect()
    }

    /// A stored lane of `left` and `right` in `store`, as an f32 file at
    /// `path`, with its overview.
    pub(crate) fn stored_f32(
        store: &Rc<MemoryStore>,
        path: &str,
        left: &[f32],
        right: &[f32],
    ) -> LaneAudio {
        store.put(path, f32_file(left, right));
        let (columns, scale) = engine::overview::lane_columns(left, right);
        LaneAudio::Stored {
            file: LaneFile::f32(path.into(), left.len()),
            overview: Rc::new(Overview { columns, scale }),
            store: Rc::clone(store) as Rc<dyn LaneStore>,
        }
    }

    const BLOCKS: [usize; 4] = [1, 1_000, 16_384, 32_768];

    /// D3 of the M7 plan: a stored lane reads the samples that `read_stem`
    /// and `read_back` gave, bit for bit: a 16-bit WAV through
    /// `decode_bytes` and `padded_to`, and an f32 file as it is.  The reads
    /// cross the end of a block and the end of the file, and none is more
    /// than a block.
    #[test]
    fn a_stored_lane_reads_as_its_samples() {
        let frames = 70_001;
        let song = song(frames);
        let (left, right) = (&song.stems[1].left, &song.stems[1].right);
        let store = MemoryStore::new();
        // A 16-bit WAV one frame short of the song, as a stem can be.
        let wav = engine::wav::encode_pcm16(&left[..frames - 1], &right[..frames - 1], 44_100);
        let path = "acme/stems/drums.wav";
        let head = &wav[..4_096.min(wav.len())];
        let file = crate::store_meta::stem_lane_file(path, head, wav.len() as u64, frames)
            .expect("a complete stem");
        store.put(path, wav.clone());
        let audio = engine::decode::decode_bytes(wav, Some("wav")).expect("decoded");
        let mut channels = audio.channels.into_iter();
        let mut padded = || engine::song::padded_to(channels.next().expect("a channel"), frames);
        let want_wav = (padded(), padded());
        // An f32 file of fewer frames than the song: silence after it.
        let short = 50_000;
        store.put(
            "acme/decoded/0.f32",
            f32_file(&left[..short], &right[..short]),
        );
        let f32_file = LaneFile::f32("acme/decoded/0.f32".into(), short);
        let mut want_f32 = (left[..short].to_owned(), right[..short].to_owned());
        want_f32.0.resize(frames, 0.0);
        want_f32.1.resize(frames, 0.0);

        for (file, want) in [(file, want_wav), (f32_file, want_f32)] {
            let bits = |x: &[f32]| x.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
            let lane = LaneAudio::Stored {
                file: file.clone(),
                overview: Rc::new(Overview {
                    columns: Vec::new(),
                    scale: 1.0,
                }),
                store: Rc::clone(&store) as Rc<dyn LaneStore>,
            };
            for block in BLOCKS {
                let pcm = LanePcm::new(lane.clone(), frames, 44_100);
                let (l, r, _) = bits_of(&pcm, block);
                assert!(
                    l == bits(&want.0),
                    "{}: left in blocks of {block}",
                    file.path
                );
                assert!(
                    r == bits(&want.1),
                    "{}: right in blocks of {block}",
                    file.path
                );
            }
            let block_bytes = file.format.block_bytes() as u64;
            assert!(store.largest_read() <= block_bytes, "{}", file.path);
        }
    }

    /// D3: a mix and a save of stored lanes equal those of the same samples
    /// in memory, bit for bit, also when some lanes are in memory.
    #[test]
    fn a_stored_song_mixes_and_saves_as_in_memory() {
        let frames = 70_001;
        let song = song(frames);
        let memory = lanes_of(&song);
        let store = MemoryStore::new();
        let stored: Vec<LaneAudio> = song
            .stems
            .iter()
            .map(|s| stored_f32(&store, &format!("acme/{}.f32", s.name), &s.left, &s.right))
            .collect();
        let some_stored: Vec<LaneAudio> = (0..memory.len())
            .map(|i| {
                if i % 2 == 0 {
                    stored[i].clone()
                } else {
                    memory[i].clone()
                }
            })
            .collect();
        for gains in [
            vec![1.0, 1.0, 1.0, 0.0],
            vec![0.75, 1.5, 0.0, 0.25],
            vec![0.0; 4],
        ] {
            let recipe = MixRecipe::new(gains);
            let want = LanesMix::new(&memory, frames, 48_000, &recipe);
            for lanes in [&stored, &some_stored] {
                let got = LanesMix::new(lanes, frames, 48_000, &recipe);
                for block in BLOCKS {
                    assert!(
                        bits_of(&got, block) == bits_of(&want, block),
                        "{recipe:?} in blocks of {block}"
                    );
                }
            }
        }
        for (k, lane) in stored.iter().enumerate() {
            let want = LanePcm::new(memory[k].clone(), frames, 48_000);
            let got = LanePcm::new(lane.clone(), frames, 48_000);
            assert!(bits_of(&got, 16_384) == bits_of(&want, 16_384), "lane {k}");
        }
        // A spectrogram reads windows inside its ranges.
        let image = |pcm: LanesMix| {
            let mut job = crate::spectrogram::SpectrogramJob::new(Box::new(pcm));
            assert!(job.run_while(|| true));
            job.finish()
                .0
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<u32>>()
        };
        let all = MixRecipe::new(vec![1.0, 0.5, 1.0, 0.0]);
        assert!(
            image(LanesMix::new(&some_stored, frames, 48_000, &all))
                == image(LanesMix::new(&memory, frames, 48_000, &all)),
            "the spectrogram of the mix"
        );
        // Each read of a job is at most a block of a lane.
        assert!(store.largest_read() <= LaneFormat::F32.block_bytes() as u64);
    }

    /// A sink that keeps what it is given.
    #[derive(Default)]
    struct Kept(Vec<f32>, Vec<f32>);

    impl crate::export::Sink for Kept {
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            assert_eq!(at, self.0.len(), "blocks in order");
            self.0.extend_from_slice(left);
            self.1.extend_from_slice(right);
        }
    }

    /// D3: a job whose read is held makes no progress, starts no second
    /// read, then gives the same output when the reads complete.
    #[test]
    fn a_job_waits_for_a_held_read() {
        use crate::export::{BLOCK, ExportJob};
        let frames = 2 * BLOCK + 5;
        let song = song(frames);
        let store = MemoryStore::new();
        let lane = stored_f32(
            &store,
            "acme/bass.f32",
            &song.stems[0].left,
            &song.stems[0].right,
        );
        store.hold(true);
        let mut job = ExportJob::new(
            Box::new(LanePcm::new(lane, frames, 48_000)),
            Kept::default(),
        );
        assert!(!job.run_while(|| true), "it waits");
        assert!(job.is_waiting());
        assert_eq!(job.fraction(), 0.0, "no progress");
        assert!(!job.run_while(|| true));
        assert_eq!(store.requests(), 1, "one read in flight, not a second");
        store.release();
        assert!(!job.run_while(|| false), "one block, then no time");
        assert!(!job.is_waiting());
        assert_eq!(job.fraction(), BLOCK as f32 / frames as f32);
        store.hold(false);
        assert!(job.run_while(|| true));
        let got = job.into_sink();
        let mut want = ExportJob::new(
            Box::new(LanePcm::new(
                LaneAudio::of_stem(&song.stems[0]),
                frames,
                48_000,
            )),
            Kept::default(),
        );
        assert!(want.run_while(|| true));
        let want = want.into_sink();
        assert!(got.0 == want.0 && got.1 == want.1);
    }

    #[test]
    #[should_panic(expected = "one gain for each lane")]
    fn a_recipe_for_other_lanes_panics() {
        let song = song(10);
        LanesMix::new(&lanes_of(&song), 10, 48_000, &MixRecipe::new(vec![1.0]));
    }
}
