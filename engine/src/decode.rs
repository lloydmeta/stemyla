//! Decoding audio files into per-channel `f32` buffers with Symphonia.
//!
//! [`decode_bytes`] and [`decode_path`] decode a whole file into [`Audio`].
//! [`Decoder`] gives one packet at a time, so a caller can pass the samples
//! on and not hold the file's samples.  Both keep the limit of
//! [`MAX_DECODED_BYTES`].  `decode_bytes` is built on `Decoder`.

use crate::resample::SONG_RATE;
use anyhow::{Context, Result, anyhow};
use std::io::Cursor;
use std::path::Path;
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::well_known::{CODEC_ID_FLAC, CODEC_ID_MP3};
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
/// What a [`Decoder`] reads: `Read + Seek + Send + Sync`, with its length
/// if known.  Re-exported so a caller can give its own source.
pub use symphonia::core::io::MediaSource;
use symphonia::core::io::MediaSourceStream;
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
/// channels, with a mono file counted as two (decision 0013): 50.7 minutes
/// of 44.1 kHz audio.  A FLAC of
/// about 80 kB can hold 2 hours of silence.
pub const MAX_DECODED_BYTES: u64 = 1 << 30;

/// How many times smaller than PCM at its stated bits a FLAC is trusted to
/// be, for its reservation.  The demo song's stems are 2.2 to 11.3 times
/// smaller at 16 bits.
const FLAC_MOST_RATIO: u64 = 16;

/// The frames at [`SONG_RATE`] that `frames` frames at `rate` give: as
/// `resample::to_rate` gives them, the frames times [`SONG_RATE`] over
/// `rate`, rounded up.
pub fn song_frames(frames: u64, rate: u32) -> u64 {
    let product = u128::from(frames) * u128::from(SONG_RATE);
    u64::try_from(product.div_ceil(u128::from(rate.max(1)))).unwrap_or(u64::MAX)
}

/// The error of a song longer than a limit in frames at [`SONG_RATE`]
/// (D3 of `plans/active/stream-from-storage-catchup.md`): the memory
/// path's limit, which a caller gives.  A caller finds it with
/// `anyhow::Error::downcast_ref`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong {
    /// The limit, in frames at [`SONG_RATE`].
    pub max_frames: usize,
}

impl std::fmt::Display for TooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the song is too long to open here: it passes {} frames at {} Hz",
            self.max_frames, SONG_RATE
        )
    }
}

impl std::error::Error for TooLong {}

/// What the head of a file states about its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadLength {
    /// A WAV or a FLAC that states its length: its frames at
    /// [`SONG_RATE`].  A WAV counts at most the frames that its file holds.
    Frames(u64),
    /// A WAV or a FLAC that states no length: a streaming WAV, a WAV whose
    /// data chunk is not in the head, or a FLAC with a total of 0.
    UnstatedPcm,
    /// Any other file, such as an MP3, which states an estimate at most.
    Unstated,
}

/// The bytes of a file's head that [`head_length`] needs at most.  A WAV
/// can put chunks, such as tags, before its data chunk.
pub const HEAD_BYTES: usize = 64 * 1024;

/// Room for tags, such as cover art, in a file whose head states no
/// length: 16 MiB.
pub const TAG_ROOM: u64 = 16 << 20;

/// The bytes of one second of the largest WAV or FLAC that states no
/// length: two channels of 32-bit samples at 384 kHz, the highest rate
/// that the page converts.  The song keeps two channels.
const PCM_BYTES_PER_SECOND: u64 = 384_000 * 2 * 4;

/// The bytes of one second of the largest MP3: 320 kbit/s, the highest
/// bitrate of MPEG-1 Layer III.
const MP3_BYTES_PER_SECOND: u64 = 320_000 / 8;

/// What the first bytes of a file of `file_bytes` bytes state about its
/// length.  It reads only `head` (at most [`HEAD_BYTES`] are needed), so a
/// caller can refuse a long song before it reads the whole file.
pub fn head_length(head: &[u8], file_bytes: u64) -> HeadLength {
    if head.starts_with(b"RIFF") || head.starts_with(b"RF64") {
        return wav_length(head, file_bytes);
    }
    let start = id3_size(head).unwrap_or(0);
    match head.get(start..) {
        Some(rest) if rest.starts_with(b"fLaC") => flac_length(rest),
        _ => HeadLength::Unstated,
    }
}

