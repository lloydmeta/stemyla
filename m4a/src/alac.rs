//! An Apple Lossless (ALAC) encoder for 16-bit stereo.
//!
//! This file is a port of the encoder of `alac-encoder` 0.3.0
//! (<https://github.com/LinusU/rust-alac-encoder>), which is a port of
//! Apple's ALAC reference (<https://github.com/macosforge/alac>).  The port
//! keeps 16-bit stereo only, and it is safe Rust: the bit writer of
//! `crate::bits` replaces the raw pointers, the input is `&[i16]`, and a
//! `match` replaces the `transmute` of the sign.
//!
//! Changes from those sources (Apache-2.0, section 4(b)): this file was
//! changed on 2026-11-08.  It drops mono, 20, 24 and 32 bits, more than 2
//! channels, `FormatDescription` and the log calls.  It drops the predictor
//! branches that 16-bit stereo does not use, and makes the unrolled loops
//! for 4 and 8 coefficients one loop.  Its trial encodes count bits, and
//! keep no bytes.
//!
//! This crate takes `alac-encoder` under Apache-2.0, so the crate is
//! Apache-2.0 only (decision 0028).
//!
//! The notice of `alac-encoder` 0.3.0 (its `LICENSE-MIT`):
//!
//! ```text
//! Copyright (c) 2019 Linus Unnebäck
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all
//! copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.
//! ```
//!
//! The notice of Apple's ALAC reference (`codec/ALACEncoder.cpp`):
//!
//! ```text
//! Copyright (c) 2011 Apple Inc. All rights reserved.
//!
//! @APPLE_APACHE_LICENSE_HEADER_START@
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//!     http://www.apache.org/licenses/LICENSE-2.0
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.
//!
//! @APPLE_APACHE_LICENSE_HEADER_END@
//! ```

use core::cmp::Ordering;
use core::num::Wrapping;

use crate::bits::{BitCount, BitSink, BitWriter};

/// The frames in each packet but the last.  The cookie gives this value.
pub const FRAMES_PER_PACKET: usize = 4096;

/// The largest packet of 16-bit stereo: an escape packet of 4096 frames.
pub const MAX_PACKET_BYTES: usize = 16_388;

const BIT_DEPTH: u32 = 16;
const CHANNELS: usize = 2;
/// The stereo mix adds 1 bit to each sample.
const CHAN_BITS: u32 = BIT_DEPTH + 1;

const ELEMENT_CPE: u32 = 1;
const ELEMENT_END: u32 = 7;

const MIX_BITS: i32 = 2;
const MAX_RES: i32 = 4;
const MIN_UV: usize = 4;
const MAX_UV: usize = 8;
const DEFAULT_NUM_UV: usize = 8;
const PB_FACTOR: u32 = 4;
const MODE: u32 = 0;
const DENSHIFT: u32 = 9;

const MAX_RUN: u16 = 255;
const COMPATIBLE_VERSION: u8 = 0;

/// The parameters of the adaptive Golomb coder.
const PB0: u32 = 40;
const MB0: u32 = 10;
const KB0: u32 = 14;
const WB0: u32 = (1 << KB0) - 1;
const QBSHIFT: u32 = 9;
const QB: u32 = 1 << QBSHIFT;
const MMULSHIFT: u32 = 2;
const MDENSHIFT: u32 = QBSHIFT - MMULSHIFT - 1;
const MOFF: u32 = 1 << (MDENSHIFT - 2);
const BITOFF: u32 = 24;
const MAX_PREFIX_16: u32 = 9;
const MAX_PREFIX_32: u32 = 9;
const MAX_DATATYPE_BITS_16: u32 = 16;
const N_MAX_MEAN_CLAMP: u32 = 0xffff;
const N_MEAN_CLAMP_VAL: u32 = 0xffff;
const MAX_ZERO_RUN: u32 = 65_535;

/// The predictor coefficients of one channel: one set of 4 and one of 8.
type CoefSets = [[i16; MAX_UV]; 2];

/// How a packet was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PacketKind {
    Compressed,
    /// The trial encodes predicted that compression does not pay.
    Escape,
    /// The compressed packet was not smaller than an escape packet, so it
    /// was removed and written again as an escape packet.
    Rewound,
}

