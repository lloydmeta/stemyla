//! Decoding audio files into per-channel `f32` buffers with Symphonia.

use anyhow::{Context, Result, anyhow};
use std::io::Cursor;
use std::path::Path;
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::well_known::CODEC_ID_MP3;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

/// Decoded audio: one `Vec<f32>` per channel, all the same length.  The
/// decoders here leave no spare capacity (a song's channels are tens of MB,
/// and the web page's memory never shrinks).
#[derive(Clone, Debug)]
pub struct Audio {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

/// Decodes a file on disk (WAV or MP3).
pub fn decode_path(path: &Path) -> Result<Audio> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_owned);
    decode_source(Box::new(file), ext.as_deref())
        .with_context(|| format!("decoding {}", path.display()))
}

/// Decodes an in-memory file, for example one dropped into the web app.
/// `ext` (such as `"mp3"`) helps the format probe.
pub fn decode_bytes(bytes: Vec<u8>, ext: Option<&str>) -> Result<Audio> {
    decode_source(Box::new(Cursor::new(bytes)), ext)
}

/// How many frames to make room for before decoding: the length the file
/// states, but never more than the `most` its bytes could hold, so a
/// truncated file or a damaged header cannot reserve more than that.
fn frames_to_reserve(stated: Option<u64>, most: Option<u64>) -> usize {
    let (Some(stated), Some(most)) = (stated, most) else {
        return 0;
    };
    usize::try_from(stated.min(most)).unwrap_or(0)
}

/// The most frames to reserve for `byte_len` bytes of audio in `params`'
/// codec.
///
/// An MP3 is reserved for at most what it would hold at 128 kbit/s.  Most
/// music is at that bitrate or more, so is reserved in full; a file at a
/// lower one grows past its reservation an eighth at a time (see
/// `append`).  An overstated length (a crafted header, or a variable-
/// bitrate file with no length tag, whose length symphonia estimates from
/// its first frames, which a quiet start makes small) then reserves at most
/// 1.5 times a 192 kbit/s file, where the smallest legal packet would allow
/// six times it.  Every other codec here stores each sample in
/// `bits_per_coded_sample` bits, or in at least a byte (A-law, mu-law and
/// float WAVs do not state it).
fn most_frames(params: &AudioCodecParameters, byte_len: u64) -> u64 {
    if params.codec == CODEC_ID_MP3 {
        // An unknown or impossible rate counts as the highest MP3 has.
        let rate = params
            .sample_rate
            .filter(|r| (8_000..=48_000).contains(r))
            .map_or(48_000, u64::from);
        const BYTES_PER_SECOND: u64 = 128_000 / 8;
        return byte_len.saturating_mul(rate) / BYTES_PER_SECOND;
    }
    let bits = params.bits_per_coded_sample.filter(|b| *b > 0).unwrap_or(8);
    let channels = params.channels.as_ref().map_or(1, Channels::count).max(1);
    byte_len.saturating_mul(8) / (u64::from(bits) * channels as u64)
}

/// An empty channel with room for `frames`.  If that much memory is not
/// available the channel starts with none and grows as it is decoded, so it
/// fails only if the audio, while growing, does not fit.
fn channel_with_room_for(frames: usize) -> Vec<f32> {
    let mut channel = Vec::new();
    if channel.try_reserve_exact(frames).is_err() {
        return Vec::new();
    }
    channel
}

/// Appends `src` to `channel`.  Past its reservation (none, too big to
/// make, or capped by what the file's bytes could hold) the channel grows by
/// an eighth at a time rather than doubling, so a song-sized channel never
/// briefly holds three times itself; the decoder's `shrink_to_fit` returns
/// what is left spare.
fn append(channel: &mut Vec<f32>, src: &[f32]) {
    if src.len() > channel.capacity() - channel.len() {
        channel.reserve_exact(src.len().max(channel.len() / 8));
    }
    channel.extend_from_slice(src);
}

/// A file opened for decoding: its reader, the audio track and how many
/// frames to reserve for each channel.
struct Opened {
    format: Box<dyn FormatReader>,
    track_id: u32,
    params: AudioCodecParameters,
    reserve: usize,
}

