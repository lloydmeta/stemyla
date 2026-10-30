//! The read-ahead buffer of the lanes in session files (M6, D3 to D5).
//!
//! A [`ReadAhead`] holds a fixed number of slots.  Each slot holds one
//! block ([`BLOCK_FRAMES`] frames) of every stored lane, as the file's own
//! bytes: f32 stereo, or 16-bit stereo.  It makes every buffer when it is
//! made, and its size does not depend on the song's length.
//!
//! Two sides share it:
//!
//! * The audio callback reads through a [`StoredSource`] for each lane.  It
//!   finds a block among the slots by an atomic block number, takes the
//!   slot with `try_lock` only, and converts the bytes.  A frame that no
//!   slot holds plays as 0.0 and counts as an underrun.  It never waits,
//!   allocates or frees (RT-1).
//! * The reader (on the page, an async task on the main thread) asks
//!   [`ReadAhead::next_read`] for the most wanted block that no slot holds,
//!   reads it from the files, and puts it in with [`ReadAhead::fill`].
//!   [`wanted_blocks`] gives the order: the run of a seek that holds, then
//!   the run ahead of the play head, then the loop's first blocks.

use crate::params::SharedParams;
use crate::player::PlayerConfig;
use crate::source::{BLOCK_FRAMES, Hint, StemSource, loop_in_force, next_run};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, Mutex, PoisonError};

/// The most bytes that the slots of one [`ReadAhead`] take.
pub const MAX_READ_AHEAD_BYTES: usize = 32 << 20;
/// Slots for the run ahead of the play head, and for the loop.
pub const AHEAD_SLOTS: usize = 16;
pub const LOOP_SLOTS: usize = 8;
/// The fewest slots for each, when the lanes need smaller counts to fit.
pub const MIN_AHEAD_SLOTS: usize = 4;
pub const MIN_LOOP_SLOTS: usize = 2;

const NO_BLOCK: u64 = u64::MAX;

/// The read-ahead buffers alive now, and the most alive at once since
/// [`ReadAhead::take_high_water`] (for a test of the page, D9 of M6).
static LIVE: AtomicUsize = AtomicUsize::new(0);
static HIGH_WATER: AtomicUsize = AtomicUsize::new(0);

/// The format of a lane file's samples: interleaved stereo, little-endian.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneFormat {
    /// `f32`, 8 bytes a frame (`mix.f32`, `decoded/<i>.f32`).
    F32,
    /// 16-bit PCM, 4 bytes a frame (`stems/<name>.wav`).
    Pcm16,
}

impl LaneFormat {
    pub fn bytes_per_frame(self) -> usize {
        match self {
            LaneFormat::F32 => 8,
            LaneFormat::Pcm16 => 4,
        }
    }

    /// The bytes of one block of a lane in this format.
    pub fn block_bytes(self) -> usize {
        BLOCK_FRAMES as usize * self.bytes_per_frame()
    }
}

/// A 16-bit sample as `f32`, as the decoder converts it (symphonia:
/// `s as f32 / 32_768.0`).
pub fn pcm16_to_f32(sample: i16) -> f32 {
    f32::from(sample) / 32_768.0
}

/// Decodes the whole frames at the start of `bytes`, a run of a lane file
/// in `format`, into `left` and `right`, as the read-ahead buffer converts
/// them.  Gives the frames decoded: at most `left.len()`.
pub fn decode_frames(
    format: LaneFormat,
    bytes: &[u8],
    left: &mut [f32],
    right: &mut [f32],
) -> usize {
    let frames = (bytes.len() / format.bytes_per_frame()).min(left.len());
    for k in 0..frames {
        (left[k], right[k]) = frame_at(format, bytes, k);
    }
    frames
}