/// Encodes 16-bit stereo into ALAC packets, one packet at a time.
///
/// The encoder keeps state from packet to packet: the predictor
/// coefficients, the last stereo mix and its buffers.  So give it the
/// packets of one file in order.
pub struct AlacEncoder {
    sample_rate: u32,
    last_mix_res: i32,
    mix_u: Vec<i32>,
    mix_v: Vec<i32>,
    predictor_u: Vec<i32>,
    predictor_v: Vec<i32>,
    coefs_u: CoefSets,
    coefs_v: CoefSets,
    max_packet_bytes: u32,
}

impl AlacEncoder {
    pub fn new(sample_rate: u32) -> Self {
        let coefs = [initial_coefs(), initial_coefs()];
        AlacEncoder {
            sample_rate,
            last_mix_res: 0,
            mix_u: vec![0; FRAMES_PER_PACKET],
            mix_v: vec![0; FRAMES_PER_PACKET],
            predictor_u: vec![0; FRAMES_PER_PACKET],
            predictor_v: vec![0; FRAMES_PER_PACKET],
            coefs_u: coefs,
            coefs_v: coefs,
            max_packet_bytes: 0,
        }
    }

    /// Appends one packet to `out`, and gives its size in bytes.
    /// `interleaved` holds 1 to 4096 frames, left then right.
    ///
    /// # Panics
    ///
    /// On an odd length, on 0 frames and on more than 4096 frames.
    pub fn encode(&mut self, interleaved: &[i16], out: &mut Vec<u8>) -> usize {
        self.encode_packet(interleaved, out).0
    }

    /// The 24-byte `ALACSpecificConfig`.  It holds the largest packet so
    /// far, so it is final only after the last packet.
    pub fn cookie(&self) -> [u8; 24] {
        let mut cookie = [0; 24];
        cookie[0..4].copy_from_slice(&(FRAMES_PER_PACKET as u32).to_be_bytes());
        cookie[4] = COMPATIBLE_VERSION;
        cookie[5] = BIT_DEPTH as u8;
        cookie[6] = PB0 as u8;
        cookie[7] = MB0 as u8;
        cookie[8] = KB0 as u8;
        cookie[9] = CHANNELS as u8;
        cookie[10..12].copy_from_slice(&MAX_RUN.to_be_bytes());
        cookie[12..16].copy_from_slice(&self.max_packet_bytes.to_be_bytes());
        // Bytes 16 to 19 are the average bit rate: 0, "unknown".
        cookie[20..24].copy_from_slice(&self.sample_rate.to_be_bytes());
        cookie
    }

    pub(crate) fn encode_packet(
        &mut self,
        interleaved: &[i16],
        out: &mut Vec<u8>,
    ) -> (usize, PacketKind) {
        assert!(
            interleaved.len().is_multiple_of(CHANNELS),
            "ALAC input is interleaved stereo, so its length must be even, not {}",
            interleaved.len()
        );
        let frames = interleaved.len() / CHANNELS;
        assert!(
            (1..=FRAMES_PER_PACKET).contains(&frames),
            "an ALAC packet holds 1 to {FRAMES_PER_PACKET} frames, not {frames}"
        );
        let mut bits = BitWriter::new(out);
        bits.write(ELEMENT_CPE, 3);
        bits.write(0, 4);
        let kind = self.encode_stereo(&mut bits, interleaved, frames);
        bits.write(ELEMENT_END, 3);
        bits.byte_align();
        let size = bits.bit_len() / 8;
        self.max_packet_bytes = self.max_packet_bytes.max(size as u32);
        (size, kind)
    }

