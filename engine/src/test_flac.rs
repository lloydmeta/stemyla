//! FLAC files for the tests, made in memory (ASSET-1).  `compressed` uses
//! `flacenc`, which writes 16 bits well but not 24.  `by_hand` writes
//! verbatim or constant subframes bit by bit (RFC 9639), for 24-bit files
//! and for long silence, without `flacenc`.

use flacenc::component::BitRepr;
use flacenc::error::Verify;

/// A 16-bit stereo FLAC of `left` and `right`, compressed by `flacenc`.
pub fn compressed(left: &[f32], right: &[f32], rate: u32) -> Vec<u8> {
    const BLOCK: usize = 4096;
    let interleaved: Vec<i32> = left
        .iter()
        .zip(right)
        .flat_map(|(l, r)| [step(*l, 16), step(*r, 16)])
        .collect();
    let source = flacenc::source::MemSource::from_samples(&interleaved, 2, 16, rate as usize);
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .expect("the default config");
    let mut stream =
        flacenc::encode_with_fixed_block_size(&config, source, BLOCK).expect("encoded");
    // flacenc gives the short last block as the minimum, which RFC 9639
    // (8.2) excludes, and Symphonia then rejects every frame.
    stream
        .stream_info_mut()
        .set_block_sizes(BLOCK, BLOCK)
        .expect("block sizes");
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).expect("written");
    sink.into_inner()
}

/// `x` as a signed sample of `bits` bits, rounded and clipped at full
/// scale, as `encode_pcm16` does.
pub fn step(x: f32, bits: u32) -> i32 {
    let full = (1_i64 << (bits - 1)) as f64;
    (f64::from(x) * full).round().clamp(-full, full - 1.0) as i32
}

/// The audio of a hand-made file.
pub enum Blocks<'a> {
    /// These channels' samples, in verbatim subframes of 4,096 frames.
    Verbatim(&'a [Vec<i32>]),
    /// `blocks` frames of `block` samples of silence, in constant
    /// subframes: about 17 bytes a FLAC frame, however long the block.
    Silence { block: usize, blocks: usize },
}

/// A FLAC file of `channels` channels at `rate` and `bits`, whose
/// STREAMINFO states `stated` frames (0 means "unknown").
pub fn by_hand(rate: u32, bits: u32, channels: usize, stated: u64, blocks: Blocks) -> Vec<u8> {
    const VERBATIM_BLOCK: usize = 4096;
    let block = match blocks {
        Blocks::Verbatim(_) => VERBATIM_BLOCK,
        Blocks::Silence { block, .. } => block,
    };
    let mut out = Bits::default();
    out.bytes.extend_from_slice(b"fLaC");
    out.put(1, 1); // the last metadata block
    out.put(0, 7); // STREAMINFO
    out.put(34, 24);
    out.put(block as u64, 16);
    out.put(block as u64, 16);
    out.put(0, 24); // frame sizes unknown
    out.put(0, 24);
    out.put(u64::from(rate), 20);
    out.put(channels as u64 - 1, 3);
    out.put(u64::from(bits) - 1, 5);
    out.put(stated, 36);
    out.put(0, 64); // MD5 unknown
    out.put(0, 64);
    let mut bytes = out.bytes;
    match blocks {
        Blocks::Verbatim(samples) => {
            let frames = samples[0].len();
            for (number, start) in (0..frames).step_by(VERBATIM_BLOCK).enumerate() {
                let end = frames.min(start + VERBATIM_BLOCK);
                let subframes: Vec<&[i32]> = samples.iter().map(|c| &c[start..end]).collect();
                bytes.extend(frame(number, bits, &subframes, end - start));
            }
        }
        Blocks::Silence { block, blocks } => {
            let silence: Vec<&[i32]> = vec![&[0]; channels];
            for number in 0..blocks {
                bytes.extend(frame(number, bits, &silence, block));
            }
        }
    }
    bytes
}

/// One FLAC frame with independent channels.  A subframe of one sample is
/// constant; any other is verbatim.  The rate and the bits come from
/// STREAMINFO.
fn frame(number: usize, bits: u32, subframes: &[&[i32]], len: usize) -> Vec<u8> {
    let mut f = Bits::default();
    f.put(0x3ffe, 14); // sync
    f.put(0, 1);
    f.put(0, 1); // fixed blocking: a frame number
    f.put(0b0111, 4); // block size - 1 follows, in 16 bits
    f.put(0, 4); // rate from STREAMINFO
    f.put(subframes.len() as u64 - 1, 4);
    f.put(0, 3); // bits from STREAMINFO
    f.put(0, 1);
    f.bytes.extend(coded_number(number));
    f.put(len as u64 - 1, 16);
    let crc = crc8(&f.bytes);
    f.put(u64::from(crc), 8);
    for samples in subframes {
        let kind = if samples.len() == 1 { 0 } else { 1 };
        f.put(0, 1);
        f.put(kind, 6);
        f.put(0, 1); // no wasted bits
        for s in *samples {
            f.put(u64::from(*s as u32) & ((1 << bits) - 1), bits);
        }
    }
    f.align();
    let crc = crc16(&f.bytes);
    f.put(u64::from(crc), 16);
    f.bytes
}

/// The frame number in FLAC's form of UTF-8 (RFC 9639, 9.1.5).
fn coded_number(n: usize) -> Vec<u8> {
    let n = n as u32;
    let tail = |shift: u32| 0x80 | ((n >> shift) & 0x3f) as u8;
    match n {
        0..0x80 => vec![n as u8],
        0x80..0x800 => vec![0xc0 | (n >> 6) as u8, tail(0)],
        0x800..0x1_0000 => vec![0xe0 | (n >> 12) as u8, tail(6), tail(0)],
        _ => {
            assert!(n < 0x20_0000, "frame number {n} needs more bytes");
            vec![0xf0 | (n >> 18) as u8, tail(12), tail(6), tail(0)]
        }
    }
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0_u8;
    for b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x07
            };
        }
    }
    crc
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for b in bytes {
        crc ^= u16::from(*b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x8005
            };
        }
    }
    crc
}

/// Bits written most significant first.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    pending: u8,
    count: u32,
}

impl Bits {
    fn put(&mut self, value: u64, bits: u32) {
        for i in (0..bits).rev() {
            self.pending = (self.pending << 1) | ((value >> i) & 1) as u8;
            self.count += 1;
            if self.count == 8 {
                self.bytes.push(self.pending);
                (self.pending, self.count) = (0, 0);
            }
        }
    }

    fn align(&mut self) {
        while self.count != 0 {
            self.put(0, 1);
        }
    }
}
