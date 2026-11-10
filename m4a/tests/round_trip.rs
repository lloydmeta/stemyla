//! Each file of `alac_file` decodes through Symphonia 0.6.1 to its input,
//! bit for bit.

mod common;

use std::io::Cursor;

use m4a::{Edit, Order, alac_file};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

const RATE: u32 = 44_100;
const LENGTHS: [usize; 5] = [0, 1, 4095, 4096, 4097];

/// What Symphonia gives for a file.
#[derive(Debug, PartialEq)]
struct Decoded {
    rate: Option<u32>,
    channels: usize,
    stated_frames: Option<u64>,
    interleaved: Vec<i16>,
}

fn decode(file: Vec<u8>) -> Decoded {
    let source = MediaSourceStream::new(Box::new(Cursor::new(file)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .expect("Symphonia opens the file");
    let track = format
        .default_track(TrackType::Audio)
        .expect("the file has an audio track");
    let track_id = track.id;
    let stated_frames = track.num_frames;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .expect("the track has audio parameters")
        .clone();
    let mut codec = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .expect("Symphonia has an ALAC decoder");
    let mut interleaved = Vec::new();
    let mut packet_samples: Vec<i32> = Vec::new();
    while let Some(packet) = format.next_packet().expect("each packet reads") {
        assert_eq!(packet.track_id, track_id);
        let buffer = codec.decode(&packet).expect("each packet decodes");
        buffer.copy_to_vec_interleaved(&mut packet_samples);
        for &sample in &packet_samples {
            // The decoder gives 16-bit samples in the high bits of an i32.
            assert_eq!(sample & 0xFFFF, 0, "a 16-bit sample");
            interleaved.push((sample >> 16) as i16);
        }
    }
    Decoded {
        rate: params.sample_rate,
        channels: params.channels.as_ref().map_or(0, |c| c.count()),
        stated_frames,
        interleaved,
    }
}

fn check(name: &str, input: &[i16], order: Order) {
    check_at(name, input, RATE, order);
}

/// Gives the size of the file.
fn check_at(name: &str, input: &[i16], rate: u32, order: Order) -> usize {
    let file = alac_file(input, rate, None, order).expect("the file is made");
    let size = file.len();
    let decoded = decode(file);
    let frames = input.len() / 2;
    assert_eq!(decoded.rate, Some(rate), "{name}, {frames} frames");
    assert_eq!(decoded.channels, 2, "{name}, {frames} frames");
    assert_eq!(
        decoded.stated_frames,
        Some(frames as u64),
        "{name}, {frames} frames"
    );
    assert_eq!(
        decoded.interleaved.len(),
        input.len(),
        "{name}, {frames} frames"
    );
    assert!(
        decoded.interleaved == input,
        "{name}, {frames} frames: the samples differ"
    );
    size
}

#[test]
fn each_signal_decodes_bit_for_bit_at_each_length() {
    for (name, signal) in common::signals() {
        for frames in LENGTHS {
            check(name, &signal(frames), Order::FastStart);
        }
    }
}

/// The music-like signal has left != right, and its packets are mostly
/// compressed, so the stereo mix goes through Symphonia.
#[test]
fn a_music_like_signal_decodes_bit_for_bit() {
    for frames in [4097, 10 * 4096 + 1000] {
        let input = common::music(frames);
        assert!(input.chunks_exact(2).any(|f| f[0] != f[1]));
        let size = check_at("music-like", &input, RATE, Order::FastStart);
        assert!(
            size < frames * 4,
            "{frames} frames: {size} bytes, not compressed"
        );
    }
}

#[test]
fn a_file_at_48_khz_decodes_at_48_khz() {
    for (name, signal) in [
        ("mono-like", common::mono_like as common::Signal),
        ("music-like", common::music),
    ] {
        check_at(name, &signal(4097), 48_000, Order::FastStart);
    }
}

#[test]
fn a_file_with_moov_last_decodes_bit_for_bit() {
    for (name, signal) in common::signals() {
        check(name, &signal(4097), Order::MoovLast);
    }
}

#[test]
fn a_file_with_an_edit_decodes_its_whole_track() {
    let input = common::mono_like(3 * 4096);
    let edit = Edit {
        skip: 2_112,
        frames: 10_000,
    };
    let file = alac_file(&input, RATE, Some(edit), Order::FastStart).unwrap();
    // Symphonia 0.6.1 reads no `elst`, so it gives every frame of the track.
    assert!(decode(file).interleaved == input);
}

#[test]
#[should_panic(expected = "length must be even")]
fn an_odd_length_panics() {
    let _ = alac_file(&[0; 3], RATE, None, Order::FastStart);
}

#[test]
fn a_refusal_of_the_layout_is_the_refusal_of_the_file() {
    assert_eq!(
        alac_file(&[0; 2], 0, None, Order::FastStart),
        Err(m4a::Error::SampleRate(0))
    );
}

/// Symphonia probes an AAC file of the writer: 3 packets of zeros, so no
/// decode.  The track states the codec, channels, rate, length and the
/// AudioSpecificConfig.
#[test]
fn symphonia_probes_an_aac_file() {
    use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
    let config = [0x12, 0x10];
    let layout = m4a::Layout::new(
        m4a::Codec::Aac {
            channels: 2,
            config_len: 2,
        },
        3 * 1024,
        RATE,
        None,
        Order::FastStart,
    )
    .unwrap();
    let entry = m4a::SampleEntry::Aac {
        config: config.into(),
    };
    let sizes = [20; 3];
    let head = layout.head(&entry, &sizes);
    assert!(head.is_ok(), "{head:?}");
    let mut file = head.unwrap();
    file.resize(file.len() + 60, 0);
    let source = MediaSourceStream::new(Box::new(Cursor::new(file)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .expect("Symphonia opens the file");
    let track = format
        .default_track(TrackType::Audio)
        .expect("the file has an audio track");
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).unwrap();
    assert_eq!(params.codec, CODEC_ID_AAC);
    assert_eq!(params.channels.as_ref().map(|c| c.count()), Some(2));
    assert_eq!(params.sample_rate, Some(44_100));
    assert_eq!(track.num_frames, Some(3 * 1024));
    assert_eq!(params.extra_data.as_deref(), Some(&config[..]));
}

/// A reader that knows no `iTunSMPB` still reads the file: Symphonia
/// decodes an ALAC file with the tag bit for bit.
#[test]
fn a_file_with_a_gapless_tag_decodes_bit_for_bit() {
    use m4a::{AlacEncoder, Codec, FRAMES_PER_PACKET, Gapless, Layout, SampleEntry};
    let input = common::music(3 * 4096 + 100);
    let frames = (input.len() / 2) as u64;
    for order in [Order::FastStart, Order::MoovLast] {
        let layout = Layout::new(Codec::Alac, frames, RATE, None, order)
            .unwrap()
            .with_gapless(Gapless {
                delay: 2112,
                padding: 100,
                frames: frames - 2212,
            });
        assert!(layout.is_ok(), "{layout:?}");
        let layout = layout.unwrap();
        let mut encoder = AlacEncoder::new(RATE);
        let mut file = vec![0; layout.head_len()];
        let sizes: Vec<u32> = input
            .chunks(FRAMES_PER_PACKET * 2)
            .map(|packet| encoder.encode(packet, &mut file) as u32)
            .collect();
        let entry = SampleEntry::Alac {
            cookie: encoder.cookie(),
        };
        let head = layout.head(&entry, &sizes).unwrap();
        file[..head.len()].copy_from_slice(&head);
        file.extend_from_slice(&layout.tail(&entry, &sizes).unwrap());
        assert!(
            file.windows(8).any(|w| w == b"iTunSMPB"),
            "{order:?}: the file holds the tag"
        );
        let decoded = decode(file);
        assert_eq!(decoded.stated_frames, Some(frames), "{order:?}");
        assert!(
            decoded.interleaved == input,
            "{order:?}: the samples differ"
        );
    }
}
