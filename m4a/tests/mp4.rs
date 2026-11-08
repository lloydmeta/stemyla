//! The boxes of the M4A writer, read back with a small box reader.

mod common;

use m4a::{
    AlacEncoder, Codec, Edit, Error, FRAMES_PER_PACKET, Layout, MAX_PACKET_BYTES, Order,
    SampleEntry, alac_file,
};

const RATE: u32 = 44_100;
const COOKIE: [u8; 24] = [
    0, 0, 16, 0, 0, 16, 40, 10, 14, 2, 0, 255, 0, 0, 3, 232, 0, 0, 0, 0, 0, 0, 172, 68,
];
const ENTRY: SampleEntry = SampleEntry::Alac { cookie: COOKIE };
const FOUR_GIB: u64 = 1 << 32;

/// One box of a file: its type, its body and the offset of its header.
#[derive(Clone, Copy)]
struct Mp4Box<'a> {
    kind: [u8; 4],
    body: &'a [u8],
}

impl<'a> Mp4Box<'a> {
    fn kind(&self) -> &str {
        std::str::from_utf8(&self.kind).expect("a box type is ASCII")
    }

    /// The children of a container, after `skip` bytes of the body.
    fn children(&self, skip: usize) -> Vec<Mp4Box<'a>> {
        boxes(&self.body[skip..])
    }

    fn child(&self, skip: usize, kind: &str) -> Mp4Box<'a> {
        let children = self.children(skip);
        let found: Vec<_> = children.iter().filter(|b| b.kind() == kind).collect();
        assert_eq!(found.len(), 1, "one {kind} in {}", self.kind());
        *found[0]
    }

    fn has_child(&self, kind: &str) -> bool {
        self.children(0).iter().any(|b| b.kind() == kind)
    }

    fn u16_at(&self, at: usize) -> u16 {
        u16::from_be_bytes(self.body[at..at + 2].try_into().unwrap())
    }

    fn u32_at(&self, at: usize) -> u32 {
        u32::from_be_bytes(self.body[at..at + 4].try_into().unwrap())
    }

    /// The `u32` values of a full box after its version and flags.
    fn u32s(&self) -> Vec<u32> {
        self.body[4..]
            .chunks_exact(4)
            .map(|c| u32::from_be_bytes(c.try_into().unwrap()))
            .collect()
    }
}

/// The boxes, one after the other, that fill `bytes`.
fn boxes(bytes: &[u8]) -> Vec<Mp4Box<'_>> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        assert!(bytes.len() - at >= 8, "a box header needs 8 bytes at {at}");
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert!(
            size >= 8 && at + size <= bytes.len(),
            "box size {size} at {at}"
        );
        found.push(Mp4Box {
            kind: bytes[at + 4..at + 8].try_into().unwrap(),
            body: &bytes[at + 8..at + size],
        });
        at += size;
    }
    found
}

fn kinds(boxes: &[Mp4Box]) -> Vec<String> {
    boxes.iter().map(|b| b.kind().to_string()).collect()
}

/// Packets of the given sizes, each filled with its own byte.
fn packets(sizes: &[u32]) -> Vec<Vec<u8>> {
    sizes
        .iter()
        .enumerate()
        .map(|(i, &size)| vec![i as u8 + 1; size as usize])
        .collect()
}

/// Writes a file as the caller does: zeros for the head, each packet, the
/// final head at position 0, then the tail.
fn file(layout: &Layout, packets: &[Vec<u8>]) -> Vec<u8> {
    file_with(layout, &ENTRY, packets)
}

fn file_with(layout: &Layout, entry: &SampleEntry, packets: &[Vec<u8>]) -> Vec<u8> {
    let sizes: Vec<u32> = packets.iter().map(|p| p.len() as u32).collect();
    let mut out = vec![0; layout.head_len()];
    packets.iter().for_each(|p| out.extend_from_slice(p));
    let head = layout.head(entry, &sizes).expect("the head is made");
    assert_eq!(head.len(), layout.head_len());
    out[..head.len()].copy_from_slice(&head);
    let tail = layout.tail(entry, &sizes).expect("the tail is made");
    assert_eq!(tail.len(), layout.tail_len());
    out.extend_from_slice(&tail);
    out
}

/// The `stbl` of a file's one track.
fn stbl<'a>(moov: &Mp4Box<'a>) -> Mp4Box<'a> {
    moov.child(0, "trak")
        .child(0, "mdia")
        .child(0, "minf")
        .child(0, "stbl")
}

