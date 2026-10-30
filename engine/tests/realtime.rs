//! RT-1: after warm-up, the audio callback neither allocates nor frees
//! memory, for the stretchers the app ships.  It plays the stems from memory,
//! and as stored lanes over a `ReadAhead` that a fake reader fills between
//! callbacks.
//!
//! This test binary installs a global allocator that counts calls on each
//! thread, so a count covers only the code that ran on that thread.  The
//! optimiser may remove an allocation, so a zero count is evidence, not
//! proof (`std::alloc::GlobalAlloc`).

use engine::readahead::{LaneFormat, MIN_AHEAD_SLOTS, MIN_LOOP_SLOTS, ReadAhead};
use engine::slot::{Scratch, WEB_BUFFER_FRAMES};
use engine::stretch::{MAX_PUSH_FRAMES, StretcherKind};
use engine::wav::{FRAME_BYTES, write_pcm16};
use engine::{Player, PlayerConfig, PlayerSlot, SharedParams, Song, Stem};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::thread::LocalKey;

/// The stretchers that the app can play through.  The app builds `engine`
/// with `stretch-wsola` only, and `Varispeed` is always compiled in.
const SHIPPED: [StretcherKind; 2] = [StretcherKind::Varispeed, StretcherKind::Wsola];
const RATES: [u32; 2] = [44_100, 48_000];
/// 10,000 frames is over `SCRATCH_FRAMES`, so `render_interleaved` runs its
/// inner loop twice, and is not a multiple of the player's block.
/// `WEB_BUFFER_FRAMES` is the browser's callback.
const CALLBACK_FRAMES: [usize; 5] = [128, 512, WEB_BUFFER_FRAMES as usize, 4_096, 10_000];
const STEMS: usize = 6;
const SONG_SECONDS: usize = 10;

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static FREES: Cell<u64> = const { Cell::new(0) };
}

fn add_one(counter: &'static LocalKey<Cell<u64>>) {
    // A `const` `Cell` has no destructor, so `try_with` never fails; it
    // keeps a panic out of the allocator all the same.
    let _ = counter.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: every call is forwarded unchanged to `System`; counting touches
// only `const` thread-locals, which do not allocate.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        add_one(&ALLOCATIONS);
        // SAFETY: the caller's contract is the same as `System`'s.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        add_one(&ALLOCATIONS);
        // SAFETY: the caller's contract is the same as `System`'s.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        add_one(&ALLOCATIONS);
        // SAFETY: the caller's contract is the same as `System`'s.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        add_one(&FREES);
        // SAFETY: the caller's contract is the same as `System`'s.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    allocations: u64,
    frees: u64,
}

fn counts_so_far() -> Counts {
    Counts {
        allocations: ALLOCATIONS.with(Cell::get),
        frees: FREES.with(Cell::get),
    }
}

/// Runs `f` and gives the allocations and frees it made on this thread.
fn counted<T>(f: impl FnOnce() -> T) -> (T, Counts) {
    let before = counts_so_far();
    let value = f();
    let after = counts_so_far();
    let counts = Counts {
        allocations: after.allocations - before.allocations,
        frees: after.frees - before.frees,
    };
    (value, counts)
}

#[test]
fn the_counter_sees_an_allocation() {
    let ((), counts) = counted(|| {
        let mut grown = black_box(Vec::<u8>::with_capacity(64));
        grown.reserve_exact(4_096);
        black_box(&grown);
        drop(black_box(vec![0_u8; 64]));
        drop(grown);
    });
    assert_eq!(
        counts,
        Counts {
            allocations: 3,
            frees: 2
        },
        "the counter must see `alloc`, `realloc`, `alloc_zeroed` and `dealloc`, or the RT-1 test checks nothing"
    );
}

/// White noise from a xorshift generator, so each stem differs.
fn noise(frames: usize, seed: u32) -> Vec<f32> {
    let mut x = seed;
    (0..frames)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 - 0.5) * 0.2
        })
        .collect()
}

/// Where the rig's player reads its six stems.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lanes {
    Memory,
    /// 16-bit lanes over a `ReadAhead` with the fewest slots (4 and 2), so
    /// slots are given to new blocks during the run and a stall uses up
    /// the buffer in about 3 s of song.
    Stored,
}

/// The stored lanes, and the fake reader that fills their slots between
/// callbacks, outside the counted part.
struct Storage {
    ahead: Arc<ReadAhead>,
    files: Vec<Vec<u8>>,
    reading: bool,
}

