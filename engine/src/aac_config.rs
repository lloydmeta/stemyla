//! The AudioSpecificConfig (ASC) of an AAC track (ISO/IEC 14496-3,
//! 1.6.2.1), read before the decoder is made (M2 of
//! `plans/active/m4a.md`).  Symphonia 0.6.1 reads the sync extension
//! `0x2b7` only after an explicit object type 5 or 29, so it misses the
//! backward-compatible sign of SBR.  This reader takes it.

/// What the decode needs of an ASC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AacConfig {
    /// The object type of the core: 2 is AAC-LC.
    pub object_type: u32,
    /// The rate of the core, in Hz.
    pub rate: Option<u32>,
    /// The channel configuration: 0 means a program config element.
    pub channel_config: u8,
    /// The bit of the ASC where the channel configuration starts.
    pub channel_config_bit: usize,
    /// The channels of a program config element of one front element only:
    /// 1 for one channel, 2 for a channel pair.
    pub pce_channels: Option<u32>,
    /// SBR (HE-AAC): object type 5 or 29, or a sync extension with
    /// `sbrPresentFlag` 1.
    pub sbr: bool,
    /// `frameLengthFlag` 1: 960 frames in a packet.
    pub frames_960: bool,
}

/// The rates of the rate index (ISO/IEC 14496-3, 1.6.3.4).
const RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// The object types of the general audio coder, which have a
/// `GASpecificConfig`.
const GA_TYPES: [u32; 12] = [1, 2, 3, 4, 6, 7, 17, 19, 20, 21, 22, 23];

/// The object type of SBR, and of PS (HE-AAC v2).
const SBR: u32 = 5;
const PS: u32 = 29;

/// The sync word of a backward-compatible extension.
const SYNC_EXTENSION: u32 = 0x2b7;

/// Reads bits from the top of each byte.
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn left(&self) -> usize {
        self.bytes.len() * 8 - self.at
    }

    fn read(&mut self, count: usize) -> Option<u32> {
        if count > self.left() {
            return None;
        }
        let mut value = 0;
        for _ in 0..count {
            let bit = (self.bytes[self.at / 8] >> (7 - self.at % 8)) & 1;
            value = (value << 1) | u32::from(bit);
            self.at += 1;
        }
        Some(value)
    }

    fn object_type(&mut self) -> Option<u32> {
        match self.read(5)? {
            31 => Some(32 + self.read(6)?),
            object_type => Some(object_type),
        }
    }

    fn rate(&mut self) -> Option<Option<u32>> {
        match self.read(4)? {
            15 => Some(Some(self.read(24)?)),
            index => Some(RATES.get(index as usize).copied()),
        }
    }
}

/// The channels of a channel configuration, or `None` for 0 (a program
/// config element) and the reserved values (ISO/IEC 14496-3, 1.6.3.5).
pub(crate) fn channels_of(config: u8) -> Option<u32> {
    match config {
        1..=6 => Some(u32::from(config)),
        7 | 12 | 14 => Some(8),
        11 => Some(7),
        13 => Some(24),
        _ => None,
    }
}

/// Reads `bytes` as an ASC.  `None` when it does not parse.
pub(crate) fn read(bytes: &[u8]) -> Option<AacConfig> {
    let mut bits = Bits { bytes, at: 0 };
    let mut object_type = bits.object_type()?;
    let rate = bits.rate()?;
    let channel_config_bit = bits.at;
    let channel_config = bits.read(4)? as u8;
    let mut sbr = false;
    if object_type == SBR || object_type == PS {
        sbr = true;
        // The rate of the output, then the object type of the core.
        bits.rate()?;
        object_type = bits.object_type()?;
    }
    let mut frames_960 = false;
    if GA_TYPES.contains(&object_type) {
        frames_960 = bits.read(1)? == 1;
        if bits.read(1)? == 1 {
            // dependsOnCoreCoder: coreCoderDelay.
            bits.read(14)?;
        }
        let extension = bits.read(1)?;
        if channel_config == 0 {
            // A program config element follows.
            return Some(AacConfig {
                object_type,
                rate,
                channel_config,
                channel_config_bit,
                pce_channels: pce_channels(&mut bits),
                sbr,
                frames_960,
            });
        }
        if object_type == 6 || object_type == 20 {
            bits.read(3)?;
        }
        if extension == 1 {
            if object_type == 22 {
                bits.read(5 + 11)?;
            }
            if [17, 19, 20, 23].contains(&object_type) {
                bits.read(3)?;
            }
            bits.read(1)?;
        }
    }
    if !sbr && bits.left() >= 16 && bits.read(11)? == SYNC_EXTENSION && bits.object_type()? == SBR {
        // With no bit left, the flag counts as 0.
        sbr = bits.read(1) == Some(1);
    }
    Some(AacConfig {
        object_type,
        rate,
        channel_config,
        channel_config_bit,
        pce_channels: None,
        sbr,
        frames_960,
    })
}

