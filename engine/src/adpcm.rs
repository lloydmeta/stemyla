//! Checks and layouts of ADPCM for the decoders of `symphonia-codec-adpcm`
//! 0.6.1 (review round 10).

/// The adaptation of the step of Microsoft ADPCM for each nibble, in 256ths
/// (`symphonia-codec-adpcm` `codec_ms.rs`, `MS_ADAPTATION_TABLE`).
const MS_ADAPTATION: [i64; 16] = [
    230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
];

/// The least step of Microsoft ADPCM (`codec_ms.rs`, `DELTA_MIN`).
const MS_LEAST_STEP: i64 = 16;

/// The largest step that Symphonia can multiply by the largest adaptation
/// (768) in an `i32` (`codec_ms.rs`, line 96).
const MS_MOST_STEP: i64 = i32::MAX as i64 / 768;

/// Whether each step of each channel of each block of the Microsoft ADPCM
/// `packet` stays in the range that Symphonia multiplies in an `i32`
/// (findings 10-2 and 10-3 of review round 10).  The step triples for each
/// nibble of 8, from any start, so a crafted block passes `i32` in a few
/// nibbles: Symphonia then panics in a build with overflow checks.  A block
/// has a preamble of 7 bytes for each channel (the predictor of each
/// channel, then the step, the first and the second sample of each), then
/// `frames_per_block - 2` nibbles of each channel, the high nibble first,
/// one channel after the other.
pub(crate) fn ms_steps_fit(
    packet: &[u8],
    channels: usize,
    frames_per_block: u64,
    frames: u64,
) -> bool {
    let blocks = usize::try_from(frames / frames_per_block.max(1)).unwrap_or(0);
    if blocks == 0 || channels == 0 {
        return true;
    }
    let align = packet.len() / blocks;
    let nibbles = usize::try_from(frames_per_block.saturating_sub(2)).unwrap_or(0) * channels;
    packet.chunks(align).take(blocks).all(|block| {
        let mut steps: Vec<i64> = (0..channels)
            .map(|channel| {
                let at = channels + 2 * channel;
                block.get(at..at + 2).map_or(MS_LEAST_STEP, |step| {
                    i64::from(i16::from_le_bytes([step[0], step[1]]))
                })
            })
            .collect();
        (0..nibbles).all(|k| {
            let Some(byte) = block.get(7 * channels + k / 2) else {
                return true;
            };
            let nibble = if k % 2 == 0 { byte >> 4 } else { byte & 0x0F };
            let step = &mut steps[k % channels];
            if step.abs() > MS_MOST_STEP {
                return false;
            }
            *step = (MS_ADAPTATION[usize::from(nibble)] * *step / 256).max(MS_LEAST_STEP);
            true
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mono block of 10 frames from `step`, with nibbles of `byte`.
    fn block(step: i16, byte: u8) -> Vec<u8> {
        [&[0][..], &step.to_le_bytes(), &[0; 4], &[byte; 4]].concat()
    }

    /// Findings 10-2 and 10-3 of review round 10: the walk of the steps.
    #[test]
    fn the_steps_of_ms_adpcm_are_walked() {
        assert!(ms_steps_fit(&block(16, 0x00), 1, 10, 10), "small steps");
        // 16 x 3^8 = 104,976: in range.
        assert!(
            ms_steps_fit(&block(16, 0x88), 1, 10, 10),
            "8 nibbles of 8 from 16"
        );
        // 32,767 x 3^5 is past i32::MAX / 768.
        assert!(
            !ms_steps_fit(&block(32_767, 0x88), 1, 10, 10),
            "from 32,767"
        );
        let long = [&[0][..], &16_i16.to_le_bytes(), &[0; 4], &[0x88; 20]].concat();
        assert!(!ms_steps_fit(&long, 1, 42, 42), "40 nibbles of 8 from 16");
        // Two blocks: the second one passes the range.
        let two = [block(16, 0x00), block(32_767, 0x88)].concat();
        assert!(!ms_steps_fit(&two, 1, 10, 20), "the second block");
        assert!(ms_steps_fit(&[], 1, 10, 0), "no block");
    }
}