const IDENTITY_MATRIX: [u32; 9] = [0x1_0000, 0, 0, 0, 0x1_0000, 0, 0, 0, 0x4000_0000];

/// The rate and the cookie that a file must hold.
struct Track {
    rate: u32,
    cookie: [u8; 24],
}

const TRACK: Track = Track {
    rate: RATE,
    cookie: COOKIE,
};

/// Checks the tables and each field that Apple's decoder can read.
fn check_moov(moov: &Mp4Box, file: &[u8], packets: &[Vec<u8>], frames: u32, presented: u32) {
    check_track(moov, file, packets, frames, presented, &TRACK);
}

fn check_track(
    moov: &Mp4Box,
    file: &[u8],
    packets: &[Vec<u8>],
    frames: u32,
    presented: u32,
    track: &Track,
) {
    let rate = track.rate;
    let mvhd = moov.child(0, "mvhd").u32s();
    assert_eq!(mvhd[..4], [0, 0, rate, presented], "mvhd times");
    assert_eq!(mvhd[4], 0x1_0000, "mvhd rate 1.0");
    assert_eq!(mvhd[5] >> 16, 0x0100, "mvhd volume 1.0");
    assert_eq!(mvhd[8..17], IDENTITY_MATRIX, "mvhd matrix");
    assert_eq!(mvhd[23], 2, "mvhd next_track_ID");
    assert_eq!(mvhd.len(), 24);

    let trak = moov.child(0, "trak");
    let tkhd = trak.child(0, "tkhd");
    assert_eq!(tkhd.u32_at(0), 7, "tkhd version 0 and flags 7");
    let tkhd = tkhd.u32s();
    assert_eq!(
        tkhd[..5],
        [0, 0, 1, 0, presented],
        "tkhd times and track_ID"
    );
    assert_eq!(tkhd[8] >> 16, 0x0100, "tkhd volume 1.0");
    assert_eq!(tkhd[9..18], IDENTITY_MATRIX, "tkhd matrix");
    assert_eq!(tkhd[18..], [0, 0], "tkhd width and height");

    let mdia = trak.child(0, "mdia");
    assert_eq!(kinds(&mdia.children(0)), ["mdhd", "hdlr", "minf"]);
    let mdhd = mdia.child(0, "mdhd");
    assert_eq!(mdhd.u32s()[..4], [0, 0, rate, frames], "mdhd times");
    assert_eq!(
        mdhd.u16_at(20),
        (21 << 10) | (14 << 5) | 4,
        "mdhd language und"
    );
    assert_eq!(&mdia.child(0, "hdlr").body[8..12], b"soun");
    let minf = mdia.child(0, "minf");
    assert_eq!(kinds(&minf.children(0)), ["smhd", "dinf", "stbl"]);
    let dref = minf.child(0, "dinf").child(0, "dref");
    assert_eq!(dref.u32_at(4), 1, "dref entry count");
    let url = dref.child(8, "url ");
    assert_eq!(url.body, [0, 0, 0, 1], "url flags 1");

    let stbl = stbl(moov);
    assert_eq!(
        kinds(&stbl.children(0)),
        ["stsd", "stts", "stsc", "stsz", "stco"]
    );
    let stsd = stbl.child(0, "stsd");
    assert_eq!(stsd.u32_at(4), 1, "stsd entry count");
    let alac = stsd.child(8, "alac");
    assert_eq!(alac.u16_at(6), 1, "data_reference_index");
    assert_eq!(alac.u16_at(16), 2, "channels");
    assert_eq!(alac.u16_at(18), 16, "sample size");
    assert_eq!(alac.u32_at(24), rate << 16, "rate as 16.16");
    let config = alac.child(28, "alac");
    assert_eq!(config.body.len(), 36 - 8);
    assert_eq!(config.body[..4], [0, 0, 0, 0], "version and flags 0");
    assert_eq!(config.body[4..], track.cookie);

    let full = frames / FRAMES_PER_PACKET as u32;
    let rest = frames % FRAMES_PER_PACKET as u32;
    let mut stts = vec![(full > 0) as u32 + (rest > 0) as u32];
    if full > 0 {
        stts.extend([full, FRAMES_PER_PACKET as u32]);
    }
    if rest > 0 {
        stts.extend([1, rest]);
    }
    assert_eq!(stbl.child(0, "stts").u32s(), stts);
    assert_eq!(stbl.child(0, "stsc").u32s(), [1, 1, 1, 1]);
    let stsz = stbl.child(0, "stsz").u32s();
    assert_eq!(stsz[..2], [0, packets.len() as u32]);
    let sizes: Vec<u32> = packets.iter().map(|p| p.len() as u32).collect();
    assert_eq!(stsz[2..], sizes);
    let stco = stbl.child(0, "stco").u32s();
    assert_eq!(stco[0], packets.len() as u32);
    assert_eq!(stco.len(), 1 + packets.len());
    for (i, (&offset, packet)) in stco[1..].iter().zip(packets).enumerate() {
        let at = offset as usize;
        assert_eq!(&file[at..at + packet.len()], packet, "stco entry {i}");
    }
}