/// The frame `k` of a block of bytes, as `(left, right)`.
fn frame_at(format: LaneFormat, bytes: &[u8], k: usize) -> (f32, f32) {
    match format {
        LaneFormat::F32 => {
            let at = k * 8;
            let left = f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            let right =
                f32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]);
            (left, right)
        }
        LaneFormat::Pcm16 => {
            let at = k * 4;
            let left = i16::from_le_bytes([bytes[at], bytes[at + 1]]);
            let right = i16::from_le_bytes([bytes[at + 2], bytes[at + 3]]);
            (pcm16_to_f32(left), pcm16_to_f32(right))
        }
    }
}

struct SlotData {
    block: Option<u64>,
    lanes: Vec<Vec<u8>>,
}

struct Slot {
    /// The block that the slot holds, or `NO_BLOCK`.  `next_read` clears
    /// it when it chooses the slot.  `fill` clears it first and writes it
    /// last.
    block: AtomicU64,
    data: Mutex<SlotData>,
}

/// A block that the reader reads next, and the slot that it goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockRead {
    pub block: u64,
    pub slot: usize,
}

impl BlockRead {
    /// The first frame of the block.
    pub fn first_frame(&self) -> u64 {
        self.block * BLOCK_FRAMES
    }
}

/// What [`wanted_blocks`] needs.
#[derive(Clone, Copy, Debug)]
pub struct Want {
    pub frames: u64,
    /// The player's audible position.
    pub heard: u64,
    /// The target of a seek that holds.
    pub seek: Option<u64>,
    /// The loop of `SharedParams`.
    pub looping: Option<(u64, u64)>,
    /// The player's crossfade.
    pub xf: u64,
    pub ahead: usize,
    pub loop_slots: usize,
}

/// The blocks that the reader wants, most wanted first, each once (D4):
///
/// 1. the blocks of the next run from a seek that holds,
/// 2. the run ahead: `ahead` blocks from the block of `heard`, along the
///    loop when a loop is in force and the head is before its end,
/// 3. the loop: `loop_slots` blocks from the block of its start, within its
///    run.
///
/// Blocks past the song's end are not wanted.
pub fn wanted_blocks(want: &Want) -> Vec<u64> {
    let mut list: Vec<u64> = Vec::new();
    let last = want.frames.div_ceil(BLOCK_FRAMES);
    let add = |list: &mut Vec<u64>, block: u64| {
        if block < last && !list.contains(&block) {
            list.push(block);
        }
    };
    // Each block from the one of `start` to the one of `end - 1`.
    let blocks = |start: u64, end: u64| (start / BLOCK_FRAMES)..end.div_ceil(BLOCK_FRAMES);

    if let Some(target) = want.seek {
        let run = next_run(target, want.frames, want.looping, want.xf);
        for (start, end) in run.parts() {
            for block in blocks(start, end) {
                add(&mut list, block);
            }
        }
    }

    let in_force = loop_in_force(want.looping, want.frames);
    let mut path: Vec<(u64, u64)> = Vec::with_capacity(2);
    match in_force {
        Some((s, e)) if want.heard < e => {
            let tail = (e + want.xf).min(want.frames);
            path.push((want.heard, tail));
            path.push((s, tail));
        }
        _ => path.push((want.heard, want.frames)),
    }
    let mut seen: Vec<u64> = Vec::with_capacity(want.ahead);
    'ahead: for (start, end) in path {
        for block in blocks(start, end) {
            if seen.len() == want.ahead {
                break 'ahead;
            }
            if !seen.contains(&block) {
                seen.push(block);
                add(&mut list, block);
            }
        }
    }

    if let Some((s, e)) = in_force {
        let tail = (e + want.xf).min(want.frames);
        for block in blocks(s, tail).take(want.loop_slots) {
            add(&mut list, block);
        }
    }
    list
}

