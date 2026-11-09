//! Decoding audio files into per-channel `f32` buffers with Symphonia.
//!
//! [`decode_bytes`] and [`decode_path`] decode a whole file into [`Audio`].
//! [`Decoder`] gives one packet at a time, so a caller can pass the samples
//! on and not hold the file's samples.  Both keep the limit of
//! [`MAX_DECODED_BYTES`].  `decode_bytes` is built on `Decoder`.

use crate::hidden::Container;
use crate::mp4_boxes::{Found, SoundTrack};
use crate::resample::SONG_RATE;
use anyhow::{Context, Result, anyhow};
use std::collections::VecDeque;
use std::io::{Cursor, SeekFrom};
use std::path::Path;
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_ADPCM_IMA_QT, CODEC_ID_ADPCM_MS, CODEC_ID_ALAC, CODEC_ID_FLAC,
    CODEC_ID_MP3,
};
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
/// What a [`Decoder`] reads: `Read + Seek + Send + Sync`, with its length
/// if known.  Re-exported so a caller can give its own source.
pub use symphonia::core::io::MediaSource;
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::packet::Packet as SymPacket;

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

/// How many times smaller than PCM at its stated bits a FLAC or an ALAC is
/// trusted to be, for its reservation.  The demo song's stems are 2.2 to
/// 11.3 times smaller at 16 bits as FLAC.
const FLAC_MOST_RATIO: u64 = 16;

/// The bytes of one second of AAC at 32 kbit/s, the lowest bitrate that
/// an AAC is trusted to have, for its reservation.  Music is at 96 kbit/s
/// or more.
const AAC_LEAST_BYTES_PER_SECOND: u64 = 32_000 / 8;

/// The frames at [`SONG_RATE`] that `frames` frames at `rate` give: as
/// `resample::to_rate` gives them, the frames times [`SONG_RATE`] over
/// `rate`, rounded up.
pub fn song_frames(frames: u64, rate: u32) -> u64 {
    let product = u128::from(frames) * u128::from(SONG_RATE);
    u64::try_from(product.div_ceil(u128::from(rate.max(1)))).unwrap_or(u64::MAX)
}

/// The error of a song longer than a limit in frames at [`SONG_RATE`]
/// (D3 of `plans/completed/stream-from-storage-catchup.md`): the memory
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

/// The text of a file that holds video (D4 of `plans/active/m4a.md`).
/// `app` shows it too, when a head shows a video track.
pub const VIDEO_TEXT: &str =
    "This file holds video.  Open an audio file, such as M4A, MP3, WAV or FLAC.";

/// The text of an MP4 whose sample tables pass [`most_moov_bytes`], which
/// Symphonia's MP4 reader would read whole (MEM-4).
pub const MOOV_TEXT: &str =
    "The index of this file (its moov box) is too large for its length, so it cannot open.";

/// The text of a file whose tags that the decode cannot hide from
/// Symphonia pass the memory that they may keep
/// (`engine::hidden::MOST_KEPT_BYTES`).
pub const TAGS_TEXT: &str =
    "This file holds too many tags to open safely.  Remove its tags, or open another file.";

/// The most bytes of the first `moov` that Symphonia holds (`mvhd`,
/// `mvex`, and `tkhd`, `edts` and `mdia` of each `trak`,
/// `Found::moov_held`): 16 MiB of slack ([`TAG_ROOM`]),
/// and 32 bytes of tables for each 1024 units of the `mdhd` duration of
/// the first sound track, up to the decode limit of a stereo file.  The
/// decode hides the tags of `moov` from Symphonia (`engine::hidden`), so
/// they do not count.
fn most_moov_bytes(found: &Found) -> u64 {
    let most_frames = MAX_DECODED_BYTES / 8;
    let frames = found
        .sound
        .as_ref()
        .and_then(|sound| sound.duration)
        .map_or(0, |duration| duration.min(most_frames));
    TAG_ROOM + 32 * frames.div_ceil(1024)
}

/// The text of a protected file: a sound track of the sample entry
/// `drms`, `drmi` or `enca`.
pub const PROTECTED_TEXT: &str = "This file is protected (DRM), so it cannot open.";

/// The text of an HE-AAC file: SBR or PS in its AudioSpecificConfig, or a
/// core rate of half the rate of its sample entry.
pub const HE_AAC_TEXT: &str = "HE-AAC is not supported.  Open an AAC-LC file.";

/// The text of an AAC of an object type other than AAC-LC, of a program
/// config element, of 960 frames, of an AudioSpecificConfig that does not
/// parse, or of a sample entry rate that is not the rate of its config.
pub const AAC_KIND_TEXT: &str = "This kind of AAC does not open.  Open an AAC-LC file.";

/// The text of an ADTS frame of more than one raw data block and no CRC.
/// Symphonia 0.6.1 refuses more than one block, and with no CRC the frame
/// states no position of its blocks to split it by (TD-86).
pub const ADTS_BLOCKS_TEXT: &str = "This AAC file puts more than one AAC frame in each ADTS frame, so it cannot open.  Open an M4A file, or another AAC file.";

/// The text of an AIFC file of a compression type that no decoder of
/// Symphonia 0.6.1 decodes, such as `MAC3`, `MAC6`, `GSM ` or `QDMC`.  The
/// AIFF reader of Symphonia maps only PCM, A-law and µ-law types
/// (`aiff/chunks.rs`, lines 276 to 287), each of which opens.  `ima4` opens
/// through `engine::ima4` and Symphonia's ADPCM decoder.
pub const AIFC_COMPRESSED_TEXT: &str = "This AIFC file is compressed in a form that does not open.  Open an uncompressed AIFF or AIFC file.";

/// The most channels of a file that the decode opens (finding 10-1 of review
/// round 10, MEM-4).  The decoders of Symphonia keep a buffer for each
/// channel when they are made, before the first packet: measured with
/// `peak_of` on 2026-11-09, `Decoder::new` of an `ima4` AIFC holds about
/// 33 KB for each channel (`ima4::Ima4Channels`: a mono decoder and a
/// plane of 4096 frames), and of a PCM AIFF about 2.4 KB.  So the bound
/// keeps the open of each format under 8 MiB, with a margin of 2.  A WAV
/// has at most 26 (`symphonia-format-riff` `wave/chunks.rs`, line 716).
pub const MOST_CHANNELS: u16 = 128;

/// The text of a Microsoft ADPCM WAV whose step passes the range that
/// Symphonia 0.6.1 can multiply (findings 10-2 and 10-3 of review round 10).
pub const MS_ADPCM_TEXT: &str =
    "This WAV file holds Microsoft ADPCM that does not decode.  Open another file.";

/// The text of a file of more than [`MOST_CHANNELS`] channels.
pub fn channels_text(channels: u32) -> String {
    format!("This file has {channels} channels.  A file of at most {MOST_CHANNELS} channels opens.")
}

/// The text of an AAC file of more than 2 channels.
pub fn aac_channels_text(channels: u32) -> String {
    format!("This AAC file has {channels} channels.  Only mono and stereo AAC open.")
}

/// Refuses an AAC track that the decode cannot give right, before the
/// decoder is made.  It reads the AudioSpecificConfig itself
/// (`aac_config`).  A raw ADTS stream has none, so only its channels count.
fn check_aac(params: &AudioCodecParameters) -> Result<()> {
    let Some(asc) = params.extra_data.as_deref() else {
        let channels = params.channels.as_ref().map_or(0, Channels::count) as u32;
        if channels > 2 {
            return Err(anyhow!(aac_channels_text(channels)));
        }
        return Ok(());
    };
    let Some(config) = crate::aac_config::read(asc) else {
        return Err(anyhow!(AAC_KIND_TEXT));
    };
    // An entry that states twice the rate of the core is implicit SBR: the
    // entry gives the rate of the output.  An entry of rate 0 cannot hold
    // the rate (88.2 or 96 kHz), so it is unknown.  Any other rate of the
    // entry would play the core at the wrong speed.
    let rates = config
        .rate
        .zip(params.sample_rate.filter(|entry| *entry > 0));
    let implicit_sbr = rates.is_some_and(|(core, entry)| u64::from(entry) == 2 * u64::from(core));
    if config.sbr || implicit_sbr {
        return Err(anyhow!(HE_AAC_TEXT));
    }
    if rates.is_some_and(|(core, entry)| core != entry) {
        return Err(anyhow!(AAC_KIND_TEXT));
    }
    if let Some(channels) = crate::aac_config::channels_of(config.channel_config)
        && channels > 2
    {
        return Err(anyhow!(aac_channels_text(channels)));
    }
    let no_layout = config.channel_config == 0 && config.pce_channels.is_none();
    if config.object_type != 2 || no_layout || config.frames_960 {
        return Err(anyhow!(AAC_KIND_TEXT));
    }
    Ok(())
}

/// The frames that a decode drops at the start (`skip`), and the most that
/// it gives after them (`keep`, if known): the encoder delay and the
/// padding of an M4A (D7 of `plans/active/m4a.md`).  The frames are at
/// the file's rate, before any conversion to [`SONG_RATE`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Trim {
    skip: u64,
    keep: Option<u64>,
}

/// 1.0 as the 16.16 rate of an `elst` entry.
const EDIT_RATE_ONE: i32 = 0x1_0000;

/// The trim of the walker's sound track at `rate`: from `elst`, else from
/// `iTunSMPB`, else none (D7 of `plans/active/m4a.md`).
fn trim_of(found: &Found, sound: &SoundTrack, rate: Option<u32>) -> Trim {
    edit_trim(found, sound, rate)
        .or_else(|| itunsmpb_trim(found.itunsmpb.as_deref()?, sound.duration))
        .unwrap_or_default()
}

/// The trim of an `elst` of one entry with `media_time >= 0` and rate 1.0,
/// after any leading empty edits, whose media timescale is `rate`.  `keep`
/// is `segment_duration` in the media timescale, to the nearest frame.  A
/// movie timescale or a `segment_duration` of 0 gives no `keep`.
fn edit_trim(found: &Found, sound: &SoundTrack, rate: Option<u32>) -> Option<Trim> {
    let media_scale = sound.media_timescale.filter(|scale| Some(*scale) == rate)?;
    let edits = sound.edits.as_deref()?;
    let first = edits.iter().position(|edit| edit.media_time != -1)?;
    let [edit] = &edits[first..] else {
        return None;
    };
    if edit.media_time < 0 || edit.rate != EDIT_RATE_ONE {
        return None;
    }
    let keep = found
        .movie_timescale
        .filter(|scale| *scale > 0 && edit.segment_duration > 0)
        .map(|movie_scale| {
            let ticks = u128::from(edit.segment_duration) * u128::from(media_scale);
            let movie_scale = u128::from(movie_scale);
            u64::try_from((ticks + movie_scale / 2) / movie_scale).unwrap_or(u64::MAX)
        });
    Some(Trim {
        skip: edit.media_time as u64,
        keep,
    })
}

/// The trim of an `iTunSMPB` text of at least 4 fields of hexadecimal:
/// `skip` is field 2 and `keep` field 4.  A field 4 of 0 gives `keep` as
/// `duration` less `skip` and field 3 (the padding), if that is above 0.
fn itunsmpb_trim(text: &str, duration: Option<u64>) -> Option<Trim> {
    let fields: Vec<u64> = text
        .split_whitespace()
        .map(|field| u64::from_str_radix(field, 16).ok())
        .collect::<Option<_>>()?;
    let [_, skip, padding, frames, ..] = fields[..] else {
        return None;
    };
    let keep = match frames {
        0 => duration
            .and_then(|d| d.checked_sub(skip)?.checked_sub(padding))
            .filter(|keep| *keep > 0),
        frames => Some(frames),
    };
    Some(Trim { skip, keep })
}

/// What the head of a file states about its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadLength {
    /// A WAV, FLAC, M4A or AIFF that states its length: its frames at
    /// [`SONG_RATE`].  A WAV or an AIFF counts at most the frames that its
    /// file holds.  An M4A states the `mdhd` duration of its first sound
    /// track, which counts the AAC encoder delay.
    Frames(u64),
    /// A file that states no length, but whose samples can be as large as
    /// PCM: a streaming WAV, a WAV whose data chunk is not in the head, a
    /// FLAC with a total of 0, an MP4 whose head does not show the `mdhd`
    /// of its first sound track (a `moov` after the head, or after large
    /// cover art), a raw AAC stream (ADTS), or an AIFF whose `COMM` is not
    /// in the head.
    UnstatedPcm,
    /// Any other file, such as an MP3, which states an estimate at most.
    Unstated,
    /// An MP4 whose head shows a track of video (D4 of
    /// `plans/active/m4a.md`).  A caller refuses it with [`VIDEO_TEXT`].
    Video,
}

/// The bytes of a file's head that [`head_length`] needs at most.  A WAV
/// can put chunks, such as tags, before its data chunk.
pub const HEAD_BYTES: usize = 64 * 1024;

/// Room for tags, such as cover art, in the largest file of a song
/// ([`largest_file`]): 16 MiB.
pub const TAG_ROOM: u64 = 16 << 20;

/// The bytes of one second of the largest PCM: two channels of 32-bit
/// samples at 384 kHz, the highest rate that the page converts.  The song
/// keeps two channels.  [`largest_file`] uses it for
/// [`HeadLength::Frames`] and [`HeadLength::UnstatedPcm`].
const PCM_BYTES_PER_SECOND: u64 = 384_000 * 2 * 4;

/// The bytes of one second of the largest MP3: 320 kbit/s, the highest
/// bitrate of MPEG-1 Layer III.
const MP3_BYTES_PER_SECOND: u64 = 320_000 / 8;

/// What the first bytes of a file of `file_bytes` bytes state about its
/// length.  It reads only `head` (at most [`HEAD_BYTES`] are needed), so a
/// caller can refuse a long song before it reads the whole file.
pub fn head_length(head: &[u8], file_bytes: u64) -> HeadLength {
    // The scan and the start of the decode (finding 5-6 of review round 5,
    // 6-1 of review round 6): the first marker that Symphonia's probe
    // accepts after any ID3v2 tags, within the head.  A slice never fails a
    // read.
    use crate::hidden::{LAST_MARKER, first_marker, reader_start};
    let view = crate::hidden::leading_id3_bytes(&mut { head }).unwrap_or(0);
    let Some(after_tags) = usize::try_from(view).ok().and_then(|view| head.get(view..)) else {
        return HeadLength::Unstated;
    };
    let Some((container, marker)) = first_marker(after_tags, 0..LAST_MARKER + 1) else {
        // The decode steps over a chain of top-level boxes to an `ftyp`
        // past the window (`hidden::ftyp_after_boxes`), so a head that starts
        // with such a box is an MP4 whose length the head does not show
        // (finding 8-8 of review round 8).
        // An ADTS stream of more raw data blocks in a frame has no marker
        // either (finding 8-3 of review round 8).
        let adts_of_blocks = crate::hidden::is_adts_of_raw_blocks(after_tags);
        return if starts_with_a_first_box(after_tags) || adts_of_blocks {
            HeadLength::UnstatedPcm
        } else {
            HeadLength::Unstated
        };
    };
    let at = reader_start(container, marker as u64) as usize;
    let rest = &after_tags[at..];
    let rest_bytes = file_bytes.saturating_sub(view + at as u64);
    match container {
        Container::Wav => wav_length(rest, rest_bytes),
        Container::Aiff => aiff_length(rest, rest_bytes),
        Container::Flac => flac_length(rest),
        Container::Mp4 => match crate::mp4_boxes::walk(&mut { rest }) {
            Ok(Some(found)) => mp4_length(&found),
            _ => HeadLength::UnstatedPcm,
        },
        Container::Adts => HeadLength::UnstatedPcm,
        Container::Mp3 => HeadLength::Unstated,
    }
}

/// Whether `bytes` start with a box that can start an MP4
/// (`mp4_boxes::FIRST_BOXES`), of a size of at least its header.
fn starts_with_a_first_box(bytes: &[u8]) -> bool {
    let Some(kind) = bytes.get(4..8) else {
        return false;
    };
    let first = crate::mp4_boxes::FIRST_BOXES
        .iter()
        .any(|first| kind == &first[..]);
    let size = be_u32_at(bytes, 0).unwrap_or(0);
    let large = bytes
        .get(8..16)
        .map(|large| u64::from_be_bytes(large.try_into().unwrap_or_default()));
    let fits = match size {
        1 => large.is_some_and(|large| large >= 16),
        size => size >= 8,
    };
    first && fits
}

/// [`head_length`] of an MP4 file, from the boxes in its head.  The `moov`
/// can end after the head; the walker reads the boxes that are in it.
fn mp4_length(found: &Found) -> HeadLength {
    if found.video {
        return HeadLength::Video;
    }
    // A duration of 0, as a fragmented MP4 has, states no length.  The
    // walker gives no duration for one of all ones (unknown).
    let stated = found.sound.as_ref().and_then(|sound| {
        let scale = sound.media_timescale.filter(|scale| *scale > 0)?;
        let duration = sound.duration.filter(|duration| *duration > 0)?;
        Some(song_frames(duration, scale))
    });
    stated.map_or(HeadLength::UnstatedPcm, HeadLength::Frames)
}

/// The big-endian `u32` at `at` in `bytes`.
fn be_u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The rate of the 80-bit extended float of an AIFF `COMM` chunk, in whole
/// Hz: 0 if it is not a positive rate below 2^32.
fn extended_rate(bytes: &[u8; 10]) -> u32 {
    if bytes[0] & 0x80 != 0 {
        return 0;
    }
    let exponent = i32::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    let mantissa = u64::from_be_bytes([
        bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
    ]);
    let shift = 16_383 + 63 - exponent;
    if !(0..64).contains(&shift) {
        return 0;
    }
    u32::try_from(mantissa >> shift).unwrap_or(0)
}

