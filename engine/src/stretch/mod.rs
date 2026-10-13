//! Time-stretchers behind one trait, so the player can swap them.
//!
//! The player pushes mixed stereo input and pulls exactly the number of
//! output frames the audio device asked for.  `tempo` above 1.0 means
//! faster: more input is consumed per output frame.

use std::collections::VecDeque;

#[cfg(feature = "stretch-cute")]
mod cute;
#[cfg(feature = "stretch-timestretch")]
mod ts;
#[cfg(feature = "stretch-wsola")]
mod wsola_adapter;

pub trait Stretcher: Send {
    fn name(&self) -> &'static str;
    /// Playback speed: 0.75 plays at 75% speed (longer), 1.25 faster.
    fn set_tempo(&mut self, tempo: f64);
    /// Appends input frames (separate channels, equal lengths).
    fn push(&mut self, left: &[f32], right: &[f32]);
    /// True when `pull` of `frames` can be served entirely from input that
    /// has already been pushed.
    fn can_pull(&mut self, frames: usize) -> bool;
    /// Fills both slices completely.  On underrun the shortfall is silence.
    fn pull(&mut self, left: &mut [f32], right: &mut [f32]);
    /// Output frames between an input frame entering and it being heard,
    /// beyond the plain tempo mapping.  The player subtracts this when it
    /// reports the playhead.  It can be negative (WSOLA at slow tempos
    /// emits a little early).
    fn latency(&self) -> f64;
    /// Drops all buffered audio and state, ready for a new position.
    fn reset(&mut self);
}

/// Which stretcher to build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StretcherKind {
    /// Resampling: tempo and pitch change together.  Always available.
    Varispeed,
    #[cfg(feature = "stretch-wsola")]
    Wsola,
    #[cfg(feature = "stretch-timestretch")]
    Timestretch,
    #[cfg(feature = "stretch-cute")]
    CuteDsp,
}

impl StretcherKind {
    pub fn all() -> Vec<StretcherKind> {
        vec![
            StretcherKind::Varispeed,
            #[cfg(feature = "stretch-wsola")]
            StretcherKind::Wsola,
            #[cfg(feature = "stretch-timestretch")]
            StretcherKind::Timestretch,
            #[cfg(feature = "stretch-cute")]
            StretcherKind::CuteDsp,
        ]
    }

    /// The default pitch-preserving stretcher chosen by the M4 bake-off
    /// (compare them with `xtask stretch-bench`), or varispeed when no candidate is compiled in.
    pub fn default_pitch_preserving() -> StretcherKind {
        #[cfg(feature = "stretch-wsola")]
        {
            StretcherKind::Wsola
        }
        #[cfg(all(not(feature = "stretch-wsola"), feature = "stretch-timestretch"))]
        {
            StretcherKind::Timestretch
        }
        #[cfg(not(any(feature = "stretch-timestretch", feature = "stretch-wsola")))]
        {
            StretcherKind::Varispeed
        }
    }

    pub fn build(self, sample_rate: u32) -> Box<dyn Stretcher> {
        match self {
            StretcherKind::Varispeed => Box::new(Varispeed::new()),
            #[cfg(feature = "stretch-wsola")]
            StretcherKind::Wsola => Box::new(wsola_adapter::WsolaStretcher::new(sample_rate)),
            #[cfg(feature = "stretch-timestretch")]
            StretcherKind::Timestretch => Box::new(ts::TimestretchStretcher::new(sample_rate)),
            #[cfg(feature = "stretch-cute")]
            StretcherKind::CuteDsp => Box::new(cute::CuteStretcher::new(sample_rate)),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StretcherKind::Varispeed => "varispeed",
            #[cfg(feature = "stretch-wsola")]
            StretcherKind::Wsola => "wsola",
            #[cfg(feature = "stretch-timestretch")]
            StretcherKind::Timestretch => "timestretch",
            #[cfg(feature = "stretch-cute")]
            StretcherKind::CuteDsp => "cute-dsp",
        }
    }
}

