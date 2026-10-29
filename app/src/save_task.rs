//! The task loop of a save (D11 of `plans/active/stream-from-storage-m8.md`).
//!
//! A save's [`ExportJob`] runs in a task of its own, not in the UI frame.
//! The task runs the job in slices of at most a budget ([`Budget`]: 4 ms,
//! or up to 16 ms after a yield that waited long), and yields to the event
//! loop with a macrotask after each slice.  So the audio callback and the
//! UI run between slices.  When the job waits for a
//! read or a write, the task waits for the [`Signal`] that the read's or
//! the write's end gives, not for a repaint.  So the save's speed does not
//! depend on the frame rate.
//!
//! The task holds the job weakly.  The page's `Saving` holds it strongly,
//! with a [`Stop`] that wakes the task when the page drops the save.  So a
//! dropped save drops its job at once (D3 of M8 removes its file), and the
//! task ends at its next slice.  When the job is done, the task calls
//! `ended` (a repaint), and again at each signal until the page drops the
//! save: the UI frame runs the end stage, whose writes signal too.

use crate::export::{ExportJob, Sink};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll, Waker};

/// The end of a read or a write that a save's task waits for.
#[derive(Default)]
pub struct Signal {
    woken: Cell<bool>,
    waker: RefCell<Option<Waker>>,
    /// The longest slice of the task so far, in milliseconds.
    longest_ms: Cell<f64>,
    /// The longest end of a read in the reader so far, in milliseconds.
    longest_read_end_ms: Cell<f64>,
}

impl Signal {
    /// The longest slice of the save's task so far, in milliseconds, as
    /// its clock measured it (review round speed 2 of M8).
    pub fn longest_ms(&self) -> f64 {
        self.longest_ms.get()
    }

    /// The longest time that the end of a read took in the save's reader
    /// so far, in milliseconds ([`crate::lane_store::WakingStore`]).
    pub fn longest_read_end_ms(&self) -> f64 {
        self.longest_read_end_ms.get()
    }

    /// Keeps `ms`, the time of the end of a read, if it is the longest.
    pub fn note_read_end(&self, ms: f64) {
        self.longest_read_end_ms
            .set(self.longest_read_end_ms.get().max(ms));
    }

    /// Says that a read or a write ended, and wakes the task.
    pub fn notify(&self) {
        self.woken.set(true);
        let waker = self.waker.borrow_mut().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Forgets the ends signalled before now.
    fn clear(&self) {
        self.woken.set(false);
    }

    /// Resolves at the first end signalled since the last [`Self::clear`].
    fn wait(&self) -> Wait<'_> {
        Wait(self)
    }
}

struct Wait<'a>(&'a Signal);

impl Future for Wait<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0.woken.replace(false) {
            return Poll::Ready(());
        }
        *self.0.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Wakes the task of a save when the page drops the save, so the task
/// ends at once and not at the next read's end.
pub struct Stop(pub Rc<Signal>);

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.notify();
    }
}

/// The time budget of each slice of a save's task, in milliseconds.  A
/// slice after a yield may take `share` of the time that the yield took,
/// within `least_ms` and `most_ms`.  A slice after a wait for a read or a
/// write takes `least_ms`: a slow read is not a long frame (finding 4-12
/// of review round speed 4 of M8).  When the page's frames are short, the
/// slices take `least_ms`.  When a frame holds the main thread for long,
/// as a paint in software on CI does (about 125 ms in CI run 40838459678),
/// each yield to the event loop waits for that frame.  Then a slice takes
/// more, up to `most_ms`, so the save goes on at a part of the page's time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    pub least_ms: f64,
    pub most_ms: f64,
    pub share: f64,
}

impl Budget {
    /// The budget of a slice that starts `gap_ms` after the last slice
    /// ended.
    pub fn after(&self, gap_ms: f64) -> f64 {
        (gap_ms * self.share).clamp(self.least_ms, self.most_ms)
    }
}

