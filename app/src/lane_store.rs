//! The lane reader (D3 of the M7 plan): the page-side reads of stored lanes
//! in blocks, for the mix lane's waveform, the spectrograms and the saves.
//! It is not the read-ahead buffer of M6, which is for playback only.
//!
//! A [`LaneStore`] reads byte ranges of lane files and calls back once.
//! A [`BlockReader`] serves one job: it holds at most one read in flight
//! and the frames of its last read, never more than
//! `engine::source::BLOCK_FRAMES` frames of a lane.  A job asks whether its
//! frames are ready; when they are not, the reader starts their read, and
//! the job asks again later.  A save's reader reads ahead
//! ([`BlockReader::reading_ahead`], D11 of the M8 plan): it holds two
//! blocks of the job, one of them filled by the read in flight.
//!
//! [`MemoryStore`] holds files as bytes in memory, for the native tests.
//! By default it completes a read at once, inside the call that starts it,
//! so a native job never waits.  A test can hold its reads back.
//!
//! On the web, `OpfsStore` reads browser storage with
//! `Blob.slice(...).arrayBuffer()` of a `File` from `getFile`, one promise
//! for each range, as M6's reader does, and asks for a repaint when a read
//! ends.  A save reads through a [`WakingStore`] over an `OpfsStore` with
//! no repaint, which signals the save's task loop instead.

use crate::store_meta::LaneFile;
use engine::readahead::decode_frames;
use std::cell::{Cell, Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// The bytes `start..end` of the file at `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub path: String,
    pub start: u64,
    pub end: u64,
}

/// What a read gives: the bytes of each range, in the order asked.  A range
/// past a file's end gives fewer bytes, or none.
pub type ReadResult = Result<Vec<Vec<u8>>, String>;

/// Where a read's result goes.
pub type ReadDone = Box<dyn FnOnce(ReadResult)>;

/// Reads byte ranges of lane files.  It calls `done` once, with every
/// range's bytes or an error.  On the web, it asks for a repaint when a
/// read ends.
pub trait LaneStore {
    fn read(&self, ranges: Vec<ByteRange>, done: ReadDone);
}

/// Lane files as bytes in memory, for the native tests.
#[derive(Default)]
pub struct MemoryStore {
    files: RefCell<HashMap<String, Rc<Vec<u8>>>>,
    /// Reads wait for [`MemoryStore::release`] while this is true.
    hold: Cell<bool>,
    held: RefCell<Vec<(Vec<ByteRange>, ReadDone)>>,
    /// The reads started, and the largest range read, in bytes.
    requests: Cell<usize>,
    largest: Cell<u64>,
}

impl MemoryStore {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Puts the file `path`.
    pub fn put(&self, path: &str, bytes: Vec<u8>) {
        self.files.borrow_mut().insert(path.into(), Rc::new(bytes));
    }

    /// Holds the reads that start from now, until [`Self::release`], or
    /// stops holding them.
    pub fn hold(&self, on: bool) {
        self.hold.set(on);
    }

    /// Completes every read held back.
    pub fn release(&self) {
        let held = std::mem::take(&mut *self.held.borrow_mut());
        for (ranges, done) in held {
            done(self.bytes_of(&ranges));
        }
    }

    /// The reads that wait for [`Self::release`].
    pub fn held_reads(&self) -> usize {
        self.held.borrow().len()
    }

    /// The reads started so far.
    pub fn requests(&self) -> usize {
        self.requests.get()
    }

    /// The largest range read so far, in bytes.
    pub fn largest_read(&self) -> u64 {
        self.largest.get()
    }

    fn bytes_of(&self, ranges: &[ByteRange]) -> ReadResult {
        let files = self.files.borrow();
        ranges
            .iter()
            .map(|range| {
                let file = files
                    .get(&range.path)
                    .ok_or_else(|| format!("no file {}", range.path))?;
                let end = (range.end as usize).min(file.len());
                let start = (range.start as usize).min(end);
                self.largest
                    .set(self.largest.get().max((end - start) as u64));
                #[expect(
                    clippy::disallowed_methods,
                    reason = "MEM-1: one range of a lane file, at most a block, for a store in memory"
                )]
                let bytes = file[start..end].to_vec();
                Ok(bytes)
            })
            .collect()
    }
}