impl Storage {
    fn new(channels: &[(Vec<f32>, Vec<f32>)], frames: usize, params: Arc<SharedParams>) -> Self {
        let files = channels
            .iter()
            .map(|(left, right)| {
                let mut bytes = vec![0; frames * FRAME_BYTES];
                write_pcm16(left, right, &mut bytes);
                bytes
            })
            .collect();
        let formats = vec![LaneFormat::Pcm16; channels.len()];
        let ahead = ReadAhead::with_slots(
            &formats,
            frames as u64,
            params,
            MIN_AHEAD_SLOTS,
            MIN_LOOP_SLOTS,
        );
        Self {
            ahead: Arc::new(ahead),
            files,
            reading: true,
        }
    }

    /// Reads every block that the buffer wants, as a reader that keeps up.
    fn read(&self) {
        if !self.reading {
            return;
        }
        while let Some(read) = self.ahead.next_read() {
            self.ahead.fill(read, |lanes| {
                for (lane, bytes) in lanes.iter_mut().enumerate() {
                    let file = &self.files[lane];
                    let start = (read.first_frame() as usize * FRAME_BYTES).min(file.len());
                    let n = (file.len() - start).min(bytes.len());
                    bytes[..n].copy_from_slice(&file[start..start + n]);
                    bytes[n..].fill(0);
                    if self.ahead.is_failed(lane) {
                        bytes.fill(0);
                    }
                }
            });
        }
    }
}

/// One player in a slot, driven callback by callback, with the counts and
/// problems of the phase that runs.
struct Rig {
    label: String,
    rate: u32,
    frames: usize,
    params: Arc<SharedParams>,
    slot: PlayerSlot,
    storage: Option<Storage>,
    data: Vec<f32>,
    scratch: Scratch,
    counts: Counts,
    heard: bool,
    problems: Vec<String>,
}

impl Rig {
    fn new(kind: StretcherKind, rate: u32, callback_frames: usize, lanes: Lanes) -> Self {
        let frames = SONG_SECONDS * rate as usize;
        let seeds = 1..=(2 * STEMS) as u32;
        let seeds: Vec<u32> = seeds.collect();
        let channels: Vec<(Vec<f32>, Vec<f32>)> = seeds
            .chunks(2)
            .map(|pair| (noise(frames, pair[0]), noise(frames, pair[1])))
            .collect();
        let params = Arc::new(SharedParams::new(STEMS));
        let slot = PlayerSlot::new();
        let (player, storage) = match lanes {
            Lanes::Memory => {
                let song = Arc::new(Song {
                    sample_rate: rate,
                    frames,
                    stems: channels
                        .into_iter()
                        .enumerate()
                        .map(|(i, (left, right))| Stem {
                            name: format!("lloyd {i}"),
                            left: left.into(),
                            right: right.into(),
                        })
                        .collect(),
                });
                (Player::new(song, Arc::clone(&params), kind), None)
            }
            Lanes::Stored => {
                // The buffer is made, with every slot, before the player.
                let storage = Storage::new(&channels, frames, Arc::clone(&params));
                let player = Player::with_sources(
                    rate,
                    frames,
                    Arc::clone(&params),
                    kind.build(rate),
                    PlayerConfig::default(),
                    storage.ahead.sources(),
                );
                (player, Some(storage))
            }
        };
        drop(slot.replace(Some(player)));
        Self {
            label: format!(
                "{}, {rate} Hz, {callback_frames}-frame callbacks, {lanes:?} lanes",
                kind.label()
            ),
            rate,
            frames,
            params,
            slot,
            storage,
            data: vec![0.0; callback_frames * 2],
            scratch: Scratch::default(),
            counts: Counts::default(),
            heard: false,
            problems: Vec::new(),
        }
    }

    fn ahead(&self) -> &ReadAhead {
        &self.storage.as_ref().expect("stored lanes").ahead
    }

    fn set_reading(&mut self, reading: bool) {
        if let Some(storage) = &mut self.storage {
            storage.reading = reading;
        }
    }

    fn callback(&mut self) {
        if let Some(storage) = &self.storage {
            storage.read();
        }
        let (slot, data, scratch) = (&self.slot, &mut self.data, &mut self.scratch);
        let ((), counts) = counted(|| slot.render_interleaved(data, 2, scratch));
        self.counts.allocations += counts.allocations;
        self.counts.frees += counts.frees;
        self.heard |= self.data.iter().any(|v| *v != 0.0);
    }

