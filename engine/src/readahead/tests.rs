use super::*;
use crate::player::Player;
use crate::song::{Samples, Song, Stem};
use crate::source::MemorySource;
use crate::stretch::{StretcherKind, Varispeed};

const B: u64 = BLOCK_FRAMES;

fn xorshift(seed: u32) -> impl FnMut() -> u32 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    }
}

/// The lanes of a stored song: the files' bytes, and the same song in
/// memory.
struct Lanes {
    formats: Vec<LaneFormat>,
    files: Vec<Vec<u8>>,
    song: Arc<Song>,
}

/// `pcm16` 16-bit lanes and `float` f32 lanes of `frames` frames of
/// noise, at a low level so that their sum does not clip.
fn lanes(frames: usize, pcm16: usize, float: usize) -> Lanes {
    let mut formats = Vec::new();
    let mut files = Vec::new();
    let mut stems = Vec::new();
    for lane in 0..pcm16 + float {
        let mut next = xorshift(lane as u32 + 1);
        let (mut left, mut right) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
        let format = if lane < pcm16 {
            LaneFormat::Pcm16
        } else {
            LaneFormat::F32
        };
        let mut bytes = Vec::with_capacity(frames * format.bytes_per_frame());
        for _ in 0..frames {
            for side in [&mut left, &mut right] {
                let v = next();
                match format {
                    LaneFormat::Pcm16 => {
                        let s = (v as i16) / 8;
                        bytes.extend(s.to_le_bytes());
                        side.push(pcm16_to_f32(s));
                    }
                    LaneFormat::F32 => {
                        let s = (v as f32 / u32::MAX as f32 - 0.5) * 0.2;
                        bytes.extend(s.to_le_bytes());
                        side.push(s);
                    }
                }
            }
        }
        formats.push(format);
        files.push(bytes);
        stems.push(Stem {
            name: format!("lloyd {lane}"),
            left: left.into(),
            right: right.into(),
        });
    }
    Lanes {
        formats,
        files,
        song: Arc::new(Song {
            sample_rate: 44_100,
            frames,
            stems,
        }),
    }
}

/// The fake reader: reads the next block from the files in memory, as the
/// page's reader does from OPFS.  Gives whether it read one.
fn fill_one(ahead: &ReadAhead, files: &[Vec<u8>]) -> bool {
    let Some(read) = ahead.next_read() else {
        return false;
    };
    ahead.fill(read, |lanes| {
        for (lane, bytes) in lanes.iter_mut().enumerate() {
            let per_frame = ahead.lanes()[lane].bytes_per_frame();
            let file = &files[lane];
            let start = (read.first_frame() as usize * per_frame).min(file.len());
            let n = (file.len() - start).min(bytes.len());
            bytes[..n].copy_from_slice(&file[start..start + n]);
            bytes[n..].fill(0);
            if ahead.is_failed(lane) {
                bytes.fill(0);
            }
        }
    });
    true
}

/// A reader that keeps up: reads until it wants nothing more.  It fails
/// if it never stops, as when a block it reads never stays.
fn keep_up(ahead: &ReadAhead, files: &[Vec<u8>]) {
    let mut reads = 0;
    while fill_one(ahead, files) {
        reads += 1;
        assert!(reads <= 1_000, "the reader never stops reading");
    }
}

#[derive(Clone, Copy)]
enum Reader {
    KeepUp,
    Stalled,
}

/// A player of the stored lanes over a `ReadAhead`, and a player of the
/// same song in memory, driven with the same controls.
struct Pair {
    lanes: Lanes,
    ahead: Arc<ReadAhead>,
    stored: Player,
    memory: Player,
    callback: usize,
}

impl Pair {
    fn new(lanes: Lanes, kind: StretcherKind, slots: Option<(usize, usize)>) -> Self {
        let song = Arc::clone(&lanes.song);
        Self::with_memory_song(lanes, song, kind, slots)
    }