/// The read-ahead buffer of the stored lanes of one song.  See the module
/// doc.
pub struct ReadAhead {
    formats: Vec<LaneFormat>,
    frames: u64,
    params: Arc<SharedParams>,
    xf: u64,
    ahead: usize,
    loop_slots: usize,
    slots: Vec<Slot>,
    failed: Vec<AtomicBool>,
    /// The lanes that the player read since [`Self::take_lanes_read`].
    read_lanes: Vec<AtomicBool>,
    // Hints from the player.
    heard: AtomicU64,
    read: AtomicU64,
    seek: AtomicU64,
    // Counters.
    missed_mark: AtomicBool,
    underruns: AtomicU64,
    missed_lane_frames: AtomicU64,
    failed_lanes: AtomicU64,
}

impl ReadAhead {
    /// The slot counts for `lanes`: 16 ahead and 8 for the loop, or the
    /// most slots that fit in [`MAX_READ_AHEAD_BYTES`], split 2 to 1, at
    /// least 4 and 2.  `None` when even those do not fit.
    pub fn slot_counts(lanes: &[LaneFormat]) -> Option<(usize, usize)> {
        let slot_bytes: usize = lanes.iter().map(|f| f.block_bytes()).sum();
        if slot_bytes == 0 {
            return Some((AHEAD_SLOTS, LOOP_SLOTS));
        }
        let fit = MAX_READ_AHEAD_BYTES / slot_bytes;
        if fit >= AHEAD_SLOTS + LOOP_SLOTS {
            return Some((AHEAD_SLOTS, LOOP_SLOTS));
        }
        if fit < MIN_AHEAD_SLOTS + MIN_LOOP_SLOTS {
            return None;
        }
        let loop_slots = (fit / 3).max(MIN_LOOP_SLOTS);
        Some(((fit - loop_slots).max(MIN_AHEAD_SLOTS), loop_slots))
    }

    /// Makes the buffer for `lanes` of a song of `frames` frames, with
    /// every slot, before the player is made (RT-1).  `None` when the
    /// lanes need more than [`MAX_READ_AHEAD_BYTES`]: they then play from
    /// memory.
    pub fn new(lanes: &[LaneFormat], frames: u64, params: Arc<SharedParams>) -> Option<Self> {
        let (ahead, loop_slots) = Self::slot_counts(lanes)?;
        Some(Self::with_slots(lanes, frames, params, ahead, loop_slots))
    }

    /// [`Self::new`] with the slot counts given, for tests.
    pub fn with_slots(
        lanes: &[LaneFormat],
        frames: u64,
        params: Arc<SharedParams>,
        ahead: usize,
        loop_slots: usize,
    ) -> Self {
        assert!(ahead > 0, "at least one slot ahead");
        let alive = LIVE.fetch_add(1, Relaxed) + 1;
        HIGH_WATER.fetch_max(alive, Relaxed);
        let slots = (0..ahead + loop_slots)
            .map(|_| Slot {
                block: AtomicU64::new(NO_BLOCK),
                data: Mutex::new(SlotData {
                    block: None,
                    lanes: lanes.iter().map(|f| vec![0; f.block_bytes()]).collect(),
                }),
            })
            .collect();
        Self {
            formats: Vec::from_iter(lanes.iter().copied()),
            frames,
            params,
            xf: PlayerConfig::default().crossfade as u64,
            ahead,
            loop_slots,
            slots,
            failed: lanes.iter().map(|_| AtomicBool::new(false)).collect(),
            read_lanes: lanes.iter().map(|_| AtomicBool::new(false)).collect(),
            heard: AtomicU64::new(0),
            read: AtomicU64::new(0),
            seek: AtomicU64::new(NO_BLOCK),
            missed_mark: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            missed_lane_frames: AtomicU64::new(0),
            failed_lanes: AtomicU64::new(0),
        }
    }

    /// One source for each lane, in order.
    pub fn sources(self: &Arc<Self>) -> Vec<Arc<dyn StemSource>> {
        (0..self.formats.len())
            .map(|lane| {
                Arc::new(StoredSource {
                    ahead: Arc::clone(self),
                    lane,
                }) as Arc<dyn StemSource>
            })
            .collect()
    }

