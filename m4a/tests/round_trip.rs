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
    let file = alac_file(input, RATE, None, order).expect("the file is made");
    let decoded = decode(file);
    let frames = input.len() / 2;
    assert_eq!(decoded.rate, Some(RATE), "{name}, {frames} frames");
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
}

#[test]
fn each_signal_decodes_bit_for_bit_at_each_length() {
    for (name, signal) in common::signals() {
        for frames in LENGTHS {
            check(name, &signal(frames), Order::FastStart);
        }
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