fn open(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Opened> {
    let byte_len = source.byte_len();
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| anyhow!("unsupported or unrecognised format: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow!("no audio track"))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("track has no audio codec parameters"))?
        .clone();
    let track_id = track.id;
    let most = byte_len.map(|len| most_frames(&params, len));
    let reserve = frames_to_reserve(track.num_frames, most);
    Ok(Opened {
        format,
        track_id,
        params,
        reserve,
    })
}

fn decode_source(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Audio> {
    let mut audio = decode_into_reservation(source, ext)?;
    // The stated length can be wrong (an estimate from the bitrate, frames
    // skipped as corrupt), so release whatever the decoding left spare.
    for channel in &mut audio.channels {
        channel.shrink_to_fit();
    }
    Ok(audio)
}

/// Decodes into channels made with room for the file's stated length, and
/// grown by `append` past it.
fn decode_into_reservation(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Audio> {
    let Opened {
        mut format,
        track_id,
        params,
        reserve,
    } = open(source, ext)?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| anyhow!("no decoder for codec: {e}"))?;

    let mut sample_rate = params.sample_rate.unwrap_or(0);
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let mut planes: Vec<Vec<f32>> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow!("reading packet: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                if sample_rate == 0 {
                    sample_rate = buf.spec().rate();
                }
                buf.copy_to_vecs_planar::<f32>(&mut planes);
                if channels.is_empty() {
                    channels = planes
                        .iter()
                        .map(|_| channel_with_room_for(reserve))
                        .collect();
                }
                for (dst, src) in channels.iter_mut().zip(planes.iter()) {
                    append(dst, src);
                }
            }
            // A corrupt frame is skipped, as players do.
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(anyhow!("decoding packet: {e}")),
        }
    }
    if channels.is_empty() || channels[0].is_empty() {
        return Err(anyhow!("file decoded to no audio"));
    }
    Ok(Audio {
        sample_rate,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::encode_pcm16;

    /// 10,007 frames: not a whole number of 1,152-frame packets, nor near a
    /// power of two.
    const FRAMES: usize = 10_007;

    fn stereo_wav() -> (Vec<f32>, Vec<f32>, Vec<u8>) {
        let left: Vec<f32> = (0..FRAMES).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let right: Vec<f32> = (0..FRAMES).map(|i| (i as f32 * 0.03).cos() * 0.5).collect();
        let bytes = encode_pcm16(&left, &right, 44_100);
        (left, right, bytes)
    }

    fn set_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn assert_decodes_exactly(bytes: Vec<u8>, left: &[f32], right: &[f32]) {
        let audio = decode_bytes(bytes, Some("wav")).unwrap();
        assert_eq!(audio.channels.len(), 2);
        for (channel, want) in audio.channels.iter().zip([left, right]) {
            assert_eq!(channel.len(), want.len());
            assert_eq!(channel.capacity(), want.len());
            for (got, want) in channel.iter().zip(want) {
                assert!((got - want).abs() <= 0.5 / 32768.0 + 1e-7);
            }
        }
    }

    #[test]
    fn channels_of_an_awkward_length_have_no_spare_capacity() {
        let (left, right, bytes) = stereo_wav();
        assert_decodes_exactly(bytes, &left, &right);
    }

    #[test]
    fn channels_of_unstated_length_have_no_spare_capacity() {
        // A streaming WAV gives its lengths as u32::MAX: "unknown".
        let (left, right, mut bytes) = stereo_wav();
        set_u32(&mut bytes, 4, u32::MAX);
        set_u32(&mut bytes, 40, u32::MAX);
        assert_decodes_exactly(bytes, &left, &right);
    }

    #[test]
    fn a_truncated_file_decodes_what_is_there_with_no_spare_capacity() {
        let (left, right, mut bytes) = stereo_wav();
        let claimed = (FRAMES * 4 * 1000) as u32;
        set_u32(&mut bytes, 4, 36 + claimed);
        set_u32(&mut bytes, 40, claimed);
        assert_decodes_exactly(bytes, &left, &right);
    }

    fn reserve_for(bytes: Vec<u8>) -> usize {
        open(Box::new(Cursor::new(bytes)), Some("wav"))
            .unwrap()
            .reserve
    }

    #[test]
    fn a_wav_reserves_its_length() {
        let (_, _, bytes) = stereo_wav();
        assert_eq!(reserve_for(bytes), FRAMES);
    }

    #[test]
    fn a_truncated_wav_reserves_at_most_what_its_bytes_hold() {
        let (_, _, mut bytes) = stereo_wav();
        let claimed = (FRAMES * 4 * 1000) as u32;
        set_u32(&mut bytes, 4, 36 + claimed);
        set_u32(&mut bytes, 40, claimed);
        // The 44-byte header counts too: 11 frames' worth.
        assert_eq!(reserve_for(bytes), FRAMES + 11);
    }

    fn capacities_before_shrinking(bytes: Vec<u8>) -> Vec<usize> {
        let audio = decode_into_reservation(Box::new(Cursor::new(bytes)), Some("wav")).unwrap();
        audio.channels.iter().map(Vec::capacity).collect()
    }

    #[test]
    fn decoding_fills_the_reservation_without_growing() {
        let (_, _, bytes) = stereo_wav();
        assert_eq!(capacities_before_shrinking(bytes), [FRAMES, FRAMES]);
        let (_, _, mut truncated) = stereo_wav();
        let claimed = (FRAMES * 4 * 1000) as u32;
        set_u32(&mut truncated, 4, 36 + claimed);
        set_u32(&mut truncated, 40, claimed);
        assert_eq!(
            capacities_before_shrinking(truncated),
            [FRAMES + 11, FRAMES + 11]
        );
    }

    #[test]
    fn a_wav_of_unstated_length_reserves_nothing() {
        let (_, _, mut bytes) = stereo_wav();
        set_u32(&mut bytes, 4, u32::MAX);
        set_u32(&mut bytes, 40, u32::MAX);
        assert_eq!(reserve_for(bytes), 0);
    }

    fn mp3_at(rate: Option<u32>) -> AudioCodecParameters {
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_MP3)
            .with_channels(Channels::Discrete(2));
        if let Some(rate) = rate {
            params.with_sample_rate(rate);
        }
        params
    }

    #[test]
    fn an_mp3_is_reserved_for_at_most_its_length_at_128_kbit_s() {
        // 16,000 bytes is a second at 128 kbit/s.
        assert_eq!(most_frames(&mp3_at(Some(44_100)), 16_000), 44_100);
        assert_eq!(most_frames(&mp3_at(Some(24_000)), 32_000), 48_000);
        // An unknown or impossible rate counts as 48 kHz.
        for rate in [None, Some(0), Some(96_000)] {
            assert_eq!(most_frames(&mp3_at(rate), 16_000), 48_000, "{rate:?}");
        }
        // A 192 kbit/s file of 24,000 bytes holds a second; its reservation
        // is capped at 1.5 times that, however long its header says it is.
        let a_second_at_192k = most_frames(&mp3_at(Some(44_100)), 24_000);
        assert_eq!(a_second_at_192k, 66_150);
        assert_eq!(
            frames_to_reserve(Some(10 * 44_100), Some(a_second_at_192k)),
            66_150
        );
    }

    #[test]
    fn other_codecs_take_at_least_a_byte_a_sample() {
        let mut params = AudioCodecParameters::new();
        params.with_channels(Channels::Discrete(2));
        // A float WAV, which states no bits per sample.
        assert_eq!(most_frames(&params, 4000), 2000);
        params.with_bits_per_coded_sample(16);
        assert_eq!(most_frames(&params, 4000), 1000);
        params.with_bits_per_coded_sample(0);
        assert_eq!(most_frames(&params, 4000), 2000);
    }

    #[test]
    fn reserves_the_stated_length_up_to_the_most_the_file_holds() {
        assert_eq!(frames_to_reserve(Some(1000), Some(1011)), 1000);
        assert_eq!(frames_to_reserve(Some(u64::MAX), Some(1011)), 1011);
        assert_eq!(frames_to_reserve(None, Some(1011)), 0);
        assert_eq!(frames_to_reserve(Some(1000), None), 0);
    }

    #[test]
    fn a_full_channel_grows_by_an_eighth_not_double() {
        let mut channel = vec![0.5; 8000];
        append(&mut channel, &[0.25; 10]);
        assert_eq!(channel.capacity(), 9000);
        assert_eq!(channel.len(), 8010);
        assert_eq!(channel[7999..8001], [0.5, 0.25]);
        // Within the room it has, it does not grow.
        append(&mut channel, &[0.25; 990]);
        assert_eq!(channel.capacity(), 9000);
        // A packet bigger than an eighth gets exactly its own room.
        let mut small = vec![0.5; 8];
        append(&mut small, &[0.25; 1152]);
        assert_eq!(small.capacity(), 1160);
    }

    #[test]
    fn a_reservation_too_big_to_make_starts_empty() {
        let channel = channel_with_room_for(usize::MAX);
        assert_eq!(channel.capacity(), 0);
    }
}