    fn encode_stereo(
        &mut self,
        bits: &mut BitWriter,
        interleaved: &[i16],
        frames: usize,
    ) -> PacketKind {
        let start = bits.bit_len();
        let partial = frames != FRAMES_PER_PACKET;
        let partial_bits = if partial { 32 } else { 0 };

        // Find the best stereo mix on the first eighth of the packet.
        let eighth = frames / 8;
        let mut min_bits = 1 << 31;
        let mut best_res = self.last_mix_res;
        for mix_res in 0..=MAX_RES {
            mix16(
                &interleaved[..eighth * CHANNELS],
                &mut self.mix_u,
                &mut self.mix_v,
                mix_res,
            );
            pc_block(
                &self.mix_u,
                &mut self.predictor_u,
                eighth,
                coefs(&mut self.coefs_u, DEFAULT_NUM_UV),
            );
            pc_block(
                &self.mix_v,
                &mut self.predictor_v,
                eighth,
                coefs(&mut self.coefs_v, DEFAULT_NUM_UV),
            );
            let mut count = BitCount::default();
            dyn_comp(&self.predictor_u[..eighth], &mut count);
            dyn_comp(&self.predictor_v[..eighth], &mut count);
            if count.bits() < min_bits {
                min_bits = count.bits();
                best_res = mix_res;
            }
        }
        self.last_mix_res = best_res;
        let mix_res = best_res;
        mix16(interleaved, &mut self.mix_u, &mut self.mix_v, mix_res);

        // Find the best number of coefficients for each channel.
        let mut num_u = MIN_UV;
        let mut num_v = MIN_UV;
        let mut min_bits_u: usize = 1 << 31;
        let mut min_bits_v: usize = 1 << 31;
        for num_uv in [MIN_UV, MAX_UV] {
            for _ in 0..8 {
                pc_block(
                    &self.mix_u,
                    &mut self.predictor_u,
                    frames / 32,
                    coefs(&mut self.coefs_u, num_uv),
                );
                pc_block(
                    &self.mix_v,
                    &mut self.predictor_v,
                    frames / 32,
                    coefs(&mut self.coefs_v, num_uv),
                );
            }
            let bits_u = trial_bits(&self.predictor_u[..eighth]) * 8 + 16 * num_uv;
            if bits_u < min_bits_u {
                min_bits_u = bits_u;
                num_u = num_uv;
            }
            let bits_v = trial_bits(&self.predictor_v[..eighth]) * 8 + 16 * num_uv;
            if bits_v < min_bits_v {
                min_bits_v = bits_v;
                num_v = num_uv;
            }
        }

        // 8 header bytes, and the frame count of a partial packet.
        let min_bits = min_bits_u + min_bits_v + 8 * 8 + partial_bits;
        // 2 header bytes, and the frame count of a partial packet.
        let escape_bits = frames * BIT_DEPTH as usize * CHANNELS + partial_bits + 2 * 8;
        if min_bits >= escape_bits {
            write_escape(bits, interleaved, frames);
            return PacketKind::Escape;
        }

        bits.write(0, 12);
        bits.write(u32::from(partial) << 3, 4);
        if partial {
            bits.write(frames as u32, 32);
        }
        bits.write(MIX_BITS as u32, 8);
        bits.write(mix_res as u32, 8);
        write_coefs(bits, coefs(&mut self.coefs_u, num_u));
        write_coefs(bits, coefs(&mut self.coefs_v, num_v));
        pc_block(
            &self.mix_u,
            &mut self.predictor_u,
            frames,
            coefs(&mut self.coefs_u, num_u),
        );
        dyn_comp(&self.predictor_u[..frames], bits);
        pc_block(
            &self.mix_v,
            &mut self.predictor_v,
            frames,
            coefs(&mut self.coefs_v, num_v),
        );
        dyn_comp(&self.predictor_v[..frames], bits);

        if bits.bit_len() - start >= escape_bits {
            bits.truncate(start);
            write_escape(bits, interleaved, frames);
            return PacketKind::Rewound;
        }
        PacketKind::Compressed
    }
}

/// The coefficients at the start, as `init_coefs` gives them.
fn initial_coefs() -> [i16; MAX_UV] {
    const AINIT: i32 = 38;
    const BINIT: i32 = -29;
    const CINIT: i32 = -2;
    let den = 1 << DENSHIFT;
    let mut coefs = [0; MAX_UV];
    coefs[0] = ((AINIT * den) >> 4) as i16;
    coefs[1] = ((BINIT * den) >> 4) as i16;
    coefs[2] = ((CINIT * den) >> 4) as i16;
    coefs
}

/// The set of `num` coefficients, `num` 4 or 8.
fn coefs(sets: &mut CoefSets, num: usize) -> &mut [i16] {
    &mut sets[num / MIN_UV - 1][..num]
}

