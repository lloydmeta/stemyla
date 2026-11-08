//! M4A and raw AAC (ADTS) files for the decoder's tests, made in memory
//! (ASSET-1).  Every M4A comes from the writer of `m4a` (TD-52).  The AAC
//! packets come from `oxideav-aac` (D3 of `plans/active/m4a.md`).

use m4a::{AlacEncoder, Codec, Edit, FRAMES_PER_PACKET, Gapless, Layout, Order, SampleEntry};
use oxideav_aac::encoder::{EncoderConfig, StreamEncoder};

/// The frames of one AAC packet.
pub const AAC_FRAMES: usize = 1024;

/// The delay of `oxideav-aac`: its first packet decodes to silence.
pub const ENCODER_DELAY: usize = 1024;

/// `x` as a 16-bit sample, rounded as `wav::write_pcm16` rounds.
pub fn pcm16(x: f32) -> i16 {
    (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

/// `x` as the decoder gives a 16-bit sample of it.
pub fn as_decoded16(x: f32) -> f32 {
    f32::from(pcm16(x)) / 32768.0
}

/// The interleaved 16-bit samples of `channels`, all the same length.
pub fn interleaved16(channels: &[Vec<f32>]) -> Vec<i16> {
    let frames = channels[0].len();
    let mut out = Vec::with_capacity(frames * channels.len());
    for i in 0..frames {
        for channel in channels {
            out.push(pcm16(channel[i]));
        }
    }
    out
}

/// A chirp from 100 Hz to 4 kHz over `frames` at `rate`, with a phase
/// that differs on each channel.  It has no period, so its
/// cross-correlation with a copy peaks at one lag only.
pub fn chirp(channel: usize, frames: usize, rate: u32) -> Vec<f32> {
    let seconds = frames as f64 / f64::from(rate);
    let rise = (4000.0 - 100.0) / seconds.max(1e-9);
    let phase = channel as f64 * 0.7;
    (0..frames)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            let cycles = 100.0 * t + rise * t * t / 2.0;
            (0.4 * (std::f64::consts::TAU * cycles + phase).sin()) as f32
        })
        .collect()
}

/// The bytes of a file of `layout`, as a caller of the writer makes them:
/// the head, the packets, then the tail.
pub fn file_of<P: AsRef<[u8]>>(layout: &Layout, entry: &SampleEntry, packets: &[P]) -> Vec<u8> {
    let sizes: Vec<u32> = packets.iter().map(|p| p.as_ref().len() as u32).collect();
    let head = layout
        .head(entry, &sizes)
        .expect("the writer takes the packets");
    let tail = layout
        .tail(entry, &sizes)
        .expect("the writer takes the packets");
    let mut file = head;
    for packet in packets {
        file.extend_from_slice(packet.as_ref());
    }
    file.extend_from_slice(&tail);
    file
}

/// A 16-bit stereo ALAC M4A of `left` and `right` at `rate`.
pub fn alac_m4a(
    left: &[f32],
    right: &[f32],
    rate: u32,
    edit: Option<Edit>,
    order: Order,
) -> Vec<u8> {
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1: a test copies its short input into the writer's channels"
    )]
    let channels = [left.to_vec(), right.to_vec()];
    m4a::alac_file(&interleaved16(&channels), rate, edit, order).expect("the writer takes it")
}

/// A stereo ALAC M4A at 44.1 kHz of `frames` frames of silence, made from
/// one silent packet repeated, with no whole input.  Its moov is about
/// 8 bytes for each packet.
pub fn silent_alac_m4a(frames: u64) -> Vec<u8> {
    let layout = Layout::new(Codec::Alac, frames, 44_100, None, Order::FastStart)
        .expect("the writer takes it");
    let mut encoder = AlacEncoder::new(44_100);
    let mut packet = Vec::new();
    encoder.encode(&[0; FRAMES_PER_PACKET * 2], &mut packet);
    let entry = SampleEntry::Alac {
        cookie: encoder.cookie(),
    };
    let packets = vec![packet.as_slice(); layout.packets()];
    file_of(&layout, &entry, &packets)
}

/// The ADTS stream of `channels` at `rate` from `StreamEncoder`, at
/// 128 kbit/s for each 2 channels, as it is: `ceil(frames / 1024) + 1`
/// frames, with a delay of [`ENCODER_DELAY`].
pub fn adts(channels: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let count = channels.len() as u8;
    let mut encoder = StreamEncoder::new(EncoderConfig {
        sample_rate: rate,
        channels: count,
        bitrate: 128_000 * u32::from(count) / 2,
    })
    .expect("oxideav-aac takes the config");
    encoder
        .encode_all(&interleaved16(channels))
        .expect("oxideav-aac encodes")
}