fn check_mdat(mdat: &Mp4Box, packets: &[Vec<u8>]) {
    assert_eq!(mdat.body, packets.concat());
}

#[test]
fn a_fast_start_file_has_moov_before_mdat_and_offsets_at_its_packets() {
    let frames = 2 * 4096 + 100;
    let layout = Layout::new(Codec::Alac, frames, RATE, None, Order::FastStart).unwrap();
    assert_eq!(layout.packets(), 3);
    let packets = packets(&[1000, 2000, 300]);
    let file = file(&layout, &packets);
    let top = boxes(&file);
    assert_eq!(kinds(&top), ["ftyp", "moov", "mdat"]);
    assert_eq!(top[0].body, *b"M4A \0\0\0\0M4A mp42isom");
    check_moov(&top[1], &file, &packets, frames as u32, frames as u32);
    check_mdat(&top[2], &packets);
    assert!(!top[1].child(0, "trak").has_child("edts"));
    assert_eq!(layout.tail_len(), 0);
}

/// A cookie at `rate`, with `max_frame_bytes`, as the encoder gives it.
fn cookie(max_frame_bytes: u32, rate: u32) -> [u8; 24] {
    let mut cookie = COOKIE;
    cookie[12..16].copy_from_slice(&max_frame_bytes.to_be_bytes());
    cookie[20..].copy_from_slice(&rate.to_be_bytes());
    cookie
}

#[test]
fn a_file_at_48_khz_holds_its_rate_in_each_place() {
    let frames = 2 * 4096 + 100;
    let layout = Layout::new(Codec::Alac, frames, 48_000, None, Order::FastStart).unwrap();
    let track = Track {
        rate: 48_000,
        cookie: cookie(2000, 48_000),
    };
    let entry = SampleEntry::Alac {
        cookie: track.cookie,
    };
    let packets = packets(&[1000, 2000, 300]);
    let file = file_with(&layout, &entry, &packets);
    let top = boxes(&file);
    assert_eq!(kinds(&top), ["ftyp", "moov", "mdat"]);
    check_track(
        &top[1],
        &file,
        &packets,
        frames as u32,
        frames as u32,
        &track,
    );
}

/// The packets of `interleaved` from a new encoder, and its final cookie.
fn encoded(interleaved: &[i16], rate: u32) -> (Vec<Vec<u8>>, [u8; 24]) {
    let mut encoder = AlacEncoder::new(rate);
    let packets = interleaved
        .chunks(FRAMES_PER_PACKET * 2)
        .map(|packet| {
            let mut out = Vec::new();
            encoder.encode(packet, &mut out);
            out
        })
        .collect();
    (packets, encoder.cookie())
}

/// `alac_file` writes the cookie of the last packet: its maxFrameBytes is
/// the largest `stsz` entry.  The last packet is the largest here.
#[test]
fn alac_file_writes_the_final_cookie_and_its_rate() {
    let rate = 48_000;
    let quiet = common::silence(2 * FRAMES_PER_PACKET);
    let input: Vec<i16> = quiet.into_iter().chain(common::random(100)).collect();
    let frames = input.len() as u32 / 2;
    let file = alac_file(&input, rate, None, Order::FastStart).unwrap();
    let (packets, final_cookie) = encoded(&input, rate);
    let top = boxes(&file);
    let track = Track {
        rate,
        cookie: final_cookie,
    };
    check_track(&top[1], &file, &packets, frames, frames, &track);
    let stsz = stbl(&top[1]).child(0, "stsz").u32s();
    let largest = *stsz[2..].iter().max().unwrap();
    assert_eq!(
        stsz[2..].last(),
        Some(&largest),
        "the last packet is the largest"
    );
    assert_eq!(final_cookie[12..16], largest.to_be_bytes(), "maxFrameBytes");
    assert_eq!(
        final_cookie[20..],
        rate.to_be_bytes(),
        "the rate of the cookie"
    );
}

