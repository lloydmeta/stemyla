//! A song is a set of equally long stereo stems at one sample rate.
//!
//! [`Song::from_audio`] makes a song from whole decoded files.
//! [`decode_into`] decodes one source in blocks into a [`FrameSink`], and
//! gives the samples of that source's stem, bit for bit, so a caller need
//! not hold the song.

use crate::decode::{Audio, Decoder, decode_bytes, decode_path};
use crate::resample::{Resampler, SONG_RATE, to_rate};
use anyhow::{Context, Result, anyhow};
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

/// One channel of a stem: immutable once made, and shared rather than
/// copied when a stem or song is cloned.  A song's stems run to hundreds of
/// MB, and the web page's WebAssembly memory never shrinks, so a copy made
/// for a moment (to save stems while they are shown, say) stays for good.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Samples(Arc<Vec<f32>>);

impl Samples {
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// How many `Samples` share this buffer, `self` included.
    pub fn handles(&self) -> usize {
        Arc::strong_count(&self.0)
    }

    /// Whether `self` and `other` are the same buffer, not just equal.
    pub fn shares(&self, other: &Samples) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for Samples {
    type Target = [f32];

    fn deref(&self) -> &[f32] {
        &self.0
    }
}

/// Takes the vector's buffer: no copy when it has no spare capacity.  Spare
/// capacity is released first, since samples never grow and would hold it
/// for good, and releasing it may reallocate the buffer once.
impl From<Vec<f32>> for Samples {
    fn from(mut v: Vec<f32>) -> Self {
        v.shrink_to_fit();
        Self(Arc::new(v))
    }
}

impl PartialEq<Vec<f32>> for Samples {
    fn eq(&self, other: &Vec<f32>) -> bool {
        *self.0 == *other
    }
}

#[derive(Clone, Debug)]
pub struct Stem {
    pub name: String,
    pub left: Samples,
    pub right: Samples,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub sample_rate: u32,
    pub frames: usize,
    pub stems: Vec<Stem>,
}

/// Lane order used when stems are loaded from a folder.  Unknown names go
/// last, alphabetically.
const PREFERRED_ORDER: [&str; 10] = [
    "drums",
    "bass",
    "guitar",
    "piano",
    "keys",
    "vocals",
    "lead",
    "other",
    "harmonic",
    "percussive",
];

/// File names that are never stems: the original mix is recomputed from the
/// stems so every lane stays aligned.
const NOT_STEMS: [&str; 3] = ["mix", "original", "truth"];

/// Whether a file of the stem name `name` (see [`stem_name`]) is a stem:
/// `mix`, `original` and `truth` are not.
pub fn is_stem(name: &str) -> bool {
    !NOT_STEMS.contains(&name)
}

/// Where a stem of `name` goes in a song: the order of [`Song::from_audio`],
/// which sorts by it, keeping the order of equal names.
pub fn rank(name: &str) -> (usize, String) {
    let pos = PREFERRED_ORDER
        .iter()
        .position(|p| *p == name)
        .unwrap_or(PREFERRED_ORDER.len());
    (pos, name.to_owned())
}

impl Song {
    /// Builds a song at [`SONG_RATE`] from decoded files (decision 0012).
    /// Mono becomes stereo and extra channels are dropped.  A stem at
    /// another rate is converted, and a stem at that rate is kept as it is,
    /// with no copy.  Shorter stems are padded with silence to the longest.
    pub fn from_audio(mut items: Vec<(String, Audio)>) -> Result<Song> {
        if items.is_empty() {
            return Err(anyhow!("no stems to load"));
        }
        items.sort_by_key(|(n, _)| rank(n));
        let mut decoded = Vec::with_capacity(items.len());
        for (name, audio) in items {
            let rate = audio.sample_rate;
            // One channel at a time, so each input is dropped before the
            // next channel is converted.
            let mut channels = audio
                .channels
                .into_iter()
                .take(2)
                .map(|c| to_rate(c, rate, SONG_RATE).with_context(|| format!("stem {name}")));
            let Some(left) = channels.next().transpose()? else {
                return Err(anyhow!("stem {name} has no channels"));
            };
            let right = channels.next().transpose()?;
            decoded.push((name, left, right));
        }
        let frames = decoded.iter().map(|(_, l, _)| l.len()).max().unwrap_or(0);
        let padded = |c: Vec<f32>| -> Samples { padded_to(c, frames).into() };
        let stems = decoded
            .into_iter()
            .map(|(name, left, right)| {
                let left = padded(left);
                // A mono stem plays the same samples on both sides.
                let right = right.map_or_else(|| left.clone(), padded);
                Stem { name, left, right }
            })
            .collect();
        Ok(Song {
            sample_rate: SONG_RATE,
            frames,
            stems,
        })
    }

