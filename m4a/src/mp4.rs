//! The boxes of an M4A file: `ftyp`, `moov` and the `mdat` header.
//!
//! The caller writes `head_len` zeros, then each packet, then the final
//! head at position 0.  This is the order of `FlacOut::replace_header` in
//! `worker/src/web/encode.rs`.  With `Order::MoovLast`, the caller writes
//! the tail (the `moov`) after the last packet.

use core::fmt;

use crate::alac::{FRAMES_PER_PACKET, MAX_PACKET_BYTES};

/// The codec of the one track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// 16-bit stereo Apple Lossless: 4096 frames in a packet.
    Alac,
    /// AAC: 1024 frames in a packet, with `channels` channels and an
    /// AudioSpecificConfig of `config_len` bytes.
    Aac { channels: u16, config_len: u8 },
}

/// The frames in one AAC packet of the writer.  `Codec::Aac` has no form
/// of 960 frames.
const AAC_FRAMES_PER_PACKET: u32 = 1024;

/// The largest AAC packet of one channel: 6144 bits, the decoder input
/// buffer of each channel (ISO/IEC 14496-3, 4.5.3.2).
const AAC_BYTES_PER_CHANNEL: usize = 6144 / 8;

/// The longest AudioSpecificConfig that the writer takes.  With it, each
/// descriptor of `esds` keeps its size in one byte.
pub const MAX_CONFIG_LEN: u8 = 64;

impl Codec {
    /// The frames in each packet but the last: 4096 or 1024.
    pub fn frames_per_packet(self) -> u32 {
        match self {
            Codec::Alac => FRAMES_PER_PACKET as u32,
            Codec::Aac { .. } => AAC_FRAMES_PER_PACKET,
        }
    }

    /// The largest packet: `MAX_PACKET_BYTES` for ALAC, and 768 bytes
    /// (6144 bits) for each channel of AAC (ISO/IEC 14496-3, 4.5.3.2).
    pub fn max_packet_bytes(self) -> usize {
        match self {
            Codec::Alac => MAX_PACKET_BYTES,
            Codec::Aac { channels, .. } => AAC_BYTES_PER_CHANNEL * usize::from(channels),
        }
    }

    /// The channel count of the sample entry: 2 for ALAC.
    pub fn channels(self) -> u16 {
        match self {
            Codec::Alac => 2,
            Codec::Aac { channels, .. } => channels,
        }
    }

    /// The size of the sample entry of this codec.
    fn entry_len(self) -> usize {
        match self {
            Codec::Alac => ALAC_ENTRY_LEN,
            Codec::Aac { config_len, .. } => AAC_ENTRY_LEN_WITHOUT_CONFIG + usize::from(config_len),
        }
    }

    /// Whether `entry` is of this codec, with an AudioSpecificConfig of
    /// `config_len` bytes for AAC.
    fn takes(self, entry: &SampleEntry) -> bool {
        match (self, entry) {
            (Codec::Alac, SampleEntry::Alac { .. }) => true,
            (Codec::Aac { config_len, .. }, SampleEntry::Aac { config }) => {
                config.len() == usize::from(config_len)
            }
            _ => false,
        }
    }
}

/// The sample entry of the one track.  M2 added the `esds` entry for AAC,
/// for the AAC files of its decode tests.  M4 uses it for the save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SampleEntry {
    /// Apple Lossless, with its 24-byte `ALACSpecificConfig`.
    Alac { cookie: [u8; 24] },
    /// `mp4a` with an `esds` that holds `config`, the AudioSpecificConfig.
    Aac { config: Vec<u8> },
}

/// One `elst` entry: play `frames` frames from frame `skip` of the track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    pub skip: u32,
    pub frames: u64,
}

/// Where the `moov` goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// `moov` before `mdat`, so a reader needs no seek to the end.
    FastStart,
    /// `moov` after `mdat`.  Only tests use it, as a file that needs a seek.
    MoovLast,
}