impl LaneStore for MemoryStore {
    fn read(&self, ranges: Vec<ByteRange>, done: ReadDone) {
        self.requests.set(self.requests.get() + 1);
        if self.hold.get() {
            self.held.borrow_mut().push((ranges, done));
        } else {
            done(self.bytes_of(&ranges));
        }
    }
}

/// A store that calls `wake` after each read of `inner` ends, after its
/// result went to the reader.  A save reads through one, so its task loop
/// goes on when a read ends, with no repaint (D11 of the M8 plan).
pub struct WakingStore {
    inner: Rc<dyn LaneStore>,
    wake: Rc<dyn Fn()>,
}

impl WakingStore {
    pub fn new(inner: Rc<dyn LaneStore>, wake: Rc<dyn Fn()>) -> Self {
        Self { inner, wake }
    }
}

impl LaneStore for WakingStore {
    fn read(&self, ranges: Vec<ByteRange>, done: ReadDone) {
        let wake = Rc::clone(&self.wake);
        self.inner.read(
            ranges,
            Box::new(move |result| {
                done(result);
                wake();
            }),
        );
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::OpfsStore;
#[cfg(all(test, target_arch = "wasm32"))]
pub use web::reads_for_test;

/// The lane store of browser storage.  Web only.
#[cfg(target_arch = "wasm32")]
#[cfg_attr(mutants, mutants::skip)]
mod web {
    use super::{ByteRange, LaneStore, ReadDone, ReadResult};
    use eframe::egui;
    use js_sys::{Array, Uint8Array};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
// Reads `ranges[2i]..ranges[2i + 1]` of each `File` of `files` (null: no
// read), each on its own promise, so one bad file does not lose the
// others'.  Gives a Uint8Array, null, or the error's text for each.
export async function ls_read(files, ranges) {
  const got = await Promise.allSettled(files.map((f, i) =>
    f ? f.slice(ranges[2 * i], ranges[2 * i + 1]).arrayBuffer() : Promise.resolve(null)));
  return got.map((r) => r.status === "fulfilled"
    ? (r.value ? new Uint8Array(r.value) : null)
    : String((r.reason && r.reason.name) || r.reason));
}
"#)]
    extern "C" {
        async fn ls_read(files: &Array, ranges: &[f64]) -> JsValue;
    }

    /// The lane files of browser storage, read in ranges (D3 of the M7
    /// plan).  It keeps each `File` it gets, and gets it again after a
    /// read fails, as M6's reader does: a file that moved, such as a stem
    /// that a new separation replaces, is read anew.
    pub struct OpfsStore {
        ctx: Option<egui::Context>,
        files: Rc<RefCell<HashMap<String, web_sys::File>>>,
    }

    impl OpfsStore {
        /// A store that asks `ctx` for a repaint when a read ends.
        pub fn new(ctx: Option<egui::Context>) -> Self {
            Self {
                ctx,
                files: Rc::default(),
            }
        }
    }

    #[cfg(test)]
    thread_local! {
        static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// The reads that every `OpfsStore` of this thread started, for tests
    /// that count them.
    #[cfg(test)]
    pub fn reads_for_test() -> usize {
        READS.get()
    }

    impl LaneStore for OpfsStore {
        fn read(&self, ranges: Vec<ByteRange>, done: ReadDone) {
            #[cfg(test)]
            READS.set(READS.get() + 1);
            let (ctx, files) = (self.ctx.clone(), Rc::clone(&self.files));
            wasm_bindgen_futures::spawn_local(async move {
                let mut result = read_ranges(&files, &ranges).await;
                if result.is_err() {
                    // Get each file again, and read once more.
                    files.borrow_mut().clear();
                    result = read_ranges(&files, &ranges).await;
                }
                done(result);
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
        }
    }

    async fn read_ranges(
        files: &Rc<RefCell<HashMap<String, web_sys::File>>>,
        ranges: &[ByteRange],
    ) -> ReadResult {
        let list = Array::new();
        let mut flat = Vec::with_capacity(2 * ranges.len());
        for range in ranges {
            if range.start < range.end {
                let cached = files.borrow().get(&range.path).cloned();
                let file = match cached {
                    Some(file) => file,
                    None => {
                        let file =
                            crate::store::file_handle(&range.path)
                                .await
                                .ok_or_else(|| {
                                    format!("{} is missing from browser storage", range.path)
                                })?;
                        files.borrow_mut().insert(range.path.clone(), file.clone());
                        file
                    }
                };
                list.push(&file);
            } else {
                list.push(&JsValue::NULL);
            }
            flat.push(range.start as f64);
            flat.push(range.end as f64);
        }
        let got: Array = ls_read(&list, &flat).await.unchecked_into();
        ranges
            .iter()
            .enumerate()
            .map(|(k, range)| {
                if range.start >= range.end {
                    return Ok(Vec::new());
                }
                let result = got.get(k as u32);
                if let Some(why) = result.as_string() {
                    return Err(format!("{}: {why}", range.path));
                }
                let bytes: Uint8Array = result.unchecked_into();
                let expected = (range.end - range.start) as usize;
                if bytes.length() as usize != expected {
                    return Err(format!(
                        "{}: read {} of {expected} bytes",
                        range.path,
                        bytes.length()
                    ));
                }
                let mut out = vec![0; expected];
                bytes.copy_to(&mut out);
                Ok(out)
            })
            .collect()
    }
}

/// The frames of the last read of a [`BlockReader`].
pub struct Held {
    /// The frame ranges `(at, frames)` held, and where each starts in the
    /// buffers.
    ranges: Vec<(usize, usize)>,
    starts: Vec<usize>,
    /// The frames of each lane, the ranges one after another.
    left: Vec<Vec<f32>>,
    right: Vec<Vec<f32>>,
}

impl Held {
    /// The `frames` frames from `at` of the lane `lane`, from the held
    /// range that holds them.  Panics if no range holds them.
    pub fn frames(&self, lane: usize, at: usize, frames: usize) -> (&[f32], &[f32]) {
        let k = self
            .ranges
            .iter()
            .position(|&(start, n)| start <= at && at + frames <= start + n)
            .expect("the frames of a ready range");
        let from = self.starts[k] + at - self.ranges[k].0;
        (
            &self.left[lane][from..from + frames],
            &self.right[lane][from..from + frames],
        )
    }

    fn holds(&self, ranges: &[(usize, usize)]) -> bool {
        ranges.iter().all(|&(at, frames)| {
            frames == 0
                || self
                    .ranges
                    .iter()
                    .any(|&(start, n)| start <= at && at + frames <= start + n)
        })
    }
}

struct ReaderState {
    /// One buffer, or two when the reader reads ahead.
    slots: Vec<Held>,
    /// The slot that the job reads.
    current: usize,
    /// The slot that the read in flight fills, if a read is in flight.
    in_flight: Option<usize>,
    /// Why the first read that failed failed.
    failed: Option<String>,
}

/// Reads the stored lanes of one job in blocks (D3 of the M7 plan).
pub struct BlockReader {
    store: Rc<dyn LaneStore>,
    files: Vec<LaneFile>,
    /// The frames of the song.
    frames: usize,
    /// The most frames of a lane that one read holds.
    capacity: usize,
    /// The reader reads the next range while the job uses this one.
    ahead: bool,
    state: Rc<RefCell<ReaderState>>,
}

impl BlockReader {
    /// A reader of `files`, lanes of a song of `frames` frames.  Its
    /// buffers are made once, at the size of a block.
    pub fn new(store: Rc<dyn LaneStore>, files: Vec<LaneFile>, frames: usize) -> Self {
        let capacity = (engine::source::BLOCK_FRAMES as usize).min(frames);
        Self::with_slots(store, files, frames, capacity, false)
    }

    /// A reader that reads ahead, for a job that reads its source in
    /// order, `block` frames at a time (D11 of the M8 plan).  When the job
    /// takes a range, the reader starts the read of the next range of the
    /// same length.  So the job mixes one block while the next is read.
    /// It holds two buffers of `block` frames of each lane, one of them
    /// filled by the read in flight, and never more.
    pub fn reading_ahead(
        store: Rc<dyn LaneStore>,
        files: Vec<LaneFile>,
        frames: usize,
        block: usize,
    ) -> Self {
        Self::with_slots(store, files, frames, block.min(frames), true)
    }

    fn with_slots(
        store: Rc<dyn LaneStore>,
        files: Vec<LaneFile>,
        frames: usize,
        capacity: usize,
        ahead: bool,
    ) -> Self {
        let lanes = files.len();
        let slot = || Held {
            ranges: Vec::new(),
            starts: Vec::new(),
            left: vec![vec![0.0; capacity]; lanes],
            right: vec![vec![0.0; capacity]; lanes],
        };
        let slots = if ahead {
            vec![slot(), slot()]
        } else {
            vec![slot()]
        };
        Self {
            store,
            files,
            frames,
            capacity,
            ahead,
            state: Rc::new(RefCell::new(ReaderState {
                slots,
                current: 0,
                in_flight: None,
                failed: None,
            })),
        }
    }

    /// Whether the frames of every range `(at, frames)` are held.  If not,
    /// and no read is in flight, it starts one read of all the ranges.  A
    /// reader that reads ahead then starts the read of the next range.
    /// Panics if the ranges hold more than a block.
    pub fn ready(&self, ranges: &[(usize, usize)]) -> bool {
        let target = {
            let mut state = self.state.borrow_mut();
            let spare = state.slots.len() - 1 - state.current;
            if state.slots[state.current].holds(ranges) {
                None
            } else if state.in_flight != Some(spare) && state.slots[spare].holds(ranges) {
                state.current = spare;
                None
            } else if state.in_flight.is_some() {
                return false;
            } else {
                Some(spare)
            }
        };
        if let Some(target) = target {
            let total: usize = ranges.iter().map(|&(_, n)| n).sum();
            assert!(
                total <= self.capacity,
                "a read of {total} frames is more than a block of {}",
                self.capacity
            );
            self.start(ranges, target);
            // A store in memory completes the read at once.
            let mut state = self.state.borrow_mut();
            if !state.slots[target].holds(ranges) {
                return false;
            }
            state.current = target;
        }
        self.read_ahead(ranges);
        true
    }

    /// Starts the read of the range after `ranges`, when the reader reads
    /// ahead, `ranges` is one range, and no read is in flight.
    fn read_ahead(&self, ranges: &[(usize, usize)]) {
        let &[(at, frames)] = ranges else {
            return;
        };
        let next_at = at + frames;
        if !self.ahead || frames == 0 || next_at >= self.frames {
            return;
        }
        let next = [(next_at, frames.min(self.frames - next_at))];
        let target = {
            let state = self.state.borrow();
            let spare = state.slots.len() - 1 - state.current;
            if state.in_flight.is_some() || state.slots[spare].holds(&next) {
                return;
            }
            spare
        };
        self.start(&next, target);
    }

    /// Why a read failed, if one did.  The reader gave silence for it.
    pub fn failed(&self) -> Option<String> {
        self.state.borrow().failed.clone()
    }

    /// The frames of the last read that the job took.
    pub fn held(&self) -> Ref<'_, Held> {
        Ref::map(self.state.borrow(), |state| &state.slots[state.current])
    }

    /// The buffers of the reader: the frames of each lane that it can
    /// hold, in all its slots.
    pub fn held_frames(&self) -> usize {
        self.state.borrow().slots.len() * self.capacity
    }

    /// Starts one read of `ranges` into the slot `target`.
    fn start(&self, ranges: &[(usize, usize)], target: usize) {
        let reads: Vec<ByteRange> = self
            .files
            .iter()
            .flat_map(|file| {
                ranges.iter().map(|&(at, frames)| {
                    let (start, end) = file.byte_range(at as u64, frames as u64);
                    ByteRange {
                        path: file.path.clone(),
                        start,
                        end,
                    }
                })
            })
            .collect();
        let formats: Vec<_> = self.files.iter().map(|f| f.format).collect();
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: the frame ranges of one read, not audio"
        )]
        let ranges = ranges.to_vec();
        let state = Rc::clone(&self.state);
        state.borrow_mut().in_flight = Some(target);
        self.store.read(
            reads,
            Box::new(move |result| {
                let mut state = state.borrow_mut();
                state.in_flight = None;
                if let Err(e) = &result {
                    state.failed.get_or_insert_with(|| e.clone());
                }
                fill(&mut state.slots[target], &formats, ranges, result);
            }),
        );
    }
}

/// Puts the frames of a read into `held`: each lane's ranges one after
/// another, with silence past a file's end.  A failed read gives silence,
/// as a stored lane that fails plays silence (M6).
fn fill(
    held: &mut Held,
    formats: &[engine::readahead::LaneFormat],
    ranges: Vec<(usize, usize)>,
    result: ReadResult,
) {
    let bytes = match result {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            log::warn!("lanes: a read of a lane file failed: {e}");
            None
        }
    };
    held.starts.clear();
    let mut start = 0;
    for &(_, frames) in &ranges {
        held.starts.push(start);
        start += frames;
    }
    for (lane, &format) in formats.iter().enumerate() {
        for (i, &(_, frames)) in ranges.iter().enumerate() {
            let at = held.starts[i];
            let (left, right) = (
                &mut held.left[lane][at..at + frames],
                &mut held.right[lane][at..at + frames],
            );
            let read = match &bytes {
                Some(bytes) => decode_frames(format, &bytes[lane * ranges.len() + i], left, right),
                None => 0,
            };
            left[read..].fill(0.0);
            right[read..].fill(0.0);
        }
    }
    held.ranges = ranges;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An f32 lane file whose frame `k` is `(k, -k)`.
    fn counting(frames: usize) -> Vec<u8> {
        (0..frames)
            .flat_map(|k| [(k as f32).to_le_bytes(), (-(k as f32)).to_le_bytes()])
            .flatten()
            .collect()
    }

    /// The reader gives any frames inside a range it holds, from each
    /// range of one read, and silence past the file's end.
    #[test]
    fn a_held_range_gives_any_frames_inside_it() {
        let store = MemoryStore::new();
        store.put("acme/0.f32", counting(1_000));
        let file = LaneFile::f32("acme/0.f32".into(), 1_000);
        let reader = BlockReader::new(Rc::clone(&store) as Rc<dyn LaneStore>, vec![file], 5_000);
        assert!(reader.ready(&[(100, 50), (990, 20)]));
        assert_eq!(store.requests(), 1, "one read for both ranges");
        assert!(reader.ready(&[(110, 5)]), "inside a held range");
        assert_eq!(store.requests(), 1, "no new read");
        let held = reader.held();
        let (left, right) = held.frames(0, 110, 3);
        assert_eq!(
            (left, right),
            (&[110.0, 111.0, 112.0][..], &[-110.0, -111.0, -112.0][..])
        );
        let (left, _) = held.frames(0, 995, 10);
        assert_eq!(
            left,
            [995.0, 996.0, 997.0, 998.0, 999.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        );
    }

    /// A read that fails gives silence.
    #[test]
    fn a_failed_read_gives_silence() {
        let store = MemoryStore::new();
        let file = LaneFile::f32("acme/missing.f32".into(), 1_000);
        let reader = BlockReader::new(Rc::clone(&store) as Rc<dyn LaneStore>, vec![file], 1_000);
        assert_eq!(reader.failed(), None);
        assert!(reader.ready(&[(0, 10)]));
        assert_eq!(
            reader.held().frames(0, 0, 10),
            (&[0.0; 10][..], &[0.0; 10][..])
        );
        assert_eq!(reader.failed().as_deref(), Some("no file acme/missing.f32"));
    }

    /// D11 of M8: a reader that reads ahead starts the read of the next
    /// range when the job takes one, holds at most two blocks of each lane
    /// and one read in flight, and reads nothing past the song's end.
    #[test]
    fn a_reader_that_reads_ahead_keeps_one_read_in_flight() {
        let store = MemoryStore::new();
        store.put("acme/0.f32", counting(250));
        let file = LaneFile::f32("acme/0.f32".into(), 250);
        let reader = BlockReader::reading_ahead(
            Rc::clone(&store) as Rc<dyn LaneStore>,
            vec![file],
            250,
            100,
        );
        assert_eq!(reader.held_frames(), 200, "two blocks of the lane");
        store.hold(true);
        assert!(!reader.ready(&[(0, 100)]), "the first read is in flight");
        assert!(!reader.ready(&[(0, 100)]));
        assert_eq!((store.requests(), store.held_reads()), (1, 1));
        store.release();
        assert!(reader.ready(&[(0, 100)]));
        assert_eq!(
            (store.requests(), store.held_reads()),
            (2, 1),
            "the next block's read starts"
        );
        assert_eq!(reader.held().frames(0, 99, 1).0, [99.0]);
        assert!(reader.ready(&[(0, 100)]), "the held block, again");
        assert_eq!(store.requests(), 2, "no read for it");
        assert!(!reader.ready(&[(100, 100)]), "the next block is in flight");
        store.release();
        assert!(reader.ready(&[(100, 100)]));
        assert_eq!(reader.held().frames(0, 100, 2).0, [100.0, 101.0]);
        store.release();
        assert!(reader.ready(&[(200, 50)]));
        assert_eq!(reader.held().frames(0, 249, 1).0, [249.0]);
        assert_eq!(
            (store.requests(), store.held_reads()),
            (3, 0),
            "no read past the end"
        );
        assert_eq!(store.largest_read(), 800, "a block of 100 frames");
    }

    /// A reader that reads ahead from a store that completes at once gives
    /// the right frames of each block, from the block that it read ahead.
    #[test]
    fn a_block_read_ahead_at_once_gives_its_own_frames() {
        let store = MemoryStore::new();
        store.put("acme/0.f32", counting(250));
        let plus_1000: Vec<u8> = (0..250)
            .flat_map(|k| [(k as f32 + 1000.0).to_le_bytes(), [0; 4]])
            .flatten()
            .collect();
        store.put("acme/1.f32", plus_1000);
        let files = vec![
            LaneFile::f32("acme/0.f32".into(), 250),
            LaneFile::f32("acme/1.f32".into(), 250),
        ];
        let reader =
            BlockReader::reading_ahead(Rc::clone(&store) as Rc<dyn LaneStore>, files, 250, 100);
        for (k, at) in [0, 100, 200].into_iter().enumerate() {
            let frames = 100.min(250 - at);
            assert!(reader.ready(&[(at, frames)]), "block {k} is ready");
            let held = reader.held();
            let (left, right) = held.frames(0, at, frames);
            assert_eq!((left[0], right[0]), (at as f32, -(at as f32)));
            assert_eq!(left[frames - 1], (at + frames - 1) as f32);
            assert_eq!(
                held.frames(1, at, frames).0[frames - 1],
                (at + frames + 999) as f32
            );
            drop(held);
            assert_eq!(store.requests(), (k + 2).min(3), "one read for each block");
        }
    }

    /// A reader that does not read ahead starts no read that the job did
    /// not ask for.
    #[test]
    fn a_reader_reads_only_what_the_job_asks() {
        let store = MemoryStore::new();
        store.put("acme/0.f32", counting(250));
        let file = LaneFile::f32("acme/0.f32".into(), 250);
        let reader = BlockReader::new(Rc::clone(&store) as Rc<dyn LaneStore>, vec![file], 250);
        assert_eq!(reader.held_frames(), 250);
        assert!(reader.ready(&[(0, 100)]));
        assert_eq!(store.requests(), 1);
        assert!(reader.ready(&[(100, 100)]));
        assert_eq!(store.requests(), 2);
    }

    #[test]
    #[should_panic(expected = "more than a block")]
    fn a_read_of_more_than_a_block_panics() {
        let store = MemoryStore::new();
        let file = LaneFile::f32("acme/0.f32".into(), 100_000);
        let reader = BlockReader::new(store as Rc<dyn LaneStore>, vec![file], 100_000);
        reader.ready(&[(0, 32_769)]);
    }
}
