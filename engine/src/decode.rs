//! Decoding audio files into per-channel `f32` buffers with Symphonia.

use anyhow::{Context, Result, anyhow};
use std::io::Cursor;
use std::path::Path;
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::well_known::{CODEC_ID_FLAC, CODEC_ID_MP3};
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

/// The most decoded samples one file may hold, as `f32` bytes over all its
/// channels (decision 0013): 50.7 minutes of 44.1 kHz stereo.  A FLAC of
/// about 80 kB can hold 2 hours of silence.
pub const MAX_DECODED_BYTES: u64 = 1 << 30;

/// How many times smaller than PCM at its stated bits a FLAC is trusted to
/// be, for its reservation.  The demo song's stems are 2.2 to 11.3 times
/// smaller at 16 bits.
const FLAC_MOST_RATIO: u64 = 16;

/// Decodes a file on disk (WAV, MP3 or FLAC).
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
/// six times it.  A FLAC is reserved for at most `FLAC_MOST_RATIO` times
/// what its bytes hold as PCM at its stated bits: a constant subframe holds
/// a whole block in one sample, so its bytes give no real bound.  Every
/// other codec here stores each sample in `bits_per_coded_sample` bits, or
/// in at least a byte (A-law, mu-law and float WAVs do not state it).
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
    let channels = params.channels.as_ref().map_or(1, Channels::count).max(1) as u64;
    if params.codec == CODEC_ID_FLAC {
        // FLAC has 4 to 32 bits; an unknown depth counts as the fewest.
        let bits = params
            .bits_per_sample
            .filter(|b| (4..=32).contains(b))
            .map_or(4, u64::from);
        return byte_len.saturating_mul(8 * FLAC_MOST_RATIO) / (bits * channels);
    }
    let bits = params.bits_per_coded_sample.filter(|b| *b > 0).unwrap_or(8);
    byte_len.saturating_mul(8) / (u64::from(bits) * channels)
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

/// Appends `src` to `channel`, which may hold at most `most` frames.  Past
/// its reservation (none, too big to make, or capped by what the file's
/// bytes could hold) the channel grows by an eighth at a time rather than
/// doubling, so a song-sized channel never briefly holds three times
/// itself, and never past `most`; the decoder's `shrink_to_fit` returns
/// what is left spare.
fn append(channel: &mut Vec<f32>, src: &[f32], most: usize) {
    if src.len() > channel.capacity() - channel.len() {
        let room = most.saturating_sub(channel.len()).max(src.len());
        channel.reserve_exact(src.len().max(channel.len() / 8).min(room));
    }
    channel.extend_from_slice(src);
}

/// The error for a file whose samples pass `limit` bytes.
fn too_long(limit: u64) -> anyhow::Error {
    anyhow!(
        "the song is too long to open: it decodes to more than {} MiB of samples",
        limit >> 20
    )
}

/// Whether `frames` of each of `channels` channels pass `limit` bytes of
/// `f32`.
fn passes(limit: u64, channels: usize, frames: u64) -> bool {
    frames.saturating_mul(4 * channels as u64) > limit
}

/// A file opened for decoding: its reader, the audio track and how many
/// frames to reserve for each channel.
struct Opened {
    format: Box<dyn FormatReader>,
    track_id: u32,
    params: AudioCodecParameters,
    reserve: usize,
}

/// The frames of each of `channels` channels that fit in `limit` bytes of
/// `f32`.
fn frames_within(limit: u64, channels: usize) -> usize {
    usize::try_from(limit / (4 * channels.max(1) as u64)).unwrap_or(usize::MAX)
}