    /// Plays `seconds` of output: `ceil(seconds * rate / callback)`
    /// callbacks.  Gives the frames played.
    fn play(&mut self, seconds: f64) -> u64 {
        let callback_frames = self.data.len() / 2;
        let callbacks = (seconds * f64::from(self.rate) / callback_frames as f64).ceil() as usize;
        for _ in 0..callbacks {
            self.callback();
        }
        (callbacks * callback_frames) as u64
    }

    fn seconds(&self, seconds: f64) -> u64 {
        (seconds * f64::from(self.rate)) as u64
    }

    fn playhead(&self) -> u64 {
        self.params.playhead()
    }

    fn fail(&mut self, phase: &str, what: String) {
        self.problems.push(format!(
            "RT-1 test: {}, phase \"{phase}\" {what}, so it checked nothing.  Fix: make the phase play, or correct the test.",
            self.label
        ));
    }

    /// Runs one phase, then records what it allocated or freed, and
    /// whether it played anything.
    fn phase(&mut self, name: &str, steps: impl FnOnce(&mut Rig)) {
        self.counts = Counts::default();
        self.heard = false;
        steps(self);
        if self.counts != Counts::default() {
            self.problems.push(format!(
                "RT-1: the audio callback allocated or freed memory after warm-up: {}, phase \"{name}\": {} allocations, {} frees.  Fix: size the buffer in `Player::with_sources`, `ReadAhead::new` or `Stretcher::warm_up`, or reuse a buffer.  Do not allocate in `Player::process` or below.",
                self.label, self.counts.allocations, self.counts.frees
            ));
        }
        if !self.heard {
            self.fail(name, "gave only silence".into());
        }
    }
}

/// Plays one stretcher through every phase and gives the problems found.
fn run(kind: StretcherKind, rate: u32, callback_frames: usize, lanes: Lanes) -> Vec<String> {
    let mut rig = Rig::new(kind, rate, callback_frames, lanes);
    rig.phase("start", |r| {
        r.params.playing.store(true, Relaxed);
        r.params.set_tempo(1.0);
        r.play(0.5);
    });
    rig.phase("tempo sweep", |r| {
        for step in 0..=10 {
            r.params.set_tempo(0.5 + 0.1 * step as f32);
            r.play(0.1);
        }
    });
    rig.phase("fast", |r| {
        let before = r.playhead();
        r.params.set_tempo(1.5);
        // 3.5 s of the song is more than twice 65,536 input frames, so
        // wsola's input buffer reaches its largest size.
        r.play(3.5 / 1.5);
        let moved = r.playhead().saturating_sub(before);
        if moved < r.seconds(3.0) {
            r.fail(
                "fast",
                format!("moved the playhead {moved} frames, under 3 s of the song"),
            );
        }
    });
    rig.phase("chipmunk", |r| {
        let before = r.playhead();
        r.params.chipmunk.store(true, Relaxed);
        let played = r.play(0.2);
        // Chipmunk mode reads two song frames per frame played; the
        // stretcher at tempo 1.5 reads 1.5.
        let moved = r.playhead().saturating_sub(before);
        if moved * 10 < played * 18 {
            r.fail(
                "chipmunk",
                format!("moved the playhead {moved} frames in {played}, under 1.8 per frame"),
            );
        }
        r.params.chipmunk.store(false, Relaxed);
        r.play(0.2);
    });
    rig.phase("seek", |r| {
        // A pause while the seek's fade-out runs: with callbacks shorter
        // than the 256-frame fade, the paused callback finishes the jump.
        r.params.request_seek(r.seconds(2.0));
        r.callback();
        r.params.playing.store(false, Relaxed);
        r.callback();
        r.params.playing.store(true, Relaxed);
        r.play(0.2);
        r.params.playing.store(false, Relaxed);
        r.callback();
        r.params.request_seek(r.seconds(3.0));
        r.callback();
        r.params.playing.store(true, Relaxed);
        r.play(0.2);
    });
    rig.phase("loop", |r| {
        let start = r.playhead();
        let end = start + r.seconds(1.0);
        r.params.set_loop(start, end);
        r.play(2.0);
        let head = r.playhead();
        if !(start..=end).contains(&head) {
            r.fail("loop", format!("left the loop {start}..={end} for {head}"));
        }
    });
    rig.phase("short loop", |r| {
        let start = r.playhead();
        r.params.set_loop(start, start + 1);
        r.play(0.2);
    });
    rig.phase("mix", |r| {
        r.params.clear_loop();
        r.params.set_muted(0, true);
        r.play(0.1);
        r.params.set_soloed(1, true);
        r.play(0.1);
        r.params.set_muted(0, false);
        r.play(0.1);
        r.params.set_soloed(1, false);
        r.play(0.1);
    });
    if lanes == Lanes::Stored {
        stored_phases(&mut rig);
    }
    rig.phase("end", |r| {
        r.params.request_seek(r.frames as u64 - r.seconds(0.1));
        let callback_frames = r.data.len() / 2;
        let most = r.seconds(2.0) as usize / callback_frames;
        let mut callbacks = 0;
        while r.params.playing.load(Relaxed) && callbacks < most {
            r.callback();
            callbacks += 1;
        }
        r.callback();
        r.callback();
        if r.params.playing.load(Relaxed) {
            r.fail("end", "did not stop at the end of the song".into());
        }
    });
    rig.problems
}

