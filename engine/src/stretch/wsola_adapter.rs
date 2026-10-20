//! Adapter for the `wsola` crate (WSOLA, tuned for speech).

use super::Stretcher;
use std::collections::VecDeque;
use std::num::{NonZeroU16, NonZeroU32};

pub struct WsolaStretcher {
    ts: wsola::TimeStretch,
    interleaved: Vec<f32>,
    out_l: VecDeque<f32>,
    out_r: VecDeque<f32>,
    tempo: f64,
}

impl WsolaStretcher {
    pub fn new(sample_rate: u32) -> Self {
        let cfg = wsola::Config::default();
        let rate = NonZeroU32::new(sample_rate).expect("a song has a sample rate");
        let stereo = NonZeroU16::new(2).expect("two is not zero");
        Self {
            ts: wsola::TimeStretch::with_config(rate, stereo, cfg).expect("valid config"),
            interleaved: Vec::with_capacity(8192),
            out_l: VecDeque::with_capacity(16384),
            out_r: VecDeque::with_capacity(16384),
            tempo: 1.0,
        }
    }
}

impl Stretcher for WsolaStretcher {
    fn name(&self) -> &'static str {
        "wsola"
    }
    fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
        self.ts.set_tempo(tempo as f32);
    }
    fn push(&mut self, left: &[f32], right: &[f32]) {
        self.interleaved.clear();
        for i in 0..left.len() {
            self.interleaved.push(left[i]);
            self.interleaved.push(right[i]);
        }
        self.ts.push(&self.interleaved);
    }
    fn can_pull(&mut self, frames: usize) -> bool {
        while self.out_l.len() < frames {
            let want = (frames - self.out_l.len()) * 2;
            let mut samples = self.ts.pull(want);
            let mut got = false;
            while let (Some(l), Some(r)) = (samples.next(), samples.next()) {
                self.out_l.push_back(l);
                self.out_r.push_back(r);
                got = true;
            }
            if !got {
                break;
            }
        }
        self.out_l.len() >= frames
    }
    fn pull(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.can_pull(left.len());
        for i in 0..left.len() {
            left[i] = self.out_l.pop_front().unwrap_or(0.0);
            right[i] = self.out_r.pop_front().unwrap_or(0.0);
        }
    }
    fn latency(&self) -> f64 {
        latency_at(self.tempo)
    }
    fn reset(&mut self) {
        self.ts.reset();
        self.out_l.clear();
        self.out_r.clear();
    }
    /// wsola keeps 65,536 frames of input behind its read position before
    /// it drops old input, so its input buffer grows over the first 1.5 s
    /// of play.  It can hold up to
    /// `65,536 + 4 * search + frame + 2 * tempo * hop + push` frames, where
    /// a step can pass 65,536 by its advance and the search's jitter, and
    /// `push` is at most 4,096: about 75,000 at 44.1 kHz and tempo 1.5.
    /// `4 * search + frame + 2 * tempo * hop` is below `sample_rate / 4` at
    /// any tempo that wsola accepts (up to 4).  So one push of
    /// `65,536 + sample_rate / 4 + 4,096` frames of silence gives the buffer
    /// its largest size, whatever `max_tempo` is.  The push goes straight
    /// to wsola, so `interleaved` keeps its size (MEM-4).  A pull of
    /// `max_frames` plus 20 ms, more than one 15 ms wsola hop, does the
    /// same for wsola's output buffer.  `reset` keeps both capacities.
    fn warm_up(&mut self, max_tempo: f64, max_frames: usize) {
        let rate = self.ts.sample_rate().get() as usize;
        let frames = (1 << 16) + rate / 4 + 4_096;
        self.ts.push(&vec![0.0; 2 * frames]);
        let tempo = self.tempo;
        self.set_tempo(max_tempo);
        self.can_pull(max_frames + rate / 50);
        self.reset();
        self.set_tempo(tempo);
    }
}

/// Output-frame latency measured with a click train by
/// `cargo run -p xtask --release --example latency` (44.1 kHz, default
/// config).  At 1.0x it equals the 12 ms similarity search; elsewhere it
/// depends on tempo, and individual clicks jitter by up to ~300 frames.
const MEASURED: [(f64, f64); 5] = [
    (0.5, -118.0),
    (0.75, 350.0),
    (1.0, 529.0),
    (1.25, 559.0),
    (1.5, 570.0),
];

fn latency_at(tempo: f64) -> f64 {
    if tempo <= MEASURED[0].0 {
        return MEASURED[0].1;
    }
    for w in MEASURED.windows(2) {
        let ((t0, l0), (t1, l1)) = (w[0], w[1]);
        if tempo <= t1 {
            return l0 + (l1 - l0) * (tempo - t0) / (t1 - t0);
        }
    }
    MEASURED[MEASURED.len() - 1].1
}

#[cfg(test)]
mod tests {
    #[test]
    fn latency_interpolates() {
        assert_eq!(super::latency_at(1.0), 529.0);
        assert_eq!(super::latency_at(0.875), (350.0 + 529.0) / 2.0);
        assert_eq!(super::latency_at(3.0), 570.0);
    }
}