    /// A pair whose memory player plays `song`.
    fn with_memory_song(
        lanes: Lanes,
        song: Arc<Song>,
        kind: StretcherKind,
        slots: Option<(usize, usize)>,
    ) -> Self {
        let frames = lanes.song.frames;
        let stored_params = Arc::new(SharedParams::new(lanes.formats.len()));
        let ahead = Arc::new(match slots {
            Some((a, l)) => ReadAhead::with_slots(
                &lanes.formats,
                frames as u64,
                Arc::clone(&stored_params),
                a,
                l,
            ),
            None => ReadAhead::new(&lanes.formats, frames as u64, Arc::clone(&stored_params))
                .expect("the lanes fit"),
        });
        let stored = Player::with_sources(
            44_100,
            frames,
            stored_params,
            kind.build(44_100),
            PlayerConfig::default(),
            ahead.sources(),
        );
        let memory = Player::with_config(
            song,
            Arc::new(SharedParams::new(lanes.formats.len())),
            kind.build(44_100),
            PlayerConfig::default(),
        );
        Self {
            lanes,
            ahead,
            stored,
            memory,
            callback: 512,
        }
    }

    fn both(&self, f: impl Fn(&SharedParams)) {
        f(self.stored.params());
        f(self.memory.params());
    }

    fn stored_params(&self) -> &SharedParams {
        self.stored.params()
    }

    fn read(&self, reader: Reader) {
        if let Reader::KeepUp = reader {
            keep_up(&self.ahead, &self.lanes.files);
        }
    }

    /// Plays `callbacks` callbacks on both, the reader acting before each.
    /// Gives the left channels of the stored player and the memory one.
    fn play(&mut self, callbacks: usize, reader: Reader) -> (Vec<f32>, Vec<f32>) {
        let (mut stored, mut memory) = (Vec::new(), Vec::new());
        for _ in 0..callbacks {
            self.read(reader);
            let [l, r] = self.stored.render(self.callback);
            stored.extend(l.iter().chain(&r));
            let [l, r] = self.memory.render(self.callback);
            memory.extend(l.iter().chain(&r));
        }
        (stored, memory)
    }

    fn held_frames(&self) -> u64 {
        self.stored_params().held_frames.load(Relaxed)
    }
}

#[test]
fn pcm16_converts_as_the_decoder_does() {
    let values: Vec<i16> = (i16::MIN..=i16::MAX).collect();
    let mut wav = Vec::from(crate::wav::header(values.len(), 44_100));
    for v in &values {
        wav.extend(v.to_le_bytes());
        wav.extend(v.to_le_bytes());
    }
    let audio = crate::decode_bytes(wav, Some("wav")).expect("decodes");
    assert_eq!(audio.channels[0].len(), 65_536);
    for (i, v) in values.iter().enumerate() {
        assert_eq!(
            pcm16_to_f32(*v).to_bits(),
            audio.channels[0][i].to_bits(),
            "{v}"
        );
    }
    let bytes = [0x00, 0x80, 0xff, 0x7f];
    assert_eq!(
        frame_at(LaneFormat::Pcm16, &bytes, 0),
        (-1.0, 32_767.0 / 32_768.0)
    );
}

#[test]
fn an_f32_lane_converts_its_bytes() {
    let mut bytes = Vec::new();
    bytes.extend(0.25_f32.to_le_bytes());
    bytes.extend((-0.5_f32).to_le_bytes());
    bytes.extend(1.0_f32.to_le_bytes());
    bytes.extend(f32::MIN_POSITIVE.to_le_bytes());
    assert_eq!(frame_at(LaneFormat::F32, &bytes, 0), (0.25, -0.5));
    assert_eq!(
        frame_at(LaneFormat::F32, &bytes, 1),
        (1.0, f32::MIN_POSITIVE)
    );
}

