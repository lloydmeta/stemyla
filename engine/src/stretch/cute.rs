//! Adapter for `cute-dsp`'s `SignalsmithStretch`.
//!
//! In cute-dsp 0.0.41, `SignalsmithStretch::process` is documented in its own
//! source as a stub: it resamples by nearest neighbour across the requested
//! output length and does not run a phase vocoder.  It is wired up here so
//! the bake-off can measure it, not because it is expected to pass.

use super::Stretcher;
use cute_dsp::stretch::SignalsmithStretch;
use std::collections::VecDeque;

pub struct CuteStretcher {
    st: SignalsmithStretch<f32>,
    in_l: VecDeque<f32>,
    in_r: VecDeque<f32>,
    tempo: f64,
    /// Fractional input frames owed from earlier pulls.
    carry: f64,
    scratch_in: Vec<Vec<f32>>,
    scratch_out: Vec<Vec<f32>>,
}

impl CuteStretcher {
    pub fn new(sample_rate: u32) -> Self {
        let mut st = SignalsmithStretch::<f32>::new();
        st.preset_default(2, sample_rate as f32, false);
        Self {
            st,
            in_l: VecDeque::with_capacity(1 << 15),
            in_r: VecDeque::with_capacity(1 << 15),
            tempo: 1.0,
            carry: 0.0,
            scratch_in: vec![Vec::with_capacity(8192), Vec::with_capacity(8192)],
            scratch_out: vec![vec![0.0; 8192], vec![0.0; 8192]],
        }
    }

    fn input_needed(&self, frames: usize) -> usize {
        (frames as f64 * self.tempo + self.carry).floor() as usize
    }
}

impl Stretcher for CuteStretcher {
    fn name(&self) -> &'static str {
        "cute-dsp"
    }
    fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
    }
    fn push(&mut self, left: &[f32], right: &[f32]) {
        self.in_l.extend(left);
        self.in_r.extend(right);
    }
    fn can_pull(&mut self, frames: usize) -> bool {
        self.in_l.len() >= self.input_needed(frames).max(1)
    }
    fn pull(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        let exact = n as f64 * self.tempo + self.carry;
        let m = (exact.floor() as usize).min(self.in_l.len()).max(1);
        self.carry = exact - exact.floor();
        if self.in_l.is_empty() {
            left.fill(0.0);
            right.fill(0.0);
            return;
        }
        self.scratch_in[0].clear();
        self.scratch_in[1].clear();
        self.scratch_in[0].extend(self.in_l.drain(..m));
        self.scratch_in[1].extend(self.in_r.drain(..m));
        for ch in &mut self.scratch_out {
            if ch.len() < n {
                ch.resize(n, 0.0);
            }
        }
        self.st
            .process(&self.scratch_in, m, &mut self.scratch_out, n);
        left.copy_from_slice(&self.scratch_out[0][..n]);
        right.copy_from_slice(&self.scratch_out[1][..n]);
    }
    fn latency(&self) -> f64 {
        0.0
    }
    fn reset(&mut self) {
        self.st.reset();
        self.in_l.clear();
        self.in_r.clear();
        self.carry = 0.0;
    }
    /// Nothing: this stretcher is for benches only, and RT-1 does not
    /// cover it.
    fn warm_up(&mut self, _max_tempo: f64, _max_frames: usize) {}
}
