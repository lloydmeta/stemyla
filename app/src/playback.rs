//! The reader of the stored lanes (D7 and D9 of
//! `plans/active/stream-from-storage-m6.md`).  Web only.
//!
//! [`Playback::start`] makes the read-ahead buffer of a song's stored lanes
//! before its player, and starts the reader, an async task on the main
//! thread.  Each turn, the reader asks the buffer for the most wanted block
//! (`ReadAhead::next_read`).  It reads that block's bytes of every stored
//! lane, each lane on its own promise, with `Blob.slice(...).arrayBuffer()`,
//! as M1's spike reads.  It never reads more than one block of a lane at a
//! time.  Then it copies the bytes into the slot (`ReadAhead::fill`).  With
//! no block to read, it waits 10 ms.
//!
//! A read that fails gets the file again (`getFile`) and reads once more.
//! If that fails too, the lane fails: it plays silence from then on, and the
//! page says so.  The reader holds the buffer only through a `Weak`, never
//! across an `await`.  It stops when the buffer is gone or its
//! [`Playback`] is dropped (`StemApp::load_song` drops it first).

use crate::Inbox;
use crate::store_meta::LaneFile;
use eframe::egui;
use engine::SharedParams;
use engine::readahead::ReadAhead;
use engine::source::{BLOCK_FRAMES, StemSource};
use js_sys::{Array, Uint8Array};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Weak};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
// Reads `ranges[2i]..ranges[2i + 1]` of each `File` of `files` (null: no
// read), each on its own promise, so one bad lane does not lose the
// others'.  Gives a Uint8Array, null, or the error's text for each.
export async function pb_read(files, ranges) {
  const got = await Promise.allSettled(files.map((f, i) =>
    f ? f.slice(ranges[2 * i], ranges[2 * i + 1]).arrayBuffer() : Promise.resolve(null)));
  return got.map((r) => r.status === "fulfilled"
    ? (r.value ? new Uint8Array(r.value) : null)
    : String((r.reason && r.reason.name) || r.reason));
}
export function pb_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function pb_now() { return performance.now(); }
"#)]
extern "C" {
    async fn pb_read(files: &Array, ranges: &[f64]) -> JsValue;
    async fn pb_sleep(ms: u32);
    fn pb_now() -> f64;
}

/// One source for each lane of a song, `None` for a lane in memory.
pub type LaneSources = Vec<Option<Arc<dyn StemSource>>>;

/// The reader's read times, for the `playback:` line.
#[derive(Default)]
pub struct Stats {
    pub reads: u64,
    /// The last [`Stats::KEPT`] read times, in ms.
    times: Vec<f64>,
}

impl Stats {
    const KEPT: usize = 8_192;

    fn record(&mut self, ms: f64) {
        if self.times.len() == Self::KEPT {
            self.times.remove(0);
        }
        self.times.push(ms);
        self.reads += 1;
    }

    /// The read time at `fraction` (0.5 for the median) of the times kept.
    pub fn percentile(&self, fraction: f64) -> f64 {
        let mut sorted = self.times.clone();
        sorted.sort_by(f64::total_cmp);
        let Some(last) = sorted.len().checked_sub(1) else {
            return 0.0;
        };
        sorted[((last as f64) * fraction).round() as usize]
    }
}

/// The stored lanes of the shown song: the read-ahead buffer and its
/// reader.  Dropping it stops the reader.
pub struct Playback {
    pub ahead: Arc<ReadAhead>,
    /// The lanes that play from storage, and all the lanes.
    pub stored: usize,
    pub lanes: usize,
    pub stats: Rc<RefCell<Stats>>,
    stop: Rc<Cell<bool>>,
    /// Whether the reader's task runs.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "a test reads it through `Watch`")
    )]
    running: Rc<Cell<bool>>,
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop.set(true);
    }
}

/// A weak handle on a [`Playback`]'s buffer and reader, for a test.
#[cfg(test)]
pub struct Watch {
    pub ahead: Weak<ReadAhead>,
    running: Rc<Cell<bool>>,
}

#[cfg(test)]
impl Watch {
    /// Whether the reader's task still runs.
    pub fn reading(&self) -> bool {
        self.running.get()
    }
}