#[test]
fn a_moov_last_file_has_mdat_before_moov_and_offsets_at_its_packets() {
    let frames = 4097;
    let layout = Layout::new(Codec::Alac, frames, RATE, None, Order::MoovLast).unwrap();
    let packets = packets(&[16_388, 5]);
    let file = file(&layout, &packets);
    let top = boxes(&file);
    assert_eq!(kinds(&top), ["ftyp", "mdat", "moov"]);
    check_mdat(&top[1], &packets);
    check_moov(&top[2], &file, &packets, frames as u32, frames as u32);
}

#[test]
fn an_edit_gives_one_elst_entry_and_the_presented_duration() {
    let frames = 3 * 4096;
    let edit = Edit {
        skip: 2_112,
        frames: 10_000,
    };
    for order in [Order::FastStart, Order::MoovLast] {
        let layout = Layout::new(Codec::Alac, frames, RATE, Some(edit), order).unwrap();
        let packets = packets(&[10, 20, 30]);
        let file = file(&layout, &packets);
        let top = boxes(&file);
        let moov = top.iter().find(|b| b.kind() == "moov").unwrap();
        check_moov(moov, &file, &packets, frames as u32, 10_000);
        let trak = moov.child(0, "trak");
        assert_eq!(kinds(&trak.children(0)), ["tkhd", "edts", "mdia"]);
        let elst = trak.child(0, "edts").child(0, "elst");
        assert_eq!(elst.u32_at(0), 0, "version 0, flags 0");
        assert_eq!(elst.u32s(), [1, 10_000, 2_112, 0x1_0000]);
    }
}

#[test]
fn the_head_has_its_reserved_length_at_each_length() {
    for frames in [0, 1, 4095, 4096, 4097, 10 * 4096 + 1000] {
        for edit in [
            None,
            Some(Edit {
                skip: 0,
                frames: frames / 2,
            }),
        ] {
            for order in [Order::FastStart, Order::MoovLast] {
                let layout = Layout::new(Codec::Alac, frames, RATE, edit, order).unwrap();
                let sizes = vec![MAX_PACKET_BYTES as u32; layout.packets()];
                let head = layout.head(&ENTRY, &sizes).unwrap();
                assert_eq!(
                    head.len(),
                    layout.head_len(),
                    "{frames}, {edit:?}, {order:?}"
                );
                let tail = layout.tail(&ENTRY, &sizes).unwrap();
                assert_eq!(
                    tail.len(),
                    layout.tail_len(),
                    "{frames}, {edit:?}, {order:?}"
                );
                assert_eq!(
                    layout.packets(),
                    (frames as usize).div_ceil(FRAMES_PER_PACKET)
                );
            }
        }
    }
}

/// The worst case of a layout: its head and tail, and the largest packet
/// for each packet.
fn worst_case(layout: &Layout) -> u64 {
    (layout.head_len() + layout.tail_len()) as u64
        + layout.packets() as u64 * MAX_PACKET_BYTES as u64
}

#[test]
fn a_file_that_can_reach_4_gib_is_refused() {
    for order in [Order::FastStart, Order::MoovLast] {
        // With full packets, each packet adds its largest size and 8 bytes
        // of tables (`stsz`, `stco`).
        let one = Layout::new(Codec::Alac, 4096, RATE, None, order).unwrap();
        assert_eq!(one.packets(), 1);
        let base = worst_case(&one) - (MAX_PACKET_BYTES as u64 + 8);
        let most = (FOUR_GIB - 1 - base) / (MAX_PACKET_BYTES as u64 + 8);
        let largest = Layout::new(Codec::Alac, most * 4096, RATE, None, order).unwrap();
        assert_eq!(largest.packets() as u64, most);
        assert!(worst_case(&largest) < FOUR_GIB);
        // One frame more makes one packet more and an `stts` entry more.
        let next_worst = worst_case(&largest) + MAX_PACKET_BYTES as u64 + 8 + 8;
        assert!(next_worst >= FOUR_GIB, "{order:?}: {next_worst}");
        assert_eq!(
            Layout::new(Codec::Alac, most * 4096 + 1, RATE, None, order),
            Err(Error::TooLarge {
                frames: most * 4096 + 1,
                worst_case_bytes: u128::from(next_worst),
            }),
            "{order:?}"
        );
        // The base holds one `stts` entry; u64::MAX frames need two.
        let packets = u128::from(u64::MAX.div_ceil(4096));
        assert_eq!(
            Layout::new(Codec::Alac, u64::MAX, RATE, None, order),
            Err(Error::TooLarge {
                frames: u64::MAX,
                worst_case_bytes: u128::from(base) + 8 + packets * (MAX_PACKET_BYTES as u128 + 8),
            }),
            "{order:?}"
        );
    }
}