/// Runs `job` in slices of the time that `budget` gives, as `now`
/// measures them (milliseconds), until the page drops the job.  After a
/// slice that ran out of time, it awaits `pause()`, a yield to the event
/// loop.  After a slice that waits for a read or a write, it awaits
/// `signal`.  When the job is done, or stopped at a failure, it calls
/// `ended` and awaits `signal`, again and again.  A slice runs at least one
/// step, so it ends within its budget and one block of work.  The signal
/// keeps the longest slice ([`Signal::longest_ms`]).
pub async fn drive<S, P>(
    job: Weak<RefCell<ExportJob<S>>>,
    signal: Rc<Signal>,
    budget: Budget,
    now: impl Fn() -> f64,
    mut pause: impl FnMut() -> P,
    ended: impl Fn(),
) where
    S: Sink,
    P: Future<Output = ()>,
{
    // When the last slice yielded: the next slice's budget comes from the
    // time that the yield took.
    let mut yielded_at = None;
    loop {
        let Some(job) = job.upgrade() else {
            return;
        };
        signal.clear();
        let (done, waiting, stop) = {
            let mut job = job.borrow_mut();
            let start = now();
            let gap = yielded_at.take().map_or(0.0, |at| start - at);
            let end = start + budget.after(gap);
            let done = job.run_while(|| now() < end);
            let stop = now();
            signal
                .longest_ms
                .set(signal.longest_ms.get().max(stop - start));
            (done, job.is_waiting(), stop)
        };
        drop(job);
        if done {
            ended();
            signal.wait().await;
        } else if waiting {
            signal.wait().await;
        } else {
            yielded_at = Some(stop);
            pause().await;
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{page_now, spawn};

/// The task loop on the web.
#[cfg(target_arch = "wasm32")]
mod web {
    use super::{Signal, drive};
    use crate::export::{ExportJob, Sink};
    use eframe::egui;
    use std::cell::RefCell;
    use std::rc::{Rc, Weak};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
// A macrotask: a message to a channel of its own.  Browsers clamp nested
// setTimeout calls to 4 ms, but not messages.
export function st_yield() {
  return new Promise((resolve) => {
    const c = new MessageChannel();
    c.port1.onmessage = () => { c.port1.close(); resolve(); };
    c.port2.postMessage(null);
  });
}
// The page's clock in milliseconds, in a window or in a worker.
export function st_now() { return performance.now(); }
"#)]
    extern "C" {
        fn st_yield() -> js_sys::Promise;
        fn st_now() -> f64;
    }

    /// The page's clock, in milliseconds.
    #[cfg_attr(mutants, mutants::skip)]
    pub fn page_now() -> f64 {
        st_now()
    }

    /// Starts the task loop of a save's `job` ([`drive`]), with slices of
    /// `budget`, a macrotask between slices, and a repaint of `ctx` at the
    /// job's end.
    #[cfg_attr(mutants, mutants::skip)]
    pub fn spawn<S: Sink + 'static>(
        job: Weak<RefCell<ExportJob<S>>>,
        signal: Rc<Signal>,
        budget: super::Budget,
        ctx: Option<egui::Context>,
    ) {
        wasm_bindgen_futures::spawn_local(drive(
            job,
            signal,
            budget,
            st_now,
            || async {
                // A macrotask's promise does not fail.
                let _ = wasm_bindgen_futures::JsFuture::from(st_yield()).await;
            },
            move || {
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{BLOCK, SAVE_WRITE_BYTES, WavBlocks};
    use crate::lane_store::{
        Ahead, ByteRange, LaneStore, MemoryStore, ReadDone, SAVE_AHEAD, WakingStore,
    };
    use crate::lanes::{LaneAudio, LanesMix, Overview};
    use crate::store_meta::LaneFile;
    use engine::mixdown::MixRecipe;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Wake;

    /// A fake clock in milliseconds, which only work and frames move.
    type Clock = Rc<Cell<f64>>;

    /// The frames that a read of browser storage takes, and a write.
    const READ_FRAMES: usize = 2;
    const WRITE_FRAMES: usize = 2;

    /// The page of CI in a model (CI run 40838459678, review round speed 3
    /// of M8).  Each frame holds the main thread for `frame_ms`, and a
    /// task queued during a frame runs after it.  So a yield waits for one
    /// frame, and a read or a write of browser storage takes two.  On the
    /// maintainer's Mac in Chrome, with frames of 125 ms, 10 yields took
    /// 1,374 ms, a read of up to 2 MiB about 275 ms, and a write of up to
    /// 8 MiB about 300 ms.
    struct Page {
        frame_ms: f64,
        frame: Cell<usize>,
        clock: Clock,
        /// The frame that each read held back started in, in order.
        reads: RefCell<VecDeque<usize>>,
        /// The frame that each write in flight started in, in order.
        writes: RefCell<VecDeque<usize>>,
        /// The waker of the task's yield, which waits for the next frame.
        paused: RefCell<Option<Waker>>,
    }

    /// A store that notes the frame that each read starts in.
    struct Timed {
        page: Rc<Page>,
        inner: Rc<MemoryStore>,
    }

    impl LaneStore for Timed {
        fn read(&self, ranges: Vec<ByteRange>, done: ReadDone) {
            let frame = self.page.frame.get();
            self.page.reads.borrow_mut().push_back(frame);
            self.inner.read(ranges, done);
        }
    }

    /// A yield to the next frame of the page.
    struct NextFrame {
        page: Rc<Page>,
        after: usize,
    }

    impl Future for NextFrame {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.page.frame.get() > self.after {
                return Poll::Ready(());
            }
            *self.page.paused.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    }

    /// What the test sees of the save: the start of each block written,
    /// and whether a read was in flight then.
    #[derive(Default)]
    struct Seen {
        starts: Vec<usize>,
        read_in_flight: Vec<bool>,
    }

    /// A WAV sink in the model, with the rule of `StoredWav`: real
    /// [`WavBlocks`] of `write_bytes`, and a write of each full block,
    /// which takes [`WRITE_FRAMES`].  Each block costs `block_ms` of the
    /// clock.  With `overlap`, the sink takes blocks while a write is in
    /// flight, as [`WavBlocks::takes_more`] says; with no `overlap`, it
    /// takes none.  A write starts at a block that finds no write in flight.
    struct Writes {
        page: Rc<Page>,
        block_ms: f64,
        blocks: WavBlocks,
        overlap: bool,
        busy: Rc<Cell<bool>>,
        store: Rc<MemoryStore>,
        seen: Rc<RefCell<Seen>>,
    }

    impl Sink for Writes {
        fn write(&mut self, at: usize, left: &[f32], right: &[f32]) {
            let clock = &self.page.clock;
            clock.set(clock.get() + self.block_ms);
            let mut seen = self.seen.borrow_mut();
            seen.starts.push(at);
            seen.read_in_flight.push(self.store.held_reads() > 0);
            self.blocks.push(left, right);
            if !self.busy.get() && self.blocks.full().is_some() {
                self.blocks.take_full();
                self.busy.set(true);
                let frame = self.page.frame.get();
                self.page.writes.borrow_mut().push_back(frame);
            }
        }

        fn ready(&self) -> bool {
            if self.overlap {
                self.blocks.takes_more(self.busy.get())
            } else {
                !self.busy.get()
            }
        }
    }

    /// Wakes by setting a flag.
    struct Flag(AtomicBool);

    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// A yield that returns once, after its waker is called, as a
    /// macrotask resolves in a later turn of the event loop.
    struct Yield(bool);

    impl Future for Yield {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                return Poll::Ready(());
            }
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    /// Two f32 lane files of `frames` frames in `store`, as stored lanes.
    fn lanes(store: &Rc<MemoryStore>, frames: usize) -> Vec<LaneAudio> {
        let shared: Rc<dyn LaneStore> = Rc::clone(store) as Rc<dyn LaneStore>;
        (0..2)
            .map(|k| {
                let path = format!("acme/{k}.f32");
                let bytes = (0..frames)
                    .flat_map(|i| {
                        let v = ((i + k) % 7) as f32 / 10.0;
                        [v.to_le_bytes(), (-v).to_le_bytes()]
                    })
                    .flatten()
                    .collect();
                store.put(&path, bytes);
                LaneAudio::Stored {
                    file: LaneFile::f32(path, frames),
                    overview: Rc::new(Overview {
                        columns: Vec::new(),
                        scale: 1.0,
                    }),
                    store: Rc::clone(&shared),
                }
            })
            .collect()
    }

    /// A waker that sets a flag, and the flag.
    fn flag_waker() -> (Arc<Flag>, Waker) {
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&flag));
        (flag, waker)
    }

    /// How a save runs in the model: its reader, its budget, the work of a
    /// block, the blocks of a write, and the page's frame.
    struct Model {
        ahead: Ahead,
        budget: Budget,
        block_ms: f64,
        write_bytes: usize,
        overlap: bool,
        frame_ms: f64,
    }

    /// The save of a page today (review round speed 3 of M8), on CI.  CI's
    /// longest slice of one block was 1.2 ms (CI run 40838459678).
    const TODAY: Model = Model {
        ahead: SAVE_AHEAD,
        budget: crate::SAVE_BUDGET,
        block_ms: 1.2,
        write_bytes: SAVE_WRITE_BYTES,
        overlap: true,
        frame_ms: 125.0,
    };

    /// What a run of the task gave.
    struct Run {
        seen: Seen,
        ended: usize,
        done: bool,
        reads: usize,
        /// The page's frames from the start to the end of the job.
        frames: usize,
        /// The longest slice, in milliseconds.
        longest_ms: f64,
    }

    /// Runs the task of a save of the mix of two stored lanes of `frames`
    /// frames on the page of the model, frame by frame.  At each frame,
    /// the yield of the frame before returns, and the writes and the reads
    /// that started [`WRITE_FRAMES`] and [`READ_FRAMES`] before end.  No UI
    /// frame of the app runs.
    fn run(frames: usize, model: &Model) -> Run {
        let store = MemoryStore::new();
        let audio = lanes(&store, frames);
        store.hold(true);
        let page = Rc::new(Page {
            frame_ms: model.frame_ms,
            frame: Cell::new(0),
            clock: Rc::default(),
            reads: RefCell::default(),
            writes: RefCell::default(),
            paused: RefCell::default(),
        });
        let signal = Rc::new(Signal::default());
        let timed: Rc<dyn LaneStore> = Rc::new(Timed {
            page: Rc::clone(&page),
            inner: Rc::clone(&store),
        });
        let read_clock = Rc::clone(&page.clock);
        let waking: Rc<dyn LaneStore> = Rc::new(WakingStore::new(
            timed,
            Rc::clone(&signal),
            Rc::new(move || read_clock.get()),
        ));
        let audio: Vec<LaneAudio> = audio.iter().map(|a| a.read_through(&waking)).collect();
        let recipe = MixRecipe::new(vec![1.0, 0.5]);
        let seen = Rc::new(RefCell::new(Seen::default()));
        let busy = Rc::new(Cell::new(false));
        let sink = Writes {
            page: Rc::clone(&page),
            block_ms: model.block_ms,
            blocks: WavBlocks::new(frames, 44_100, model.write_bytes),
            overlap: model.overlap,
            busy: Rc::clone(&busy),
            store: Rc::clone(&store),
            seen: Rc::clone(&seen),
        };
        let source = LanesMix::reading_ahead(&audio, frames, 44_100, &recipe, model.ahead);
        let job = Rc::new(RefCell::new(
            ExportJob::new(Box::new(source), sink).stopping_at_a_failure(),
        ));
        let ended = Rc::new(Cell::new(0));
        let ended_count = Rc::clone(&ended);
        let (clock, yields) = (Rc::clone(&page.clock), Rc::clone(&page));
        let mut task = Box::pin(drive(
            Rc::downgrade(&job),
            Rc::clone(&signal),
            model.budget,
            move || clock.get(),
            move || NextFrame {
                page: Rc::clone(&yields),
                after: yields.frame.get(),
            },
            move || ended_count.set(ended_count.get() + 1),
        ));
        let (flag, waker) = flag_waker();
        let mut cx = Context::from_waker(&waker);
        flag.0.store(true, Ordering::SeqCst);
        while ended.get() == 0 {
            assert!(page.frame.get() < 100_000, "the save ends");
            if flag.0.swap(false, Ordering::SeqCst) {
                assert!(task.as_mut().poll(&mut cx).is_pending(), "the task runs on");
                continue;
            }
            let frame = page.frame.get() + 1;
            page.frame.set(frame);
            page.clock.set(page.clock.get() + page.frame_ms);
            if let Some(paused) = page.paused.borrow_mut().take() {
                paused.wake();
            }
            while page
                .writes
                .borrow()
                .front()
                .is_some_and(|&at| at + WRITE_FRAMES <= frame)
            {
                page.writes.borrow_mut().pop_front();
                busy.set(false);
                signal.notify();
            }
            let due = {
                let mut reads = page.reads.borrow_mut();
                let due = reads
                    .iter()
                    .take_while(|&&at| at + READ_FRAMES <= frame)
                    .count();
                reads.drain(..due);
                due
            };
            store.release_first(due);
        }
        let done = job.borrow().is_done();
        drop(job);
        // The page dropped the save, and its `Stop` signals: the task ends.
        let longest_ms = signal.longest_ms();
        drop(Stop(signal));
        assert!(
            task.as_mut().poll(&mut cx).is_ready(),
            "a dropped save ends its task"
        );
        Run {
            seen: std::mem::take(&mut *seen.borrow_mut()),
            ended: ended.get(),
            done,
            reads: store.requests(),
            frames: page.frame.get(),
            longest_ms,
        }
    }

    /// D11 of M8: a save ends with no UI frame.  Its task writes every
    /// block once, in order, and reads each span of the song once, with
    /// reads in flight while it mixes.  Then it asks once for the frame that
    /// ends the save.
    #[test]
    fn a_save_runs_to_its_end_in_its_task() {
        let blocks = 40;
        let r = run(blocks * BLOCK + 100, &TODAY);
        assert!(r.done, "the job is done");
        assert_eq!(r.ended, 1, "one call at the end");
        let starts: Vec<usize> = (0..=blocks).map(|k| k * BLOCK).collect();
        assert_eq!(r.seen.starts, starts, "each block once, in order");
        let spans = (blocks * BLOCK + 100).div_ceil(SAVE_AHEAD.span);
        assert_eq!(r.reads, spans, "one read for each span, and no other");
        let with_reads = r.seen.read_in_flight.iter().filter(|&&f| f).count();
        assert!(
            with_reads >= blocks / 2,
            "reads go on while the job mixes: {with_reads} of {blocks} blocks"
        );
    }

    /// The seconds that a save of 10 minutes (1,615 blocks) takes in the
    /// model, from a run of `blocks` blocks: its frames for each block.
    fn ten_minutes_s(blocks: usize, model: &Model) -> f64 {
        let r = run(blocks * BLOCK, model);
        assert!(r.done);
        let frame_s = (model.frame_ms + r.longest_ms) / 1000.0;
        r.frames as f64 / blocks as f64 * 1615.0 * frame_s
    }

    /// CI run 40838459678: during playback, each frame of the page held
    /// the main thread for about 125 ms, and a save of 10 minutes did not
    /// end within 180 s.  It needed two frames for each block's read, one
    /// for each yield, and two for each write of 1 MiB, while the job
    /// waited.  Today the reader keeps 2 spans of 4 blocks in flight (review
    /// round speed 4 of M8).  So the save takes a minute or less, and the
    /// reads in flight set its time: with slices of 4 ms, or with writes
    /// that stop the job, it takes about the same.
    #[test]
    fn a_save_with_long_frames_takes_a_minute_or_less() {
        let blocks = 128;
        let today = ten_minutes_s(blocks, &TODAY);
        eprintln!("today: {today:.1} s");
        assert!(today <= 60.0, "today: {today:.1} s");
        let one_block = Model {
            ahead: Ahead {
                span: BLOCK,
                depth: 1,
                most_bytes: usize::MAX,
            },
            ..TODAY
        };
        let before = Model {
            ahead: one_block.ahead,
            budget: Budget {
                least_ms: 4.0,
                most_ms: 4.0,
                share: 0.0,
            },
            write_bytes: 1 << 20,
            overlap: false,
            ..TODAY
        };
        for (what, model) in [("one block ahead", one_block), ("the save before", before)] {
            let s = ten_minutes_s(blocks, &model);
            eprintln!("{what}: {s:.1} s");
            assert!(s > 300.0, "{what}: {s:.1} s");
        }
    }

    /// A sink whose every block costs `ms` of its clock.
    struct Ticks {
        clock: Clock,
        ms: f64,
    }

    impl Sink for Ticks {
        fn write(&mut self, _: usize, _: &[f32], _: &[f32]) {
            self.clock.set(self.clock.get() + self.ms);
        }
    }

    /// A budget of `ms` for each slice.
    fn fixed(ms: f64) -> Budget {
        Budget {
            least_ms: ms,
            most_ms: ms,
            share: 0.0,
        }
    }

    /// The fake times of the yields of a save of `blocks` blocks in memory,
    /// each block `block_ms` of work, with slices of `budget`.  Each yield
    /// takes `pause_ms` of the clock.
    fn yields_of(blocks: usize, budget: Budget, block_ms: f64, pause_ms: f64) -> (Vec<f64>, f64) {
        let frames = blocks * BLOCK;
        let clock: Clock = Rc::default();
        let job = Rc::new(RefCell::new(ExportJob::new(
            Box::new(engine::mixdown::StemPcm::new(
                engine::Stem {
                    name: "bass".into(),
                    left: vec![0.25; frames].into(),
                    right: vec![0.25; frames].into(),
                },
                44_100,
            )),
            Ticks {
                clock: Rc::clone(&clock),
                ms: block_ms,
            },
        )));
        let signal = Rc::new(Signal::default());
        let yields = Rc::new(RefCell::new(Vec::new()));
        let (y, c) = (Rc::clone(&yields), Rc::clone(&clock));
        let ended = Rc::new(Cell::new(0));
        let e = Rc::clone(&ended);
        let mut task = Box::pin(drive(
            Rc::downgrade(&job),
            Rc::clone(&signal),
            budget,
            move || clock.get(),
            move || {
                y.borrow_mut().push(c.get());
                c.set(c.get() + pause_ms);
                Yield(false)
            },
            move || e.set(e.get() + 1),
        ));
        let (flag, waker) = flag_waker();
        let mut cx = Context::from_waker(&waker);
        for _ in 0..1_000 {
            assert!(task.as_mut().poll(&mut cx).is_pending());
            if ended.get() > 0 {
                break;
            }
            assert!(
                flag.0.swap(false, Ordering::SeqCst),
                "a yield wakes the task"
            );
        }
        assert_eq!(ended.get(), 1, "the job ended");
        // The job is done: a signal, such as the end stage's write, asks
        // for another frame, until the page drops the save.
        signal.notify();
        assert!(task.as_mut().poll(&mut cx).is_pending());
        assert_eq!(ended.get(), 2);
        drop(Stop(Rc::clone(&signal)));
        assert!(task.as_mut().poll(&mut cx).is_pending(), "the job lives");
        drop(job);
        let longest = signal.longest_ms();
        drop(Stop(signal));
        assert!(
            task.as_mut().poll(&mut cx).is_ready(),
            "a dropped save ends its task"
        );
        (yields.take(), longest)
    }

    /// D11 of M8: no slice runs longer than its budget and one block of
    /// work, and each slice that runs out of time yields to the event loop.
    #[test]
    fn a_slice_ends_within_its_budget_and_one_block() {
        // 1 ms a block, 4 ms a slice: 4 blocks, then a yield.
        let (yields, longest) = yields_of(30, fixed(4.0), 1.0, 0.0);
        assert_eq!(yields, [4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 28.0]);
        assert_eq!(longest, 4.0, "the longest slice, as the task measured it");
        // 3 ms a block: the second block ends the slice at 6 ms.
        let (yields, longest) = yields_of(30, fixed(4.0), 3.0, 0.0);
        assert_eq!(longest, 6.0);
        assert_eq!(yields.len(), 14);
        let mut start = 0.0;
        for at in yields {
            assert!(
                at - start <= 4.0 + 3.0,
                "a slice from {start} ms to {at} ms"
            );
            start = at;
        }
    }

    /// Review round speed 3 of M8: a slice takes a share of the time since
    /// the last slice, within the least and the most of its budget.
    #[test]
    fn a_budget_is_a_share_of_the_gap_within_its_bounds() {
        let budget = Budget {
            least_ms: 4.0,
            most_ms: 12.0,
            share: 0.125,
        };
        assert_eq!(budget.after(0.0), 4.0);
        assert_eq!(budget.after(32.0), 4.0);
        assert_eq!(budget.after(64.0), 8.0);
        assert_eq!(budget.after(96.0), 12.0);
        assert_eq!(budget.after(125.0), 12.0);
    }

    /// Review round speed 3 of M8: when each yield waits long, as for a
    /// long frame of the page, the slices grow to a share of the wait, and
    /// never past the most of the budget and one block.
    #[test]
    fn a_slice_grows_after_a_long_wait() {
        let budget = Budget {
            least_ms: 4.0,
            most_ms: 12.0,
            share: 0.125,
        };
        // A yield of 64 ms: the first slice takes 4 ms, the others 8 ms.
        let (yields, longest) = yields_of(30, budget, 1.0, 64.0);
        assert_eq!(yields, [4.0, 76.0, 148.0, 220.0]);
        assert_eq!(longest, 8.0);
        // A yield of 200 ms: slices of 12 ms, the most.
        let (yields, longest) = yields_of(30, budget, 1.0, 200.0);
        assert_eq!(yields, [4.0, 216.0, 428.0]);
        assert_eq!(longest, 12.0);
    }

    /// Finding 4-7 of review round speed 4 of M8: the page's budget takes
    /// 4 ms after a short yield, and up to 16 ms after a yield of a long
    /// frame (about 137 ms with frames of 125 ms on the maintainer's Mac).
    #[test]
    fn the_page_budget_grows_only_after_a_long_yield() {
        let budget = crate::SAVE_BUDGET;
        assert_eq!(budget.after(0.0), crate::SAVE_MS);
        assert_eq!(budget.after(16.7), crate::SAVE_MS, "a frame of 60 Hz");
        assert_eq!(budget.after(64.0), 8.0);
        assert_eq!(budget.after(137.0), 16.0);
    }

    /// A sink whose every block costs `ms` of its clock, and which is not
    /// ready once, after `ready_after` blocks, until the test opens it.
    struct Gate {
        clock: Clock,
        ms: f64,
        blocks: usize,
        ready_after: usize,
        open: Rc<Cell<bool>>,
    }

    impl Sink for Gate {
        fn write(&mut self, _: usize, _: &[f32], _: &[f32]) {
            self.clock.set(self.clock.get() + self.ms);
            self.blocks += 1;
            if self.blocks == self.ready_after {
                self.open.set(false);
            }
        }

        fn ready(&self) -> bool {
            self.open.get()
        }
    }

    /// Finding 4-12 of review round speed 4 of M8: a long wait for a read
    /// or a write is not a long frame.  The slice after it takes the least
    /// of its budget, so a slow read does not make a slice of 16 ms.
    #[test]
    fn a_slice_after_a_long_wait_for_a_write_takes_the_least() {
        let frames = 30 * BLOCK;
        let clock: Clock = Rc::default();
        let open = Rc::new(Cell::new(true));
        let job = Rc::new(RefCell::new(ExportJob::new(
            Box::new(engine::mixdown::StemPcm::new(
                engine::Stem {
                    name: "bass".into(),
                    left: vec![0.25; frames].into(),
                    right: vec![0.25; frames].into(),
                },
                44_100,
            )),
            Gate {
                clock: Rc::clone(&clock),
                ms: 1.0,
                blocks: 0,
                ready_after: 2,
                open: Rc::clone(&open),
            },
        )));
        let signal = Rc::new(Signal::default());
        let c = Rc::clone(&clock);
        let budget = Budget {
            least_ms: 4.0,
            most_ms: 16.0,
            share: 0.125,
        };
        let mut task = Box::pin(drive(
            Rc::downgrade(&job),
            Rc::clone(&signal),
            budget,
            move || c.get(),
            || Yield(false),
            || {},
        ));
        let (_flag, waker) = flag_waker();
        let mut cx = Context::from_waker(&waker);
        assert!(task.as_mut().poll(&mut cx).is_pending());
        assert_eq!(signal.longest_ms(), 2.0, "2 blocks, then the write");
        // The write takes 200 ms, then ends.
        clock.set(clock.get() + 200.0);
        open.set(true);
        signal.notify();
        assert!(task.as_mut().poll(&mut cx).is_pending());
        assert_eq!(signal.longest_ms(), 4.0, "a slice of the least budget");
        drop(job);
        drop(Stop(Rc::clone(&signal)));
        assert!(task.as_mut().poll(&mut cx).is_ready());
    }

    /// Finding 4-8 of review round speed 4 of M8: the signal keeps the
    /// longest end of a read through a `WakingStore`, as its clock measures
    /// it, and each end wakes the task.
    #[test]
    fn a_waking_store_keeps_the_longest_read_end() {
        let store = MemoryStore::new();
        store.put("acme/0.f32", vec![0; 64]);
        store.hold(true);
        let signal = Rc::new(Signal::default());
        let clock: Clock = Rc::default();
        let c = Rc::clone(&clock);
        let waking = WakingStore::new(
            Rc::clone(&store) as Rc<dyn LaneStore>,
            Rc::clone(&signal),
            Rc::new(move || c.get()),
        );
        for ms in [3.0, 7.0, 5.0] {
            let c = Rc::clone(&clock);
            let range = ByteRange {
                path: "acme/0.f32".into(),
                start: 0,
                end: 64,
            };
            waking.read(
                vec![range],
                Box::new(move |result| {
                    assert!(result.is_ok());
                    c.set(c.get() + ms);
                }),
            );
        }
        assert_eq!(signal.longest_read_end_ms(), 0.0, "no read ended");
        store.release();
        assert_eq!(signal.longest_read_end_ms(), 7.0);
        let (_flag, waker) = flag_waker();
        let mut cx = Context::from_waker(&waker);
        assert!(
            Pin::new(&mut signal.wait()).poll(&mut cx).is_ready(),
            "a read's end wakes"
        );
    }

    /// A signal before the wait is not lost, and one wait takes it.
    #[test]
    fn a_signal_before_the_wait_is_kept_once() {
        let signal = Signal::default();
        let (flag, waker) = flag_waker();
        let mut cx = Context::from_waker(&waker);
        signal.notify();
        assert!(Pin::new(&mut signal.wait()).poll(&mut cx).is_ready());
        let mut wait = signal.wait();
        assert!(Pin::new(&mut wait).poll(&mut cx).is_pending());
        signal.notify();
        assert!(flag.0.load(Ordering::SeqCst), "the waker is called");
        assert!(Pin::new(&mut wait).poll(&mut cx).is_ready());
        signal.notify();
        signal.clear();
        assert!(Pin::new(&mut signal.wait()).poll(&mut cx).is_pending());
    }
}
