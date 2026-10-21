//! Adapter for the `timestretch` crate's real-time engine, using its
//! `WideKeylock` profile (full-spectrum phase vocoder, rates 0.25..2.0).
//! The `Keylock` profile only holds pitch within about ±20%, which does not
//! cover the 50%..150% tempo slider.

use super::Stretcher;
use timestretch::engine::{
    Engine, EngineConfig, EngineController, EngineProcessor, EngineProfile, SourceProducer,
};

const MAX_BLOCK: usize = 1024;

pub struct TimestretchStretcher {
    controller: EngineController,
    processor: EngineProcessor,
    source: SourceProducer,
    interleaved_in: Vec<f32>,
    interleaved_out: Vec<f32>,
    tempo: f64,
}

impl TimestretchStretcher {
    pub fn new(sample_rate: u32) -> Self {
        let h = Engine::build(EngineConfig {
            sample_rate,
            channels: 2,
            profile: EngineProfile::WideKeylock,
            initial_tempo_rate: 1.0,
            max_block_frames: MAX_BLOCK,
            source_capacity_frames: 1 << 16,
            pre_analysis: None,
        })
        .expect("timestretch engine config is valid");
        let mut source = h.source;
        source.set_track_position(0);
        Self {
            controller: h.controller,
            processor: h.processor,
            source,
            interleaved_in: Vec::with_capacity(8192),
            interleaved_out: vec![0.0; 2 * 8192],
            tempo: 1.0,
        }
    }
}

impl Stretcher for TimestretchStretcher {
    fn name(&self) -> &'static str {
        "timestretch"
    }
    fn set_tempo(&mut self, tempo: f64) {
        if tempo != self.tempo {
            self.tempo = tempo;
            self.controller.set_tempo_rate(tempo);
        }
    }
    fn push(&mut self, left: &[f32], right: &[f32]) {
        self.interleaved_in.clear();
        for i in 0..left.len() {
            self.interleaved_in.push(left[i]);
            self.interleaved_in.push(right[i]);
        }
        let mut done = 0;
        while done < left.len() {
            let accepted = self.source.push(&self.interleaved_in[done * 2..]);
            if accepted == 0 {
                // Ring full: the player only pushes while can_pull is false,
                // so this means the ring is undersized.  Drop the rest.
                break;
            }
            done += accepted;
        }
    }
    fn can_pull(&mut self, frames: usize) -> bool {
        let need = self.source.demand_hint(frames, self.tempo)
            + self.processor.varispeed_lookahead_frames();
        self.source.occupied_frames() >= need
    }
    fn pull(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        if self.interleaved_out.len() < n * 2 {
            self.interleaved_out.resize(n * 2, 0.0);
        }
        let out = &mut self.interleaved_out[..n * 2];
        self.processor.process(out);
        for i in 0..n {
            left[i] = out[2 * i];
            right[i] = out[2 * i + 1];
        }
    }
    fn latency(&self) -> f64 {
        self.processor.pipeline_latency_frames() as f64
    }
    fn reset(&mut self) {
        self.processor.reset();
        self.source.set_track_position(0);
        // Re-apply tempo after the cold reset.
        self.controller.set_tempo_rate(self.tempo);
    }
    /// Nothing: this stretcher is for benches only, and RT-1 does not
    /// cover it.
    fn warm_up(&mut self, _max_tempo: f64, _max_frames: usize) {}
}

#[allow(dead_code)]
impl TimestretchStretcher {
    /// Frames the engine had to fill with silence because input ran dry.
    pub fn underruns(&self) -> u64 {
        self.controller.underrun_frames()
    }
}