#[test]
fn a_rate_outside_1_to_65535_is_refused() {
    for rate in [0, 65_536, u32::MAX] {
        assert_eq!(
            Layout::new(Codec::Alac, 1, rate, None, Order::FastStart),
            Err(Error::SampleRate(rate))
        );
    }
    for rate in [1, 65_535] {
        assert!(Layout::new(Codec::Alac, 1, rate, None, Order::FastStart).is_ok());
    }
}

#[test]
fn an_edit_past_the_end_is_refused() {
    let past = Edit {
        skip: 2_112,
        frames: 10_000 - 2_112 + 1,
    };
    assert_eq!(
        Layout::new(Codec::Alac, 10_000, RATE, Some(past), Order::FastStart),
        Err(Error::EditPastEnd {
            edit: past,
            frames: 10_000
        })
    );
    let to_the_end = Edit {
        frames: past.frames - 1,
        ..past
    };
    assert!(
        Layout::new(
            Codec::Alac,
            10_000,
            RATE,
            Some(to_the_end),
            Order::FastStart
        )
        .is_ok()
    );
    // `skip + frames` overflows a u64 here.
    for overflow in [
        Edit {
            skip: 1,
            frames: u64::MAX,
        },
        Edit {
            skip: u32::MAX,
            frames: u64::MAX,
        },
    ] {
        assert_eq!(
            Layout::new(Codec::Alac, 10_000, RATE, Some(overflow), Order::FastStart),
            Err(Error::EditPastEnd {
                edit: overflow,
                frames: 10_000
            })
        );
    }
}

#[test]
fn a_wrong_count_or_size_of_packets_is_refused() {
    for order in [Order::FastStart, Order::MoovLast] {
        let layout = Layout::new(Codec::Alac, 4097, RATE, None, order).unwrap();
        let max = MAX_PACKET_BYTES as u32;
        for sizes in [vec![1], vec![1, 2, 3]] {
            let want = Err(Error::PacketCount {
                expected: 2,
                got: sizes.len(),
            });
            assert_eq!(layout.head(&ENTRY, &sizes), want);
            assert_eq!(layout.tail(&ENTRY, &sizes), want);
        }
        let want = Err(Error::PacketSize {
            index: 1,
            size: max + 1,
            most: MAX_PACKET_BYTES,
        });
        assert_eq!(layout.head(&ENTRY, &[max, max + 1]), want);
        assert_eq!(layout.tail(&ENTRY, &[max, max + 1]), want);
        assert!(layout.head(&ENTRY, &[max, max]).is_ok());
    }
}

#[test]
fn each_refusal_says_what_is_wrong() {
    let edit = Edit { skip: 1, frames: 2 };
    let cases = [
        (Error::SampleRate(0), "1 to 65,535 Hz, not 0 Hz"),
        (
            Error::EditPastEnd { edit, frames: 2 },
            "2 frames from frame 1, past the end of 2 frames",
        ),
        (
            Error::TooLarge {
                frames: 5,
                worst_case_bytes: 6,
            },
            "5 frames can reach 6 bytes",
        ),
        (
            Error::PacketCount {
                expected: 2,
                got: 3,
            },
            "2 packets, not 3",
        ),
        (
            Error::PacketSize {
                index: 1,
                size: 9,
                most: 16_388,
            },
            "packet 1 has 9 bytes, more than 16388",
        ),
    ];
    for (error, text) in cases {
        assert!(error.to_string().contains(text), "{error}");
    }
}

/// The AudioSpecificConfig of AAC-LC at 48 kHz in stereo: object type 2,
/// rate index 3, channel configuration 2.
const ASC_48K_STEREO: [u8; 2] = [0x11, 0x90];

fn aac(channels: u16, config_len: u8) -> Codec {
    Codec::Aac {
        channels,
        config_len,
    }
}

