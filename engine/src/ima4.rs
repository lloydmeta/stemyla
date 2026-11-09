//! An AIFC of Apple's IMA ADPCM (`ima4`), for the decode (finding 8-5 of
//! review round 8).  Symphonia 0.6.1 decodes `ima4` (`symphonia-codec-adpcm`,
//! `CODEC_ID_ADPCM_IMA_QT`), but its AIFF reader gives no such codec: it
//! refuses each compression type that is not PCM (`symphonia-format-riff`
//! `aiff/chunks.rs`, line 287).  So [`Ima4Reader`] reads the blocks of the
//! SSND chunk itself.  Each block of each channel is 34 bytes of 64 frames,
//! and the blocks of the channels take turns (Apple QuickTime IMA ADPCM).

use std::io;

use symphonia::core::audio::layouts::{CHANNEL_LAYOUT_MONO, CHANNEL_LAYOUT_STEREO};
use symphonia::core::audio::{
    AsGenericAudioBufferRef, AudioBuffer, AudioMut, AudioSpec, Channels, GenericAudioBufferRef,
};
use symphonia::core::codecs::CodecInfo;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioCodecParameters;
use symphonia::core::codecs::audio::well_known::CODEC_ID_ADPCM_IMA_QT;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions, FinalizeResult};
use symphonia::core::errors::{Error, Result, SeekErrorKind};
use symphonia::core::formats::well_known::FORMAT_ID_AIFF;
use symphonia::core::formats::{
    FormatInfo, FormatReader, MediaInfo, SeekMode, SeekTo, SeekedTo, Track,
};
use symphonia::core::io::{MediaSourceStream, ReadBytes};
use symphonia::core::meta::{ChapterGroup, Metadata, MetadataLog};
use symphonia::core::packet::Packet;
use symphonia::core::packet::PacketRef;
use symphonia::core::units::{Duration, Timestamp};

use crate::mp4_boxes::ReadAt;

/// The bytes of one block of one channel.
const BLOCK_BYTES: u64 = 34;

/// The frames of one block.
const BLOCK_FRAMES: u64 = 64;

/// The blocks of each channel in one packet.
const PACKET_BLOCKS: u64 = 64;

const FORMAT_INFO: FormatInfo = FormatInfo {
    format: FORMAT_ID_AIFF,
    short_name: "aiff",
    long_name: "Audio Interchange File Format, IMA ADPCM",
};

/// What the decode needs of an `ima4` AIFC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ima4 {
    pub channels: u16,
    pub rate: u32,
    /// Where the blocks start in the file.
    pub data: u64,
    /// The bytes of whole blocks for all channels.
    pub data_bytes: u64,
}

/// The `ima4` layout of the AIFC whose `FORM` starts at `at`, by its `COMM`
/// and `SSND` chunks, in either order (finding 9-7 of review round 9).
/// `None` for an AIFF, another type, a rate of 0, no channel, or a file with
/// no such chunks.
pub(crate) fn ima4_of(source: &mut dyn ReadAt, at: u64) -> io::Result<Option<Ima4>> {
    let mut form = [0; 4];
    if source.read_at(at + 8, &mut form)? < 4 || form != *b"AIFC" {
        return Ok(None);
    }
    let mut comm = None;
    // The start of the blocks, and the bytes that `SSND` states for them.
    let mut ssnd = None;
    let mut chunk = at + 12;
    while comm.is_none() || ssnd.is_none() {
        let mut header = [0; 8];
        if source.read_at(chunk, &mut header)? < 8 {
            return Ok(None);
        }
        let size = u64::from(u32::from_be_bytes([
            header[4], header[5], header[6], header[7],
        ]));
        match &header[..4] {
            b"COMM" => {
                // The channels, the frames, the sample size, the rate of 10
                // bytes, then the compression type.
                let mut body = [0; 22];
                if source.read_at(chunk + 8, &mut body)? < 22 {
                    return Ok(None);
                }
                if !body[18..22].eq_ignore_ascii_case(b"ima4") {
                    return Ok(None);
                }
                let channels = u16::from_be_bytes([body[0], body[1]]);
                let rate = rate_of(body[8..18].try_into().unwrap_or([0; 10]));
                let Some(rate) = rate.filter(|rate| *rate > 0 && channels > 0) else {
                    return Ok(None);
                };
                comm = Some((channels, rate));
            }
            b"SSND" => {
                let mut offset = [0; 4];
                if source.read_at(chunk + 8, &mut offset)? < 4 {
                    return Ok(None);
                }
                let offset = u64::from(u32::from_be_bytes(offset));
                ssnd = Some((chunk + 16 + offset, size.saturating_sub(8 + offset)));
            }
            _ => {}
        }
        chunk += 8 + size + (size & 1);
    }
    let (Some((channels, rate)), Some((data, held))) = (comm, ssnd) else {
        return Ok(None);
    };
    let frame_bytes = BLOCK_BYTES * u64::from(channels);
    Ok(Some(Ima4 {
        channels,
        rate,
        data,
        data_bytes: held - held % frame_bytes,
    }))
}