/// [`head_length`] of an AIFF or AIFC: the frames and the rate of its
/// `COMM` chunk.  When `SSND` is in the head, an AIFF counts at most the
/// frames that its file holds, as a WAV does.  An AIFC can be compressed,
/// so its frames stay as stated.  A missing `COMM`, a rate of 0 or a frame
/// count of 0 states no length.
fn aiff_length(head: &[u8], file_bytes: u64) -> HeadLength {
    let plain = head.get(8..12) == Some(b"AIFF");
    // The frames, the rate and the bytes of a frame, from `COMM`.
    let mut format: Option<(u64, u32, u64)> = None;
    // An AIFC of `ima4`: `COMM` counts blocks of 64 frames, each of 34 bytes
    // for each channel (finding 9-1 of review round 9).
    let mut ima4 = false;
    // The bytes of samples that the file holds, from `SSND`.
    let mut held: Option<u64> = None;
    let mut at = 12;
    while let (Some(id), Some(size)) = (head.get(at..at + 4), be_u32_at(head, at + 4)) {
        let body = at + 8;
        if id == b"COMM" {
            let Some(comm) = head.get(body..body + 18) else {
                break;
            };
            let channels = u64::from(u16::from_be_bytes([comm[0], comm[1]]));
            // A file past the bound of channels is refused when it opens, so
            // the head states no length for it (finding 10-1 of review round
            // 10).
            if channels == 0 || channels > u64::from(MOST_CHANNELS) {
                return HeadLength::UnstatedPcm;
            }
            let frames = u64::from(u32::from_be_bytes([comm[2], comm[3], comm[4], comm[5]]));
            let bits = u64::from(u16::from_be_bytes([comm[6], comm[7]]));
            let mut rate = [0; 10];
            rate.copy_from_slice(&comm[8..18]);
            let rate = extended_rate(&rate);
            if rate == 0 || frames == 0 {
                return HeadLength::UnstatedPcm;
            }
            // The compression type of an AIFC.  Another compressed type than
            // `ima4` states no frames that the head can count.
            if !plain {
                let Some(kind) = head.get(body + 18..body + 22) else {
                    return HeadLength::UnstatedPcm;
                };
                ima4 = kind.eq_ignore_ascii_case(b"ima4");
                if !ima4 && crate::hidden::is_other_aifc_type(kind) {
                    return HeadLength::UnstatedPcm;
                }
            }
            format = Some(if ima4 {
                (frames.saturating_mul(64), rate, channels * 34)
            } else {
                (frames, rate, channels * bits.div_ceil(8))
            });
        } else if id == b"SSND" {
            // The samples start after `offset` and `blockSize`.
            let offset = u64::from(be_u32_at(head, body).unwrap_or(0));
            let start = body as u64 + 8 + offset;
            let stated = u64::from(size).saturating_sub(8 + offset);
            held = Some(stated.min(file_bytes.saturating_sub(start)));
        }
        // A chunk's body has an even length: an odd one has a pad byte.
        let next = body as u64 + u64::from(size) + u64::from(size & 1);
        match usize::try_from(next) {
            Ok(next) if next < head.len() => at = next,
            _ => break,
        }
    }
    let Some((frames, rate, block)) = format else {
        return HeadLength::UnstatedPcm;
    };
    let frames = match held {
        Some(held) if plain && block > 0 => frames.min(held / block),
        Some(held) if ima4 && block > 0 => frames.min(held / block * 64),
        _ => frames,
    };
    HeadLength::Frames(song_frames(frames, rate))
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
    // The rate, the bytes of a block, and the frames of a block.
    let mut format: Option<(u32, u16, u64)> = None;
    let mut at = 12;
    while let (Some(id), Some(size)) = (head.get(at..at + 4), u32_at(head, at + 4)) {
        let body = at + 8;
        if id == b"fmt " {
            let rate = u32_at(head, body + 4);
            let block = u16_at(head, body + 12);
            let per_block = match (u16_at(head, body), u16_at(head, body + 2), block) {
                (Some(tag), Some(channels), Some(block)) => {
                    adpcm_frames_per_block(tag, channels, block)
                }
                _ => None,
            };
            format = rate
                .zip(block)
                .filter(|(rate, block)| *rate > 0 && *block > 0)
                .map(|(rate, block)| (rate, block, per_block.unwrap_or(1)));
        } else if id == b"data" {
            let Some((rate, block, per_block)) = format else {
                return HeadLength::UnstatedPcm;
            };
            if size == 0 || size == u32::MAX {
                return HeadLength::UnstatedPcm;
            }
            let held = file_bytes.saturating_sub(body as u64);
            let frames = u64::from(size).min(held) / u64::from(block) * per_block;
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

/// The frames of a block of an ADPCM WAV of format `tag`, `channels` and
/// `block` bytes, as Symphonia 0.6.1 counts them (`symphonia-format-riff`
/// `common.rs`, lines 231 to 255): Microsoft ADPCM (0x0002) has 7 bytes of
/// preamble for each channel and 2 frames in it; IMA ADPCM (0x0011) has 4
/// bytes and 1 frame.  Each other frame is 4 bits for each channel.  `None`
/// for another format (finding 9-3 of review round 9).
fn adpcm_frames_per_block(tag: u16, channels: u16, block: u16) -> Option<u64> {
    let (preamble, in_preamble) = match tag {
        0x0002 => (7, 2),
        0x0011 => (4, 1),
        _ => return None,
    };
    let channels = u64::from(channels).max(1);
    let data = u64::from(block).checked_sub(preamble * channels)?;
    Some(8 * data / (4 * channels) + in_preamble)
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
/// [`SONG_RATE`] whose head states `length`, with [`TAG_ROOM`].
/// [`HeadLength::Frames`]: two channels of 32-bit samples at 384 kHz, for
/// its stated frames, at most `max_frames`.  [`HeadLength::UnstatedPcm`]:
/// the same for `max_frames`.  [`HeadLength::Unstated`]: an MP3 at
/// 320 kbit/s for `max_frames`.  [`HeadLength::Video`]: [`TAG_ROOM`] only,
/// since a caller refuses a video first.  So a caller that reads a file
/// whole holds at most this many bytes of it (finding 1-3 of review round 1
/// of the catch-up plan).
pub fn largest_file(length: HeadLength, max_frames: usize) -> u64 {
    let (frames, per_second) = match length {
        HeadLength::Frames(frames) => (frames.min(max_frames as u64), PCM_BYTES_PER_SECOND),
        HeadLength::UnstatedPcm => (max_frames as u64, PCM_BYTES_PER_SECOND),
        HeadLength::Unstated => (max_frames as u64, MP3_BYTES_PER_SECOND),
        // A caller refuses a video before it asks; one that forgets
        // refuses any file larger than the room for tags.
        HeadLength::Video => (0, 0),
    };
    let bytes = (u128::from(frames) * u128::from(per_second)).div_ceil(u128::from(SONG_RATE));
    u64::try_from(bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(TAG_ROOM)
}

/// Decodes a file on disk: WAV, MP3, FLAC, M4A (AAC-LC or ALAC, also as
/// `.mp4`), raw AAC (ADTS) or AIFF.
pub fn decode_path(path: &Path) -> Result<Audio> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_owned);
    decode_source(Box::new(file), ext.as_deref())
        .with_context(|| format!("decoding {}", path.display()))
}

/// Decodes an in-memory file, for example one dropped into the web app.
/// The decode takes the format from the bytes, so `ext` (such as `"mp3"`)
/// is not used.
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
/// six times it.  An AAC is reserved for at most its length at 32 kbit/s,
/// in the same form.  A FLAC is reserved for at most `FLAC_MOST_RATIO`
/// times what its bytes hold as PCM at its stated bits: a constant subframe
/// holds a whole block in one sample, so its bytes give no real bound.  An
/// ALAC is reserved as a FLAC is, at the bits of byte 5 of its cookie,
/// since Symphonia states no bits for ALAC.  Every other codec here (WAV,
/// AIFF) stores each sample in `bits_per_coded_sample` bits, or in at
/// least a byte (A-law, mu-law and float WAVs do not state it).
fn most_frames(params: &AudioCodecParameters, byte_len: u64) -> u64 {
    if params.codec == CODEC_ID_AAC {
        // An unknown or impossible rate counts as the highest AAC has.
        let rate = params
            .sample_rate
            .filter(|r| (8_000..=96_000).contains(r))
            .map_or(96_000, u64::from);
        return byte_len.saturating_mul(rate) / AAC_LEAST_BYTES_PER_SECOND;
    }
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
    if params.codec == CODEC_ID_ALAC {
        // The `ALACSpecificConfig` cookie: its byte 5 is the bit depth.
        // ALAC has 16 to 32 bits; an unknown depth counts as 16.
        let bits = params
            .extra_data
            .as_deref()
            .filter(|cookie| cookie.len() >= 24)
            .map(|cookie| u64::from(cookie[5]))
            .filter(|b| (16..=32).contains(b))
            .unwrap_or(16);
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
    /// The packets that the open read for its checks, to decode first.
    queued: VecDeque<SymPacket>,
    track_id: u32,
    params: AudioCodecParameters,
    reserve: usize,
    trim: Trim,
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

/// Opens `source`.  It refuses a WAV, FLAC, M4A or AIFF whose stated
/// length passes `limit` bytes of samples.  An MP3 states only an estimate
/// without a Xing or VBRI tag: a quiet start can make it 10 times too long.
/// A raw AAC stream (ADTS) states only an estimate from its bitrate.  So an
/// MP3 or an ADTS stream is never refused here; the decoder stops it at the
/// limit.  The reservation of any file is within `limit`.  With
/// `max_song_frames`, it also refuses a file of a stated length that passes
/// that many frames at [`SONG_RATE`] ([`TooLong`]), and reserves no more
/// than that.
fn open_within(
    mut source: Box<dyn MediaSource>,
    _ext: Option<&str>,
    limit: u64,
    max_song_frames: Option<usize>,
) -> Result<Opened> {
    let is_seekable = source.is_seekable();
    let byte_len = source.byte_len();
    // `engine` finds the marker that Symphonia's probe would find, in the
    // same window after any leading ID3v2 tags, and opens that one reader
    // there.  So the reader starts where the plan and the walk start
    // (findings 5-2 and 5-3 of review round 5, 6-1 of review round 6).  A
    // source that cannot seek gets no plan.
    let (source, view, start) = if source.is_seekable() {
        let mut at = crate::mp4_boxes::Seeking(&mut *source);
        let (view, start) =
            crate::hidden::start_of(&mut at).context("reading the start of the file")?;
        (source, view, start)
    } else {
        let (source, start) = scan_stream(source).context("reading the start of the file")?;
        (source, 0, start)
    };
    let mut source = source;
    let Some((container, reader_at)) = start else {
        // Symphonia's ADTS reader refuses more than one raw data block in a
        // frame, so its score takes no such stream.  The scan takes one with
        // a CRC, which the decode splits; one with no CRC states no position
        // of its blocks (TD-86).
        let mut head = [0; crate::hidden::HEADER_OF_BLOCKS];
        if is_seekable {
            use crate::mp4_boxes::ReadAt;
            let read = crate::mp4_boxes::Seeking(&mut *source).read_at(view, &mut head)?;
            if crate::hidden::is_adts_of_raw_blocks(&head[..read]) {
                return Err(anyhow!(ADTS_BLOCKS_TEXT));
            }
        }
        return Err(anyhow!(
            "unsupported or unrecognised format: no known format at the start of the file"
        ));
    };
    let mut boxes = match container {
        Container::Mp4 if is_seekable => {
            let mut at = crate::mp4_boxes::Seeking(&mut *source);
            let found = crate::mp4_boxes::walk_from(&mut at, reader_at)
                .context("reading the boxes of an MP4 file")?;
            source.seek(SeekFrom::Start(0))?;
            found
        }
        _ => None,
    };
    if let Some(found) = &boxes {
        if found.video {
            return Err(anyhow!(VIDEO_TEXT));
        }
        if found.protected {
            return Err(anyhow!(PROTECTED_TEXT));
        }
        let to_end = match found.moov_held_to_end_from {
            Some(start) => byte_len.map(|len| len.saturating_sub(start)),
            None => Some(0),
        };
        let held = to_end.map(|to_end| found.moov_held + to_end);
        if held.is_some_and(|bytes| bytes > most_moov_bytes(found)) {
            return Err(anyhow!(MOOV_TEXT));
        }
    }
    // An MP4 behind other bytes: where its chunk offsets count from.
    let (view, late) = match &boxes {
        Some(found) => mp4_view(found, view, reader_at),
        None => (view, None),
    };
    // An AIFC of Apple's IMA ADPCM, which `engine` reads itself (finding 8-5
    // of review round 8).
    let mut ima4 = None;
    // MEM-4: Symphonia reads tags and pictures whole, so they are hidden
    // from it (`engine::hidden`).
    let plan = match &mut boxes {
        Some(found) => {
            let mut plan = std::mem::take(&mut found.hidden);
            plan.start = view;
            if let Some(size) = late {
                plan.free_after_return(size);
            }
            mend_mp4(found, byte_len, &mut plan);
            plan
        }
        None if is_seekable => {
            let mut at = crate::mp4_boxes::Seeking(&mut *source);
            if container == Container::Aiff {
                // Before any reader or decoder of Symphonia keeps a buffer
                // for each channel (finding 10-1 of review round 10).
                let channels = crate::hidden::aiff_channels(&mut at, reader_at)
                    .context("reading the start of the file")?;
                if let Some(channels) = channels.filter(|channels| *channels > MOST_CHANNELS) {
                    return Err(anyhow!(channels_text(u32::from(channels))));
                }
                ima4 = crate::ima4::ima4_of(&mut at, reader_at)
                    .context("reading the start of the file")?;
                let other = crate::hidden::is_aifc_of_other_type(&mut at, reader_at)
                    .context("reading the start of the file")?;
                if ima4.is_none() && other {
                    return Err(anyhow!(AIFC_COMPRESSED_TEXT));
                }
            }
            let plan = crate::hidden::plan_of(&mut at, view, reader_at, container)
                .context("reading the tags of the file")?;
            source.seek(SeekFrom::Start(0))?;
            plan
        }
        None => crate::hidden::Plan::default(),
    };
    if plan.too_much_kept {
        return Err(anyhow!(TAGS_TEXT));
    }
    let source: Box<dyn MediaSource> = if plan.is_empty() {
        source
    } else {
        Box::new(crate::hidden::Hidden::new(source, plan)?)
    };
    let byte_len = source.byte_len();
    // `start_of` gives a reader start at or after the view.
    let skip = reader_at
        .checked_sub(view)
        .ok_or_else(|| anyhow!("unsupported or unrecognised format: a start before the file"))?;
    let (source, skip): (Box<dyn MediaSource>, u64) = if container == Container::Adts {
        // Each ADTS frame of more raw data blocks and a CRC is split into a
        // frame for each block (finding 8-3 of review round 8, TD-86).
        use std::io::Read;
        let mut source = source;
        std::io::copy(&mut (&mut source).take(skip), &mut std::io::sink())
            .context("reading the start of the file")?;
        let left = byte_len.map(|len| len.saturating_sub(skip));
        (
            Box::new(crate::adts_split::SplitSource::new(source, left)),
            0,
        )
    } else {
        (source, skip)
    };
    let mut mss = MediaSourceStream::new(source, Default::default());
    {
        use symphonia::core::io::ReadBytes;
        mss.ignore_bytes(skip)
            .context("reading the start of the file")?;
    }
    let mut format: Box<dyn FormatReader> = match ima4 {
        Some(layout) => Box::new(
            crate::ima4::Ima4Reader::new(mss, layout, reader_at)
                .map_err(|e| anyhow!("unsupported or unrecognised format: {e}"))?,
        ),
        None => open_reader(mss, container)
            .map_err(|e| anyhow!("unsupported or unrecognised format: {e}"))?,
    };
    let estimated = container == Container::Adts;
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
    let num_frames = track.num_frames;
    if params.codec == CODEC_ID_AAC {
        check_aac(&params)?;
    }
    let channel_count = params.channels.as_ref().map_or(0, Channels::count);
    if channel_count > usize::from(MOST_CHANNELS) {
        return Err(anyhow!(channels_text(channel_count as u32)));
    }
    // The mend reads an AAC config; other codecs have other extra data
    // (finding 8-1 of review round 8).
    let params = if params.codec == CODEC_ID_AAC {
        with_pce_channels(params)
    } else {
        params
    };
    let queued = if params.codec == CODEC_ID_AAC {
        first_packets_of_lc(&mut *format, track_id)?
    } else {
        VecDeque::new()
    };
    // The walker keeps the first sound track.  Symphonia takes the first
    // track of a known codec.  When they differ, no trim.
    let trim = boxes
        .as_ref()
        .and_then(|found| {
            let sound = found.sound.as_ref()?;
            (sound.track_id == Some(track_id)).then(|| trim_of(found, sound, params.sample_rate))
        })
        .unwrap_or_default();
    let channels = params.channels.as_ref().map_or(1, Channels::count).max(1);
    let stated_exactly = params.codec != CODEC_ID_MP3 && !estimated;
    if stated_exactly && num_frames.is_some_and(|frames| passes(limit, channels, frames)) {
        return Err(too_long(limit));
    }
    let rate = params.sample_rate.filter(|rate| *rate > 0);
    let stated_song_frames = num_frames
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
    let stated = trim.keep.or(num_frames);
    let reserve = frames_to_reserve(stated, most.map(|m| m.min(within_limit)));
    Ok(Opened {
        format,
        queued,
        track_id,
        params,
        reserve,
        trim,
    })
}

/// Changes the view of an MP4 where Symphonia 0.6.1 refuses a valid file,
/// or reads a box whole that it does not need (review round 7).
///
/// * A last `mdat` past the end of the file gets a size of 0.
/// * A first `moov` of size 0 runs to the end of the file, but Symphonia
///   reads past the end of such a parent (`atoms/mod.rs`, line 640).  The
///   view gives it its size (TD-85).
/// * Symphonia refuses an `mvhd` timescale of 0 (`atoms/mvhd.rs`, lines 51
///   and 62), and uses it only for its `media_info` (`demuxer.rs`, line
///   338), which `engine` does not read.  The view gives the timescale of
///   the first sound track, or 1000 (TD-82).  The trim reads the file.
/// * Symphonia needs a first `ftyp` only to exist (`demuxer.rs`, line 243),
///   and reads each of its brands (`atoms/ftyp.rs`, line 43).  The view
///   shows one past `MOST_BOX_BYTES` as 16 bytes with no brands, then a
///   `free` box of the rest, with a 64-bit size (B5).
fn mend_mp4(found: &Found, byte_len: Option<u64>, plan: &mut crate::hidden::Plan) {
    // A last `mdat` that ends past the end of the file, as in a cut file:
    // Symphonia skips a top-level box that ends past the end of its stream
    // (`atoms/mod.rs`, line 671), and then finds no `mdat`.  The view gives
    // it a size of 0, to the end of the file (B of review round 8).
    if let (Some((start, end)), Some(len)) = (found.last_mdat, byte_len)
        && end != u64::MAX
        && end > len
    {
        plan.patch(start, &[0; 4]);
    }
    if let Some(moov) = &found.moov
        && moov.end.is_none()
        && let Some(size) =
            byte_len.and_then(|len| u32::try_from(len.checked_sub(moov.start)?).ok())
    {
        plan.patch(moov.start, &size.to_be_bytes());
    }
    if let Some(at) = found.zero_movie_timescale_at {
        let scale = found
            .sound
            .as_ref()
            .and_then(|sound| sound.media_timescale)
            .filter(|scale| *scale > 0)
            .unwrap_or(1000);
        plan.patch(at, &scale.to_be_bytes());
    }
    if let (Some(at), Some(bytes)) = (found.ftyp_at, found.ftyp_bytes)
        && bytes > crate::mp4_boxes::MOST_BOX_BYTES as u64
    {
        plan.patch(at, &16_u32.to_be_bytes());
        plan.patch(at + 20, b"free");
        if bytes == u64::MAX {
            // It runs to the end of the file, and so does the `free` box.
            plan.patch(at + 16, &[0; 4]);
        } else {
            let rest = (bytes - 16).to_be_bytes();
            plan.patch(at + 16, &1_u32.to_be_bytes());
            plan.patch(at + 24, &rest[..4]);
            plan.patch(at + 28, &rest[4..]);
        }
    }
}

/// Where Symphonia's view of an MP4 starts, when other bytes come before
/// it in the view that starts at `view`, and its reader starts at
/// `reader_at` (B1 of review round 7).  Symphonia takes each chunk offset
/// as a position in its stream.
///
/// * When the offsets count from the MP4 start, the view starts there.
/// * Else they count from `view`, and the view stays.  When `moov` is last,
///   Symphonia's MP4 reader walks the top-level boxes again from the start
///   of its stream (`demuxer.rs`, line 257), where the other bytes are not
///   a box.  So the view shows a `free` box from its start to the first
///   `mdat` or `moof`, after that seek only: the reader's first walk starts
///   at `reader_at`, inside that box.  It gives the size of that box.
fn mp4_view(found: &Found, view: u64, reader_at: u64) -> (u64, Option<u32>) {
    let before = reader_at.saturating_sub(view);
    if before == 0 {
        return (view, None);
    }
    let bodies: Vec<(u64, u64)> = found
        .mdats
        .iter()
        .map(|(body, end)| (body.saturating_sub(view), end.saturating_sub(view)))
        .collect();
    let first_chunk = found.sound.as_ref().and_then(|sound| sound.first_chunk);
    let from_mp4 = first_chunk.is_none_or(|chunk| counts_from_mp4_start(chunk, before, &bodies));
    if from_mp4 {
        return (reader_at, None);
    }
    let free = found
        .first_media
        .and_then(|media| u32::try_from(media.saturating_sub(view)).ok())
        .filter(|size| *size >= 8);
    (view, free)
}

/// Whether the chunk offsets of an MP4 that starts `before` bytes into its
/// view count from the MP4 start, by the first offset `chunk` and the
/// `mdat` bodies (from the start of the view).  The one form that puts the
/// offset in a body wins.  When both or neither do, the form that puts it
/// at the start of a body wins, and else the MP4 start, as after an ID3v2
/// tag.
fn counts_from_mp4_start(chunk: u64, before: u64, bodies: &[(u64, u64)]) -> bool {
    let from_mp4 = chunk.saturating_add(before);
    let inside = |at: u64| bodies.iter().any(|(body, end)| (*body..*end).contains(&at));
    let at_start = |at: u64| bodies.iter().any(|(body, _)| *body == at);
    match (inside(from_mp4), inside(chunk)) {
        (true, false) => true,
        (false, true) => false,
        _ => at_start(from_mp4) || !at_start(chunk),
    }
}

/// The packets of the audio track that the HE-AAC check reads.
const SBR_CHECK_PACKETS: usize = 8;

/// The most packets of any track that the HE-AAC check reads.
const MOST_CHECK_PACKETS: usize = 64;

/// Reads the first [`SBR_CHECK_PACKETS`] packets of the AAC track
/// `track_id`, and gives each packet that it read, to decode first.  It
/// refuses an HE-AAC stream whose config shows no SBR (TD-81, closed by
/// review round 7): each packet of the track that it read holds SBR data in
/// a fill element before its end (`aac_config::has_sbr_fill`).  Symphonia 0.6.1 finds that data too
/// (`aac/mod.rs`, line 194), but does not show it, and decodes the AAC-LC
/// core at half the band.
fn first_packets_of_lc(
    format: &mut dyn FormatReader,
    track_id: u32,
) -> Result<VecDeque<SymPacket>> {
    let mut queued = VecDeque::new();
    let mut audio = 0;
    let mut with_sbr = 0;
    while audio < SBR_CHECK_PACKETS && queued.len() < MOST_CHECK_PACKETS {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow!("reading packet: {e}")),
        };
        if packet.track_id == track_id {
            audio += 1;
            if crate::aac_config::has_sbr_fill(&packet.data) {
                with_sbr += 1;
            }
        }
        queued.push_back(packet);
    }
    if audio > 0 && with_sbr == audio {
        return Err(anyhow!(HE_AAC_TEXT));
    }
    Ok(queued)
}

/// `params` of an AAC-LC track whose config holds a program config element
/// of one front element: with the channel configuration of that element
/// (1 or 2) in a copy of the config (B6 of review round 7).  Symphonia
/// 0.6.1 refuses a program config element in the config
/// (`symphonia-common` `mpeg/audio/mod.rs`, line 289), and reads no bit
/// after the GASpecificConfig of AAC-LC.  Other `params` stay.
fn with_pce_channels(mut params: AudioCodecParameters) -> AudioCodecParameters {
    use symphonia::core::audio::layouts::{CHANNEL_LAYOUT_MONO, CHANNEL_LAYOUT_STEREO};
    let Some(asc) = params.extra_data.as_deref() else {
        return params;
    };
    let Some(config) = crate::aac_config::read(asc) else {
        return params;
    };
    let Some(channels) = config.pce_channels.filter(|_| config.channel_config == 0) else {
        return params;
    };
    let mended =
        crate::aac_config::with_channel_config(asc, config.channel_config_bit, channels as u8);
    params.extra_data = Some(mended.into_boxed_slice());
    params.channels = Some(if channels == 2 {
        CHANNEL_LAYOUT_STEREO
    } else {
        CHANNEL_LAYOUT_MONO
    });
    params
}

/// A source to open, and its container with where its reader starts.
type Scanned = (Box<dyn MediaSource>, Option<(Container, u64)>);

/// The container of a source that cannot seek, and where its reader starts,
/// as [`crate::hidden::start_of`] finds them with no ID3v2 tags.  It reads
/// the window of the probe and the reach of its scores, at most 1,056,773
/// bytes, into one buffer of that size.  It gives a source that reads those
/// bytes again, then the rest.  The open of a WAV of 4 MiB peaks at
/// 1,130,166 bytes, and the buffer stays until the decode ends (finding 7-8
/// of review round 7).
fn scan_stream(mut source: Box<dyn MediaSource>) -> std::io::Result<Scanned> {
    use crate::hidden::{LAST_MARKER, MOST_SCORE_BYTES, first_marker, reader_start};
    use std::io::Read;
    let most = LAST_MARKER + MOST_SCORE_BYTES;
    let mut window = Vec::with_capacity(most);
    (&mut source).take(most as u64).read_to_end(&mut window)?;
    let start = first_marker(&window, 0..LAST_MARKER + 1)
        .map(|(container, at)| (container, reader_start(container, at as u64)))
        .or_else(|| {
            crate::hidden::is_adts_of_raw_blocks_with_crc(&window).then_some((Container::Adts, 0))
        });
    let again = Cursor::new(window).chain(source);
    Ok((Box::new(ReadOnlySource::new(again)), start))
}

/// Opens the one Symphonia reader of `container` at the start of `mss`.
/// Each reader reads its marker from there (`try_new`), but for MP4: its
/// reader steps back 4 bytes from the box type that Symphonia's probe
/// found (`IsoMp4Reader::try_new`), so the decode reads the 4 bytes of the
/// first box size first.
fn open_reader(
    mut mss: MediaSourceStream<'static>,
    container: Container,
) -> symphonia::core::errors::Result<Box<dyn FormatReader>> {
    use symphonia::core::io::ReadBytes;
    use symphonia::default::formats::{
        AdtsReader, AiffReader, FlacReader, IsoMp4Reader, MpaReader, WavReader,
    };
    let opts = FormatOptions::default();
    Ok(match container {
        Container::Flac => Box::new(FlacReader::try_new(mss, opts)?),
        Container::Wav => Box::new(WavReader::try_new(mss, opts)?),
        Container::Aiff => Box::new(AiffReader::try_new(mss, opts)?),
        Container::Adts => Box::new(AdtsReader::try_new(mss, opts)?),
        Container::Mp3 => Box::new(MpaReader::try_new(mss, opts)?),
        Container::Mp4 => {
            mss.read_quad_bytes()?;
            Box::new(IsoMp4Reader::try_new(mss, opts)?)
        }
    })
}

/// Decodes a file one packet at a time, so a caller can keep each packet's
/// samples or pass them on.  It opens the file as `decode_bytes` does, and
/// keeps the limit of [`MAX_DECODED_BYTES`]: it stops with the same error
/// when the frames so far pass it.  It holds one packet's samples, not the
/// file's.
pub struct Decoder {
    format: Box<dyn FormatReader>,
    /// The packets that the open read for its checks, to decode first.
    queued: VecDeque<SymPacket>,
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
    /// The frames still to drop at the start (the encoder delay).
    skip: u64,
    /// The frames still to give, if the file states them; the rest is
    /// padding.
    keep: Option<u64>,
    planes: Vec<Vec<f32>>,
    /// The channels and the frames of a block of a Microsoft ADPCM track,
    /// whose steps the decode walks before Symphonia decodes each packet.
    ms_adpcm: Option<(usize, u64)>,
}

/// The samples of one packet.
pub struct Packet<'a> {
    /// The rate of the samples, in Hz.
    pub sample_rate: u32,
    /// One `Vec` of samples for each channel, all the same length.
    pub planes: &'a [Vec<f32>],
}

impl Decoder {
    /// Opens `source`.  The decode takes the format from the bytes, so `ext`
    /// (such as `"mp3"`) is not used.
    pub fn new(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Decoder> {
        Decoder::within(source, ext, MAX_DECODED_BYTES)
    }

    /// Opens `source` as [`Decoder::new`] does, for a song of at most
    /// `max_frames` frames at [`SONG_RATE`] (D3 of
    /// `plans/completed/stream-from-storage-catchup.md`).  It refuses a WAV or
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
            queued,
            track_id,
            params,
            reserve,
            trim,
        } = open_within(source, ext, limit, max_song_frames)?;
        let more_than_stereo = params.channels.as_ref().is_some_and(|c| c.count() > 2);
        let codec: Box<dyn AudioDecoder> =
            if params.codec == CODEC_ID_ADPCM_IMA_QT && more_than_stereo {
                Box::new(
                    crate::ima4::Ima4Channels::new(&params)
                        .map_err(|e| anyhow!("no decoder for codec: {e}"))?,
                )
            } else {
                symphonia::default::get_codecs()
                    .make_audio_decoder(&params, &AudioDecoderOptions::default())
                    .map_err(|e| anyhow!("no decoder for codec: {e}"))?
            };
        Ok(Decoder {
            format,
            queued,
            codec,
            track_id,
            reserve,
            limit,
            max_song_frames,
            sample_rate: params.sample_rate.unwrap_or(0),
            frames: 0,
            skip: trim.skip,
            keep: trim.keep,
            planes: Vec::new(),
            ms_adpcm: (params.codec == CODEC_ID_ADPCM_MS).then(|| {
                let channels = params.channels.as_ref().map_or(1, Channels::count);
                (channels, params.frames_per_block.unwrap_or(1))
            }),
        })
    }

    /// The samples of the next packet of the audio track, or `None` at the
    /// end of the file.  A corrupt packet is skipped, as players do.  An
    /// error comes before the frames so far pass the limit.  It drops the
    /// encoder delay and the padding that an M4A states (D7 of
    /// `plans/active/m4a.md`), so a packet can give fewer frames, and the
    /// limits count the frames given.
    pub fn next_packet(&mut self) -> Result<Option<Packet<'_>>> {
        loop {
            if self.keep == Some(0) {
                return Ok(None);
            }
            let next = match self.queued.pop_front() {
                Some(packet) => Ok(Some(packet)),
                None => self.format.next_packet(),
            };
            let packet = match next {
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
            if let Some((channels, per_block)) = self.ms_adpcm
                && !crate::adpcm::ms_steps_fit(
                    &packet.data,
                    channels,
                    per_block,
                    packet.block_dur().get(),
                )
            {
                return Err(anyhow!(MS_ADPCM_TEXT));
            }
            match self.codec.decode(&packet) {
                Ok(buf) => {
                    if self.sample_rate == 0 {
                        self.sample_rate = buf.spec().rate();
                    }
                    buf.copy_to_vecs_planar::<f32>(&mut self.planes);
                    let packet_frames = self.trim_packet();
                    if packet_frames == 0 {
                        continue;
                    }
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

    /// Drops the frames of the packet in `planes` that the trim does not
    /// give, and gives the frames left.
    fn trim_packet(&mut self) -> u64 {
        let decoded = self.planes.first().map_or(0, Vec::len) as u64;
        let dropped = self.skip.min(decoded);
        self.skip -= dropped;
        let mut given = decoded - dropped;
        if let Some(keep) = self.keep {
            given = given.min(keep);
            self.keep = Some(keep - given);
        }
        if given < decoded {
            for plane in &mut self.planes {
                plane.drain(..dropped as usize);
                plane.truncate(given as usize);
            }
        }
        given
    }

    /// The frames of each channel given so far.
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
        let audio = decode_bytes(bytes, ext);
        assert!(audio.is_ok(), "{ext:?}, {bits} bits: {audio:?}");
        let audio = audio.unwrap();
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
            "m4a" => {
                crate::test_m4a::alac_m4a(&tones[0], &tones[1], rate, None, m4a::Order::FastStart)
            }
            "aiff" => crate::test_audio::aiff(&tones, rate, 16, crate::test_audio::Aiff::Plain),
            "ima4" => {
                let whole = frames - frames % 64;
                let tones = [Vec::from(&tones[0][..whole]), Vec::from(&tones[1][..whole])];
                crate::test_audio::ima4_aifc(&tones, rate).0
            }
            _ => mp3(&tones, rate),
        }
    }

    /// Checks that a block decode of a minute of `ext` at 44.1 kHz and
    /// 48 kHz keeps its memory flat.
    fn block_decode_is_flat_for(ext: &str) {
        for rate in [44_100, 48_000] {
            let (short, long) = (file_of(ext, 2, rate), file_of(ext, 60, rate));
            assert_block_decode_is_flat(short, long, ext, &format!("{ext} at {rate} Hz"));
        }
    }

    /// A WAV.  Each other format has its own test, so the formats run in
    /// parallel: each takes 10 to 16 s in a test build.
    #[test]
    fn a_block_decode_keeps_its_memory_flat() {
        block_decode_is_flat_for("wav");
    }

    /// Finding 9-5 of review round 9: an `ima4` AIFC, whose reader gives
    /// packets of 64 blocks.
    #[test]
    fn a_block_decode_of_ima4_keeps_its_memory_flat() {
        for rate in [44_100, 48_000] {
            let (short, long) = (file_of("ima4", 2, rate), file_of("ima4", 60, rate));
            assert_block_decode_is_flat(short, long, "aiff", &format!("ima4 at {rate} Hz"));
        }
    }

    #[test]
    fn a_block_decode_of_flac_keeps_its_memory_flat() {
        block_decode_is_flat_for("flac");
    }

    #[test]
    fn a_block_decode_of_mp3_keeps_its_memory_flat() {
        block_decode_is_flat_for("mp3");
    }

    #[test]
    fn a_block_decode_of_alac_m4a_keeps_its_memory_flat() {
        block_decode_is_flat_for("m4a");
    }

    #[test]
    fn a_block_decode_of_aiff_keeps_its_memory_flat() {
        block_decode_is_flat_for("aiff");
    }

    /// Checks that a decode of `long` in blocks peaks within 1 MiB of one
    /// of `short`, and that a whole decode of `long` does not (the
    /// control).  The decode gives the frames of the whole decode.
    fn assert_block_decode_is_flat(short: Vec<u8>, long: Vec<u8>, ext: &str, case: &str) {
        const MIB: usize = 1 << 20;
        let whole = decode_bytes(long.clone(), Some(ext));
        assert!(whole.is_ok(), "{case}: {whole:?}");
        let whole = whole.unwrap();
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
        let rate = u64::from(whole.sample_rate);
        let want = (whole.frames() as u64 * 44_100).div_ceil(rate) as usize;
        assert_eq!(frames, want, "{case}: the frames of the long file");
        assert!(
            long_peak.abs_diff(short_peak) < MIB,
            "{case}: the short file peaks {short_peak} bytes, the long {long_peak} bytes"
        );
        // The control: a whole decode of the long file holds it.
        let (whole_short, _) = peak_of(|| decode_bytes(short, Some(ext)).unwrap());
        let (whole_long, _) = peak_of(|| decode_bytes(long, Some(ext)).unwrap());
        assert!(
            whole_long > whole_short + MIB,
            "{case}: the control peaks {whole_short} and {whole_long} bytes"
        );
    }

    /// Finding 1-20 of review round 1: a block decode of AAC keeps its
    /// memory flat too, in an M4A with an `elst` trim and as a raw ADTS
    /// stream.  An encode of AAC is slow, so the long file is 8 s, and
    /// each length is encoded once.
    #[test]
    fn a_block_decode_of_aac_keeps_its_memory_flat() {
        use crate::test_audio::tone;
        use crate::test_m4a::{ENCODER_DELAY, aac_m4a_of, adts, adts_packets, config};
        let files = |seconds: usize| {
            let frames = seconds * 44_100;
            let stream = adts(&[tone(0, frames, 44_100), tone(1, frames, 44_100)], 44_100);
            let edit = m4a::Edit {
                skip: ENCODER_DELAY as u32,
                frames: frames as u64,
            };
            let m4a = aac_m4a_of(
                &adts_packets(&stream),
                2,
                &config(2, 44_100, 2),
                44_100,
                Some(edit),
                None,
                m4a::Order::FastStart,
            );
            (m4a, stream)
        };
        let ((short_m4a, short_adts), (long_m4a, long_adts)) = (files(2), files(8));
        assert_block_decode_is_flat(short_m4a, long_m4a, "m4a", "an AAC M4A with an elst");
        assert_block_decode_is_flat(short_adts, long_adts, "aac", "a raw ADTS stream");
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
            Decoder::new(Box::new(failing), Some("wav")).expect("the open reads the start");
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

    /// D3 of the catch-up plan: the cap of a file.  A WAV or FLAC: 384 kHz,
    /// 32 bits, two channels, for its stated length or else for the limit.
    /// An MP3: 320 kbit/s for the limit.  Each with room for tags.
    #[test]
    fn the_largest_file_of_a_song_within_the_limit() {
        let hundred_seconds = 100 * 44_100;
        assert_eq!(TAG_ROOM, 16 << 20);
        assert_eq!(
            largest_file(HeadLength::UnstatedPcm, hundred_seconds),
            307_200_000 + TAG_ROOM
        );
        assert_eq!(
            largest_file(HeadLength::Unstated, hundred_seconds),
            4_000_000 + TAG_ROOM
        );
        // Part of a second rounds up.
        assert_eq!(largest_file(HeadLength::Unstated, 1), 1 + TAG_ROOM);
        // Finding 1-3 of review round 1: a file that states 10 s gets the
        // bytes of 10 s, and a stated length past the limit gets the limit.
        assert_eq!(
            largest_file(HeadLength::Frames(441_000), hundred_seconds),
            30_720_000 + TAG_ROOM
        );
        assert_eq!(
            largest_file(HeadLength::Frames(u64::MAX), hundred_seconds),
            307_200_000 + TAG_ROOM
        );
    }

    /// The 44-byte header `wav` with a chunk of `id` and `body` put before
    /// its `data` chunk, which starts at byte 36.
    fn with_chunk_before_data(wav: &[u8], id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut file = Vec::with_capacity(wav.len() + 9 + body.len());
        file.extend_from_slice(&wav[..36]);
        file.extend_from_slice(id);
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(body);
        if body.len() % 2 == 1 {
            file.push(0);
        }
        file.extend_from_slice(&wav[36..]);
        file
    }

    /// Review round 1 of the catch-up plan (missed mutants of
    /// `wav_length` and `HEAD_BYTES`): a WAV head skips a chunk of odd size
    /// and its pad byte, and a chunk of 2,000 bytes, before `data`.  A rate
    /// or a block align of 0 states no length.
    #[test]
    fn a_wav_head_skips_chunks_before_data_and_needs_a_format() {
        let (_, _, wav) = stereo_wav();
        let stated = HeadLength::Frames(FRAMES as u64);
        let odd = with_chunk_before_data(&wav, b"junk", &[1, 2, 3]);
        assert_eq!(head_length(&odd, odd.len() as u64), stated, "odd chunk");
        let tags = with_chunk_before_data(&wav, b"LIST", &[7; 2_000]);
        assert_eq!(HEAD_BYTES, 65_536);
        assert_eq!(
            head_length(&tags[..HEAD_BYTES.min(tags.len())], tags.len() as u64),
            stated,
            "2,000 bytes of tags"
        );
        let mut no_rate = wav.clone();
        set_u32(&mut no_rate, 24, 0);
        assert_eq!(
            head_length(&no_rate, no_rate.len() as u64),
            HeadLength::UnstatedPcm
        );
        let mut no_block = wav;
        no_block[32..34].copy_from_slice(&0_u16.to_le_bytes());
        assert_eq!(
            head_length(&no_block, no_block.len() as u64),
            HeadLength::UnstatedPcm
        );
    }

    /// Review round 1 of the catch-up plan (missed mutants of
    /// `flac_length` and `id3_size`): a STREAMINFO total of 2^32 + 5
    /// frames, and a FLAC after an ID3v2.4 tag with a footer.
    #[test]
    fn a_flac_head_states_a_36_bit_total_after_a_tag_with_a_footer() {
        let mut flac = silence_of(2, 8_192, 4_096, 2);
        // STREAMINFO starts at byte 8: the total's top 4 bits are the low
        // half of its byte 13, and its low 32 bits are bytes 14 to 17.
        flac[21] = (flac[21] & 0xF0) | 0x01;
        flac[22..26].copy_from_slice(&5_u32.to_be_bytes());
        assert_eq!(
            head_length(&flac, flac.len() as u64),
            HeadLength::Frames((1 << 32) + 5)
        );
        let stated = silence_of(2, 8_192, 4_096, 2);
        // Flags 0x10: a footer of 10 bytes follows the 128 bytes of tags.
        let mut tagged = vec![b'I', b'D', b'3', 4, 0, 0x10, 0, 0, 1, 0];
        tagged.extend_from_slice(&[0; 128]);
        tagged.extend_from_slice(&[b'3', b'D', b'I', 4, 0, 0x10, 0, 0, 1, 0]);
        tagged.extend_from_slice(&stated);
        assert_eq!(
            head_length(&tagged, tagged.len() as u64),
            HeadLength::Frames(8_192)
        );
    }

    /// Finding 1-12 of review round 1 of the catch-up plan: the decoder of
    /// `Decoder::within_song_frames` stops a decode of unstated length by
    /// itself, at the limit in frames at `SONG_RATE`, with no sink.
    #[test]
    fn the_decoder_stops_at_the_song_frame_limit() {
        let (left, right, _) = stereo_wav();
        for rate in [22_050, 44_100] {
            let mut streaming = encode_pcm16(&left, &right, rate);
            set_u32(&mut streaming, 4, u32::MAX);
            set_u32(&mut streaming, 40, u32::MAX);
            let max = song_frames(FRAMES as u64, rate) as usize;
            let open = |max: usize| {
                let source = Box::new(Cursor::new(streaming.clone()));
                Decoder::within_song_frames(source, Some("wav"), max).expect("no stated length")
            };
            assert_eq!(every_packet(open(max)).unwrap(), FRAMES as u64, "{rate} Hz");
            let error = every_packet(open(max - 1)).expect_err("past the limit");
            assert_eq!(
                error.downcast_ref::<TooLong>(),
                Some(&TooLong {
                    max_frames: max - 1
                }),
                "{rate} Hz"
            );
        }
    }

    /// Review round 1 of the catch-up plan (missed mutants of
    /// `source_frames_within` and `Decoder::song_frames_to_reserve`): a
    /// decoder reserves the song frames of its stated length, within its
    /// limit in frames at `SONG_RATE`.
    #[test]
    fn a_decoder_reserves_song_frames_within_its_limit() {
        use crate::test_audio::{mp3, tone};
        let ten_seconds = vec![0.25_f32; 441_000];
        let wav = encode_pcm16(&ten_seconds, &ten_seconds, 44_100);
        let decoder =
            Decoder::within_song_frames(Box::new(Cursor::new(wav)), Some("wav"), 4_410_000)
                .unwrap();
        assert_eq!(decoder.song_frames_to_reserve(), 441_000);
        // An MP3 of 1 s, with a limit of 2,000 song frames: 1,000 frames at
        // 22.05 kHz, and 2,176 at 48 kHz, each 2,000 at 44.1 kHz.
        for rate in [22_050, 48_000] {
            let frames = rate as usize;
            let bytes = mp3(&[tone(0, frames, rate), tone(1, frames, rate)], rate);
            let decoder =
                Decoder::within_song_frames(Box::new(Cursor::new(bytes)), Some("mp3"), 2_000)
                    .unwrap();
            assert_eq!(decoder.song_frames_to_reserve(), 2_000, "{rate} Hz");
        }
    }

    /// Review round 1 of the catch-up plan: the text of `TooLong`.
    #[test]
    fn too_long_says_the_limit() {
        assert_eq!(
            TooLong { max_frames: 4_410 }.to_string(),
            "the song is too long to open here: it passes 4410 frames at 44100 Hz"
        );
    }

    #[test]
    fn the_limit_is_1_gib_or_50_minutes_of_stereo() {
        assert_eq!(MAX_DECODED_BYTES, 1 << 30);
        let minutes = MAX_DECODED_BYTES as f64 / (8.0 * 44_100.0 * 60.0);
        assert!((minutes - 50.7).abs() < 0.05, "{minutes}");
    }

    /// The decoded channels of `bytes`, from `Decoder` one packet at a
    /// time.
    fn in_blocks(bytes: Vec<u8>, ext: &str) -> Result<Vec<Vec<f32>>> {
        let mut decoder = Decoder::new(Box::new(Cursor::new(bytes)), Some(ext))?;
        let mut channels: Vec<Vec<f32>> = Vec::new();
        while let Some(packet) = decoder.next_packet()? {
            channels.resize(packet.planes.len(), Vec::new());
            for (channel, plane) in channels.iter_mut().zip(packet.planes) {
                channel.extend_from_slice(plane);
            }
        }
        Ok(channels)
    }

    /// `channels` as the decoder gives 16-bit samples of them.
    fn decoded16(channels: &[&[f32]]) -> Vec<Vec<f32>> {
        use crate::test_m4a::as_decoded16;
        channels
            .iter()
            .map(|c| c.iter().map(|x| as_decoded16(*x)).collect())
            .collect()
    }

    /// M2 of the M4A plan: an ALAC M4A decodes bit for bit, with `moov`
    /// first or last, at 44.1 kHz and 48 kHz, whole and in blocks.
    #[test]
    fn an_alac_m4a_decodes_bit_for_bit() {
        use crate::test_m4a::alac_m4a;
        use m4a::Order;
        let (left, right, _) = stereo_wav();
        let want = decoded16(&[&left, &right]);
        for rate in [44_100, 48_000] {
            for order in [Order::FastStart, Order::MoovLast] {
                let case = format!("{rate} Hz, {order:?}");
                let file = alac_m4a(&left, &right, rate, None, order);
                let audio = decode_bytes(file.clone(), Some("m4a"));
                assert!(audio.is_ok(), "{case}: {audio:?}");
                let audio = audio.unwrap();
                assert_eq!(audio.sample_rate, rate, "{case}");
                assert!(audio.channels == want, "{case}: the samples differ");
                let blocks = in_blocks(file, "m4a");
                assert!(blocks.is_ok(), "{case}: {blocks:?}");
                assert!(blocks.unwrap() == want, "{case}: the blocks differ");
            }
        }
    }

    /// Two seconds of a stereo chirp at 44.1 kHz.
    fn chirps() -> Vec<Vec<f32>> {
        use crate::test_m4a::chirp;
        (0..2).map(|c| chirp(c, 88_200, 44_100)).collect()
    }

    /// The sum of the two channels of `channels`.
    fn sum(channels: &[Vec<f32>]) -> Vec<f32> {
        channels[0]
            .iter()
            .zip(&channels[1])
            .map(|(l, r)| l + r)
            .collect()
    }

    /// No container states the delay of a raw ADTS stream, so it decodes
    /// with it (TD-80).
    #[test]
    fn a_raw_adts_stream_decodes_with_its_delay() {
        use crate::test_m4a::{ENCODER_DELAY, aac_track_frames, adts, peak_lag};
        let source = chirps();
        let audio = decode_bytes(adts(&source, 44_100), Some("aac"));
        assert!(audio.is_ok(), "{audio:?}");
        let audio = audio.unwrap();
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.frames() as u64, aac_track_frames(88_200));
        let lag = peak_lag(&sum(&source), &sum(&audio.channels), 20_000, 10_000, 2048);
        assert_eq!(lag, ENCODER_DELAY as isize);
    }

    /// A raw ADTS stream states only an estimate of its length, as an MP3
    /// does: it opens past the limit, and its decode stops at the limit.
    /// An AAC M4A states its length exactly, so the same limit refuses it.
    #[test]
    fn a_raw_adts_stream_past_the_limit_opens_and_stops_at_the_limit() {
        use crate::test_m4a::{aac_m4a, adts};
        let source = chirps();
        let limit = 8 * 44_100;
        let opened = open_within(
            Box::new(Cursor::new(adts(&source, 44_100))),
            Some("aac"),
            limit,
            None,
        );
        assert!(opened.is_ok(), "{:?}", opened.err());
        let decoder = Decoder::within(
            Box::new(Cursor::new(adts(&source, 44_100))),
            Some("aac"),
            limit,
        );
        assert!(decoder.is_ok(), "{:?}", decoder.err());
        assert!(is_too_long(every_packet(decoder.unwrap())));
        let m4a = aac_m4a(&source, 44_100, None, None, m4a::Order::FastStart);
        let refused = open_within(Box::new(Cursor::new(m4a)), Some("m4a"), limit, None);
        assert!(is_too_long(refused.map(|_| ())));
    }

    /// An AIFF of 16 and 24 bits, and an AIFC of compression type `NONE`,
    /// decode exactly, as the FLAC tests do.
    #[test]
    fn an_aiff_decodes_exactly() {
        use crate::test_audio::{Aiff, aiff};
        let (left, right, _) = stereo_wav();
        let channels = [left.clone(), right.clone()];
        for (bits, kind) in [
            (16, Aiff::Plain),
            (24, Aiff::Plain),
            (16, Aiff::UncompressedAifc),
            (24, Aiff::UncompressedAifc),
        ] {
            let bytes = aiff(&channels, 44_100, bits, kind);
            for ext in ["aiff", "aif", "aifc"] {
                assert_decodes_as(bytes.clone(), Some(ext), &left, &right, bits);
            }
        }
    }

    /// An ALAC M4A that states more frames than the limit is refused
    /// before it decodes, from a file below 2 MB.
    #[test]
    fn an_alac_m4a_past_the_limit_is_refused_before_it_decodes() {
        use crate::test_m4a::silent_alac_m4a;
        let frames = MAX_DECODED_BYTES / 8 + 4096;
        let file = silent_alac_m4a(frames);
        assert!(file.len() < 2_000_000, "{} bytes", file.len());
        let opened = open_within(
            Box::new(Cursor::new(file)),
            Some("m4a"),
            MAX_DECODED_BYTES,
            None,
        );
        assert!(is_too_long(opened.map(|_| ())));
    }

    fn alac_at(bits: Option<u8>) -> AudioCodecParameters {
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(symphonia::core::codecs::audio::well_known::CODEC_ID_ALAC)
            .with_channels(Channels::Discrete(2));
        if let Some(bits) = bits {
            let mut cookie = [0; 24];
            cookie[5] = bits;
            params.with_extra_data(cookie.into());
        }
        params
    }

    /// An ALAC is reserved as a FLAC is: 16 times what its bytes hold as
    /// PCM at its bits, from byte 5 of its cookie.
    #[test]
    fn an_alac_is_reserved_for_16_times_its_pcm() {
        assert_eq!(most_frames(&alac_at(Some(16)), 1000), 4000);
        assert_eq!(most_frames(&alac_at(Some(24)), 1200), 3200);
        assert_eq!(most_frames(&alac_at(Some(32)), 1000), 2000);
        // A missing cookie, or bits outside 16 to 32, count as 16.
        for bits in [None, Some(0), Some(15), Some(33)] {
            assert_eq!(most_frames(&alac_at(bits), 1000), 4000, "{bits:?}");
        }
        let mut short = alac_at(None);
        short.with_extra_data(vec![0, 0, 0, 0, 0, 24].into());
        assert_eq!(most_frames(&short, 1000), 4000, "a short cookie");
    }

    fn aac_at(rate: Option<u32>) -> AudioCodecParameters {
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(symphonia::core::codecs::audio::well_known::CODEC_ID_AAC)
            .with_channels(Channels::Discrete(2));
        if let Some(rate) = rate {
            params.with_sample_rate(rate);
        }
        params
    }

    /// An AAC is reserved for at most its length at 32 kbit/s.
    #[test]
    fn an_aac_is_reserved_for_at_most_its_length_at_32_kbit_s() {
        // 4,000 bytes is a second at 32 kbit/s.
        assert_eq!(most_frames(&aac_at(Some(44_100)), 4_000), 44_100);
        assert_eq!(most_frames(&aac_at(Some(8_000)), 8_000), 16_000);
        assert_eq!(most_frames(&aac_at(Some(96_000)), 4_000), 96_000);
        // An unknown or impossible rate counts as 96 kHz.
        for rate in [None, Some(0), Some(7_999), Some(96_001)] {
            assert_eq!(most_frames(&aac_at(rate), 4_000), 96_000, "{rate:?}");
        }
    }

    /// An AIFF is reserved as the other codecs: its stated bits for each
    /// sample.
    #[test]
    fn an_aiff_is_reserved_for_the_frames_that_its_bytes_hold() {
        use crate::test_audio::{Aiff, aiff};
        let (left, right, _) = stereo_wav();
        for bits in [16, 24] {
            let bytes = aiff(&[left.clone(), right.clone()], 44_100, bits, Aiff::Plain);
            let byte_len = bytes.len();
            let opened = open_within(
                Box::new(Cursor::new(bytes)),
                Some("aiff"),
                MAX_DECODED_BYTES,
                None,
            );
            assert!(opened.is_ok(), "{bits} bits: {:?}", opened.err());
            let opened = opened.unwrap();
            assert_eq!(
                most_frames(&opened.params, byte_len as u64),
                (byte_len * 8 / (bits as usize * 2)) as u64,
                "{bits} bits"
            );
            assert_eq!(opened.reserve, FRAMES, "{bits} bits");
        }
    }

    /// The error text of `bytes` through `decode_bytes`, and through
    /// `Decoder::new`, which must be the same.
    fn refusal(bytes: Vec<u8>, ext: &str) -> String {
        let whole = decode_bytes(bytes.clone(), Some(ext));
        assert!(whole.is_err(), "{ext}: it decodes");
        let blocks = Decoder::new(Box::new(Cursor::new(bytes)), Some(ext));
        assert!(blocks.is_err(), "{ext}: the decoder opens");
        let (whole, blocks) = (whole.unwrap_err(), blocks.err().unwrap());
        assert_eq!(format!("{whole:#}"), format!("{blocks:#}"), "{ext}");
        format!("{blocks:#}")
    }

    fn short_tone() -> Vec<Vec<f32>> {
        use crate::test_m4a::chirp;
        (0..2).map(|c| chirp(c, 4096, 44_100)).collect()
    }

    fn alac_of(order: m4a::Order) -> Vec<u8> {
        let tone = short_tone();
        crate::test_m4a::alac_m4a(&tone[0], &tone[1], 44_100, None, order)
    }

    /// D4 of the M4A plan: a file with a video track gives the video text,
    /// with any extension, with `moov` first or last.
    #[test]
    fn a_file_with_video_gives_the_video_text() {
        use crate::test_m4a::{patched, with_second_trak};
        use m4a::Order;
        for order in [Order::FastStart, Order::MoovLast] {
            let only_video = patched(alac_of(order), b"soun", b"vide");
            for ext in ["mp4", "m4a"] {
                assert_eq!(
                    refusal(only_video.clone(), ext),
                    VIDEO_TEXT,
                    "{order:?}, {ext}"
                );
            }
        }
        let second = with_second_trak(alac_of(Order::MoovLast), b"vide", 2);
        assert_eq!(refusal(second, "mp4"), VIDEO_TEXT);
    }

    #[test]
    fn a_protected_file_gives_the_protected_text() {
        use crate::test_m4a::{aac_m4a, with_entry};
        use m4a::Order;
        for order in [Order::FastStart, Order::MoovLast] {
            let drms = with_entry(alac_of(order), b"drms");
            assert_eq!(refusal(drms, "m4a"), PROTECTED_TEXT, "{order:?}");
            let aac = aac_m4a(&short_tone(), 44_100, None, None, order);
            let enca = with_entry(aac, b"enca");
            assert_eq!(refusal(enca, "m4a"), PROTECTED_TEXT, "{order:?}");
        }
    }

    /// An AAC M4A of `config` with 3 packets of zeros, in an `mp4a` entry
    /// of `channels` at `rate`.
    fn dummy_aac(config: &[u8], channels: u16, rate: u32) -> Vec<u8> {
        let packet = [0_u8; 16];
        crate::test_m4a::aac_m4a_of(
            &[&packet[..]; 3],
            channels,
            config,
            rate,
            None,
            None,
            m4a::Order::FastStart,
        )
    }

    /// HE-AAC: an explicit object type 5, an SBR flag of 1 in the sync
    /// extension, and implicit SBR whose entry states the output rate.
    #[test]
    fn an_he_aac_file_gives_the_he_aac_text() {
        use crate::test_m4a::{aac_m4a_of, adts_packets, config, he_aac};
        let (stream, explicit, compatible) = he_aac(&chirps(), 44_100);
        let packets = adts_packets(&stream);
        for asc in [explicit, compatible] {
            let file = aac_m4a_of(&packets, 2, &asc, 44_100, None, None, m4a::Order::FastStart);
            assert_eq!(refusal(file, "m4a"), HE_AAC_TEXT, "{asc:02x?}");
        }
        let implicit = dummy_aac(&config(2, 22_050, 2), 2, 44_100);
        assert_eq!(refusal(implicit, "m4a"), HE_AAC_TEXT);
    }

    /// Symphonia 0.6.1 refuses 6 channels of AAC, so the decode says so.
    #[test]
    fn an_aac_of_more_than_2_channels_gives_its_channels() {
        use crate::test_m4a::{aac_m4a, adts, chirp};
        let six: Vec<Vec<f32>> = (0..6).map(|c| chirp(c, 4096, 44_100)).collect();
        let text = aac_channels_text(6);
        assert_eq!(
            text,
            "This AAC file has 6 channels.  Only mono and stereo AAC open."
        );
        let file = aac_m4a(&six, 44_100, None, None, m4a::Order::FastStart);
        assert_eq!(refusal(file, "m4a"), text);
        // A raw ADTS stream has no AudioSpecificConfig: its channels count.
        assert_eq!(refusal(adts(&six, 44_100), "aac"), text);
    }

    /// AAC Main, a program config element, 960 frames, and a config that
    /// does not parse.
    #[test]
    fn another_kind_of_aac_gives_the_aac_lc_text() {
        use crate::test_m4a::config;
        for (asc, case) in [
            (Vec::from(config(1, 44_100, 2)), "AAC Main"),
            (Vec::from(config(2, 44_100, 0)), "a program config element"),
            (vec![0x12, 0x14], "960 frames"),
            (vec![0xF8], "no parse"),
        ] {
            assert_eq!(
                refusal(dummy_aac(&asc, 2, 44_100), "m4a"),
                AAC_KIND_TEXT,
                "{case}"
            );
        }
    }

    /// The form of the maintainer's file: AAC-LC with the sync extension
    /// and an SBR flag of 0.  It decodes.
    #[test]
    fn an_aac_lc_file_with_an_sbr_flag_of_0_decodes() {
        use crate::test_m4a::{aac_m4a_of, aac_track_frames, adts, adts_packets, config};
        let source = chirps();
        let stream = adts(&source, 44_100);
        let mut asc = Vec::from(config(2, 44_100, 2));
        asc.extend_from_slice(&[0x56, 0xE5, 0x00]);
        let file = aac_m4a_of(
            &adts_packets(&stream),
            2,
            &asc,
            44_100,
            None,
            None,
            m4a::Order::FastStart,
        );
        let audio = decode_bytes(file, Some("m4a"));
        assert!(audio.is_ok(), "{audio:?}");
        assert_eq!(audio.unwrap().frames() as u64, aac_track_frames(88_200));
    }

    /// The frames of `channels` from `from`, `count` of them.
    fn frames_of(channels: &[Vec<f32>], from: usize, count: usize) -> Vec<Vec<f32>> {
        channels
            .iter()
            .map(|c| Vec::from(&c[from..from + count]))
            .collect()
    }

    /// A stereo ALAC M4A at 44.1 kHz of `delay` frames, then `source`, then
    /// `padding` frames, with an edit of `delay` and the source's frames.
    /// The delay and the padding are a chirp, not silence.
    fn alac_with_edit(
        source: &[Vec<f32>],
        delay: usize,
        padding: usize,
        order: m4a::Order,
    ) -> Vec<u8> {
        use crate::test_m4a::{alac_m4a, chirp};
        let frames = source[0].len();
        let around = chirp(1, delay + padding, 44_100);
        let input: Vec<Vec<f32>> = source
            .iter()
            .map(|c| [&around[..delay], c, &around[delay..]].concat())
            .collect();
        let edit = m4a::Edit {
            skip: delay as u32,
            frames: frames as u64,
        };
        alac_m4a(&input[0], &input[1], 44_100, Some(edit), order)
    }

    /// D7 of the M4A plan: an ALAC M4A with an `elst` delay of 2112 frames
    /// decodes to its source, bit for bit, whole and in blocks.
    #[test]
    fn an_alac_m4a_with_an_edit_decodes_to_its_source() {
        use m4a::Order;
        let source = short_tone();
        let want = decoded16(&[&source[0], &source[1]]);
        for order in [Order::FastStart, Order::MoovLast] {
            let file = alac_with_edit(&source, 2112, 700, order);
            let audio = decode_bytes(file.clone(), Some("m4a"));
            assert!(audio.is_ok(), "{order:?}: {audio:?}");
            let audio = audio.unwrap();
            assert_eq!(audio.frames(), 4096, "{order:?}");
            assert!(audio.channels == want, "{order:?}: the samples differ");
            let blocks = in_blocks(file, "m4a");
            assert!(blocks.is_ok(), "{order:?}: {blocks:?}");
            assert!(blocks.unwrap() == want, "{order:?}: the blocks differ");
        }
    }

    /// Checks that `file` decodes to the frames of `source`, whole and in
    /// blocks, and that its cross-correlation with it peaks at lag 0.
    fn assert_trimmed_to(file: Vec<u8>, source: &[Vec<f32>], case: &str) {
        use crate::test_m4a::peak_lag;
        let audio = decode_bytes(file.clone(), Some("m4a"));
        assert!(audio.is_ok(), "{case}: {audio:?}");
        let audio = audio.unwrap();
        assert_eq!(audio.frames(), source[0].len(), "{case}");
        let lag = peak_lag(&sum(source), &sum(&audio.channels), 20_000, 10_000, 4096);
        assert_eq!(lag, 0, "{case}");
        let blocks = in_blocks(file, "m4a");
        assert!(blocks.is_ok(), "{case}: {blocks:?}");
        assert!(
            blocks.unwrap() == audio.channels,
            "{case}: the blocks differ"
        );
    }

    /// D7 of the M4A plan: an AAC-LC M4A of Apple's delay, trimmed from
    /// `elst`, or from `iTunSMPB` alone, and one of the encoder's own
    /// delay (as the maintainer's file has) trimmed from `elst`.
    #[test]
    fn an_aac_m4a_decodes_to_the_length_of_its_source() {
        use crate::test_m4a::{
            APPLE_DELAY, ENCODER_DELAY, aac_m4a, aac_track_frames, with_apple_delay,
        };
        use m4a::{Edit, Gapless, Order};
        let source = chirps();
        let frames = 88_200;
        let delayed = with_apple_delay(&source);
        let track = aac_track_frames(frames + APPLE_DELAY - ENCODER_DELAY);
        let apple_edit = Edit {
            skip: APPLE_DELAY as u32,
            frames: frames as u64,
        };
        let gapless = Gapless {
            delay: APPLE_DELAY as u32,
            padding: (track - (APPLE_DELAY + frames) as u64) as u32,
            frames: frames as u64,
        };
        let own_edit = Edit {
            skip: ENCODER_DELAY as u32,
            frames: frames as u64,
        };
        for order in [Order::FastStart, Order::MoovLast] {
            let cases = [
                (
                    "elst of Apple's delay",
                    aac_m4a(&delayed, 44_100, Some(apple_edit), None, order),
                ),
                (
                    "iTunSMPB of Apple's delay",
                    aac_m4a(&delayed, 44_100, None, Some(gapless), order),
                ),
                (
                    "elst of the encoder's delay",
                    aac_m4a(&source, 44_100, Some(own_edit), None, order),
                ),
            ];
            for (case, file) in cases {
                assert_trimmed_to(file, &source, &format!("{case}, {order:?}"));
            }
        }
    }

    #[test]
    fn an_aac_m4a_with_no_edit_and_no_tag_keeps_every_frame() {
        use crate::test_m4a::{aac_m4a, aac_track_frames};
        let audio = decode_bytes(
            aac_m4a(&chirps(), 44_100, None, None, m4a::Order::FastStart),
            Some("m4a"),
        );
        assert!(audio.is_ok(), "{audio:?}");
        assert_eq!(audio.unwrap().frames() as u64, aac_track_frames(88_200));
    }

    /// D7 of the M4A plan: `elst` wins over an `iTunSMPB` that disagrees.
    #[test]
    fn an_elst_wins_over_an_itunsmpb_that_disagrees() {
        use crate::test_m4a::{APPLE_DELAY, ENCODER_DELAY, aac_m4a, aac_track_frames};
        use m4a::{Edit, Gapless};
        let source = chirps();
        let frames = 88_200;
        let track = aac_track_frames(frames);
        let edit = Edit {
            skip: ENCODER_DELAY as u32,
            frames: frames as u64,
        };
        let other = Gapless {
            delay: APPLE_DELAY as u32,
            padding: (track - (APPLE_DELAY + frames - 2000) as u64) as u32,
            frames: (frames - 2000) as u64,
        };
        let file = aac_m4a(
            &source,
            44_100,
            Some(edit),
            Some(other),
            m4a::Order::FastStart,
        );
        assert_trimmed_to(file, &source, "elst and iTunSMPB");
    }

    /// The decode of `file`, which must succeed.
    fn decoded(file: Vec<u8>, case: &str) -> Audio {
        let audio = decode_bytes(file, Some("m4a"));
        assert!(audio.is_ok(), "{case}: {audio:?}");
        audio.unwrap()
    }

    /// The maintainer's file states `segment_duration` in an `mvhd`
    /// timescale of 1000.  `keep` is its conversion to the nearest frame.
    #[test]
    fn a_movie_timescale_of_1000_gives_keep_to_the_nearest_frame() {
        use crate::test_m4a::with_movie_timescale;
        let source: Vec<Vec<f32>> = (0..2)
            .map(|c| crate::test_m4a::chirp(c, 100_000, 44_100))
            .collect();
        let want = decoded16(&[&source[0], &source[1]]);
        // 2000 ms is 88,200 frames; 2007 ms is 88,508.7, so 88,509.
        for (milliseconds, keep) in [(2000, 88_200), (2007, 88_509)] {
            let file = with_movie_timescale(
                alac_with_edit(&source, 2112, 0, m4a::Order::FastStart),
                1000,
                milliseconds,
            );
            let audio = decoded(file, &format!("{milliseconds} ms"));
            assert!(
                audio.channels == frames_of(&want, 0, keep),
                "{milliseconds} ms"
            );
        }
    }

    /// D7 of the M4A plan: `elst` gives a trim only in its common form.
    /// Two entries, a rate of 0.5, and a negative `media_time` other than
    /// -1 give none.  A leading empty edit is skipped.  A
    /// `segment_duration` of 0, or a movie timescale of 0, gives `skip`
    /// only.
    #[test]
    fn only_the_common_form_of_elst_gives_a_trim() {
        use crate::test_m4a::{with_elst, with_movie_timescale};
        const ONE: u32 = 0x1_0000;
        let source = short_tone();
        let input_frames = 2112 + 4096 + 700;
        let file = alac_with_edit(&source, 2112, 700, m4a::Order::MoovLast);
        let all = decoded(file.clone(), "the whole track").channels;
        assert_eq!(all[0].len(), 4096, "the edit of the writer");
        let want = decoded16(&[&source[0], &source[1]]);
        // Each case: its `elst` entries, and the frames of the whole track
        // that it gives (from, count).
        type Case<'a> = (&'a str, &'a [(u32, i32, u32)], usize, usize);
        let cases: [Case; 6] = [
            (
                "two entries",
                &[(4096, 2112, ONE), (100, 0, ONE)],
                0,
                input_frames,
            ),
            ("a rate of 0.5", &[(4096, 2112, ONE / 2)], 0, input_frames),
            ("a media_time of -2", &[(4096, -2, ONE)], 0, input_frames),
            (
                "a leading empty edit",
                &[(500, -1, ONE), (4096, 2112, ONE)],
                2112,
                4096,
            ),
            (
                "a segment_duration of 0",
                &[(0, 2112, ONE)],
                2112,
                input_frames - 2112,
            ),
            (
                "a keep past the end",
                &[(9000, 2112, ONE)],
                2112,
                input_frames - 2112,
            ),
        ];
        let whole = decoded(
            with_elst(file.clone(), &[(input_frames as u32, 0, ONE)]),
            "no trim",
        )
        .channels;
        assert_eq!(whole[0].len(), input_frames);
        assert!(frames_of(&whole, 2112, 4096) == want);
        for (case, entries, from, count) in cases {
            let audio = decoded(with_elst(file.clone(), entries), case);
            assert!(
                audio.channels == frames_of(&whole, from, count),
                "{case}: {} frames",
                audio.frames()
            );
        }
        // Symphonia 0.6.1 refuses a movie timescale of 0, so the view gives
        // `mvhd` the media timescale (TD-82, closed by review round 7).  The
        // trim reads the file, so it is `skip` only.
        let no_movie_scale = decoded(with_movie_timescale(file.clone(), 0, 4096), "scale 0");
        let skip_only = frames_of(&whole, 2112, input_frames - 2112);
        assert!(
            no_movie_scale.channels == skip_only,
            "a movie timescale of 0"
        );
        let past = decode_bytes(
            with_elst(file, &[(10, input_frames as i32 + 10, ONE)]),
            Some("m4a"),
        );
        assert!(
            past.as_ref()
                .is_err_and(|e| format!("{e:#}").contains("file decoded to no audio")),
            "a skip past the end: {past:?}"
        );
    }

    /// A movie timescale of 0 gives `skip` with no `keep`.  The decode of
    /// such a file is in `only_the_common_form_of_elst_gives_a_trim`.
    #[test]
    fn trim_of_a_movie_timescale_of_0_is_skip_only() {
        use crate::mp4_boxes::EditEntry;
        let sound = SoundTrack {
            track_id: Some(1),
            entry: Some(*b"alac"),
            media_timescale: Some(44_100),
            duration: Some(10_000),
            edits: Some(vec![EditEntry {
                segment_duration: 4096,
                media_time: 2112,
                rate: EDIT_RATE_ONE,
            }]),
            first_chunk: None,
        };
        let found = |movie_timescale| Found {
            movie_timescale,
            sound: Some(sound.clone()),
            ..Found::default()
        };
        let skip_only = Trim {
            skip: 2112,
            keep: None,
        };
        assert_eq!(trim_of(&found(Some(0)), &sound, Some(44_100)), skip_only);
        assert_eq!(trim_of(&found(None), &sound, Some(44_100)), skip_only);
        assert_eq!(
            trim_of(&found(Some(44_100)), &sound, Some(44_100)),
            Trim {
                skip: 2112,
                keep: Some(4096)
            }
        );
        // A media timescale other than the rate: no trim from `elst`.
        assert_eq!(
            trim_of(&found(Some(44_100)), &sound, Some(48_000)),
            Trim::default()
        );
    }

    /// With a form of `elst` that gives no trim, `iTunSMPB` gives it.
    #[test]
    fn an_itunsmpb_trims_when_the_elst_gives_no_trim() {
        use crate::test_m4a::{ENCODER_DELAY, aac_m4a, aac_track_frames, with_elst};
        use m4a::{Edit, Gapless};
        let source = chirps();
        let frames = 88_200;
        let edit = Edit {
            skip: ENCODER_DELAY as u32,
            frames: frames as u64,
        };
        let gapless = Gapless {
            delay: ENCODER_DELAY as u32,
            padding: (aac_track_frames(frames) - (ENCODER_DELAY + frames) as u64) as u32,
            frames: frames as u64,
        };
        let file = aac_m4a(
            &source,
            44_100,
            Some(edit),
            Some(gapless),
            m4a::Order::MoovLast,
        );
        let two = with_elst(file, &[(88_200, 1024, 0x1_0000), (100, 0, 0x1_0000)]);
        assert_trimmed_to(two, &source, "two entries and iTunSMPB");
    }

    /// A malformed `iTunSMPB` is ignored.  A field 4 of 0 takes the
    /// frames from the `mdhd` duration, less the delay and the padding.
    #[test]
    fn a_malformed_itunsmpb_is_ignored() {
        use crate::test_m4a::{ENCODER_DELAY, aac_m4a, aac_track_frames, with_itunsmpb_text};
        use m4a::Gapless;
        let source = chirps();
        let frames = 88_200;
        let track = aac_track_frames(frames);
        let gapless = Gapless {
            delay: ENCODER_DELAY as u32,
            padding: (track - (ENCODER_DELAY + frames) as u64) as u32,
            frames: frames as u64,
        };
        let file = aac_m4a(&source, 44_100, None, Some(gapless), m4a::Order::FastStart);
        let not_hex = with_itunsmpb_text(file.clone(), |text| text[12] = b'G');
        assert_eq!(decoded(not_hex, "not hexadecimal").frames() as u64, track);
        let few = with_itunsmpb_text(file.clone(), |text| text[18..].fill(b' '));
        assert_eq!(decoded(few, "too few fields").frames() as u64, track);
        let no_frames = with_itunsmpb_text(file, |text| text[28..44].fill(b'0'));
        assert_trimmed_to(no_frames, &source, "a field 4 of 0");
    }

    /// When Symphonia takes another track than the walker's first sound
    /// track, the decode does no trim: it gives every frame of its track.
    #[test]
    fn no_trim_when_symphonia_takes_another_track() {
        use crate::test_m4a::{with_first_entry, with_second_trak};
        let source = short_tone();
        let file = alac_with_edit(&source, 2112, 700, m4a::Order::MoovLast);
        let two = with_first_entry(with_second_trak(file, b"soun", 7), b"lloy");
        let audio = decoded(two, "the second track");
        assert_eq!(audio.frames(), 2112 + 4096 + 700);
    }

    /// The head of `file`: at most `HEAD_BYTES` of it.
    /// An `ftyp` box, the marker that Symphonia's probe takes for MP4.
    fn ftyp() -> Vec<u8> {
        crate::test_m4a::bx(b"ftyp", &[b"M4A ", &[0; 4]])
    }

    fn head_of(file: &[u8]) -> HeadLength {
        head_length(&file[..HEAD_BYTES.min(file.len())], file.len() as u64)
    }

    /// `file`, an M4A with `moov` first, with a `free` box of `size` bytes
    /// put in `moov` before its `trak`.  The packets move, so the file is
    /// for the head check only.
    fn with_free_before_trak(file: &[u8], size: usize) -> Vec<u8> {
        use crate::test_m4a::{child_box, top_box};
        let (moov, moov_size) = top_box(file, b"moov");
        let (trak, _) = child_box(file, moov, b"trak");
        let mut free = Vec::from((size as u32).to_be_bytes());
        free.extend_from_slice(b"free");
        free.resize(size, 0);
        let mut out = Vec::from(&file[..trak]);
        out.extend_from_slice(&free);
        out.extend_from_slice(&file[trak..]);
        out[moov..moov + 4].copy_from_slice(&((moov_size + size) as u32).to_be_bytes());
        out
    }

    /// An `ftyp`, then an `mdat` of 100 bytes with the size `size_field`
    /// (and a 64-bit size when it is 1), then the `moov` of `file`.
    fn with_mdat_first(file: &[u8], size_field: u32) -> Vec<u8> {
        use crate::test_m4a::top_box;
        let (ftyp, ftyp_size) = top_box(file, b"ftyp");
        let (moov, moov_size) = top_box(file, b"moov");
        let mut out = Vec::from(&file[ftyp..ftyp + ftyp_size]);
        out.extend_from_slice(&size_field.to_be_bytes());
        out.extend_from_slice(b"mdat");
        if size_field == 1 {
            out.extend_from_slice(&(16_u64 + 100).to_be_bytes());
        }
        out.resize(out.len() + 100, 7);
        out.extend_from_slice(&file[moov..moov + moov_size]);
        out
    }

    /// M2 of the M4A plan: the head of an M4A states the length of its
    /// first sound track from its `mdhd`, if the head shows it.
    #[test]
    fn a_head_states_the_length_of_an_m4a() {
        use crate::test_m4a::{
            aac_m4a, aac_track_frames, patched, silent_alac_m4a, version_1, with_second_trak,
        };
        use m4a::Order;
        let tone = short_tone();
        for rate in [44_100, 48_000] {
            let file = crate::test_m4a::alac_m4a(&tone[0], &tone[1], rate, None, Order::FastStart);
            let want = HeadLength::Frames(song_frames(4096, rate));
            assert_eq!(head_of(&file), want, "{rate} Hz");
            assert_eq!(
                head_of(&with_mdat_first(&file, 1)),
                want,
                "a 64-bit mdat first, {rate} Hz"
            );
            assert_eq!(
                head_of(&with_mdat_first(&file, 0)),
                HeadLength::UnstatedPcm,
                "an mdat of size 0 first"
            );
        }
        // A moov after an mdat that passes the head.
        let two_seconds = chirps();
        let moov_last = crate::test_m4a::alac_m4a(
            &two_seconds[0],
            &two_seconds[1],
            44_100,
            None,
            Order::MoovLast,
        );
        assert!(crate::test_m4a::top_box(&moov_last, b"moov").0 > HEAD_BYTES);
        assert_eq!(head_of(&moov_last), HeadLength::UnstatedPcm, "moov last");
        // A moov that ends after the head: its mdhd is near its start.
        let frames = MAX_DECODED_BYTES / 8 + 4096;
        let long = silent_alac_m4a(frames);
        assert!(crate::test_m4a::top_box(&long, b"moov").1 > HEAD_BYTES);
        assert_eq!(head_of(&long), HeadLength::Frames(frames));
        let behind_free = with_free_before_trak(&alac_of(Order::FastStart), 70 * 1024);
        assert_eq!(
            head_of(&behind_free),
            HeadLength::UnstatedPcm,
            "a free box of 70 KiB"
        );
        assert_eq!(
            head_of(&[ftyp(), version_1()].concat()),
            HeadLength::Frames(song_frames(3 << 32, 48_000))
        );
        let aac = aac_m4a(&short_tone(), 44_100, None, None, Order::FastStart);
        assert_eq!(
            head_of(&aac),
            HeadLength::Frames(aac_track_frames(4096)),
            "AAC, with its delay"
        );
        let only_video = patched(alac_of(Order::FastStart), b"soun", b"vide");
        assert_eq!(head_of(&only_video), HeadLength::Video);
        let second = with_second_trak(alac_of(Order::MoovLast), b"vide", 2);
        assert_eq!(head_of(&second), HeadLength::Video);
    }

    /// A raw ADTS stream states no length, also after an ID3 tag; an MP3
    /// stays `Unstated`.
    #[test]
    fn a_head_of_a_raw_adts_stream_states_no_length() {
        use crate::test_audio::{mp3, tone};
        let adts = crate::test_m4a::adts(&short_tone(), 44_100);
        assert_eq!(head_of(&adts), HeadLength::UnstatedPcm);
        let mut tagged = vec![b'I', b'D', b'3', 4, 0, 0, 0, 0, 1, 0];
        tagged.extend_from_slice(&[0; 128]);
        tagged.extend_from_slice(&adts);
        assert_eq!(head_of(&tagged), HeadLength::UnstatedPcm);
        let mp3 = mp3(&[tone(0, 44_100, 44_100), tone(1, 44_100, 44_100)], 44_100);
        assert_eq!(head_of(&mp3), HeadLength::Unstated);
    }

    /// An AIFF or AIFC states its frames and rate in `COMM`.  When `SSND`
    /// is in the head, an AIFF counts at most the frames that the file
    /// holds.
    #[test]
    fn a_head_states_the_length_of_an_aiff() {
        use crate::test_audio::{Aiff, aiff};
        let (left, right, _) = stereo_wav();
        let channels = [left, right];
        for rate in [22_050, 44_100, 48_000] {
            let want = HeadLength::Frames(song_frames(FRAMES as u64, rate));
            for (bits, kind) in [
                (16, Aiff::Plain),
                (24, Aiff::Plain),
                (16, Aiff::UncompressedAifc),
            ] {
                let file = aiff(&channels, rate, bits, kind);
                assert_eq!(head_of(&file), want, "{bits} bits, {kind:?}, {rate} Hz");
            }
        }
        let file = aiff(&channels, 44_100, 16, Aiff::Plain);
        // A file that holds 100 frames of what it states.
        let short = file.len() - (FRAMES - 100) * 4;
        assert_eq!(
            head_length(&file[..short], short as u64),
            HeadLength::Frames(100)
        );
        // A COMM after a chunk of 70 KiB: past the head.
        let mut late = Vec::from(&file[..12]);
        late.extend_from_slice(b"APPL");
        late.extend_from_slice(&(70_u32 * 1024).to_be_bytes());
        late.resize(late.len() + 70 * 1024, 0);
        late.extend_from_slice(&file[12..]);
        assert_eq!(head_of(&late), HeadLength::UnstatedPcm);
        // A rate of 0 states no length.
        let mut no_rate = file.clone();
        let comm = crate::test_m4a::place_once(&no_rate, b"COMM") + 8;
        no_rate[comm + 8..comm + 18].fill(0);
        assert_eq!(head_of(&no_rate), HeadLength::UnstatedPcm);
    }

    /// The head check never panics, on any prefix of each fixture.
    #[test]
    fn every_prefix_of_a_head_gives_a_length() {
        use crate::test_audio::{Aiff, aiff};
        use crate::test_m4a::{aac_m4a, version_1};
        let (left, right, _) = stereo_wav();
        let fixtures = [
            alac_of(m4a::Order::FastStart),
            aac_m4a(&short_tone(), 44_100, None, None, m4a::Order::FastStart),
            [ftyp(), version_1()].concat(),
            aiff(
                &[Vec::from(&left[..500]), Vec::from(&right[..500])],
                44_100,
                16,
                Aiff::UncompressedAifc,
            ),
            crate::test_m4a::adts(&short_tone(), 44_100),
        ];
        for file in fixtures {
            for end in 0..=file.len() {
                let _ = head_length(&file[..end], file.len() as u64);
            }
        }
    }

    /// A caller refuses a video before it asks for the largest file; one
    /// that forgets gets the room for tags only.
    #[test]
    fn the_largest_file_of_a_video_is_the_room_for_tags() {
        assert_eq!(largest_file(HeadLength::Video, 100 * 44_100), TAG_ROOM);
    }

    /// The peak of a block decode of `bytes`, and its frames.
    fn block_peak(bytes: Vec<u8>, ext: &str) -> (usize, Result<usize>) {
        peak_of(|| {
            let decoder = Decoder::new(Box::new(Cursor::new(bytes)), Some(ext))?;
            let mut counter = Counter(0);
            crate::song::decode_into(decoder, &mut counter)?;
            Ok(counter.0)
        })
    }

    /// Checks that `tagged` decodes bit for bit to `plain`, and that its
    /// block decode peaks within 1 MiB of that of `plain`: Symphonia does
    /// not read its tags.
    fn assert_tags_are_hidden(plain: Vec<u8>, tagged: Vec<u8>, ext: &str, case: &str) {
        let want = decode_bytes(plain.clone(), Some(ext));
        assert!(want.is_ok(), "{case}: {want:?}");
        let got = decode_bytes(tagged.clone(), Some(ext));
        assert!(got.is_ok(), "{case}: {got:?}");
        assert!(
            got.unwrap().channels == want.unwrap().channels,
            "{case}: the samples differ"
        );
        let (plain_peak, plain_frames) = block_peak(plain, ext);
        let (tagged_peak, tagged_frames) = block_peak(tagged, ext);
        assert!(tagged_frames.is_ok(), "{case}: {tagged_frames:?}");
        assert_eq!(tagged_frames.ok(), plain_frames.ok(), "{case}: the frames");
        assert!(
            tagged_peak < plain_peak + (1 << 20),
            "{case}: the plain file peaks at {plain_peak} bytes, the file with tags at {tagged_peak}"
        );
    }

    /// The art of round 2 fixes: 64 MiB, which Symphonia would hold about
    /// twice.
    const ART: usize = 64 << 20;

    /// Findings 2-1, 2-2 and 2-9 of review round 2 (MEM-4), by the
    /// maintainer's design: Symphonia reads each `moov/udta`, each
    /// top-level `meta` and each `moov` whole.  The decode hides them, so
    /// an M4A with 64 MiB of art opens at a flat peak, bit for bit.
    #[test]
    fn the_art_of_an_m4a_is_hidden_from_symphonia() {
        use crate::test_m4a::{bx, cover_meta, top_box, with_cover};
        let plain = alac_of(m4a::Order::MoovLast);
        let in_udta = with_cover(plain.clone(), ART);
        assert_tags_are_hidden(plain.clone(), in_udta.clone(), "m4a", "art in moov/udta");
        // Finding 2-9: a `moov` of size 0 runs to the end of the file.
        // Symphonia 0.6.1 reads past its end (`atoms/mod.rs`, line 640), so
        // the view gives it its size (TD-85, closed by review round 7).
        // ALAC and AAC open bit for bit, with the art hidden.
        let size_0 = |mut file: Vec<u8>| {
            let (moov, _) = top_box(&file, b"moov");
            file[moov..moov + 4].copy_from_slice(&[0; 4]);
            file
        };
        let to_the_end = size_0(in_udta);
        assert_tags_are_hidden(
            plain.clone(),
            to_the_end,
            "m4a",
            "a moov of size 0 with art",
        );
        let aac = crate::test_m4a::aac_m4a(&short_tone(), 44_100, None, None, m4a::Order::MoovLast);
        assert_tags_are_hidden(aac.clone(), size_0(aac), "m4a", "an AAC moov of size 0");
        let mut in_meta = plain.clone();
        in_meta.extend_from_slice(&cover_meta(ART));
        assert_tags_are_hidden(plain.clone(), in_meta, "m4a", "art in a top-level meta");
        let mut second = plain.clone();
        second.extend_from_slice(&bx(b"moov", &[&bx(b"udta", &[&cover_meta(ART)])]));
        assert_tags_are_hidden(plain, second, "m4a", "art in a second moov");
    }

    /// The maintainer's design of review round 2 (MEM-4): Symphonia reads a
    /// FLAC PICTURE, VORBIS_COMMENT and APPLICATION block whole.  The
    /// decode shows each as PADDING, so a FLAC with 64 MiB of art opens at a
    /// flat peak, bit for bit.
    #[test]
    fn the_tags_of_a_flac_are_hidden_from_symphonia() {
        use crate::test_audio::with_flac_tags;
        let (left, right, _) = stereo_wav();
        let plain = compressed(&left, &right, 44_100);
        let tagged = with_flac_tags(&plain, ART);
        assert_tags_are_hidden(plain.clone(), tagged, "flac", "a FLAC with art");
        let small = with_flac_tags(&plain, 100);
        assert_tags_are_hidden(plain, small, "flac", "a FLAC with small tags");
    }

    /// The maintainer's design of review round 2 (MEM-4), and finding 2-8:
    /// Symphonia's probe gives up past 1 MiB of a leading ID3v2 tag, since
    /// `engine`'s Symphonia has no ID3v2 reader.  The decode starts
    /// Symphonia's view after the tags, so an MP3, a raw AAC or a FLAC with
    /// a tag of 2 MiB or 64 MiB opens at a flat peak, bit for bit.
    #[test]
    fn a_leading_id3_tag_is_hidden_from_symphonia() {
        use crate::test_audio::{id3_tag, mp3, tone};
        let tones = [tone(0, 44_100, 44_100), tone(1, 44_100, 44_100)];
        let (left, right, _) = stereo_wav();
        let files = [
            ("mp3", mp3(&tones, 44_100)),
            ("aac", crate::test_m4a::adts(&short_tone(), 44_100)),
            ("flac", compressed(&left, &right, 44_100)),
        ];
        for (ext, plain) in files {
            for bytes in [2 << 20, ART] {
                let tagged = [id3_tag(bytes), plain.clone()].concat();
                let case = format!("{ext} after a tag of {bytes} bytes");
                assert_tags_are_hidden(plain.clone(), tagged, ext, &case);
            }
            // Two tags, one after the other.
            let two = [id3_tag(100), id3_tag(2 << 20), plain.clone()].concat();
            assert_tags_are_hidden(plain, two, ext, &format!("{ext} after two tags"));
        }
    }

    /// The maintainer's design of review round 2 (MEM-4): Symphonia reads a
    /// WAV `LIST` chunk, and AIFF `APPL`, `MARK`, `COMT`, text and `ID3 `
    /// chunks, whole.  The decode shows each as `JUNK`, which Symphonia
    /// skips, so a file with 64 MiB of tags opens at a flat peak, bit for
    /// bit.
    #[test]
    fn the_tags_of_a_wav_and_an_aiff_are_hidden_from_symphonia() {
        use crate::test_audio::{Aiff, aiff, with_aiff_tags, with_wav_tags};
        let (left, right, wav) = stereo_wav();
        let channels = [left, right];
        assert_tags_are_hidden(wav.clone(), with_wav_tags(&wav, ART), "wav", "a WAV");
        assert_tags_are_hidden(wav.clone(), with_wav_tags(&wav, 101), "wav", "small tags");
        for kind in [Aiff::Plain, Aiff::UncompressedAifc] {
            let plain = aiff(&channels, 44_100, 16, kind);
            let case = format!("{kind:?}");
            assert_tags_are_hidden(plain.clone(), with_aiff_tags(&plain, ART), "aiff", &case);
        }
    }

    /// Checks that `bytes` is refused with `text` before Symphonia opens it:
    /// through
    /// `decode_bytes` and `Decoder`, and at a small peak.
    fn assert_refused_early(bytes: Vec<u8>, ext: &str, text: &str, case: &str) {
        assert_eq!(refusal(bytes.clone(), ext), text, "{case}");
        let (peak, opened) = peak_of(|| {
            open_within(
                Box::new(Cursor::new(bytes)),
                Some(ext),
                MAX_DECODED_BYTES,
                None,
            )
        });
        let error = opened.err().map(|e| format!("{e:#}"));
        assert_eq!(error.as_deref(), Some(text), "{case}");
        assert!(peak < 1 << 20, "{case}: the open peaks at {peak} bytes");
    }

    /// Finding 4-1 of review round 4 (MEM-4), by the maintainer's rule: a
    /// file is not refused for its count of tags.  A plan hides
    /// `MOST_SMALL` small containers, and Symphonia keeps the rest.  A file
    /// opens while their `kept_charge` is at most `MOST_KEPT_BYTES`; one
    /// more is refused before Symphonia opens it, at a small peak.  A file of 5000
    /// small tags opens.
    ///
    /// Findings 5-1 and 5-5 of review round 5: the charge holds for the
    /// densest containers too.  At the limit, the decode of each form peaks
    /// at most `MOST_KEPT_BYTES` above the plain file, with 256 KiB for the
    /// plan.
    #[test]
    fn a_file_is_refused_only_when_its_kept_tags_pass_the_limit() {
        use crate::hidden::{MOST_KEPT_BYTES, MOST_SMALL, kept_charge};
        use crate::test_audio::{
            Aiff, Dense, aiff, dense_container_bytes, with_dense_tags, with_small_aiff_tags,
            with_small_flac_tags,
        };
        use crate::test_m4a::{bx, dense_meta};
        let (left, right, wav) = stereo_wav();
        let flac = compressed(&left, &right, 44_100);
        let plain_aiff = aiff(&[left, right], 44_100, 16, Aiff::Plain);
        let m4a = alac_of(m4a::Order::MoovLast);
        let five_thousand = with_small_aiff_tags(&plain_aiff, 5000);
        assert_eq!(decoded(five_thousand, "5000 tags").frames(), FRAMES);
        let empty_meta = bx(b"meta", &[&[0; 4]]);
        let dense_meta = dense_meta();
        // Each form: its file, its container size, a file of `count` of
        // them, and whether Symphonia opens a file of many of them.
        type Tagged<'a> = Box<dyn Fn(usize) -> Vec<u8> + 'a>;
        fn dense(plain: &[u8], form: Dense) -> (u64, Tagged<'_>) {
            let size = dense_container_bytes(form) as u64;
            let tagged = move |count| with_dense_tags(plain, form, count);
            (size, Box::new(tagged))
        }
        fn metas<'a>(m4a: &'a [u8], meta: &'a [u8]) -> (u64, Tagged<'a>) {
            let tagged = move |count| [m4a, &meta.repeat(count)].concat();
            (meta.len() as u64, Box::new(tagged))
        }
        type Form<'a> = (&'a str, &'a str, &'a [u8], (u64, Tagged<'a>), bool);
        let forms: [Form; 8] = [
            (
                "flac",
                "APPLICATION blocks",
                &flac,
                (1028, Box::new(|count| with_small_flac_tags(&flac, count))),
                true,
            ),
            (
                "aiff",
                "NAME chunks",
                &plain_aiff,
                (
                    10,
                    Box::new(|count| with_small_aiff_tags(&plain_aiff, count)),
                ),
                true,
            ),
            ("m4a", "empty metas", &m4a, metas(&m4a, &empty_meta), true),
            (
                "flac",
                "dense VORBIS_COMMENT",
                &flac,
                dense(&flac, Dense::FlacComments),
                true,
            ),
            (
                "wav",
                "dense LIST/INFO",
                &wav,
                dense(&wav, Dense::WavInfo),
                true,
            ),
            (
                "aiff",
                "dense ID3 chunks",
                &plain_aiff,
                dense(&plain_aiff, Dense::AiffId3),
                true,
            ),
            // Symphonia refuses a second COMT chunk by itself.
            (
                "aiff",
                "dense COMT",
                &plain_aiff,
                dense(&plain_aiff, Dense::AiffComments),
                false,
            ),
            ("m4a", "dense ilst", &m4a, metas(&m4a, &dense_meta), true),
        ];
        for (ext, form, plain, (size, tagged), opens) in forms {
            let case = format!("{ext} of {form}");
            let most = MOST_SMALL + (MOST_KEPT_BYTES / kept_charge(size)) as usize;
            let (plain_peak, _) = block_peak(Vec::from(plain), ext);
            let (peak, frames) = block_peak(tagged(most), ext);
            assert_eq!(frames.is_ok(), opens, "{case} at the limit: {frames:?}");
            let bound = plain_peak + MOST_KEPT_BYTES as usize + (256 << 10);
            assert!(
                peak <= bound,
                "{case} at the limit: a peak of {peak} bytes, over {bound}"
            );
            assert_refused_early(
                tagged(most + 1),
                ext,
                TAGS_TEXT,
                &format!("{case} past the limit"),
            );
        }
    }

    /// Finding 4-2 of review round 4 (MEM-4): an M4A behind an ID3v2 tag
    /// opens at the start of its view, and one that starts with a `uuid`
    /// box opens as MP4.  The decode walks it from the same start, so its
    /// art is hidden and its checks hold.
    #[test]
    fn an_m4a_behind_an_id3_tag_or_a_uuid_box_is_walked() {
        use crate::hidden::{MOST_KEPT_BYTES, MOST_SMALL, kept_charge};
        use crate::test_audio::id3_tag;
        use crate::test_m4a::{bx, with_cover, with_stco_moved};
        let plain = alac_of(m4a::Order::MoovLast);
        let tag = id3_tag(100);
        let behind_tag = [tag.clone(), with_cover(plain.clone(), ART)].concat();
        assert_tags_are_hidden(plain.clone(), behind_tag, "m4a", "art behind an ID3 tag");
        let uuid = bx(b"uuid", &[&[7; 92]]);
        let moved = with_stco_moved(with_cover(plain.clone(), ART), uuid.len() as i64);
        let behind_uuid = [uuid, moved].concat();
        assert_tags_are_hidden(plain.clone(), behind_uuid, "m4a", "art behind a uuid box");
        let large = crate::test_m4a::with_ftyp_of(&plain, ART + 16);
        let ftyp = [tag.clone(), large].concat();
        assert_tags_are_hidden(
            plain.clone(),
            ftyp,
            "m4a",
            "an ftyp of 64 MiB behind an ID3 tag",
        );
        let past = MOST_SMALL + (MOST_KEPT_BYTES / kept_charge(12)) as usize + 1;
        let metas = bx(b"meta", &[&[0; 4]]).repeat(past);
        let many = [tag, plain, metas].concat();
        assert_refused_early(many, "m4a", TAGS_TEXT, "small metas behind an ID3 tag");
    }

    /// Finding 3-2 of review round 3 (MEM-4): Symphonia reads each `ftyp`
    /// whole.  A later `ftyp` is hidden.  B5 of review round 7: Symphonia
    /// needs the first one only to exist, and reads no brand, so the view
    /// shows one past `MOST_BOX_BYTES` as 16 bytes and a `free` box.  It
    /// opens bit for bit at a flat peak.
    #[test]
    fn a_large_ftyp_is_hidden() {
        use crate::test_m4a::bx;
        let plain = alac_of(m4a::Order::MoovLast);
        let brands = b"isom".repeat(ART / 4);
        let large = bx(b"ftyp", &[b"M4A ", &[0; 4], &brands]);
        let appended = [plain.clone(), large.clone()].concat();
        assert_tags_are_hidden(plain.clone(), appended, "m4a", "a later ftyp of 64 MiB");
        use crate::mp4_boxes::MOST_BOX_BYTES;
        use crate::test_m4a::with_ftyp_of;
        let first = with_ftyp_of(&plain, large.len());
        assert_tags_are_hidden(plain.clone(), first, "m4a", "a first ftyp of 64 MiB");
        // Finding 4-3 and missed mutant 4-13 of review round 4: a first
        // `ftyp` of size 0 runs to the end of the file, so no `moov` follows.
        let mut size_0 = plain.clone();
        size_0[..4].copy_from_slice(&[0; 4]);
        let text =
            "unsupported or unrecognised format: unsupported feature: isomp4: missing moov atom";
        assert_refused_early(size_0, "m4a", text, "a first ftyp of size 0");
        for size in [MOST_BOX_BYTES, MOST_BOX_BYTES + 4] {
            let case = format!("a first ftyp of {size} bytes");
            assert_tags_are_hidden(plain.clone(), with_ftyp_of(&plain, size), "m4a", &case);
        }
    }

    /// Finding 3-7 and missed mutant 3-22 of review round 3: a `moov`, its
    /// last `trak` and that `trak`'s last `mdia` of size 0 run to the end
    /// of the file.  Their tables count to the end of the file.
    #[test]
    fn tables_that_run_to_the_end_of_the_file_count_in_the_bound() {
        use crate::test_m4a::{with_grown_stsz, with_sizes_of_0};
        let grown = with_grown_stsz(alac_of(m4a::Order::MoovLast), 17 << 20);
        let to_the_end = with_sizes_of_0(grown, &[b"moov", b"trak", b"mdia"]);
        assert_refused_early(to_the_end, "m4a", MOOV_TEXT, "tables to the end");
    }

    /// Missed mutant 3-21 of review round 3: a `moov` that holds exactly
    /// `most_moov_bytes` opens; one more byte gets `MOOV_TEXT`.  Finding
    /// 3-14: an `mvex` counts too, since Symphonia keeps a Vec of `trex`.
    #[test]
    fn a_moov_at_its_bound_opens_and_one_byte_more_is_refused() {
        use crate::test_m4a::{bx, with_child};
        let plain = alac_of(m4a::Order::MoovLast);
        let found = crate::mp4_boxes::walk(&mut &plain[..]).ok().flatten();
        assert!(found.is_some(), "an MP4 file");
        let found = found.unwrap();
        let room = (most_moov_bytes(&found) - found.moov_held) as usize;
        let free = |size: usize| bx(b"free", &[&vec![0; size - 8]]);
        let at = with_child(plain.clone(), &[b"moov", b"trak", b"mdia"], &free(room));
        assert_eq!(decoded(at, "at the bound").frames(), 4096);
        let past = with_child(plain.clone(), &[b"moov", b"trak", b"mdia"], &free(room + 1));
        assert_refused_early(past, "m4a", MOOV_TEXT, "one byte past the bound");
        let mvex = bx(b"mvex", &[&vec![0; 17 << 20]]);
        assert_refused_early(
            with_child(plain, &[b"moov"], &mvex),
            "m4a",
            MOOV_TEXT,
            "an mvex",
        );
    }

    /// Finding 3-6 of review round 3: Symphonia reads `tkhd`, `edts` and
    /// `mdia` of a `trak`, and skips its `udta`.  So art there opens.
    #[test]
    fn art_in_a_trak_opens() {
        use crate::test_m4a::{bx, cover_meta, with_child};
        let plain = alac_of(m4a::Order::MoovLast);
        let udta = bx(b"udta", &[&cover_meta(ART)]);
        let tagged = with_child(plain.clone(), &[b"moov", b"trak"], &udta);
        assert_tags_are_hidden(plain, tagged, "m4a", "art in trak/udta");
    }

    /// A source that cannot seek, as a FIFO.
    struct Fifo(Cursor<Vec<u8>>);

    impl std::io::Read for Fifo {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl std::io::Seek for Fifo {
        fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::other("Illegal seek"))
        }
    }

    impl MediaSource for Fifo {
        fn is_seekable(&self) -> bool {
            false
        }

        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    /// Missed mutant 3-20 of review round 3: a source that cannot seek gets
    /// no walk and no plan, and decodes.
    #[test]
    fn a_source_that_cannot_seek_decodes_with_no_plan() {
        let (_, _, wav) = stereo_wav();
        let decoder = Decoder::new(Box::new(Fifo(Cursor::new(wav))), Some("wav"));
        assert!(decoder.is_ok(), "{:?}", decoder.err());
        let frames = every_packet(decoder.unwrap());
        assert_eq!(frames.ok(), Some(FRAMES as u64));
    }

    /// The `iTunSMPB` tag of a hidden `udta` still gives the trim: the
    /// walker reads it, not Symphonia.
    #[test]
    fn an_m4a_with_art_keeps_its_itunsmpb_trim() {
        use crate::test_m4a::{ENCODER_DELAY, aac_m4a, aac_track_frames, with_cover};
        let source = short_tone();
        let track = aac_track_frames(4096);
        let gapless = m4a::Gapless {
            delay: ENCODER_DELAY as u32,
            padding: (track - ENCODER_DELAY as u64 - 4096) as u32,
            frames: 4096,
        };
        let file = aac_m4a(&source, 44_100, None, Some(gapless), m4a::Order::MoovLast);
        let audio = decoded(with_cover(file, ART), "art and iTunSMPB");
        assert_eq!(audio.frames(), 4096);
    }

    /// Finding 2-2 of review round 2 (MEM-4): sample tables past
    /// `most_moov_bytes` are refused before Symphonia opens the file, at a
    /// small peak.
    #[test]
    fn an_m4a_with_tables_past_the_bound_is_refused_early() {
        use crate::test_m4a::with_grown_stsz;
        let grown = with_grown_stsz(alac_of(m4a::Order::MoovLast), 17 << 20);
        assert_eq!(refusal(grown.clone(), "m4a"), MOOV_TEXT);
        let (peak, opened) = peak_of(|| {
            open_within(
                Box::new(Cursor::new(grown)),
                Some("m4a"),
                MAX_DECODED_BYTES,
                None,
            )
        });
        let error = opened.err().map(|e| format!("{e:#}"));
        assert_eq!(error.as_deref(), Some(MOOV_TEXT));
        assert!(peak < 1 << 20, "the open peaks at {peak} bytes");
    }

    /// The bound of a `moov`: `TAG_ROOM`, and 32 bytes for each 1024
    /// frames of the `mdhd` duration, up to the decode limit.
    #[test]
    fn the_bound_of_a_moov_grows_with_its_duration() {
        use crate::mp4_boxes::{Found, SoundTrack};
        let found = |duration: Option<u64>| Found {
            sound: Some(SoundTrack {
                duration,
                ..SoundTrack::default()
            }),
            ..Found::default()
        };
        assert_eq!(most_moov_bytes(&Found::default()), TAG_ROOM);
        assert_eq!(most_moov_bytes(&found(None)), TAG_ROOM);
        assert_eq!(most_moov_bytes(&found(Some(10 * 1024))), TAG_ROOM + 320);
        assert_eq!(most_moov_bytes(&found(Some(10 * 1024 + 1))), TAG_ROOM + 352);
        let most = TAG_ROOM + 32 * (MAX_DECODED_BYTES / 8).div_ceil(1024);
        assert_eq!(most_moov_bytes(&found(Some(u64::MAX))), most);
    }

    /// Finding 1-2 of review round 1: the data handler of a QuickTime
    /// `minf` does not hide the kind of the track, in the head check and
    /// in the decode.
    #[test]
    fn a_quicktime_data_handler_keeps_the_kind_of_the_track() {
        use crate::test_m4a::{patched, with_data_handler};
        let sound = with_data_handler(alac_with_edit(
            &short_tone(),
            2112,
            700,
            m4a::Order::MoovLast,
        ));
        assert_eq!(head_of(&sound), HeadLength::Frames(2112 + 4096 + 700));
        let audio = decoded(sound, "a sound track");
        assert_eq!(audio.frames(), 4096, "the trim of its elst");
        let video = with_data_handler(patched(alac_of(m4a::Order::MoovLast), b"soun", b"vide"));
        assert_eq!(head_of(&video), HeadLength::Video);
        assert_eq!(refusal(video, "mp4"), VIDEO_TEXT);
    }

    /// `file` with the `mdhd` duration of its one track of version 0 as
    /// `duration`.
    fn with_mdhd_duration(mut file: Vec<u8>, duration: u32) -> Vec<u8> {
        // After the type: version and flags, two times, the timescale.
        let at = crate::test_m4a::place_once(&file, b"mdhd") + 4 + 16;
        file[at..at + 4].copy_from_slice(&duration.to_be_bytes());
        file
    }

    /// Finding 1-3 of review round 1: an `mdhd` duration of 0, or of all
    /// ones (unknown), states no length.  Missed mutant 1-27: nor does a
    /// timescale of 0.
    #[test]
    fn an_mdhd_of_no_length_states_no_length() {
        use crate::test_m4a::{bx, place_once, version_1};
        let file = alac_of(m4a::Order::FastStart);
        for duration in [0, u32::MAX] {
            assert_eq!(
                head_of(&with_mdhd_duration(file.clone(), duration)),
                HeadLength::UnstatedPcm,
                "{duration}"
            );
        }
        let mut v1 = [ftyp(), version_1()].concat();
        let at = place_once(&v1, b"mdhd") + 4 + 24;
        v1[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(head_of(&v1), HeadLength::UnstatedPcm);
        v1[at..at + 8].copy_from_slice(&0_u64.to_be_bytes());
        assert_eq!(head_of(&v1), HeadLength::UnstatedPcm);
        let full = [0_u8; 4];
        let mdhd = bx(
            b"mdhd",
            &[
                &full,
                &[0; 8],
                &0_u32.to_be_bytes(),
                &1000_u32.to_be_bytes(),
                &[0; 4],
            ],
        );
        let hdlr = bx(b"hdlr", &[&full, &[0; 4], b"soun"]);
        let no_scale = bx(b"moov", &[&bx(b"trak", &[&bx(b"mdia", &[&mdhd, &hdlr])])]);
        let no_scale = [ftyp(), no_scale].concat();
        assert_eq!(head_of(&no_scale), HeadLength::UnstatedPcm);
    }

    /// Finding 1-4 of review round 1: an `mp4a` entry of rate 0 (it cannot
    /// hold 88.2 or 96 kHz) states no rate.  It is no sign of SBR, and the
    /// decode takes the rate of its first packet.
    #[test]
    fn an_mp4a_entry_of_rate_0_decodes_at_the_rate_of_its_config() {
        use crate::test_m4a::{aac_m4a, place_once};
        let mut file = aac_m4a(&chirps(), 44_100, None, None, m4a::Order::FastStart);
        // After the type: reserved, the reference index, reserved, the
        // channels, the sample size, then 4 bytes before the rate.
        let at = place_once(&file, b"mp4a") + 4 + 6 + 2 + 8 + 2 + 2 + 4;
        assert_eq!(file[at..at + 4], (44_100_u32 << 16).to_be_bytes());
        file[at..at + 4].copy_from_slice(&[0; 4]);
        let audio = decoded(file, "an entry of rate 0");
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(
            audio.frames() as u64,
            crate::test_m4a::aac_track_frames(88_200)
        );
    }

    /// Finding 2-12 of review round 2: an `mp4a` entry rate other than the
    /// rate of the AudioSpecificConfig, 0 and twice it, would play at the
    /// wrong speed, so it gives the AAC-LC text.
    #[test]
    fn an_mp4a_entry_of_another_rate_gives_the_aac_lc_text() {
        use crate::test_m4a::config;
        for entry in [48_000, 32_000] {
            let file = dummy_aac(&config(2, 44_100, 2), 2, entry);
            assert_eq!(refusal(file, "m4a"), AAC_KIND_TEXT, "{entry} Hz");
        }
        assert_eq!(
            refusal(dummy_aac(&config(2, 22_050, 2), 2, 44_100), "m4a"),
            HE_AAC_TEXT
        );
    }

    /// The parts of an AIFF: a chunk of `id` and `body`, with its pad byte.
    fn aiff_chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::from(*id);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// A `COMM` chunk of 2 channels of 16 bits at 44.1 kHz.
    fn aiff_comm(frames: u32) -> Vec<u8> {
        const RATE: [u8; 10] = [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0];
        let body = [
            &2_u16.to_be_bytes()[..],
            &frames.to_be_bytes(),
            &16_u16.to_be_bytes(),
            &RATE,
        ]
        .concat();
        aiff_chunk(b"COMM", &body)
    }

    fn aiff_form(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body = [&b"AIFF"[..], &chunks.concat()].concat();
        [&b"FORM"[..], &(body.len() as u32).to_be_bytes(), &body].concat()
    }

    /// Missed mutants 1-25 and 1-28 of review round 1: the samples of an
    /// `SSND` start after its offset, and a chunk of odd size has a pad
    /// byte.
    #[test]
    fn an_aiff_head_takes_the_ssnd_offset_and_the_pad_byte() {
        let mut ssnd = Vec::from(40_u32.to_be_bytes());
        ssnd.extend_from_slice(&[0; 4 + 40 + 1000 * 4]);
        let file = aiff_form(&[aiff_comm(1000), aiff_chunk(b"SSND", &ssnd)]);
        // A file cut after 100 frames of samples.
        let cut = file.len() - 900 * 4;
        assert_eq!(
            head_length(&file[..cut], cut as u64),
            HeadLength::Frames(100)
        );
        let odd = aiff_form(&[
            aiff_chunk(b"ANNO", b"abc"),
            aiff_comm(1000),
            aiff_chunk(b"SSND", &[0; 8 + 4000]),
        ]);
        assert_eq!(head_of(&odd), HeadLength::Frames(1000));
    }

    /// Checks that a decode of `bytes` holds no more than 4 MiB at its
    /// peak, whether it opens or not: Symphonia reads none of its tags.
    fn assert_small_peak(bytes: Vec<u8>, ext: &str, case: &str) {
        let (peak, _) = block_peak(bytes, ext);
        assert!(peak < 4 << 20, "{case}: the decode peaks at {peak} bytes");
    }

    /// Findings 5-2, 5-3 and 5-4 of review round 5 (MEM-4), and 6-1 of
    /// review round 6: `engine` opens the one reader of the first marker
    /// that Symphonia's probe accepts, where the plan and the walk start.
    /// So the tags of the container that opens are hidden, after any bytes.
    #[test]
    fn a_file_opens_at_the_first_marker_with_its_tags_hidden() {
        use crate::test_audio::{
            Aiff, aiff, id3_tag, mp3, tone, with_aiff_tags, with_flac_tags, with_wav_tags,
        };
        use crate::test_m4a::{bx, place_once, with_cover, with_stco_moved};
        let plain = alac_of(m4a::Order::MoovLast);
        let art = with_cover(plain.clone(), ART);
        let inside_free = bx(b"free", &[&art]);
        assert_small_peak(inside_free, "m4a", "an M4A inside a free box");
        let avi = [&b"RIFF"[..], &[0, 0, 0, 0], b"AVI ", &art].concat();
        assert_small_peak(avi, "m4a", "an M4A after RIFF/AVI");
        let junk_form = [&b"FORM"[..], &[0, 0, 0, 0], b"JUNK", &art].concat();
        assert_small_peak(junk_form, "m4a", "an M4A after FORM/JUNK");
        let bad_sync = [&[0xFF, 0xE0][..], &art].concat();
        assert_small_peak(bad_sync, "m4a", "an M4A after a bad sync");
        let (left, right, wav) = stereo_wav();
        let flac = compressed(&left, &right, 44_100);
        let plain_aiff = aiff(&[left, right], 44_100, 16, Aiff::Plain);
        for (ext, tagged) in [
            ("flac", with_flac_tags(&flac, ART)),
            ("wav", with_wav_tags(&wav, ART)),
            ("aiff", with_aiff_tags(&plain_aiff, ART)),
        ] {
            let behind = [vec![0x42], tagged].concat();
            assert_small_peak(behind, ext, &format!("a {ext} behind one junk byte"));
        }
        // A `free` box with `ftyp` in it: the probe takes that marker, so
        // the MP4 reader starts in the `free` box, as on `main`.
        let decoy = bx(b"free", &[b"ftyp", &[0; 20]]);
        let moved = with_stco_moved(art.clone(), decoy.len() as i64);
        assert_small_peak([decoy, moved].concat(), "m4a", "a free decoy");
        let uuid = bx(b"uuid", &[&vec![7; 100 << 10]]);
        let moved = with_stco_moved(art, uuid.len() as i64);
        assert_tags_are_hidden(plain, [uuid, moved].concat(), "m4a", "a uuid of 100 KiB");
        let mut ftyp_tag = with_flac_tags(&flac, ART);
        let title = place_once(&ftyp_tag, b"lloyd");
        ftyp_tag[title..title + 5].copy_from_slice(b"ftyp ");
        assert_tags_are_hidden(flac, ftyp_tag, "flac", "a FLAC with ftyp in a tag");
        let tones = [tone(0, 44_100, 44_100), tone(1, 44_100, 44_100)];
        let plain_mp3 = mp3(&tones, 44_100);
        let junk = [id3_tag(100), vec![0; 50], plain_mp3.clone()].concat();
        assert_tags_are_hidden(plain_mp3, junk, "mp3", "an MP3 with junk after its tag");
    }

    /// Finding 5-6 of review round 5: the head check takes the rule and
    /// the start of the decode, so an M4A behind an ID3v2 tag states its
    /// length, and a video behind one is a video.
    #[test]
    fn a_head_check_starts_after_the_id3_tags() {
        use crate::test_audio::id3_tag;
        use crate::test_m4a::patched;
        let m4a = [id3_tag(100), alac_of(m4a::Order::FastStart)].concat();
        assert_eq!(head_of(&m4a), HeadLength::Frames(4096));
        let video = patched(alac_of(m4a::Order::FastStart), b"soun", b"vide");
        assert_eq!(head_of(&[id3_tag(100), video].concat()), HeadLength::Video);
    }

    /// Finding 6-1 of review round 6: a FLAC, a WAV, an AIFF and an M4A
    /// behind junk bytes open bit for bit, as on `main`, since junk bytes
    /// put no memory at risk.  Their tags stay hidden.  The M4A here has
    /// `moov` first; `an_m4a_behind_junk_opens_wherever_its_offsets_count_from`
    /// tests each form of M4A behind junk.
    #[test]
    fn a_file_behind_junk_bytes_opens() {
        use crate::test_audio::{Aiff, aiff, with_flac_tags};
        use crate::test_m4a::{cover_meta, with_stco_moved};
        let (left, right, wav) = stereo_wav();
        let flac = compressed(&left, &right, 44_100);
        let plain_aiff = aiff(&[left, right], 44_100, 16, Aiff::Plain);
        let m4a = alac_of(m4a::Order::FastStart);
        let junks = [vec![0x42], vec![0x42; 100], vec![0xFF, 0xFB, 0x90, 0x00]];
        for junk in &junks {
            let moved = with_stco_moved(m4a.clone(), junk.len() as i64);
            for (ext, plain, behind) in [
                ("flac", &flac, [junk.as_slice(), &flac].concat()),
                ("wav", &wav, [junk.as_slice(), &wav].concat()),
                ("aiff", &plain_aiff, [junk.as_slice(), &plain_aiff].concat()),
                ("m4a", &m4a, [junk.as_slice(), &moved].concat()),
            ] {
                let case = format!("a {ext} behind {} junk bytes", junk.len());
                let want = decode_bytes(plain.clone(), Some(ext));
                let got = decode_bytes(behind, Some(ext));
                assert!(got.is_ok(), "{case}: {:?}", got.err());
                assert!(got.unwrap().channels == want.unwrap().channels, "{case}");
            }
        }
        let junk = vec![0x42; 100];
        let moved = with_stco_moved(m4a.clone(), junk.len() as i64);
        let behind = [junk.clone(), moved, cover_meta(ART)].concat();
        assert_tags_are_hidden(m4a, behind, "m4a", "an M4A with art behind junk");
        let behind = [junk, with_flac_tags(&flac, ART)].concat();
        assert_tags_are_hidden(flac, behind, "flac", "a FLAC with art behind junk");
    }

    /// Finding 6-1 and missed mutant 6-13 of review round 6: a file with no
    /// marker that the probe accepts in its first `SCAN_BYTES` is refused,
    /// also an MP3 after that window, which the MP3 reader could reach.
    #[test]
    fn a_file_with_no_marker_in_the_window_is_refused() {
        use crate::hidden::SCAN_BYTES;
        use crate::test_audio::{mp3, tone};
        let text = "unsupported or unrecognised format: no known format at the start of the file";
        let (left, right, _) = stereo_wav();
        let flac = compressed(&left, &right, 44_100);
        let tones = [tone(0, 44_100, 44_100), tone(1, 44_100, 44_100)];
        let far = vec![0; SCAN_BYTES + 64 * 1024];
        for (ext, file) in [("flac", flac), ("mp3", mp3(&tones, 44_100))] {
            let behind = [far.as_slice(), &file].concat();
            assert_refused_early(behind, ext, text, &format!("a {ext} past the window"));
        }
        assert_refused_early(vec![0x42; 100], "wav", text, "no marker");
    }

    /// Finding 6-1 of review round 6: a source that cannot seek scans the
    /// same window, and its reader opens after the junk bytes.
    #[test]
    fn a_source_that_cannot_seek_opens_behind_junk_bytes() {
        let (_, _, wav) = stereo_wav();
        let behind = [vec![0x42; 100], wav.clone()].concat();
        let decoder = Decoder::new(Box::new(Fifo(Cursor::new(behind))), Some("wav"));
        assert!(decoder.is_ok(), "{:?}", decoder.err());
        let frames = every_packet(decoder.unwrap());
        assert_eq!(frames.ok(), Some(FRAMES as u64));
        use crate::hidden::LAST_MARKER;
        let at = |offset: usize| Fifo(Cursor::new([vec![0; offset], wav.clone()].concat()));
        let last = Decoder::new(Box::new(at(LAST_MARKER)), Some("wav"));
        assert!(last.is_ok(), "at the last offset: {:?}", last.err());
        let past = Decoder::new(Box::new(at(LAST_MARKER + 1)), Some("wav"));
        assert!(past.is_err(), "past the window");
    }

    /// Findings 6-1 and 6-5 of review round 6: the head check scans as the
    /// decode does, so a WAV behind junk states its length.  A WAV or an
    /// AIFF that states more frames than it holds, behind an ID3v2 tag,
    /// counts the frames that it holds after the tag.
    #[test]
    fn a_head_check_finds_the_marker_and_counts_the_frames_held() {
        use crate::test_audio::{Aiff, aiff, id3_tag};
        let (left, right, wav) = stereo_wav();
        let behind = [vec![0x42; 100], wav.clone()].concat();
        assert_eq!(head_of(&behind), HeadLength::Frames(FRAMES as u64));
        let plain_aiff = aiff(&[left, right], 44_100, 16, Aiff::Plain);
        let cut = |file: &[u8]| Vec::from(&file[..file.len() - (FRAMES - 100) * 4]);
        for (case, file) in [("wav", cut(&wav)), ("aiff", cut(&plain_aiff))] {
            let tagged = [id3_tag(1000), file.clone()].concat();
            assert_eq!(head_of(&tagged), HeadLength::Frames(100), "{case}");
            // Finding 7-2 of review round 7: the bytes before the marker
            // count too.
            let behind = [vec![0; 1000], file].concat();
            assert_eq!(
                head_of(&behind),
                HeadLength::Frames(100),
                "{case} behind junk"
            );
        }
        // Findings 7-11 and 7-16 of review round 7: a head that holds the
        // last offset that the scan tests.
        use crate::hidden::LAST_MARKER;
        let far = [vec![0; LAST_MARKER], wav].concat();
        let head = head_length(&far, far.len() as u64);
        assert_eq!(
            head,
            HeadLength::Frames(FRAMES as u64),
            "at the last offset"
        );
    }

    /// Finding 6-2 of review round 6 (MEM-4): Symphonia keeps an entry for
    /// each top-level `sidx` of its own `reference_id`.  The decode reads
    /// in order and needs no segment index, so each `sidx` is hidden as
    /// `free`.  A file of 2000 of them opens at a flat peak.  One of about
    /// 16 MiB of them is refused early by the limit of kept tags, past its
    /// first `MOST_SMALL`.
    #[test]
    fn each_top_level_sidx_is_hidden() {
        use crate::test_m4a::sidx_boxes;
        let plain = alac_of(m4a::Order::MoovLast);
        let some = [plain.clone(), sidx_boxes(2000)].concat();
        assert_tags_are_hidden(plain.clone(), some, "m4a", "2000 sidx boxes");
        let many = [plain.clone(), sidx_boxes(524_288)].concat();
        assert_refused_early(many, "m4a", TAGS_TEXT, "16 MiB of sidx boxes");
        // Finding 7-4 of review round 7: Symphonia keeps one entry for an
        // id, so the `sidx` of an id that it holds costs nothing.
        let one_id = [plain.clone(), crate::test_m4a::same_sidx_boxes(13_000)].concat();
        assert_tags_are_hidden(plain, one_id, "m4a", "13,000 sidx boxes of one id");
    }

    /// Finding 6-3 of review round 6 (MEM-4): Symphonia holds each point of
    /// a FLAC SEEKTABLE.  The decode reads in order and needs no seek table,
    /// so it shows SEEKTABLE as PADDING.  A table of 932,067 points, the
    /// most that a block holds, opens bit for bit at a flat peak.
    #[test]
    fn a_flac_seektable_is_hidden() {
        use crate::test_audio::with_seektable;
        let (left, right, _) = stereo_wav();
        let flac = compressed(&left, &right, 44_100);
        let small = with_seektable(&flac, 10);
        assert_tags_are_hidden(flac.clone(), small, "flac", "a seek table of 10 points");
        let large = with_seektable(&flac, 932_067);
        assert_tags_are_hidden(flac, large, "flac", "a seek table of 932,067 points");
    }

    /// Finding 7-1 of review round 7: an `ftyp` at offsets 0 to 3 of the
    /// view after an ID3v2 tag does not panic.  The MP4 reader starts at the
    /// view, and it refuses the file.
    #[test]
    fn an_ftyp_at_the_start_of_a_view_does_not_panic() {
        use crate::test_audio::id3_tag;
        for offset in 0..4 {
            let file = [
                id3_tag(100),
                vec![0; offset],
                Vec::from(*b"ftyp"),
                vec![0; 64],
            ]
            .concat();
            let opened = decode_bytes(file, Some("m4a"));
            assert!(opened.is_err(), "{offset}");
        }
    }

    /// A source that cannot seek and counts the bytes read from it.
    struct Counted(
        Cursor<Vec<u8>>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    );

    impl std::io::Read for Counted {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let read = self.0.read(buf)?;
            self.1.fetch_add(read, std::sync::atomic::Ordering::Relaxed);
            Ok(read)
        }
    }

    impl std::io::Seek for Counted {
        fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::other("Illegal seek"))
        }
    }

    impl MediaSource for Counted {
        fn is_seekable(&self) -> bool {
            false
        }

        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    /// Finding 7-8 and missed mutant 7-13 of review round 7: the scan of a
    /// source that cannot seek takes at most the window and the reach of its
    /// scores from the source, and the open of a WAV of 4 MiB peaks under
    /// 1.1 MiB.
    #[test]
    fn a_source_that_cannot_seek_reads_at_most_the_window() {
        use crate::hidden::{LAST_MARKER, MOST_SCORE_BYTES};
        let tone: Vec<f32> = (0..1 << 20)
            .map(|i| (i as f32 * 0.01).sin() * 0.5)
            .collect();
        let wav = encode_pcm16(&tone, &tone, 44_100);
        assert!(wav.len() > 4 << 20);
        let read = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let source = Counted(Cursor::new(wav.clone()), read.clone());
        let scanned = scan_stream(Box::new(source));
        assert!(scanned.is_ok());
        let read = read.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(read, LAST_MARKER + MOST_SCORE_BYTES, "the bytes taken");
        let source = Fifo(Cursor::new(wav));
        let (peak, opened) = peak_of(|| Decoder::new(Box::new(source), Some("wav")));
        assert!(opened.is_ok(), "{:?}", opened.err());
        assert!(peak < 1_153_434, "the open peaks at {peak} bytes");
    }

    /// Missed mutant 7-17 of review round 7: an M4A with `moov` first from a
    /// source that cannot seek decodes bit for bit, with no walk.
    #[test]
    fn a_fast_start_m4a_that_cannot_seek_decodes() {
        let file = alac_of(m4a::Order::FastStart);
        let want = decode_bytes(file.clone(), Some("m4a"));
        let got = decode_source(Box::new(Fifo(Cursor::new(file))), Some("m4a"));
        assert!(got.is_ok(), "{:?}", got.err());
        assert!(got.unwrap().channels == want.unwrap().channels);
    }

    /// B1 of review round 7: an M4A behind junk bytes opens bit for bit,
    /// with `moov` first or last, whether its chunk offsets count from the
    /// MP4 start or from the file start.  With `moov` last, Symphonia walks
    /// the boxes again from the start of its stream (`demuxer.rs`, line
    /// 257): the view starts at the MP4 start, or shows the junk as a `free`
    /// box after that seek, also for 1 to 7 bytes.  The head check gives the
    /// length of the plain file.
    #[test]
    fn an_m4a_behind_junk_opens_wherever_its_offsets_count_from() {
        use crate::test_m4a::{aac_m4a, with_stco_moved};
        let alac = |order| alac_of(order);
        let aac = |order| aac_m4a(&short_tone(), 44_100, None, None, order);
        type Make<'a> = &'a dyn Fn(m4a::Order) -> Vec<u8>;
        let codecs: [(&str, Make); 2] = [("alac", &alac), ("aac", &aac)];
        for (codec, make) in codecs {
            for order in [m4a::Order::MoovLast, m4a::Order::FastStart] {
                let plain = make(order);
                let want = decode_bytes(plain.clone(), Some("m4a"));
                assert!(want.is_ok(), "{codec} {order:?}");
                let want = want.unwrap().channels;
                for before in [100, 8, 7, 4, 1] {
                    let junk = vec![0x42; before];
                    let moved = with_stco_moved(plain.clone(), before as i64);
                    for (form, file) in [
                        ("from the MP4 start", [junk.as_slice(), &plain].concat()),
                        ("from the file start", [junk.as_slice(), &moved].concat()),
                    ] {
                        let case = format!("{codec} {order:?} behind {before} bytes, {form}");
                        assert_eq!(head_of(&file), head_of(&plain), "{case}");
                        let got = decode_bytes(file, Some("m4a"));
                        assert!(got.is_ok(), "{case}: {:?}", got.err());
                        assert!(got.unwrap().channels == want, "{case}");
                    }
                }
            }
        }
        use crate::test_m4a::with_cover;
        let plain = alac_of(m4a::Order::MoovLast);
        let junk = vec![0x42; 100];
        let art = [junk.clone(), with_cover(plain.clone(), ART)].concat();
        assert_tags_are_hidden(plain.clone(), art, "m4a", "art, from the MP4 start");
        let moved = with_stco_moved(with_cover(plain.clone(), ART), 100);
        let art = [junk, moved].concat();
        assert_tags_are_hidden(plain, art, "m4a", "art, from the file start");
    }

    /// B1 of review round 7: the form of the chunk offsets of an MP4 behind
    /// other bytes.
    #[test]
    fn the_chunk_offsets_count_from_where_they_fall_in_an_mdat() {
        // One `mdat` body from 116 to 1000 of the view, the MP4 at 100.
        let bodies = [(116, 1000)];
        assert!(
            counts_from_mp4_start(16, 100, &bodies),
            "only from the MP4 start"
        );
        assert!(
            !counts_from_mp4_start(116, 100, &bodies),
            "at a body start from the file start"
        );
        assert!(
            !counts_from_mp4_start(900, 100, &bodies),
            "only from the file start"
        );
        assert!(
            counts_from_mp4_start(120, 100, &bodies),
            "inside for both, at no start"
        );
        assert!(
            counts_from_mp4_start(16, 100, &[(116, 1000), (16, 50)]),
            "both at a start"
        );
        assert!(counts_from_mp4_start(5000, 100, &bodies), "neither");
    }

    /// B6 of review round 7: Symphonia 0.6.1 refuses an SSND block size
    /// other than 0 (`aiff/chunks.rs`, line 321).  The block size only
    /// aligns the samples, which start at the offset, so the view shows 0.
    /// An AIFF and an AIFC with a block size and an offset of 4096 open bit
    /// for bit.
    #[test]
    fn an_aiff_with_block_aligned_samples_opens() {
        use crate::test_audio::{Aiff, aiff, with_block_aligned_ssnd};
        let (left, right, _) = stereo_wav();
        for form in [Aiff::Plain, Aiff::UncompressedAifc] {
            let plain = aiff(&[left.clone(), right.clone()], 44_100, 16, form);
            let aligned = with_block_aligned_ssnd(&plain, 4096);
            assert_tags_are_hidden(plain, aligned, "aiff", &format!("{form:?}"));
        }
    }

    /// B6 of review round 7: an `ftyp` past the first 1 MiB of the view,
    /// after a chain of top-level boxes, opens bit for bit, with `moov`
    /// first or last.  Bytes that are not a box before it still give no
    /// known format.
    #[test]
    fn an_m4a_after_a_large_free_or_uuid_box_opens() {
        use crate::test_m4a::{bx, with_stco_moved};
        for kind in [b"free", b"uuid"] {
            for order in [m4a::Order::MoovLast, m4a::Order::FastStart] {
                let plain = alac_of(order);
                let prefix = bx(kind, &[&vec![0; 2 << 20]]);
                let moved = with_stco_moved(plain.clone(), prefix.len() as i64);
                let case = format!("{order:?} after a {}", String::from_utf8_lossy(kind));
                assert_tags_are_hidden(plain, [prefix, moved].concat(), "m4a", &case);
            }
        }
        let junk = [vec![0x42; 2 << 20], alac_of(m4a::Order::FastStart)].concat();
        let text = "unsupported or unrecognised format: no known format at the start of the file";
        assert_refused_early(junk, "m4a", text, "an M4A after 2 MiB of junk");
    }

    /// TD-81, closed by review round 7: an HE-AAC stream whose config does
    /// not show SBR carries SBR data in a fill element before the end of each
    /// packet.  The decode checks the first 8 packets, and gives the HE-AAC
    /// text when each holds it: an M4A with the core rate in its entry, one
    /// with an entry rate of 0, and an ADTS stream.  AAC-LC still opens,
    /// also with one such packet in the first 8.
    #[test]
    fn an_aac_whose_packets_carry_sbr_gives_the_he_aac_text() {
        use crate::test_m4a::{
            aac_m4a_of, adts, adts_of, adts_packets, config, place_once, with_sbr_fill,
        };
        let stream = adts(&chirps(), 44_100);
        let plain = adts_packets(&stream);
        let sbr: Vec<Vec<u8>> = plain.iter().map(|packet| with_sbr_fill(packet)).collect();
        let sbr: Vec<&[u8]> = sbr.iter().map(Vec::as_slice).collect();
        let lc = config(2, 44_100, 2);
        let order = m4a::Order::MoovLast;
        let m4a = aac_m4a_of(&sbr, 2, &lc, 44_100, None, None, order);
        assert_eq!(
            refusal(m4a.clone(), "m4a"),
            HE_AAC_TEXT,
            "an entry at the core rate"
        );
        let mut rate_0 = m4a;
        let at = place_once(&rate_0, b"mp4a") + 4 + 6 + 2 + 8 + 2 + 2 + 4;
        rate_0[at..at + 4].copy_from_slice(&[0; 4]);
        assert_eq!(refusal(rate_0, "m4a"), HE_AAC_TEXT, "an entry rate of 0");
        assert_eq!(
            refusal(adts_of(&sbr, 44_100, 2, 0), "aac"),
            HE_AAC_TEXT,
            "ADTS"
        );
        let few = aac_m4a_of(&sbr[..3], 2, &lc, 44_100, None, None, order);
        assert_eq!(refusal(few, "m4a"), HE_AAC_TEXT, "3 packets");
        let mut one = plain.clone();
        one[3] = sbr[3];
        let lc_with_one = aac_m4a_of(&one, 2, &lc, 44_100, None, None, order);
        let want = decoded(
            aac_m4a_of(&plain, 2, &lc, 44_100, None, None, order),
            "AAC-LC",
        );
        assert!(decoded(lc_with_one, "one SBR tail").channels == want.channels);
        // The eighth packet of the check is AAC-LC, so the file opens.
        assert!(plain.len() > 8, "packets past the check");
        let mut seven = sbr.clone();
        seven[7] = plain[7];
        let seven = aac_m4a_of(&seven, 2, &lc, 44_100, None, None, order);
        assert!(decode_bytes(seven, Some("m4a")).is_ok(), "7 SBR tails of 8");
        let adts_lc = decode_bytes(adts_of(&plain, 44_100, 2, 0), Some("aac"));
        assert!(adts_lc.is_ok(), "ADTS AAC-LC: {:?}", adts_lc.err());
        // A track of no packet is no HE-AAC.
        let empty = aac_m4a_of(&[], 2, &lc, 44_100, None, None, order);
        let opened = Decoder::new(Box::new(Cursor::new(empty)), Some("m4a"));
        assert!(opened.is_ok(), "no packet: {:?}", opened.err());
    }

    /// B6 of review round 7: Symphonia refuses a program config element in
    /// the AudioSpecificConfig (`symphonia-common` `mpeg/audio/mod.rs`, line
    /// 289), and reads no bit after the GASpecificConfig of AAC-LC.  So a
    /// config of one front element, a channel pair or one channel, gets the
    /// channel configuration 2 or 1, and decodes bit for bit.  Another
    /// program config gives the text of the kind of AAC.
    #[test]
    fn an_aac_with_a_program_config_of_one_element_opens() {
        use crate::test_m4a::{aac_m4a_of, adts, adts_packets, config, pce_config};
        let order = m4a::Order::MoovLast;
        let tone = short_tone();
        for (channels, pair) in [
            (vec![tone[0].clone(), tone[1].clone()], true),
            (vec![tone[0].clone()], false),
        ] {
            let stream = adts(&channels, 44_100);
            let packets = adts_packets(&stream);
            let count = channels.len() as u8;
            let plain = aac_m4a_of(
                &packets,
                u16::from(count),
                &config(2, 44_100, count),
                44_100,
                None,
                None,
                order,
            );
            let pce = aac_m4a_of(
                &packets,
                u16::from(count),
                &pce_config(44_100, &[pair]),
                44_100,
                None,
                None,
                order,
            );
            let want = decoded(plain, "the channel configuration");
            let got = decoded(pce, &format!("a program config of {count} channels"));
            assert!(got.channels == want.channels, "{count} channels");
        }
        let stream = adts(&tone, 44_100);
        let packets = adts_packets(&stream);
        let two = aac_m4a_of(
            &packets,
            2,
            &pce_config(44_100, &[false, false]),
            44_100,
            None,
            None,
            order,
        );
        assert_eq!(refusal(two, "m4a"), AAC_KIND_TEXT, "two front elements");
    }

    /// B6 of review round 7: Symphonia refuses an ADTS frame of more than one
    /// raw data block (`adts.rs`, line 192).  With no CRC, the frame states
    /// no position of its blocks, so the decode gives its own text (TD-86).
    #[test]
    fn an_adts_stream_of_more_than_one_raw_block_gives_its_text() {
        use crate::test_m4a::{adts, adts_of, adts_packets};
        let stream = adts(&short_tone(), 44_100);
        let packets = adts_packets(&stream);
        let blocks = adts_of(&packets, 44_100, 2, 1);
        assert_refused_early(blocks, "aac", ADTS_BLOCKS_TEXT, "two raw blocks");
    }

    /// B6 of review round 7: a fragmented ALAC M4A, with a `moov` of empty
    /// tables and an `mvex`, then `moof` and `mdat` pairs, decodes bit for
    /// bit, with a `sidx` too.  The decode reads the fragments in order, and
    /// needs no `sidx` (finding 6-2 of review round 6).
    #[test]
    fn a_fragmented_m4a_decodes() {
        use crate::test_m4a::{alac_m4a, fragmented, places};
        let tone = chirps();
        let plain = alac_m4a(&tone[0], &tone[1], 44_100, None, m4a::Order::FastStart);
        let packets = 88_200_usize.div_ceil(4096);
        let want = decoded(plain.clone(), "the plain file").channels;
        for (per_fragment, sidx) in [(1, false), (2, true), (100, true)] {
            let case = format!("{per_fragment} packets in each fragment, sidx {sidx}");
            let file = fragmented(&plain, per_fragment, sidx);
            let moofs = places(&file, b"moof").len();
            assert_eq!(moofs, packets.div_ceil(per_fragment), "{case}");
            let got = decode_bytes(file, Some("m4a"));
            assert!(got.is_ok(), "{case}: {:?}", got.err());
            assert!(got.unwrap().channels == want, "{case}");
        }
    }

    /// B6 of review round 7: Symphonia 0.6.1 refuses a compressed AIFC type
    /// that it does not decode (`aiff/chunks.rs`, line 287).  The decode
    /// gives its own text for it, such as `MAC3`, for which Symphonia has no
    /// decoder.  `ima4` opens (`an_ima4_aifc_decodes`).
    #[test]
    fn a_compressed_aifc_gives_its_text() {
        use crate::test_audio::{Aiff, aiff};
        let (left, right, _) = stereo_wav();
        let plain = aiff(&[left, right], 44_100, 16, Aiff::UncompressedAifc);
        let at = (0..plain.len())
            .find(|at| plain[*at..].starts_with(b"NONE"))
            .unwrap();
        let mut mac3 = plain.clone();
        mac3[at..at + 4].copy_from_slice(b"MAC3");
        assert_refused_early(mac3, "aiff", AIFC_COMPRESSED_TEXT, "MAC3");
        let mut sowt = plain;
        sowt[at..at + 4].copy_from_slice(b"sowt");
        assert!(
            decode_bytes(sowt, Some("aiff")).is_ok(),
            "a type that Symphonia decodes"
        );
    }

    /// Finding 8-1 of review round 8: the mend of an AAC config runs only
    /// for AAC.  A FLAC of a fixed block size of 4100 and frame sizes of 0
    /// decodes.
    #[test]
    fn a_flac_of_a_fixed_block_of_4100_decodes() {
        use crate::test_flac::{Blocks, by_hand};
        let flac = by_hand(
            44_100,
            16,
            2,
            0,
            Blocks::Silence {
                block: 4100,
                blocks: 3,
            },
        );
        let audio = decode_bytes(flac, Some("flac"));
        assert!(audio.is_ok(), "{:?}", audio.err());
        assert_eq!(audio.unwrap().frames(), 3 * 4100);
    }

    /// A reader of `packets`, each (track, data), for the HE-AAC check.
    struct Scripted(
        std::collections::VecDeque<(u32, Vec<u8>)>,
        Vec<symphonia::core::formats::Track>,
    );

    impl FormatReader for Scripted {
        fn format_info(&self) -> &symphonia::core::formats::FormatInfo {
            unimplemented!("not used by the check")
        }

        fn media_info(&self) -> &symphonia::core::formats::MediaInfo {
            unimplemented!("not used by the check")
        }

        fn next_packet(&mut self) -> symphonia::core::errors::Result<Option<SymPacket>> {
            Ok(self.0.pop_front().map(|(track, data)| {
                SymPacket::new(
                    track,
                    symphonia::core::units::Timestamp::new(0),
                    symphonia::core::units::Duration::new(1024),
                    data,
                )
            }))
        }

        fn metadata(&mut self) -> symphonia::core::meta::Metadata<'_> {
            unimplemented!("not used by the check")
        }

        fn chapters(&self) -> Option<&symphonia::core::meta::ChapterGroup> {
            None
        }

        fn tracks(&self) -> &[symphonia::core::formats::Track] {
            &self.1
        }

        fn seek(
            &mut self,
            _: symphonia::core::formats::SeekMode,
            _: symphonia::core::formats::SeekTo,
        ) -> symphonia::core::errors::Result<symphonia::core::formats::SeekedTo> {
            unimplemented!("not used by the check")
        }

        fn into_inner<'s>(self: Box<Self>) -> MediaSourceStream<'s>
        where
            Self: 's,
        {
            unimplemented!("not used by the check")
        }
    }

    /// Findings 8-7 and missed mutant 8-27 of review round 8: the HE-AAC
    /// check counts only the packets of the audio track, reads at most
    /// `MOST_CHECK_PACKETS` packets, and the first `SBR_CHECK_PACKETS` of the
    /// track only.
    #[test]
    fn the_he_aac_check_takes_the_first_packets_of_the_audio_track() {
        use crate::test_m4a::{adts, adts_packets, with_sbr_fill};
        let stream = adts(&chirps(), 44_100);
        let lc: Vec<Vec<u8>> = adts_packets(&stream).into_iter().map(Vec::from).collect();
        let sbr: Vec<Vec<u8>> = lc.iter().map(|packet| with_sbr_fill(packet)).collect();
        let check = |packets: Vec<(u32, Vec<u8>)>| {
            let mut reader = Scripted(packets.into(), Vec::new());
            first_packets_of_lc(&mut reader, 1)
                .map(|queued| queued.len())
                .map_err(|e| e.to_string())
        };
        let mixed = |audio: &[Vec<u8>], other: &[Vec<u8>]| {
            audio
                .iter()
                .zip(other)
                .flat_map(|(a, o)| [(1, a.clone()), (2, o.clone())])
                .collect()
        };
        assert_eq!(
            check(mixed(&sbr, &lc)),
            Err(HE_AAC_TEXT.to_string()),
            "SBR on the audio track"
        );
        assert_eq!(check(mixed(&lc, &sbr)), Ok(15), "SBR on the other track");
        let first: Vec<(u32, Vec<u8>)> = (0..80).map(|k| (2, sbr[k % sbr.len()].clone())).collect();
        let other_first = [first, sbr.iter().map(|p| (1, p.clone())).collect()].concat();
        assert_eq!(check(other_first), Ok(MOST_CHECK_PACKETS), "the bound");
        let late = [
            sbr.iter().take(8).cloned().collect::<Vec<_>>(),
            (0..80).map(|k| lc[k % lc.len()].clone()).collect(),
        ]
        .concat();
        assert_eq!(
            check(late.into_iter().map(|p| (1, p)).collect()),
            Err(HE_AAC_TEXT.to_string())
        );
    }

    /// Finding 8-8 of review round 8: the head of an M4A after a chain of
    /// boxes past the window states no length, but is no MP3.
    #[test]
    fn the_head_of_an_m4a_after_a_large_box_is_unstated_pcm() {
        use crate::test_m4a::{bx, with_stco_moved};
        let plain = alac_of(m4a::Order::FastStart);
        for kind in [b"free", b"uuid"] {
            for size in [100 << 10, 2 << 20] {
                let prefix = bx(kind, &[&vec![0; size]]);
                let file = [
                    prefix.clone(),
                    with_stco_moved(plain.clone(), prefix.len() as i64),
                ]
                .concat();
                let case = format!("{} of {size}", String::from_utf8_lossy(kind));
                assert_eq!(head_of(&file), HeadLength::UnstatedPcm, "{case}");
            }
        }
        assert_eq!(
            head_of(&[vec![0x42; 2 << 20], plain].concat()),
            HeadLength::Unstated,
            "junk"
        );
    }

    /// Missed mutants of review round 8, each with the validator's input.
    #[test]
    fn the_inputs_of_the_missed_mutants_of_round_8() {
        use crate::test_audio::{Aiff, aiff};
        use crate::test_m4a::{adts, bx, top_box};
        // 8-21: an empty `mdat` before the real one, behind 26 junk bytes.
        // The first chunk of the MP4 is 10 bytes into the real `mdat`, and
        // from the start of the file it falls at the start of the empty
        // one, in no body.
        let plain = alac_of(m4a::Order::MoovLast);
        let (ftyp, ftyp_size) = top_box(&plain, b"ftyp");
        let (mdat, mdat_size) = top_box(&plain, b"mdat");
        let real = [
            &((mdat_size + 10) as u32).to_be_bytes()[..],
            b"mdat",
            &[0; 10],
            &plain[mdat + 8..mdat + mdat_size],
        ]
        .concat();
        let mp4 = [
            &plain[..ftyp + ftyp_size],
            &bx(b"mdat", &[]),
            &bx(b"free", &[]),
            &real,
            &plain[mdat + mdat_size..],
        ]
        .concat();
        let mp4 = crate::test_m4a::with_stco_moved(mp4, 26);
        let want = decoded(mp4.clone(), "an empty mdat").channels;
        let behind = [vec![0x42; 26], mp4].concat();
        assert!(
            decoded(behind, "an empty mdat behind junk").channels == want,
            "8-21"
        );
        // 8-22: a WAV with a sample rate of 0 opens under a frame limit.
        let (_, _, mut wav) = stereo_wav();
        wav[24..32].copy_from_slice(&[0; 8]);
        let opened = Decoder::within_song_frames(Box::new(Cursor::new(wav)), Some("wav"), 1 << 20);
        assert!(opened.is_ok(), "8-22: {:?}", opened.err());
        // 8-24: an ADTS stream of one frame is no stream of raw blocks.
        let stream = adts(&short_tone(), 44_100);
        let length = (usize::from(stream[3] & 3) << 11)
            | (usize::from(stream[4]) << 3)
            | usize::from(stream[5] >> 5);
        let one = Vec::from(&stream[..length]);
        let error = decode_bytes(one, Some("aac"))
            .err()
            .map(|e| format!("{e:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("no known format")),
            "8-24: {error:?}"
        );
        // 8-29 and 8-33: an ADTS stream cut at 700 bytes decodes the whole
        // frames that it holds.
        let cut = &stream[..700];
        let (mut at, mut whole) = (0, 0);
        while at + 7 <= cut.len() {
            let frame = (usize::from(cut[at + 3] & 3) << 11)
                | (usize::from(cut[at + 4]) << 3)
                | usize::from(cut[at + 5] >> 5);
            if at + frame > cut.len() {
                break;
            }
            whole += 1;
            at += frame;
        }
        assert!(whole >= 1 && at < cut.len(), "a cut inside a frame");
        let decoded_cut = decode_bytes(Vec::from(cut), Some("aac"));
        assert!(decoded_cut.is_ok(), "8-29: {:?}", decoded_cut.err());
        assert_eq!(decoded_cut.unwrap().frames(), whole * 1024, "8-29");
        // 8-26: an AIFC cut inside its compression type.
        let (left, right, _) = stereo_wav();
        let aifc = aiff(&[left, right], 44_100, 16, Aiff::UncompressedAifc);
        let at = (0..aifc.len())
            .find(|at| aifc[*at..].starts_with(b"NONE"))
            .unwrap();
        let error = decode_bytes(Vec::from(&aifc[..at + 2]), Some("aiff"))
            .err()
            .map(|e| format!("{e:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("unexpected end of file")),
            "8-26: {error:?}"
        );
        // 8-31: an odd-sized chunk before the COMM of a compressed AIFC.
        let mut mac3 = aifc.clone();
        mac3[at..at + 4].copy_from_slice(b"MAC3");
        let odd = [&b"ANNO"[..], &1_u32.to_be_bytes(), b"a", &[0]].concat();
        let mut odd_first = [&mac3[..12], &odd, &mac3[12..]].concat();
        let form = (odd_first.len() - 8) as u32;
        odd_first[4..8].copy_from_slice(&form.to_be_bytes());
        assert_refused_early(odd_first, "aiff", AIFC_COMPRESSED_TEXT, "8-31");
    }

    /// Findings 8-6 and missed mutant 8-30 of review round 8, and a check
    /// of the validation: each `sidx` that Symphonia would hold too much of,
    /// or refuse, is hidden.
    #[test]
    fn each_sidx_that_symphonia_would_hold_or_refuse_is_hidden() {
        use crate::test_m4a::{bx, same_sidx_boxes};
        let plain = alac_of(m4a::Order::MoovLast);
        let sidx = |scale: u32, references: u16| {
            let mut body = Vec::from([0_u8; 4]);
            body.extend_from_slice(&1_u32.to_be_bytes());
            body.extend_from_slice(&scale.to_be_bytes());
            body.extend_from_slice(&[0; 8]);
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(&references.to_be_bytes());
            body.extend_from_slice(&vec![0; 12 * usize::from(references)]);
            bx(b"sidx", &[&body])
        };
        // Past MOST_SMALL, the id is seen.  Then a large `sidx` of it, and a
        // small one of another timescale.
        let seen = same_sidx_boxes(crate::hidden::MOST_SMALL + 1);
        let large = [plain.clone(), seen.clone(), sidx(1000, 65_535)].concat();
        assert_tags_are_hidden(plain.clone(), large, "m4a", "a large sidx of a seen id");
        let other = [plain.clone(), seen, sidx(2000, 0)].concat();
        assert_tags_are_hidden(plain.clone(), other, "m4a", "a sidx of another timescale");
        let three = [plain.clone(), sidx(1000, 0), sidx(2000, 0), sidx(1000, 0)].concat();
        assert_tags_are_hidden(plain, three, "m4a", "timescales 1000, 2000, 1000");
    }

    /// B of review round 8: an M4A cut inside its last `mdat` decodes the
    /// packets that it holds.  Symphonia skips a top-level box that ends past
    /// the end of the stream (`atoms/mod.rs`, line 671), so it found no
    /// `mdat` ("no atom pending read"), also at fca7f18.  The view gives that
    /// `mdat` a size of 0: to the end of the file.
    #[test]
    fn an_m4a_cut_inside_its_mdat_decodes_what_it_holds() {
        use crate::test_m4a::{aac_m4a, top_box};
        let aac = aac_m4a(&chirps(), 44_100, None, None, m4a::Order::FastStart);
        let tone = chirps();
        let alac =
            crate::test_m4a::alac_m4a(&tone[0], &tone[1], 44_100, None, m4a::Order::FastStart);
        for (codec, file) in [("aac", aac), ("alac", alac)] {
            let whole = decoded(file.clone(), codec).channels;
            let (mdat, size) = top_box(&file, b"mdat");
            for cut in [1, 100, 1000, 3001, size / 3, size * 2 / 3, size - 9] {
                let case = format!("{codec} cut {cut} bytes into its mdat");
                let got = decode_bytes(Vec::from(&file[..mdat + 8 + cut]), Some("m4a"));
                let error = got.as_ref().err().map(|e| format!("{e:#}"));
                let nothing_held = error.as_deref() == Some("file decoded to no audio");
                assert!(
                    got.is_ok() || (cut < size / 3 && nothing_held),
                    "{case}: {error:?}"
                );
                if let Ok(audio) = got {
                    let frames = audio.frames();
                    assert!(frames > 0, "{case}");
                    let prefix: Vec<&[f32]> = whole.iter().map(|c| &c[..frames]).collect();
                    let held: Vec<&[f32]> = audio.channels.iter().map(Vec::as_slice).collect();
                    assert!(held == prefix, "{case}: {frames} frames");
                }
            }
        }
    }

    /// Finding 8-3 of review round 8 (TD-86): an ADTS frame with a CRC states
    /// where each of its raw data blocks starts, so the decode gives each
    /// block as its own frame.  Streams of 2, 3 and 4 blocks in each frame,
    /// one that starts with frames of one block, and one from a source that
    /// cannot seek decode bit for bit.  A frame of more blocks with no CRC
    /// gives `ADTS_BLOCKS_TEXT`, first or later.
    #[test]
    fn an_adts_stream_of_raw_blocks_with_a_crc_decodes() {
        use crate::test_m4a::{adts, adts_in_blocks, adts_packets};
        let stream = adts(&chirps(), 44_100);
        let packets = adts_packets(&stream);
        let want = decoded(
            adts_in_blocks(&packets, 44_100, 2, &[1], false),
            "one block",
        )
        .channels;
        for per_frame in [&[2][..], &[3], &[4], &[1, 1, 3]] {
            let case = format!("{per_frame:?} blocks");
            let file = adts_in_blocks(&packets, 44_100, 2, per_frame, true);
            assert_eq!(head_of(&file), HeadLength::UnstatedPcm, "{case}");
            let got = decode_bytes(file.clone(), Some("aac"));
            assert!(got.is_ok(), "{case}: {:?}", got.err());
            assert!(got.unwrap().channels == want, "{case}");
            let fifo = decode_source(Box::new(Fifo(Cursor::new(file))), Some("aac"));
            assert!(
                fifo.is_ok(),
                "{case}, a source that cannot seek: {:?}",
                fifo.err()
            );
            assert!(
                fifo.unwrap().channels == want,
                "{case}, a source that cannot seek"
            );
        }
        let later = adts_in_blocks(&packets, 44_100, 2, &[1, 1, 2], false);
        let error = decode_bytes(later, Some("aac"))
            .err()
            .map(|e| format!("{e:#}"));
        assert!(
            error.is_some_and(|e| e.contains(ADTS_BLOCKS_TEXT)),
            "a later frame of 2 with no CRC"
        );
    }

    /// Finding 8-5 of review round 8: Symphonia 0.6.1 decodes Apple's IMA
    /// ADPCM (`ima4`, `symphonia-codec-adpcm` `CODEC_ID_ADPCM_IMA_QT`), but its
    /// AIFF reader gives no such codec (`aiff/chunks.rs`, line 287).  The
    /// decode reads the blocks of such an AIFC itself, and gives them to that
    /// decoder.  Mono and stereo decode to the output of the encoder of the
    /// test, also with tags and with an offset in SSND.
    #[test]
    fn an_ima4_aifc_decodes() {
        use crate::test_audio::ima4_aifc;
        let tone = chirps();
        let frames = 64 * 300;
        let left = Vec::from(&tone[0][..frames]);
        let right = Vec::from(&tone[1][..frames]);
        for channels in [vec![left.clone()], vec![left, right]] {
            let case = format!("{} channels", channels.len());
            let (file, want) = ima4_aifc(&channels, 44_100);
            let tagged = crate::test_audio::with_aiff_tags(&file, 1 << 20);
            let aligned = crate::test_audio::with_block_aligned_ssnd(&file, 4096);
            for (form, file) in [("plain", file), ("tags", tagged), ("offset", aligned)] {
                let got = decode_bytes(file, Some("aiff"));
                assert!(got.is_ok(), "{case}, {form}: {:?}", got.err());
                let got = got.unwrap();
                assert_eq!(got.sample_rate, 44_100, "{case}, {form}");
                assert!(got.channels == want, "{case}, {form}");
            }
        }
    }

    /// Findings 9-2, 9-6 and 9-25 of review round 9 (TD-86): the split takes
    /// each frame after bytes that are not a frame, and a cut last frame of
    /// more blocks gives the whole blocks that it holds.  A split stream
    /// keeps the length of the file for its reserve.
    #[test]
    fn a_split_adts_stream_takes_junk_and_a_cut_end() {
        use crate::test_m4a::{adts, adts_in_blocks, adts_packets};
        let stream = adts(&chirps(), 44_100);
        let packets = adts_packets(&stream);
        let whole = decoded(
            adts_in_blocks(&packets, 44_100, 2, &[1], false),
            "one block",
        )
        .channels;
        let (first, rest) = packets.split_at(10);
        for junk in [vec![0x00], vec![0x42; 3]] {
            let file = [
                adts_in_blocks(first, 44_100, 2, &[1], false),
                junk.clone(),
                adts_in_blocks(rest, 44_100, 2, &[2], true),
            ]
            .concat();
            let case = format!("{} junk bytes between frames", junk.len());
            let got = decode_bytes(file, Some("aac"));
            assert!(got.is_ok(), "{case}: {:?}", got.err());
            assert!(got.unwrap().channels == whole, "{case}");
        }
        let three = adts_in_blocks(&packets, 44_100, 2, &[3], true);
        let reserve =
            Decoder::new(Box::new(Cursor::new(three.clone())), Some("aac")).map(|d| d.reserve);
        assert!(
            reserve.as_ref().is_ok_and(|r| *r > 0),
            "the reserve: {reserve:?}"
        );
        for cut in [1, 2, 5, 100] {
            let case = format!("cut by {cut} bytes");
            let got = decode_bytes(Vec::from(&three[..three.len() - cut]), Some("aac"));
            assert!(got.is_ok(), "{case}: {:?}", got.err());
            let got = got.unwrap();
            let frames = got.frames();
            assert!(
                frames > 0 && frames.is_multiple_of(1024) && frames < whole[0].len(),
                "{case}: {frames}"
            );
            let prefix: Vec<&[f32]> = whole.iter().map(|c| &c[..frames]).collect();
            let held: Vec<&[f32]> = got.channels.iter().map(Vec::as_slice).collect();
            assert!(held == prefix, "{case}");
        }
    }

    /// Findings 9-1 and 9-7, and missed mutants 9-18, 9-20 and 9-26 of review
    /// round 9: an `ima4` AIFC with `SSND` before `COMM`, and of 3 and 6
    /// channels, decodes to the encoder's output.  Its head states the
    /// frames that it decodes: `COMM` counts blocks of 64 frames.  A rate of 0
    /// is refused, a partial last block is dropped, and a file cut inside its
    /// `SSND` offset is refused with the AIFC text.
    #[test]
    fn each_ima4_aifc_decodes_and_states_its_length() {
        use crate::test_audio::{ima4_aifc, with_ssnd_first};
        let tone = chirps();
        let frames = 64 * 300;
        let channel = |k: usize| Vec::from(&tone[k % 2][k * 64..k * 64 + frames]);
        for rate in [44_100, 48_000] {
            for count in [1, 2, 3, 6] {
                let channels: Vec<Vec<f32>> = (0..count).map(channel).collect();
                let (file, want) = ima4_aifc(&channels, rate);
                let case = format!("{count} channels at {rate} Hz");
                assert_eq!(
                    head_of(&file),
                    HeadLength::Frames(song_frames(frames as u64, rate)),
                    "{case}"
                );
                for (form, file) in [
                    ("COMM first", file.clone()),
                    ("SSND first", with_ssnd_first(&file)),
                ] {
                    let got = decode_bytes(file, Some("aiff"));
                    assert!(got.is_ok(), "{case}, {form}: {:?}", got.err());
                    assert!(got.unwrap().channels == want, "{case}, {form}");
                }
            }
        }
        let (file, want) = ima4_aifc(&[channel(0)], 44_100);
        // A `COMM` that states more blocks than `SSND` holds.
        let comm = (0..file.len())
            .find(|at| file[*at..].starts_with(b"COMM"))
            .unwrap();
        let mut more = file.clone();
        more[comm + 10..comm + 14].copy_from_slice(&600_u32.to_be_bytes());
        assert_eq!(
            head_of(&more),
            HeadLength::Frames(song_frames(frames as u64, 44_100)),
            "more blocks than held"
        );
        // 9-18: a rate of 0.
        let comm = (0..file.len())
            .find(|at| file[*at..].starts_with(b"COMM"))
            .unwrap();
        let mut rate_0 = file.clone();
        rate_0[comm + 16..comm + 26].copy_from_slice(&[0; 10]);
        assert!(decode_bytes(rate_0, Some("aiff")).is_err(), "a rate of 0");
        // 9-20: a partial last block.
        let ssnd = (0..file.len())
            .find(|at| file[*at..].starts_with(b"SSND"))
            .unwrap();
        let mut partial = [&file[..], &[0x11; 10]].concat();
        let size = u32::from_be_bytes(partial[ssnd + 4..ssnd + 8].try_into().unwrap()) + 10;
        partial[ssnd + 4..ssnd + 8].copy_from_slice(&size.to_be_bytes());
        let form = (partial.len() - 8) as u32;
        partial[4..8].copy_from_slice(&form.to_be_bytes());
        let got = decode_bytes(partial, Some("aiff"));
        assert!(
            got.is_ok_and(|audio| audio.channels == want),
            "a partial last block"
        );
        // 9-26: cut inside the `SSND` offset.
        let cut = Vec::from(&file[..ssnd + 10]);
        assert_refused_early(cut, "aiff", AIFC_COMPRESSED_TEXT, "cut in the SSND offset");
    }

    /// Finding 9-4 of review round 9: the patches of the `sidx` boxes of a
    /// seen id with another timescale stop at the kept limit, and the file is
    /// refused.
    #[test]
    fn the_patches_of_sidx_boxes_have_a_bound() {
        use crate::test_m4a::{bx, same_sidx_boxes};
        let plain = alac_of(m4a::Order::MoovLast);
        let other = bx(
            b"sidx",
            &[
                &[0; 4],
                &1_u32.to_be_bytes(),
                &2000_u32.to_be_bytes(),
                &[0; 12],
            ],
        );
        let file = [
            plain,
            same_sidx_boxes(crate::hidden::MOST_SMALL + 1),
            other.repeat(200_000),
        ]
        .concat();
        let found = crate::mp4_boxes::walk(&mut file.as_slice()).ok().flatten();
        let patches = found.as_ref().map_or(0, |found| found.hidden.patches.len());
        assert!(
            found.is_some_and(|found| found.hidden.too_much_kept),
            "refused"
        );
        assert!(
            patches < 2 * crate::hidden::MOST_SMALL + 10_000,
            "{patches} patches"
        );
        assert_refused_early(file, "m4a", TAGS_TEXT, "200,000 sidx boxes");
    }

    /// Missed mutant 9-23 of review round 9: a first box with a 64-bit size of
    /// 32 is an MP4 box; one of 8, under its header of 16, is not.
    #[test]
    fn a_first_box_of_a_64_bit_size_must_hold_its_header() {
        let head = |size: u64| {
            [
                &1_u32.to_be_bytes()[..],
                b"free",
                &size.to_be_bytes(),
                &[0x42; 64],
            ]
            .concat()
        };
        assert_eq!(head_of(&head(32)), HeadLength::UnstatedPcm);
        assert_eq!(head_of(&head(8)), HeadLength::Unstated);
    }

    /// Finding 9-3 of review round 9: the feature `adpcm` makes Symphonia
    /// decode IMA and Microsoft ADPCM in a WAV too, on purpose.  Mono and
    /// stereo of each decode to the output of the encoder of the test, and
    /// the head states the frames that they decode: blocks times the frames
    /// of a block.
    #[test]
    fn an_adpcm_wav_decodes_and_states_its_length() {
        use crate::test_audio::{WavAdpcm, adpcm_wav};
        let tone = chirps();
        for kind in [WavAdpcm::Ima, WavAdpcm::Ms] {
            for channels in [vec![tone[0].clone()], tone.clone()] {
                let case = format!("{kind:?}, {} channels", channels.len());
                let (file, want) = adpcm_wav(&channels, 44_100, kind);
                let got = decode_bytes(file.clone(), Some("wav"));
                assert!(got.is_ok(), "{case}: {:?}", got.err());
                let got = got.unwrap();
                assert!(got.channels == want, "{case}");
                let frames = want[0].len() as u64;
                assert_eq!(
                    head_of(&file),
                    HeadLength::Frames(song_frames(frames, 44_100)),
                    "{case}"
                );
            }
        }
    }

    /// A Microsoft ADPCM WAV of `channels` channels and one block whose step
    /// grows past `i32`: a preamble step of `step`, then nibbles of the
    /// largest adaptation (8: 768 / 256, 3 times the step for each nibble).
    fn growing_ms_adpcm(channels: usize, step: i16) -> Vec<u8> {
        use crate::test_audio::{WavAdpcm, adpcm_wav};
        let (mut file, _) = adpcm_wav(&vec![vec![0.0; 500]; channels], 44_100, WavAdpcm::Ms);
        let data = (0..file.len())
            .find(|at| file[*at..].starts_with(b"data"))
            .unwrap()
            + 8;
        // The predictor of each channel, then the step of each channel.
        for channel in 0..channels {
            let at = data + channels + 2 * channel;
            file[at..at + 2].copy_from_slice(&step.to_le_bytes());
        }
        for byte in &mut file[data + 7 * channels..data + 256 * channels] {
            *byte = 0x88;
        }
        file
    }

    /// Findings 10-2 and 10-3 of review round 10 (TD-87, closed): the step
    /// of Microsoft ADPCM triples for each nibble of 8 from any start, and
    /// Symphonia multiplies it with no bound (`symphonia-codec-adpcm`
    /// `codec_ms.rs`, line 96).  The decode walks the steps of each block
    /// before Symphonia decodes it, and refuses the file with a text.  It
    /// runs in each build: no panic, also with overflow checks.
    #[test]
    fn a_crafted_ms_adpcm_wav_is_refused_with_no_panic() {
        for channels in [1, 2] {
            for step in [16, 32_767] {
                let case = format!("{channels} channels from a step of {step}");
                let file = growing_ms_adpcm(channels, step);
                let decoded = std::panic::catch_unwind(|| {
                    decode_bytes(file, Some("wav"))
                        .err()
                        .map(|e| format!("{e:#}"))
                });
                assert!(decoded.is_ok(), "{case}: a panic");
                let error = decoded.unwrap();
                assert!(
                    error.as_deref().is_some_and(|e| e.contains(MS_ADPCM_TEXT)),
                    "{case}: {error:?}"
                );
            }
        }
    }

    /// The samples of `bytes` through Symphonia's own probe and decoder: the
    /// path of the decode on `main`.
    fn through_symphonia(bytes: Vec<u8>) -> Vec<Vec<f32>> {
        use symphonia::core::formats::probe::Hint;
        let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
        let probe = symphonia::default::get_probe();
        let mut format = probe
            .probe(&Hint::new(), mss, Default::default(), Default::default())
            .expect("the probe opens it");
        let track = format
            .default_track(TrackType::Audio)
            .expect("an audio track");
        let id = track.id;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .expect("audio")
            .clone();
        let mut codec = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .expect("a decoder");
        let mut out: Vec<Vec<f32>> = Vec::new();
        let mut planes: Vec<Vec<f32>> = Vec::new();
        while let Ok(Some(packet)) = format.next_packet() {
            if packet.track_id != id {
                continue;
            }
            let Ok(buf) = codec.decode(&packet) else {
                continue;
            };
            buf.copy_to_vecs_planar::<f32>(&mut planes);
            out.resize(planes.len(), Vec::new());
            for (all, plane) in out.iter_mut().zip(&planes) {
                all.extend_from_slice(plane);
            }
        }
        out
    }

    /// The orchestrator's check of review round 9: an MP3, which `main`
    /// opens, decodes to the samples of Symphonia's own path, with no tags
    /// and with an ID3v2 tag of a title (`TIT2`) and a small picture
    /// (`APIC`).  The file with the tag decodes to the same samples as the
    /// one with none.
    #[test]
    fn an_mp3_decodes_as_on_main_with_and_without_a_tag() {
        use crate::test_audio::{mp3, tone};
        let frame = |id: &[u8; 4], body: &[u8]| {
            let size = body.len() as u32;
            let synchsafe = [21, 14, 7, 0].map(|shift| ((size >> shift) & 0x7F) as u8);
            [&id[..], &synchsafe, &[0, 0], body].concat()
        };
        let title = frame(b"TIT2", &[&[3][..], b"lloyd"].concat());
        let picture = frame(
            b"APIC",
            &[&[3][..], b"image/png\0", &[3], b"\0", &[0x89; 200]].concat(),
        );
        let frames = [title, picture].concat();
        let size = frames.len() as u32;
        let synchsafe = [21, 14, 7, 0].map(|shift| ((size >> shift) & 0x7F) as u8);
        let tag = [&b"ID3"[..], &[4, 0, 0], &synchsafe, &frames].concat();
        let tones = [tone(0, 44_100, 44_100), tone(1, 44_100, 44_100)];
        let plain = mp3(&tones, 44_100);
        let tagged = [tag, plain.clone()].concat();
        let want = through_symphonia(plain.clone());
        assert!(
            !want.is_empty() && want[0].len() > 44_100,
            "the path of main decodes it"
        );
        for (case, file) in [("no tags", plain), ("an ID3v2 tag", tagged)] {
            let main_path = through_symphonia(file.clone());
            let got = decoded_as(file, "mp3", case);
            assert!(got == main_path, "{case}: as on main");
            assert!(got == want, "{case}: as with no tags");
        }
    }

    /// The channels of `file` as `ext`, which must decode.
    fn decoded_as(file: Vec<u8>, ext: &str, case: &str) -> Vec<Vec<f32>> {
        let audio = decode_bytes(file, Some(ext));
        assert!(audio.is_ok(), "{case}: {:?}", audio.err());
        audio.unwrap().channels
    }

    /// The channels of the `COMM` of `file`, an AIFF or AIFC, set to `count`.
    fn with_comm_channels(mut file: Vec<u8>, count: u16) -> Vec<u8> {
        let comm = (0..file.len())
            .find(|at| file[*at..].starts_with(b"COMM"))
            .unwrap();
        file[comm + 8..comm + 10].copy_from_slice(&count.to_be_bytes());
        file
    }

    /// Finding 10-1 of review round 10 (MEM-4): a file of more than
    /// `MOST_CHANNELS` channels is refused before any decoder is made, with
    /// a text.  At the bound, the open of an `ima4` AIFC and of a PCM AIFF
    /// peaks under 8 MiB.  The head of such a file states no length.
    #[test]
    fn a_file_of_too_many_channels_is_refused_before_its_decoder() {
        use crate::test_audio::{Aiff, aiff, ima4_aifc};
        let tone: Vec<f32> = (0..64 * 4).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let most = usize::from(MOST_CHANNELS);
        let (ima4, _) = ima4_aifc(&vec![tone.clone(); most], 44_100);
        let pcm = aiff(&vec![tone.clone(); most], 44_100, 16, Aiff::Plain);
        for (case, file) in [("ima4", ima4), ("PCM", pcm)] {
            let (peak, opened) = peak_of(|| {
                Decoder::new(Box::new(Cursor::new(file.clone())), Some("aiff")).map(|_| ())
            });
            assert!(opened.is_ok(), "{case} at the bound: {:?}", opened.err());
            assert!(
                peak < 8 << 20,
                "{case} at the bound: the open peaks at {peak} bytes"
            );
            for count in [MOST_CHANNELS + 1, u16::MAX] {
                let many = with_comm_channels(file.clone(), count);
                assert_eq!(head_of(&many), HeadLength::UnstatedPcm, "{case} of {count}");
                let text = channels_text(u32::from(count));
                assert_refused_early(many, "aiff", &text, &format!("{case} of {count} channels"));
            }
        }
    }

    /// Missed mutants 10-12, 10-14, 10-15, 10-19 and 10-22 of review round
    /// 10: the head and the open of AIFF of no channel, of a cut AIFC, and of
    /// a whole mono AIFF.
    #[test]
    fn the_heads_and_opens_of_aiff_edge_cases() {
        use crate::test_audio::{Aiff, aiff, ima4_aifc};
        let tone: Vec<f32> = (0..64 * 100)
            .map(|i| (i as f32 * 0.01).sin() * 0.5)
            .collect();
        // 10-22: a whole mono 16-bit AIFF.
        let mono = aiff(std::slice::from_ref(&tone), 44_100, 16, Aiff::Plain);
        assert_eq!(
            head_of(&mono),
            HeadLength::Frames(6400),
            "a whole mono AIFF"
        );
        // 10-14 and 10-15: an AIFC NONE whose COMM states more frames than it
        // holds keeps its stated frames.
        let aifc = aiff(
            std::slice::from_ref(&tone),
            44_100,
            16,
            Aiff::UncompressedAifc,
        );
        let comm = (0..aifc.len())
            .find(|at| aifc[*at..].starts_with(b"COMM"))
            .unwrap();
        let mut more = aifc.clone();
        more[comm + 10..comm + 14].copy_from_slice(&1_000_000_u32.to_be_bytes());
        assert_eq!(
            head_of(&more),
            HeadLength::Frames(1_000_000),
            "a cut AIFC NONE"
        );
        // 10-12 and 10-19: no channel, in the head and in the open.
        let (ima4, _) = ima4_aifc(&[tone], 44_100);
        for (case, file) in [("ima4", ima4), ("AIFF", mono)] {
            let none = with_comm_channels(file, 0);
            assert_eq!(
                head_of(&none),
                HeadLength::UnstatedPcm,
                "{case} of no channel"
            );
            let opened = std::panic::catch_unwind(|| decode_bytes(none, Some("aiff")).is_err());
            assert_eq!(
                opened.ok(),
                Some(true),
                "{case} of no channel is refused with no panic"
            );
        }
    }
}
