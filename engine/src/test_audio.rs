//! WAV, FLAC and MP3 files for the tests, made in memory (ASSET-1).  `flac`
//! uses the writers of `test_flac`.

use crate::test_flac::{Blocks, by_hand, compressed, step};

/// A tone of `frames` frames at `rate`, different on each channel, with
/// an offset so that few samples are 0.
pub fn tone(channel: usize, frames: usize, rate: u32) -> Vec<f32> {
    let hz = 220.0 * (channel + 1) as f64;
    (0..frames)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            (0.4 * (2.0 * std::f64::consts::PI * hz * t).sin() + 0.05) as f32
        })
        .collect()
}

/// How a WAV file stores its samples.
#[derive(Clone, Copy, Debug)]
pub enum WavSamples {
    Pcm16,
    Float,
}

/// A WAV file of `channels`, all the same length, at `rate`.
pub fn wav(channels: &[Vec<f32>], rate: u32, samples: WavSamples) -> Vec<u8> {
    let (format, bits): (u16, u16) = match samples {
        WavSamples::Pcm16 => (1, 16),
        WavSamples::Float => (3, 32),
    };
    let count = channels.len() as u16;
    let frames = channels[0].len();
    let block = count * bits / 8;
    let data = (frames * usize::from(block)) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for i in 0..frames {
        for channel in channels {
            match samples {
                WavSamples::Pcm16 => {
                    out.extend_from_slice(&(step(channel[i], 16) as i16).to_le_bytes());
                }
                WavSamples::Float => out.extend_from_slice(&channel[i].to_le_bytes()),
            }
        }
    }
    out
}

/// A 16-bit FLAC of `channels` at `rate`: compressed by `flacenc` for two
/// channels, and in verbatim subframes for one.
pub fn flac(channels: &[Vec<f32>], rate: u32) -> Vec<u8> {
    if let [left, right] = channels {
        return compressed(left, right, rate);
    }
    let steps: Vec<Vec<i32>> = channels
        .iter()
        .map(|c| c.iter().map(|x| step(*x, 16)).collect())
        .collect();
    let frames = channels[0].len() as u64;
    by_hand(rate, 16, channels.len(), frames, Blocks::Verbatim(&steps))
}

/// A 192 kbit/s MP3 of one or two `channels` at `rate`, from `rusty_mp3`.
pub fn mp3(channels: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let mut encoder = rusty_mp3::Mp3Encoder::new(rusty_mp3::Mp3EncoderConfig {
        bitrate_kbps: 192,
        vbr_quality: None,
    });
    let mut interleaved = Vec::with_capacity(channels.len() * channels[0].len());
    for i in 0..channels[0].len() {
        for channel in channels {
            interleaved.push(channel[i]);
        }
    }
    encoder
        .push_pcm_f32(&interleaved, channels.len() as u16, rate)
        .expect("encoded");
    encoder.finish();
    let mut out = Vec::new();
    while let Ok(packet) = encoder.next_packet() {
        out.extend_from_slice(&packet);
    }
    out
}