impl Playback {
    /// Makes the read-ahead buffer of the lanes of `files` that are `Some`,
    /// and starts its reader.  Gives the buffer and one source for each
    /// lane, `None` for a memory lane.  `None` when there is no stored lane,
    /// or the stored lanes need more than `MAX_READ_AHEAD_BYTES`: every
    /// lane then plays from memory.
    pub fn start(
        files: &[Option<LaneFile>],
        names: &[String],
        frames: usize,
        params: Arc<SharedParams>,
        pending: Rc<RefCell<Inbox>>,
        ctx: Option<egui::Context>,
    ) -> Option<(Self, LaneSources)> {
        let stored: Vec<(LaneFile, String)> = files
            .iter()
            .zip(names)
            .filter_map(|(f, n)| f.clone().map(|f| (f, n.clone())))
            .collect();
        if stored.is_empty() {
            return None;
        }
        let formats: Vec<_> = stored.iter().map(|(f, _)| f.format).collect();
        let Some(ahead) = ReadAhead::new(&formats, frames as u64, params) else {
            log::warn!(
                "playback: {} stored lanes need more than the read-ahead buffer holds; they play from memory",
                stored.len()
            );
            return None;
        };
        let ahead = Arc::new(ahead);
        let mut from_storage = ahead.sources().into_iter();
        let sources = files
            .iter()
            .map(|f| f.as_ref().and_then(|_| from_storage.next()))
            .collect();
        let stop = Rc::new(Cell::new(false));
        let stats = Rc::new(RefCell::new(Stats::default()));
        let (lane_files, lane_names) = stored.into_iter().unzip();
        let reader = Reader {
            ahead: Arc::downgrade(&ahead),
            files: lane_files,
            names: lane_names,
            stop: Rc::clone(&stop),
            stats: Rc::clone(&stats),
            pending,
            ctx,
        };
        let running = Rc::new(Cell::new(true));
        let ended = Rc::clone(&running);
        wasm_bindgen_futures::spawn_local(async move {
            reader.run().await;
            ended.set(false);
        });
        let playback = Self {
            stored: formats.len(),
            lanes: files.len(),
            ahead,
            stats,
            stop,
            running,
        };
        Some((playback, sources))
    }

    /// A weak handle on the buffer and the reader, for a test.
    #[cfg(test)]
    pub fn watch(&self) -> Watch {
        Watch {
            ahead: Arc::downgrade(&self.ahead),
            running: Rc::clone(&self.running),
        }
    }
}

/// The `playback:` line that the page logs every 2 s while it plays (D9),
/// for a song with `lanes` lanes.
pub fn line(playback: Option<&Playback>, lanes: usize, held_frames: u64) -> String {
    let (stored, mib, underruns, missed, failed, reads, p50, p99) = match playback {
        Some(p) => {
            let stats = p.stats.borrow();
            (
                p.stored,
                p.ahead.bytes() as f64 / (1 << 20) as f64,
                p.ahead.underruns(),
                p.ahead.missed_lane_frames(),
                p.ahead.failed_lanes(),
                stats.reads,
                stats.percentile(0.5),
                stats.percentile(0.99),
            )
        }
        None => (0, 0.0, 0, 0, 0, 0, 0.0, 0.0),
    };
    format!(
        "playback: lanes from storage {stored} of {lanes}, read-ahead {mib:.1} MiB, underruns {underruns} (callbacks), missed lane frames {missed}, held {held_frames} frames, failed lanes {failed}, reads {reads}, read p50 {p50:.1} ms, p99 {p99:.1} ms"
    )
}

struct Reader {
    ahead: Weak<ReadAhead>,
    files: Vec<LaneFile>,
    names: Vec<String>,
    stop: Rc<Cell<bool>>,
    stats: Rc<RefCell<Stats>>,
    pending: Rc<RefCell<Inbox>>,
    ctx: Option<egui::Context>,
}

/// What one read of a lane's block gave.
enum Got {
    Bytes(Uint8Array),
    /// No bytes: the block is past the file's end, or the lane failed.
    Nothing,
    Failed(String),
}