/// The bytes of the ID3v2 tag at the start of `head`, if it has one.
fn id3_size(head: &[u8]) -> Option<usize> {
    let tag = head.get(..10).filter(|tag| tag.starts_with(b"ID3"))?;
    let size = tag[6..10]
        .iter()
        .fold(0_usize, |size, byte| (size << 7) | usize::from(byte & 0x7F));
    let footer = if tag[5] & 0x10 != 0 { 10 } else { 0 };
    Some(10 + size + footer)
}

/// The little-endian `u16` at `at` in `bytes`.
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

/// The little-endian `u32` at `at` in `bytes`.
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// [`head_length`] of a RIFF or RF64 file.  An RF64 file keeps its sizes in
/// another chunk, so it counts as unstated.
fn wav_length(head: &[u8], file_bytes: u64) -> HeadLength {
    if !head.starts_with(b"RIFF") || head.get(8..12) != Some(b"WAVE") {
        return HeadLength::UnstatedPcm;
    }
    let mut format: Option<(u32, u16)> = None;
    let mut at = 12;
    while let (Some(id), Some(size)) = (head.get(at..at + 4), u32_at(head, at + 4)) {
        let body = at + 8;
        if id == b"fmt " {
            let rate = u32_at(head, body + 4);
            let block = u16_at(head, body + 12);
            format = rate
                .zip(block)
                .filter(|(rate, block)| *rate > 0 && *block > 0);
        } else if id == b"data" {
            let Some((rate, block)) = format else {
                return HeadLength::UnstatedPcm;
            };
            if size == 0 || size == u32::MAX {
                return HeadLength::UnstatedPcm;
            }
            let held = file_bytes.saturating_sub(body as u64);
            let frames = u64::from(size).min(held) / u64::from(block);
            return HeadLength::Frames(song_frames(frames, rate));
        }
        // A chunk's body has an even length: an odd one has a pad byte.
        let next = body as u64 + u64::from(size) + u64::from(size & 1);
        match usize::try_from(next) {
            Ok(next) if next < head.len() => at = next,
            _ => break,
        }
    }
    HeadLength::UnstatedPcm
}

/// [`head_length`] of a FLAC stream that starts at `flac`: its STREAMINFO
/// block, which comes first (RFC 9639, section 8.2).
fn flac_length(flac: &[u8]) -> HeadLength {
    let Some(info) = flac.get(4..42).filter(|block| block[0] & 0x7F == 0) else {
        return HeadLength::UnstatedPcm;
    };
    let info = &info[4..];
    let rate = (u32::from(info[10]) << 12) | (u32::from(info[11]) << 4) | u32::from(info[12] >> 4);
    let low = u32::from_be_bytes([info[14], info[15], info[16], info[17]]);
    let total = (u64::from(info[13] & 0x0F) << 32) | u64::from(low);
    if rate == 0 || total == 0 {
        return HeadLength::UnstatedPcm;
    }
    HeadLength::Frames(song_frames(total, rate))
}

