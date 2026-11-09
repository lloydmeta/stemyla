//! M4A and raw AAC (ADTS) files for the decoder's tests, made in memory
//! (ASSET-1).  Every M4A comes from the writer of `m4a` (TD-52).  The AAC
//! packets come from `oxideav-aac` (D3 of `plans/active/m4a.md`).

use m4a::{AlacEncoder, Codec, Edit, FRAMES_PER_PACKET, Gapless, Layout, Order, SampleEntry};
use oxideav_aac::encoder::{EncoderConfig, StreamEncoder};

/// The frames of one AAC packet.
pub const AAC_FRAMES: usize = 1024;

/// The delay of `oxideav-aac`: its first packet decodes to silence.
pub const ENCODER_DELAY: usize = 1024;

/// Apple's AAC encoder delay: 2112 frames.
pub const APPLE_DELAY: usize = 2112;

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
/// frames, with a delay of [`ENCODER_DELAY`].  An encode is slow, about
/// 1.6 s for each second of stereo, so each input is encoded once and
/// kept for the other tests ([`ENCODES`]).
pub fn adts(channels: &[Vec<f32>], rate: u32) -> Vec<u8> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let samples = interleaved16(channels);
    let mut hasher = DefaultHasher::new();
    samples.hash(&mut hasher);
    let key = (hasher.finish(), samples.len(), channels.len(), rate);
    let once = {
        let mut encodes = ENCODES.lock().unwrap_or_else(|e| e.into_inner());
        std::sync::Arc::clone(encodes.entry(key).or_default())
    };
    let stream = once.get_or_init(|| {
        let count = channels.len() as u8;
        let mut encoder = StreamEncoder::new(EncoderConfig {
            sample_rate: rate,
            channels: count,
            bitrate: 128_000 * u32::from(count) / 2,
        })
        .expect("oxideav-aac takes the config");
        encoder.encode_all(&samples).expect("oxideav-aac encodes")
    });
    #[expect(
        clippy::disallowed_methods,
        reason = "MEM-1: each test takes its own copy of a short encoded fixture"
    )]
    let copy = stream.to_vec();
    copy
}

/// The kept encodes of [`adts`], by (hash, length, channels, rate) of the
/// input.  A test that waits on another's encode of the same input gets
/// its result.
type Encodes = std::collections::BTreeMap<
    (u64, usize, usize, u32),
    std::sync::Arc<std::sync::OnceLock<Vec<u8>>>,
>;
static ENCODES: std::sync::Mutex<Encodes> = std::sync::Mutex::new(Encodes::new());

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