/// The phases of stored lanes only.  Each fails unless its counter moved.
fn stored_phases(rig: &mut Rig) {
    rig.phase("reader stalls", |r| {
        // From 1 s at tempo 1, so the stall and the phases after it fit in
        // the song.
        r.params.set_tempo(1.0);
        r.params.request_seek(r.seconds(1.0));
        r.play(0.3);
        let before = r.ahead().underruns();
        r.set_reading(false);
        r.play(4.0);
        r.set_reading(true);
        r.play(0.5);
        if r.ahead().underruns() == before {
            r.fail("reader stalls", "counted no underrun".into());
        }
    });
    rig.phase("seek holds", |r| {
        let before = r.params.held_seeks.load(Relaxed);
        r.set_reading(false);
        // About 3 s ahead: past the four blocks ahead that the buffer holds.
        let target = r.playhead() + r.seconds(3.2);
        r.params.request_seek(target);
        r.play(0.2);
        r.set_reading(true);
        r.play(0.3);
        if r.params.held_seeks.load(Relaxed) == before {
            r.fail("seek holds", "held no seek".into());
        }
    });
    rig.phase("loop behind the head", |r| {
        r.params.request_seek(r.seconds(8.0));
        r.play(0.3);
        let before = r.params.held_seeks.load(Relaxed);
        r.set_reading(false);
        // The buffer holds the blocks from 8 s on, not the loop's.
        r.params.set_loop(r.seconds(2.0), r.seconds(3.0));
        r.play(0.2);
        r.set_reading(true);
        r.play(1.0);
        r.params.clear_loop();
        if r.params.held_seeks.load(Relaxed) == before {
            r.fail("loop behind the head", "held no seek".into());
        }
    });
    rig.phase("seek held at the end", |r| {
        // The old position reaches the song's end while a seek holds; the
        // seek then fades out and jumps, in callbacks shorter than its fade
        // too (review round 3 of M6, finding 3-1).
        r.params.request_seek(r.frames as u64 - r.seconds(0.05));
        r.play(0.1);
        r.params.playing.store(true, Relaxed);
        r.set_reading(false);
        r.params.request_seek(r.seconds(5.0));
        r.play(0.3);
        r.set_reading(true);
        r.play(0.5);
        if !r.params.playing.load(Relaxed) {
            r.fail("seek held at the end", "stopped at the song's end".into());
        }
    });
    rig.phase("lane fails", |r| {
        r.ahead().fail_lane(0);
        r.play(0.5);
        if r.ahead().failed_lanes() != 1 {
            r.fail("lane fails", "failed no lane".into());
        }
    });
}