/// The largest file, in bytes, of a song of at most `max_frames` frames at
/// [`SONG_RATE`] whose head states `length`: `None` when it states the
/// length.  A WAV or FLAC: two channels of 32-bit samples at 384 kHz.
/// Anything else: an MP3 at 320 kbit/s.  Each with [`TAG_ROOM`].
pub fn largest_file(length: HeadLength, max_frames: usize) -> Option<u64> {
    let per_second = match length {
        HeadLength::Frames(_) => return None,
        HeadLength::UnstatedPcm => PCM_BYTES_PER_SECOND,
        HeadLength::Unstated => MP3_BYTES_PER_SECOND,
    };
    let bytes = (max_frames as u128 * u128::from(per_second)).div_ceil(u128::from(SONG_RATE));
    Some(
        u64::try_from(bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(TAG_ROOM),
    )
}

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

/// Appends `src` to `channel`, which may hold at most `most` frames.  The
/// caller keeps `channel.len() + src.len()` within `most`.  Past its
/// reservation, the channel grows by an eighth at a time, not by double.
/// So a song-sized channel never briefly holds three times itself.  It
/// never grows past `most`.  The decoder's `shrink_to_fit` returns what is
/// left spare.  The reservation can be none, too big to make, or capped by
/// what the file's bytes could hold.
pub(crate) fn append(channel: &mut Vec<f32>, src: &[f32], most: usize) {
    if src.len() > channel.capacity() - channel.len() {
        let room = most.saturating_sub(channel.len()).max(src.len());
        channel.reserve_exact(src.len().max(channel.len() / 8).min(room));
    }
    channel.extend_from_slice(src);
}

/// The error for a file whose samples pass `limit` bytes.
fn too_long(limit: u64) -> anyhow::Error {
    anyhow!(
        "the song is too long to open: it passes {} MiB of samples, with mono counted as two channels",
        limit >> 20
    )
}

/// The channels that a file of `channels` channels counts as against the
/// limit.  A mono file counts as stereo.  Its one channel then holds at
/// most half the limit, so its realloc, which holds the old and the new
/// buffer, stays under the limit; counted as one channel, it held 1.9
/// times.  The mix lane and the separation request also hold two channels
/// of a mono song.
fn counted(channels: usize) -> u64 {
    channels.max(2) as u64
}

/// Whether `frames` of each of `channels` channels pass `limit` bytes of
/// `f32`.
fn passes(limit: u64, channels: usize, frames: u64) -> bool {
    frames.saturating_mul(4 * counted(channels)) > limit
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
    usize::try_from(limit / (4 * counted(channels))).unwrap_or(usize::MAX)
}

/// The most frames at `rate` whose frames at [`SONG_RATE`] are at most
/// `max_song_frames`.
fn source_frames_within(max_song_frames: usize, rate: u32) -> u64 {
    let product = max_song_frames as u128 * u128::from(rate);
    u64::try_from(product / u128::from(SONG_RATE)).unwrap_or(u64::MAX)
}

/// Opens `source`.  It refuses a WAV or FLAC whose stated length passes
/// `limit` bytes of samples.  An MP3 states only an estimate without a
/// Xing or VBRI tag: a quiet start can make it 10 times too long.  So an
/// MP3 is never refused here; the decoder stops it at the limit.  The
/// reservation of any file is within `limit`.  With `max_song_frames`,
/// it also refuses a WAV or FLAC whose stated
/// length passes that many frames at [`SONG_RATE`] ([`TooLong`]), and
/// reserves no more than that.
fn open_within(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    limit: u64,
    max_song_frames: Option<usize>,
) -> Result<Opened> {
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
    let rate = params.sample_rate.filter(|rate| *rate > 0);
    let stated_song_frames = track
        .num_frames
        .zip(rate)
        .filter(|_| stated_exactly)
        .map(|(frames, rate)| song_frames(frames, rate));
    if let Some(max_frames) = max_song_frames
        && stated_song_frames.is_some_and(|frames| frames > max_frames as u64)
    {
        return Err(anyhow::Error::new(TooLong { max_frames }));
    }
    let most = byte_len.map(|len| most_frames(&params, len));
    let within_song = max_song_frames
        .zip(rate)
        .map_or(u64::MAX, |(max, rate)| source_frames_within(max, rate));
    let within_limit = (frames_within(limit, channels) as u64).min(within_song);
    let reserve = frames_to_reserve(track.num_frames, most.map(|m| m.min(within_limit)));
    Ok(Opened {
        format,
        track_id,
        params,
        reserve,
    })
}

/// Decodes a file one packet at a time, so a caller can keep each packet's
/// samples or pass them on.  It opens the file as `decode_bytes` does, and
/// keeps the limit of [`MAX_DECODED_BYTES`]: it stops with the same error
/// when the frames so far pass it.  It holds one packet's samples, not the
/// file's.
pub struct Decoder {
    format: Box<dyn FormatReader>,
    codec: Box<dyn AudioDecoder>,
    track_id: u32,
    /// The frames that `open` found to reserve, for `decode_bytes`.
    reserve: usize,
    limit: u64,
    /// The most frames at [`SONG_RATE`] that the samples may give, if the
    /// caller set a limit ([`Decoder::within_song_frames`]).
    max_song_frames: Option<usize>,
    /// The rate of the samples: the stated rate, or else the first
    /// packet's.
    sample_rate: u32,
    frames: u64,
    planes: Vec<Vec<f32>>,
}

/// The samples of one packet.
pub struct Packet<'a> {
    /// The rate of the samples, in Hz.
    pub sample_rate: u32,
    /// One `Vec` of samples for each channel, all the same length.
    pub planes: &'a [Vec<f32>],
}

