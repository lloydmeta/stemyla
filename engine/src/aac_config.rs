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
            // A program config element follows; the decode refuses it.
            return Some(AacConfig {
                object_type,
                rate,
                channel_config,
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
        sbr,
        frames_960,
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
            sbr: false,
            frames_960: false,
        }
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