#[test]
fn a_stored_song_plays_as_the_memory_song() {
    for kind in [StretcherKind::Varispeed, StretcherKind::Wsola] {
        let mut pair = Pair::new(lanes(40 * B as usize, 6, 1), kind, None);
        let check = |pair: &mut Pair, callbacks: usize, what: &str| {
            let (stored, memory) = pair.play(callbacks, Reader::KeepUp);
            assert!(stored == memory, "{}: {what}", kind.label());
        };
        pair.ahead.hint(Hint::Reading { heard: 0, read: 0 });
        pair.both(|p| p.playing.store(true, Relaxed));
        check(&mut pair, 20, "tempo 1");
        pair.both(|p| p.set_tempo(0.5));
        check(&mut pair, 20, "tempo 0.5");
        pair.both(|p| p.set_tempo(1.5));
        check(&mut pair, 20, "tempo 1.5");
        pair.both(|p| p.set_tempo(1.0));
        // A seek to a held position, then a chipmunk change about 1,000
        // frames after a block's start.
        pair.both(|p| p.request_seek(2 * B + 744));
        check(&mut pair, 1, "a seek");
        pair.both(|p| p.chipmunk.store(true, Relaxed));
        check(&mut pair, 20, "chipmunk on");
        pair.both(|p| p.chipmunk.store(false, Relaxed));
        check(&mut pair, 20, "chipmunk off");
        // A loop with a crossfade whose end is 100 frames before a block's
        // end.
        let (s, e) = (5 * B - 20_100, 5 * B - 100);
        pair.both(|p| p.request_seek(s));
        check(&mut pair, 1, "a seek into the loop");
        pair.both(|p| p.set_loop(s, e));
        check(&mut pair, 100, "a loop");
        // A loop shorter than a block.
        pair.both(|p| p.set_loop(5 * B + 100, 5 * B + 3_100));
        check(&mut pair, 100, "a short loop");
        // A loop longer than the loop slots, played round.
        pair.both(|p| p.set_loop(4 * B, 14 * B));
        pair.both(|p| p.set_tempo(1.5));
        pair.callback = 1_024;
        let mut wrapped = false;
        for _ in 0..46 {
            let before = pair.stored_params().playhead();
            check(&mut pair, 10, "a long loop");
            wrapped |= pair.stored_params().playhead() < before;
        }
        assert!(wrapped, "the long loop wrapped");
        pair.callback = 512;
        pair.both(|p| p.set_tempo(1.0));
        // Mute and solo.
        pair.both(SharedParams::clear_loop);
        pair.both(|p| p.set_muted(0, true));
        check(&mut pair, 10, "a mute");
        pair.both(|p| p.set_soloed(6, true));
        check(&mut pair, 10, "a solo");
        pair.both(|p| p.set_soloed(6, false));
        pair.both(|p| p.set_muted(0, false));
        check(&mut pair, 10, "all");
        assert_eq!(pair.ahead.underruns(), 0, "{}", kind.label());
        assert_eq!(pair.held_frames(), 0, "{}", kind.label());
        assert_eq!(pair.stored_params().held_seeks.load(Relaxed), 0);
    }
}

/// The input frames that each `process` read: from the read position at
/// the end of the one before to the one at its end.
fn read_ranges(starts: &[u64]) -> Vec<(u64, u64)> {
    starts.windows(2).map(|w| (w[0], w[1])).collect()
}

#[test]
fn a_slow_reader_underruns_into_silence() {
    let lanes = lanes(12 * B as usize, 6, 1);
    let lane_count = lanes.formats.len() as u64;
    let mut pair = Pair::new(lanes, StretcherKind::Varispeed, None);
    fill_one(&pair.ahead, &pair.lanes.files);
    pair.both(|p| p.playing.store(true, Relaxed));
    // One block for each 100 callbacks of 512 frames: slower than the
    // 64 callbacks that play one block.
    let (mut stored, mut memory) = (Vec::new(), Vec::new());
    // The read position at the end of each callback, which the player
    // hints there.
    let mut starts = vec![0];
    let mut filled_at = [usize::MAX; 12];
    filled_at[0] = 0;
    let callbacks = 600;
    for c in 0..callbacks {
        if c > 0 && c % 100 == 0 {
            let read = pair.ahead.next_read().expect("a block to read");
            filled_at[read.block as usize] = c;
            fill_one(&pair.ahead, &pair.lanes.files);
        }
        let [l, _] = pair.stored.render(512);
        stored.extend(l);
        starts.push(pair.ahead.read_position());
        let [l, _] = pair.memory.render(512);
        memory.extend(l);
    }
    let (missed_lane_frames, underruns_counted) =
        (pair.ahead.missed_lane_frames(), pair.ahead.underruns());
    // At tempo 1, Varispeed maps output frame f to input frame f.  A frame
    // is missing when its block was not held in the callback that read it.
    let ranges = read_ranges(&starts);
    let mut missing = vec![false; stored.len() + 1_024];
    let mut underruns = 0;
    for (c, (start, end)) in ranges.iter().enumerate() {
        let mut missed = false;
        for f in *start..*end {
            if c < filled_at[(f / B) as usize] {
                missing[f as usize] = true;
                missed = true;
            }
        }
        underruns += u64::from(missed);
    }
    let missing_frames = (0..stored.len()).filter(|f| missing[*f]).count() as u64;
    assert!(missing_frames > 0, "the reader was slow enough to miss");
    for f in 0..stored.len() {
        if missing[f] {
            assert_eq!(stored[f], 0.0, "frame {f} is missing, so silent");
        } else {
            assert_eq!(stored[f], memory[f], "frame {f} plays as in memory");
        }
    }
    let read_missing: u64 = ranges
        .iter()
        .flat_map(|(s, e)| *s..*e)
        .filter(|f| missing[*f as usize])
        .count() as u64;
    assert_eq!(missed_lane_frames, read_missing * lane_count);
    assert_eq!(underruns_counted, underruns);
    assert_eq!(pair.held_frames(), 0);
}