/// Why the writer refuses a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The sample entry holds the rate as 16.16 fixed point.
    SampleRate(u32),
    /// The edit plays frames that the track does not have.
    EditPastEnd { edit: Edit, frames: u64 },
    /// The file can reach 4 GiB, past the 32-bit offsets of `stco`.
    TooLarge { frames: u64, worst_case_bytes: u128 },
    /// `head` or `tail` got a count of packet sizes that is not `packets()`.
    PacketCount { expected: usize, got: usize },
    /// A packet is larger than the codec's `max_packet_bytes`, `most`.
    PacketSize {
        index: usize,
        size: u32,
        most: usize,
    },
    /// An AAC layout of 0 channels.
    NoChannels,
    /// An AudioSpecificConfig length of 0, or above `MAX_CONFIG_LEN`.
    ConfigLen(u8),
    /// `head` or `tail` got a sample entry of another codec, or an
    /// AudioSpecificConfig of a length that is not the layout's.
    EntryCodec,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::SampleRate(rate) => {
                write!(f, "an M4A rate is 1 to 65,535 Hz, not {rate} Hz")
            }
            Error::EditPastEnd { edit, frames } => write!(
                f,
                "the edit plays {} frames from frame {}, past the end of {frames} frames",
                edit.frames, edit.skip
            ),
            Error::TooLarge {
                frames,
                worst_case_bytes,
            } => write!(
                f,
                "an M4A of {frames} frames can reach {worst_case_bytes} bytes, past the limit of 4 GiB"
            ),
            Error::PacketCount { expected, got } => {
                write!(f, "the file has {expected} packets, not {got}")
            }
            Error::PacketSize { index, size, most } => {
                write!(f, "packet {index} has {size} bytes, more than {most}")
            }
            Error::NoChannels => write!(f, "an AAC track has 1 channel or more, not 0"),
            Error::ConfigLen(len) => write!(
                f,
                "an AudioSpecificConfig has 1 to {MAX_CONFIG_LEN} bytes, not {len}"
            ),
            Error::EntryCodec => write!(
                f,
                "the sample entry is not of the codec of the layout, or its AudioSpecificConfig has another length"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// The shape of one file: its codec, frames, rate, edit and order.  It
/// gives the size of the head before the first packet, and the final
/// bytes after the last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    codec: Codec,
    frames: u64,
    sample_rate: u32,
    edit: Option<Edit>,
    order: Order,
    packets: usize,
}

const FTYP_LEN: usize = 8 + 4 + 4 + 3 * 4;
const MDAT_HEADER_LEN: usize = 8;
const MVHD_LEN: usize = 108;
const TKHD_LEN: usize = 92;
const ELST_LEN: usize = 8 + 4 + 4 + 12;
const EDTS_LEN: usize = 8 + ELST_LEN;
const MDHD_LEN: usize = 32;
const HANDLER_NAME: &[u8] = b"SoundHandler\0";
const HDLR_LEN: usize = 8 + 4 + 4 + 4 + 12 + HANDLER_NAME.len();
const SMHD_LEN: usize = 16;
const DINF_LEN: usize = 8 + 8 + 4 + 4 + 12;
/// The fields of an `AudioSampleEntry` with its box header.
const AUDIO_ENTRY_FIELDS_LEN: usize = 36;
const ALAC_ENTRY_LEN: usize = AUDIO_ENTRY_FIELDS_LEN + 8 + 4 + 24;
/// The `DecoderSpecificInfo` descriptor without the AudioSpecificConfig.
const DECODER_SPECIFIC_LEN: usize = 2;
const DECODER_CONFIG_LEN_WITHOUT_CONFIG: usize = 2 + 13 + DECODER_SPECIFIC_LEN;
const SL_CONFIG_LEN: usize = 3;
const ES_DESCRIPTOR_LEN_WITHOUT_CONFIG: usize =
    2 + 3 + DECODER_CONFIG_LEN_WITHOUT_CONFIG + SL_CONFIG_LEN;