/// Opens `source`.  It refuses a WAV or FLAC whose stated length passes
/// `limit` bytes of samples.  An MP3 states only an estimate without a
/// Xing or VBRI tag: a quiet start can make it 10 times too long.  So an
/// MP3 is never refused here; the decoder stops it at the limit.  The
/// reservation of any file is within `limit`.
fn open(source: Box<dyn MediaSource>, ext: Option<&str>, limit: u64) -> Result<Opened> {
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
    let channels = params.channels.as_ref().map_or(1, Channels::count).max(1);
    let stated_exactly = params.codec != CODEC_ID_MP3;
    if stated_exactly
        && track
            .num_frames
            .is_some_and(|frames| passes(limit, channels, frames))
    {
        return Err(too_long(limit));
    }
    let most = byte_len.map(|len| most_frames(&params, len));
    let within_limit = frames_within(limit, channels) as u64;
    let reserve = frames_to_reserve(track.num_frames, most.map(|m| m.min(within_limit)));
    Ok(Opened {
        format,
        track_id,
        params,
        reserve,
    })
}

fn decode_source(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Audio> {
    decode_source_within(source, ext, MAX_DECODED_BYTES)
}

fn decode_source_within(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    limit: u64,
) -> Result<Audio> {
    let mut audio = decode_into_reservation(source, ext, limit)?;
    // The stated length can be wrong (an estimate from the bitrate, frames
    // skipped as corrupt), so release whatever the decoding left spare.
    for channel in &mut audio.channels {
        channel.shrink_to_fit();
    }
    Ok(audio)
}

/// Decodes into channels made with room for the file's stated length, and
/// grown by `append` past it.  It stops with an error before the channels
/// pass `limit` bytes.
fn decode_into_reservation(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    limit: u64,
) -> Result<Audio> {
    let Opened {
        mut format,
        track_id,
        params,
        reserve,
    } = open(source, ext, limit)?;
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
                let decoded = channels.first().map_or(0, Vec::len);
                let packet_frames = planes.first().map_or(0, Vec::len);
                if passes(limit, planes.len(), (decoded + packet_frames) as u64) {
                    return Err(too_long(limit));
                }
                let most = frames_within(limit, planes.len());
                if channels.is_empty() {
                    channels = planes
                        .iter()
                        .map(|_| channel_with_room_for(reserve.min(most)))
                        .collect();
                }
                for (dst, src) in channels.iter_mut().zip(planes.iter()) {
                    append(dst, src, most);
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
    use crate::test_flac::{Blocks, by_hand, compressed, step};
    use crate::wav::encode_pcm16;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        /// Heap bytes this thread allocated and has not freed.
        static LIVE: Cell<usize> = const { Cell::new(0) };
        /// The most `LIVE` has been since a test set it.
        static PEAK: Cell<usize> = const { Cell::new(0) };
    }

    /// Keeps `LIVE` and `PEAK` for each thread.  Only `alloc` and `dealloc`
    /// are written, so the default `realloc` holds the old and the new
    /// buffer at once, as `System`'s may.
    struct PeakAllocator;

    // SAFETY: every call is forwarded unchanged to `System`; counting
    // touches only `const` thread-locals, which do not allocate.
    unsafe impl GlobalAlloc for PeakAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = LIVE.try_with(|live| {
                let now = live.get() + layout.size();
                live.set(now);
                let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
            });
            // SAFETY: the caller's contract is the same as `System`'s.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(layout.size())));
            // SAFETY: the caller's contract is the same as `System`'s.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: PeakAllocator = PeakAllocator;

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
        assert_decodes_as(bytes, Some("wav"), left, right, 16);
    }

    /// `bytes` decode to `left` and `right` within half a step of `bits`
    /// bits, with no spare capacity.
    fn assert_decodes_as(
        bytes: Vec<u8>,
        ext: Option<&str>,
        left: &[f32],
        right: &[f32],
        bits: u32,
    ) {
        let half_step = 0.5 / (1_u32 << (bits - 1)) as f32;
        let audio = decode_bytes(bytes, ext).unwrap();
        assert_eq!(audio.channels.len(), 2);
        for (channel, want) in audio.channels.iter().zip([left, right]) {
            assert_eq!(channel.len(), want.len());
            assert_eq!(channel.capacity(), want.len());
            for (got, want) in channel.iter().zip(want) {
                assert!((got - want).abs() <= half_step + 1e-7, "{got} for {want}");
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
        reserve_of(bytes, "wav")
    }

    fn reserve_of(bytes: Vec<u8>, ext: &str) -> usize {
        open(Box::new(Cursor::new(bytes)), Some(ext), MAX_DECODED_BYTES)
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
        let audio =
            decode_into_reservation(Box::new(Cursor::new(bytes)), Some("wav"), MAX_DECODED_BYTES)
                .unwrap();
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
        append(&mut channel, &[0.25; 10], usize::MAX);
        assert_eq!(channel.capacity(), 9000);
        assert_eq!(channel.len(), 8010);
        assert_eq!(channel[7999..8001], [0.5, 0.25]);
        // Within the room it has, it does not grow.
        append(&mut channel, &[0.25; 990], usize::MAX);
        assert_eq!(channel.capacity(), 9000);
        // A packet bigger than an eighth gets exactly its own room.
        let mut small = vec![0.5; 8];
        append(&mut small, &[0.25; 1152], usize::MAX);
        assert_eq!(small.capacity(), 1160);
    }

    #[test]
    fn a_reservation_too_big_to_make_starts_empty() {
        let channel = channel_with_room_for(usize::MAX);
        assert_eq!(channel.capacity(), 0);
    }

    #[test]
    fn a_16_bit_flac_decodes_exactly() {
        let (left, right, _) = stereo_wav();
        let bytes = compressed(&left, &right, 44_100);
        assert_decodes_as(bytes, Some("flac"), &left, &right, 16);
    }

    /// 24-bit stereo samples, with full scale at both ends, and the same
    /// as `f32`.
    fn samples_24() -> (Vec<Vec<i32>>, Vec<f32>, Vec<f32>) {
        let (mut left, right, _) = stereo_wav();
        (left[0], left[1]) = (1.0, -1.0);
        let channels: Vec<Vec<i32>> = [&left, &right]
            .iter()
            .map(|c| c.iter().map(|x| step(*x, 24)).collect())
            .collect();
        let as_f32 =
            |c: &[i32]| -> Vec<f32> { c.iter().map(|s| *s as f32 / 8_388_608.0).collect() };
        let (l, r) = (as_f32(&channels[0]), as_f32(&channels[1]));
        (channels, l, r)
    }

    #[test]
    fn a_24_bit_flac_decodes_exactly() {
        let (channels, left, right) = samples_24();
        assert_eq!(channels[0][..2], [8_388_607, -8_388_608]);
        let bytes = by_hand(96_000, 24, 2, FRAMES as u64, Blocks::Verbatim(&channels));
        assert_decodes_as(bytes, Some("flac"), &left, &right, 24);
    }

    #[test]
    fn a_flac_with_no_extension_is_probed() {
        let (left, right, _) = stereo_wav();
        assert_decodes_as(compressed(&left, &right, 44_100), None, &left, &right, 16);
    }

    #[test]
    fn a_truncated_flac_decodes_what_is_there() {
        let (left, right, _) = stereo_wav();
        let mut bytes = compressed(&left, &right, 44_100);
        bytes.truncate(bytes.len() * 3 / 5);
        let audio = decode_bytes(bytes, Some("flac")).unwrap();
        let frames = audio.frames();
        assert!(0 < frames && frames < FRAMES, "{frames} frames");
        for (channel, want) in audio.channels.iter().zip([&left, &right]) {
            assert_eq!(channel.capacity(), frames);
            for (got, want) in channel.iter().zip(want.iter()) {
                assert!((got - want).abs() <= 0.5 / 32768.0 + 1e-7);
            }
        }
    }

    #[test]
    fn a_flac_reserves_its_length() {
        let (channels, _, _) = samples_24();
        let bytes = by_hand(44_100, 24, 2, FRAMES as u64, Blocks::Verbatim(&channels));
        assert_eq!(reserve_of(bytes, "flac"), FRAMES);
    }

    /// Stereo silence in `blocks` FLAC frames of `block` frames, stating
    /// `stated` frames.
    fn silence(stated: u64, block: usize, blocks: usize) -> Vec<u8> {
        by_hand(44_100, 16, 2, stated, Blocks::Silence { block, blocks })
    }

    #[test]
    fn a_flac_that_states_40_minutes_reserves_at_most_16_times_its_bytes() {
        let bytes = silence(40 * 60 * 44_100, 4096, 3);
        let byte_len = bytes.len();
        // 16 times what 16-bit stereo PCM holds in those bytes.
        assert_eq!(reserve_of(bytes, "flac"), byte_len * 8 * 16 / (16 * 2));
    }

    fn flac_at(bits: Option<u32>) -> AudioCodecParameters {
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_FLAC)
            .with_channels(Channels::Discrete(2));
        if let Some(bits) = bits {
            params.with_bits_per_sample(bits);
        }
        params
    }

    #[test]
    fn a_flac_is_reserved_for_16_times_its_pcm() {
        assert_eq!(most_frames(&flac_at(Some(16)), 1000), 4000);
        assert_eq!(most_frames(&flac_at(Some(24)), 1200), 3200);
        // An unknown or impossible depth counts as 4 bits.
        for bits in [None, Some(0), Some(3), Some(33)] {
            assert_eq!(most_frames(&flac_at(bits), 1000), 16_000, "{bits:?}");
        }
    }

    /// Two hours of 44.1 kHz stereo frames.
    const TWO_HOURS: u64 = 2 * 3600 * 44_100;

    /// Two hours of stereo silence in FLAC frames of 65,535 frames: about
    /// 17 bytes each, for 524,280 bytes of `f32`.
    fn two_hours_of_silence(stated: u64) -> Vec<u8> {
        silence(stated, 65_535, TWO_HOURS.div_ceil(65_535) as usize)
    }

    #[test]
    fn a_flac_that_states_more_than_the_limit_is_refused() {
        let bytes = two_hours_of_silence(TWO_HOURS);
        assert!(bytes.len() < 100_000, "{} bytes", bytes.len());
        let error = decode_bytes(bytes, Some("flac")).unwrap_err();
        assert!(
            format!("{error:#}").contains("too long to open"),
            "{error:#}"
        );
    }

    fn decode_within(bytes: Vec<u8>, ext: &str, limit: u64) -> Result<Audio> {
        decode_source_within(Box::new(Cursor::new(bytes)), Some(ext), limit)
    }

    #[test]
    fn a_flac_of_unstated_length_stops_at_the_limit() {
        const LIMIT: u64 = 64 << 20;
        let bytes = two_hours_of_silence(0);
        let before = LIVE.with(Cell::get);
        PEAK.with(|peak| peak.set(before));
        let error = decode_within(bytes, "flac", LIMIT).unwrap_err();
        let held = (PEAK.with(Cell::get) - before) as u64;
        assert!(
            format!("{error:#}").contains("too long to open"),
            "{error:#}"
        );
        // No channel reserves past the limit, but a realloc holds the old
        // and the new buffer while a channel grows.
        assert!(
            2 * held < 3 * LIMIT,
            "decoding past a {LIMIT}-byte limit held {held} bytes at its peak"
        );
    }

    #[test]
    fn a_wav_past_the_limit_is_refused() {
        let (_, _, stated) = stereo_wav();
        let mut streaming = stated.clone();
        set_u32(&mut streaming, 4, u32::MAX);
        set_u32(&mut streaming, 40, u32::MAX);
        let limit = (FRAMES * 8 - 1) as u64;
        for bytes in [stated, streaming] {
            let error = decode_within(bytes, "wav", limit).unwrap_err();
            assert!(
                format!("{error:#}").contains("too long to open"),
                "{error:#}"
            );
        }
    }

    #[test]
    fn a_song_at_the_limit_opens() {
        let (left, right, stated) = stereo_wav();
        let mut streaming = stated.clone();
        set_u32(&mut streaming, 4, u32::MAX);
        set_u32(&mut streaming, 40, u32::MAX);
        let limit = (FRAMES * 8) as u64;
        for bytes in [stated, streaming] {
            let audio = decode_within(bytes, "wav", limit).unwrap();
            assert_eq!(
                audio.channels,
                [left.clone(), right.clone()].map(|c| {
                    c.iter()
                        .map(|x| step(*x, 16) as f32 / 32768.0)
                        .collect::<Vec<f32>>()
                })
            );
        }
    }

    /// An MPEG-1 Layer III frame of silence at 44.1 kHz stereo: its header,
    /// then zero side information and data.
    fn mp3_frame(bitrate_index: u8, bytes: usize) -> Vec<u8> {
        let mut frame = vec![0; bytes];
        frame[..4].copy_from_slice(&[0xff, 0xfb, bitrate_index << 4, 0x00]);
        frame
    }

    /// 20 quiet frames at 32 kbit/s, then `loud` frames at 320 kbit/s, with
    /// no Xing or VBRI tag.  Symphonia then estimates the length from the
    /// first 16 frames, so it states about 10 times the real length.
    fn quiet_start_mp3(loud: usize) -> Vec<u8> {
        let quiet = (0..20).flat_map(|_| mp3_frame(1, 104));
        let loud = (0..loud).flat_map(|_| mp3_frame(14, 1044));
        quiet.chain(loud).collect()
    }

    /// 4 MiB: more than the estimated length of `quiet_start_mp3(200)`
    /// (about 18.7 MB), less than its real length (about 2 MB).
    const MP3_LIMIT: u64 = 4 << 20;

    #[test]
    fn an_mp3_whose_estimated_length_passes_the_limit_opens() {
        let audio = decode_within(quiet_start_mp3(200), "mp3", MP3_LIMIT).unwrap();
        assert_eq!(audio.frames(), 220 * 1152);
    }

    #[test]
    fn an_mp3_that_states_more_than_the_limit_reserves_at_most_the_limit() {
        let opened = open(
            Box::new(Cursor::new(quiet_start_mp3(200))),
            Some("mp3"),
            MP3_LIMIT,
        )
        .unwrap();
        // Stereo: 8 bytes a frame.
        assert_eq!(opened.reserve as u64, MP3_LIMIT / 8);
    }

    #[test]
    fn a_stated_length_past_the_limit_is_refused_before_it_decodes() {
        let refused = |bytes: Vec<u8>, ext: &str, limit: u64| {
            let error = open(Box::new(Cursor::new(bytes)), Some(ext), limit)
                .err()
                .expect("refused when it opens");
            assert!(
                format!("{error:#}").contains("too long to open"),
                "{error:#}"
            );
        };
        refused(two_hours_of_silence(TWO_HOURS), "flac", MAX_DECODED_BYTES);
        let (_, _, wav) = stereo_wav();
        refused(wav, "wav", (FRAMES * 8 - 1) as u64);
    }

    #[test]
    fn a_channel_never_grows_past_the_most_it_may_hold() {
        let mut channel = vec![0.5; 8000];
        append(&mut channel, &[0.25; 5], 8010);
        assert_eq!(channel.capacity(), 8010);
        assert_eq!(channel.len(), 8005);
    }

    #[test]
    fn the_limit_is_1_gib_or_50_minutes_of_stereo() {
        assert_eq!(MAX_DECODED_BYTES, 1 << 30);
        let minutes = MAX_DECODED_BYTES as f64 / (8.0 * 44_100.0 * 60.0);
        assert!((minutes - 50.7).abs() < 0.05, "{minutes}");
    }
}