impl Decoder {
    /// Opens `source`, with `ext` (such as `"mp3"`) to help the probe.
    pub fn new(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Decoder> {
        Decoder::within(source, ext, MAX_DECODED_BYTES)
    }

    /// Opens `source` as [`Decoder::new`] does, for a song of at most
    /// `max_frames` frames at [`SONG_RATE`] (D3 of
    /// `plans/active/stream-from-storage-catchup.md`).  It refuses a WAV or
    /// FLAC whose stated length passes that, and stops a decode whose
    /// frames pass it, each with [`TooLong`].  The limit of
    /// [`MAX_DECODED_BYTES`] stays.
    pub fn within_song_frames(
        source: Box<dyn MediaSource>,
        ext: Option<&str>,
        max_frames: usize,
    ) -> Result<Decoder> {
        Decoder::open(source, ext, MAX_DECODED_BYTES, Some(max_frames))
    }

    fn within(source: Box<dyn MediaSource>, ext: Option<&str>, limit: u64) -> Result<Decoder> {
        Decoder::open(source, ext, limit, None)
    }

    fn open(
        source: Box<dyn MediaSource>,
        ext: Option<&str>,
        limit: u64,
        max_song_frames: Option<usize>,
    ) -> Result<Decoder> {
        let Opened {
            format,
            track_id,
            params,
            reserve,
        } = open_within(source, ext, limit, max_song_frames)?;
        let codec = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| anyhow!("no decoder for codec: {e}"))?;
        Ok(Decoder {
            format,
            codec,
            track_id,
            reserve,
            limit,
            max_song_frames,
            sample_rate: params.sample_rate.unwrap_or(0),
            frames: 0,
            planes: Vec::new(),
        })
    }