    /// Loads every WAV, MP3 and FLAC in a folder as a stem (skipping
    /// `mix.*`).
    pub fn load_folder(dir: &Path) -> Result<Song> {
        let mut items = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let path = e.path();
            let ext = path
                .extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(ext.as_deref(), Some("wav" | "mp3" | "flac")) {
                continue;
            }
            let name = stem_name(&path.to_string_lossy());
            if !is_stem(&name) {
                continue;
            }
            items.push((name, decode_path(&path)?));
        }
        Song::from_audio(items)
    }

    /// Loads in-memory files, as from a multi-file drop: `(file name, bytes)`.
    pub fn load_files(files: Vec<(String, Vec<u8>)>) -> Result<Song> {
        let mut items = Vec::new();
        for (file_name, bytes) in files {
            let name = stem_name(&file_name);
            if !is_stem(&name) {
                continue;
            }
            let ext = file_name.rsplit('.').next().map(str::to_ascii_lowercase);
            items.push((name, decode_bytes(bytes, ext.as_deref())?));
        }
        Song::from_audio(items)
    }

    /// Loads in-memory files as [`Song::load_files`] does, bit for bit, for
    /// a song of at most `max_frames` frames at [`SONG_RATE`] (D3 of
    /// `plans/active/stream-from-storage-catchup.md`), with [`SongWithin`].
    /// A WAV or FLAC whose stated length passes it is refused before it
    /// decodes, and any decode stops once it passes it, each with
    /// `decode::TooLong`.
    pub fn load_files_within(files: Vec<(String, Vec<u8>)>, max_frames: usize) -> Result<Song> {
        files
            .into_iter()
            .try_fold(SongWithin::new(max_frames), |song, (name, bytes)| {
                song.with_file(&name, bytes)
            })?
            .song()
    }

    pub fn duration_s(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }

    pub fn stem_index(&self, name: &str) -> Option<usize> {
        self.stems.iter().position(|s| s.name == name)
    }

    /// The mix lane: every stem at full volume, summed with `dsp::mix`.
    pub fn mix_lane(&self) -> (Vec<f32>, Vec<f32>) {
        let volumes = vec![1.0; self.stems.len()];
        let lefts: Vec<&[f32]> = self.stems.iter().map(|s| s.left.as_slice()).collect();
        let rights: Vec<&[f32]> = self.stems.iter().map(|s| s.right.as_slice()).collect();
        (dsp::mix(&lefts, &volumes), dsp::mix(&rights, &volumes))
    }
}

/// A song that its caller makes one file at a time, for a song of at
/// most `max_frames` frames at [`SONG_RATE`] (D3 of
/// `plans/active/stream-from-storage-catchup.md`).  Each file decodes in
/// blocks with [`decode_into`] when it is added, and its bytes go when its
/// decode ends.  So a caller that reads each file just before it adds it
/// holds the bytes of one file at a time.  The song holds no samples at a
/// source's rate.  It holds only its own samples, at most `max_frames` of
/// each channel (finding 1-3 of review round 1 of the catch-up plan).
pub struct SongWithin {
    max_frames: usize,
    /// Each source's name, left side, and right side unless it is mono.
    decoded: Vec<(String, Vec<f32>, Option<Vec<f32>>)>,
}

impl SongWithin {
    /// An empty song with the limit `max_frames`.
    pub fn new(max_frames: usize) -> Self {
        Self {
            max_frames,
            decoded: Vec::new(),
        }
    }

    /// The song with the file `file_name` of `bytes` added, if it is a
    /// stem ([`is_stem`]).  A WAV or FLAC whose stated length passes the
    /// limit is refused before it decodes, and a decode stops once it
    /// passes the limit, each with `decode::TooLong`.
    pub fn with_file(self, file_name: &str, bytes: Vec<u8>) -> Result<Self> {
        let name = stem_name(file_name);
        if !is_stem(&name) {
            return Ok(self);
        }
        let ext = file_name.rsplit('.').next().map(str::to_ascii_lowercase);
        let source = Box::new(std::io::Cursor::new(bytes));
        let decoder = Decoder::within_song_frames(source, ext.as_deref(), self.max_frames)?;
        let reserve = decoder.song_frames_to_reserve().min(self.max_frames);
        let mut sink = SongSink::new(reserve, self.max_frames);
        decode_into(decoder, &mut sink)?;
        let mut decoded = self.decoded;
        decoded.push((name, sink.left, sink.right));
        Ok(Self {
            max_frames: self.max_frames,
            decoded,
        })
    }