/// The rate of an 80-bit IEEE extended number (the `COMM` rate of AIFF),
/// in whole Hz.  `None` for a negative or too large rate.
fn rate_of(bytes: [u8; 10]) -> Option<u32> {
    let exponent = i32::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    let mantissa = u64::from_be_bytes(bytes[2..10].try_into().ok()?);
    if exponent & 0x8000 != 0 {
        return None;
    }
    // The value is mantissa * 2^(exponent - 16383 - 63).
    let shift = 16383 + 63 - exponent;
    let rate = match shift {
        0..=63 => mantissa >> shift,
        64.. => 0,
        _ => return None,
    };
    u32::try_from(rate).ok()
}

/// The reader of an `ima4` AIFC: packets of [`PACKET_BLOCKS`] blocks of each
/// channel, in order.  It cannot seek.
pub(crate) struct Ima4Reader {
    mss: MediaSourceStream<'static>,
    tracks: Vec<Track>,
    media_info: MediaInfo,
    metadata: MetadataLog,
    frame_bytes: u64,
    left: u64,
    next: Timestamp,
}

impl Ima4Reader {
    /// The reader of `layout`, with `mss` at `at` in the file.  It skips to
    /// the blocks.
    pub(crate) fn new(mut mss: MediaSourceStream<'static>, layout: Ima4, at: u64) -> Result<Self> {
        mss.ignore_bytes(layout.data.saturating_sub(at))?;
        let frame_bytes = BLOCK_BYTES * u64::from(layout.channels);
        let blocks = layout.data_bytes / frame_bytes;
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_ADPCM_IMA_QT)
            .with_sample_rate(layout.rate)
            .with_channels(match layout.channels {
                1 => CHANNEL_LAYOUT_MONO,
                2 => CHANNEL_LAYOUT_STEREO,
                count => Channels::Discrete(count),
            })
            .with_frames_per_block(BLOCK_FRAMES)
            .with_max_frames_per_packet(PACKET_BLOCKS * BLOCK_FRAMES);
        let mut track = Track::new(0);
        track
            .with_codec_params(CodecParameters::Audio(params))
            .with_num_frames(blocks * BLOCK_FRAMES);
        Ok(Ima4Reader {
            mss,
            media_info: MediaInfo::from_track(&track),
            tracks: vec![track],
            metadata: MetadataLog::default(),
            frame_bytes,
            left: layout.data_bytes,
            next: Timestamp::new(0),
        })
    }
}

impl FormatReader for Ima4Reader {
    fn format_info(&self) -> &FormatInfo {
        &FORMAT_INFO
    }

    fn media_info(&self) -> &MediaInfo {
        &self.media_info
    }

    fn next_packet(&mut self) -> Result<Option<Packet>> {
        let bytes = self.left.min(PACKET_BLOCKS * self.frame_bytes);
        if bytes == 0 {
            return Ok(None);
        }
        let data = self.mss.read_boxed_slice_exact(bytes as usize)?;
        self.left -= bytes;
        let frames = bytes / self.frame_bytes * BLOCK_FRAMES;
        let pts = self.next;
        self.next = Timestamp::new(pts.get() + frames as i64);
        Ok(Some(Packet::new(0, pts, Duration::new(frames), data)))
    }