impl Reader {
    async fn run(self) {
        let mut handles = Vec::with_capacity(self.files.len());
        for file in &self.files {
            handles.push(crate::store::file_handle(&file.path).await);
        }
        loop {
            if self.stop.get() {
                return;
            }
            let (next, failed) = {
                let Some(ahead) = self.ahead.upgrade() else {
                    return;
                };
                let failed: Vec<bool> = (0..self.files.len()).map(|l| ahead.is_failed(l)).collect();
                (ahead.next_read(), failed)
            };
            let Some(read) = next else {
                pb_sleep(10).await;
                continue;
            };
            let ranges: Vec<(u64, u64)> = self
                .files
                .iter()
                .map(|f| f.byte_range(read.first_frame(), BLOCK_FRAMES))
                .collect();
            let started = pb_now();
            let mut got = self.read(&handles, &ranges, &failed).await;
            // A read that failed gets the file again, and reads once more.
            for lane in 0..got.len() {
                if let Got::Failed(_) = got[lane] {
                    handles[lane] = crate::store::file_handle(&self.files[lane].path).await;
                    let only: Vec<bool> = (0..got.len()).map(|l| l != lane).collect();
                    let again = self.read(&handles, &ranges, &only).await;
                    if let Some(lane_got) = again.into_iter().nth(lane) {
                        got[lane] = lane_got;
                    }
                }
            }
            self.stats.borrow_mut().record(pb_now() - started);
            if self.stop.get() {
                return;
            }
            let Some(ahead) = self.ahead.upgrade() else {
                return;
            };
            for (lane, lane_got) in got.iter().enumerate() {
                if let Got::Failed(why) = lane_got
                    && !ahead.is_failed(lane)
                {
                    self.fail(&ahead, lane, why);
                }
            }
            ahead.fill(read, |slot| {
                for (lane, bytes) in slot.iter_mut().enumerate() {
                    let n = match &got[lane] {
                        Got::Bytes(read) if !ahead.is_failed(lane) => {
                            let n = (read.length() as usize).min(bytes.len());
                            read.subarray(0, n as u32).copy_to(&mut bytes[..n]);
                            n
                        }
                        _ => 0,
                    };
                    bytes[n..].fill(0);
                }
            });
        }
    }

    /// Reads the block's range of each lane that is not `skip`ped.
    async fn read(
        &self,
        handles: &[Option<web_sys::File>],
        ranges: &[(u64, u64)],
        skip: &[bool],
    ) -> Vec<Got> {
        let files = Array::new();
        let mut flat = Vec::with_capacity(2 * ranges.len());
        for (lane, (start, end)) in ranges.iter().enumerate() {
            let wanted = !skip[lane] && start < end;
            match (&handles[lane], wanted) {
                (Some(file), true) => files.push(file),
                _ => files.push(&JsValue::NULL),
            };
            flat.push(*start as f64);
            flat.push(*end as f64);
        }
        let results: Array = pb_read(&files, &flat).await.unchecked_into();
        ranges
            .iter()
            .enumerate()
            .map(|(lane, (start, end))| {
                if skip[lane] || start >= end {
                    return Got::Nothing;
                }
                if handles[lane].is_none() {
                    return Got::Failed("the file is missing from browser storage".into());
                }
                let result = results.get(lane as u32);
                if let Some(why) = result.as_string() {
                    return Got::Failed(why);
                }
                let bytes: Uint8Array = result.unchecked_into();
                let expected = end - start;
                if u64::from(bytes.length()) == expected {
                    Got::Bytes(bytes)
                } else {
                    Got::Failed(format!("read {} of {expected} bytes", bytes.length()))
                }
            })
            .collect()
    }

    /// Fails a lane for good: it plays silence, and the page says so once.
    fn fail(&self, ahead: &ReadAhead, lane: usize, why: &str) {
        ahead.fail_lane(lane);
        let name = &self.names[lane];
        log::warn!("playback: lane {name} failed: {why}");
        self.pending.borrow_mut().status = Some(format!(
            "Could not read the {name} lane from browser storage; it plays silent."
        ));
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }
}
