//! The ALAC port against `alac-encoder` 0.3.0: each signal must give the
//! same packets and the same cookie from both.

mod common;

use alac_encoder::FormatDescription;
use m4a::{AlacEncoder, FRAMES_PER_PACKET, MAX_PACKET_BYTES};

/// The lengths of the differential test, in frames.
const LENGTHS: [usize; 5] = [1, 4095, 4096, 4097, 10 * 4096 + 1000];

/// The size of the work buffer of `alac-encoder` 0.3.0 for 2 channels,
/// plus 8.  Its compressed try can pass `MAX_PACKET_BYTES` before its
/// rewind, and its bit writer writes up to 4 bytes past the bit position.
const REFERENCE_OUT_BYTES: usize = FRAMES_PER_PACKET * 2 * 5 + 1 + 8;

struct Encoded {
    packets: Vec<Vec<u8>>,
    cookie: Vec<u8>,
}

fn port(interleaved: &[i16]) -> Encoded {
    let mut encoder = AlacEncoder::new(44_100);
    let packets = interleaved
        .chunks(FRAMES_PER_PACKET * 2)
        .map(|packet| {
            let mut out = Vec::new();
            let size = encoder.encode(packet, &mut out);
            assert_eq!(size, out.len());
            out
        })
        .collect();
    Encoded {
        packets,
        cookie: encoder.cookie().into(),
    }
}

fn reference(interleaved: &[i16]) -> Encoded {
    let input_format = FormatDescription::pcm::<i16>(44_100.0, 2);
    let output_format = FormatDescription::alac(44_100.0, FRAMES_PER_PACKET as u32, 2);
    let mut encoder = alac_encoder::AlacEncoder::new(&output_format);
    let bytes: Vec<u8> = interleaved.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = vec![0; REFERENCE_OUT_BYTES];
    let packets = bytes
        .chunks(FRAMES_PER_PACKET * 4)
        .map(|packet| {
            // Its escape path reads the input as `&[i16]`.
            assert_eq!(packet.as_ptr() as usize % 2, 0, "the input is aligned");
            let size = encoder.encode(&input_format, packet, &mut out);
            out[..size].into()
        })
        .collect();
    Encoded {
        packets,
        cookie: encoder.magic_cookie(),
    }
}

/// Bit 22 of a packet: after the 3-bit tag, the 4-bit instance tag, 12
/// zero bits, the partial bit and 2 shift bits.
fn is_escape(packet: &[u8]) -> bool {
    packet[2] & 0b0000_0010 != 0
}

#[test]
fn the_port_gives_the_packets_and_the_cookie_of_the_reference() {
    let music: common::Signal = common::music;
    for (name, signal) in common::signals().into_iter().chain([("music-like", music)]) {
        for frames in LENGTHS {
            let input = signal(frames);
            let ours = port(&input);
            let theirs = reference(&input);
            assert_eq!(
                ours.packets.len(),
                frames.div_ceil(FRAMES_PER_PACKET),
                "{name}, {frames} frames"
            );
            assert_eq!(
                ours.packets.len(),
                theirs.packets.len(),
                "{name}, {frames} frames"
            );
            for (i, (a, b)) in ours.packets.iter().zip(&theirs.packets).enumerate() {
                assert!(a == b, "{name}, {frames} frames: packet {i} differs");
                assert!(a.len() <= MAX_PACKET_BYTES, "{name}: packet {i} is too big");
            }
            assert_eq!(ours.cookie, theirs.cookie, "{name}, {frames} frames");
        }
    }
}

#[test]
fn noise_gives_escape_packets_and_a_sine_compressed_packets() {
    let frames = 10 * FRAMES_PER_PACKET;
    let noise = port(&common::random(frames));
    assert!(noise.packets.iter().all(|p| p.len() == MAX_PACKET_BYTES));
    assert!(noise.packets.iter().all(|p| is_escape(p)));
    let sine = port(&common::mono_like(frames));
    assert!(
        sine.packets
            .iter()
            .all(|p| (3..MAX_PACKET_BYTES / 2).contains(&p.len()))
    );
    assert!(sine.packets.iter().all(|p| !is_escape(p)));
    let rewound = port(&common::rewind(frames));
    assert!(rewound.packets.iter().all(|p| is_escape(p)));
}

#[test]
fn each_packet_is_appended_after_the_bytes_of_out() {
    let mut encoder = AlacEncoder::new(44_100);
    let mut out = vec![1, 2, 3];
    let size = encoder.encode(&common::mono_like(FRAMES_PER_PACKET), &mut out);
    assert!(size > 0);
    assert_eq!(out.len(), 3 + size);
    assert_eq!(out[..3], [1, 2, 3]);
}

#[test]
#[should_panic(expected = "length must be even")]
fn an_odd_length_panics() {
    AlacEncoder::new(44_100).encode(&[0; 3], &mut Vec::new());
}

#[test]
#[should_panic(expected = "1 to 4096 frames, not 0")]
fn no_frames_panics() {
    AlacEncoder::new(44_100).encode(&[], &mut Vec::new());
}

#[test]
#[should_panic(expected = "1 to 4096 frames, not 4097")]
fn more_than_a_packet_panics() {
    AlacEncoder::new(44_100).encode(&[0; 2 * 4097], &mut Vec::new());
}

#[test]
fn a_music_like_signal_gives_fewer_bytes_than_its_wav() {
    let frames = 10 * 44_100;
    let file = m4a::alac_file(&common::music(frames), 44_100, None, m4a::Order::FastStart)
        .expect("the file is made");
    let wav_bytes = 44 + frames * 4;
    println!(
        "music-like, 10 s: {} bytes of M4A, {wav_bytes} bytes of WAV, ratio {:.3}",
        file.len(),
        file.len() as f64 / wav_bytes as f64
    );
    assert!(file.len() < wav_bytes, "{} >= {wav_bytes}", file.len());
}