    pub fn lanes(&self) -> &[LaneFormat] {
        &self.formats
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The slots for the run ahead, and for the loop.
    pub fn slot_split(&self) -> (usize, usize) {
        (self.ahead, self.loop_slots)
    }

    /// The bytes of all the slots.
    pub fn bytes(&self) -> usize {
        self.slots.len() * self.formats.iter().map(|f| f.block_bytes()).sum::<usize>()
    }

    /// The read-ahead buffers alive now.
    pub fn live() -> usize {
        LIVE.load(Relaxed)
    }

    /// The most read-ahead buffers alive at once since the last call; the
    /// next count starts from those alive now.
    pub fn take_high_water() -> usize {
        HIGH_WATER.swap(LIVE.load(Relaxed), Relaxed)
    }

    /// The stored lanes that the player read since the last call: the
    /// lanes that play, not muted.
    pub fn take_lanes_read(&self) -> usize {
        self.read_lanes
            .iter()
            .filter(|lane| lane.swap(false, Relaxed))
            .count()
    }

    /// The `process` calls that missed frames of a lane.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Relaxed)
    }

    /// The frames missed, times the lanes that missed them.
    pub fn missed_lane_frames(&self) -> u64 {
        self.missed_lane_frames.load(Relaxed)
    }

    pub fn failed_lanes(&self) -> u64 {
        self.failed_lanes.load(Relaxed)
    }

    pub fn is_failed(&self, lane: usize) -> bool {
        self.failed[lane].load(Relaxed)
    }

    /// The player's read position at the end of its last `process`.
    pub fn read_position(&self) -> u64 {
        self.read.load(Relaxed)
    }

    /// Marks a lane failed: it plays silence from now on, counts no
    /// underrun and is always ready.  The reader fills its bytes with zero.
    pub fn fail_lane(&self, lane: usize) {
        if !self.failed[lane].swap(true, Relaxed) {
            self.failed_lanes.fetch_add(1, Relaxed);
        }
    }

    fn held_slot(&self, block: u64) -> Option<&Slot> {
        self.slots
            .iter()
            .find(|slot| slot.block.load(Relaxed) == block)
    }

    /// Whether a slot holds `block`.
    pub fn holds(&self, block: u64) -> bool {
        self.held_slot(block).is_some()
    }

    /// The blocks the reader wants now, most wanted first (D4).
    pub fn wanted(&self) -> Vec<u64> {
        let seek = self.seek.load(Relaxed);
        wanted_blocks(&Want {
            frames: self.frames,
            heard: self.heard.load(Relaxed),
            seek: (seek != NO_BLOCK).then_some(seek),
            looping: self.params.loop_region(),
            xf: self.xf,
            ahead: self.ahead,
            loop_slots: self.loop_slots,
        })
    }

    /// The first of the kept wanted blocks that no slot holds, and the slot
    /// to put it in: an empty slot, or else one whose block is not kept.
    /// `None` when there is nothing to read, or no slot to read it into.
    /// The slot no longer holds its old block from now on, so nothing plays
    /// from it, and a seek into that block holds, until a fill publishes a
    /// block there (review round 3 of M6, finding 3-2).  If the read is
    /// dropped, the slot stays empty.  Not for the callback.
    pub fn next_read(&self) -> Option<BlockRead> {
        let mut kept = self.wanted();
        kept.truncate(self.slots.len());
        let block = kept.iter().copied().find(|b| !self.holds(*b))?;
        let held = |slot: &Slot| slot.block.load(Relaxed);
        let slot = self
            .slots
            .iter()
            .position(|s| held(s) == NO_BLOCK)
            .or_else(|| self.slots.iter().position(|s| !kept.contains(&held(s))))?;
        let chosen = &self.slots[slot];
        chosen.block.store(NO_BLOCK, Relaxed);
        chosen
            .data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .block = None;
        Some(BlockRead { block, slot })
    }