const AAC_ENTRY_LEN_WITHOUT_CONFIG: usize =
    AUDIO_ENTRY_FIELDS_LEN + 8 + 4 + ES_DESCRIPTOR_LEN_WITHOUT_CONFIG;
/// The `stsd` without its one entry.
const STSD_LEN_WITHOUT_ENTRY: usize = 8 + 4 + 4;

/// The descriptor tags of `esds` (ISO/IEC 14496-1, 7.2.2.1).
const ES_DESCRIPTOR_TAG: u8 = 3;
const DECODER_CONFIG_TAG: u8 = 4;
const DECODER_SPECIFIC_TAG: u8 = 5;
const SL_CONFIG_TAG: u8 = 6;
/// `objectTypeIndication` 0x40: Audio ISO/IEC 14496-3.
const OBJECT_TYPE_AUDIO: u8 = 0x40;
/// `streamType` 5 (audio) in the top 6 bits, `upStream` 0, then the
/// reserved bit 1.
const STREAM_TYPE_AUDIO: u8 = (5 << 2) | 1;
/// `SLConfigDescriptor` `predefined` 2: the form of MP4 files.
const SL_PREDEFINED_MP4: u8 = 2;

/// 1.0 as 16.16 fixed point.
const FIXED_ONE: u32 = 0x0001_0000;
/// 1.0 as 8.8 fixed point.
const VOLUME_ONE: u16 = 0x0100;
const IDENTITY_MATRIX: [u32; 9] = [FIXED_ONE, 0, 0, 0, FIXED_ONE, 0, 0, 0, 0x4000_0000];
/// "und", 3 letters of 5 bits each, each letter minus 0x60.
const LANGUAGE_UND: u16 = (21 << 10) | (14 << 5) | 4;
/// `tkhd` flags: enabled, in the movie, in the preview.
const TRACK_FLAGS: u32 = 7;
/// `url ` flags: the data is in this file.
const SELF_CONTAINED: u32 = 1;

impl Layout {
    /// Refuses a rate of 0 or above 65,535, an AAC codec of 0 channels or
    /// of a config length outside 1 to `MAX_CONFIG_LEN`, an edit past the
    /// end, and a file that can reach 4 GiB.
    pub fn new(
        codec: Codec,
        frames: u64,
        sample_rate: u32,
        edit: Option<Edit>,
        order: Order,
    ) -> Result<Layout, Error> {
        if !(1..=u32::from(u16::MAX)).contains(&sample_rate) {
            return Err(Error::SampleRate(sample_rate));
        }
        if let Codec::Aac {
            channels,
            config_len,
        } = codec
        {
            if channels == 0 {
                return Err(Error::NoChannels);
            }
            if !(1..=MAX_CONFIG_LEN).contains(&config_len) {
                return Err(Error::ConfigLen(config_len));
            }
        }
        let packets = u128::from(frames.div_ceil(u64::from(codec.frames_per_packet())));
        let stts_entries = stts_entry_count(codec, frames) as u128;
        let fixed = (FTYP_LEN
            + MDAT_HEADER_LEN
            + Self::moov_len_without_tables(codec, edit.is_some())) as u128;
        let worst_case_bytes =
            fixed + 8 * stts_entries + packets * (8 + codec.max_packet_bytes() as u128);
        if worst_case_bytes >= 1 << 32 {
            return Err(Error::TooLarge {
                frames,
                worst_case_bytes,
            });
        }
        if let Some(edit) = edit
            && edit
                .frames
                .checked_add(u64::from(edit.skip))
                .is_none_or(|end| end > frames)
        {
            return Err(Error::EditPastEnd { edit, frames });
        }
        Ok(Layout {
            codec,
            frames,
            sample_rate,
            edit,
            order,
            packets: packets as usize,
        })
    }

    /// The number of packets: each holds the codec's `frames_per_packet`,
    /// and the last can hold fewer.
    pub fn packets(&self) -> usize {
        self.packets
    }

