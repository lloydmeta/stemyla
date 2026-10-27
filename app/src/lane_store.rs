//! The lane reader (D3 of the M7 plan): the page-side reads of stored lanes
//! in blocks, for the mix lane's waveform, the spectrograms and the saves.
//! It is not the read-ahead buffer of M6, which is for playback only.
//!
//! A [`LaneStore`] reads byte ranges of lane files and calls back once.
//! A [`BlockReader`] serves one job: it holds at most one read in flight
//! and the frames of its last read, never more than
//! `engine::source::BLOCK_FRAMES` frames of a lane.  A job asks whether its
//! frames are ready; when they are not, the reader starts their read, and
//! the job asks again in a later frame.
//!
//! [`MemoryStore`] holds files as bytes in memory, for the native tests.
//! By default it completes a read at once, inside the call that starts it,
//! so a native job never waits.  A test can hold its reads back.
//!
//! On the web, `OpfsStore` reads browser storage with
//! `Blob.slice(...).arrayBuffer()` of a `File` from `getFile`, one promise
//! for each range, as M6's reader does, and asks for a repaint when a read
//! ends.

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

#[cfg(target_arch = "wasm32")]
pub use web::OpfsStore;
#[cfg(all(test, target_arch = "wasm32"))]
pub use web::reads_for_test;

/// The lane store of browser storage.  Web only.
#[cfg(target_arch = "wasm32")]
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
    held: Held,
    /// A read has started and has not ended.
    in_flight: bool,
    /// Why the first read that failed failed.
    failed: Option<String>,
}

/// Reads the stored lanes of one job in blocks (D3 of the M7 plan).
pub struct BlockReader {
    store: Rc<dyn LaneStore>,
    files: Vec<LaneFile>,
    /// The most frames of a lane that one read holds.
    capacity: usize,
    state: Rc<RefCell<ReaderState>>,
}

impl BlockReader {
    /// A reader of `files`, lanes of a song of `frames` frames.  Its
    /// buffers are made once, at the size of a block.
    pub fn new(store: Rc<dyn LaneStore>, files: Vec<LaneFile>, frames: usize) -> Self {
        let capacity = (engine::source::BLOCK_FRAMES as usize).min(frames);
        let lanes = files.len();
        let held = Held {
            ranges: Vec::new(),
            starts: Vec::new(),
            left: vec![vec![0.0; capacity]; lanes],
            right: vec![vec![0.0; capacity]; lanes],
        };
        Self {
            store,
            files,
            capacity,
            state: Rc::new(RefCell::new(ReaderState {
                held,
                in_flight: false,
                failed: None,
            })),
        }
    }

    /// Whether the frames of every range `(at, frames)` are held.  If not,
    /// and no read is in flight, it starts one read of all the ranges.
    /// Panics if the ranges hold more than a block.
    pub fn ready(&self, ranges: &[(usize, usize)]) -> bool {
        {
            let state = self.state.borrow();
            if state.held.holds(ranges) {
                return true;
            }
            if state.in_flight {
                return false;
            }
        }
        let total: usize = ranges.iter().map(|&(_, n)| n).sum();
        assert!(
            total <= self.capacity,
            "a read of {total} frames is more than a block of {}",
            self.capacity
        );
        self.start(ranges);
        // A store in memory completes the read at once.
        self.state.borrow().held.holds(ranges)
    }

    /// Why a read failed, if one did.  The reader gave silence for it.
    pub fn failed(&self) -> Option<String> {
        self.state.borrow().failed.clone()
    }

    /// The frames of the last read.
    pub fn held(&self) -> Ref<'_, Held> {
        Ref::map(self.state.borrow(), |state| &state.held)
    }

    fn start(&self, ranges: &[(usize, usize)]) {
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
        state.borrow_mut().in_flight = true;
        self.store.read(
            reads,
            Box::new(move |result| {
                let mut state = state.borrow_mut();
                state.in_flight = false;
                if let Err(e) = &result {
                    state.failed.get_or_insert_with(|| e.clone());
                }
                fill(&mut state.held, &formats, ranges, result);
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

    #[test]
    #[should_panic(expected = "more than a block")]
    fn a_read_of_more_than_a_block_panics() {
        let store = MemoryStore::new();
        let file = LaneFile::f32("acme/0.f32".into(), 100_000);
        let reader = BlockReader::new(store as Rc<dyn LaneStore>, vec![file], 100_000);
        reader.ready(&[(0, 32_769)]);
    }
}
