//! The pass on a session in memory (`cargo test -p decode-worker`).

#![expect(
    clippy::disallowed_methods,
    reason = "MEM-1: test code, which does not ship"
)]

use super::*;
use crate::source::BlockSource;
use engine::Song;
use engine::decode::{Audio, decode_bytes};
use engine::overview::lane_columns;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use stem_protocol::decode::Source;

const SESSION: &str = "sessions/1";

/// Runs a future that never waits, as every future of [`Memory`] is.
fn ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(out) => out,
        std::task::Poll::Pending => panic!("a session in memory never waits"),
    }
}

type Files = Rc<RefCell<BTreeMap<String, Vec<u8>>>>;

/// A session's files in memory, with failures to order.
#[derive(Default)]
struct Memory {
    files: Files,
    /// Source reads fail from this byte on, in the source at this path.
    fail_reads: Option<(String, u64)>,
    /// The write that fails, counted from 1 over every part file.
    fail_write: Option<usize>,
    /// Each close of a part file fails.
    fail_close: bool,
    writes: Rc<Cell<usize>>,
    /// The size of each write, in order.
    write_sizes: Rc<RefCell<Vec<usize>>>,
    /// The largest read of a part or stored file.
    largest_read: Rc<Cell<usize>>,
    /// The paths of the sources that the pass opened.
    sources: Vec<String>,
}

struct MemoryPart {
    path: String,
    files: Files,
    fail_write: Option<usize>,
    writes: Rc<Cell<usize>>,
    write_sizes: Rc<RefCell<Vec<usize>>>,
    largest_read: Rc<Cell<usize>>,
}

impl PartFile for MemoryPart {
    fn write_at(&mut self, at: u64, bytes: &[u8]) -> Result<(), String> {
        self.writes.set(self.writes.get() + 1);
        if self.fail_write == Some(self.writes.get()) {
            return Err("QuotaExceededError: refused by the test".into());
        }
        self.write_sizes.borrow_mut().push(bytes.len());
        let mut files = self.files.borrow_mut();
        let file = files.get_mut(&self.path).expect("the part file exists");
        let end = at as usize + bytes.len();
        if file.len() < end {
            file.resize(end, 0);
        }
        file[at as usize..end].copy_from_slice(bytes);
        Ok(())
    }

    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> Result<(), String> {
        self.largest_read
            .set(self.largest_read.get().max(buf.len()));
        let files = self.files.borrow();
        let file = &files[&self.path];
        let range = at as usize..at as usize + buf.len();
        buf.copy_from_slice(file.get(range).ok_or("a read past the end")?);
        Ok(())
    }
}

struct MemoryStored {
    bytes: Vec<u8>,
    largest_read: Rc<Cell<usize>>,
}

impl StoredFile for MemoryStored {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> Result<(), String> {
        self.largest_read
            .set(self.largest_read.get().max(buf.len()));
        let range = at as usize..at as usize + buf.len();
        buf.copy_from_slice(self.bytes.get(range).ok_or("a read past the end")?);
        Ok(())
    }
}

type Reads = Box<dyn FnMut(u64, &mut [u8]) -> Result<(), String> + Send + Sync>;

impl Session for Memory {
    type Part = MemoryPart;
    type Stored = MemoryStored;

    async fn source(
        &mut self,
        path: &str,
        failure: ReadFailure,
    ) -> Result<Box<dyn MediaSource>, String> {
        self.sources.push(path.into());
        let bytes = self
            .files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| format!("NotFoundError: no file {path}"))?;
        let fail_from = match &self.fail_reads {
            Some((at, from)) if at == path => *from,
            _ => u64::MAX,
        };
        let len = bytes.len() as u64;
        let read: Reads = Box::new(move |at: u64, buf: &mut [u8]| {
            if at + buf.len() as u64 > fail_from {
                return Err("NotReadableError: refused by the test".into());
            }
            buf.copy_from_slice(&bytes[at as usize..at as usize + buf.len()]);
            Ok(())
        });
        Ok(Box::new(BlockSource::new(len, read, failure)))
    }

    async fn stored(&mut self, path: &str) -> Result<MemoryStored, String> {
        let bytes = self.files.borrow().get(path).cloned();
        Ok(MemoryStored {
            bytes: bytes.ok_or_else(|| format!("NotFoundError: no file {path}"))?,
            largest_read: self.largest_read.clone(),
        })
    }

    async fn create(&mut self, path: &str) -> Result<MemoryPart, String> {
        self.files.borrow_mut().insert(path.into(), Vec::new());
        Ok(MemoryPart {
            path: path.into(),
            files: self.files.clone(),
            fail_write: self.fail_write,
            writes: self.writes.clone(),
            write_sizes: self.write_sizes.clone(),
            largest_read: self.largest_read.clone(),
        })
    }

    async fn close(&mut self, _: MemoryPart) -> Result<(), String> {
        if self.fail_close {
            return Err("InvalidStateError: refused by the test".into());
        }
        Ok(())
    }

    async fn remove(&mut self, path: &str) {
        self.files.borrow_mut().remove(path);
    }
}