    /// The size of the head: `ftyp`, the `moov` with `FastStart`, and the
    /// `mdat` header.
    pub fn head_len(&self) -> usize {
        match self.order {
            Order::FastStart => FTYP_LEN + self.moov_len() + MDAT_HEADER_LEN,
            Order::MoovLast => FTYP_LEN + MDAT_HEADER_LEN,
        }
    }

    /// The size of the tail: the `moov` with `MoovLast`, else 0.
    pub fn tail_len(&self) -> usize {
        match self.order {
            Order::FastStart => 0,
            Order::MoovLast => self.moov_len(),
        }
    }

    /// The final head.  `packet_sizes` holds one size for each packet, in
    /// order.  Its length is always `head_len()`.
    pub fn head(&self, entry: &SampleEntry, packet_sizes: &[u32]) -> Result<Vec<u8>, Error> {
        self.check(entry, packet_sizes)?;
        let mut out = Boxes(Vec::with_capacity(self.head_len()));
        write_ftyp(&mut out);
        if self.order == Order::FastStart {
            self.write_moov(&mut out, entry, packet_sizes);
        }
        let mdat_len =
            MDAT_HEADER_LEN as u64 + packet_sizes.iter().map(|&s| u64::from(s)).sum::<u64>();
        out.u32(u32::try_from(mdat_len).expect("Layout::new keeps the file below 4 GiB"));
        out.kind(b"mdat");
        Ok(out.0)
    }

    /// The `moov` that follows the last packet, with `MoovLast`.  Empty
    /// with `FastStart`.  Its length is always `tail_len()`.
    pub fn tail(&self, entry: &SampleEntry, packet_sizes: &[u32]) -> Result<Vec<u8>, Error> {
        self.check(entry, packet_sizes)?;
        let mut out = Boxes(Vec::with_capacity(self.tail_len()));
        if self.order == Order::MoovLast {
            self.write_moov(&mut out, entry, packet_sizes);
        }
        Ok(out.0)
    }

    fn check(&self, entry: &SampleEntry, packet_sizes: &[u32]) -> Result<(), Error> {
        if !self.codec.takes(entry) {
            return Err(Error::EntryCodec);
        }
        if packet_sizes.len() != self.packets {
            return Err(Error::PacketCount {
                expected: self.packets,
                got: packet_sizes.len(),
            });
        }
        let most = self.codec.max_packet_bytes();
        match packet_sizes.iter().position(|&size| size as usize > most) {
            Some(index) => Err(Error::PacketSize {
                index,
                size: packet_sizes[index],
                most,
            }),
            None => Ok(()),
        }
    }

    /// The `stts` entries: (packets, frames in each).
    fn stts_entries(&self) -> Vec<(u32, u32)> {
        let frames_per_packet = u64::from(self.codec.frames_per_packet());
        let full = self.frames / frames_per_packet;
        let rest = self.frames % frames_per_packet;
        let mut entries = Vec::with_capacity(2);
        if full > 0 {
            entries.push((to_u32(full), self.codec.frames_per_packet()));
        }
        if rest > 0 {
            entries.push((1, to_u32(rest)));
        }
        entries
    }

    fn moov_len_without_tables(codec: Codec, has_edit: bool) -> usize {
        let edts = if has_edit { EDTS_LEN } else { 0 };
        let stsd = STSD_LEN_WITHOUT_ENTRY + codec.entry_len();
        let stbl_fixed = 8 + stsd + 16 + 16 + 12 + 20 + 16;
        8 + MVHD_LEN
            + 8
            + TKHD_LEN
            + edts
            + 8
            + MDHD_LEN
            + HDLR_LEN
            + 8
            + SMHD_LEN
            + DINF_LEN
            + stbl_fixed
    }

    fn moov_len(&self) -> usize {
        Self::moov_len_without_tables(self.codec, self.edit.is_some())
            + 8 * stts_entry_count(self.codec, self.frames)
            + 8 * self.packets
    }

