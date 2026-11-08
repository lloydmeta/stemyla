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