#[test]
fn a_stalled_reader_plays_silence_then_resumes() {
    let mut pair = Pair::new(
        lanes(40 * B as usize, 6, 1),
        StretcherKind::Varispeed,
        Some((4, 2)),
    );
    pair.both(|p| p.playing.store(true, Relaxed));
    let (stored, memory) = pair.play(100, Reader::KeepUp);
    assert_eq!(stored, memory);
    // Four slots ahead hold about 3 s: a stall of 5 s runs out.
    let stall = 5 * 44_100 / 512;
    let (stored, memory) = pair.play(stall, Reader::Stalled);
    assert!(pair.ahead.underruns() > 0, "the stall ran out");
    let silent = stored.iter().zip(&memory).filter(|(s, m)| s != m).count();
    assert!(silent > 0);
    assert!(
        stored.iter().zip(&memory).all(|(s, m)| s == m || *s == 0.0),
        "each frame plays as in memory or is silent"
    );
    // It resumes: the reader reads again, and after a moment every frame
    // plays as in memory.
    pair.play(10, Reader::KeepUp);
    let underruns = pair.ahead.underruns();
    let (stored, memory) = pair.play(100, Reader::KeepUp);
    assert_eq!(stored, memory, "it plays again");
    assert_eq!(pair.ahead.underruns(), underruns, "no more underruns");
    assert_eq!(pair.held_frames(), 0);
}

#[test]
fn a_failed_lane_is_silent_and_the_others_play() {
    let stored = lanes(12 * B as usize, 6, 1);
    // The memory player plays lane 1 as silence.
    let mut silent = (*stored.song).clone();
    let zeros: Samples = vec![0.0; silent.frames].into();
    silent.stems[1].left = zeros.clone();
    silent.stems[1].right = zeros;
    let mut pair = Pair::with_memory_song(stored, Arc::new(silent), StretcherKind::Varispeed, None);
    pair.ahead.fail_lane(1);
    pair.ahead.fail_lane(1);
    assert_eq!(pair.ahead.failed_lanes(), 1, "a lane fails once");
    pair.both(|p| p.playing.store(true, Relaxed));
    let (stored, memory) = pair.play(200, Reader::KeepUp);
    assert_eq!(stored, memory);
    assert_eq!(pair.ahead.underruns(), 0);

    // Every stored lane failed, and one lane in memory: a jump to a
    // position that no slot holds does not hold, and the memory lane
    // plays.
    let all = lanes(12 * B as usize, 2, 0);
    let params = Arc::new(SharedParams::new(3));
    let ahead = Arc::new(ReadAhead::new(&all.formats, 12 * B, Arc::clone(&params)).expect("fits"));
    ahead.fail_lane(0);
    ahead.fail_lane(1);
    let memory_lane: Vec<f32> = (0..12 * B).map(|i| (i % 100) as f32 / 1_000.0).collect();
    let memory_lane: Samples = memory_lane.into();
    let mut sources = ahead.sources();
    sources.push(Arc::new(MemorySource::new(
        memory_lane.clone(),
        memory_lane.clone(),
    )));
    let mut player = Player::with_sources(
        44_100,
        12 * B as usize,
        Arc::clone(&params),
        Box::new(Varispeed::new()),
        PlayerConfig::default(),
        sources,
    );
    params.request_seek(9 * B);
    player.render(512);
    params.playing.store(true, Relaxed);
    let [l, _] = player.render(512);
    assert_eq!(params.held_frames.load(Relaxed), 0, "no hold");
    assert_eq!(l[..], memory_lane[9 * B as usize..9 * B as usize + 512]);
    assert_eq!(ahead.underruns(), 0);
    assert_eq!(ahead.failed_lanes(), 2);
}