    /// The song of the files added: the lanes in rank order, a mono source
    /// on both sides with one buffer, and shorter lanes padded with
    /// silence.  An error if no file was a stem.
    pub fn song(self) -> Result<Song> {
        let mut decoded = self.decoded;
        if decoded.is_empty() {
            return Err(anyhow!("no stems to load"));
        }
        decoded.sort_by_key(|(name, ..)| rank(name));
        let frames = decoded.iter().map(|(_, l, _)| l.len()).max().unwrap_or(0);
        let padded = |c: Vec<f32>| -> Samples { padded_to(c, frames).into() };
        let stems = decoded
            .into_iter()
            .map(|(name, left, right)| {
                let left = padded(left);
                // A mono stem plays the same samples on both sides.
                let right = right.map_or_else(|| left.clone(), padded);
                Stem { name, left, right }
            })
            .collect();
        Ok(Song {
            sample_rate: SONG_RATE,
            frames,
            stems,
        })
    }
}

/// Takes the frames of a decoded source in blocks, as two channels at
/// [`SONG_RATE`].
pub trait FrameSink {
    /// Takes the next block.  `left` and `right` have the same length.  A
    /// mono source gives the same slice as both.  An error stops the
    /// decode.
    fn put(&mut self, left: &[f32], right: &[f32]) -> Result<()>;
}

/// Keeps the blocks of one source of [`SongWithin`]: its
/// left side, and its right side unless the source is mono.  It refuses a
/// block that takes it past `max_frames`.
struct SongSink {
    left: Vec<f32>,
    /// `None` while every block of the source was mono.
    right: Option<Vec<f32>>,
    max_frames: usize,
}

impl SongSink {
    /// A sink with room for `reserve` frames of each side.
    fn new(reserve: usize, max_frames: usize) -> Self {
        Self {
            left: Vec::with_capacity(reserve),
            right: None,
            max_frames,
        }
    }
}

impl FrameSink for SongSink {
    fn put(&mut self, left: &[f32], right: &[f32]) -> Result<()> {
        if self.left.len() + left.len() > self.max_frames {
            return Err(anyhow::Error::new(crate::decode::TooLong {
                max_frames: self.max_frames,
            }));
        }
        // `decode_into` gives a mono source as one slice on both sides.
        let mono = std::ptr::eq(left, right);
        if !mono && self.right.is_none() {
            let mut right = Vec::with_capacity(self.left.capacity());
            right.extend_from_slice(&self.left);
            self.right = Some(right);
        }
        crate::decode::append(&mut self.left, left, self.max_frames);
        if let Some(side) = &mut self.right {
            crate::decode::append(side, right, self.max_frames);
        }
        Ok(())
    }
}

/// What [`decode_into`] gave its sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedSource {
    /// The frames at [`SONG_RATE`].
    pub frames: usize,
    /// The channels of the source that the song keeps: 1 or 2.
    pub channels: usize,
}

/// Decodes one source into `sink`, block by block, as one stem of
/// [`Song::from_audio`]: at most two channels, a mono source on both sides,
/// and the rate converted to [`SONG_RATE`].  The sink gets the samples of
/// that stem, bit for bit, with no padding.  It holds no more than a packet
/// and a filter window, so its memory does not grow with the source.
pub fn decode_into(mut decoder: Decoder, sink: &mut dyn FrameSink) -> Result<DecodedSource> {
    // A converter for the left side, and one for the right of a source of
    // two channels or more, made at the first packet that has a channel,
    // as `decode_bytes` makes its channels.
    let mut sides: Option<(Resampler, Option<Resampler>)> = None;
    let mut frames = 0;
    while let Some(packet) = decoder.next_packet()? {
        let planes = packet.planes;
        if sides.is_none() && !planes.is_empty() {
            let new = || Resampler::new(packet.sample_rate, SONG_RATE);
            sides = Some((new()?, if planes.len() > 1 { Some(new()?) } else { None }));
        }
        let Some((left, right)) = &mut sides else {
            continue;
        };
        if right.is_some() && planes.len() < 2 {
            return Err(anyhow!("the channels of the file changed while it decoded"));
        }
        let left_block = left.push(&planes[0]);
        let right_block = match right {
            Some(right) => right.push(&planes[1]),
            None => left_block,
        };
        frames += put(sink, left_block, right_block)?;
    }
    let Some((mut left, mut right)) = sides.filter(|_| decoder.frames() > 0) else {
        return Err(anyhow!("file decoded to no audio"));
    };
    let left_block = left.finish();
    let right_block = match &mut right {
        Some(right) => right.finish(),
        None => left_block,
    };
    frames += put(sink, left_block, right_block)?;
    let channels = if right.is_some() { 2 } else { 1 };
    Ok(DecodedSource { frames, channels })
}