/// The raw AAC packets of an ADTS stream: each frame without its header
/// of 7 bytes, or 9 bytes when `protection_absent` is 0.
pub fn adts_packets(stream: &[u8]) -> Vec<&[u8]> {
    let mut packets = Vec::new();
    let mut at = 0;
    while at + 7 <= stream.len() {
        let header = &stream[at..at + 7];
        assert_eq!(header[0], 0xFF, "an ADTS sync word at {at}");
        let length = (usize::from(header[3] & 0x03) << 11)
            | (usize::from(header[4]) << 3)
            | usize::from(header[5] >> 5);
        let header_len = if header[1] & 1 == 1 { 7 } else { 9 };
        packets.push(&stream[at + header_len..at + length]);
        at += length;
    }
    assert_eq!(at, stream.len(), "the stream ends on a frame");
    packets
}

/// The index of `rate` in the table of AAC rates (ISO/IEC 14496-3,
/// 1.6.3.4).
fn rate_index(rate: u32) -> u8 {
    const RATES: [u32; 13] = [
        96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
        8_000, 7_350,
    ];
    RATES.iter().position(|&r| r == rate).expect("an AAC rate") as u8
}

/// The AudioSpecificConfig of an `object_type` (5 bits) at `rate` with
/// `channels` (the channel configuration), and a GASpecificConfig of 0.
pub fn config(object_type: u8, rate: u32, channels: u8) -> [u8; 2] {
    let bits = (u16::from(object_type) << 11)
        | (u16::from(rate_index(rate)) << 7)
        | (u16::from(channels) << 3);
    bits.to_be_bytes()
}

/// The frames of the track of [`aac_m4a`] for `frames` frames of source:
/// its packets times 1024.
pub fn aac_track_frames(frames: usize) -> u64 {
    ((frames.div_ceil(AAC_FRAMES) + 1) * AAC_FRAMES) as u64
}

/// An AAC-LC M4A of `channels` at `rate`: the packets of [`adts`] in a
/// layout of `Codec::Aac`, with an AudioSpecificConfig of object type 2.
/// Its track has [`aac_track_frames`] frames.
pub fn aac_m4a(
    channels: &[Vec<f32>],
    rate: u32,
    edit: Option<Edit>,
    gapless: Option<Gapless>,
    order: Order,
) -> Vec<u8> {
    let stream = adts(channels, rate);
    let packets = adts_packets(&stream);
    let count = channels.len() as u16;
    let config = config(2, rate, count as u8);
    aac_m4a_of(&packets, count, &config, rate, edit, gapless, order)
}

/// An AAC M4A of `packets`, with `config` in an `mp4a` entry of
/// `channels` at `rate`.  Its track has 1024 frames for each packet.
pub fn aac_m4a_of(
    packets: &[&[u8]],
    channels: u16,
    config: &[u8],
    rate: u32,
    edit: Option<Edit>,
    gapless: Option<Gapless>,
    order: Order,
) -> Vec<u8> {
    let codec = Codec::Aac {
        channels,
        config_len: config.len() as u8,
    };
    let frames = (packets.len() * AAC_FRAMES) as u64;
    let layout = Layout::new(codec, frames, rate, edit, order).expect("the writer takes it");
    let layout = match gapless {
        Some(gapless) => layout
            .with_gapless(gapless)
            .expect("the tag fits the track"),
        None => layout,
    };
    let entry = SampleEntry::Aac {
        config: config.into(),
    };
    file_of(&layout, &entry, packets)
}

/// The lag in `-reach..=reach` at which `decoded` best matches `source`:
/// the largest cross-correlation over `window` frames from `from`.  A lag
/// of `d` means that `decoded[i + d]` matches `source[i]`.
pub fn peak_lag(
    source: &[f32],
    decoded: &[f32],
    from: usize,
    window: usize,
    reach: isize,
) -> isize {
    let mut best = (0, f64::MIN);
    for lag in -reach..=reach {
        let mut sum = 0.0;
        for i in from..from + window {
            let j = i as isize + lag;
            if let (Some(&s), Ok(j)) = (source.get(i), usize::try_from(j))
                && let Some(&d) = decoded.get(j)
            {
                sum += f64::from(s) * f64::from(d);
            }
        }
        if sum > best.1 {
            best = (lag, sum);
        }
    }
    best.0
}