/// Plays the input faster or slower by resampling it with linear
/// interpolation, like speeding up a tape.  Pitch follows tempo.
pub struct Varispeed {
    left: VecDeque<f32>,
    right: VecDeque<f32>,
    /// Read position within the buffered input, in frames.
    t: f64,
    tempo: f64,
}

impl Varispeed {
    pub fn new() -> Self {
        Self {
            left: VecDeque::with_capacity(1 << 15),
            right: VecDeque::with_capacity(1 << 15),
            t: 0.0,
            tempo: 1.0,
        }
    }
}

impl Default for Varispeed {
    fn default() -> Self {
        Self::new()
    }
}

impl Stretcher for Varispeed {
    fn name(&self) -> &'static str {
        "varispeed"
    }
    fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo;
    }
    fn push(&mut self, left: &[f32], right: &[f32]) {
        self.left.extend(left);
        self.right.extend(right);
    }
    fn can_pull(&mut self, frames: usize) -> bool {
        if frames == 0 {
            return true;
        }
        let last = self.t + (frames - 1) as f64 * self.tempo;
        (last.floor() as usize) + 1 < self.left.len()
    }
    fn pull(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = self.left.len();
        for k in 0..left.len() {
            let i = self.t.floor() as usize;
            let f = (self.t - i as f64) as f32;
            if i + 1 < n {
                left[k] = self.left[i] + (self.left[i + 1] - self.left[i]) * f;
                right[k] = self.right[i] + (self.right[i + 1] - self.right[i]) * f;
            } else {
                left[k] = 0.0;
                right[k] = 0.0;
            }
            self.t += self.tempo;
        }
        let consumed = (self.t.floor() as usize).min(n);
        self.left.drain(..consumed);
        self.right.drain(..consumed);
        self.t -= consumed as f64;
    }
    fn latency(&self) -> f64 {
        0.0
    }
    fn reset(&mut self) {
        self.left.clear();
        self.right.clear();
        self.t = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    #[test]
    fn varispeed_identity_at_tempo_one() {
        let mut v = Varispeed::new();
        let x = ramp(100);
        v.push(&x, &x);
        assert!(v.can_pull(99));
        assert!(!v.can_pull(100));
        let mut l = vec![0.0; 50];
        let mut r = vec![0.0; 50];
        v.pull(&mut l, &mut r);
        assert_eq!(l, x[..50]);
        v.pull(&mut l[..10], &mut r[..10]);
        assert_eq!(l[..10], x[50..60]);
    }

    #[test]
    fn varispeed_half_speed_interpolates() {
        let mut v = Varispeed::new();
        v.set_tempo(0.5);
        let x = ramp(10);
        v.push(&x, &x);
        let mut l = vec![0.0; 6];
        let mut r = vec![0.0; 6];
        v.pull(&mut l, &mut r);
        assert_eq!(l, [0.0, 0.5, 1.0, 1.5, 2.0, 2.5]);
    }

    #[test]
    fn every_stretcher_reaches_steady_state() {
        for kind in StretcherKind::all() {
            let mut s = kind.build(44_100);
            s.set_tempo(0.75);
            let x: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
            let mut fed = 0;
            let mut out = vec![0.0; 512];
            let mut out_r = vec![0.0; 512];
            let mut produced = 0;
            let mut fed_at_20 = 0;
            for step in 0..60 {
                if step == 20 {
                    fed_at_20 = fed;
                    produced = 0;
                }
                while !s.can_pull(512) && fed + 256 <= x.len() {
                    s.push(&x[fed..fed + 256], &x[fed..fed + 256]);
                    fed += 256;
                }
                s.pull(&mut out, &mut out_r);
                produced += 512;
            }
            // Slope after the start-up lookahead has been filled.
            let ratio = (fed - fed_at_20) as f64 / produced as f64;
            assert!(
                (ratio - 0.75).abs() < 0.08,
                "{}: consumed/produced = {ratio}",
                kind.label()
            );
        }
    }
}