    fn metadata(&mut self) -> Metadata<'_> {
        self.metadata.metadata()
    }

    fn chapters(&self) -> Option<&ChapterGroup> {
        None
    }

    fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    fn seek(&mut self, _: SeekMode, _: SeekTo) -> Result<SeekedTo> {
        Err(Error::SeekError(SeekErrorKind::Unseekable))
    }

    fn into_inner<'s>(self: Box<Self>) -> MediaSourceStream<'s>
    where
        Self: 's,
    {
        self.mss
    }
}

/// A decoder of `ima4` of more than 2 channels, which Symphonia's ADPCM
/// decoder refuses (finding 9-7 of review round 9).  The blocks of each
/// channel stand alone, so it gives the blocks of each channel to its own
/// mono decoder of Symphonia, and puts the channels side by side.
pub(crate) struct Ima4Channels {
    params: AudioCodecParameters,
    channels: Vec<Box<dyn AudioDecoder>>,
    /// The blocks of one channel of a packet.
    blocks: Vec<u8>,
    /// The samples of one channel.
    plane: Vec<Vec<i32>>,
    buf: AudioBuffer<i32>,
}

impl Ima4Channels {
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
        Ok(Ima4Channels {
            params: params.clone(),
            channels,
            blocks: Vec::new(),
            plane: Vec::new(),
            buf: AudioBuffer::new(spec, (PACKET_BLOCKS * BLOCK_FRAMES) as usize),
        })
    }
}

impl AudioDecoder for Ima4Channels {
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
        let block = BLOCK_BYTES as usize;
        let frames = packet.data.len() / (block * count) * BLOCK_FRAMES as usize;
        self.buf.clear();
        self.buf.render_uninit(Some(frames));
        for (index, channel) in self.channels.iter_mut().enumerate() {
            self.blocks.clear();
            for frame in packet.data.chunks_exact(block * count) {
                self.blocks
                    .extend_from_slice(&frame[index * block..(index + 1) * block]);
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

    /// Finding 8-5 of review round 8: the rate of the COMM chunk.
    #[test]
    fn the_rate_of_an_extended_number_reads() {
        use crate::test_audio::extended;
        for rate in [8_000, 22_050, 44_100, 48_000, 96_000, 192_000] {
            assert_eq!(rate_of(extended(rate)), Some(rate), "{rate}");
        }
        assert_eq!(rate_of([0; 10]), Some(0));
        assert_eq!(rate_of([0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None, "negative");
        assert_eq!(
            rate_of([0x7F, 0xFF, 0x80, 0, 0, 0, 0, 0, 0, 0]),
            None,
            "too large"
        );
    }

    /// Finding 9-5 of review round 9: a packet holds at most
    /// `PACKET_BLOCKS` blocks of each channel.
    #[test]
    fn a_packet_holds_at_most_64_blocks_of_each_channel() {
        use crate::test_audio::ima4_aifc;
        let tone: Vec<f32> = (0..64 * 200)
            .map(|i| (i as f32 * 0.01).sin() * 0.5)
            .collect();
        for count in [1, 2, 3] {
            let (file, _) = ima4_aifc(&vec![tone.clone(); count], 44_100);
            let layout = ima4_of(&mut file.as_slice(), 0).ok().flatten();
            assert!(layout.is_some(), "{count} channels");
            let layout = layout.unwrap();
            let mss =
                MediaSourceStream::new(Box::new(std::io::Cursor::new(file)), Default::default());
            let mut reader = Ima4Reader::new(mss, layout, 0).expect("the reader");
            let most = (PACKET_BLOCKS * BLOCK_BYTES) as usize * count;
            let mut blocks = 0;
            while let Ok(Some(packet)) = reader.next_packet() {
                assert!(
                    packet.data.len() <= most,
                    "{count} channels: {}",
                    packet.data.len()
                );
                blocks += packet.data.len() / (34 * count);
            }
            assert_eq!(blocks, 200, "{count} channels");
        }
    }
}
