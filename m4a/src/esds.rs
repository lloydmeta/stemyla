//! The AudioSpecificConfig of an AAC encoder's description, and the fields
//! of a config that a caller checks.
//!
//! A WebCodecs `AudioEncoder` gives its config as `decoderConfig.description`.
//! Chrome gives the bare AudioSpecificConfig.  Safari gives a whole
//! `ES_Descriptor` (ISO/IEC 14496-1, 7.2.6.5), with 4-byte descriptor sizes.
//! `audio_specific_config` takes the config out of either form.

use crate::mp4::{Error, MAX_CONFIG_LEN};

/// The descriptor tags of `esds` (ISO/IEC 14496-1, 7.2.2.1).
pub(crate) const ES_DESCRIPTOR_TAG: u8 = 3;
pub(crate) const DECODER_CONFIG_TAG: u8 = 4;
pub(crate) const DECODER_SPECIFIC_TAG: u8 = 5;

/// The fixed fields of a `DecoderConfigDescriptor` before its children:
/// the object type, the stream type, `bufferSizeDB` and two bit rates.
const DECODER_CONFIG_FIELDS_LEN: usize = 1 + 1 + 3 + 4 + 4;

/// The most bytes of a descriptor size (ISO/IEC 14496-1, 8.3.3).
const MOST_SIZE_BYTES: usize = 4;

/// The flags of an `ES_Descriptor` (ISO/IEC 14496-1, 7.2.6.5.1).
const STREAM_DEPENDENCE_FLAG: u8 = 0x80;
const URL_FLAG: u8 = 0x40;
const OCR_STREAM_FLAG: u8 = 0x20;

/// The AudioSpecificConfig of `description`.
///
/// A description that starts with tag 3 is an `ES_Descriptor`.  The
/// config is then the body of its tag 5, inside its tag 4.  Any other
/// description is the bare config.  No AudioSpecificConfig starts with the
/// byte 3, since that is object type 0, the null object type.
///
/// Refuses a descriptor that runs past the end, a size of more than 4
/// bytes, an `ES_Descriptor` without tag 4 or tag 5, and a config of 0
/// bytes or more than `MAX_CONFIG_LEN`.
pub fn audio_specific_config(description: &[u8]) -> Result<&[u8], Error> {
    let config = match description.first() {
        Some(&ES_DESCRIPTOR_TAG) => config_of_es_descriptor(description)?,
        _ => description,
    };
    if config.is_empty() || config.len() > usize::from(MAX_CONFIG_LEN) {
        return Err(Error::ConfigBytes(config.len()));
    }
    Ok(config)
}

fn config_of_es_descriptor(description: &[u8]) -> Result<&[u8], Error> {
    let es = child(description, ES_DESCRIPTOR_TAG, ES_DESCRIPTOR_TAG)?;
    let fields = es_fields_len(es)?;
    let decoder_config = child(&es[fields..], DECODER_CONFIG_TAG, ES_DESCRIPTOR_TAG)?;
    let children =
        decoder_config
            .get(DECODER_CONFIG_FIELDS_LEN..)
            .ok_or(Error::DescriptorPastEnd {
                tag: DECODER_CONFIG_TAG,
            })?;
    child(children, DECODER_SPECIFIC_TAG, DECODER_CONFIG_TAG)
}

/// The length of the fields of an `ES_Descriptor` body before its first
/// child: `ES_ID`, the flags, and the fields that the flags add.
fn es_fields_len(es: &[u8]) -> Result<usize, Error> {
    let past_end = Error::DescriptorPastEnd {
        tag: ES_DESCRIPTOR_TAG,
    };
    let flags = *es.get(2).ok_or(past_end.clone())?;
    let mut len = 3;
    if flags & STREAM_DEPENDENCE_FLAG != 0 {
        len += 2;
    }
    if flags & URL_FLAG != 0 {
        let url_len = *es.get(len).ok_or(past_end.clone())?;
        len += 1 + usize::from(url_len);
    }
    if flags & OCR_STREAM_FLAG != 0 {
        len += 2;
    }
    if len > es.len() {
        return Err(past_end);
    }
    Ok(len)
}

/// The body of the first descriptor of tag `tag` in `bytes`, a run of
/// descriptors inside the descriptor of tag `within`.
fn child(mut bytes: &[u8], tag: u8, within: u8) -> Result<&[u8], Error> {
    while let Some(found) = descriptor(bytes)? {
        if found.tag == tag {
            return Ok(found.body);
        }
        bytes = found.rest;
    }
    Err(Error::NoDescriptor { tag, within })
}

/// One descriptor: its tag, its body, and the bytes after it.
struct Descriptor<'a> {
    tag: u8,
    body: &'a [u8],
    rest: &'a [u8],
}