impl Memory {
    fn plant(&self, path: &str, bytes: Vec<u8>) {
        self.files.borrow_mut().insert(path.into(), bytes);
    }

    fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.files
            .borrow()
            .get(&format!("{SESSION}/{name}"))
            .cloned()
    }

    fn part(&self, name: &str) -> Vec<u8> {
        self.file(&format!("{name}{PART}"))
            .unwrap_or_else(|| panic!("no part file {name}"))
    }

    /// The names of the part files, from the session folder.
    fn parts(&self) -> Vec<String> {
        self.files
            .borrow()
            .keys()
            .filter(|path| path.ends_with(PART))
            .map(|path| path[SESSION.len() + 1..path.len() - PART.len()].to_string())
            .collect()
    }

    fn run(&mut self, req: &DecodeRequest) -> DecodeReply {
        ready(run(self, req, &mut Buffers::default()))
    }
}

/// A WAV of `channels` at `rate`: 16-bit, or 32-bit float if `float`.
fn wav(channels: &[Vec<f32>], rate: u32, float: bool) -> Vec<u8> {
    let (format, bits): (u16, u16) = if float { (3, 32) } else { (1, 16) };
    let count = channels.len() as u16;
    let frames = channels[0].len();
    let block = count * bits / 8;
    let data = (frames * usize::from(block)) as u32;
    let mut out = Vec::new();
    for field in [
        &b"RIFF"[..],
        &(36 + data).to_le_bytes(),
        b"WAVEfmt ",
        &16_u32.to_le_bytes(),
        &format.to_le_bytes(),
        &count.to_le_bytes(),
        &rate.to_le_bytes(),
        &(rate * u32::from(block)).to_le_bytes(),
        &block.to_le_bytes(),
        &bits.to_le_bytes(),
        b"data",
        &data.to_le_bytes(),
    ] {
        out.extend_from_slice(field);
    }
    for i in 0..frames {
        for channel in channels {
            if float {
                out.extend_from_slice(&channel[i].to_le_bytes());
            } else {
                let step = (channel[i] * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
                out.extend_from_slice(&step.to_le_bytes());
            }
        }
    }
    out
}

/// A tone of `frames` frames, `level` loud, different for each `seed`.
fn tone(frames: usize, level: f32, seed: usize) -> Vec<f32> {
    let step = 0.01 * (seed + 1) as f32;
    (0..frames)
        .map(|i| level * (step * i as f32).sin() + 0.01)
        .collect()
}

/// A source in `src/<index>.wav`, for the request.
fn source(index: u32, in_mix: bool) -> Source {
    Source {
        file: format!("src/{index}.wav"),
        index,
        ext: "wav".into(),
        in_mix,
    }
}

fn request(sources: Vec<Source>, mix_frames: Option<u64>) -> DecodeRequest {
    DecodeRequest {
        session: SESSION.into(),
        sources,
        mix_frames,
    }
}

/// A session with these WAVs as `src/0.wav`, `src/1.wav` and so on.
fn session_of(wavs: &[Vec<u8>]) -> Memory {
    let memory = Memory::default();
    for (i, bytes) in wavs.iter().enumerate() {
        memory.plant(&format!("{SESSION}/src/{i}.wav"), bytes.clone());
    }
    memory
}

/// The song that `Song::from_audio` makes of these WAVs, named so that
/// it keeps their order.
fn song_of(wavs: &[Vec<u8>]) -> Song {
    let audio: Vec<(String, Audio)> = wavs
        .iter()
        .enumerate()
        .map(|(i, bytes)| {
            let audio = decode_bytes(bytes.clone(), Some("wav")).unwrap();
            (format!("source {i}"), audio)
        })
        .collect();
    Song::from_audio(audio).unwrap()
}

/// Two channels as the bytes of a session file.
fn interleaved(left: &[f32], right: &[f32]) -> Vec<u8> {
    left.iter()
        .zip(right)
        .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
        .flatten()
        .collect()
}

/// Columns as the bytes of an overview file.
fn overview_bytes(cols: &[(f32, f32)]) -> Vec<u8> {
    cols.iter()
        .flat_map(|(lo, hi)| [lo.to_le_bytes(), hi.to_le_bytes()])
        .flatten()
        .collect()
}

/// The mix of the sources in the mix, as `mix_of_sources` makes it.
fn mix_of(song: &Song, in_mix: &[bool]) -> (Vec<f32>, Vec<f32>) {
    let stems: Vec<&engine::Stem> = song
        .stems
        .iter()
        .zip(in_mix)
        .filter(|(_, m)| **m)
        .map(|(s, _)| s)
        .collect();
    if stems.is_empty() {
        return (vec![0.0; song.frames], vec![0.0; song.frames]);
    }
    let ones = vec![1.0; stems.len()];
    let lefts: Vec<&[f32]> = stems.iter().map(|s| s.left.as_slice()).collect();
    let rights: Vec<&[f32]> = stems.iter().map(|s| s.right.as_slice()).collect();
    (dsp::mix(&lefts, &ones), dsp::mix(&rights, &ones))
}

#[test]
fn one_source_decodes_into_the_mix() {
    // Louder than full scale, so the mix lane (clamped) and the source
    // lane (not clamped) differ.
    for (channels, want) in [(2, 2), (1, 1)] {
        let tones: Vec<Vec<f32>> = (0..channels).map(|c| tone(10_007, 1.5, c)).collect();
        let wavs = [wav(&tones, 48_000, true)];
        let mut memory = session_of(&wavs);
        let reply = memory.run(&request(vec![source(0, true)], None));
        let song = song_of(&wavs);
        let stem = &song.stems[0];
        assert_eq!(
            reply,
            DecodeReply::Decoded {
                frames: song.frames as u64,
                new_mix: true,
                channels: vec![want],
            }
        );
        assert!(memory.part("mix.f32") == interleaved(&stem.left, &stem.right));
        assert_eq!(
            memory.parts(),
            ["mix.f32", "overviews/0.f32", "overviews/mix.f32"]
        );
        let (cols, _) = lane_columns(&stem.left, &stem.right);
        assert!(memory.part("overviews/0.f32") == overview_bytes(&cols));
        let (ml, mr) = mix_of(&song, &[true]);
        let (mix_cols, _) = lane_columns(&ml, &mr);
        assert!(memory.part("overviews/mix.f32") == overview_bytes(&mix_cols));
        assert_ne!(
            cols, mix_cols,
            "the source passes full scale; the mix is clamped"
        );
    }
}

#[test]
fn several_sources_decode_into_padded_files_and_a_mix() {
    let wavs = [
        wav(&[tone(8_000, 0.6, 0), tone(8_000, 0.5, 1)], 44_100, false),
        wav(&[tone(9_000, 0.7, 2)], 48_000, false),
        wav(&[tone(3_000, 0.4, 3), tone(3_000, 0.3, 4)], 22_050, true),
    ];
    let mut memory = session_of(&wavs);
    let sources = vec![source(0, true), source(1, true), source(2, true)];
    let reply = memory.run(&request(sources, None));
    let song = song_of(&wavs);
    assert_eq!(
        reply,
        DecodeReply::Decoded {
            frames: song.frames as u64,
            new_mix: true,
            channels: vec![2, 1, 2],
        }
    );
    for (i, stem) in song.stems.iter().enumerate() {
        let file = memory.part(&format!("decoded/{i}.f32"));
        assert!(file == interleaved(&stem.left, &stem.right), "source {i}");
        let (cols, _) = lane_columns(&stem.left, &stem.right);
        let overview = memory.part(&format!("overviews/{i}.f32"));
        assert!(overview == overview_bytes(&cols), "source {i}");
    }
    let (ml, mr) = mix_of(&song, &[true, true, true]);
    assert!(memory.part("mix.f32") == interleaved(&ml, &mr));
    assert!(
        ml.iter().any(|x| x.abs() == 1.0),
        "the mix is clamped somewhere"
    );
    let (cols, _) = lane_columns(&ml, &mr);
    assert!(memory.part("overviews/mix.f32") == overview_bytes(&cols));
}

#[test]
fn a_derived_source_stays_out_of_the_mix_and_the_song_has_the_longest_frames() {
    // `drums` is in the mix and shorter; `classic drums` is derived.
    let wavs = [
        wav(&[tone(5_000, 1.5, 0), tone(5_000, 1.2, 1)], 44_100, true),
        wav(&[tone(7_000, 0.5, 2), tone(7_000, 0.5, 3)], 44_100, false),
    ];
    let mut memory = session_of(&wavs);
    let reply = memory.run(&request(vec![source(0, true), source(1, false)], None));
    let song = song_of(&wavs);
    assert_eq!(song.frames, 7_000);
    assert_eq!(
        reply,
        DecodeReply::Decoded {
            frames: 7_000,
            new_mix: true,
            channels: vec![2, 2],
        }
    );
    let drums = &song.stems[0];
    assert!(
        memory.part("mix.f32") == interleaved(&drums.left, &drums.right),
        "the mix is drums alone, padded, not clamped"
    );
    let classic = &song.stems[1];
    assert!(memory.part("decoded/1.f32") == interleaved(&classic.left, &classic.right));
    assert!(memory.file(&format!("decoded/0.f32{PART}")).is_none());
    let (ml, mr) = mix_of(&song, &[true, false]);
    let (cols, _) = lane_columns(&ml, &mr);
    assert!(memory.part("overviews/mix.f32") == overview_bytes(&cols));
}

#[test]
fn a_song_with_no_source_in_the_mix_has_a_silent_mix() {
    let wavs = [wav(&[tone(4_000, 0.5, 0)], 44_100, false)];
    let mut memory = session_of(&wavs);
    let reply = memory.run(&request(vec![source(0, false)], None));
    assert_eq!(
        reply,
        DecodeReply::Decoded {
            frames: 4_000,
            new_mix: true,
            channels: vec![1],
        }
    );
    assert!(memory.part("mix.f32") == vec![0; 8 * 4_000]);
    let (cols, _) = lane_columns(&[0.0; 4_000], &[0.0; 4_000]);
    assert!(memory.part("overviews/mix.f32") == overview_bytes(&cols));
}

#[test]
fn a_usable_mix_is_not_written_again() {
    // A song of one source: the mix and a derived source, with no file
    // for the source in the mix, so a read of it would fail.  The second
    // case is a mono mix of M4: each frame's sides are equal.
    for (mono, channels) in [(false, 2), (true, 1)] {
        let left = tone(6_000, 0.5, 0);
        let right = if mono {
            left.clone()
        } else {
            tone(6_000, 0.5, 1)
        };
        let kept_mix = interleaved(&left, &right);
        let derived = wav(&[tone(4_000, 0.4, 2)], 44_100, false);
        let mut memory = Memory::default();
        memory.plant(&format!("{SESSION}/mix.f32"), kept_mix.clone());
        memory.plant(&format!("{SESSION}/src/1.wav"), derived.clone());
        let sources = vec![source(0, true), source(1, false)];
        let reply = memory.run(&request(sources, Some(6_000)));
        assert_eq!(
            reply,
            DecodeReply::Decoded {
                frames: 6_000,
                new_mix: false,
                channels: vec![channels, 1],
            },
            "mono: {mono}"
        );
        assert_eq!(
            memory.sources,
            [format!("{SESSION}/src/1.wav")],
            "no source in the mix is read"
        );
        assert_eq!(memory.file("mix.f32"), Some(kept_mix), "the mix stays");
        assert_eq!(
            memory.parts(),
            [
                "decoded/1.f32",
                "overviews/0.f32",
                "overviews/1.f32",
                "overviews/mix.f32"
            ]
        );
        let derived_song = song_of(&[derived]);
        let stem = &derived_song.stems[0];
        let mut padded = interleaved(&stem.left, &stem.right);
        padded.resize(8 * 6_000, 0);
        assert!(memory.part("decoded/1.f32") == padded);
        let (cols, _) = lane_columns(&left, &right);
        assert!(memory.part("overviews/0.f32") == overview_bytes(&cols));
    }
    // A song of several sources decodes its sources and writes no mix.
    let wavs = [
        wav(&[tone(5_000, 0.5, 0)], 44_100, false),
        wav(&[tone(3_000, 0.5, 1)], 44_100, false),
    ];
    let mut memory = session_of(&wavs);
    memory.plant(&format!("{SESSION}/mix.f32"), vec![7; 8 * 5_000]);
    let reply = memory.run(&request(
        vec![source(0, true), source(1, true)],
        Some(5_000),
    ));
    assert_eq!(
        reply,
        DecodeReply::Decoded {
            frames: 5_000,
            new_mix: false,
            channels: vec![1, 1],
        }
    );
    assert!(memory.file(&format!("mix.f32{PART}")).is_none());
    assert_eq!(memory.file("mix.f32"), Some(vec![7; 8 * 5_000]));
}

/// No part file of the decode is left.
fn no_part_is_left(memory: &Memory, case: &str) {
    assert_eq!(memory.parts(), Vec::<String>::new(), "{case}");
}

fn is_unstored(reply: &DecodeReply, containing: &str) -> bool {
    matches!(reply, DecodeReply::Unstored { text } if text.contains(containing))
}

#[test]
fn a_mix_of_other_frames_ends_with_unstored() {
    // Several sources whose frames are not the kept mix's.
    let wavs = [
        wav(&[tone(5_000, 0.5, 0)], 44_100, false),
        wav(&[tone(3_000, 0.5, 1)], 44_100, false),
    ];
    for kept in [4_999, 5_001] {
        let mut memory = session_of(&wavs);
        memory.plant(&format!("{SESSION}/mix.f32"), vec![0; 8 * kept]);
        let sources = vec![source(0, true), source(1, true)];
        let reply = memory.run(&request(sources, Some(kept as u64)));
        assert!(
            is_unstored(&reply, "frames of the kept mix"),
            "{kept}: {reply:?}"
        );
        no_part_is_left(&memory, &format!("{kept}"));
    }
    // One source, with a derived source longer than the kept mix.
    let mut memory = Memory::default();
    memory.plant(&format!("{SESSION}/mix.f32"), vec![0; 8 * 2_000]);
    memory.plant(&format!("{SESSION}/src/1.wav"), wavs[0].clone());
    let reply = memory.run(&request(
        vec![source(0, true), source(1, false)],
        Some(2_000),
    ));
    assert!(
        is_unstored(&reply, "more than the 2000 frames"),
        "{reply:?}"
    );
    no_part_is_left(&memory, "a long derived source");
    // A kept mix file of another size than its frames.
    let mut memory = Memory::default();
    memory.plant(&format!("{SESSION}/mix.f32"), vec![0; 8 * 2_000 + 4]);
    let reply = memory.run(&request(vec![source(0, true)], Some(2_000)));
    assert!(
        is_unstored(&reply, "the mix file holds 16004 bytes"),
        "{reply:?}"
    );
    no_part_is_left(&memory, "a mix file of the wrong size");
}

#[test]
fn a_bad_source_ends_with_undecodable_and_leaves_no_part_file() {
    let good = wav(&[tone(3_000, 0.5, 0)], 44_100, false);
    for (bad, why) in [
        (
            b"not audio at all, lloyd".to_vec(),
            "unsupported or unrecognised format",
        ),
        (wav(&[Vec::new()], 44_100, false), "no audio"),
        (
            wav(&[tone(100, 0.5, 0)], 44_056, false),
            "44056 Hz are not supported",
        ),
    ] {
        let mut memory = session_of(&[good.clone(), bad]);
        // A stale part file of an earlier decode goes too.
        memory.plant(&format!("{SESSION}/src/7.wav"), good.clone());
        memory.plant(&format!("{SESSION}/overviews/7.f32{PART}"), vec![1]);
        let sources = vec![source(0, true), source(1, true), source(7, false)];
        let reply = memory.run(&request(sources, None));
        assert!(
            matches!(&reply, DecodeReply::Undecodable { text } if text.contains(why)),
            "{why}: {reply:?}"
        );
        no_part_is_left(&memory, why);
    }
}

#[test]
fn a_failed_write_ends_with_unstored_and_leaves_no_part_file() {
    // More than a block, so the first pass, the padding, the mix and the
    // overviews each write.
    let wavs = [
        wav(&[tone(150_000, 0.5, 0)], 44_100, false),
        wav(&[tone(10_000, 0.5, 1)], 44_100, false),
    ];
    let sources = || vec![source(0, true), source(1, true)];
    let mut all = session_of(&wavs);
    let reply = all.run(&request(sources(), None));
    assert!(matches!(reply, DecodeReply::Decoded { .. }), "{reply:?}");
    let writes = all.writes.get();
    assert!(writes >= 8, "{writes} writes");
    for fail in 1..=writes {
        let mut memory = session_of(&wavs);
        memory.fail_write = Some(fail);
        let reply = memory.run(&request(sources(), None));
        assert!(
            is_unstored(&reply, "QuotaExceededError"),
            "write {fail}: {reply:?}"
        );
        no_part_is_left(&memory, &format!("write {fail}"));
    }
}

/// Finding 2-4 of review round 2: a part file that does not close ends a
/// decode that worked with `decode-unstored`, and leaves no part file.
#[test]
fn a_failed_close_ends_with_unstored_and_leaves_no_part_file() {
    let mut memory = session_of(&[wav(&[tone(3_000, 0.5, 0)], 44_100, false)]);
    memory.fail_close = true;
    let reply = memory.run(&request(vec![source(0, true)], None));
    assert!(
        is_unstored(&reply, "could not be closed") && is_unstored(&reply, "InvalidStateError"),
        "{reply:?}"
    );
    no_part_is_left(&memory, "a failed close");
}

#[test]
fn a_read_that_fails_ends_with_unstored_in_the_probe_and_in_the_middle() {
    let wavs = [
        wav(&[tone(3_000, 0.5, 0)], 44_100, false),
        wav(&[tone(30_000, 0.5, 1)], 44_100, false),
    ];
    for (fail_from, case) in [(0, "the probe"), (10, "the header"), (30_000, "the middle")] {
        let mut memory = session_of(&wavs);
        memory.fail_reads = Some((format!("{SESSION}/src/1.wav"), fail_from));
        let reply = memory.run(&request(vec![source(0, true), source(1, true)], None));
        assert!(
            is_unstored(&reply, "src/1.wav: bytes") && is_unstored(&reply, "NotReadableError"),
            "{case}: {reply:?}"
        );
        no_part_is_left(&memory, case);
    }
    // A missing source is a storage failure too.
    let mut memory = session_of(&wavs[..1]);
    let reply = memory.run(&request(vec![source(0, true), source(1, true)], None));
    assert!(is_unstored(&reply, "NotFoundError"), "{reply:?}");
    no_part_is_left(&memory, "a missing source");
}

#[test]
fn the_pass_writes_and_reads_in_blocks_of_at_most_1_mib() {
    let wavs = [
        wav(&[tone(300_000, 0.5, 0)], 44_100, false),
        wav(&[tone(10_000, 0.5, 1)], 44_100, false),
    ];
    let mut memory = session_of(&wavs);
    let reply = memory.run(&request(vec![source(0, true), source(1, true)], None));
    assert!(
        matches!(
            reply,
            DecodeReply::Decoded {
                frames: 300_000,
                ..
            }
        ),
        "{reply:?}"
    );
    let sizes = memory.write_sizes.borrow();
    let block = BLOCK_FRAMES * 8;
    assert!(sizes.iter().all(|size| *size <= block), "{sizes:?}");
    let whole_blocks = sizes.iter().filter(|size| **size == block).count();
    assert!(whole_blocks >= 3, "{sizes:?}");
    assert_eq!(memory.largest_read.get(), block);
}

#[test]
fn no_source_is_undecodable() {
    let mut memory = Memory::default();
    let reply = memory.run(&request(Vec::new(), None));
    assert_eq!(
        reply,
        DecodeReply::Undecodable {
            text: "no stems to load".into()
        }
    );
}