fn aac_entry(config: &[u8]) -> SampleEntry {
    SampleEntry::Aac {
        config: config.into(),
    }
}

/// The `esds` body after its version and flags, for the AudioSpecificConfig
/// `config` (ISO/IEC 14496-1, 7.2.6).
fn esds_descriptors(config: &[u8]) -> Vec<u8> {
    let n = config.len() as u8;
    let mut want = vec![3, 23 + n, 0, 0, 0];
    want.extend([4, 15 + n, 0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    want.extend([5, n]);
    want.extend_from_slice(config);
    want.extend([6, 1, 2]);
    want
}

#[test]
fn an_aac_file_has_an_mp4a_entry_with_its_esds() {
    let frames = 3 * 1024;
    let layout = Layout::new(aac(2, 2), frames, 48_000, None, Order::FastStart).unwrap();
    assert_eq!(layout.packets(), 3);
    let packets = packets(&[100, 200, 300]);
    let entry = aac_entry(&ASC_48K_STEREO);
    let file = file_with(&layout, &entry, &packets);
    let top = boxes(&file);
    assert_eq!(kinds(&top), ["ftyp", "moov", "mdat"]);
    check_mdat(&top[2], &packets);
    let stbl = stbl(&top[1]);
    let stsd = stbl.child(0, "stsd");
    assert_eq!(stsd.u32_at(4), 1, "stsd entry count");
    let mp4a = stsd.child(8, "mp4a");
    assert_eq!(mp4a.u16_at(6), 1, "data_reference_index");
    assert_eq!(mp4a.u16_at(16), 2, "channels");
    assert_eq!(mp4a.u16_at(18), 16, "sample size");
    assert_eq!(mp4a.u32_at(24), 48_000 << 16, "rate as 16.16");
    let esds = mp4a.child(28, "esds");
    assert_eq!(esds.body[..4], [0, 0, 0, 0], "version and flags 0");
    assert_eq!(esds.body[4..], esds_descriptors(&ASC_48K_STEREO));
    assert_eq!(stbl.child(0, "stts").u32s(), [1, 3, 1024]);
    let stsz = stbl.child(0, "stsz").u32s();
    assert_eq!(stsz, [0, 3, 100, 200, 300]);
    let stco = stbl.child(0, "stco").u32s();
    for (i, (&offset, packet)) in stco[1..].iter().zip(&packets).enumerate() {
        let at = offset as usize;
        assert_eq!(&file[at..at + packet.len()], packet, "stco entry {i}");
    }
}

#[test]
fn an_aac_layout_has_1024_frames_in_each_packet() {
    let layout = Layout::new(aac(2, 2), 2 * 1024 + 1, 44_100, None, Order::FastStart).unwrap();
    assert_eq!(layout.packets(), 3);
    let packets = packets(&[10, 20, 30]);
    let file = file_with(&layout, &aac_entry(&ASC_48K_STEREO), &packets);
    let stts = stbl(&boxes(&file)[1]).child(0, "stts").u32s();
    assert_eq!(stts, [2, 2, 1024, 1, 1]);
}

#[test]
fn a_6_channel_aac_layout_has_6_channels_and_a_bound_of_4608() {
    let codec = aac(6, 2);
    assert_eq!(codec.channels(), 6);
    assert_eq!(codec.max_packet_bytes(), 4608);
    assert_eq!(codec.frames_per_packet(), 1024);
    assert_eq!(aac(1, 2).max_packet_bytes(), 768);
    assert_eq!(Codec::Alac.max_packet_bytes(), MAX_PACKET_BYTES);
    assert_eq!(Codec::Alac.frames_per_packet(), 4096);
    assert_eq!(Codec::Alac.channels(), 2);
    let layout = Layout::new(codec, 1024, 44_100, None, Order::FastStart).unwrap();
    let entry = aac_entry(&[0x12, 0x30]);
    let file = file_with(&layout, &entry, &packets(&[4608]));
    let mp4a = stbl(&boxes(&file)[1]).child(0, "stsd").child(8, "mp4a");
    assert_eq!(mp4a.u16_at(16), 6, "channels");
    assert_eq!(
        layout.head(&entry, &[4609]),
        Err(Error::PacketSize {
            index: 0,
            size: 4609,
            most: 4608
        })
    );
}

#[test]
fn an_aac_layout_refuses_0_channels_and_a_config_length_outside_1_to_64() {
    let new = |codec| Layout::new(codec, 1024, 44_100, None, Order::FastStart);
    assert_eq!(new(aac(0, 2)), Err(Error::NoChannels));
    assert_eq!(new(aac(2, 0)), Err(Error::ConfigLen(0)));
    assert_eq!(new(aac(2, 65)), Err(Error::ConfigLen(65)));
    assert!(new(aac(2, 1)).is_ok());
    assert!(new(aac(2, 64)).is_ok());
}

#[test]
fn a_sample_entry_of_another_codec_or_config_length_is_refused() {
    for order in [Order::FastStart, Order::MoovLast] {
        let alac = Layout::new(Codec::Alac, 4096, RATE, None, order).unwrap();
        let aac_layout = Layout::new(aac(2, 2), 1024, RATE, None, order).unwrap();
        let cases = [
            (&alac, aac_entry(&ASC_48K_STEREO)),
            (&aac_layout, ENTRY),
            (&aac_layout, aac_entry(&[0x11, 0x90, 0, 0, 0])),
        ];
        for (layout, entry) in cases {
            assert_eq!(
                layout.head(&entry, &[1]),
                Err(Error::EntryCodec),
                "{order:?}"
            );
            assert_eq!(
                layout.tail(&entry, &[1]),
                Err(Error::EntryCodec),
                "{order:?}"
            );
        }
    }
}

#[test]
fn a_mono_aac_packet_of_769_bytes_is_refused() {
    let layout = Layout::new(aac(1, 2), 1024, RATE, None, Order::FastStart).unwrap();
    let entry = aac_entry(&[0x12, 0x08]);
    assert!(layout.head(&entry, &[768]).is_ok());
    let want = Err(Error::PacketSize {
        index: 0,
        size: 769,
        most: 768,
    });
    assert_eq!(layout.head(&entry, &[769]), want);
    assert_eq!(layout.tail(&entry, &[769]), want);
}

#[test]
fn an_aac_head_has_its_reserved_length() {
    for config in [&ASC_48K_STEREO[..], &[0x11, 0x90, 0x56, 0xE5, 0x00]] {
        for frames in [0, 1, 1024, 1025] {
            for edit in [
                None,
                Some(Edit {
                    skip: 0,
                    frames: frames / 2,
                }),
            ] {
                for order in [Order::FastStart, Order::MoovLast] {
                    let codec = aac(2, config.len() as u8);
                    let layout = Layout::new(codec, frames, RATE, edit, order).unwrap();
                    let entry = aac_entry(config);
                    let sizes = vec![768; layout.packets()];
                    let case = format!("{config:?}, {frames}, {edit:?}, {order:?}");
                    assert_eq!(layout.packets(), (frames as usize).div_ceil(1024), "{case}");
                    let head = layout.head(&entry, &sizes);
                    assert!(head.is_ok(), "{case}: {head:?}");
                    assert_eq!(head.unwrap().len(), layout.head_len(), "{case}");
                    let tail = layout.tail(&entry, &sizes);
                    assert!(tail.is_ok(), "{case}: {tail:?}");
                    assert_eq!(tail.unwrap().len(), layout.tail_len(), "{case}");
                }
            }
        }
    }
}

/// The descriptor sizes are one byte each, so the largest
/// AudioSpecificConfig keeps the `ES_Descriptor` below 128 bytes.
#[test]
fn a_64_byte_config_gives_an_es_descriptor_below_128_bytes() {
    let config = [0xA5; 64];
    let layout = Layout::new(aac(2, 64), 1024, RATE, None, Order::FastStart).unwrap();
    let file = file_with(&layout, &aac_entry(&config), &packets(&[10]));
    let esds = stbl(&boxes(&file)[1])
        .child(0, "stsd")
        .child(8, "mp4a")
        .child(28, "esds");
    assert_eq!(esds.body[4..], esds_descriptors(&config));
    let es_descriptor_size = esds.body[5];
    assert_eq!(es_descriptor_size, 23 + 64);
    assert!(es_descriptor_size < 128);
}

#[test]
fn each_aac_refusal_says_what_is_wrong() {
    let cases = [
        (Error::NoChannels, "1 channel or more, not 0"),
        (Error::ConfigLen(65), "1 to 64 bytes, not 65"),
        (Error::EntryCodec, "not of the codec of the layout"),
    ];
    for (error, text) in cases {
        assert!(error.to_string().contains(text), "{error}");
    }
}