#[test]
fn after_a_seek_the_old_position_plays_until_the_new_block_arrives() {
    let mut pair = Pair::new(lanes(40 * B as usize, 6, 1), StretcherKind::Wsola, None);
    pair.both(|p| p.playing.store(true, Relaxed));
    pair.play(20, Reader::KeepUp);
    let target = 30 * B + 5_000;
    pair.stored_params().request_seek(target);
    // The reader holds the target's block back for 300 ms of callbacks.
    let withheld = 300 * 44_100 / 1_000 / 512;
    for _ in 0..withheld {
        while let Some(read) = pair.ahead.next_read() {
            if read.block == target / B {
                break;
            }
            fill_one(&pair.ahead, &pair.lanes.files);
        }
        let [l, _] = pair.stored.render(512);
        let [m, _] = pair.memory.render(512);
        assert_eq!(l, m, "the old position plays");
        assert_eq!(pair.stored_params().playhead(), target, "the target shows");
    }
    assert!(!pair.ahead.holds(target / B));
    pair.memory.params().request_seek(target);
    let (stored, memory) = pair.play(20, Reader::KeepUp);
    assert_eq!(stored, memory, "it fades out and plays the target");
    let head = pair.stored_params().playhead();
    assert!((target..target + 20 * 512).contains(&head), "{head}");
    assert_eq!(pair.ahead.underruns(), 0);
    assert_eq!(pair.stored_params().held_seeks.load(Relaxed), 1);
}

#[test]
fn the_wanted_blocks_follow_the_head_the_loop_and_a_seek() {
    let want = Want {
        frames: 40 * B,
        heard: B + 10,
        seek: None,
        looping: None,
        xf: 256,
        ahead: 4,
        loop_slots: 2,
    };
    // The run ahead, from the block of the head.
    assert_eq!(wanted_blocks(&want), [1, 2, 3, 4]);
    // The song's end.
    let end = Want {
        heard: 38 * B,
        ..want
    };
    assert_eq!(wanted_blocks(&end), [38, 39]);
    // A loop that holds the head: the run ahead goes round it, and the
    // loop's blocks come once.
    let held = Want {
        looping: Some((B, 3 * B)),
        ..want
    };
    assert_eq!(wanted_blocks(&held), [1, 2, 3]);
    // A run ahead that reaches the loop's end goes on from its start.
    let round = Want {
        heard: 3 * B,
        looping: Some((0, 4 * B)),
        loop_slots: 1,
        ..want
    };
    assert_eq!(wanted_blocks(&round), [3, 4, 0, 1]);
    // A loop end within the crossfade of a block's end takes in the next
    // block.
    let edge = Want {
        heard: 5 * B,
        looping: Some((5 * B, 6 * B - 100)),
        ..want
    };
    assert_eq!(wanted_blocks(&edge), [5, 6]);
    // A loop ahead of the head: the run ahead, then the loop's start.
    let later = Want {
        heard: 0,
        looping: Some((10 * B, 20 * B)),
        ..want
    };
    assert_eq!(wanted_blocks(&later), [0, 1, 2, 3, 10, 11]);
    // A loop set behind the head: its target is a held seek, first, then
    // the run ahead from the old position, then the loop.
    let behind = Want {
        heard: 30 * B,
        seek: Some(5 * B + 7),
        looping: Some((4 * B, 8 * B)),
        ..want
    };
    assert_eq!(wanted_blocks(&behind), [5, 6, 30, 31, 32, 33, 4]);
    // A seek with a loop set and the head outside it: the seek's blocks
    // come first, so they are kept.
    let seek = Want {
        heard: 20 * B,
        seek: Some(2 * B + 5),
        looping: Some((25 * B, 26 * B)),
        ..want
    };
    let list = wanted_blocks(&seek);
    assert_eq!(list[..2], [2, 3]);
    assert_eq!(list, [2, 3, 20, 21, 22, 23, 25, 26]);
    // Each block is listed once.
    let mut sorted = list.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), list.len());
}

#[test]
fn a_seek_is_read_first_into_a_slot_that_is_not_kept() {
    let all = lanes(40 * B as usize, 1, 0);
    let params = Arc::new(SharedParams::new(1));
    let ahead = ReadAhead::with_slots(&all.formats, 40 * B, Arc::clone(&params), 4, 2);
    params.set_loop(25 * B, 26 * B);
    ahead.hint(Hint::Reading {
        heard: 20 * B,
        read: 20 * B,
    });
    keep_up(&ahead, &all.files);
    for block in [20, 21, 22, 23, 25, 26] {
        assert!(ahead.holds(block), "{block}");
    }
    ahead.hint(Hint::Seek(Some(2 * B + 5)));
    let read = ahead.next_read().expect("the seek's block");
    assert_eq!(read.block, 2);
    keep_up(&ahead, &all.files);
    assert!(ahead.holds(2) && ahead.holds(3));
    assert!(ahead.holds(20) && ahead.holds(23), "the run ahead stays");
}

