//! The playback engine: one read position for all stems, mix first, then
//! stretch once.
//!
//! For every block:
//! 1. read every stem at the shared position, through its [`StemSource`],
//! 2. multiply each by its (smoothed) gain and add them up,
//! 3. feed the sum through one stretcher,
//! 4. apply the master volume and clamp to ±1,
//! 5. output.
//!
//! Because there is only one read position, stems cannot drift apart.
//!
//! The player holds its sources, not the [`Song`].  A source can lack the
//! frames of a position for a time (a lane in a session file, read through
//! the read-ahead buffer).  So the player plays from a position that it has
//! not played from yet only when [`next_run`] from there is ready:
//!
//! * A seek while playing whose run is not ready holds: the old position
//!   plays on, the player publishes the target as the playhead, and gives
//!   [`Hint::Seek`].  When the run is ready, the seek fades out and jumps.
//! * A loop set behind the head is such a seek, to the position where the
//!   read wraps.  The player keeps its old loop until it jumps.
//! * After a jump (a paused seek, a chipmunk change, a new player), the
//!   player renders silence and does not move until the run is ready (a
//!   hold).  It counts the frames in [`SharedParams::held_frames`].
//!
//! A lane in memory is always ready, so a song in memory never holds.

use crate::params::{MAX_TEMPO, SharedParams};
use crate::song::Song;
use crate::source::{Hint, StemSource, loop_in_force, memory_sources, next_run};
use crate::stretch::{MAX_PUSH_FRAMES, Stretcher, StretcherKind};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

#[derive(Clone, Copy, Debug)]
pub struct PlayerConfig {
    /// Largest number of output frames rendered in one inner step.
    pub block: usize,
    /// Input frames read, mixed and pushed into the stretcher at a time.
    pub hop: usize,
    /// Length of the loop crossfade and of the seek fade-out and fade-in.
    /// Zero disables them (used by tests to show the click they prevent).
    pub crossfade: usize,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            block: 512,
            hop: 256,
            crossfade: 256,
        }
    }
}

/// Where a run of contiguous song frames starts in the stretcher's input.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    input_index: u64,
    song_pos: u64,
}

enum Fade {
    None,
    /// Fading out before a pending seek; `k` frames done.
    Out {
        k: usize,
        target: u64,
    },
    /// Fading in after a seek; `k` frames done.
    In {
        k: usize,
    },
}

/// A seek that waits for its run: asked by the user, or the wrap of a loop
/// set behind the head (`for_loop`, the loop it was made for).
#[derive(Clone, Copy, Debug)]
struct Pending {
    target: u64,
    for_loop: Option<(u64, u64)>,
    /// Whether it has held for at least one `process`.
    held: bool,
}

pub struct Player {
    sample_rate: u32,
    frames: usize,
    sources: Vec<Arc<dyn StemSource>>,
    params: Arc<SharedParams>,
    stretcher: Box<dyn Stretcher>,
    cfg: PlayerConfig,

    /// Next song frame to read (input side of the stretcher).
    pos: u64,
    /// Whether the player has not read from `pos` since it jumped there.
    fresh: bool,
    /// The loop the player plays: the loop of `params`, taken at the start
    /// of each `process`, unless its change waits as a [`Pending`] seek.
    looping: Option<(u64, u64)>,
    pending: Option<Pending>,
    /// Gains actually applied, ramped towards the targets once per hop.
    gains: Vec<f32>,
    /// Crossfade tables: `fin[k] + fout[k] == 1` for every k.
    fin: Vec<f32>,
    fout: Vec<f32>,
    /// Loop crossfade in progress: the old position still fading out.
    xf_old: Option<u64>,
    xf_k: usize,
    fade: Fade,

    // Playhead bookkeeping.
    anchors: VecDeque<Anchor>,
    pushed: u64,
    consumed: f64,
    chipmunk: bool,

    // Scratch buffers, sized once so the audio path does not allocate.
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    pos_buf: Vec<u64>,
    old_buf: Vec<Option<u64>>,
    xf_buf: Vec<usize>,
    chip_l: Vec<f32>,
    chip_r: Vec<f32>,
    /// One lane's frames at `pos_buf` and at `old_buf`.
    lane_l: Vec<f32>,
    lane_r: Vec<f32>,
    old_l: Vec<f32>,
    old_r: Vec<f32>,
}

impl Player {
    pub fn new(song: Arc<Song>, params: Arc<SharedParams>, kind: StretcherKind) -> Self {
        let stretcher = kind.build(song.sample_rate);
        Self::with_config(song, params, stretcher, PlayerConfig::default())
    }

    /// Makes a player of the stems of `song` in memory, with no copy.  See
    /// [`Self::with_sources`].
    pub fn with_config(
        song: Arc<Song>,
        params: Arc<SharedParams>,
        stretcher: Box<dyn Stretcher>,
        cfg: PlayerConfig,
    ) -> Self {
        Self::with_sources(
            song.sample_rate,
            song.frames,
            params,
            stretcher,
            cfg,
            memory_sources(&song),
        )
    }