    fn write_moov(&self, out: &mut Boxes, entry: &SampleEntry, packet_sizes: &[u32]) {
        let start = out.0.len();
        let presented = to_u32(self.edit.map_or(self.frames, |edit| edit.frames));
        let first_packet = match self.order {
            Order::FastStart => self.head_len(),
            Order::MoovLast => FTYP_LEN + MDAT_HEADER_LEN,
        };
        out.open(b"moov", |out| {
            out.full(b"mvhd", 0, |out| {
                out.u32(0);
                out.u32(0);
                out.u32(self.sample_rate);
                out.u32(presented);
                out.u32(FIXED_ONE);
                out.u16(VOLUME_ONE);
                out.zeros(2 + 8);
                IDENTITY_MATRIX.iter().for_each(|&v| out.u32(v));
                out.zeros(24);
                out.u32(2);
            });
            out.open(b"trak", |out| {
                out.full(b"tkhd", TRACK_FLAGS, |out| {
                    out.u32(0);
                    out.u32(0);
                    out.u32(1);
                    out.zeros(4);
                    out.u32(presented);
                    out.zeros(8);
                    out.u16(0);
                    out.u16(0);
                    out.u16(VOLUME_ONE);
                    out.zeros(2);
                    IDENTITY_MATRIX.iter().for_each(|&v| out.u32(v));
                    out.u32(0);
                    out.u32(0);
                });
                if let Some(edit) = self.edit {
                    out.open(b"edts", |out| {
                        out.full(b"elst", 0, |out| {
                            out.u32(1);
                            out.u32(to_u32(edit.frames));
                            out.u32(edit.skip);
                            out.u32(FIXED_ONE);
                        });
                    });
                }
                out.open(b"mdia", |out| {
                    out.full(b"mdhd", 0, |out| {
                        out.u32(0);
                        out.u32(0);
                        out.u32(self.sample_rate);
                        out.u32(to_u32(self.frames));
                        out.u16(LANGUAGE_UND);
                        out.u16(0);
                    });
                    out.full(b"hdlr", 0, |out| {
                        out.u32(0);
                        out.kind(b"soun");
                        out.zeros(12);
                        out.bytes(HANDLER_NAME);
                    });
                    out.open(b"minf", |out| {
                        out.full(b"smhd", 0, |out| out.zeros(4));
                        out.open(b"dinf", |out| {
                            out.full(b"dref", 0, |out| {
                                out.u32(1);
                                out.full(b"url ", SELF_CONTAINED, |_| {});
                            });
                        });
                        out.open(b"stbl", |out| {
                            out.full(b"stsd", 0, |out| {
                                out.u32(1);
                                write_sample_entry(out, entry, self.codec, self.sample_rate);
                            });
                            let stts = self.stts_entries();
                            out.full(b"stts", 0, |out| {
                                out.u32(stts.len() as u32);
                                for &(count, delta) in &stts {
                                    out.u32(count);
                                    out.u32(delta);
                                }
                            });
                            out.full(b"stsc", 0, |out| {
                                out.u32(1);
                                out.u32(1);
                                out.u32(1);
                                out.u32(1);
                            });
                            out.full(b"stsz", 0, |out| {
                                out.u32(0);
                                out.u32(packet_sizes.len() as u32);
                                packet_sizes.iter().for_each(|&size| out.u32(size));
                            });
                            out.full(b"stco", 0, |out| {
                                out.u32(packet_sizes.len() as u32);
                                let mut offset = first_packet as u64;
                                for &size in packet_sizes {
                                    out.u32(to_u32(offset));
                                    offset += u64::from(size);
                                }
                            });
                        });
                    });
                });
            });
        });
        debug_assert_eq!(out.0.len() - start, self.moov_len());
    }
}

fn write_ftyp(out: &mut Boxes) {
    out.open(b"ftyp", |out| {
        out.kind(b"M4A ");
        out.u32(0);
        out.kind(b"M4A ");
        out.kind(b"mp42");
        out.kind(b"isom");
    });
}