    /// The samples of the next packet of the audio track, or `None` at the
    /// end of the file.  A corrupt packet is skipped, as players do.  An
    /// error comes before the frames so far pass the limit.
    pub fn next_packet(&mut self) -> Result<Option<Packet<'_>>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(None),
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(anyhow!("reading packet: {e}")),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.codec.decode(&packet) {
                Ok(buf) => {
                    if self.sample_rate == 0 {
                        self.sample_rate = buf.spec().rate();
                    }
                    buf.copy_to_vecs_planar::<f32>(&mut self.planes);
                    let packet_frames = self.planes.first().map_or(0, Vec::len) as u64;
                    let frames = self.frames + packet_frames;
                    if passes(self.limit, self.planes.len(), frames) {
                        return Err(too_long(self.limit));
                    }
                    if let Some(max_frames) = self.max_song_frames
                        && song_frames(frames, self.sample_rate) > max_frames as u64
                    {
                        return Err(anyhow::Error::new(TooLong { max_frames }));
                    }
                    self.frames = frames;
                    return Ok(Some(Packet {
                        sample_rate: self.sample_rate,
                        planes: &self.planes,
                    }));
                }
                Err(SymError::DecodeError(_)) => continue,
                Err(e) => return Err(anyhow!("decoding packet: {e}")),
            }
        }
    }

    /// The frames of each channel decoded so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The frames at [`SONG_RATE`] to make room for: the stated length,
    /// within what the file's bytes and the limits allow (see `open`).
    pub(crate) fn song_frames_to_reserve(&self) -> usize {
        if self.sample_rate == 0 {
            return 0;
        }
        usize::try_from(song_frames(self.reserve as u64, self.sample_rate)).unwrap_or(0)
    }
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
    let mut decoder = Decoder::within(source, ext, limit)?;
    let reserve = decoder.reserve;
    let mut sample_rate = 0;
    let mut channels: Vec<Vec<f32>> = Vec::new();
    while let Some(packet) = decoder.next_packet()? {
        sample_rate = packet.sample_rate;
        let most = frames_within(limit, packet.planes.len());
        if channels.is_empty() {
            // `open` reserved for the stated channels; the planes can be
            // more.
            channels = packet
                .planes
                .iter()
                .map(|_| channel_with_room_for(reserve.min(most)))
                .collect();
        }
        for (dst, src) in channels.iter_mut().zip(packet.planes) {
            append(dst, src, most);
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
    use crate::test_alloc::peak_of;
    use crate::test_flac::{Blocks, by_hand, compressed, step};
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
        open_within(
            Box::new(Cursor::new(bytes)),
            Some(ext),
            MAX_DECODED_BYTES,
            None,
        )
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
        silence_of(2, stated, block, blocks)
    }

    /// As `silence`, in `channels` channels.
    fn silence_of(channels: usize, stated: u64, block: usize, blocks: usize) -> Vec<u8> {
        by_hand(
            44_100,
            16,
            channels,
            stated,
            Blocks::Silence { block, blocks },
        )
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
        let blocks = TWO_HOURS.div_ceil(65_535) as usize;
        for channels in [1, 2] {
            let bytes = silence_of(channels, 0, 65_535, blocks);
            let (held, result) = peak_of(|| decode_within(bytes, "flac", LIMIT));
            let held = held as u64;
            let error = result.unwrap_err();
            assert!(
                format!("{error:#}").contains("too long to open"),
                "{error:#}"
            );
            // The peak while a channel grows, when a realloc holds the old
            // and the new buffer.  A mono file counts as stereo, so its one
            // channel's realloc does not reach twice the limit.
            // `a_channel_never_grows_past_the_most_it_may_hold` checks the
            // cap on each channel.
            assert!(
                2 * held < 3 * LIMIT,
                "{channels} channels: decoding past a {LIMIT}-byte limit held {held} bytes at its peak"
            );
        }
    }

    #[test]
    fn a_mono_file_counts_as_stereo() {
        let (left, _, _) = stereo_wav();
        let mono = [left.iter().map(|x| step(*x, 16)).collect::<Vec<i32>>()];
        let bytes = || by_hand(44_100, 16, 1, FRAMES as u64, Blocks::Verbatim(&mono));
        // The limit counts 8 bytes a frame, as for stereo.
        let stereo_bytes = (FRAMES * 8) as u64;
        assert_eq!(
            decode_within(bytes(), "flac", stereo_bytes)
                .unwrap()
                .frames(),
            FRAMES
        );
        let error = decode_within(bytes(), "flac", stereo_bytes - 1).unwrap_err();
        assert!(
            format!("{error:#}").contains("too long to open"),
            "{error:#}"
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

    /// The channel mode byte of an MP3 frame header: stereo or mono.
    const STEREO: u8 = 0x00;
    const MONO: u8 = 0xc0;

    /// An MPEG-1 Layer III frame of silence at 44.1 kHz: its header, then
    /// zero side information and data.
    fn mp3_frame(bitrate_index: u8, mode: u8, bytes: usize) -> Vec<u8> {
        let mut frame = vec![0; bytes];
        frame[..4].copy_from_slice(&[0xff, 0xfb, bitrate_index << 4, mode]);
        frame
    }

    /// 20 quiet frames at 32 kbit/s, then `loud` frames at 320 kbit/s, with
    /// no Xing or VBRI tag.  Symphonia then estimates the length from the
    /// first 16 frames, so it states about 10 times the real length.
    fn quiet_start_mp3(loud: usize) -> Vec<u8> {
        quiet_start_mp3_in(STEREO, loud)
    }

    fn quiet_start_mp3_in(mode: u8, loud: usize) -> Vec<u8> {
        let quiet = (0..20).flat_map(|_| mp3_frame(1, mode, 104));
        let loud = (0..loud).flat_map(|_| mp3_frame(14, mode, 1044));
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
        // 8 bytes a frame, for stereo and for mono, which counts as stereo.
        for mode in [STEREO, MONO] {
            let opened = open_within(
                Box::new(Cursor::new(quiet_start_mp3_in(mode, 200))),
                Some("mp3"),
                MP3_LIMIT,
                None,
            )
            .unwrap();
            assert_eq!(opened.reserve as u64, MP3_LIMIT / 8, "mode {mode:#x}");
        }
    }

    #[test]
    fn a_stated_length_past_the_limit_is_refused_before_it_decodes() {
        let refused = |bytes: Vec<u8>, ext: &str, limit: u64| {
            let error = open_within(Box::new(Cursor::new(bytes)), Some(ext), limit, None)
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

    /// Counts the frames it gets, and keeps nothing.
    struct Counter(usize);

    impl crate::song::FrameSink for Counter {
        fn put(&mut self, left: &[f32], right: &[f32]) -> Result<()> {
            assert_eq!(left.len(), right.len());
            self.0 += left.len();
            Ok(())
        }
    }

    /// A stereo `ext` file of `seconds` of tones at `rate`.
    fn file_of(ext: &str, seconds: usize, rate: u32) -> Vec<u8> {
        use crate::test_audio::{WavSamples, flac, mp3, tone, wav};
        let frames = seconds * rate as usize;
        let tones = [tone(0, frames, rate), tone(1, frames, rate)];
        match ext {
            "wav" => wav(&tones, rate, WavSamples::Pcm16),
            "flac" => flac(&tones, rate),
            _ => mp3(&tones, rate),
        }
    }

    #[test]
    fn a_block_decode_keeps_its_memory_flat() {
        const MIB: usize = 1 << 20;
        for rate in [44_100, 48_000] {
            for ext in ["wav", "flac", "mp3"] {
                let (short, long) = (file_of(ext, 2, rate), file_of(ext, 60, rate));
                let want = decode_bytes(long.clone(), Some(ext)).unwrap().frames();
                let blocks = |bytes: Vec<u8>| {
                    peak_of(|| {
                        let decoder = Decoder::new(Box::new(Cursor::new(bytes)), Some(ext));
                        let mut counter = Counter(0);
                        crate::song::decode_into(decoder.unwrap(), &mut counter).unwrap();
                        counter.0
                    })
                };
                let (short_peak, _) = blocks(short.clone());
                let (long_peak, frames) = blocks(long.clone());
                let case = format!("{ext} at {rate} Hz");
                let want = (want as u64 * 44_100).div_ceil(u64::from(rate)) as usize;
                assert_eq!(frames, want, "{case}: the frames of a minute");
                assert!(
                    long_peak.abs_diff(short_peak) < MIB,
                    "{case}: 2 s peaks {short_peak} bytes, 60 s peaks {long_peak} bytes"
                );
                // The control: a whole decode of the long file holds it.
                let (whole_short, _) = peak_of(|| decode_bytes(short, Some(ext)).unwrap());
                let (whole_long, _) = peak_of(|| decode_bytes(long, Some(ext)).unwrap());
                assert!(
                    whole_long > whole_short + MIB,
                    "{case}: the control peaks {whole_short} and {whole_long} bytes"
                );
            }
        }
    }

    /// Whether `result` is the error of `too_long`.
    fn is_too_long<T>(result: Result<T>) -> bool {
        result.is_err_and(|e| format!("{e:#}").contains("too long to open"))
    }

    /// Decodes every packet of `decoder`, to its end or an error.
    fn every_packet(mut decoder: Decoder) -> Result<u64> {
        while decoder.next_packet()?.is_some() {}
        Ok(decoder.frames())
    }

    /// A file of `bytes` whose reads fail from byte `fail_from` on, with an
    /// error of kind `Other`, as the decode worker's source fails (D7a).
    struct Failing {
        bytes: Cursor<Vec<u8>>,
        fail_from: u64,
    }

    impl std::io::Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.bytes.position() + buf.len() as u64 > self.fail_from {
                return Err(std::io::Error::other("NotReadableError"));
            }
            self.bytes.read(buf)
        }
    }

    impl std::io::Seek for Failing {
        fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
            self.bytes.seek(to)
        }
    }

    impl MediaSource for Failing {
        fn is_seekable(&self) -> bool {
            true
        }

        fn byte_len(&self) -> Option<u64> {
            Some(self.bytes.get_ref().len() as u64)
        }
    }

    /// D7a: a read that fails in the middle of a file is an error, never
    /// the end of the file.
    #[test]
    fn a_read_that_fails_is_an_error_not_the_end() {
        let (_, _, wav) = stereo_wav();
        let failing = Failing {
            bytes: Cursor::new(wav),
            fail_from: 20_000,
        };
        let decoder =
            Decoder::new(Box::new(failing), Some("wav")).expect("the probe reads the start");
        let ended = every_packet(decoder);
        let error = ended.expect_err("a failed read is an error");
        assert!(
            format!("{error:#}").contains("NotReadableError"),
            "{error:#}"
        );
    }

    #[test]
    fn the_decode_limit_stops_a_block_decode() {
        let within = |bytes: Vec<u8>, ext: &str, limit: u64| {
            Decoder::within(Box::new(Cursor::new(bytes)), Some(ext), limit)
        };
        // A stated length past the limit is refused when it opens.
        let (_, _, wav) = stereo_wav();
        assert!(is_too_long(within(
            wav.clone(),
            "wav",
            (FRAMES * 8 - 1) as u64
        )));
        let flac = two_hours_of_silence(TWO_HOURS);
        assert!(is_too_long(Decoder::new(
            Box::new(Cursor::new(flac)),
            Some("flac")
        )));
        // An unstated length stops when the frames so far pass it, and a
        // song at the limit opens.
        let mut streaming = wav;
        set_u32(&mut streaming, 4, u32::MAX);
        set_u32(&mut streaming, 40, u32::MAX);
        let limit = (FRAMES * 8) as u64;
        let decoder = within(streaming.clone(), "wav", limit - 1).unwrap();
        assert!(is_too_long(every_packet(decoder)));
        let decoder = within(streaming, "wav", limit).unwrap();
        assert_eq!(every_packet(decoder).unwrap(), FRAMES as u64);
        // A mono FLAC of unstated length counts as stereo.
        const LIMIT: u64 = 64 << 20;
        for channels in [1, 2] {
            let blocks = TWO_HOURS.div_ceil(65_535) as usize;
            let bytes = silence_of(channels, 0, 65_535, blocks);
            let decoder = within(bytes, "flac", LIMIT).unwrap();
            assert!(is_too_long(every_packet(decoder)), "{channels} channels");
        }
        // An MP3 is never refused when it opens; it stops while it decodes.
        let decoder = within(quiet_start_mp3(200), "mp3", MP3_LIMIT / 4).unwrap();
        assert!(is_too_long(every_packet(decoder)));
        // `decode_into` gives the decoder's error.
        let decoder = within(quiet_start_mp3(200), "mp3", MP3_LIMIT / 4).unwrap();
        let mut counter = Counter(0);
        assert!(is_too_long(crate::song::decode_into(decoder, &mut counter)));
        assert!(
            counter.0 as u64 * 8 <= MP3_LIMIT / 4,
            "{} frames",
            counter.0
        );
    }

    /// The frames at `SONG_RATE` of `frames` frames at `rate`, as the
    /// converter gives them.
    fn converted(frames: usize, rate: u32) -> u64 {
        crate::resample::to_rate(vec![0.0; frames], rate, crate::resample::SONG_RATE)
            .unwrap()
            .len() as u64
    }

    #[test]
    fn song_frames_are_the_frames_that_the_converter_gives() {
        for rate in [8_000, 22_050, 44_100, 48_000, 96_000, 384_000] {
            for frames in [1, 161, FRAMES, 48_000] {
                assert_eq!(
                    song_frames(frames as u64, rate),
                    converted(frames, rate),
                    "{frames} frames at {rate} Hz"
                );
            }
        }
    }

    /// D3 of the catch-up plan: the head of a WAV or a FLAC states its
    /// length, in frames at `SONG_RATE`, as the decode gives them.
    #[test]
    fn a_head_states_the_length_of_a_wav_or_a_flac() {
        use crate::test_audio::{WavSamples, flac, mp3, tone, wav};
        for rate in [22_050, 44_100, 48_000] {
            let tones = [tone(0, FRAMES, rate), tone(1, FRAMES, rate)];
            let want = HeadLength::Frames(converted(FRAMES, rate));
            let files = [
                wav(&tones, rate, WavSamples::Pcm16),
                wav(&tones, rate, WavSamples::Float),
                flac(&tones, rate),
                flac(&tones[..1], rate),
            ];
            for (k, file) in files.iter().enumerate() {
                let size = file.len() as u64;
                let head = &file[..HEAD_BYTES.min(file.len())];
                assert_eq!(head_length(head, size), want, "file {k} at {rate} Hz");
                // The header alone is enough.
                let header = if k < 2 { 44 } else { 42 };
                assert_eq!(head_length(&file[..header], size), want, "file {k}");
            }
            let mp3 = mp3(&tones, rate);
            assert_eq!(head_length(&mp3, mp3.len() as u64), HeadLength::Unstated);
        }
        // A WAV of unstated length: a streaming WAV, or a size of 0.
        let (_, _, wav) = stereo_wav();
        let size = wav.len() as u64;
        for unstated in [u32::MAX, 0] {
            let mut streaming = wav.clone();
            set_u32(&mut streaming, 40, unstated);
            assert_eq!(head_length(&streaming, size), HeadLength::UnstatedPcm);
        }
        // A WAV that states more than its file holds: what the file holds.
        let mut truncated = wav.clone();
        set_u32(&mut truncated, 40, (FRAMES * 4 * 1000) as u32);
        assert_eq!(
            head_length(&truncated, size),
            HeadLength::Frames(FRAMES as u64)
        );
        // A head that ends before the data chunk.
        assert_eq!(head_length(&wav[..40], size), HeadLength::UnstatedPcm);
        // A FLAC of unstated length, and a FLAC after an ID3v2 tag.
        let silence = silence_of(2, 0, 4_096, 2);
        let flac_size = silence.len() as u64;
        assert_eq!(head_length(&silence, flac_size), HeadLength::UnstatedPcm);
        let stated = silence_of(2, 8_192, 4_096, 2);
        let mut tagged = vec![b'I', b'D', b'3', 4, 0, 0, 0, 0, 1, 0];
        tagged.extend_from_slice(&[0; 128]);
        tagged.extend_from_slice(&stated);
        assert_eq!(
            head_length(&tagged, tagged.len() as u64),
            HeadLength::Frames(8_192)
        );
        // An ID3v2 tag before an MP3, and bytes of no known format.
        let mut tagged_mp3 = vec![b'I', b'D', b'3', 4, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0];
        tagged_mp3.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        assert_eq!(head_length(&tagged_mp3, 1 << 20), HeadLength::Unstated);
        assert_eq!(head_length(b"lloyd", 5), HeadLength::Unstated);
    }

    /// D3 of the catch-up plan: the cap of a file whose head states no
    /// length.  A WAV or FLAC: 384 kHz, 32 bits, two channels.  An MP3:
    /// 320 kbit/s.  Each with room for tags.
    #[test]
    fn the_largest_file_of_a_song_whose_head_states_no_length() {
        let hundred_seconds = 100 * 44_100;
        assert_eq!(TAG_ROOM, 16 << 20);
        assert_eq!(largest_file(HeadLength::Frames(1), hundred_seconds), None);
        assert_eq!(
            largest_file(HeadLength::UnstatedPcm, hundred_seconds),
            Some(307_200_000 + TAG_ROOM)
        );
        assert_eq!(
            largest_file(HeadLength::Unstated, hundred_seconds),
            Some(4_000_000 + TAG_ROOM)
        );
        // Part of a second rounds up.
        assert_eq!(largest_file(HeadLength::Unstated, 1), Some(1 + TAG_ROOM));
    }

    #[test]
    fn the_limit_is_1_gib_or_50_minutes_of_stereo() {
        assert_eq!(MAX_DECODED_BYTES, 1 << 30);
        let minutes = MAX_DECODED_BYTES as f64 / (8.0 * 44_100.0 * 60.0);
        assert!((minutes - 50.7).abs() < 0.05, "{minutes}");
    }
}