/// The channels of the program config element at `bits` (ISO/IEC 14496-3,
/// 4.4.1.1), when it holds one front element and no other: 1 for a single
/// channel, 2 for a channel pair.  Data elements do not count, since the
/// decoder skips them.
fn pce_channels(bits: &mut Bits) -> Option<u32> {
    // The tag, the object type and the rate index.
    bits.read(4 + 2 + 4)?;
    let front = bits.read(4)?;
    let side = bits.read(4)?;
    let back = bits.read(4)?;
    let lfe = bits.read(2)?;
    let _data = bits.read(3)?;
    let coupling = bits.read(4)?;
    // The mono, stereo and matrix mixdowns.
    for extra in [4, 4, 3] {
        if bits.read(1)? == 1 {
            bits.read(extra)?;
        }
    }
    if (front, side, back, lfe, coupling) != (1, 0, 0, 0, 0) {
        return None;
    }
    let pair = bits.read(1)?;
    Some(if pair == 1 { 2 } else { 1 })
}

/// `asc` with the channel configuration `config` in place at the bit
/// `at`.  Symphonia 0.6.1 reads no bit of AAC-LC after the
/// GASpecificConfig, so the program config element after it stays.
pub(crate) fn with_channel_config(asc: &[u8], at: usize, config: u8) -> Vec<u8> {
    let mut out = Vec::from(asc);
    for k in 0..4 {
        let bit = at + k;
        let Some(byte) = out.get_mut(bit / 8) else {
            break;
        };
        let mask = 1 << (7 - bit % 8);
        if (config >> (3 - k)) & 1 == 1 {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }
    out
}

/// The most fill elements before ID_END that [`has_sbr_fill`] walks.
const MOST_TRAILING_FILLS: usize = 3;

/// The extension types of a fill element that is not SBR data
/// (ISO/IEC 14496-3, table 4.121): `EXT_FILL`, `EXT_FILL_DATA`,
/// `EXT_DATA_ELEMENT`, `EXT_DYNAMIC_RANGE` and `EXT_SAC_DATA`.
const OTHER_FILLS: [u32; 5] = [0x0, 0x1, 0x2, 0xB, 0xC];

/// Whether the AAC raw data block `packet` holds a fill element of SBR
/// data (`EXT_SBR_DATA` 0xD or `EXT_SBR_DATA_CRC` 0xE) in the fill elements
/// just before its ID_END, as an HE-AAC packet does (ISO/IEC 14496-3,
/// 4.4.2.1 and 4.5.2.1).  Symphonia's parser finds SBR data in any fill
/// element (`aac/mod.rs`, lines 178 to 205).  ID_END is the last 1 bits of
/// the packet, before zeros to the byte.  It walks back over up to
/// [`MOST_TRAILING_FILLS`] fill elements, each of a length that ends where
/// the next one starts, and of a known extension type.
pub(crate) fn has_sbr_fill(packet: &[u8]) -> bool {
    let bit = |at: usize| u32::from((packet[at / 8] >> (7 - at % 8)) & 1);
    let read = |at: usize, count: usize| (0..count).fold(0, |value, k| (value << 1) | bit(at + k));
    let Some(end) = (0..packet.len() * 8)
        .rev()
        .find(|at| bit(*at) == 1)
        .and_then(|last| last.checked_sub(2))
    else {
        return false;
    };
    read(end, 3) == 7 && sbr_in_fills(&read, end, MOST_TRAILING_FILLS)
}

/// Whether a chain of at most `depth` fill elements that ends at the bit
/// `end` holds SBR data.  Each fill element is ID_FIL (3 bits), a count of
/// 4 bits (15 and an escape of 8 bits: 15 + escape - 1), then the count in
/// bytes, of which the first 4 bits are the extension type.
fn sbr_in_fills(read: &dyn Fn(usize, usize) -> u32, end: usize, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    (0..=269_usize).any(|count| {
        let head = if count < 15 { 3 + 4 } else { 3 + 4 + 8 };
        let Some(start) = end.checked_sub(head + count * 8) else {
            return false;
        };
        let field = read(start + 3, 4) as usize;
        let stated = match field {
            15 => 15 + read(start + 7, 8) as usize - 1,
            field => field,
        };
        if read(start, 3) != 6 || stated != count {
            return false;
        }
        let extension = (count > 0).then(|| read(start + head, 4));
        match extension {
            Some(0xD | 0xE) => true,
            Some(other) if !OTHER_FILLS.contains(&other) => false,
            _ => sbr_in_fills(read, start, depth - 1),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_m4a::{config, he_aac};

    fn lc(rate: u32, channel_config: u8) -> AacConfig {
        AacConfig {
            object_type: 2,
            rate: Some(rate),
            channel_config,
            channel_config_bit: 9,
            pce_channels: None,
            sbr: false,
            frames_960: false,
        }
    }

    /// B6 of review round 7: a program config element of one front element
    /// gives its channels, and the channel configuration goes in place.
    #[test]
    fn a_program_config_of_one_front_element_gives_its_channels() {
        use crate::test_m4a::pce_config;
        let pce =
            |elements: &[bool]| read(&pce_config(44_100, elements)).and_then(|c| c.pce_channels);
        assert_eq!(pce(&[true]), Some(2));
        assert_eq!(pce(&[false]), Some(1));
        assert_eq!(pce(&[true, false]), None);
        assert_eq!(pce(&[]), None);
        let asc = pce_config(44_100, &[true]);
        let mended = with_channel_config(&asc, 9, 2);
        assert_eq!(read(&mended).map(|c| c.channel_config), Some(2));
        assert_eq!(&mended[2..], &asc[2..], "the rest stays");
        assert_eq!(
            with_channel_config(&config(2, 44_100, 2), 9, 1),
            config(2, 44_100, 1)
        );
        // A side, back, LFE or coupling element is not one front element.
        for (field, at) in [("side", 30), ("back", 34), ("lfe", 39), ("coupling", 44)] {
            let mut other = pce_config(44_100, &[true]);
            other[at / 8] |= 1 << (7 - at % 8);
            assert_eq!(read(&other).and_then(|c| c.pce_channels), None, "{field}");
        }
    }

    /// TD-81, closed by review round 7: the end of a packet with a fill
    /// element of SBR data, of each length of its count.
    #[test]
    fn a_packet_that_has_sbr_fill_data_is_found() {
        use crate::test_m4a::BitWriter;
        let packet = |count: usize, extension: u32, end: bool| {
            let mut bits = BitWriter::default();
            // A few bits of another element first.
            bits.put(9, 0x15A);
            bits.put(3, 6);
            if count < 15 {
                bits.put(4, count as u32);
            } else {
                bits.put(4, 15);
                bits.put(8, (count - 14) as u32);
            }
            bits.put(4, extension);
            bits.put(count * 8 - 4, 0);
            if end {
                bits.put(3, 7);
            }
            bits.bytes
        };
        for count in [1, 14, 15, 16, 269] {
            assert!(has_sbr_fill(&packet(count, 0xD, true)), "{count}");
            assert!(has_sbr_fill(&packet(count, 0xE, true)), "{count} with CRC");
        }
        for extension in [0x0, 0x1, 0xB] {
            assert!(!has_sbr_fill(&packet(4, extension, true)), "{extension:x}");
        }
        assert!(!has_sbr_fill(&packet(4, 0xD, false)), "no ID_END");
        assert!(!has_sbr_fill(&[]), "no bits");
        assert!(!has_sbr_fill(&[0xE0]), "ID_END only");
    }

    /// Findings 8-4 and 8-9 of review round 8: SBR data in a fill element
    /// before other fill elements is found, as Symphonia's parser finds it
    /// in any fill element.  A data element (ID_DSE) with the same bits is
    /// not a fill element.
    #[test]
    fn sbr_data_before_other_fill_elements_is_found() {
        use crate::test_m4a::BitWriter;
        let fill = |bits: &mut BitWriter, count: usize, extension: u32| {
            bits.put(3, 6);
            bits.put(4, count as u32);
            if count > 0 {
                bits.put(4, extension);
                bits.put(count * 8 - 4, 0x5A);
            }
        };
        let packet = |fills: &[(usize, u32)], id: u32| {
            let mut bits = BitWriter::default();
            bits.put(9, 0x15A);
            let (first, rest) = fills.split_first().unwrap();
            bits.put(3, id);
            bits.put(4, first.0 as u32);
            bits.put(4, first.1);
            bits.put(first.0 * 8 - 4, 0x5A);
            for (count, extension) in rest {
                fill(&mut bits, *count, *extension);
            }
            bits.put(3, 7);
            bits.bytes
        };
        assert!(
            has_sbr_fill(&packet(&[(3, 0xD), (2, 0x0)], 6)),
            "then EXT_FILL"
        );
        assert!(
            has_sbr_fill(&packet(&[(3, 0xD), (2, 0x1)], 6)),
            "then EXT_FILL_DATA"
        );
        assert!(
            has_sbr_fill(&packet(&[(3, 0xE), (0, 0), (2, 0x1)], 6)),
            "then two"
        );
        assert!(!has_sbr_fill(&packet(&[(3, 0x1), (2, 0x0)], 6)), "no SBR");
        assert!(
            !has_sbr_fill(&packet(&[(3, 0xD), (2, 0x7)], 6)),
            "an unknown type after"
        );
        assert!(!has_sbr_fill(&packet(&[(3, 0xD)], 4)), "ID_DSE");
        let deep = packet(&[(3, 0xD), (1, 0x0), (1, 0x0), (1, 0x0)], 6);
        assert!(!has_sbr_fill(&deep), "past the most fill elements");
    }

    /// Each AudioSpecificConfig of the fixtures, and the maintainer's.
    #[test]
    fn each_config_of_the_fixtures_reads() {
        assert_eq!(read(&config(2, 44_100, 2)), Some(lc(44_100, 2)));
        assert_eq!(read(&config(2, 48_000, 1)), Some(lc(48_000, 1)));
        assert_eq!(read(&config(2, 44_100, 6)), Some(lc(44_100, 6)));
        assert_eq!(
            read(&config(1, 44_100, 2)),
            Some(AacConfig {
                object_type: 1,
                ..lc(44_100, 2)
            })
        );
        assert_eq!(read(&config(2, 44_100, 0)), Some(lc(44_100, 0)));
        // The maintainer's file: AAC-LC at 48 kHz in stereo, then the sync
        // extension 0x2b7, object type 5 and an SBR flag of 0.
        assert_eq!(read(&[0x11, 0x90, 0x56, 0xE5, 0x00]), Some(lc(48_000, 2)));
        // The same with an SBR flag of 1.
        let sbr = read(&[0x11, 0x90, 0x56, 0xE5, 0x80]);
        assert_eq!(sbr.map(|c| c.sbr), Some(true));
        // `frameLengthFlag` 1: 960 frames.
        let short = read(&[0x12, 0x14]);
        assert_eq!(short.map(|c| c.frames_960), Some(true));
        // A rate index of 15: a rate of 24 bits.
        let explicit_rate = read(&[0x17, 0x80, 0x56, 0x22, 0x10]);
        assert_eq!(explicit_rate.and_then(|c| c.rate), Some(44_100));
        // Finding 2-13 of review round 2: a sync extension of object type 5
        // with no bit left for its SBR flag counts as a flag of 0.
        assert_eq!(read(&[0x11, 0x90, 0x56, 0xE5]), Some(lc(48_000, 2)));
        // An object type of 31 needs 6 more bits.
        assert_eq!(read(&[0xF8]), None);
        assert_eq!(read(&[]), None);
    }

    /// The two configs of `HeAacEncoder`: object type 5, and object type 2
    /// with the sync extension and SBR (ISO/IEC 14496-3, 1.6.6).
    #[test]
    fn both_configs_of_he_aac_read_as_sbr() {
        let tone = vec![0.0; 4096];
        let (_, explicit, compatible) = he_aac(&[tone.clone(), tone], 44_100);
        for asc in [explicit, compatible] {
            let read = read(&asc);
            assert_eq!(
                read.map(|c| (c.object_type, c.rate, c.channel_config, c.sbr)),
                Some((2, Some(22_050), 2, true)),
                "{asc:02x?}"
            );
        }
    }

    #[test]
    fn each_channel_configuration_has_its_channels() {
        let channels: Vec<Option<u32>> = (0..16).map(channels_of).collect();
        assert_eq!(
            channels,
            [
                None,
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(8),
                None,
                None,
                None,
                Some(7),
                Some(8),
                Some(24),
                Some(8),
                None
            ]
        );
    }
}
