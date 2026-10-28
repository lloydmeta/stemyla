//! Saving a stem or the mix as heard, one block at a time (decision 0014).
//!
//! An [`ExportJob`] reads a [`Pcm`] source in blocks and writes each block
//! to a [`Sink`].  The page steps a job for a few milliseconds in each UI
//! frame, since the main thread also feeds the Web Audio output.  The sinks
//! on the web write into JS arrays, or into a save file in browser storage
//! (`StoredWav`, D3 of M8), so no song-sized buffer is made in
//! WebAssembly memory (MEM-1, MEM-4).  A sink can make the job wait while
//! a write is in flight ([`Sink::ready`]).

use crate::DERIVED;
use engine::mixdown::{MixRecipe, Pcm};
use engine::overview::COLUMNS;

/// Frames that a job reads at a time.
pub const BLOCK: usize = 16_384;

/// Where a job writes its blocks.
pub trait Sink {
    /// Takes the frames from `at`; both channels have the same length.
    fn write(&mut self, at: usize, left: &[f32], right: &[f32]);

    /// Whether the sink takes a block now.  A sink that writes browser
    /// storage is not ready while a write is in flight, and asks for a
    /// repaint when it ends (D3 of M8).
    fn ready(&self) -> bool {
        true
    }

    /// Why a write of the sink failed, if one did.  A job that stops at a
    /// failure stops at it.
    fn failed(&self) -> Option<String> {
        None
    }
}

/// Bytes of each write of a save file but the last: 1 MiB (D3 of M8).
pub const SAVE_WRITE_BYTES: usize = 1 << 20;

/// The bytes of a 16-bit WAV in blocks of one size, for a sink that writes
/// them to browser storage (D3 of M8).  It takes the job's blocks, and
/// gives each full block, then the last short block.  The WAV's header
/// comes first.  It holds at most one full block and one job block.
pub struct WavBlocks {
    size: usize,
    pending: Vec<u8>,
}

impl WavBlocks {
    /// For a WAV of `frames` frames at `sample_rate`, in blocks of `size`
    /// bytes.
    pub fn new(frames: usize, sample_rate: u32, size: usize) -> Self {
        let mut pending = Vec::with_capacity(size + BLOCK * engine::wav::FRAME_BYTES);
        pending.extend_from_slice(&engine::wav::header(frames, sample_rate));
        Self { size, pending }
    }

    /// Adds the 16-bit samples of one block of the job.
    pub fn push(&mut self, left: &[f32], right: &[f32]) {
        let at = self.pending.len();
        self.pending
            .resize(at + left.len() * engine::wav::FRAME_BYTES, 0);
        engine::wav::write_pcm16(left, right, &mut self.pending[at..]);
    }

    /// The next full block, if there is one.
    pub fn full(&self) -> Option<&[u8]> {
        (self.pending.len() >= self.size).then(|| &self.pending[..self.size])
    }

    /// Drops the full block that [`Self::full`] gave.
    pub fn take_full(&mut self) {
        if self.pending.len() >= self.size {
            self.pending.drain(..self.size);
        }
    }

    /// The bytes after the last full block: the last block, once the job
    /// is done.
    pub fn rest(&self) -> &[u8] {
        &self.pending
    }
}

