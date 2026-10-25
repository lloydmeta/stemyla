//! A 16-bit PCM WAV writer: for stems stored in the browser and for
//! downloads.  16 bits is CD resolution (about 96 dB of dynamic range),
//! half the size of the f32 the engine works in.

/// The bytes before the samples of a 16-bit stereo WAV file.
pub const HEADER_BYTES: usize = 44;

/// The bytes of one 16-bit stereo frame.
pub const FRAME_BYTES: usize = 4;

/// The header of a 16-bit stereo WAV file of `frames` frames.
pub fn header(frames: usize, sample_rate: u32) -> [u8; HEADER_BYTES] {
    let data_len = (frames * FRAME_BYTES) as u32;
    let mut out = [0; HEADER_BYTES];
    let fields: [&[u8]; 12] = [
        b"RIFF",
        &(36 + data_len).to_le_bytes(),
        b"WAVEfmt ",
        &16u32.to_le_bytes(), // fmt chunk size
        &1u16.to_le_bytes(),  // PCM
        &2u16.to_le_bytes(),  // channels
        &sample_rate.to_le_bytes(),
        &(sample_rate * FRAME_BYTES as u32).to_le_bytes(), // bytes per second
        &(FRAME_BYTES as u16).to_le_bytes(),               // bytes per frame
        &16u16.to_le_bytes(),                              // bits per sample
        b"data",
        &data_len.to_le_bytes(),
    ];
    let mut at = 0;
    for field in fields {
        out[at..at + field.len()].copy_from_slice(field);
        at += field.len();
    }
    out
}

/// Interleaves two channels into 16-bit samples, `FRAME_BYTES` for each
/// frame of `out`.  Samples are rounded to the nearest of 65,536 steps and
/// clipped at full scale.  Panics unless `out` holds exactly the frames of
/// `left` and of `right`.
pub fn write_pcm16(left: &[f32], right: &[f32], out: &mut [u8]) {
    assert!(
        left.len() == right.len() && out.len() == left.len() * FRAME_BYTES,
        "write_pcm16 needs equal channels and {FRAME_BYTES} bytes for each frame"
    );
    // Full scale is 32768 steps, as decoders divide by it; +1.0 clips to
    // 32767.
    let q = |x: f32| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
    for (i, frame) in out.chunks_exact_mut(FRAME_BYTES).enumerate() {
        frame[..2].copy_from_slice(&q(left[i]).to_le_bytes());
        frame[2..].copy_from_slice(&q(right[i]).to_le_bytes());
    }
}

/// Interleaves two channels into a 16-bit stereo WAV file, of the frames
/// that both channels have.  Samples are rounded as `write_pcm16` rounds.
pub fn encode_pcm16(left: &[f32], right: &[f32], sample_rate: u32) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let mut out = vec![0; HEADER_BYTES + frames * FRAME_BYTES];
    out[..HEADER_BYTES].copy_from_slice(&header(frames, sample_rate));
    write_pcm16(&left[..frames], &right[..frames], &mut out[HEADER_BYTES..]);
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
        let right: Vec<f32> = (0..n)
            .map(|i| if i % 2 == 0 { 1.5 } else { -0.25 })
            .collect();
        let bytes = encode_pcm16(&left, &right, 44_100);
        assert_eq!(bytes.len(), 44 + n * 4);
        let audio = decode_bytes(bytes, Some("wav")).unwrap();
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.channels.len(), 2);
        assert_eq!(audio.channels[0].len(), n);
        for i in 0..n {
            assert!(
                (audio.channels[0][i] - left[i]).abs() <= 0.5 / 32768.0 + 1e-7,
                "left {i}"
            );
            let want = right[i].clamp(-1.0, 32767.0 / 32768.0);
            assert!(
                (audio.channels[1][i] - want).abs() <= 0.5 / 32768.0 + 1e-7,
                "right {i}"
            );
        }
    }

    /// The writer before `header` and `write_pcm16`, kept to pin its bytes.
    fn encode_pcm16_as_before(left: &[f32], right: &[f32], sample_rate: u32) -> Vec<u8> {
        let frames = left.len().min(right.len());
        let data_len = frames * 4;
        let mut out = Vec::with_capacity(44 + data_len);
        let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        let u16le = |out: &mut Vec<u8>, v: u16| out.extend_from_slice(&v.to_le_bytes());
        out.extend_from_slice(b"RIFF");
        u32le(&mut out, (36 + data_len) as u32);
        out.extend_from_slice(b"WAVEfmt ");
        u32le(&mut out, 16);
        u16le(&mut out, 1);
        u16le(&mut out, 2);
        u32le(&mut out, sample_rate);
        u32le(&mut out, sample_rate * 4);
        u16le(&mut out, 4);
        u16le(&mut out, 16);
        out.extend_from_slice(b"data");
        u32le(&mut out, data_len as u32);
        let q = |x: f32| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        for i in 0..frames {
            out.extend_from_slice(&q(left[i]).to_le_bytes());
            out.extend_from_slice(&q(right[i]).to_le_bytes());
        }
        out
    }

    #[test]
    fn encode_pcm16_writes_the_bytes_it_wrote_before() {
        let left: Vec<f32> = (0..1001).map(|i| (i as f32 * 0.02).sin() * 1.2).collect();
        let right: Vec<f32> = (0..1000).map(|i| -(i as f32 * 0.03).cos()).collect();
        assert_eq!(
            encode_pcm16(&left, &right, 48_000),
            encode_pcm16_as_before(&left, &right, 48_000)
        );
    }

    #[test]
    fn encode_pcm16_is_header_then_blocks() {
        let frames = 10_007;
        let left: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.01).sin()).collect();
        let right: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.07).cos() * 0.5).collect();
        let mut blocks = vec![0; HEADER_BYTES + frames * FRAME_BYTES];
        blocks[..HEADER_BYTES].copy_from_slice(&header(frames, 44_100));
        let mut at = 0;
        while at < frames {
            let n = 4096.min(frames - at);
            let bytes = HEADER_BYTES + at * FRAME_BYTES..HEADER_BYTES + (at + n) * FRAME_BYTES;
            write_pcm16(&left[at..at + n], &right[at..at + n], &mut blocks[bytes]);
            at += n;
        }
        assert_eq!(blocks, encode_pcm16(&left, &right, 44_100));
    }

    #[test]
    #[should_panic(expected = "bytes for each frame")]
    fn write_pcm16_refuses_a_short_output() {
        write_pcm16(&[0.0; 2], &[0.0; 2], &mut [0; 7]);
    }
}