#[test]
fn the_read_ahead_has_a_fixed_size() {
    let mut formats = vec![LaneFormat::Pcm16; 6];
    formats.push(LaneFormat::F32);
    let params = Arc::new(SharedParams::new(7));
    let minute = ReadAhead::new(&formats, 60 * 44_100, Arc::clone(&params)).expect("fits");
    let long = ReadAhead::new(&formats, 30 * 60 * 44_100, Arc::clone(&params)).expect("fits");
    assert_eq!(minute.bytes(), long.bytes());
    assert_eq!(minute.bytes(), 24 << 20, "1 MiB a slot, 24 slots");
    assert!(long.bytes() <= MAX_READ_AHEAD_BYTES);
    assert_eq!(minute.slot_split(), (16, 8));
    // Lanes that do not fit 24 slots: the most that fit, 2 to 1.
    assert_eq!(ReadAhead::slot_counts(&[LaneFormat::F32; 15]), Some((6, 2)));
    assert_eq!(ReadAhead::slot_counts(&[LaneFormat::F32; 10]), Some((8, 4)));
    // At least 4 and 2, and none when even those do not fit.
    assert_eq!(ReadAhead::slot_counts(&[LaneFormat::F32; 21]), Some((4, 2)));
    assert_eq!(ReadAhead::slot_counts(&[LaneFormat::F32; 22]), None);
    let many = vec![LaneFormat::F32; 15];
    let made = ReadAhead::new(&many, 60 * 44_100, Arc::clone(&params)).expect("fits");
    assert!(made.bytes() <= MAX_READ_AHEAD_BYTES, "{}", made.bytes());
    assert!(ReadAhead::new(&[LaneFormat::F32; 22], 60, params).is_none());
}

#[test]
fn a_paused_seek_into_a_short_loop_then_play_starts() {
    let mut pair = Pair::new(lanes(40 * B as usize, 6, 1), StretcherKind::Wsola, None);
    let (s, e) = (10 * B + 1_000, 10 * B + 5_000);
    pair.both(|p| p.set_loop(s, e));
    pair.both(|p| p.request_seek(e - 100));
    pair.play(1, Reader::Stalled);
    pair.both(|p| p.playing.store(true, Relaxed));
    // The first callback after Play holds: no block is read yet.
    pair.play(1, Reader::Stalled);
    assert_eq!(pair.held_frames(), 512);
    let (stored, _) = pair.play(100, Reader::KeepUp);
    assert!(stored.iter().any(|v| *v != 0.0), "it plays");
    assert!(pair.held_frames() < B, "{}", pair.held_frames());
    assert_eq!(pair.ahead.underruns(), 0);
    let head = pair.stored_params().playhead();
    assert!((s..e).contains(&head), "{head}");
}

#[test]
fn a_held_seek_keeps_the_old_position_in_the_buffer() {
    let mut pair = Pair::new(
        lanes(40 * B as usize, 6, 1),
        StretcherKind::Varispeed,
        Some((4, 0)),
    );
    pair.both(|p| p.playing.store(true, Relaxed));
    pair.play(100, Reader::KeepUp);
    pair.read(Reader::KeepUp);
    assert!(pair.ahead.next_read().is_none());
    assert!(
        pair.ahead
            .slots
            .iter()
            .all(|s| s.block.load(Relaxed) != NO_BLOCK),
        "every slot holds a block"
    );
    let target = 30 * B + 77;
    pair.stored_params().request_seek(target);
    // One callback with no read: the seek holds, and the old position
    // plays.  Then the reader reads on.
    let (stored, memory) = pair.play(1, Reader::Stalled);
    assert_eq!(stored, memory);
    pair.memory.params().request_seek(target);
    let (stored, memory) = pair.play(20, Reader::KeepUp);
    assert_eq!(stored, memory, "the old position, then the target");
    assert_eq!(pair.ahead.underruns(), 0);
    let head = pair.stored_params().playhead();
    assert!((target..target + 20 * 512).contains(&head), "{head}");
}