fn write_coefs(bits: &mut BitWriter, coefs: &[i16]) {
    bits.write((MODE << 4) | DENSHIFT, 8);
    bits.write((PB_FACTOR << 5) | coefs.len() as u32, 8);
    for &coef in coefs {
        bits.write(u32::from(coef as u16), 16);
    }
}

fn write_escape(bits: &mut BitWriter, interleaved: &[i16], frames: usize) {
    let partial = frames != FRAMES_PER_PACKET;
    bits.write(0, 12);
    // The low bit 1 means "not compressed".
    bits.write((u32::from(partial) << 3) | 1, 4);
    if partial {
        bits.write(frames as u32, 32);
    }
    for &sample in interleaved {
        bits.write(u32::from(sample as u16), 16);
    }
}

fn trial_bits(predicted: &[i32]) -> usize {
    let mut count = BitCount::default();
    dyn_comp(predicted, &mut count);
    count.bits()
}

/// Splits interleaved stereo into the two channels of the predictor.  A
/// `mix_res` of 0 keeps left and right.  Other values mix them, losslessly.
fn mix16(interleaved: &[i16], u: &mut [i32], v: &mut [i32], mix_res: i32) {
    let m2 = (1 << MIX_BITS) - mix_res;
    for (j, frame) in interleaved.chunks_exact(CHANNELS).enumerate() {
        let left = i32::from(frame[0]);
        let right = i32::from(frame[1]);
        if mix_res == 0 {
            u[j] = left;
            v[j] = right;
        } else {
            u[j] = (mix_res * left + m2 * right) >> MIX_BITS;
            v[j] = left - right;
        }
    }
}

/// The dynamic predictor: writes into `predicted` the error of an adaptive
/// prediction of `input`, for `num` samples, and adapts `coefs`.
///
/// Like the source, it always writes entries 0 to `coefs.len()`, even when
/// `num` is smaller.  The trial encodes then read the entries that an
/// earlier call left, so the buffers keep 4096 entries.
fn pc_block(input: &[i32], predicted: &mut [i32], num: usize, coefs: &mut [i16]) {
    let num_active = coefs.len();
    let chan_shift = 32 - CHAN_BITS;
    let den_half: i32 = 1 << (DENSHIFT - 1);

    predicted[0] = input[0];
    for j in 1..=num_active {
        let del = input[j] - input[j - 1];
        predicted[j] = (del << chan_shift) >> chan_shift;
    }

    let lim = num_active + 1;
    for j in lim..num {
        let top = input[j - lim];
        let mut sum = Wrapping(den_half);
        for (k, &coef) in coefs.iter().enumerate() {
            sum -= Wrapping(i32::from(coef)) * Wrapping(top - input[j - 1 - k]);
        }
        let mut del = ((input[j] - top - (sum.0 >> DENSHIFT)) << chan_shift) >> chan_shift;
        predicted[j] = del;

        // Adapt the coefficients from the last to the first, until the
        // error changes sign.
        let direction = match del.cmp(&0) {
            Ordering::Greater => 1,
            Ordering::Less => -1,
            Ordering::Equal => continue,
        };
        for k in (0..num_active).rev() {
            let diff = top - input[j - 1 - k];
            let sign = direction * diff.signum();
            coefs[k] -= sign as i16;
            del -= (num_active - k) as i32 * ((sign * diff) >> DENSHIFT);
            if del * direction <= 0 {
                break;
            }
        }
    }
}

/// The adaptive Golomb coder: writes the prediction errors `predicted`.
fn dyn_comp(predicted: &[i32], bits: &mut impl BitSink) {
    let num = predicted.len();
    let mut mb = MB0;
    let mut zmode = 0;
    let mut c = 0;
    while c < num {
        let k = (31 - ((mb >> QBSHIFT) + 3).leading_zeros()).min(KB0);
        let m = (1 << k) - 1;

        let del = predicted[c];
        c += 1;
        let n = ((del.abs() << 1) - ((del >> 31) & 1)) as u32 - zmode;
        dyn_code_32bit(bits, m, k, n);

        mb = PB0 * (n + zmode) + mb - ((PB0 * mb) >> QBSHIFT);
        if n > N_MAX_MEAN_CLAMP {
            mb = N_MEAN_CLAMP_VAL;
        }
        zmode = 0;

        if (mb << MMULSHIFT) < QB && c < num {
            zmode = 1;
            let mut zeros = 0;
            while c < num && predicted[c] == 0 {
                c += 1;
                zeros += 1;
                if zeros >= MAX_ZERO_RUN {
                    zmode = 0;
                    break;
                }
            }
            let k = mb.leading_zeros() - BITOFF + ((mb + MOFF) >> MDENSHIFT);
            let mz = ((1 << k) - 1) & WB0;
            dyn_code(bits, mz, k, zeros);
            mb = 0;
        }
    }
}