/// The descriptor at the start of `bytes`, or `None` if `bytes` is empty.
/// Its size is 1 to 4 bytes of 7 bits each (ISO/IEC 14496-1, 8.3.3).
/// Each size byte but the last has its top bit set.
fn descriptor(bytes: &[u8]) -> Result<Option<Descriptor<'_>>, Error> {
    let Some((&tag, after_tag)) = bytes.split_first() else {
        return Ok(None);
    };
    let mut size = 0;
    let mut size_bytes = 0;
    loop {
        let byte = *after_tag
            .get(size_bytes)
            .ok_or(Error::DescriptorPastEnd { tag })?;
        size_bytes += 1;
        size = (size << 7) | usize::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            break;
        }
        if size_bytes == MOST_SIZE_BYTES {
            return Err(Error::DescriptorSizeBytes { tag });
        }
    }
    let after_size = &after_tag[size_bytes..];
    if size > after_size.len() {
        return Err(Error::DescriptorPastEnd { tag });
    }
    let (body, rest) = after_size.split_at(size);
    Ok(Some(Descriptor { tag, body, rest }))
}

/// The rates of the rate index (ISO/IEC 14496-3, 1.6.3.4).  Index 13 and
/// 14 are reserved, and index 15 gives the rate in 24 bits.
pub const AAC_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// The rate index of an explicit 24-bit rate.
const EXPLICIT_RATE_INDEX: u32 = 15;

/// The object type that escapes to a 6-bit extension.
const OBJECT_TYPE_ESCAPE: u32 = 31;

/// The object types of SBR and of PS (HE-AAC v2), which hold the object
/// type of their core after an extension rate.
const SBR: u32 = 5;
const PS: u32 = 29;

/// The object types of the general audio coder, which have a
/// `GASpecificConfig` with `frameLengthFlag` (ISO/IEC 14496-3, 4.4.1).
const GA_TYPES: [u32; 12] = [1, 2, 3, 4, 6, 7, 17, 19, 20, 21, 22, 23];

/// The fields of an AudioSpecificConfig (ISO/IEC 14496-3, 1.6.2.1) that
/// a caller checks before it writes an AAC file.
///
/// This is a second reader beside `engine::aac_config`, which is
/// `pub(crate)`.  `engine` takes `m4a` only as a dev-dependency, so
/// `stem-worker` cannot use the reader of `engine` (LAYER-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AacConfig {
    /// The object type that the config gives first: 2 is AAC-LC, and 5
    /// and 29 are HE-AAC.
    pub object_type: u32,
    /// The rate of the core, in Hz.
    pub rate: u32,
    /// The channel configuration: 2 is a channel pair.
    pub channel_config: u8,
    /// `frameLengthFlag` 1: 960 frames in each packet.  It is false for an
    /// object type without a `GASpecificConfig`.
    pub frames_960: bool,
}

impl AacConfig {
    /// Reads the fields of `config`.  Refuses a config that ends before a
    /// field, and a reserved rate index.
    pub fn read(config: &[u8]) -> Result<AacConfig, Error> {
        let mut bits = Bits {
            bytes: config,
            at: 0,
        };
        let object_type = bits.object_type()?;
        let rate = bits.rate()?;
        let channel_config = bits.read(4, "channelConfiguration")? as u8;
        let mut core = object_type;
        if object_type == SBR || object_type == PS {
            bits.rate()?;
            core = bits.object_type()?;
        }
        let frames_960 = GA_TYPES.contains(&core) && bits.read(1, "frameLengthFlag")? == 1;
        Ok(AacConfig {
            object_type,
            rate,
            channel_config,
            frames_960,
        })
    }
}

/// Reads bits from the top of each byte.
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn read(&mut self, count: usize, field: &'static str) -> Result<u32, Error> {
        if count > self.bytes.len() * 8 - self.at {
            return Err(Error::ConfigEnds { field });
        }
        let mut value = 0;
        for _ in 0..count {
            let bit = (self.bytes[self.at / 8] >> (7 - self.at % 8)) & 1;
            value = (value << 1) | u32::from(bit);
            self.at += 1;
        }
        Ok(value)
    }

    fn object_type(&mut self) -> Result<u32, Error> {
        match self.read(5, "audioObjectType")? {
            OBJECT_TYPE_ESCAPE => Ok(32 + self.read(6, "audioObjectTypeExt")?),
            object_type => Ok(object_type),
        }
    }

    fn rate(&mut self) -> Result<u32, Error> {
        match self.read(4, "samplingFrequencyIndex")? {
            EXPLICIT_RATE_INDEX => self.read(24, "samplingFrequency"),
            index => AAC_RATES
                .get(index as usize)
                .copied()
                .ok_or(Error::ConfigRateIndex(index as u8)),
        }
    }
}