#[test]
fn a_loop_set_behind_the_head_plays_on_until_its_target_is_read() {
    let mut pair = Pair::new(
        lanes(40 * B as usize, 6, 1),
        StretcherKind::Varispeed,
        Some((4, 2)),
    );
    pair.both(|p| p.request_seek(20 * B));
    pair.play(1, Reader::KeepUp);
    pair.both(|p| p.playing.store(true, Relaxed));
    pair.play(50, Reader::KeepUp);
    let head = pair.stored.read_position();
    let (s, e) = (8 * B + 3, 11 * B);
    let target = dsp::wrap_loop(head as usize, s as usize, e as usize) as u64;
    assert!(!pair.ahead.holds(target / B), "the target is not read yet");
    pair.stored_params().set_loop(s, e);
    let (stored, memory) = pair.play(3, Reader::Stalled);
    assert_eq!(stored, memory, "the old position plays on");
    assert_eq!(pair.stored_params().playhead(), target);
    assert_eq!(pair.stored_params().held_seeks.load(Relaxed), 1);
    pair.memory.params().request_seek(target);
    pair.memory.params().set_loop(s, e);
    let (stored, memory) = pair.play(200, Reader::KeepUp);
    assert_eq!(
        stored, memory,
        "then it fades out, jumps and takes the loop"
    );
    let head = pair.stored_params().playhead();
    assert!((s..e).contains(&head), "{head}");
    assert_eq!(pair.ahead.underruns(), 0);
    assert_eq!(pair.held_frames(), 0);
}

