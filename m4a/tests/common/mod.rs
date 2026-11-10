//! The test signals of the `m4a` tests: interleaved 16-bit stereo.

#![allow(dead_code, reason = "each test file uses some of the signals")]

/// A fixed-seed xorshift generator, so each run gives the same signal.
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Noise(seed)
    }

    pub fn next_i16(&mut self) -> i16 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 16) as i16
    }
}

pub fn silence(frames: usize) -> Vec<i16> {
    vec![0; frames * 2]
}

/// A full-scale square wave of 100 frames for each period, in phase on
/// both channels.
pub fn square(frames: usize) -> Vec<i16> {
    (0..frames)
        .flat_map(|i| {
            let sample = if i % 100 < 50 { i16::MAX } else { i16::MIN };
            [sample, sample]
        })
        .collect()
}

/// A sine of 441 Hz at 44.1 kHz, the same on both channels.
pub fn mono_like(frames: usize) -> Vec<i16> {
    (0..frames)
        .flat_map(|i| {
            let phase = std::f64::consts::TAU * i as f64 / 100.0;
            let sample = (phase.sin() * 20_000.0).round() as i16;
            [sample, sample]
        })
        .collect()
}

/// Random 16-bit samples on both channels.
pub fn random(frames: usize) -> Vec<i16> {
    let mut noise = Noise::new(0x9E37_79B9);
    (0..frames * 2).map(|_| noise.next_i16()).collect()
}

/// In each packet of 4096 frames, the first eighth is silent and the rest
/// is random.  The trial encodes see only the silent part, so they predict
/// a small packet, and the full encode is rewound to an escape packet.
pub fn rewind(frames: usize) -> Vec<i16> {
    let mut noise = Noise::new(0x0BAD_5EED);
    (0..frames)
        .flat_map(|i| {
            if i % 4096 < 4096 / 8 {
                [0, 0]
            } else {
                [noise.next_i16(), noise.next_i16()]
            }
        })
        .collect()
}

/// Makes a signal of the given number of frames.
pub type Signal = fn(usize) -> Vec<i16>;

/// A music-like signal at 44.1 kHz: a chord of 4 sines, a note each half
/// second with a decay, and low-level noise on each channel.
pub fn music(frames: usize) -> Vec<i16> {
    const PARTIALS: [(f64, f64); 4] = [(110.0, 0.30), (220.0, 0.20), (329.63, 0.15), (440.0, 0.10)];
    let mut noise = Noise::new(0x5EED_0001);
    (0..frames)
        .flat_map(|i| {
            let t = i as f64 / 44_100.0;
            let envelope = (-(t % 0.5) * 6.0).exp();
            let chord = |phase: f64| -> f64 {
                PARTIALS
                    .iter()
                    .map(|&(hz, gain)| gain * (std::f64::consts::TAU * hz * t + phase).sin())
                    .sum::<f64>()
                    * envelope
                    * 30_000.0
            };
            let left = chord(0.0) + f64::from(noise.next_i16()) / 256.0;
            let right = chord(0.3) + f64::from(noise.next_i16()) / 256.0;
            [left, right].map(|s| s.round().clamp(-32_768.0, 32_767.0) as i16)
        })
        .collect()
}

/// The signals of the differential test and the round trip, with a name.
pub fn signals() -> [(&'static str, Signal); 5] {
    [
        ("silence", silence),
        ("square", square),
        ("mono-like", mono_like),
        ("random", random),
        ("rewind", rewind),
    ]
}

/// The noise of the mutant probes of review round 1: xorshift32, scaled to
/// `-amp..amp`.
pub struct ScaledNoise {
    state: u32,
    amp: i32,
}

impl ScaledNoise {
    pub fn new(seed: u32, amp: i32) -> Self {
        ScaledNoise { state: seed, amp }
    }

    pub fn next_i16(&mut self) -> i16 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (((self.state >> 16) as i32 - 32_768) * self.amp / 32_768) as i16
    }

    /// The seed of the probes for `amp`.
    pub fn probe(amp: i32) -> Self {
        ScaledNoise::new(0x2545_F491 ^ amp as u32, amp)
    }
}

/// `value` on one channel at one frame in `every`, else silence.
pub fn impulses(frames: usize, value: i16, every: usize, on_left: bool) -> Vec<i16> {
    (0..frames)
        .flat_map(|i| {
            let impulse = if i % every == 0 { value } else { 0 };
            if on_left { [impulse, 0] } else { [0, impulse] }
        })
        .collect()
}

/// Noise of amplitude `amp` on both channels.
pub fn low_noise(frames: usize, seed: u32, amp: i32) -> Vec<i16> {
    let mut noise = ScaledNoise::new(seed, amp);
    (0..frames * 2).map(|_| noise.next_i16()).collect()
}

/// Noise of amplitude `amp` on `percent` % of the frames, else silence.
pub fn sparse_noise(frames: usize, amp: i32, percent: u32) -> Vec<i16> {
    let mut pick = Noise::new(0x0005_EED5);
    let mut noise = ScaledNoise::probe(amp);
    (0..frames)
        .flat_map(|_| {
            if (pick.next_i16() as u16 as u32) % 100 < percent {
                [noise.next_i16(), noise.next_i16()]
            } else {
                [0, 0]
            }
        })
        .collect()
}

/// In each packet, noise of amplitude `amp` in the first eighth if
/// `noise_first`, else in the other seven eighths, and silence elsewhere.
pub fn eighth_split(frames: usize, amp: i32, noise_first: bool) -> Vec<i16> {
    let eighth = frames.min(4096) / 8;
    let mut noise = ScaledNoise::probe(amp);
    (0..frames)
        .flat_map(|i| {
            if (i % 4096 < eighth) == noise_first {
                [noise.next_i16(), noise.next_i16()]
            } else {
                [0, 0]
            }
        })
        .collect()
}

/// The signals that the mutant probes of review round 1 found, with a
/// name.  They reach edges of the Golomb coder, the zero runs, the
/// predictor's adaptation and the escape decision.  The amplitudes of the
/// eighth splits put a packet within a few bits of the escape size, so a
/// change of the escape decision by a few bits changes the bytes.
pub fn probe_signals(frames: usize) -> Vec<(String, Vec<i16>)> {
    let mut signals = Vec::new();
    for (value, every, on_left) in [
        (1, 13, true),
        (1, 31, true),
        (3, 13, true),
        (-20_000, 2, true),
        (3, 14, false),
        (-20_000, 10, false),
    ] {
        signals.push((
            format!("impulses {value} in {every}, on the left: {on_left}"),
            impulses(frames, value, every, on_left),
        ));
    }
    for seed in [1, 77] {
        signals.push((format!("noise 2, seed {seed}"), low_noise(frames, seed, 2)));
    }
    signals.push((
        "sparse noise 2 on 2 %".to_string(),
        sparse_noise(frames, 2, 2),
    ));
    for amp in (9_300..=11_100).step_by(200).chain([10_550, 11_200]) {
        signals.push((
            format!("noise {amp} then silence"),
            eighth_split(frames, amp, true),
        ));
    }
    for amp in (15_600..=15_950)
        .step_by(50)
        .chain([16_020, 16_125, 16_500])
    {
        signals.push((
            format!("silence then noise {amp}"),
            eighth_split(frames, amp, false),
        ));
    }
    signals
}