/// The Golomb code of `n` with the prefix and suffix that it fits.
fn golomb(m: u32, k: u32, n: u32) -> (u32, u32) {
    let division = n / m;
    let modulo = n % m;
    let de = u32::from(modulo == 0);
    let num_bits = division + k + 1 - de;
    let value = (((1 << division) - 1) << (num_bits - division)) + modulo + 1 - de;
    (value, num_bits)
}

/// Writes the length of a run of zeros.
fn dyn_code(bits: &mut impl BitSink, m: u32, k: u32, n: u32) {
    if n / m < MAX_PREFIX_16 {
        let (value, num_bits) = golomb(m, k, n);
        if num_bits <= MAX_PREFIX_16 + MAX_DATATYPE_BITS_16 {
            bits.write(value, num_bits);
            return;
        }
    }
    let value = (((1 << MAX_PREFIX_16) - 1) << MAX_DATATYPE_BITS_16) + n;
    bits.write(value, MAX_PREFIX_16 + MAX_DATATYPE_BITS_16);
}

/// Writes one prediction error.
fn dyn_code_32bit(bits: &mut impl BitSink, m: u32, k: u32, n: u32) {
    if n / m < MAX_PREFIX_32 {
        let (value, num_bits) = golomb(m, k, n);
        if num_bits <= 25 {
            bits.write(value, num_bits);
            return;
        }
    }
    bits.write((1 << MAX_PREFIX_32) - 1, MAX_PREFIX_32);
    bits.write(n, CHAN_BITS);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A packet whose first eighth is silent and whose rest is noise.  The
    /// trial encodes see only the silent part.
    fn quiet_then_noise(frames: usize) -> Vec<i16> {
        let mut state: u32 = 0x1234_5678;
        (0..frames * CHANNELS)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                if i < frames / 8 * CHANNELS {
                    0
                } else {
                    state as i16
                }
            })
            .collect()
    }

    #[test]
    fn a_compressed_packet_that_does_not_pay_is_rewound_to_an_escape_packet() {
        let mut encoder = AlacEncoder::new(44_100);
        let mut out = vec![9];
        let input = quiet_then_noise(FRAMES_PER_PACKET);
        let (size, kind) = encoder.encode_packet(&input, &mut out);
        assert_eq!(kind, PacketKind::Rewound);
        assert_eq!(size, MAX_PACKET_BYTES);
        assert_eq!(out.len(), 1 + MAX_PACKET_BYTES);
        // The tags, 12 zero bits, the escape flag (bit 22), then the first
        // sample, 0.
        assert_eq!(out[1..5], [0b0010_0000, 0, 0b0000_0010, 0]);
        // The last 2 bits of the end tag, then zero bits to the byte end.
        assert_eq!(out[MAX_PACKET_BYTES], 0b1100_0000);
    }

    #[test]
    fn the_cookie_holds_the_largest_packet_and_the_rate() {
        let mut encoder = AlacEncoder::new(48_000);
        let mut out = Vec::new();
        let big = encoder.encode(&quiet_then_noise(FRAMES_PER_PACKET), &mut out);
        let small = encoder.encode(&[0; 2 * 100], &mut out);
        assert!(small < big);
        let mut want = [0; 24];
        want[..12].copy_from_slice(&[0, 0, 0x10, 0, 0, 16, 40, 10, 14, 2, 0, 255]);
        want[12..16].copy_from_slice(&(big as u32).to_be_bytes());
        want[20..].copy_from_slice(&48_000_u32.to_be_bytes());
        assert_eq!(encoder.cookie(), want);
    }
}
