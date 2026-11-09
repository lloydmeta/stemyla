//! Checks and layouts of ADPCM for the decoders of `symphonia-codec-adpcm`
//! 0.6.1 (review round 10).

use symphonia::core::audio::layouts::CHANNEL_LAYOUT_MONO;
use symphonia::core::audio::{
    AsGenericAudioBufferRef, AudioBuffer, AudioMut, AudioSpec, Channels, GenericAudioBufferRef,
};
use symphonia::core::codecs::CodecInfo;
use symphonia::core::codecs::audio::well_known::{CODEC_ID_ADPCM_IMA_QT, CODEC_ID_ADPCM_IMA_WAV};
use symphonia::core::codecs::audio::{
    AudioCodecId, AudioCodecParameters, AudioDecoder, AudioDecoderOptions, FinalizeResult,
};
use symphonia::core::errors::Result;
use symphonia::core::packet::PacketRef;

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
            let nibble = if k.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 0x0F
            };
            let step = &mut steps[k % channels];
            if step.abs() > MS_MOST_STEP {
                return false;
            }
            *step = (MS_ADAPTATION[usize::from(nibble)] * *step / 256).max(MS_LEAST_STEP);
            true
        })
    })
}

/// A decoder of ADPCM of more than 2 channels, which Symphonia's ADPCM
/// decoder refuses (`symphonia-codec-adpcm` `lib.rs`, line 99; findings 9-7
/// and 10-4 of review rounds 9 and 10).  The samples of each channel of a
/// block depend on that channel only, so it makes one mono block of each
/// channel of each block ([`mono_block`]), gives them to its own mono
/// decoder of Symphonia, and puts the channels side by side.
pub(crate) struct AdpcmChannels {
    params: AudioCodecParameters,
    channels: Vec<Box<dyn AudioDecoder>>,
    /// The mono blocks of one channel of a packet.
    blocks: Vec<u8>,
    /// The samples of one channel.
    plane: Vec<Vec<i32>>,
    buf: AudioBuffer<i32>,
}

impl AdpcmChannels {
    pub(crate) fn new(params: &AudioCodecParameters) -> Result<Self> {
        let count = params.channels.as_ref().map_or(0, Channels::count);
        let rate = params.sample_rate.unwrap_or(0);
        let mut mono = params.clone();
        mono.with_channels(CHANNEL_LAYOUT_MONO);
        let channels = (0..count)
            .map(|_| {
                symphonia::default::get_codecs()
                    .make_audio_decoder(&mono, &AudioDecoderOptions::default())
            })
            .collect::<Result<Vec<_>>>()?;
        let spec = AudioSpec::new(rate, Channels::Discrete(count as u16));
        let frames = params.max_frames_per_packet.unwrap_or(0) as usize;
        Ok(AdpcmChannels {
            params: params.clone(),
            channels,
            blocks: Vec::new(),
            plane: Vec::new(),
            buf: AudioBuffer::new(spec, frames),
        })
    }
}

/// Puts in `out` the mono block of `channel` of `block`, a block of
/// `channels` channels of `codec` of `per_block` frames, in the layout that
/// the mono decoder of Symphonia reads (`codec_ima_qt.rs`,
/// `codec_ima_wav.rs` and `codec_ms.rs`).  A short block gives zeros.
/// * `ima4`: the blocks of 34 bytes of each channel take turns.
/// * IMA ADPCM in a WAV: a preamble of 4 bytes of each channel, then groups
///   of 4 bytes (8 nibbles) of each channel in turn.
/// * Microsoft ADPCM: the predictor of each channel, then the step, the first
///   and the second sample of each, then the nibbles of each frame, one
///   channel after the other, two in a byte, the high one first.
fn mono_block(
    codec: AudioCodecId,
    block: &[u8],
    channels: usize,
    channel: usize,
    per_block: usize,
    out: &mut Vec<u8>,
) {
    let bytes =
        |at: usize, count: usize| block.get(at..at + count).map_or(vec![0; count], Vec::from);
    if codec == CODEC_ID_ADPCM_IMA_QT {
        out.extend(bytes(34 * channel, 34));
    } else if codec == CODEC_ID_ADPCM_IMA_WAV {
        out.extend(bytes(4 * channel, 4));
        for group in 0..per_block.saturating_sub(1) / 8 {
            out.extend(bytes(4 * channels + 4 * (group * channels + channel), 4));
        }
    } else {
        out.extend(bytes(channel, 1));
        for field in 0..3 {
            out.extend(bytes(channels + 2 * (field * channels + channel), 2));
        }
        let nibble = |k: usize| {
            let byte = block.get(7 * channels + k / 2).copied().unwrap_or(0);
            if k.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 0x0F
            }
        };
        let frames = per_block.saturating_sub(2);
        for pair in 0..frames.div_ceil(2) {
            let high = nibble(2 * pair * channels + channel);
            let low = if 2 * pair + 1 < frames {
                nibble((2 * pair + 1) * channels + channel)
            } else {
                0
            };
            out.push((high << 4) | low);
        }
    }
}

impl AudioDecoder for AdpcmChannels {
    fn reset(&mut self) {
        self.channels.iter_mut().for_each(|channel| channel.reset());
    }

    fn codec_info(&self) -> &CodecInfo {
        self.channels[0].codec_info()
    }

    fn codec_params(&self) -> &AudioCodecParameters {
        &self.params
    }

    fn decode_ref(&mut self, packet: &PacketRef<'_>) -> Result<GenericAudioBufferRef<'_>> {
        let count = self.channels.len();
        let per_block = self.params.frames_per_block.unwrap_or(1).max(1) as usize;
        let blocks = packet.block_dur().get() as usize / per_block;
        let align = packet.data.len().checked_div(blocks).unwrap_or(0).max(1);
        self.buf.clear();
        self.buf.render_uninit(Some(blocks * per_block));
        for (index, channel) in self.channels.iter_mut().enumerate() {
            self.blocks.clear();
            for block in packet.data.chunks(align).take(blocks) {
                mono_block(
                    self.params.codec,
                    block,
                    count,
                    index,
                    per_block,
                    &mut self.blocks,
                );
            }
            let mono = PacketRef::new(0, packet.pts, packet.dur, &self.blocks);
            channel
                .decode_ref(&mono)?
                .copy_to_vecs_planar::<i32>(&mut self.plane);
            if let (Some(plane), Some(samples)) = (self.buf.plane_mut(index), self.plane.first()) {
                let len = plane.len().min(samples.len());
                plane[..len].copy_from_slice(&samples[..len]);
            }
        }
        Ok(self.buf.as_generic_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> GenericAudioBufferRef<'_> {
        self.buf.as_generic_audio_buffer_ref()
    }
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