    /// Makes a player of one source for each lane, in the order of
    /// `params`, and warms its stretcher up on this thread, so the audio
    /// callback does not allocate (RT-1).
    pub fn with_sources(
        sample_rate: u32,
        frames: usize,
        params: Arc<SharedParams>,
        mut stretcher: Box<dyn Stretcher>,
        cfg: PlayerConfig,
        sources: Vec<Arc<dyn StemSource>>,
    ) -> Self {
        assert_eq!(sources.len(), params.stems.len(), "params must match stems");
        // A fade_in of n+1 ones gives 0, 1/(n+1), .., n/(n+1).  Dropping the
        // leading zero leaves a fade-in whose reverse is the matching
        // fade-out: each pair sums to exactly 1, so the level never dips.
        let ramp = dsp::fade_in(&vec![1.0; cfg.crossfade + 1]);
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: the crossfade ramp, made once for each player"
        )]
        let fin: Vec<f32> = ramp[1..].to_vec();
        let mut fout = fin.clone();
        fout.reverse();
        let gains = (0..sources.len())
            .map(|i| params.effective_gain(i))
            .collect();
        let max_in = cfg.hop.max(cfg.block * 2);
        assert!(
            cfg.hop <= MAX_PUSH_FRAMES,
            "RT-1: a hop of {} frames is over the {MAX_PUSH_FRAMES} that `Stretcher::warm_up` covers.  Fix: use a smaller hop, or raise `MAX_PUSH_FRAMES`.",
            cfg.hop
        );
        stretcher.warm_up(f64::from(MAX_TEMPO), cfg.block);
        let looping = params.loop_region();
        Self {
            sample_rate,
            frames,
            sources,
            params,
            stretcher,
            cfg,
            pos: 0,
            fresh: true,
            looping,
            pending: None,
            gains,
            fin,
            fout,
            xf_old: None,
            xf_k: 0,
            fade: Fade::None,
            anchors: VecDeque::with_capacity(1024),
            pushed: 0,
            consumed: 0.0,
            chipmunk: false,
            in_l: vec![0.0; max_in],
            in_r: vec![0.0; max_in],
            pos_buf: vec![0; max_in],
            old_buf: vec![None; max_in],
            xf_buf: vec![0; max_in],
            chip_l: vec![0.0; cfg.block * 2],
            chip_r: vec![0.0; cfg.block * 2],
            lane_l: vec![0.0; max_in],
            lane_r: vec![0.0; max_in],
            old_l: vec![0.0; max_in],
            old_r: vec![0.0; max_in],
        }
    }

    pub fn params(&self) -> &Arc<SharedParams> {
        &self.params
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The song's length in frames.
    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn stretcher_name(&self) -> &'static str {
        self.stretcher.name()
    }

    /// Offline render: plays `frames` frames and returns them per channel.
    pub fn render(&mut self, frames: usize) -> [Vec<f32>; 2] {
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        self.process(&mut l, &mut r);
        [l, r]
    }

    fn hint(&self, hint: Hint) {
        for source in &self.sources {
            source.hint(hint);
        }
    }

    /// Whether every source holds the next run from `p`, with `looping`.
    fn run_ready(&self, p: u64, looping: Option<(u64, u64)>) -> bool {
        let run = next_run(p, self.frames as u64, looping, self.cfg.crossfade as u64);
        self.sources
            .iter()
            .all(|source| run.ready(|from, frames| source.ready(from, frames)))
    }

    /// Whether the player is in a hold: it has not read from its position
    /// since a jump, and the run from there is not ready.
    fn holding(&self) -> bool {
        self.fresh && !self.run_ready(self.pos, self.looping)
    }

    /// Where the next read wraps to if the player takes `wanted` now, when
    /// that is a jump: the read position is at or after the loop's end.
    fn wrap_target(&self, wanted: Option<(u64, u64)>) -> Option<u64> {
        let (s, e) = loop_in_force(wanted, self.frames as u64)?;
        (self.pos >= e).then(|| dsp::wrap_loop(self.pos as usize, s as usize, e as usize) as u64)
    }

    /// Takes a new seek and a change of the loop, and starts each seek
    /// whose run is ready.  A seek whose run is not ready waits.
    fn take_seeks(&mut self, playing: bool) {
        let p = Arc::clone(&self.params);
        let wanted = p.loop_region();
        if let Some(target) = p.take_seek() {
            self.pending = Some(Pending {
                target,
                for_loop: None,
                held: false,
            });
        }
        if matches!(self.pending, Some(Pending { for_loop: Some(l), .. }) if Some(l) != wanted) {
            self.pending = None;
        }
        let wrap_holds = matches!(
            self.pending,
            Some(Pending {
                for_loop: Some(_),
                ..
            })
        );
        if wanted != self.looping {
            match self.wrap_target(wanted) {
                None => self.looping = wanted,
                // A held wrap of this loop waits for its own run.
                Some(_) if wrap_holds => {}
                Some(target) if self.run_ready(target, wanted) => self.looping = wanted,
                Some(target) if self.pending.is_none() => {
                    self.pending = wanted.map(|l| Pending {
                        target,
                        for_loop: Some(l),
                        held: false,
                    });
                }
                // A seek waits, and its jump takes the loop.
                Some(_) => {}
            }
        }
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        if !playing || self.holding() {
            self.jump_to(pending.target);
        } else if self.run_ready(pending.target, wanted) {
            if self.cfg.crossfade > 0 {
                self.fade = Fade::Out {
                    k: 0,
                    target: pending.target,
                };
            } else {
                self.jump_to(pending.target);
            }
        } else {
            if !pending.held {
                pending.held = true;
                p.held_seeks.fetch_add(1, Relaxed);
            }
            self.pending = Some(pending);
        }
    }

    /// The playhead to publish: the target of a seek that holds, or else
    /// the player's own.
    fn shown_head(&mut self) -> u64 {
        match self.pending {
            Some(pending) => pending.target,
            None => self.playhead(),
        }
    }

    /// Fills both output slices.  This is what the audio callback calls.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let p = Arc::clone(&self.params);
        let playing = p.playing.load(Relaxed);
        let heard = self.playhead();
        self.hint(Hint::Reading {
            heard,
            read: self.pos,
        });

        self.take_seeks(playing);
        self.hint(Hint::Seek(self.pending.map(|pending| pending.target)));

        if !playing {
            out_l.fill(0.0);
            out_r.fill(0.0);
            if matches!(self.fade, Fade::Out { .. }) {
                if let Fade::Out { target, .. } = self.fade {
                    self.jump_to(target);
                }
                self.fade = Fade::None;
            }
            return;
        }

        let chip = p.chipmunk.load(Relaxed);
        if chip != self.chipmunk {
            self.chipmunk = chip;
            let here = self.playhead();
            self.jump_to(here);
        }

        if self.holding() {
            out_l.fill(0.0);
            out_r.fill(0.0);
            p.held_frames.fetch_add(out_l.len() as u64, Relaxed);
            let head = self.shown_head();
            p.playhead.store(head, Relaxed);
            return;
        }

        let mut done = 0;
        while done < out_l.len() {
            let mut n = (out_l.len() - done).min(self.cfg.block);
            if let Fade::Out { k, .. } = self.fade {
                n = n.min(self.cfg.crossfade - k);
            }
            let (l, r) = (&mut out_l[done..done + n], &mut out_r[done..done + n]);
            if self.chipmunk {
                self.render_chipmunk(l, r);
            } else {
                self.render_stretched(l, r);
            }
            self.apply_fade(l, r);
            done += n;
        }

        let master = p.master.get();
        for i in 0..out_l.len() {
            out_l[i] = (out_l[i] * master).clamp(-1.0, 1.0);
            out_r[i] = (out_r[i] * master).clamp(-1.0, 1.0);
        }

        // The stop at the end uses the player's own head, not a target.
        let head = self.playhead();
        let shown = self.shown_head();
        p.playhead.store(shown, Relaxed);
        if p.loop_region().is_none() && head >= self.frames as u64 {
            p.playing.store(false, Relaxed);
        }
    }

    fn apply_fade(&mut self, l: &mut [f32], r: &mut [f32]) {
        match self.fade {
            Fade::None => {}
            Fade::Out { k, target } => {
                for i in 0..l.len() {
                    let g = self.fout[k + i];
                    l[i] *= g;
                    r[i] *= g;
                }
                let k = k + l.len();
                if k >= self.cfg.crossfade {
                    self.jump_to(target);
                    self.fade = Fade::In { k: 0 };
                } else {
                    self.fade = Fade::Out { k, target };
                }
            }
            Fade::In { k } => {
                let m = l.len().min(self.cfg.crossfade - k);
                for i in 0..m {
                    let g = self.fin[k + i];
                    l[i] *= g;
                    r[i] *= g;
                }
                self.fade = if k + m >= self.cfg.crossfade {
                    Fade::None
                } else {
                    Fade::In { k: k + m }
                };
            }
        }
    }

    /// Moves the read position, takes the loop of `params`, and resets the
    /// stretcher.
    fn jump_to(&mut self, target: u64) {
        self.pos = target.min(self.frames as u64);
        self.fresh = true;
        self.looping = self.params.loop_region();
        self.stretcher.reset();
        self.anchors.clear();
        self.pushed = 0;
        self.consumed = 0.0;
        self.xf_old = None;
        self.xf_k = 0;
        self.params.playhead.store(self.pos, Relaxed);
    }

    fn render_stretched(&mut self, l: &mut [f32], r: &mut [f32]) {
        let tempo = self.params.tempo() as f64;
        self.stretcher.set_tempo(tempo);
        let hop = self.cfg.hop;
        let mut guard = 0;
        while !self.stretcher.can_pull(l.len()) && guard < 1024 {
            self.read_mixed(hop);
            self.stretcher.push(&self.in_l[..hop], &self.in_r[..hop]);
            guard += 1;
        }
        self.stretcher.pull(l, r);
        self.consumed += l.len() as f64 * tempo;
    }

    /// Chipmunk mode: read twice as many frames and keep every second one,
    /// as `dsp::double_speed` does, but write them into `l` and `r` without
    /// allocating (RT-1).  Bypasses the stretcher.
    fn render_chipmunk(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n2 = l.len() * 2;
        let mut got = 0;
        while got < n2 {
            let m = (n2 - got).min(self.cfg.hop);
            self.read_mixed(m);
            self.chip_l[got..got + m].copy_from_slice(&self.in_l[..m]);
            self.chip_r[got..got + m].copy_from_slice(&self.in_r[..m]);
            got += m;
        }
        for i in 0..l.len() {
            l[i] = self.chip_l[2 * i];
            r[i] = self.chip_r[2 * i];
        }
        // No stretcher in the path: what was just read is what is heard.
        self.anchors.clear();
        self.pushed = 0;
        self.consumed = 0.0;
    }

    /// Reads `n` frames of every stem at the shared position, applies the
    /// gains (ramped across the block), sums them into `in_l`/`in_r`, and
    /// handles loop wraps with a crossfade.
    fn read_mixed(&mut self, n: usize) {
        self.fresh = false;
        let looping = self.looping;

        // 1. Work out which song frame each output frame comes from.
        for i in 0..n {
            if let Some((s, e)) = loop_in_force(looping, self.frames as u64)
                && self.pos >= e
            {
                if self.cfg.crossfade > 0 {
                    self.xf_old = Some(self.pos);
                    self.xf_k = 0;
                }
                self.pos = dsp::wrap_loop(self.pos as usize, s as usize, e as usize) as u64;
                self.drop_oldest_anchor_when_full();
                self.anchors.push_back(Anchor {
                    input_index: self.pushed + i as u64,
                    song_pos: self.pos,
                });
            }
            if self.anchors.is_empty() {
                self.anchors.push_back(Anchor {
                    input_index: self.pushed + i as u64,
                    song_pos: self.pos,
                });
            }
            self.pos_buf[i] = self.pos;
            self.pos += 1;
            match self.xf_old {
                Some(old) if self.xf_k < self.cfg.crossfade => {
                    self.old_buf[i] = Some(old);
                    self.xf_buf[i] = self.xf_k;
                    self.xf_old = Some(old + 1);
                    self.xf_k += 1;
                }
                _ => {
                    self.old_buf[i] = None;
                    self.xf_old = None;
                }
            }
        }
        self.pushed += n as u64;

        // 2. Read each lane with a gain, one run of consecutive frames at a
        // time, and mix with gains ramped from the current to the target
        // values.
        self.in_l[..n].fill(0.0);
        self.in_r[..n].fill(0.0);
        for (si, source) in self.sources.iter().enumerate() {
            let g0 = self.gains[si];
            let g1 = self.params.effective_gain(si);
            self.gains[si] = g1;
            if g0 == 0.0 && g1 == 0.0 {
                continue;
            }
            let mut i = 0;
            while i < n {
                let start = i;
                i += 1;
                while i < n && self.pos_buf[i] == self.pos_buf[i - 1] + 1 {
                    i += 1;
                }
                source.read(
                    self.pos_buf[start],
                    &mut self.lane_l[start..i],
                    &mut self.lane_r[start..i],
                );
            }
            let mut i = 0;
            while i < n {
                let Some(from) = self.old_buf[i] else {
                    i += 1;
                    continue;
                };
                let start = i;
                i += 1;
                while i < n && self.old_buf[i] == Some(from + (i - start) as u64) {
                    i += 1;
                }
                source.read(from, &mut self.old_l[start..i], &mut self.old_r[start..i]);
            }
            let step = (g1 - g0) / n as f32;
            for i in 0..n {
                let g = g0 + step * (i + 1) as f32;
                let (mut vl, mut vr) = (self.lane_l[i], self.lane_r[i]);
                if self.old_buf[i].is_some() {
                    let k = self.xf_buf[i];
                    let (ol, or) = (self.old_l[i], self.old_r[i]);
                    vl = vl * self.fin[k] + ol * self.fout[k];
                    vr = vr * self.fin[k] + or * self.fout[k];
                }
                self.in_l[i] += vl * g;
                self.in_r[i] += vr * g;
            }
        }
    }

    /// Keeps `anchors` within its capacity, so the audio callback does not
    /// allocate (RT-1).  Only a loop of under 16 frames fills it.  The
    /// anchor dropped can be the one being heard, and the playhead then
    /// shows the loop's start early, while the stretcher's buffered audio
    /// plays; the audio does not change.
    fn drop_oldest_anchor_when_full(&mut self) {
        if self.anchors.len() == self.anchors.capacity() {
            self.anchors.pop_front();
        }
    }

    /// The song frame being heard right now: the read position, minus what
    /// is still inside the stretcher, mapped back through any loop jumps.
    pub fn playhead(&mut self) -> u64 {
        if self.chipmunk || self.anchors.is_empty() {
            return self.pos;
        }
        let tempo = self.params.tempo() as f64;
        let audible = self.consumed - self.stretcher.latency() * tempo;
        if audible < 0.0 {
            return self.anchors[0].song_pos;
        }
        while self.anchors.len() > 1 && (self.anchors[1].input_index as f64) <= audible {
            self.anchors.pop_front();
        }
        let a = self.anchors[0];
        a.song_pos + (audible - a.input_index as f64).max(0.0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{Samples, Stem};
    use crate::source::MemorySource;
    use crate::stretch::Varispeed;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU64;

    fn song(stems: Vec<(&str, Vec<f32>)>) -> Arc<Song> {
        let frames = stems[0].1.len();
        Arc::new(Song {
            sample_rate: 44_100,
            frames,
            stems: stems
                .into_iter()
                .map(|(n, v)| Stem {
                    name: n.into(),
                    left: v.clone().into(),
                    right: v.into(),
                })
                .collect(),
        })
    }

    fn player(song: Arc<Song>, crossfade: usize) -> Player {
        let params = Arc::new(SharedParams::new(song.stems.len()));
        params.playing.store(true, Relaxed);
        Player::with_config(
            song,
            params,
            Box::new(Varispeed::new()),
            PlayerConfig {
                crossfade,
                ..Default::default()
            },
        )
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 - 0.5) * 0.4
            })
            .collect()
    }

    #[test]
    fn identity_render_equals_dsp_mix() {
        let a = noise(10_000, 1);
        let b = noise(10_000, 2);
        let s = song(vec![("a", a.clone()), ("b", b.clone())]);
        let mut p = player(s, 256);
        p.params.set_volume(0, 0.5);
        p.params.set_volume(1, 0.75);
        // Start from settled gains so no ramp is involved.
        p.gains = vec![0.5, 0.75];
        let [l, _] = p.render(9_000);
        let reference = dsp::mix(&[&a[..9_000], &b[..9_000]], &[0.5, 0.75]);
        for i in 0..9_000 {
            assert!((l[i] - reference[i]).abs() < 1e-6, "frame {i}");
        }
    }

    #[test]
    fn seek_lands_within_one_block() {
        let s = song(vec![("a", noise(100_000, 3))]);
        let mut p = player(s, 256);
        p.render(1_000);
        p.params.request_seek(50_000);
        p.render(512);
        let h = p.params.playhead();
        // After the fade-out (256 frames) and fade-in, the head is just past
        // the target, by at most one block.
        assert!((50_000..=50_000 + 512).contains(&h), "playhead {h}");
    }

    #[test]
    fn paused_player_is_silent_and_seeks_immediately() {
        let s = song(vec![("a", vec![0.5; 1000])]);
        let mut p = player(s, 256);
        p.params.playing.store(false, Relaxed);
        p.params.request_seek(700);
        let [l, _] = p.render(100);
        assert!(l.iter().all(|v| *v == 0.0));
        assert_eq!(p.pos, 700);
    }

    #[test]
    fn stops_at_the_end_without_a_loop() {
        let s = song(vec![("a", vec![0.1; 2_000])]);
        let mut p = player(s, 256);
        p.render(3_000);
        assert!(!p.params.playing.load(Relaxed));
    }

    #[test]
    fn loop_wraps_and_playhead_stays_inside() {
        let ramp: Vec<f32> = (0..20_000).map(|i| i as f32 / 20_000.0).collect();
        let s = song(vec![("a", ramp)]);
        let mut p = player(s, 0);
        p.params.set_loop(1_000, 3_000);
        for _ in 0..40 {
            p.render(256);
            let h = p.params.playhead();
            assert!(h < 3_000, "playhead {h} escaped the loop");
        }
        // Without a crossfade, output after the wrap is the loop start.
        let [l, _] = p.render(2_000);
        assert!(l.iter().all(|v| (0.0..0.15 + 1e-3).contains(v)));
    }

    fn with_hop(hop: usize) -> Player {
        let s = song(vec![("a", vec![0.0; 1_000])]);
        let params = Arc::new(SharedParams::new(1));
        let cfg = PlayerConfig {
            hop,
            ..Default::default()
        };
        Player::with_config(s, params, Box::new(Varispeed::new()), cfg)
    }

    #[test]
    fn the_largest_hop_the_warm_up_covers_is_accepted() {
        assert_eq!(with_hop(MAX_PUSH_FRAMES).cfg.hop, MAX_PUSH_FRAMES);
    }

    #[test]
    #[should_panic(expected = "RT-1: a hop of")]
    fn a_hop_the_warm_up_does_not_cover_is_refused() {
        with_hop(MAX_PUSH_FRAMES + 1);
    }

    #[test]
    fn chipmunk_matches_dsp_double_speed() {
        let rising: Vec<f32> = (0..4_000).map(|i| i as f32 / 4_000.0).collect();
        let falling: Vec<f32> = rising.iter().map(|v| 1.0 - v).collect();
        let s = Arc::new(Song {
            sample_rate: 44_100,
            frames: rising.len(),
            stems: vec![Stem {
                name: "lloyd".into(),
                left: rising.clone().into(),
                right: falling.clone().into(),
            }],
        });
        let mut p = player(s, 256);
        p.gains = vec![1.0];
        p.params.chipmunk.store(true, Relaxed);
        let [l, r] = p.render(1_000);
        assert_eq!(l, dsp::double_speed(&rising[..2_000]));
        assert_eq!(r, dsp::double_speed(&falling[..2_000]));
    }

    #[test]
    fn a_very_short_loop_keeps_its_anchors() {
        let s = song(vec![("a", noise(20_000, 4))]);
        let mut p = player(s, 256);
        let capacity = p.anchors.capacity();
        p.params.set_tempo(1.5);
        p.params.set_loop(1_000, 1_001);
        p.render(8_192);
        assert_eq!(p.anchors.capacity(), capacity, "RT-1: the queue grew");
        let h = p.params.playhead();
        assert!((1_000..=1_001).contains(&h), "playhead {h}");
    }

    /// 64-bit FNV-1a over the bits of samples: no crate and no libm, so
    /// the same on every platform.
    struct Fnv(u64);

    impl Fnv {
        fn new() -> Self {
            Self(0xcbf2_9ce4_8422_2325)
        }

        fn add(&mut self, samples: &[f32]) {
            for v in samples {
                for byte in v.to_bits().to_le_bytes() {
                    self.0 ^= u64::from(byte);
                    self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
    }

    /// Varispeed that also hashes every frame the player pushes into it.
    struct Recorder {
        inner: Varispeed,
        pushed: Arc<std::sync::Mutex<Fnv>>,
    }

    impl Stretcher for Recorder {
        fn name(&self) -> &'static str {
            "recorder"
        }
        fn set_tempo(&mut self, tempo: f64) {
            self.inner.set_tempo(tempo);
        }
        fn push(&mut self, left: &[f32], right: &[f32]) {
            let mut hash = self.pushed.lock().expect("lock");
            hash.add(left);
            hash.add(right);
            self.inner.push(left, right);
        }
        fn can_pull(&mut self, frames: usize) -> bool {
            self.inner.can_pull(frames)
        }
        fn pull(&mut self, left: &mut [f32], right: &mut [f32]) {
            self.inner.pull(left, right);
        }
        fn latency(&self) -> f64 {
            self.inner.latency()
        }
        fn reset(&mut self) {
            self.inner.reset();
        }
        fn warm_up(&mut self, max_tempo: f64, max_frames: usize) {
            self.inner.warm_up(max_tempo, max_frames);
        }
    }

    /// Plays three stems of noise through every path of the player: the
    /// hash of what it pushes into the stretcher, and the hash of its
    /// output through `Varispeed` (chipmunk output included).
    fn golden_hashes() -> (u64, u64) {
        let frames = 200_000;
        let s = song(vec![
            ("a", noise(frames, 11)),
            ("b", noise(frames, 12)),
            ("c", noise(frames, 13)),
        ]);
        let params = Arc::new(SharedParams::new(3));
        let pushed = Arc::new(std::sync::Mutex::new(Fnv::new()));
        let recorder = Recorder {
            inner: Varispeed::new(),
            pushed: Arc::clone(&pushed),
        };
        let mut p = Player::with_config(
            s,
            Arc::clone(&params),
            Box::new(recorder),
            PlayerConfig::default(),
        );
        let mut out = Fnv::new();
        let mut play = |p: &mut Player, callbacks: usize, size: usize| {
            for _ in 0..callbacks {
                let [l, r] = p.render(size);
                out.add(&l);
                out.add(&r);
            }
        };
        params.playing.store(true, Relaxed);
        play(&mut p, 10, 1_000);
        params.set_tempo(0.75);
        play(&mut p, 10, 777);
        params.set_tempo(1.5);
        play(&mut p, 10, 1_024);
        params.set_tempo(1.0);
        params.chipmunk.store(true, Relaxed);
        play(&mut p, 5, 900);
        params.chipmunk.store(false, Relaxed);
        play(&mut p, 5, 900);
        // A loop with a crossfade around the head.
        let head = params.playhead();
        params.set_loop(head - 1_000, head + 3_000);
        play(&mut p, 20, 512);
        // A loop shorter than a hop.
        let head = params.playhead();
        params.set_loop(head, head + 100);
        play(&mut p, 5, 512);
        // A loop set behind the head.
        params.clear_loop();
        play(&mut p, 5, 512);
        let head = params.playhead();
        params.set_loop(head - 20_000, head - 5_000);
        play(&mut p, 20, 1_000);
        params.clear_loop();
        // A seek while playing.
        params.request_seek(120_000);
        play(&mut p, 5, 1_000);
        // A paused seek, then Play.
        params.playing.store(false, Relaxed);
        play(&mut p, 1, 500);
        params.request_seek(60_000);
        play(&mut p, 1, 500);
        params.playing.store(true, Relaxed);
        play(&mut p, 5, 1_000);
        // Mute and solo.
        params.set_muted(0, true);
        play(&mut p, 3, 1_000);
        params.set_soloed(1, true);
        play(&mut p, 3, 1_000);
        params.set_muted(0, false);
        play(&mut p, 3, 1_000);
        params.set_soloed(1, false);
        play(&mut p, 3, 1_000);
        // On to the end of the song.
        params.request_seek(frames as u64 - 3_000);
        play(&mut p, 6, 1_000);
        assert!(!params.playing.load(Relaxed), "the song ends");
        let pushed = pushed.lock().expect("lock").0;
        (pushed, out.0)
    }

    /// The player's output, bit for bit, as it was before the player read
    /// its stems through `StemSource` (M6, D2).
    #[test]
    fn the_player_output_is_unchanged() {
        let (pushed, out) = golden_hashes();
        assert_eq!(
            (pushed, out),
            (0xea6e_e1f9_d4b9_abb9, 0x866d_28a2_377d_a34d),
            "the player's output changed: pushed {pushed:#018x}, output {out:#018x}"
        );
    }

    #[test]
    fn chipmunk_reads_twice_as_fast() {
        let ramp: Vec<f32> = (0..10_000).map(|i| i as f32 / 10_000.0).collect();
        let s = song(vec![("a", ramp)]);
        let mut p = player(s, 256);
        p.params.chipmunk.store(true, Relaxed);
        let [l, _] = p.render(1_000);
        assert!((l[500] - 1_000.0 / 10_000.0).abs() < 1e-6);
        assert_eq!(p.params.playhead(), 2_000);
    }

    /// A memory lane whose frames are ready only in `lo..hi`, which the
    /// test sets.  It counts the frames it gave as 0.0 for want of them,
    /// and records every hint.
    struct Gated {
        inner: MemorySource,
        lo: AtomicU64,
        hi: AtomicU64,
        missed: AtomicU64,
        hints: Mutex<Vec<Hint>>,
    }

    impl Gated {
        fn new(left: Samples, right: Samples) -> Arc<Self> {
            Arc::new(Self {
                inner: MemorySource::new(left, right),
                lo: AtomicU64::new(0),
                hi: AtomicU64::new(u64::MAX),
                missed: AtomicU64::new(0),
                hints: Mutex::new(Vec::new()),
            })
        }

        fn window(&self, lo: u64, hi: u64) {
            self.lo.store(lo, Relaxed);
            self.hi.store(hi, Relaxed);
        }

        fn open(&self) {
            self.window(0, u64::MAX);
        }

        fn holds(&self, frame: u64) -> bool {
            (self.lo.load(Relaxed)..self.hi.load(Relaxed)).contains(&frame)
        }

        fn last_seek_hint(&self) -> Option<Option<u64>> {
            self.hints
                .lock()
                .expect("lock")
                .iter()
                .rev()
                .find_map(|h| match h {
                    Hint::Seek(t) => Some(*t),
                    Hint::Reading { .. } => None,
                })
        }
    }

    impl StemSource for Gated {
        fn read(&self, from: u64, left: &mut [f32], right: &mut [f32]) {
            self.inner.read(from, left, right);
            for i in 0..left.len() {
                if !self.holds(from + i as u64) {
                    left[i] = 0.0;
                    right[i] = 0.0;
                    self.missed.fetch_add(1, Relaxed);
                }
            }
        }

        fn ready(&self, from: u64, frames: usize) -> bool {
            frames == 0 || (self.holds(from) && self.holds(from + frames as u64 - 1))
        }

        fn hint(&self, hint: Hint) {
            self.hints.lock().expect("lock").push(hint);
        }
    }

    const TWIN_FRAMES: usize = 400_000;

    /// A player of one gated lane, and a player of the same lane in
    /// memory, driven with the same controls.
    struct Twin {
        lane: Arc<Gated>,
        gated: Player,
        memory: Player,
    }

    impl Twin {
        fn new() -> Self {
            let ramp: Vec<f32> = (0..TWIN_FRAMES)
                .map(|i| i as f32 / TWIN_FRAMES as f32)
                .collect();
            let (left, right): (Samples, Samples) = (ramp.into(), noise(TWIN_FRAMES, 9).into());
            let lane = Gated::new(left.clone(), right.clone());
            let make = |source: Arc<dyn StemSource>| {
                Player::with_sources(
                    44_100,
                    TWIN_FRAMES,
                    Arc::new(SharedParams::new(1)),
                    Box::new(Varispeed::new()),
                    PlayerConfig::default(),
                    vec![source],
                )
            };
            Self {
                gated: make(Arc::clone(&lane) as Arc<dyn StemSource>),
                memory: make(Arc::new(MemorySource::new(left, right))),
                lane,
            }
        }

        /// Sets the same control on both players.
        fn both(&self, f: impl Fn(&SharedParams)) {
            f(self.gated.params());
            f(self.memory.params());
        }

        fn gated_params(&self) -> &SharedParams {
            self.gated.params()
        }

        /// Plays `callbacks` callbacks of 512 frames on both, and gives
        /// the outputs of the gated player and of the memory one.
        fn play(&mut self, callbacks: usize) -> (Vec<f32>, Vec<f32>) {
            let (mut gated, mut memory) = (Vec::new(), Vec::new());
            for _ in 0..callbacks {
                let [l, r] = self.gated.render(512);
                gated.extend(l.iter().chain(&r));
                let [l, r] = self.memory.render(512);
                memory.extend(l.iter().chain(&r));
            }
            (gated, memory)
        }

        /// Plays the gated player only.
        fn play_gated(&mut self, callbacks: usize) -> Vec<f32> {
            let mut out = Vec::new();
            for _ in 0..callbacks {
                let [l, _] = self.gated.render(512);
                out.extend(l);
            }
            out
        }

        fn held_frames(&self) -> u64 {
            self.gated_params().held_frames.load(Relaxed)
        }

        fn held_seeks(&self) -> u64 {
            self.gated_params().held_seeks.load(Relaxed)
        }
    }

    #[test]
    fn a_seek_holds_for_its_block_and_the_old_position_plays() {
        let mut twin = Twin::new();
        twin.lane.window(0, 100_000);
        twin.both(|p| p.playing.store(true, Relaxed));
        let (g, m) = twin.play(20);
        assert_eq!(g, m);
        // The gated player holds the seek; the memory player plays on from
        // the old position too, because it gets no seek yet.
        twin.gated_params().request_seek(200_000);
        let (g, m) = twin.play(40);
        assert_eq!(g, m, "the old position plays while the seek holds");
        assert_eq!(twin.held_seeks(), 1);
        assert_eq!(twin.lane.last_seek_hint(), Some(Some(200_000)));
        assert_eq!(twin.lane.missed.load(Relaxed), 0, "no underrun");
        // The block arrives: both seek now and play the same.
        twin.lane.open();
        twin.memory.params().request_seek(200_000);
        let (g, m) = twin.play(20);
        assert_eq!(g, m, "the seek fades out and jumps as in memory");
        assert_eq!(
            twin.lane.last_seek_hint(),
            Some(None),
            "the hint is cleared"
        );
        let head = twin.gated_params().playhead();
        assert!((200_000..210_000).contains(&head), "playhead {head}");
        assert_eq!(twin.held_frames(), 0);
        assert_eq!(twin.lane.missed.load(Relaxed), 0);
    }

    #[test]
    fn a_held_seek_shows_its_target() {
        let mut twin = Twin::new();
        twin.lane.window(0, 100_000);
        twin.both(|p| p.playing.store(true, Relaxed));
        twin.play_gated(10);
        twin.gated_params().request_seek(300_000);
        for _ in 0..10 {
            twin.play_gated(1);
            assert_eq!(twin.gated_params().playhead(), 300_000);
        }
        // A newer seek replaces the held one.
        twin.gated_params().request_seek(250_000);
        twin.play_gated(1);
        assert_eq!(twin.gated_params().playhead(), 250_000);
        assert_eq!(twin.lane.last_seek_hint(), Some(Some(250_000)));
        assert!(twin.gated.pos < 100_000, "the player itself has not moved");
    }

    #[test]
    fn a_paused_seek_jumps_at_once() {
        let mut twin = Twin::new();
        twin.lane.window(0, 100_000);
        twin.play_gated(1);
        twin.gated_params().request_seek(300_000);
        twin.play_gated(1);
        assert_eq!(twin.gated.pos, 300_000);
        assert_eq!(twin.gated_params().playhead(), 300_000);
        assert_eq!(twin.held_seeks(), 0);
        // A pause turns a held seek into a paused seek.
        twin.lane.window(0, 100_000);
        twin.gated.jump_to(0);
        twin.gated_params().playing.store(true, Relaxed);
        twin.play_gated(4);
        twin.gated_params().request_seek(350_000);
        twin.play_gated(1);
        assert_eq!(twin.held_seeks(), 1);
        twin.gated_params().playing.store(false, Relaxed);
        twin.play_gated(1);
        assert_eq!(twin.gated.pos, 350_000, "the held seek jumps when paused");
        assert_eq!(twin.lane.last_seek_hint(), Some(None));
    }

    #[test]
    fn a_jump_to_a_position_not_ready_holds_and_counts_no_underrun() {
        // A paused seek, then Play.
        let mut twin = Twin::new();
        twin.lane.window(0, 0);
        twin.both(|p| p.request_seek(100_000));
        twin.play(1);
        twin.both(|p| p.playing.store(true, Relaxed));
        let held = twin.play_gated(10);
        assert!(held.iter().all(|v| *v == 0.0), "a hold is silent");
        assert_eq!(twin.held_frames(), 10 * 512);
        assert_eq!(twin.gated_params().playhead(), 100_000, "the head waits");
        twin.lane.open();
        let (g, m) = twin.play(10);
        assert_eq!(g, m, "it then plays the target as in memory");
        assert_eq!(twin.lane.missed.load(Relaxed), 0, "a hold is no underrun");
        assert_eq!(twin.held_seeks(), 0);

        // A chipmunk change.
        let mut twin = Twin::new();
        twin.both(|p| p.playing.store(true, Relaxed));
        twin.play(10);
        twin.lane.window(0, 0);
        twin.both(|p| p.chipmunk.store(true, Relaxed));
        let held = twin.play_gated(5);
        assert!(held.iter().all(|v| *v == 0.0));
        assert_eq!(twin.held_frames(), 5 * 512);
        twin.lane.open();
        let (g, m) = twin.play(10);
        assert_eq!(g, m);
        assert_eq!(twin.lane.missed.load(Relaxed), 0);

        // A new player.
        let mut twin = Twin::new();
        twin.lane.window(0, 0);
        twin.both(|p| p.playing.store(true, Relaxed));
        twin.play_gated(3);
        assert_eq!(twin.held_frames(), 3 * 512);
        twin.lane.open();
        let (g, m) = twin.play(10);
        assert_eq!(g, m);
        assert_eq!(twin.lane.missed.load(Relaxed), 0);
    }

    #[test]
    fn a_loop_set_behind_the_head_is_a_held_seek() {
        // Its target is not ready: the old position plays on.
        let mut twin = Twin::new();
        twin.both(|p| p.playing.store(true, Relaxed));
        twin.play(100);
        let head = twin.gated.pos;
        twin.lane.window(head - 1_000, u64::MAX);
        let (s, e) = (head - 30_000, head - 10_000);
        twin.gated_params().set_loop(s, e);
        let target = dsp::wrap_loop(head as usize, s as usize, e as usize) as u64;
        let (g, m) = twin.play(10);
        assert_eq!(g, m, "the old position plays on with no loop");
        assert_eq!(twin.held_seeks(), 1);
        assert_eq!(twin.lane.last_seek_hint(), Some(Some(target)));
        assert_eq!(twin.gated_params().playhead(), target);
        // The target arrives: it fades out, jumps and takes the loop.
        twin.lane.open();
        twin.memory.params().request_seek(target);
        twin.memory.params().set_loop(s, e);
        let (g, m) = twin.play(100);
        assert_eq!(g, m);
        let head = twin.gated_params().playhead();
        assert!((s..e).contains(&head), "playhead {head} in the loop");
        assert_eq!(twin.lane.missed.load(Relaxed), 0);
        assert_eq!(twin.held_frames(), 0);

        // Its target is ready: it wraps at once, with the loop crossfade,
        // as a memory player does.
        let mut twin = Twin::new();
        twin.both(|p| p.playing.store(true, Relaxed));
        twin.play(100);
        let head = twin.gated.pos;
        twin.both(|p| p.set_loop(head - 30_000, head - 10_000));
        let (g, m) = twin.play(100);
        assert_eq!(g, m);
        assert_eq!(twin.held_seeks(), 0);
        let (s, e) = (head - 30_000, head - 10_000);
        let w = dsp::wrap_loop(head as usize, s as usize, e as usize) as u64;
        let ramp_at = |frame: u64| frame as f32 / TWIN_FRAMES as f32;
        // The first frame read after the wrap mixes the old and the new.
        let mut fresh = Twin::new();
        fresh.gated.jump_to(w);
        fresh.gated_params().playing.store(true, Relaxed);
        let jumped = fresh.play_gated(1);
        assert_ne!(g[..512], jumped[..], "a wrap with the crossfade, not a cut");
        assert!(g[0] > ramp_at(w), "the old position still sounds");
    }

    #[test]
    fn a_loop_that_ends_on_a_read_wraps_with_no_hold() {
        let mut twin = Twin::new();
        // 16 hops of 256 frames: a read ends exactly at the loop's end.
        twin.both(|p| p.set_loop(0, 4_096));
        twin.lane.window(0, 4_096 + 256);
        twin.both(|p| p.playing.store(true, Relaxed));
        let (g, m) = twin.play(40);
        assert_eq!(g, m, "it wraps with the crossfade, as in memory");
        assert_eq!(twin.held_frames(), 0);
        assert_eq!(twin.held_seeks(), 0);
        assert_eq!(twin.lane.missed.load(Relaxed), 0);
        assert!(twin.gated_params().playhead() < 4_096);
    }

    #[test]
    fn the_player_hints_while_paused() {
        let mut twin = Twin::new();
        twin.gated_params().request_seek(1_234);
        twin.play_gated(1);
        twin.lane.hints.lock().expect("lock").clear();
        twin.play_gated(2);
        let hints = twin.lane.hints.lock().expect("lock").clone();
        let reading = Hint::Reading {
            heard: 1_234,
            read: 1_234,
        };
        assert_eq!(
            hints,
            [reading, Hint::Seek(None), reading, Hint::Seek(None)]
        );
    }
}