fn write_sample_entry(out: &mut Boxes, entry: &SampleEntry, codec: Codec, sample_rate: u32) {
    let fields = |out: &mut Boxes| {
        out.zeros(6);
        // data_reference_index: the one `url ` entry.
        out.u16(1);
        out.zeros(8);
        out.u16(codec.channels());
        out.u16(16);
        out.zeros(4);
        out.u32(sample_rate << 16);
    };
    match entry {
        SampleEntry::Alac { cookie } => out.open(b"alac", |out| {
            fields(out);
            out.full(b"alac", 0, |out| out.bytes(cookie));
        }),
        SampleEntry::Aac { config } => out.open(b"mp4a", |out| {
            fields(out);
            out.full(b"esds", 0, |out| write_es_descriptor(out, config));
        }),
    }
}

/// The `ES_Descriptor` of `esds` (ISO/IEC 14496-1, 7.2.6.5): `ES_ID` 0
/// and no flags, then the decoder config with the AudioSpecificConfig,
/// then the SL config.
fn write_es_descriptor(out: &mut Boxes, config: &[u8]) {
    out.descriptor(ES_DESCRIPTOR_TAG, |out| {
        out.u16(0);
        out.u8(0);
        out.descriptor(DECODER_CONFIG_TAG, |out| {
            out.u8(OBJECT_TYPE_AUDIO);
            out.u8(STREAM_TYPE_AUDIO);
            // bufferSizeDB (24 bits), maxBitrate and avgBitrate: 0.
            out.zeros(3 + 4 + 4);
            out.descriptor(DECODER_SPECIFIC_TAG, |out| out.bytes(config));
        });
        out.descriptor(SL_CONFIG_TAG, |out| out.u8(SL_PREDEFINED_MP4));
    });
}

/// The `stts` entries of `frames`: one for the full packets, if any, and
/// one for a last, shorter packet, if any.
fn stts_entry_count(codec: Codec, frames: u64) -> usize {
    let frames_per_packet = u64::from(codec.frames_per_packet());
    usize::from(frames >= frames_per_packet)
        + usize::from(!frames.is_multiple_of(frames_per_packet))
}

/// A value that `Layout::new` keeps below 2^32.
fn to_u32(value: u64) -> u32 {
    u32::try_from(value).expect("Layout::new keeps each value below 4 GiB")
}

/// Big-endian box bytes.
struct Boxes(Vec<u8>);

impl Boxes {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn kind(&mut self, kind: &[u8; 4]) {
        self.0.extend_from_slice(kind);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    fn zeros(&mut self, count: usize) {
        self.0.resize(self.0.len() + count, 0);
    }

    /// Writes a box: its size, its type, then what `body` writes.
    fn open(&mut self, kind: &[u8; 4], body: impl FnOnce(&mut Self)) {
        let start = self.0.len();
        self.u32(0);
        self.kind(kind);
        body(self);
        let size = u32::try_from(self.0.len() - start).expect("a box is below 4 GiB");
        self.0[start..start + 4].copy_from_slice(&size.to_be_bytes());
    }

    /// Writes a descriptor of `esds`: its tag, its size in one byte, then
    /// what `body` writes.  `MAX_CONFIG_LEN` keeps each size below 128.
    fn descriptor(&mut self, tag: u8, body: impl FnOnce(&mut Self)) {
        self.u8(tag);
        let size_at = self.0.len();
        self.u8(0);
        body(self);
        let size = self.0.len() - size_at - 1;
        self.0[size_at] = u8::try_from(size)
            .ok()
            .filter(|size| *size < 0x80)
            .expect("MAX_CONFIG_LEN keeps a descriptor below 128 bytes");
    }

    /// Writes a full box: version 0 and `flags` before the body.
    fn full(&mut self, kind: &[u8; 4], flags: u32, body: impl FnOnce(&mut Self)) {
        self.open(kind, |out| {
            out.u32(flags);
            body(out);
        });
    }
}