/// `channels` after 1088 frames of silence, so the delay of an encode is
/// 1024 + 1088 = 2112 frames, Apple's delay.
pub fn with_apple_delay(channels: &[Vec<f32>]) -> Vec<Vec<f32>> {
    const SILENCE: usize = APPLE_DELAY - ENCODER_DELAY;
    channels
        .iter()
        .map(|channel| {
            let mut delayed = vec![0.0; SILENCE + channel.len()];
            delayed[SILENCE..].copy_from_slice(channel);
            delayed
        })
        .collect()
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
pub fn places(bytes: &[u8], pattern: &[u8]) -> Vec<usize> {
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

/// `file`, which has `moov` last and one `elst`, with the `elst` entries
/// `entries` (`segment_duration`, `media_time`, rate as 16.16), all of
/// version 0.  The sizes of `elst`, `edts`, `trak` and `moov` change to
/// match.  The packets do not move, since `moov` is last.
pub fn with_elst(mut file: Vec<u8>, entries: &[(u32, i32, u32)]) -> Vec<u8> {
    let (moov, moov_size) = top_box(&file, b"moov");
    assert_eq!(moov + moov_size, file.len(), "moov is last");
    let (trak, _) = child_box(&file, moov, b"trak");
    let (edts, _) = child_box(&file, trak, b"edts");
    let (elst, elst_size) = child_box(&file, edts, b"elst");
    let mut body = vec![0, 0, 0, 0];
    body.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for &(segment, media_time, rate) in entries {
        body.extend_from_slice(&segment.to_be_bytes());
        body.extend_from_slice(&media_time.to_be_bytes());
        body.extend_from_slice(&rate.to_be_bytes());
    }
    let mut new_elst = Vec::from(((8 + body.len()) as u32).to_be_bytes());
    new_elst.extend_from_slice(b"elst");
    new_elst.extend_from_slice(&body);
    let delta = new_elst.len() as i64 - elst_size as i64;
    file.splice(elst..elst + elst_size, new_elst);
    for at in [edts, trak, moov] {
        let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap());
        let size = (i64::from(size) + delta) as u32;
        file[at..at + 4].copy_from_slice(&size.to_be_bytes());
    }
    file
}

/// `file` with a movie timescale of `scale`, and `segment` as the
/// duration of `mvhd` and the `segment_duration` of its one `elst` entry,
/// in that timescale.  The writer makes the movie timescale the rate;
/// Apple's files use 1000.
pub fn with_movie_timescale(mut file: Vec<u8>, scale: u32, segment: u32) -> Vec<u8> {
    // After the type of `mvhd`: version and flags, two times, then the
    // timescale and the duration.
    let mvhd = place_once(&file, b"mvhd") + 4;
    file[mvhd + 12..mvhd + 16].copy_from_slice(&scale.to_be_bytes());
    file[mvhd + 16..mvhd + 20].copy_from_slice(&segment.to_be_bytes());
    // After the type of `elst`: version and flags, the count, then the
    // first `segment_duration`.
    let elst = place_once(&file, b"elst") + 4;
    file[elst + 8..elst + 12].copy_from_slice(&segment.to_be_bytes());
    file
}

/// `file` with the type of its first sample entry as `to`, in a file of
/// more than one track.
pub fn with_first_entry(mut file: Vec<u8>, to: &[u8; 4]) -> Vec<u8> {
    let at = places(&file, b"stsd")[0] + 4 + 4 + 4 + 4;
    file[at..at + 4].copy_from_slice(to);
    file
}

/// `file` with its `iTunSMPB` text changed by `change`.
pub fn with_itunsmpb_text(mut file: Vec<u8>, change: impl FnOnce(&mut [u8])) -> Vec<u8> {
    // After the name: the `data` box header, its type and its locale.
    let at = place_once(&file, b"iTunSMPB") + 8 + 8 + 8;
    change(&mut file[at..at + 116]);
    file
}

/// A box of `kind` with `body`.
pub fn bx(kind: &[u8; 4], body: &[&[u8]]) -> Vec<u8> {
    let body = body.concat();
    let mut out = Vec::from(((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(&body);
    out
}

/// A file of one sound track with an `mdhd` and an `elst` of version 1
/// (64-bit times), and a `tkhd` and an `mvhd` of version 1.
pub fn version_1() -> Vec<u8> {
    let full = |version: u8| [version, 0, 0, 0];
    let mvhd = bx(
        b"mvhd",
        &[&full(1), &[0; 16], &1000_u32.to_be_bytes(), &[0; 8]],
    );
    let tkhd = bx(b"tkhd", &[&full(1), &[0; 16], &9_u32.to_be_bytes()]);
    let elst = bx(
        b"elst",
        &[
            &full(1),
            &1_u32.to_be_bytes(),
            &(5_u64 << 32).to_be_bytes(),
            &(1024_u64 << 32).to_be_bytes(),
            &0x1_0000_u32.to_be_bytes(),
        ],
    );
    let mdhd = bx(
        b"mdhd",
        &[
            &full(1),
            &[0; 16],
            &48_000_u32.to_be_bytes(),
            &(3_u64 << 32).to_be_bytes(),
        ],
    );
    let hdlr = bx(b"hdlr", &[&full(0), &[0; 4], b"soun"]);
    let entry = bx(b"mp4a", &[&[0; 28]]);
    let stsd = bx(b"stsd", &[&full(0), &1_u32.to_be_bytes(), &entry]);
    let stbl = bx(b"stbl", &[&stsd]);
    let minf = bx(b"minf", &[&stbl]);
    let mdia = bx(b"mdia", &[&mdhd, &hdlr, &minf]);
    let trak = bx(b"trak", &[&tkhd, &bx(b"edts", &[&elst]), &mdia]);
    bx(b"moov", &[&mvhd, &trak])
}

/// A `meta` box of iTunes tags with a cover image of `bytes` bytes, in
/// `meta/ilst/covr/data`.
pub fn cover_meta(bytes: usize) -> Vec<u8> {
    let image = vec![0xFF; bytes];
    // The `data` type 13 is JPEG; the locale is 0.
    let data = bx(b"data", &[&[0, 0, 0, 13, 0, 0, 0, 0], &image]);
    let hdlr = bx(b"hdlr", &[&[0; 8], b"mdir", b"appl", &[0; 9]]);
    bx(
        b"meta",
        &[&[0; 4], &hdlr, &bx(b"ilst", &[&bx(b"covr", &[&data])])],
    )
}

/// `file`, which has `moov` last, with a cover image of `bytes` bytes in
/// `moov/udta/meta/ilst/covr/data`.
pub fn with_cover(mut file: Vec<u8>, bytes: usize) -> Vec<u8> {
    let (moov, moov_size) = top_box(&file, b"moov");
    assert_eq!(moov + moov_size, file.len(), "moov is last");
    let udta = bx(b"udta", &[&cover_meta(bytes)]);
    file.extend_from_slice(&udta);
    let size = (moov_size + udta.len()) as u32;
    file[moov..moov + 4].copy_from_slice(&size.to_be_bytes());
    file
}

/// `file`, which has `moov` last, with a QuickTime data handler (`dhlr`,
/// `alis`) in the `minf` of its one track, before its other boxes.
pub fn with_data_handler(mut file: Vec<u8>) -> Vec<u8> {
    let (moov, moov_size) = top_box(&file, b"moov");
    assert_eq!(moov + moov_size, file.len(), "moov is last");
    let (trak, _) = child_box(&file, moov, b"trak");
    let (mdia, _) = child_box(&file, trak, b"mdia");
    let (minf, _) = child_box(&file, mdia, b"minf");
    let hdlr = bx(b"hdlr", &[&[0; 4], b"dhlr", b"alis", &[0; 12], &[0]]);
    file.splice(minf + 8..minf + 8, hdlr.iter().copied());
    for at in [minf, mdia, trak, moov] {
        let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap());
        file[at..at + 4].copy_from_slice(&(size + hdlr.len() as u32).to_be_bytes());
    }
    file
}

/// `file`, which has `moov` last, with `extra` bytes of zeros at the end
/// of its `stsz`, and its sample count grown to match.  The sizes of the
/// boxes that hold it grow too.  So its sample tables pass their bound.
pub fn with_grown_stsz(mut file: Vec<u8>, extra: usize) -> Vec<u8> {
    let (moov, moov_size) = top_box(&file, b"moov");
    assert_eq!(moov + moov_size, file.len(), "moov is last");
    let (trak, _) = child_box(&file, moov, b"trak");
    let (mdia, _) = child_box(&file, trak, b"mdia");
    let (minf, _) = child_box(&file, mdia, b"minf");
    let (stbl, _) = child_box(&file, minf, b"stbl");
    let (stsz, stsz_size) = child_box(&file, stbl, b"stsz");
    // After the header: version and flags, the sample size, the count.
    let count_at = stsz + 8 + 4 + 4;
    let count = u32::from_be_bytes(file[count_at..count_at + 4].try_into().unwrap());
    file[count_at..count_at + 4].copy_from_slice(&(count + (extra / 4) as u32).to_be_bytes());
    let end = stsz + stsz_size;
    file.splice(end..end, std::iter::repeat_n(0, extra));
    for at in [stsz, stbl, minf, mdia, trak, moov] {
        let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap());
        file[at..at + 4].copy_from_slice(&(size + extra as u32).to_be_bytes());
    }
    file
}

/// `file`, which has `moov` last, with `child` appended to the box at the
/// end of `path` from the top (such as `moov`, `trak`, `mdia`).  The sizes
/// of the boxes of `path` grow to match.
pub fn with_child(mut file: Vec<u8>, path: &[&[u8; 4]], child: &[u8]) -> Vec<u8> {
    let (moov, moov_size) = top_box(&file, path[0]);
    assert_eq!(moov + moov_size, file.len(), "moov is last");
    let mut boxes = vec![moov];
    for kind in &path[1..] {
        let (at, _) = child_box(&file, *boxes.last().unwrap(), kind);
        boxes.push(at);
    }
    let parent = *boxes.last().unwrap();
    let end = parent + u32::from_be_bytes(file[parent..parent + 4].try_into().unwrap()) as usize;
    file.splice(end..end, child.iter().copied());
    for at in boxes {
        let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap());
        file[at..at + 4].copy_from_slice(&(size + child.len() as u32).to_be_bytes());
    }
    file
}

/// `file`, which has `moov` last, with each box of `path` from the top
/// (such as `moov`, `trak`, `mdia`) of size 0: to the end of the file.
pub fn with_sizes_of_0(mut file: Vec<u8>, path: &[&[u8; 4]]) -> Vec<u8> {
    let (mut at, _) = top_box(&file, path[0]);
    let mut boxes = vec![at];
    for kind in &path[1..] {
        at = child_box(&file, at, kind).0;
        boxes.push(at);
    }
    for at in boxes {
        file[at..at + 4].copy_from_slice(&[0; 4]);
    }
    file
}

/// `file`, which has `moov` last, with each `stco` offset moved by
/// `delta`.
pub fn with_stco_moved(mut out: Vec<u8>, delta: i64) -> Vec<u8> {
    // After the type of `stco`: version and flags, the count, the offsets.
    let stco = place_once(&out, b"stco") + 4;
    let count = u32::from_be_bytes(out[stco + 4..stco + 8].try_into().unwrap()) as usize;
    for k in 0..count {
        let at = stco + 8 + 4 * k;
        let offset = u32::from_be_bytes(out[at..at + 4].try_into().unwrap());
        out[at..at + 4].copy_from_slice(&((i64::from(offset) + delta) as u32).to_be_bytes());
    }
    out
}

/// `file`, which has `moov` last, with a first `ftyp` of `size` bytes:
/// brands of `isom` after its major and minor brands.  The packets move
/// by the change in length, so each `stco` offset moves too.
pub fn with_ftyp_of(file: &[u8], size: usize) -> Vec<u8> {
    assert!(
        size >= 16 && size.is_multiple_of(4),
        "an ftyp of whole brands"
    );
    let (_, old_size) = top_box(file, b"ftyp");
    let brands = b"isom".repeat((size - 16) / 4);
    let ftyp = bx(b"ftyp", &[b"M4A ", &[0; 4], &brands]);
    let out = [ftyp, Vec::from(&file[old_size..])].concat();
    with_stco_moved(out, size as i64 - old_size as i64)
}

/// A top-level `meta` of iTunes tags of fewer than 4096 bytes, with 150
/// `ilst` items of a text of one byte: the dense form of M4A tags.
pub fn dense_meta() -> Vec<u8> {
    let item = bx(
        b"\xA9nam",
        &[&bx(b"data", &[&[0, 0, 0, 1, 0, 0, 0, 0], b"a"])],
    );
    let hdlr = bx(b"hdlr", &[&[0; 8], b"mdir", b"appl", &[0; 9]]);
    let meta = bx(
        b"meta",
        &[&[0; 4], &hdlr, &bx(b"ilst", &[&item.repeat(150)])],
    );
    assert!(meta.len() < 4096, "a small container");
    meta
}

/// `count` top-level `sidx` boxes (version 0) with no references, each of
/// its own `reference_id`, of 32 bytes each.
pub fn sidx_boxes(count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count * 32);
    for id in 0..count as u32 {
        out.extend_from_slice(&bx(
            b"sidx",
            &[
                &[0; 4],
                &id.to_be_bytes(),
                &1000_u32.to_be_bytes(),
                &[0; 8],
                &[0; 4],
            ],
        ));
    }
    out
}

/// `count` top-level `sidx` boxes as [`sidx_boxes`] makes them, all of
/// `reference_id` 1.
pub fn same_sidx_boxes(count: usize) -> Vec<u8> {
    let one = bx(
        b"sidx",
        &[
            &[0; 4],
            &1_u32.to_be_bytes(),
            &1000_u32.to_be_bytes(),
            &[0; 8],
            &[0; 4],
        ],
    );
    one.repeat(count)
}
