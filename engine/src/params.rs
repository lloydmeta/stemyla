//! Lock-free state shared between the UI thread and the audio thread.
//!
//! The UI writes controls and reads the playhead; the audio thread does the
//! reverse.  Everything is an atomic, so neither side ever blocks.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

/// An `f32` stored in an `AtomicU32`.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Relaxed)
    }
}

const NO_SEEK: u64 = u64::MAX;

pub const MIN_TEMPO: f32 = 0.5;
pub const MAX_TEMPO: f32 = 1.5;

#[derive(Debug)]
pub struct StemControls {
    pub volume: AtomicF32,
    pub muted: AtomicBool,
    pub soloed: AtomicBool,
}

#[derive(Debug)]
pub struct SharedParams {
    pub playing: AtomicBool,
    tempo: AtomicF32,
    pub master: AtomicF32,
    pub stems: Vec<StemControls>,
    pub loop_enabled: AtomicBool,
    loop_start: AtomicU64,
    loop_end: AtomicU64,
    seek_request: AtomicU64,
    pub chipmunk: AtomicBool,
    /// Published by the audio thread: the song frame currently audible.
    pub playhead: AtomicU64,
    /// Published by the audio thread: engine time for the last callback, in ns.
    pub last_callback_ns: AtomicU64,
    /// Published by the audio thread: the largest callback time seen, in ns.
    pub max_callback_ns: AtomicU64,
    /// Published by the audio thread: number of callbacks served.
    pub callbacks: AtomicU64,
    /// Published by the audio thread: total engine time over all callbacks, in ns.
    pub total_callback_ns: AtomicU64,
    /// Published by the audio thread: frames in the most recent callback.
    pub last_callback_frames: AtomicU64,
}

impl SharedParams {
    pub fn new(n_stems: usize) -> Self {
        Self {
            playing: AtomicBool::new(false),
            tempo: AtomicF32::new(1.0),
            master: AtomicF32::new(1.0),
            stems: (0..n_stems)
                .map(|_| StemControls {
                    volume: AtomicF32::new(1.0),
                    muted: AtomicBool::new(false),
                    soloed: AtomicBool::new(false),
                })
                .collect(),
            loop_enabled: AtomicBool::new(false),
            loop_start: AtomicU64::new(0),
            loop_end: AtomicU64::new(0),
            seek_request: AtomicU64::new(NO_SEEK),
            chipmunk: AtomicBool::new(false),
            playhead: AtomicU64::new(0),
            last_callback_ns: AtomicU64::new(0),
            max_callback_ns: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            total_callback_ns: AtomicU64::new(0),
            last_callback_frames: AtomicU64::new(0),
        }
    }

    pub fn tempo(&self) -> f32 {
        self.tempo.get()
    }

    /// Tempo is clamped to the 50%..150% range the app offers.
    pub fn set_tempo(&self, t: f32) {
        self.tempo.set(t.clamp(MIN_TEMPO, MAX_TEMPO));
    }

    pub fn set_volume(&self, stem: usize, v: f32) {
        self.stems[stem].volume.set(v.clamp(0.0, 2.0));
    }
    pub fn set_muted(&self, stem: usize, m: bool) {
        self.stems[stem].muted.store(m, Relaxed);
    }
    pub fn set_soloed(&self, stem: usize, s: bool) {
        self.stems[stem].soloed.store(s, Relaxed);
    }
    pub fn is_muted(&self, stem: usize) -> bool {
        self.stems[stem].muted.load(Relaxed)
    }
    pub fn is_soloed(&self, stem: usize) -> bool {
        self.stems[stem].soloed.load(Relaxed)
    }

    /// The gain the engine really applies to a stem: 0 when muted, 0 when
    /// some other stem is soloed and this one is not, otherwise its volume.
    pub fn effective_gain(&self, stem: usize) -> f32 {
        let any_solo = self.stems.iter().any(|s| s.soloed.load(Relaxed));
        let c = &self.stems[stem];
        if c.muted.load(Relaxed) || (any_solo && !c.soloed.load(Relaxed)) {
            0.0
        } else {
            c.volume.get()
        }
    }

    /// Sets the loop region in song frames.  An empty or reversed region
    /// disables looping.
    pub fn set_loop(&self, start: u64, end: u64) {
        if end > start {
            self.loop_start.store(start, Relaxed);
            self.loop_end.store(end, Relaxed);
            self.loop_enabled.store(true, Relaxed);
        } else {
            self.loop_enabled.store(false, Relaxed);
        }
    }

    pub fn clear_loop(&self) {
        self.loop_enabled.store(false, Relaxed);
    }

    /// `Some((start, end))` when looping is on.
    pub fn loop_region(&self) -> Option<(u64, u64)> {
        if self.loop_enabled.load(Relaxed) {
            Some((self.loop_start.load(Relaxed), self.loop_end.load(Relaxed)))
        } else {
            None
        }
    }

    pub fn request_seek(&self, frame: u64) {
        self.seek_request.store(frame, Relaxed);
        // Show the new position immediately, even while paused.
        self.playhead.store(frame, Relaxed);
    }

    /// Audio thread: takes the pending seek, if any.
    pub fn take_seek(&self) -> Option<u64> {
        let v = self.seek_request.swap(NO_SEEK, Relaxed);
        (v != NO_SEEK).then_some(v)
    }

    pub fn playhead(&self) -> u64 {
        self.playhead.load(Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_and_solo_semantics() {
        let p = SharedParams::new(3);
        p.set_volume(0, 0.5);
        assert_eq!(p.effective_gain(0), 0.5);
        p.set_muted(0, true);
        assert_eq!(p.effective_gain(0), 0.0);
        p.set_muted(0, false);
        p.set_soloed(1, true);
        assert_eq!(p.effective_gain(0), 0.0);
        assert_eq!(p.effective_gain(1), 1.0);
        assert_eq!(p.effective_gain(2), 0.0);
        // Solo is additive.
        p.set_soloed(2, true);
        assert_eq!(p.effective_gain(2), 1.0);
        // Mute wins over solo.
        p.set_muted(2, true);
        assert_eq!(p.effective_gain(2), 0.0);
    }

    #[test]
    fn tempo_is_clamped_and_seek_is_taken_once() {
        let p = SharedParams::new(1);
        p.set_tempo(3.0);
        assert_eq!(p.tempo(), MAX_TEMPO);
        p.request_seek(42);
        assert_eq!(p.take_seek(), Some(42));
        assert_eq!(p.take_seek(), None);
    }
}
