//! A 16-bit PCM WAV writer: for stems stored in the browser and for
//! downloads.  16 bits is CD resolution (about 96 dB of dynamic range),
//! half the size of the f32 the engine works in.

/// Interleaves two channels into a 16-bit stereo WAV file.  Samples are
/// rounded to the nearest of 65,536 steps and clipped at full scale.
pub fn encode_pcm16(left: &[f32], right: &[f32], sample_rate: u32) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let data_len = frames * 4;
    let mut out = Vec::with_capacity(44 + data_len);
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    let u16le = |out: &mut Vec<u8>, v: u16| out.extend_from_slice(&v.to_le_bytes());
    out.extend_from_slice(b"RIFF");
    u32le(&mut out, (36 + data_len) as u32);
    out.extend_from_slice(b"WAVEfmt ");
    u32le(&mut out, 16); // fmt chunk size
    u16le(&mut out, 1); // PCM
    u16le(&mut out, 2); // channels
    u32le(&mut out, sample_rate);
    u32le(&mut out, sample_rate * 4); // bytes per second
    u16le(&mut out, 4); // bytes per frame
    u16le(&mut out, 16); // bits per sample
    out.extend_from_slice(b"data");
    u32le(&mut out, data_len as u32);
    // Full scale is 32768 steps, as decoders divide by it; +1.0 clips to
    // 32767.
    let q = |x: f32| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
    for i in 0..frames {
        out.extend_from_slice(&q(left[i]).to_le_bytes());
        out.extend_from_slice(&q(right[i]).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::decode_bytes;

    #[test]
    fn round_trip_through_the_decoder() {
        let n = 10_000;
        let left: Vec<f32> = (0..n).map(|i| (i as f32 * 0.01).sin() * 0.9).collect();
        let right: Vec<f32> = (0..n).map(|i| if i % 2 == 0 { 1.5 } else { -0.25 }).collect();
        let bytes = encode_pcm16(&left, &right, 44_100);
        assert_eq!(bytes.len(), 44 + n * 4);
        let audio = decode_bytes(bytes, Some("wav")).unwrap();
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.channels.len(), 2);
        assert_eq!(audio.channels[0].len(), n);
        for i in 0..n {
            assert!((audio.channels[0][i] - left[i]).abs() <= 0.5 / 32768.0 + 1e-7, "left {i}");
            let want = right[i].clamp(-1.0, 32767.0 / 32768.0);
            assert!((audio.channels[1][i] - want).abs() <= 0.5 / 32768.0 + 1e-7, "right {i}");
        }
    }
}