/// The places of `pattern` in `bytes`.
fn places(bytes: &[u8], pattern: &[u8]) -> Vec<usize> {
    bytes
        .windows(pattern.len())
        .enumerate()
        .filter(|(_, window)| *window == pattern)
        .map(|(at, _)| at)
        .collect()
}

/// The one place of `pattern` in `bytes`.
pub fn place_once(bytes: &[u8], pattern: &[u8]) -> usize {
    let found = places(bytes, pattern);
    assert_eq!(found.len(), 1, "{pattern:?} is in one place");
    found[0]
}

/// `file` with the one place of `from` changed to `to`.
pub fn patched(mut file: Vec<u8>, from: &[u8; 4], to: &[u8; 4]) -> Vec<u8> {
    let at = place_once(&file, from);
    file[at..at + 4].copy_from_slice(to);
    file
}

/// The start and the size of the first top-level box `kind` of `file`.
pub fn top_box(file: &[u8], kind: &[u8; 4]) -> (usize, usize) {
    let mut at = 0;
    while at + 8 <= file.len() {
        let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
        if &file[at + 4..at + 8] == kind {
            return (at, size);
        }
        assert!(size >= 8, "a box of {size} bytes at {at}");
        at += size;
    }
    panic!("no {kind:?} box");
}

/// The start and the size of the child `kind` of the box at `parent`.
pub fn child_box(file: &[u8], parent: usize, kind: &[u8; 4]) -> (usize, usize) {
    let end = parent + u32::from_be_bytes(file[parent..parent + 4].try_into().unwrap()) as usize;
    let mut at = parent + 8;
    while at + 8 <= end {
        let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
        if &file[at + 4..at + 8] == kind {
            return (at, size);
        }
        assert!(size >= 8, "a box of {size} bytes at {at}");
        at += size;
    }
    panic!("no {kind:?} box in the box at {parent}");
}

/// `file` with the type of the sample entry of its one track as `to`.
pub fn with_entry(mut file: Vec<u8>, to: &[u8; 4]) -> Vec<u8> {
    // After `stsd`: version and flags, the entry count, the entry size.
    let at = place_once(&file, b"stsd") + 4 + 4 + 4 + 4;
    file[at..at + 4].copy_from_slice(to);
    file
}

/// `file`, which has `moov` last, with a copy of its first `trak` appended
/// to `moov`.  The copy has the handler type `handler` and the `track_ID`
/// `track_id`.
pub fn with_second_trak(mut file: Vec<u8>, handler: &[u8; 4], track_id: u32) -> Vec<u8> {
    let (moov, moov_size) = top_box(&file, b"moov");
    assert_eq!(moov + moov_size, file.len(), "moov is last");
    let (trak, trak_size) = child_box(&file, moov, b"trak");
    let mut copy = Vec::from(&file[trak..trak + trak_size]);
    let at = place_once(&copy, b"soun");
    copy[at..at + 4].copy_from_slice(handler);
    // The `track_ID` of a version 0 `tkhd`: after its header, version
    // and flags, and two times.
    let (tkhd, _) = child_box(&copy, 0, b"tkhd");
    copy[tkhd + 20..tkhd + 24].copy_from_slice(&track_id.to_be_bytes());
    file.extend_from_slice(&copy);
    let size = (moov_size + trak_size) as u32;
    file[moov..moov + 4].copy_from_slice(&size.to_be_bytes());
    file
}

/// The ADTS stream of `channels` at `rate` from `HeAacEncoder`, and its
/// two AudioSpecificConfigs: explicit (object type 5), and backward
/// compatible (object type 2 with the sync extension and SBR).
pub fn he_aac(channels: &[Vec<f32>], rate: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    use oxideav_aac::he_aac_encoder::{HeAacConfig, HeAacEncoder};
    let count = channels.len() as u8;
    let mut encoder = HeAacEncoder::new(HeAacConfig::new(rate, count, 48_000))
        .expect("oxideav-aac takes the config");
    let stream = encoder
        .encode_all(&interleaved16(channels))
        .expect("oxideav-aac encodes");
    let explicit = encoder.audio_specific_config(true);
    let compatible = encoder.audio_specific_config(false);
    (stream, explicit, compatible)
}