#[test]
fn a_held_slot_never_blocks_the_callback() {
    let all = lanes(4 * B as usize, 1, 1);
    let params = Arc::new(SharedParams::new(2));
    let ahead = Arc::new(ReadAhead::new(&all.formats, 4 * B, params).expect("fits"));
    keep_up(&ahead, &all.files);
    assert!(ahead.holds(0));
    let source = &ahead.sources()[0];
    let (mut l, mut r) = (vec![9.0; 64], vec![9.0; 64]);
    source.read(0, &mut l, &mut r);
    assert!(l.iter().any(|v| *v != 0.0), "the block plays");
    let slot = ahead.held_slot(0).expect("held");
    let held = slot.data.lock().expect("lock");
    // The slot's lock is held, and its number still says block 0: the
    // read, on another thread as the audio callback is, returns at once
    // with silence and counts the frames.
    source.hint(Hint::Reading { heard: 0, read: 0 });
    let (tx, rx) = std::sync::mpsc::channel();
    let audio = Arc::clone(source);
    let thread = std::thread::spawn(move || {
        let (mut l, mut r) = (vec![9.0; 64], vec![9.0; 64]);
        audio.read(0, &mut l, &mut r);
        tx.send((l, r))
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    drop(held);
    thread.join().expect("joined").expect("sent");
    let (l2, r2) = got.expect("the read returns while the slot is held");
    assert!(l2.iter().chain(&r2).all(|v| *v == 0.0));
    assert_eq!(ahead.underruns(), 1);
    assert_eq!(ahead.missed_lane_frames(), 64);
    source.read(0, &mut l, &mut r);
    assert!(
        l.iter().any(|v| *v != 0.0),
        "it plays once the lock is free"
    );
}

#[test]
fn a_block_that_fill_writes_is_not_read() {
    let all = lanes(4 * B as usize, 1, 0);
    let params = Arc::new(SharedParams::new(1));
    let ahead = Arc::new(ReadAhead::new(&all.formats, 4 * B, params).expect("fits"));
    let source = Arc::clone(&ahead.sources()[0]);
    let read = ahead.next_read().expect("a block");
    ahead.fill(read, |lanes| {
        lanes[0].fill(1);
        // Inside `fill`, the slot holds no block yet.
        assert!(!source.ready(0, 10));
        let (mut l, mut r) = ([9.0; 4], [9.0; 4]);
        source.read(0, &mut l, &mut r);
        assert_eq!(l, [0.0; 4]);
    });
    assert!(source.ready(0, 10));
    // The slot of block 0 takes block 1: while `fill` writes it, the slot
    // holds neither.
    let slot = ahead
        .slots
        .iter()
        .position(|s| s.block.load(Relaxed) == 0)
        .expect("block 0 is held");
    ahead.fill(BlockRead { block: 1, slot }, |lanes| {
        lanes[0].fill(2);
        assert!(!ahead.holds(0), "block 0 is no longer held");
        assert!(!source.ready(0, 10));
        assert!(!ahead.holds(1));
    });
    assert!(source.ready(B, 10) && !source.ready(0, 10));
}

#[test]
fn a_slot_whose_block_changed_is_not_read() {
    let all = lanes(4 * B as usize, 1, 0);
    let params = Arc::new(SharedParams::new(1));
    let ahead = Arc::new(ReadAhead::new(&all.formats, 4 * B, params).expect("fits"));
    keep_up(&ahead, &all.files);
    let source = Arc::clone(&ahead.sources()[0]);
    // The callback found block 0 by its number, and then a fill began in
    // that slot: under the lock, the slot holds no block.
    let slot = ahead.held_slot(0).expect("held");
    slot.data.lock().expect("lock").block = None;
    let (mut l, mut r) = ([9.0; 4], [9.0; 4]);
    source.read(0, &mut l, &mut r);
    assert_eq!(l, [0.0; 4]);
    assert_eq!(ahead.missed_lane_frames(), 4);
}

/// Review round 1 of M6, finding 1-10: the buffer counts the lanes that
/// the player read since the last count, so the page can show that a
/// lane it unmuted plays.
#[test]
fn the_lanes_read_since_the_last_count_are_counted() {
    let all = lanes(4 * B as usize, 2, 1);
    let params = Arc::new(SharedParams::new(3));
    let ahead = Arc::new(ReadAhead::new(&all.formats, 4 * B, params).expect("fits"));
    let sources = ahead.sources();
    assert_eq!(ahead.take_lanes_read(), 0);
    let (mut l, mut r) = ([0.0; 8], [0.0; 8]);
    sources[0].read(0, &mut l, &mut r);
    sources[2].read(0, &mut l, &mut r);
    sources[2].read(8, &mut l, &mut r);
    assert_eq!(ahead.take_lanes_read(), 2);
    assert_eq!(ahead.take_lanes_read(), 0, "counted once");
    ahead.fail_lane(1);
    sources[1].read(0, &mut l, &mut r);
    assert_eq!(ahead.take_lanes_read(), 1, "a failed lane read counts");
}

/// Review round 3 of M6, finding 3-2: the slot that `next_read` gives the
/// reader no longer holds its old block, so a seek into that block holds
/// until a fill gives the block again, and nothing plays from a slot that
/// is being filled.
#[test]
fn a_seek_into_a_slot_being_refilled_holds() {
    let mut pair = Pair::new(
        lanes(40 * B as usize, 1, 0),
        StretcherKind::Varispeed,
        Some((2, 1)),
    );
    pair.both(|p| p.playing.store(true, Relaxed));
    // Past block 1, so all three slots hold blocks.
    pair.play(100, Reader::KeepUp);
    // Play on with no reads until the reader would refill a slot that
    // holds a block.
    let mut picked = None;
    for _ in 0..200 {
        pair.play(1, Reader::Stalled);
        let before: Vec<u64> = pair
            .ahead
            .slots
            .iter()
            .map(|s| s.block.load(Relaxed))
            .collect();
        if let Some(read) = pair.ahead.next_read() {
            let old = before[read.slot];
            if old != NO_BLOCK && old != read.block {
                picked = Some((read, old));
                break;
            }
        }
    }
    let (read, old) = picked.expect("the reader refills a slot that holds a block");
    let target = old * B + 100;
    let run = next_run(target, 40 * B, None, 256);
    let ready = |pair: &Pair| {
        run.parts()
            .all(|(s, e)| (s / B..e.div_ceil(B)).all(|b| pair.ahead.holds(b)))
    };
    pair.stored_params().request_seek(target);
    let mut seeked = false;
    let mut same = true;
    for step in 0..40 {
        if step == 1 {
            // The reader's read of `read.block` ends now.
            let files = pair.lanes.files.clone();
            pair.ahead.fill(read, |lanes| {
                let start = read.first_frame() as usize * 4;
                let n = (files[0].len() - start).min(lanes[0].len());
                lanes[0][..n].copy_from_slice(&files[0][start..start + n]);
                lanes[0][n..].fill(0);
            });
        } else if step > 1 {
            pair.read(Reader::KeepUp);
        }
        // The memory player seeks when the stored target's run is ready.
        if !seeked && ready(&pair) {
            pair.memory.params().request_seek(target);
            seeked = true;
        }
        let [l, _] = pair.stored.render(pair.callback);
        let [m, _] = pair.memory.render(pair.callback);
        same &= l == m;
    }
    assert!(seeked, "the target was read");
    assert_eq!(
        pair.ahead.underruns(),
        0,
        "nothing played from a slot being filled"
    );
    assert!(same, "the old position, then the target, as in memory");
}
