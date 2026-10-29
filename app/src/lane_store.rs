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
//! ([`BlockReader::reading_ahead`], D11 of the M8 plan): it reads the song
//! in order, in spans of [`SAVE_AHEAD`], with several reads in flight.  It
//! holds a fixed number of spans, whatever the song's length.
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

    /// Completes the first `n` reads held back, in the order they
    /// started.  Reads that their ends start are held after the others.
    pub fn release_first(&self, n: usize) {
        let first: Vec<_> = self.held.borrow_mut().drain(..n).collect();
        for (ranges, done) in first {
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

/// A store that signals a save's task after each read of `inner` ends,
/// after its result went to the reader.  A save reads through one, so its
/// task loop goes on when a read ends, with no repaint (D11 of the M8
/// plan).  The signal keeps the longest time that the end of a read took
/// in the reader, as `now` measures it (finding 4-8 of review round speed
/// 4 of M8): the decode of a span of every lane into its slot.
pub struct WakingStore {
    inner: Rc<dyn LaneStore>,
    signal: Rc<crate::save_task::Signal>,
    now: Rc<dyn Fn() -> f64>,
}

impl WakingStore {
    pub fn new(
        inner: Rc<dyn LaneStore>,
        signal: Rc<crate::save_task::Signal>,
        now: Rc<dyn Fn() -> f64>,
    ) -> Self {
        Self { inner, signal, now }
    }
}

impl LaneStore for WakingStore {
    fn read(&self, ranges: Vec<ByteRange>, done: ReadDone) {
        let (signal, now) = (Rc::clone(&self.signal), Rc::clone(&self.now));
        self.inner.read(
            ranges,
            Box::new(move |result| {
                let start = now();
                done(result);
                signal.note_read_end(now() - start);
                signal.notify();
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

/// How a reader reads ahead (D11 of the M8 plan): spans of `span` frames
/// of each lane, read in order, with up to `depth` reads in flight.  The
/// reader holds `depth + 1` spans of each lane at most: one that the job
/// reads, and `depth` read ahead or in flight.  The spans of all lanes
/// hold at most `most_bytes` of samples ([`Ahead::for_lanes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ahead {
    pub span: usize,
    pub depth: usize,
    pub most_bytes: usize,
}

/// The bytes of one frame of a lane in a reader's slot: two `f32`s.
const SLOT_FRAME_BYTES: usize = 8;

impl Ahead {
    /// This read-ahead for `lanes` lanes.  When `depth + 1` spans of every
    /// lane hold more than `most_bytes`, the span shrinks to fit, in job
    /// blocks (`export::BLOCK`), and never below one job block (finding (b)
    /// of review round speed 4 of M8).
    pub fn for_lanes(self, lanes: usize) -> Self {
        let fit = self.most_bytes / ((self.depth + 1) * SLOT_FRAME_BYTES * lanes.max(1));
        if fit >= self.span {
            return self;
        }
        let block = crate::export::BLOCK;
        Self {
            span: (fit / block * block).max(block).min(self.span),
            ..self
        }
    }

    /// The bytes of samples that the slots of `lanes` lanes hold.
    pub fn slot_bytes(self, lanes: usize) -> usize {
        (self.depth + 1) * self.span * SLOT_FRAME_BYTES * lanes
    }
}

/// The read-ahead of a save (review rounds speed 3 and 4 of M8): spans of
/// 4 job blocks, 2 reads in flight, 3 spans held.  For up to 7 lanes (an
/// iPhone's 6 stems and a source), it holds 196,608 frames of each lane:
/// 1.5 MiB for each lane, 10.5 MiB in all (MEM-4).  For more lanes, the
/// spans shrink, so the slots never hold more than that, up to 28 lanes.
/// A read of browser storage ends only in a later task.  When each frame
/// of the page holds the main thread for long, as a paint in software
/// does on CI (about 125 ms in CI run 40838459678), a read ends about two
/// frames after it starts, whatever its size up to 2 MiB.  So the frames
/// in flight, not the reads, set the speed of a save.
pub const SAVE_AHEAD: Ahead = Ahead {
    span: 65_536,
    depth: SAVE_DEPTH,
    most_bytes: (SAVE_DEPTH + 1) * 65_536 * SLOT_FRAME_BYTES * 7,
};
const SAVE_DEPTH: usize = 2;

struct ReaderState {
    /// One buffer, or `depth + 1` when the reader reads ahead.
    slots: Vec<Held>,
    /// The slots that a read in flight fills.
    filling: Vec<bool>,
    /// The slot that the job reads.
    current: usize,
    /// The first frame of the job's last range.  A slot whose frames are
    /// all before it is free for a read ahead.
    reading_at: usize,
    /// A reader that reads ahead: the start of the next span to read.
    next_at: usize,
    /// Why the first read that failed failed.
    failed: Option<String>,
}

impl ReaderState {
    fn in_flight(&self) -> usize {
        self.filling.iter().filter(|&&f| f).count()
    }

    /// The slot that holds every range of `ranges`, with no read in flight.
    fn holding(&self, ranges: &[(usize, usize)]) -> Option<usize> {
        (0..self.slots.len()).find(|&k| !self.filling[k] && self.slots[k].holds(ranges))
    }

    /// A slot that a read ahead may fill: not the job's, with no read in
    /// flight, and empty or before the job.
    fn free_slot(&self) -> Option<usize> {
        (0..self.slots.len()).find(|&k| {
            k != self.current
                && !self.filling[k]
                && self.slots[k]
                    .ranges
                    .iter()
                    .all(|&(at, n)| at + n <= self.reading_at)
        })
    }
}

/// What a read needs.  The end of a read holds it weakly: a reader that
/// the job dropped fills no slot and starts no read.
struct Inner {
    store: Rc<dyn LaneStore>,
    files: Vec<LaneFile>,
    formats: Vec<engine::readahead::LaneFormat>,
    /// The frames of the song.
    frames: usize,
    /// The most frames of a lane that one read holds.
    capacity: usize,
    /// `Some` when the reader reads ahead.
    ahead: Option<Ahead>,
    state: RefCell<ReaderState>,
}

/// Reads the stored lanes of one job in blocks (D3 of the M7 plan).
pub struct BlockReader {
    inner: Rc<Inner>,
}

impl BlockReader {
    /// A reader of `files`, lanes of a song of `frames` frames.  Its
    /// buffers are made once, at the size of a block.
    pub fn new(store: Rc<dyn LaneStore>, files: Vec<LaneFile>, frames: usize) -> Self {
        let capacity = (engine::source::BLOCK_FRAMES as usize).min(frames);
        Self::with_slots(store, files, frames, capacity, None)
    }

    /// A reader that reads ahead, for a job that reads its source in
    /// order, one range of at most `ahead.span` frames at a time (D11 of
    /// the M8 plan).  It reads the song in spans of `ahead.span` frames,
    /// with up to `ahead.depth` reads in flight.  When the job leaves a
    /// span, the reader starts the read of the next span into its slot.  It
    /// holds `depth + 1` spans of each lane, and never more.  When the job
    /// asks for a range out of order, the reader waits for its reads in
    /// flight, then reads on from that range.
    pub fn reading_ahead(
        store: Rc<dyn LaneStore>,
        files: Vec<LaneFile>,
        frames: usize,
        ahead: Ahead,
    ) -> Self {
        assert!(
            ahead.depth > 0,
            "a reader that reads ahead reads 1 span or more"
        );
        let ahead = ahead.for_lanes(files.len());
        Self::with_slots(store, files, frames, ahead.span.min(frames), Some(ahead))
    }

    fn with_slots(
        store: Rc<dyn LaneStore>,
        files: Vec<LaneFile>,
        frames: usize,
        capacity: usize,
        ahead: Option<Ahead>,
    ) -> Self {
        let lanes = files.len();
        let slot = || Held {
            ranges: Vec::new(),
            starts: Vec::new(),
            left: vec![vec![0.0; capacity]; lanes],
            right: vec![vec![0.0; capacity]; lanes],
        };
        let count = ahead.map_or(1, |a| a.depth + 1);
        let formats = files.iter().map(|f| f.format).collect();
        Self {
            inner: Rc::new(Inner {
                store,
                files,
                formats,
                frames,
                capacity,
                ahead,
                state: RefCell::new(ReaderState {
                    slots: (0..count).map(|_| slot()).collect(),
                    filling: vec![false; count],
                    current: 0,
                    reading_at: 0,
                    next_at: 0,
                    failed: None,
                }),
            }),
        }
    }

    /// Whether the frames of every range `(at, frames)` are held.  If not,
    /// and no read is in flight, it starts one read of all the ranges.  A
    /// reader that reads ahead takes one range, and keeps its reads of the
    /// next spans in flight.  Panics if the ranges hold more than a block.
    pub fn ready(&self, ranges: &[(usize, usize)]) -> bool {
        let total: usize = ranges.iter().map(|&(_, n)| n).sum();
        assert!(
            total <= self.inner.capacity,
            "a read of {total} frames is more than a block of {}",
            self.inner.capacity
        );
        match (self.inner.ahead, ranges) {
            (None, _) => self.ready_once(ranges),
            (Some(_), &[range]) => self.ready_ahead(range),
            (Some(_), _) => panic!("a reader that reads ahead takes one range at a time"),
        }
    }

    /// [`Self::ready`] of a reader that does not read ahead.
    fn ready_once(&self, ranges: &[(usize, usize)]) -> bool {
        {
            let state = self.inner.state.borrow();
            if state.slots[0].holds(ranges) {
                return true;
            }
            if state.in_flight() > 0 {
                return false;
            }
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: the frame ranges of one read, not audio"
        )]
        let owned = ranges.to_vec();
        start(&self.inner, owned, 0);
        // A store in memory completes the read at once.
        self.inner.state.borrow().slots[0].holds(ranges)
    }

    /// [`Self::ready`] of a reader that reads ahead.
    fn ready_ahead(&self, range: (usize, usize)) -> bool {
        let ranges = [range];
        {
            let mut state = self.inner.state.borrow_mut();
            match state.holding(&ranges) {
                Some(k) => state.current = k,
                None if state.in_flight() > 0 => return false,
                None => {
                    // The first range, or a range out of order: read on
                    // from it.
                    for held in &mut state.slots {
                        held.ranges.clear();
                    }
                    state.next_at = range.0;
                }
            }
            state.reading_at = range.0;
        }
        top_up(&self.inner);
        let found = self.inner.state.borrow().holding(&ranges);
        let Some(k) = found else {
            return false;
        };
        self.inner.state.borrow_mut().current = k;
        // The slot of the job's last span may be free now.
        top_up(&self.inner);
        true
    }

    /// Why a read failed, if one did.  The reader gave silence for it.
    pub fn failed(&self) -> Option<String> {
        self.inner.state.borrow().failed.clone()
    }

    /// The frames of the last read that the job took.
    pub fn held(&self) -> Ref<'_, Held> {
        Ref::map(self.inner.state.borrow(), |state| {
            &state.slots[state.current]
        })
    }

    /// The buffers of the reader: the frames of each lane that it can
    /// hold, in all its slots.
    pub fn held_frames(&self) -> usize {
        self.inner.state.borrow().slots.len() * self.inner.capacity
    }
}

/// Starts the reads of the next spans while fewer than `depth` are in
/// flight and a slot is free.
fn top_up(inner: &Rc<Inner>) {
    let Some(ahead) = inner.ahead else {
        return;
    };
    loop {
        let (slot, at, frames) = {
            let mut state = inner.state.borrow_mut();
            if state.in_flight() >= ahead.depth || state.next_at >= inner.frames {
                return;
            }
            let Some(slot) = state.free_slot() else {
                return;
            };
            let at = state.next_at;
            let frames = inner.capacity.min(inner.frames - at);
            state.next_at = at + frames;
            state.slots[slot].ranges.clear();
            (slot, at, frames)
        };
        start(inner, vec![(at, frames)], slot);
    }
}

/// Starts one read of `ranges` into the slot `target`.  Its end fills the
/// slot, while the reader lives.  It starts no read: with `depth + 1`
/// slots, no slot is free while `depth` reads are in flight, so the job's
/// next [`BlockReader::ready`] starts the next reads (finding 4-14 of
/// review round speed 4 of M8).
fn start(inner: &Rc<Inner>, ranges: Vec<(usize, usize)>, target: usize) {
    let reads: Vec<ByteRange> = inner
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
    inner.state.borrow_mut().filling[target] = true;
    let reader = Rc::downgrade(inner);
    inner.store.read(
        reads,
        Box::new(move |result| {
            let Some(inner) = reader.upgrade() else {
                return;
            };
            let mut state = inner.state.borrow_mut();
            state.filling[target] = false;
            if let Err(e) = &result {
                state.failed.get_or_insert_with(|| e.clone());
            }
            fill(&mut state.slots[target], &inner.formats, ranges, result);
        }),
    );
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
            Ahead {
                span: 100,
                depth: 1,
                most_bytes: usize::MAX,
            },
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

    const ONE: Ahead = Ahead {
        span: 100,
        depth: 1,
        most_bytes: usize::MAX,
    };

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
            BlockReader::reading_ahead(Rc::clone(&store) as Rc<dyn LaneStore>, files, 250, ONE);
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

    /// A reader of the f32 lane `acme/0.f32` of `frames` counting frames,
    /// which reads ahead as `ahead` says, from a store that holds its
    /// reads.
    fn holding_reader(frames: usize, ahead: Ahead) -> (Rc<MemoryStore>, BlockReader) {
        let store = MemoryStore::new();
        store.put("acme/0.f32", counting(frames));
        let file = LaneFile::f32("acme/0.f32".into(), frames);
        let reader = BlockReader::reading_ahead(
            Rc::clone(&store) as Rc<dyn LaneStore>,
            vec![file],
            frames,
            ahead,
        );
        store.hold(true);
        (store, reader)
    }

    /// Review round speed 3 of M8: a reader that reads ahead keeps `depth`
    /// reads in flight, of the next spans in order.  The end of a read
    /// starts no read until the job frees a slot.  Each span is read once,
    /// and nothing past the song's end.
    #[test]
    fn a_reader_keeps_its_depth_of_reads_in_flight() {
        let ahead = Ahead {
            span: 100,
            depth: 3,
            most_bytes: usize::MAX,
        };
        let (store, reader) = holding_reader(950, ahead);
        assert_eq!(reader.held_frames(), 400, "depth + 1 spans of the lane");
        assert!(!reader.ready(&[(0, 50)]));
        assert_eq!(store.held_reads(), 3, "3 spans in flight at the start");
        store.release();
        assert_eq!(
            store.requests(),
            3,
            "the ends start no read: no slot is free"
        );
        assert!(reader.ready(&[(0, 50)]));
        assert_eq!(
            store.held_reads(),
            1,
            "the empty slot reads the fourth span"
        );
        assert!(reader.ready(&[(50, 50)]));
        assert_eq!(store.held_reads(), 1, "the job's span holds its slot");
        assert!(reader.ready(&[(100, 50)]));
        assert_eq!(
            store.held_reads(),
            2,
            "the job left a span: its slot reads on"
        );
        assert_eq!(reader.held().frames(0, 149, 1).0, [149.0]);
        let mut at = 150;
        while at < 950 {
            store.release();
            let n = 50.min(950 - at);
            assert!(reader.ready(&[(at, n)]), "frames {at} are ready");
            assert_eq!(
                reader.held().frames(0, at + n - 1, 1).0,
                [(at + n - 1) as f32]
            );
            assert!(store.held_reads() <= 3);
            at += n;
        }
        assert_eq!(store.requests(), 10, "each span once");
        assert_eq!(store.held_reads(), 0, "no read past the end");
        assert_eq!(store.largest_read(), 800, "a span of 100 frames");
    }

    /// Review rounds speed 3 and 4 of M8: the bound of a save's reader is
    /// a constant, for 1 minute as for 10, and for any number of lanes up
    /// to 28.  Up to 7 lanes, as on an iPhone with 6 stems and a source,
    /// the spans have 65,536 frames.  With more, the spans shrink in job
    /// blocks: 16,384 frames for 21 lanes, the most that a stored song
    /// plays from storage (MEM-4).
    #[test]
    fn the_save_reader_holds_a_fixed_bound() {
        assert_eq!(SAVE_AHEAD.most_bytes, 11_010_048, "10.5 MiB");
        let store = MemoryStore::new();
        let files = |lanes, frames| -> Vec<LaneFile> {
            (0..lanes)
                .map(|k| LaneFile::f32(format!("acme/{k}.f32"), frames))
                .collect()
        };
        for (lanes, span) in [
            (1, 65_536),
            (7, 65_536),
            (8, 49_152),
            (21, 16_384),
            (28, 16_384),
        ] {
            for frames in [2_646_000, 26_460_000] {
                let reader = BlockReader::reading_ahead(
                    Rc::clone(&store) as Rc<dyn LaneStore>,
                    files(lanes, frames),
                    frames,
                    SAVE_AHEAD,
                );
                let what = format!("{lanes} lanes, {frames} frames");
                assert_eq!(
                    reader.held_frames(),
                    (SAVE_AHEAD.depth + 1) * span,
                    "{what}"
                );
                let bytes = reader.held_frames() * 2 * 4 * lanes;
                assert!(bytes <= SAVE_AHEAD.most_bytes, "{what}: {bytes} bytes");
                assert_eq!(
                    bytes,
                    SAVE_AHEAD.for_lanes(lanes).slot_bytes(lanes),
                    "{what}"
                );
            }
        }
    }

    /// A reader that the job dropped frees its slots at once, also while
    /// its reads are in flight, and their ends start no read (D3 of M8: a
    /// dropped save stops).
    #[test]
    fn a_dropped_reader_frees_its_slots_at_once() {
        let (store, reader) = holding_reader(
            1_000,
            Ahead {
                span: 100,
                depth: 2,
                most_bytes: usize::MAX,
            },
        );
        assert!(!reader.ready(&[(0, 100)]));
        assert_eq!(store.held_reads(), 2);
        let inner = Rc::downgrade(&reader.inner);
        drop(reader);
        assert!(
            inner.upgrade().is_none(),
            "the reads in flight hold no slot"
        );
        store.release();
        assert_eq!(store.requests(), 2, "no read after the drop");
    }

    /// A job that asks for a range out of order waits for the reads in
    /// flight, then the reader reads on from that range.
    #[test]
    fn a_range_out_of_order_reads_on_from_it() {
        let (store, reader) = holding_reader(
            1_000,
            Ahead {
                span: 100,
                depth: 2,
                most_bytes: usize::MAX,
            },
        );
        assert!(!reader.ready(&[(0, 100)]));
        store.release();
        assert!(reader.ready(&[(0, 100)]));
        assert!(!reader.ready(&[(650, 100)]), "a read is in flight");
        store.release();
        assert!(!reader.ready(&[(650, 100)]), "the reads from 650 start");
        assert_eq!(store.held_reads(), 2);
        store.release();
        assert!(reader.ready(&[(650, 100)]));
        assert_eq!(reader.held().frames(0, 700, 1).0, [700.0]);
        assert!(
            reader.ready(&[(750, 100)]),
            "read ahead with the span of 650"
        );
        assert_eq!(reader.held().frames(0, 849, 1).0, [849.0]);
    }

    /// A read ahead that fails gives silence, and the reader says why.
    #[test]
    fn a_failed_read_ahead_gives_silence() {
        let store = MemoryStore::new();
        let file = LaneFile::f32("acme/missing.f32".into(), 1_000);
        let reader = BlockReader::reading_ahead(
            Rc::clone(&store) as Rc<dyn LaneStore>,
            vec![file],
            1_000,
            Ahead {
                span: 100,
                depth: 2,
                most_bytes: usize::MAX,
            },
        );
        assert!(reader.ready(&[(0, 100)]));
        assert_eq!(
            reader.held().frames(0, 0, 3),
            (&[0.0; 3][..], &[0.0; 3][..])
        );
        assert_eq!(reader.failed().as_deref(), Some("no file acme/missing.f32"));
    }

    #[test]
    #[should_panic(expected = "one range at a time")]
    fn a_reader_that_reads_ahead_takes_one_range() {
        let (_store, reader) = holding_reader(
            1_000,
            Ahead {
                span: 100,
                depth: 2,
                most_bytes: usize::MAX,
            },
        );
        reader.ready(&[(0, 10), (20, 10)]);
    }
}