    /// Puts a block in its slot.  `put` gets one byte buffer for each lane,
    /// [`LaneFormat::block_bytes`] long, and writes every byte of each: the
    /// bytes after a file's end, and those of a failed lane, are zero.
    /// The slot holds no block while `put` runs.  Not for the callback.
    pub fn fill(&self, read: BlockRead, put: impl FnOnce(&mut [&mut [u8]])) {
        let slot = &self.slots[read.slot];
        slot.block.store(NO_BLOCK, Relaxed);
        let mut data = slot.data.lock().unwrap_or_else(PoisonError::into_inner);
        data.block = None;
        let mut lanes: Vec<&mut [u8]> = data.lanes.iter_mut().map(Vec::as_mut_slice).collect();
        put(&mut lanes);
        data.block = Some(read.block);
        drop(data);
        slot.block.store(read.block, Relaxed);
    }

    fn hint(&self, hint: Hint) {
        match hint {
            Hint::Reading { heard, read } => {
                self.heard.store(heard, Relaxed);
                self.read.store(read, Relaxed);
                self.missed_mark.store(false, Relaxed);
            }
            Hint::Seek(target) => self.seek.store(target.unwrap_or(NO_BLOCK), Relaxed),
        }
    }

    fn missed(&self, frames: u64) {
        self.missed_lane_frames.fetch_add(frames, Relaxed);
        if !self.missed_mark.swap(true, Relaxed) {
            self.underruns.fetch_add(1, Relaxed);
        }
    }

    /// The callback's read of one lane (RT-1: no wait, no allocation).
    fn read(&self, lane: usize, from: u64, left: &mut [f32], right: &mut [f32]) {
        self.read_lanes[lane].store(true, Relaxed);
        left.fill(0.0);
        right.fill(0.0);
        if self.is_failed(lane) {
            return;
        }
        let format = self.formats[lane];
        let end = (from + left.len() as u64).min(self.frames);
        let mut pos = from;
        let mut missing = 0;
        while pos < end {
            let block = pos / BLOCK_FRAMES;
            let block_end = ((block + 1) * BLOCK_FRAMES).min(end);
            let n = (block_end - pos) as usize;
            let at = (pos - from) as usize;
            let copied = self.held_slot(block).is_some_and(|slot| {
                let Ok(data) = slot.data.try_lock() else {
                    return false;
                };
                if data.block != Some(block) {
                    return false;
                }
                let bytes = &data.lanes[lane];
                let first = (pos - block * BLOCK_FRAMES) as usize;
                for k in 0..n {
                    let (l, r) = frame_at(format, bytes, first + k);
                    left[at + k] = l;
                    right[at + k] = r;
                }
                true
            });
            if !copied {
                missing += n as u64;
            }
            pos = block_end;
        }
        if missing > 0 {
            self.missed(missing);
        }
    }

    fn ready(&self, lane: usize, from: u64, frames: usize) -> bool {
        if self.is_failed(lane) {
            return true;
        }
        let end = (from + frames as u64).min(self.frames);
        if from >= end {
            return true;
        }
        (from / BLOCK_FRAMES..end.div_ceil(BLOCK_FRAMES)).all(|block| self.holds(block))
    }
}

impl Drop for ReadAhead {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Relaxed);
    }
}

/// One stored lane of a [`ReadAhead`].
pub struct StoredSource {
    ahead: Arc<ReadAhead>,
    lane: usize,
}

impl StemSource for StoredSource {
    fn read(&self, from: u64, left: &mut [f32], right: &mut [f32]) {
        self.ahead.read(self.lane, from, left, right);
    }

    fn ready(&self, from: u64, frames: usize) -> bool {
        self.ahead.ready(self.lane, from, frames)
    }

    fn hint(&self, hint: Hint) {
        self.ahead.hint(hint);
    }
}

#[cfg(test)]
mod tests;