/// Reads a source in blocks and writes them, in order, to a sink.
pub struct ExportJob<S: Sink> {
    source: Box<dyn Pcm>,
    sink: S,
    at: usize,
    clipped: usize,
    /// The last step found the source not ready.
    waiting: bool,
    /// The job ends at the first failed read ([`Self::stopping_at_a_failure`]).
    stops_at_a_failure: bool,
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
            stops_at_a_failure: false,
            left: vec![0.0; block],
            right: vec![0.0; block],
        }
    }

    /// Reads and writes the next block.  True when the source is done, or
    /// when a job that stops at a failure meets a failed read or write.
    /// When the sink or the source is not ready, it does nothing and waits.
    pub fn step(&mut self) -> bool {
        if self.stops_at_a_failure
            && (self.source.failed().is_some() || self.sink.failed().is_some())
        {
            self.waiting = false;
            return true;
        }
        let n = self.left.len().min(self.source.frames() - self.at);
        self.waiting = n > 0 && !(self.sink.ready() && self.source.ready(self.at, n));
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

    /// The job ends at the first failed read of its source's storage, so
    /// that its caller can give up (finding 5-1 of review round 5 of M7).
    /// Else a failed read gives silence and the job goes on, as the mix
    /// lane's waveform needs.
    pub fn stopping_at_a_failure(mut self) -> Self {
        self.stops_at_a_failure = true;
        self
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

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    pub fn into_sink(self) -> S {
        self.sink
    }
}

/// Both channels in memory, for a source of at most the memory path's
/// length: the input of classic drums that the page reads first (D8 of
/// the M7 plan).
pub struct VecSink {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl VecSink {
    pub fn new(frames: usize) -> Self {
        Self {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        }
    }
}

impl Sink for VecSink {
    fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
        self.left[at..at + left.len()].copy_from_slice(left);
        self.right[at..at + right.len()].copy_from_slice(right);
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

/// The job that makes the overview of a lane or of a mix of `frames`
/// frames on the page (D7 of the M7 plan).  It stops at the first failed
/// read, so the page gives up at once (finding 6-2 of review round 6 of
/// M7).
pub fn overview_job(pcm: Box<dyn Pcm>, frames: usize) -> ExportJob<PeaksSink> {
    ExportJob::new(pcm, PeaksSink::new(frames, COLUMNS)).stopping_at_a_failure()
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
        #[cfg_attr(mutants, mutants::skip)]
        pub fn new(frames: usize) -> Self {
            Self {
                left: Float32Array::new_with_length(frames as u32),
                right: Float32Array::new_with_length(frames as u32),
            }
        }

        /// The channels as the source of an encode request, which
        /// transfers them (MEM-2).
        #[cfg_attr(mutants, mutants::skip)]
        pub fn into_encode(self) -> stem_protocol::Encode {
            stem_protocol::Encode::Arrays {
                left: self.left,
                right: self.right,
            }
        }
    }

    impl Sink for JsChannels {
        #[cfg_attr(mutants, mutants::skip)]
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
        #[cfg_attr(mutants, mutants::skip)]
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

        #[cfg_attr(mutants, mutants::skip)]
        pub fn into_bytes(self) -> Uint8Array {
            self.bytes
        }
    }

    impl Sink for JsWav {
        #[cfg_attr(mutants, mutants::skip)]
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
        #[cfg_attr(mutants, mutants::skip)]
        pub fn new(format: Format, frames: usize, sample_rate: u32) -> Self {
            match format {
                Format::Wav => JsSink::Wav(JsWav::new(frames, sample_rate)),
                Format::Flac => JsSink::Flac(JsChannels::new(frames)),
                Format::Mp3 => JsSink::Mp3(JsChannels::new(frames)),
            }
        }
    }

    impl Sink for JsSink {
        #[cfg_attr(mutants, mutants::skip)]
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            match self {
                JsSink::Wav(s) => s.write(at, left, right),
                JsSink::Flac(s) | JsSink::Mp3(s) => s.write(at, left, right),
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use stored::{Finish, SaveSink, StoredWav};

/// The sink of the storage path of a save: a 16-bit WAV save file in
/// browser storage, written in blocks (D3 of M8).
#[cfg(target_arch = "wasm32")]
mod stored {
    use super::{SAVE_WRITE_BYTES, Sink, WavBlocks};
    use crate::store;
    use js_sys::Uint8Array;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsValue;

    /// What a [`StoredWav`] shares with its writes in flight.
    #[derive(Default)]
    struct Shared {
        /// The stream, once open.
        stream: Option<JsValue>,
        /// An open, a write or the close is in flight.
        busy: bool,
        failed: Option<String>,
        /// The close resolved: the file is complete.
        closed: bool,
        /// The sink was dropped before its file was complete.
        dropped: bool,
    }

    /// Where the end stage of a [`StoredWav`] is.
    #[derive(Debug, PartialEq)]
    pub enum Finish {
        /// A write or the close is in flight; ask again after a repaint.
        Pending,
        /// The file is complete.
        Done,
        /// The open, a write or the close failed; the file is gone.
        Failed(String),
    }

    /// A 16-bit WAV save file in browser storage (D3 of M8).  Each full
    /// block of [`SAVE_WRITE_BYTES`] goes through one JS array, made once
    /// for the save, and written whole (MEM-5).  At most one write is in
    /// flight.  While one is, the sink is not ready, so the job waits and
    /// takes no time in the frame.  The end of each write calls `wake`,
    /// which asks for a repaint.  After the job is done, [`Self::finish`]
    /// writes the last block, in a new array of its own length, and closes
    /// the stream.  The file is complete only after the close.  A failed
    /// open, write or close removes the file and sets [`Sink::failed`].  A
    /// sink dropped before its file is complete aborts its stream and
    /// removes the file.
    pub struct StoredWav {
        path: String,
        shared: Rc<RefCell<Shared>>,
        blocks: WavBlocks,
        /// The JS array of every full block, made at the first one.
        array: Option<Uint8Array>,
        wake: Rc<dyn Fn()>,
        /// The end stage has sent the last block.
        ended: bool,
    }

    /// What a write in flight does.
    enum Op {
        Put(Uint8Array),
        /// The last block, if any, then the close.
        Last(Option<Uint8Array>),
    }

    impl StoredWav {
        /// Opens the save file at `path` for a WAV of `frames` frames at
        /// `sample_rate`.  The open runs in the background: the sink is
        /// ready when it ends.
        #[cfg_attr(mutants, mutants::skip)]
        pub fn new(path: String, frames: usize, sample_rate: u32, wake: Rc<dyn Fn()>) -> Self {
            let shared = Rc::new(RefCell::new(Shared {
                busy: true,
                ..Shared::default()
            }));
            let (task_shared, task_path, task_wake) =
                (Rc::clone(&shared), path.clone(), Rc::clone(&wake));
            wasm_bindgen_futures::spawn_local(async move {
                let opened = store::save_open(&task_path).await;
                let dropped = task_shared.borrow().dropped;
                match opened {
                    Ok(stream) if dropped => store::save_abort(&stream, &task_path).await,
                    Ok(stream) => task_shared.borrow_mut().stream = Some(stream),
                    Err(e) => task_shared.borrow_mut().failed = Some(e),
                }
                task_shared.borrow_mut().busy = false;
                task_wake();
            });
            Self {
                path,
                shared,
                blocks: WavBlocks::new(frames, sample_rate, SAVE_WRITE_BYTES),
                array: None,
                wake,
                ended: false,
            }
        }

        /// The save file, from the root of browser storage.
        pub fn path(&self) -> &str {
            &self.path
        }

        /// The end stage, which the caller runs once the job is done, until
        /// it is not [`Finish::Pending`].  It writes the last block, then
        /// closes the stream.
        #[cfg_attr(mutants, mutants::skip)]
        pub fn finish(&mut self) -> Finish {
            {
                let s = self.shared.borrow();
                if let Some(e) = &s.failed {
                    return Finish::Failed(e.clone());
                }
                if s.closed {
                    return Finish::Done;
                }
                if s.busy || s.stream.is_none() || self.ended {
                    return Finish::Pending;
                }
            }
            self.ended = true;
            let rest = self.blocks.rest();
            // A new array of its own length, never a view (MEM-5).
            let last = (!rest.is_empty()).then(|| Uint8Array::from(rest));
            self.send(Op::Last(last));
            Finish::Pending
        }

        /// Starts `op` on the stream, which must be open and idle.
        #[cfg_attr(mutants, mutants::skip)]
        fn send(&self, op: Op) {
            let stream = {
                let mut s = self.shared.borrow_mut();
                s.busy = true;
                s.stream.clone().expect("an open stream")
            };
            let (shared, path, wake) = (
                Rc::clone(&self.shared),
                self.path.clone(),
                Rc::clone(&self.wake),
            );
            wasm_bindgen_futures::spawn_local(async move {
                // A failed close removed the file and ended the stream.
                let (written, closing) = match op {
                    Op::Put(bytes) => (store::save_put(&stream, &bytes).await, false),
                    Op::Last(bytes) => {
                        let put = match bytes {
                            Some(bytes) => store::save_put(&stream, &bytes).await,
                            None => Ok(()),
                        };
                        match put {
                            Ok(()) => (store::save_close(&stream, &path).await, true),
                            Err(e) => (Err(e), false),
                        }
                    }
                };
                let dropped = shared.borrow().dropped;
                match written {
                    Ok(()) if closing => {
                        if dropped {
                            store::remove_file(&path).await;
                        }
                        shared.borrow_mut().closed = true;
                    }
                    Ok(()) => {
                        if dropped {
                            store::save_abort(&stream, &path).await;
                        }
                    }
                    Err(e) => {
                        if !closing {
                            store::save_abort(&stream, &path).await;
                        }
                        shared.borrow_mut().failed = Some(e);
                    }
                }
                shared.borrow_mut().busy = false;
                wake();
            });
        }
    }

    impl Sink for StoredWav {
        #[cfg_attr(mutants, mutants::skip)]
        fn write(&mut self, _at: usize, left: &[f32], right: &[f32]) {
            self.blocks.push(left, right);
            let Some(full) = self.blocks.full() else {
                return;
            };
            let array = self
                .array
                .get_or_insert_with(|| Uint8Array::new_with_length(SAVE_WRITE_BYTES as u32));
            array.copy_from(full);
            let array = array.clone();
            self.blocks.take_full();
            self.send(Op::Put(array));
        }

        #[cfg_attr(mutants, mutants::skip)]
        fn ready(&self) -> bool {
            let s = self.shared.borrow();
            s.stream.is_some() && !s.busy && s.failed.is_none() && !s.closed
        }

        #[cfg_attr(mutants, mutants::skip)]
        fn failed(&self) -> Option<String> {
            self.shared.borrow().failed.clone()
        }
    }

    /// The sink of a save: a WAV save file (the storage path of M8), or JS
    /// arrays (the memory path, decision 0015).
    pub enum SaveSink {
        Stored(StoredWav),
        Memory(super::JsSink),
    }

    impl Sink for SaveSink {
        #[cfg_attr(mutants, mutants::skip)]
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            match self {
                Self::Stored(s) => s.write(at, left, right),
                Self::Memory(s) => s.write(at, left, right),
            }
        }

        #[cfg_attr(mutants, mutants::skip)]
        fn ready(&self) -> bool {
            match self {
                Self::Stored(s) => s.ready(),
                Self::Memory(s) => s.ready(),
            }
        }

        #[cfg_attr(mutants, mutants::skip)]
        fn failed(&self) -> Option<String> {
            match self {
                Self::Stored(s) => s.failed(),
                Self::Memory(s) => s.failed(),
            }
        }
    }

    impl Drop for StoredWav {
        #[cfg_attr(mutants, mutants::skip)]
        fn drop(&mut self) {
            let mut s = self.shared.borrow_mut();
            if s.closed || s.failed.is_some() || s.dropped {
                return;
            }
            s.dropped = true;
            if s.busy {
                // The open or the write in flight ends the file.
                return;
            }
            if let Some(stream) = s.stream.clone() {
                let path = self.path.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    store::save_abort(&stream, &path).await;
                });
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

    /// A sink that takes a block only when the test says so, and can fail.
    /// It counts how often a job asks.
    #[derive(Default)]
    struct Gate {
        rec: Recorder,
        closed: std::cell::Cell<bool>,
        failure: Option<String>,
        asked: std::cell::Cell<usize>,
    }

    impl Sink for Gate {
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            assert!(!self.closed.get(), "a job writes only to a ready sink");
            self.rec.write(at, left, right);
        }
        fn ready(&self) -> bool {
            self.asked.set(self.asked.get() + 1);
            !self.closed.get()
        }
        fn failed(&self) -> Option<String> {
            self.failure.clone()
        }
    }

    /// D3 of M8: a job waits for a sink that is not ready, as for a source
    /// that is not, and asks for no time while it waits.
    #[test]
    fn a_job_waits_for_a_sink_that_is_not_ready() {
        let frames = 2 * BLOCK + 5;
        let song = Arc::new(song(&["bass", "drums"], frames));
        let recipe = MixRecipe::new(vec![1.0, 0.5]);
        let gate = Gate::default();
        gate.closed.set(true);
        let mut job = ExportJob::new(Box::new(Mix::new(Arc::clone(&song), &recipe)), gate);
        let mut asked_for_time = 0;
        let more = || {
            asked_for_time += 1;
            true
        };
        assert!(!job.run_while(more), "a job that waits is not done");
        assert_eq!(asked_for_time, 0, "it does not ask for time while it waits");
        assert_eq!(job.sink().asked.get(), 1, "it asks once: no busy loop");
        assert!(job.is_waiting());
        assert_eq!(job.fraction(), 0.0, "no progress");
        assert!(!job.step(), "a step waits too");
        assert!(job.sink().rec.starts.is_empty());

        job.sink().closed.set(false);
        assert!(job.run_while(|| true));
        assert!(!job.is_waiting());
        let want = run_mix(&song, &recipe);
        let gate = job.into_sink();
        assert_eq!(gate.rec.starts, [0, BLOCK, 2 * BLOCK]);
        assert_eq!((gate.rec.left, gate.rec.right), (want.left, want.right));
    }

    /// D3 of M8: a job that stops at a failure stops at a failed write of
    /// its sink; another job goes on.
    #[test]
    fn a_job_that_stops_at_a_failure_stops_at_a_sink_failure() {
        let frames = 2 * BLOCK + 5;
        let song = Arc::new(song(&["bass"], frames));
        let recipe = MixRecipe::new(vec![1.0]);
        let failing = || Gate {
            failure: Some("QuotaExceededError".into()),
            ..Gate::default()
        };
        let mut job = ExportJob::new(Box::new(Mix::new(Arc::clone(&song), &recipe)), failing())
            .stopping_at_a_failure();
        assert!(job.step(), "it ends at once");
        assert!(!job.is_done(), "with the source not done");
        assert!(job.sink().rec.starts.is_empty());
        let mut job = ExportJob::new(Box::new(Mix::new(Arc::clone(&song), &recipe)), failing());
        assert!(job.run_while(|| true), "a job that does not stop goes on");
        assert_eq!(job.sink().rec.starts, [0, BLOCK, 2 * BLOCK]);
    }

    /// Writes each full block of a [`WavBlocks`] as it comes, as a save's
    /// sink does.
    struct BlockRecorder {
        blocks: WavBlocks,
        written: Vec<Vec<u8>>,
    }

    impl Sink for BlockRecorder {
        fn write(&mut self, _: usize, left: &[f32], right: &[f32]) {
            self.blocks.push(left, right);
            while let Some(full) = self.blocks.full() {
                self.written.push(Vec::from(full));
                self.blocks.take_full();
            }
        }
    }

    /// D3 of M8: the blocks of a save's WAV, joined, are the WAV of the
    /// mix.  Every block but the last has the block size, and the sink
    /// never holds more than a full block and a job block.
    #[test]
    fn wav_blocks_are_the_wav_in_blocks_of_one_size() {
        let job_block = BLOCK * engine::wav::FRAME_BYTES;
        for (frames, size) in [
            (2 * BLOCK + 5, 100_000),
            (5 * BLOCK, job_block),
            (4 * BLOCK, 4 * job_block),
            (3, SAVE_WRITE_BYTES),
            (0, SAVE_WRITE_BYTES),
        ] {
            let song = Arc::new(song(&["bass", "drums"], frames));
            let recipe = MixRecipe::new(vec![1.0, 0.5]);
            let want = run_mix(&song, &recipe);
            let want = engine::wav::encode_pcm16(&want.left, &want.right, 48_000);
            let sink = BlockRecorder {
                blocks: WavBlocks::new(frames, 48_000, size),
                written: Vec::new(),
            };
            let mut job = ExportJob::new(Box::new(Mix::new(Arc::clone(&song), &recipe)), sink);
            assert!(job.run_while(|| true));
            let sink = job.into_sink();
            assert!(
                sink.written.iter().all(|b| b.len() == size),
                "{frames} frames in blocks of {size}"
            );
            assert!(sink.blocks.rest().len() < size, "{frames} frames");
            let joined: Vec<u8> = sink
                .written
                .iter()
                .flatten()
                .chain(sink.blocks.rest())
                .copied()
                .collect();
            assert!(joined == want, "{frames} frames in blocks of {size}");
            assert!(
                sink.blocks.pending.capacity() <= size + job_block,
                "{frames} frames: it held {} bytes",
                sink.blocks.pending.capacity()
            );
        }
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
#[cfg_attr(mutants, mutants::skip)]
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