#[test]
fn the_callback_does_not_allocate_after_warm_up() {
    let mut runs = Vec::new();
    for kind in SHIPPED {
        for rate in RATES {
            for callback_frames in CALLBACK_FRAMES {
                for lanes in [Lanes::Memory, Lanes::Stored] {
                    runs.push(std::thread::spawn(move || {
                        run(kind, rate, callback_frames, lanes)
                    }));
                }
            }
        }
    }
    let problems: Vec<String> = runs
        .into_iter()
        .flat_map(|run| run.join().expect("a run does not panic"))
        .collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// What `Stretcher::warm_up` promises: after it, a push of up to 4,096
/// frames and a pull of up to `max_frames`, at up to `max_tempo`, do not
/// allocate.  The player pushes less, so this checks the promise itself,
/// at low rates, where wsola's buffers are smallest, and at high ones.
#[test]
fn a_stretcher_keeps_its_warm_up_promise() {
    const PUSH: usize = MAX_PUSH_FRAMES;
    let mut runs = Vec::new();
    for kind in SHIPPED {
        for (rate, pull) in [
            (8_000, 512),
            (16_000, 512),
            (44_100, 512),
            (44_100, 4_096),
            (48_000, 512),
            (96_000, 512),
            (192_000, 512),
        ] {
            runs.push(std::thread::spawn(move || {
                let input = noise(PUSH, rate);
                let (mut left, mut right) = (vec![0.0; pull], vec![0.0; pull]);
                let mut stretcher = kind.build(rate);
                stretcher.set_tempo(1.5);
                stretcher.warm_up(1.5, pull);
                // About 190,000 input frames: wsola drops old input about
                // every 70,000.
                let pulls = 250 * 512 / pull;
                let ((), counts) = counted(|| {
                    for _ in 0..pulls {
                        while !stretcher.can_pull(pull) {
                            stretcher.push(&input, &input);
                        }
                        stretcher.pull(&mut left, &mut right);
                    }
                });
                (counts != Counts::default()).then(|| {
                    format!(
                        "RT-1: {} at {rate} Hz allocated after `warm_up`, with pushes of {PUSH} frames and pulls of {pull} at tempo 1.5: {} allocations, {} frees.  Fix: make `warm_up` size the buffer, or narrow its promise.",
                        kind.label(),
                        counts.allocations,
                        counts.frees
                    )
                })
            }));
        }
    }
    let problems: Vec<String> = runs
        .into_iter()
        .filter_map(|run| run.join().expect("a run does not panic"))
        .collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// What in the app's manifest could put a stretcher that `SHIPPED` does not
/// cover into the app.
fn app_stretcher_problems(manifest: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let stretchers: BTreeSet<&str> = manifest
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .filter(|word| word.starts_with("stretch-"))
        .collect();
    if stretchers != BTreeSet::from(["stretch-wsola"]) {
        problems.push(format!(
            "RT-1: `app/Cargo.toml` turns on {stretchers:?}, but `engine/tests/realtime.rs` covers only `SHIPPED` (varispeed, and wsola through `stretch-wsola`).  Fix: add the stretcher to `SHIPPED`, make the test pass, and update this set."
        ));
    }
    let mut engine_lines = 0;
    for line in manifest.lines().map(str::trim) {
        let key = line.split('=').next().unwrap_or_default().trim();
        if key == "engine" {
            engine_lines += 1;
            if !line.contains("default-features = false") {
                problems.push(format!(
                    "RT-1: `app/Cargo.toml` takes `engine` with its default features, which turn on every stretcher: `{line}`.  Fix: add `default-features = false`."
                ));
            }
            if line.contains("\"default\"") {
                problems.push(format!(
                    "RT-1: `app/Cargo.toml` turns on `engine`'s default features, which turn on every stretcher: `{line}`.  Fix: name only the features the app needs."
                ));
            }
        } else if key.starts_with("engine.")
            || (line.starts_with('[') && line.ends_with(".engine]"))
        {
            problems.push(format!(
                "RT-1: `app/Cargo.toml` writes `engine` in a form this check does not read: `{line}`.  Fix: write `engine` as one inline table."
            ));
        }
    }
    if engine_lines == 0 {
        problems.push(
            "RT-1: `app/Cargo.toml` has no `engine = { ... }` line.  Fix: write `engine` as one inline table, or update this check.".into(),
        );
    }
    for default in ["engine/default", "engine?/default"] {
        if manifest.contains(default) {
            problems.push(format!(
                "RT-1: `app/Cargo.toml` turns on `{default}`, which turns on every stretcher.  Fix: name only the features the app needs."
            ));
        }
    }
    problems
}

#[test]
fn the_manifest_check_finds_a_stretcher() {
    let base = r#"[features]
audio = ["engine/cpal"]

[dependencies]
engine = { path = "../engine", default-features = false, features = ["stretch-wsola"] }
"#;
    assert_eq!(app_stretcher_problems(base), Vec::<String>::new());
    let breaches = [
        base.replace(
            r#""engine/cpal""#,
            r#""engine/cpal", "engine/stretch-cute""#,
        ),
        base.replace("default-features = false, ", ""),
        base.replace(r#""engine/cpal""#, r#""engine/default""#),
        base.replace(r#""engine/cpal""#, r#""engine?/default""#),
        format!("{base}\n[target.'cfg(unix)'.dependencies.engine]\npath = \"../engine\"\n"),
        format!("{base}engine.path = \"../engine\"\n"),
    ];
    for breach in breaches {
        let problems = app_stretcher_problems(&breach);
        assert_eq!(problems.len(), 1, "{breach}\n{problems:#?}");
        assert!(problems[0].starts_with("RT-1:"), "{}", problems[0]);
    }
}

#[test]
fn the_app_ships_only_these_stretchers() {
    let problems = app_stretcher_problems(include_str!("../../app/Cargo.toml"));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
