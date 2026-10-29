//! The task loop of a save (D11 of `plans/active/stream-from-storage-m8.md`).
//!
//! A save's [`ExportJob`] runs in a task of its own, not in the UI frame.
//! The task runs the job in slices of at most a budget (`SAVE_MS`), and
//! yields to the event loop with a macrotask after each slice.  So the
//! audio callback and the UI run between slices.  When the job waits for a
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
}

impl Signal {
    /// The longest slice of the save's task so far, in milliseconds, as
    /// its clock measured it (review round speed 2 of M8).
    pub fn longest_ms(&self) -> f64 {
        self.longest_ms.get()
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

/// Runs `job` in slices of at most `budget_ms`, as `now` measures them
/// (milliseconds), until the page drops the job.  After a slice that ran
/// out of time, it awaits `pause()`, a yield to the event loop.  After a
/// slice that waits for a read or a write, it awaits `signal`.  When the
/// job is done, or stopped at a failure, it calls `ended` and awaits
/// `signal`, again and again.  A slice runs at least one step, so it ends
/// within the budget and one block of work.  The signal keeps the longest
/// slice ([`Signal::longest_ms`]).
pub async fn drive<S, P>(
    job: Weak<RefCell<ExportJob<S>>>,
    signal: Rc<Signal>,
    budget_ms: f64,
    now: impl Fn() -> f64,
    mut pause: impl FnMut() -> P,
    ended: impl Fn(),
) where
    S: Sink,
    P: Future<Output = ()>,
{
    loop {
        let Some(job) = job.upgrade() else {
            return;
        };
        signal.clear();
        let (done, waiting) = {
            let mut job = job.borrow_mut();
            let start = now();
            let end = start + budget_ms;
            let done = job.run_while(|| now() < end);
            let took = now() - start;
            signal.longest_ms.set(signal.longest_ms.get().max(took));
            (done, job.is_waiting())
        };
        drop(job);
        if done {
            ended();
            signal.wait().await;
        } else if waiting {
            signal.wait().await;
        } else {
            pause().await;
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::spawn;

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

    /// Starts the task loop of a save's `job` ([`drive`]), with slices of
    /// `budget_ms`, a macrotask between slices, and a repaint of `ctx` at
    /// the job's end.
    #[cfg_attr(mutants, mutants::skip)]
    pub fn spawn<S: Sink + 'static>(
        job: Weak<RefCell<ExportJob<S>>>,
        signal: Rc<Signal>,
        budget_ms: f64,
        ctx: Option<egui::Context>,
    ) {
        wasm_bindgen_futures::spawn_local(drive(
            job,
            signal,
            budget_ms,
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
    use crate::export::BLOCK;
    use crate::lane_store::{LaneStore, MemoryStore, WakingStore};
    use crate::lanes::{LaneAudio, LanesMix, Overview};
    use crate::store_meta::LaneFile;
    use engine::mixdown::MixRecipe;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Wake;

    /// A fake clock in milliseconds, which only work moves.
    type Clock = Rc<Cell<f64>>;

    /// What the test sees of the save: the start of each block written,
    /// the time of each write, and whether a read was in flight then.
    #[derive(Default)]
    struct Seen {
        starts: Vec<usize>,
        next_read_in_flight: Vec<bool>,
    }

    /// A sink whose every block costs `block_ms` of the fake clock.
    struct Slow {
        clock: Clock,
        block_ms: f64,
        store: Rc<MemoryStore>,
        seen: Rc<RefCell<Seen>>,
    }

    impl Sink for Slow {
        fn write(&mut self, at: usize, _: &[f32], _: &[f32]) {
            self.clock.set(self.clock.get() + self.block_ms);
            let mut seen = self.seen.borrow_mut();
            seen.starts.push(at);
            seen.next_read_in_flight.push(self.store.held_reads() == 1);
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

    /// What a run of the task gave.
    struct Run {
        seen: Seen,
        ended: usize,
        done: bool,
        reads: usize,
        polls: usize,
    }

    /// A waker that sets a flag, and the flag.
    fn flag_waker() -> (Arc<Flag>, Waker) {
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&flag));
        (flag, waker)
    }

    /// Runs the task of a save of the mix of two stored lanes of `frames`
    /// frames.  Every read completes later: only when the task waits, the
    /// test completes the reads in flight.  No UI frame runs.
    fn run(frames: usize) -> Run {
        let store = MemoryStore::new();
        let audio = lanes(&store, frames);
        store.hold(true);
        let signal = Rc::new(Signal::default());
        let wake_signal = Rc::clone(&signal);
        let waking: Rc<dyn LaneStore> = Rc::new(WakingStore::new(
            Rc::clone(&store) as Rc<dyn LaneStore>,
            Rc::new(move || wake_signal.notify()),
        ));
        let audio: Vec<LaneAudio> = audio.iter().map(|a| a.read_through(&waking)).collect();
        let recipe = MixRecipe::new(vec![1.0, 0.5]);
        let clock: Clock = Rc::default();
        let seen = Rc::new(RefCell::new(Seen::default()));
        let sink = Slow {
            clock: Rc::clone(&clock),
            block_ms: 1.0,
            store: Rc::clone(&store),
            seen: Rc::clone(&seen),
        };
        let source = LanesMix::reading_ahead(&audio, frames, 44_100, &recipe, BLOCK);
        let job = Rc::new(RefCell::new(
            ExportJob::new(Box::new(source), sink).stopping_at_a_failure(),
        ));
        let ended = Rc::new(Cell::new(0));
        let ended_count = Rc::clone(&ended);
        let mut task = Box::pin(drive(
            Rc::downgrade(&job),
            Rc::clone(&signal),
            4.0,
            move || clock.get(),
            || Yield(false),
            move || ended_count.set(ended_count.get() + 1),
        ));
        let (flag, waker) = flag_waker();
        let mut cx = Context::from_waker(&waker);
        let mut polls = 0;
        while polls < 100_000 {
            polls += 1;
            assert!(task.as_mut().poll(&mut cx).is_pending(), "the task runs on");
            if ended.get() > 0 {
                break;
            }
            if !flag.0.swap(false, Ordering::SeqCst) {
                // The task waits for a read: it completes now.
                assert_eq!(store.held_reads(), 1, "the task waits for one read");
                store.release();
                assert!(
                    flag.0.swap(false, Ordering::SeqCst),
                    "a read's end wakes it"
                );
            }
        }
        let done = job.borrow().is_done();
        drop(job);
        // The page dropped the save, and its `Stop` signals: the task ends.
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
            polls,
        }
    }

    /// D11 of M8: a save ends with no UI frame.  Its task reads every
    /// block once, in order, with the next block's read in flight while it
    /// mixes one.  Then it asks once for the frame that ends the save.
    #[test]
    fn a_save_runs_to_its_end_in_its_task() {
        let r = run(5 * BLOCK + 100);
        assert!(r.done, "the job is done");
        assert_eq!(r.ended, 1, "one call at the end");
        let blocks: Vec<usize> = (0..6).map(|k| k * BLOCK).collect();
        assert_eq!(r.seen.starts, blocks, "each block once, in order");
        assert_eq!(r.reads, 6, "one read for each block, and no other");
        assert_eq!(
            r.seen.next_read_in_flight,
            [true, true, true, true, true, false],
            "the next block's read is in flight while the job mixes one"
        );
        assert!(
            r.polls <= 7,
            "a poll for each block and one to start: {}",
            r.polls
        );
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

    /// The fake times of the yields of a save of `blocks` blocks in memory,
    /// each block `block_ms` of work, with slices of `budget_ms`.
    fn yields_of(blocks: usize, budget_ms: f64, block_ms: f64) -> (Vec<f64>, f64) {
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
            budget_ms,
            move || clock.get(),
            move || {
                y.borrow_mut().push(c.get());
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
        let (yields, longest) = yields_of(30, 4.0, 1.0);
        assert_eq!(yields, [4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 28.0]);
        assert_eq!(longest, 4.0, "the longest slice, as the task measured it");
        // 3 ms a block: the second block ends the slice at 6 ms.
        let (yields, longest) = yields_of(30, 4.0, 3.0);
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