/// Gives `sink` a block, if it has frames, and returns its frames.
fn put(sink: &mut dyn FrameSink, left: &[f32], right: &[f32]) -> Result<usize> {
    if !left.is_empty() {
        sink.put(left, right)?;
    }
    Ok(left.len())
}

/// `c` padded with silence (or cut) to `frames`, growing by exactly what it
/// needs: `resize` alone would double a full buffer, briefly holding three
/// times the channel.
pub fn padded_to(mut c: Vec<f32>, frames: usize) -> Vec<f32> {
    c.reserve_exact(frames.saturating_sub(c.len()));
    c.resize(frames, 0.0);
    c
}

/// `"path/to/Drums.wav"` becomes `"drums"`.
pub fn stem_name(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let base = base.rsplit_once('.').map_or(base, |(b, _)| b);
    base.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem1_lint_is_on() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1 canary: clippy fails here if this path is off"
        )]
        let copy = [1.0_f32].to_vec();
        assert_eq!(copy, [1.0]);
    }

    /// Round 2 findings 2-9 to 2-33: a song of two stems of 88,200 frames
    /// at 44.1 kHz gives its mix lane, its length and its stems' indexes.
    #[test]
    fn a_song_gives_its_mix_lane_duration_and_stem_indexes() {
        const FRAMES: usize = 88_200;
        let stem = |name: &str, left: f32, right: f32| Stem {
            name: name.into(),
            left: vec![left; FRAMES].into(),
            right: vec![right; FRAMES].into(),
        };
        let song = Song {
            sample_rate: 44_100,
            frames: FRAMES,
            stems: vec![stem("bass", 0.25, -0.5), stem("drums", 0.5, 0.125)],
        };
        let (left, right) = song.mix_lane();
        assert_eq!((left.len(), right.len()), (FRAMES, FRAMES));
        assert!(left.iter().all(|&s| s == 0.75), "the sum of the lefts");
        assert!(right.iter().all(|&s| s == -0.375), "the sum of the rights");
        assert_eq!(song.duration_s(), 2.0);
        assert_eq!(song.stem_index("bass"), Some(0));
        assert_eq!(song.stem_index("drums"), Some(1));
        assert_eq!(song.stem_index("vocals"), None);
    }

    fn audio(rate: u32, chans: Vec<Vec<f32>>) -> Audio {
        Audio {
            sample_rate: rate,
            channels: chans,
        }
    }

    /// Keeps every block, joined: the samples of the stem.
    #[derive(Default)]
    struct MemorySink {
        left: Vec<f32>,
        right: Vec<f32>,
    }

    impl FrameSink for MemorySink {
        fn put(&mut self, left: &[f32], right: &[f32]) -> Result<()> {
            self.left.extend_from_slice(left);
            self.right.extend_from_slice(right);
            Ok(())
        }
    }

    /// Keeps each block on its own.
    #[derive(Default)]
    struct BlockSink(Vec<(Vec<f32>, Vec<f32>)>);

    impl FrameSink for BlockSink {
        fn put(&mut self, left: &[f32], right: &[f32]) -> Result<()> {
            #[expect(
                clippy::disallowed_methods,
                reason = "MEM-1: the test keeps each small block to compare it"
            )]
            self.0.push((left.to_vec(), right.to_vec()));
            Ok(())
        }
    }

    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|s| s.to_bits()).collect()
    }

    fn decoder_of(bytes: &[u8], ext: &str) -> Decoder {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: the test decodes one small file more than once"
        )]
        let source = Box::new(std::io::Cursor::new(bytes.to_vec()));
        Decoder::new(source, Some(ext)).unwrap()
    }

    /// The test files: a 16-bit WAV, a float WAV, a FLAC and an MP3 of
    /// `channels` tones at `rate`.
    fn files(channels: usize, rate: u32) -> Vec<(&'static str, Vec<u8>)> {
        use crate::test_audio::{WavSamples, flac, mp3, tone, wav};
        // Not a whole number of packets of any of the formats.
        let frames = 10_007;
        let tones: Vec<Vec<f32>> = (0..channels).map(|c| tone(c, frames, rate)).collect();
        let mut files = vec![
            ("wav", wav(&tones, rate, WavSamples::Pcm16)),
            ("wav", wav(&tones, rate, WavSamples::Float)),
        ];
        if channels <= 2 {
            files.push(("flac", flac(&tones, rate)));
            files.push(("mp3", mp3(&tones, rate)));
        }
        files
    }

    #[test]
    fn a_block_decode_gives_the_whole_decode() {
        for rate in [SONG_RATE, 48_000] {
            for channels in [1, 2, 3] {
                for (ext, bytes) in files(channels, rate) {
                    let case = format!("{ext}, {channels} channels at {rate} Hz");
                    #[expect(
                        clippy::disallowed_methods,
                        reason = "MEM-1: the test decodes one small file more than once"
                    )]
                    let whole = decode_bytes(bytes.to_vec(), Some(ext)).unwrap();
                    let song = Song::from_audio(vec![("bass".into(), whole.clone())]).unwrap();
                    let stem = &song.stems[0];

                    let mut memory = MemorySink::default();
                    let decoded = decode_into(decoder_of(&bytes, ext), &mut memory).unwrap();
                    let kept = channels.min(2);
                    assert_eq!(
                        decoded,
                        DecodedSource {
                            frames: song.frames,
                            channels: kept
                        },
                        "{case}"
                    );
                    assert!(bits(&memory.left) == bits(&stem.left), "{case}: left");
                    assert!(bits(&memory.right) == bits(&stem.right), "{case}: right");
                    if rate == SONG_RATE {
                        let right = &whole.channels[kept - 1];
                        assert!(bits(&memory.left) == bits(&whole.channels[0]), "{case}");
                        assert!(bits(&memory.right) == bits(right), "{case}");
                    }

                    let mut blocks = BlockSink::default();
                    decode_into(decoder_of(&bytes, ext), &mut blocks).unwrap();
                    assert!(blocks.0.len() > 1, "{case}: {} blocks", blocks.0.len());
                    let mut joined = MemorySink::default();
                    for (left, right) in &blocks.0 {
                        assert_eq!(left.len(), right.len(), "{case}");
                        joined.put(left, right).unwrap();
                    }
                    assert!(bits(&joined.left) == bits(&stem.left), "{case}: blocks");
                    assert!(bits(&joined.right) == bits(&stem.right), "{case}: blocks");

                    // The decoder's packets are the whole decode's channels.
                    let mut decoder = decoder_of(&bytes, ext);
                    let mut planes = vec![Vec::new(); whole.channels.len()];
                    while let Some(packet) = decoder.next_packet().unwrap() {
                        assert_eq!(packet.sample_rate, whole.sample_rate, "{case}");
                        for (all, plane) in planes.iter_mut().zip(packet.planes) {
                            all.extend_from_slice(plane);
                        }
                    }
                    assert_eq!(decoder.frames(), whole.frames() as u64, "{case}");
                    for (all, want) in planes.iter().zip(&whole.channels) {
                        assert!(bits(all) == bits(want), "{case}: packets");
                    }
                }
            }
        }
    }

    #[test]
    fn a_block_decode_of_no_audio_is_refused() {
        // A WAV with a header and no samples.
        let bytes = crate::test_audio::wav(&[Vec::new(), Vec::new()], SONG_RATE, {
            crate::test_audio::WavSamples::Pcm16
        });
        let whole = decode_bytes(bytes.clone(), Some("wav")).unwrap_err();
        let error = decode_into(decoder_of(&bytes, "wav"), &mut MemorySink::default())
            .expect_err("refused");
        assert_eq!(format!("{error:#}"), format!("{whole:#}"));
    }

    #[test]
    fn a_block_decode_at_an_odd_rate_is_refused() {
        let tones = [crate::test_audio::tone(0, 1_000, 44_056)];
        let bytes = crate::test_audio::wav(&tones, 44_056, crate::test_audio::WavSamples::Pcm16);
        let mut sink = MemorySink::default();
        let error = decode_into(decoder_of(&bytes, "wav"), &mut sink).expect_err("refused");
        assert!(
            format!("{error:#}").contains("songs at 44056 Hz are not supported"),
            "{error:#}"
        );
        assert!(sink.left.is_empty());
    }

    #[test]
    fn a_failing_sink_stops_the_decode() {
        struct Full(usize);
        impl FrameSink for Full {
            fn put(&mut self, _: &[f32], _: &[f32]) -> Result<()> {
                self.0 += 1;
                Err(anyhow!("the sink is full"))
            }
        }
        let (_, bytes) = files(2, SONG_RATE).remove(0);
        let mut sink = Full(0);
        let error = decode_into(decoder_of(&bytes, "wav"), &mut sink).expect_err("stopped");
        assert_eq!(format!("{error:#}"), "the sink is full");
        assert_eq!(sink.0, 1);
    }

    #[test]
    fn mix_original_and_truth_are_not_stems() {
        for name in ["mix", "original", "truth"] {
            assert!(!is_stem(name), "{name}");
        }
        for name in ["drums", "classic drums", "lloyd"] {
            assert!(is_stem(name), "{name}");
        }
    }

    #[test]
    fn names_and_order() {
        assert_eq!(stem_name("a/b/Drums.WAV"), "drums");
        let s = Song::from_audio(vec![
            ("zeta".into(), audio(SONG_RATE, vec![vec![0.0; 3]])),
            ("bass".into(), audio(SONG_RATE, vec![vec![0.0; 3]])),
            ("drums".into(), audio(SONG_RATE, vec![vec![0.0; 3]])),
        ])
        .unwrap();
        let names: Vec<_> = s.stems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["drums", "bass", "zeta"]);
    }

    #[test]
    fn mono_and_padding() {
        let s = Song::from_audio(vec![
            (
                "drums".into(),
                audio(SONG_RATE, vec![vec![1.0; 10], vec![0.5; 10]]),
            ),
            ("bass".into(), audio(SONG_RATE, vec![vec![0.25; 4]])),
        ])
        .unwrap();
        assert_eq!(s.frames, 10);
        assert_eq!(s.stems[1].left.len(), 10);
        assert_eq!(s.stems[1].left[..4], [0.25; 4]);
        assert_eq!(s.stems[1].right[9], 0.0);
        assert_eq!(s.stems[0].right[0], 0.5);
    }

    /// A file of `channels` tones of `frames` frames at `rate`, as `ext`.
    fn file_at(ext: &str, channels: usize, frames: usize, rate: u32) -> Vec<u8> {
        use crate::test_audio::{WavSamples, flac, mp3, tone, wav};
        let tones: Vec<Vec<f32>> = (0..channels).map(|c| tone(c, frames, rate)).collect();
        match ext {
            "wav" => wav(&tones, rate, WavSamples::Pcm16),
            "flac" => flac(&tones, rate),
            _ => mp3(&tones, rate),
        }
    }

    /// Whether `result` is the error of a song past `max_frames`.
    fn past<T>(result: &Result<T>, max_frames: usize) -> bool {
        result
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<crate::decode::TooLong>())
            .is_some_and(|too_long| too_long.max_frames == max_frames)
    }

    /// D3 of the catch-up plan: the page's limit is in frames at
    /// `SONG_RATE`, whatever the rate of the source.  A WAV or a FLAC that
    /// states its length is refused when it opens; a streaming WAV and an
    /// MP3 stop when the decode passes the limit.
    #[test]
    fn the_page_limit_counts_frames_at_the_song_rate() {
        use crate::decode::Decoder;
        use std::io::Cursor;
        for rate in [22_050, 48_000] {
            for (ext, streaming) in [
                ("wav", false),
                ("wav", true),
                ("flac", false),
                ("mp3", false),
            ] {
                let mut bytes = file_at(ext, 2, 10_007, rate);
                if streaming {
                    bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
                    bytes[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
                }
                let case = format!("{ext} at {rate} Hz, streaming {streaming}");
                let name = format!("lloyd.{ext}");
                let whole = Song::load_files(vec![(name.clone(), bytes.clone())]).unwrap();
                let max = whole.frames;
                let at = Song::load_files_within(vec![(name.clone(), bytes.clone())], max);
                assert_eq!(at.unwrap().frames, max, "{case}: at the limit");
                let over = Song::load_files_within(vec![(name, bytes.clone())], max - 1);
                assert!(past(&over, max - 1), "{case}: one frame over");
                let opened =
                    Decoder::within_song_frames(Box::new(Cursor::new(bytes)), Some(ext), max - 1);
                let stated = ext != "mp3" && !streaming;
                assert_eq!(
                    past(&opened, max - 1),
                    stated,
                    "{case}: refused when it opens"
                );
            }
        }
        // 22.05 kHz doubles: 10,007 frames are 20,014 at 44.1 kHz.
        let bytes = file_at("wav", 2, 10_007, 22_050);
        let song = Song::load_files_within(vec![("lloyd.wav".into(), bytes)], 20_014).unwrap();
        assert_eq!(song.frames, 20_014);
    }

    /// D3 of the catch-up plan: within its limit, a song loads as
    /// `load_files` loads it, bit for bit: lanes in rank order, a mono
    /// source on both sides with one buffer, shorter lanes padded, and a
    /// `mix` file left out.
    #[test]
    fn a_song_within_the_page_limit_is_the_song_of_load_files() {
        let files = vec![
            ("bass.flac".to_string(), file_at("flac", 1, 5_003, 22_050)),
            ("drums.wav".to_string(), file_at("wav", 2, 9_001, 48_000)),
            ("mix.wav".to_string(), file_at("wav", 2, 100, 44_100)),
            ("vocals.mp3".to_string(), file_at("mp3", 2, 4_000, 48_000)),
        ];
        let whole = Song::load_files(files.clone()).unwrap();
        let within = Song::load_files_within(files, whole.frames).unwrap();
        assert_eq!(
            (within.sample_rate, within.frames),
            (whole.sample_rate, whole.frames)
        );
        let names =
            |song: &Song| -> Vec<String> { song.stems.iter().map(|s| s.name.clone()).collect() };
        assert_eq!(names(&within), ["drums", "bass", "vocals"]);
        assert_eq!(names(&within), names(&whole));
        for (a, b) in within.stems.iter().zip(&whole.stems) {
            let bits = |x: &[f32]| -> Vec<u32> { x.iter().map(|v| v.to_bits()).collect() };
            assert_eq!(bits(&a.left), bits(&b.left), "{}", a.name);
            assert_eq!(bits(&a.right), bits(&b.right), "{}", a.name);
        }
        let bass = &within.stems[1];
        assert!(
            bass.left.shares(&bass.right),
            "a mono source has one buffer"
        );
        assert!(Song::load_files_within(Vec::new(), 1).is_err(), "no stems");
    }

    /// Review round 1 of the catch-up plan (the missed mutant of
    /// `Decoder::song_frames_to_reserve`): a song within the page limit
    /// makes room for each source's stated length at once.  So its heap
    /// peaks at about its lanes, not at the old and the new buffer of a
    /// lane that grows.
    #[test]
    fn a_song_within_the_page_limit_reserves_its_lanes_once() {
        let frames = 441_000;
        let side = vec![0.25_f32; frames];
        let bytes = crate::wav::encode_pcm16(&side, &side, 44_100);
        let lanes = 2 * 4 * frames;
        let (peak, song) = crate::test_alloc::peak_of(move || {
            Song::load_files_within(vec![("lloyd.wav".into(), bytes)], 10 * frames)
        });
        assert_eq!(song.unwrap().frames, frames);
        assert!(
            (peak as f64) < 1.1 * lanes as f64,
            "the heap peaked at {peak} bytes for lanes of {lanes} bytes"
        );
    }

    #[test]
    fn a_96k_flac_opens_at_44k() {
        let tone: Vec<f32> = (0..96_000)
            .map(|i| 0.25 * (0.05 * i as f32).sin())
            .collect();
        let flac = crate::test_flac::compressed(&tone, &tone, 96_000);
        let s = Song::load_files(vec![("lloyd.flac".into(), flac)]).unwrap();
        assert_eq!((s.sample_rate, s.frames), (SONG_RATE, 44_100));
    }

    #[test]
    fn a_folder_with_a_flac_stem_loads_it() {
        let dir = std::env::temp_dir().join(format!("stemyla-flac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tone: Vec<f32> = (0..4410).map(|i| 0.25 * (0.05 * i as f32).sin()).collect();
        let files = [
            (
                "bass.flac",
                crate::test_flac::compressed(&tone, &tone, SONG_RATE),
            ),
            (
                "drums.wav",
                crate::wav::encode_pcm16(&tone, &tone, SONG_RATE),
            ),
            ("notes.txt", b"not audio".into()),
        ];
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let loaded = Song::load_folder(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        let s = loaded.unwrap();
        let names: Vec<_> = s.stems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["drums", "bass"]);
        assert_eq!(s.frames, 4410);
    }

    #[test]
    fn a_48k_wav_opens_at_44k() {
        let tone: Vec<f32> = (0..48_000)
            .map(|i| 0.25 * (0.05 * i as f32).sin())
            .collect();
        let wav = crate::wav::encode_pcm16(&tone, &tone, 48_000);
        let s = Song::load_files(vec![("lloyd.wav".into(), wav)]).unwrap();
        assert_eq!((s.sample_rate, s.frames), (SONG_RATE, 44_100));
        assert_eq!(s.stems[0].left.len(), 44_100);
        assert_eq!(s.stems[0].right.len(), 44_100);
        // Both sides hold the same tone, so both must convert the same way.
        assert!(s.stems[0].right.as_slice() == s.stems[0].left.as_slice());
    }

    #[test]
    fn each_channel_of_a_48k_stem_converts_on_its_own() {
        // The right side is the left at half the level.  Halving is exact
        // in floating point, so the converted right is exactly half the
        // converted left, and a dropped or unconverted right side differs.
        let left: Vec<f32> = (0..4_800).map(|i| 0.25 * (0.05 * i as f32).sin()).collect();
        let right: Vec<f32> = left.iter().map(|v| v * 0.5).collect();
        let s = Song::from_audio(vec![("bass".into(), audio(48_000, vec![left, right]))]).unwrap();
        let bass = &s.stems[0];
        assert_eq!((bass.left.len(), bass.right.len()), (4_410, 4_410));
        for (l, r) in bass.left.iter().zip(bass.right.iter()) {
            assert_eq!((l * 0.5).to_bits(), r.to_bits());
        }
        assert!(
            bass.left.iter().any(|v| v.abs() > 0.1),
            "the left side is not silent"
        );
    }

    #[test]
    fn stems_at_mixed_rates_all_open_at_44k() {
        let s = Song::from_audio(vec![
            ("drums".into(), audio(48_000, vec![vec![0.5; 4_800]])),
            ("bass".into(), audio(22_050, vec![vec![0.25; 2_205]])),
            ("vocals".into(), audio(SONG_RATE, vec![vec![0.1; 4_410]])),
        ])
        .unwrap();
        assert_eq!((s.sample_rate, s.frames), (SONG_RATE, 4_410));
        // Away from the edges, a constant converts to the same constant.
        for (stem, level) in s.stems.iter().zip([0.5, 0.25, 0.1]) {
            assert_eq!(stem.left.len(), 4_410, "{}", stem.name);
            assert!((stem.left[2_205] - level).abs() < 1e-6, "{}", stem.name);
        }
    }

    #[test]
    fn a_44k_stem_is_not_copied() {
        let (left, right) = (vec![0.25, -0.5, 0.75], vec![0.1, 0.2, 0.3]);
        let at = (left.as_ptr(), right.as_ptr());
        let s =
            Song::from_audio(vec![("bass".into(), audio(SONG_RATE, vec![left, right]))]).unwrap();
        let bass = &s.stems[0];
        assert_eq!(bass.left.as_slice().as_ptr(), at.0);
        assert_eq!(bass.right.as_slice().as_ptr(), at.1);
        assert_eq!(bass.left, vec![0.25, -0.5, 0.75]);
        assert_eq!(bass.right, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn a_stem_at_an_odd_rate_is_refused() {
        let e = Song::from_audio(vec![("bass".into(), audio(44_056, vec![vec![0.0; 10]]))])
            .unwrap_err();
        assert!(
            format!("{e:#}").contains("stem bass: songs at 44056 Hz"),
            "{e:#}"
        );
    }

    #[test]
    fn a_mono_stem_shares_one_buffer_and_clones_share_every_buffer() {
        let s = Song::from_audio(vec![
            ("bass".into(), audio(SONG_RATE, vec![vec![0.25; 4]])),
            (
                "drums".into(),
                audio(SONG_RATE, vec![vec![1.0; 4], vec![0.5; 4]]),
            ),
        ])
        .unwrap();
        let (drums, bass) = (&s.stems[0], &s.stems[1]);
        assert!(bass.left.shares(&bass.right), "mono: one buffer");
        assert!(!drums.left.shares(&drums.right));
        let copy = s.clone();
        for (a, b) in s.stems.iter().zip(&copy.stems) {
            assert!(a.left.shares(&b.left) && a.right.shares(&b.right));
        }
    }

    #[test]
    fn samples_from_a_vector_keep_its_buffer() {
        let v = vec![1.0, 2.0];
        let at = v.as_ptr();
        let s = Samples::from(v);
        assert_eq!(s.as_slice().as_ptr(), at);
        assert_eq!(s, vec![1.0, 2.0]);
    }

    #[test]
    fn samples_from_a_vector_with_spare_capacity_keep_none_of_it() {
        let mut v = Vec::with_capacity(1024);
        v.extend_from_slice(&[0.5; 1003]);
        let s = Samples::from(v);
        assert_eq!(s.0.capacity(), 1003);
        assert_eq!(s, vec![0.5; 1003]);
    }

    #[test]
    fn padding_grows_a_full_buffer_by_exactly_the_padding() {
        let padded = padded_to(vec![0.25; 600], 1003);
        assert_eq!(padded.capacity(), 1003);
        assert_eq!(padded[..600], [0.25; 600]);
        assert_eq!(padded[600..], [0.0; 403]);
        assert_eq!(padded_to(vec![0.25; 600], 4), vec![0.25; 4]);
    }

    #[test]
    fn padded_stems_keep_no_spare_capacity() {
        let s = Song::from_audio(vec![
            // Padded from 600 samples to 1,003.
            (
                "bass".into(),
                audio(SONG_RATE, vec![vec![0.25; 600], vec![0.5; 600]]),
            ),
            ("drums".into(), audio(SONG_RATE, vec![vec![1.0; 1003]])),
        ])
        .unwrap();
        for stem in &s.stems {
            assert_eq!(stem.left.0.capacity(), 1003, "{}", stem.name);
            assert_eq!(stem.right.0.capacity(), 1003, "{}", stem.name);
        }
    }
}
