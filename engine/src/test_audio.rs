//! WAV, FLAC, MP3 and AIFF files for the tests, made in memory (ASSET-1).
//! `flac` uses the writers of `test_flac`.  `test_m4a` makes M4A and raw
//! AAC files.

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

/// `rate` as the 80-bit extended float of an AIFF `COMM` chunk: a sign
/// and 15-bit exponent, then a 64-bit mantissa with its top bit set.
pub fn extended(rate: u32) -> [u8; 10] {
    let mut out = [0; 10];
    if rate == 0 {
        return out;
    }
    let top = 31 - rate.leading_zeros();
    let exponent = 16_383 + top as u16;
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    let mantissa = u64::from(rate) << (63 - top);
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// The kind of an AIFF file.
#[derive(Clone, Copy, Debug)]
pub enum Aiff {
    /// `FORM` of type `AIFF`.
    Plain,
    /// `FORM` of type `AIFC`, with compression type `NONE`.
    UncompressedAifc,
}

/// An AIFF file of `channels`, all the same length, at `rate`, with
/// big-endian samples of `bits` (16 or 24) bits.
pub fn aiff(channels: &[Vec<f32>], rate: u32, bits: u32, kind: Aiff) -> Vec<u8> {
    let count = channels.len() as u16;
    let frames = channels[0].len();
    let bytes = (bits / 8) as usize;
    let mut comm = Vec::new();
    comm.extend_from_slice(&count.to_be_bytes());
    comm.extend_from_slice(&(frames as u32).to_be_bytes());
    comm.extend_from_slice(&(bits as u16).to_be_bytes());
    comm.extend_from_slice(&extended(rate));
    if let Aiff::UncompressedAifc = kind {
        comm.extend_from_slice(b"NONE");
        // An empty Pascal string, with its pad byte.
        comm.extend_from_slice(&[0, 0]);
    }
    let mut ssnd = vec![0; 8];
    for i in 0..frames {
        for channel in channels {
            let sample = step(channel[i], bits).to_be_bytes();
            ssnd.extend_from_slice(&sample[4 - bytes..]);
        }
    }
    let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    let form_type = match kind {
        Aiff::Plain => b"AIFF",
        Aiff::UncompressedAifc => {
            // The version of AIFC of 1990-05-23.
            chunks.push((b"FVER", 0xA280_5140_u32.to_be_bytes().into()));
            b"AIFC"
        }
    };
    chunks.push((b"COMM", comm));
    chunks.push((b"SSND", ssnd));
    let mut body = Vec::from(*form_type);
    for (id, chunk) in chunks {
        body.extend_from_slice(id);
        body.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        body.extend_from_slice(&chunk);
        if chunk.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = Vec::from(*b"FORM");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// A block of FLAC metadata of `kind` with `body`, the last one if `last`.
fn flac_block(kind: u8, last: bool, body: &[u8]) -> Vec<u8> {
    let len = (body.len() as u32).to_be_bytes();
    let mut out = vec![kind | if last { 0x80 } else { 0 }, len[1], len[2], len[3]];
    out.extend_from_slice(body);
    out
}

/// `flac` with tags after its STREAMINFO block: a VORBIS_COMMENT block, an
/// APPLICATION block, and `picture` bytes of art in PICTURE blocks of at
/// most 16 MiB each.  Symphonia 0.6.1 reads each of them whole.
pub fn with_flac_tags(flac: &[u8], picture: usize) -> Vec<u8> {
    assert_eq!(flac[..4], *b"fLaC");
    let streaminfo_last = flac[4] & 0x80 != 0;
    let comment = b"TITLE=lloyd";
    let vorbis = [
        &4_u32.to_le_bytes()[..],
        b"acme",
        &1_u32.to_le_bytes(),
        &(comment.len() as u32).to_le_bytes(),
        comment,
    ]
    .concat();
    let mut blocks = vec![(4, vorbis), (2, Vec::from(*b"beachape"))];
    let mut left = picture;
    while left > 0 {
        let data = left.min((1 << 24) - 64);
        let mime = b"image/jpeg";
        let mut body = Vec::with_capacity(data + 64);
        body.extend_from_slice(&3_u32.to_be_bytes());
        body.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        body.extend_from_slice(mime);
        // No description; width, height, depth and colours of 0.
        body.extend_from_slice(&[0; 4 + 16]);
        body.extend_from_slice(&(data as u32).to_be_bytes());
        body.resize(body.len() + data, 0xFF);
        blocks.push((6, body));
        left -= data;
    }
    let count = blocks.len();
    let mut out = Vec::with_capacity(flac.len() + picture + 1024);
    out.extend_from_slice(b"fLaC");
    out.push(flac[4] & 0x7F);
    out.extend_from_slice(&flac[5..42]);
    for (k, (kind, body)) in blocks.into_iter().enumerate() {
        out.extend_from_slice(&flac_block(kind, streaminfo_last && k + 1 == count, &body));
    }
    out.extend_from_slice(&flac[42..]);
    out
}

/// An ID3v2.4 tag of `bytes` bytes of padding after its header, with its
/// size in synchsafe form.
pub fn id3_tag(bytes: usize) -> Vec<u8> {
    let size = bytes as u32;
    let synchsafe = [21, 14, 7, 0].map(|shift| ((size >> shift) & 0x7F) as u8);
    let mut out = vec![b'I', b'D', b'3', 4, 0, 0];
    out.extend_from_slice(&synchsafe);
    out.resize(10 + bytes, 0);
    out
}

/// A RIFF or AIFF chunk of `id` and `body`, with its pad byte, its size
/// little-endian for RIFF and big-endian for AIFF.
fn chunk(id: &[u8; 4], body: &[u8], little_endian: bool) -> Vec<u8> {
    let size = body.len() as u32;
    let mut out = Vec::from(*id);
    out.extend_from_slice(&if little_endian {
        size.to_le_bytes()
    } else {
        size.to_be_bytes()
    });
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// `wav`, of the 44-byte header of `wav`, with a `LIST` chunk of form
/// `INFO` before its `data` chunk: a comment of `bytes` bytes, and a
/// title.  Symphonia 0.6.1 reads it whole.
pub fn with_wav_tags(wav: &[u8], bytes: usize) -> Vec<u8> {
    assert_eq!(wav[..4], *b"RIFF");
    let title = chunk(b"INAM", b"lloyd", true);
    let comment = chunk(b"ICMT", &vec![b'a'; bytes], true);
    let list = chunk(b"LIST", &[&b"INFO"[..], &title, &comment].concat(), true);
    let mut out = Vec::with_capacity(wav.len() + list.len());
    out.extend_from_slice(&wav[..36]);
    out.extend_from_slice(&list);
    out.extend_from_slice(&wav[36..]);
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

/// `aiff` with tag chunks after its `FORM` header: `NAME` of an odd
/// size, `ANNO`, `APPL` of `bytes` bytes and `ID3 `, and an `AUTH` chunk
/// after its `SSND`.  Symphonia 0.6.1 reads each whole, also after `SSND`.
pub fn with_aiff_tags(aiff: &[u8], bytes: usize) -> Vec<u8> {
    assert_eq!(aiff[..4], *b"FORM");
    let mut application = Vec::from(*b"acme");
    application.resize(4 + bytes, 0);
    let tags = [
        chunk(b"NAME", b"lloyd", false),
        chunk(b"ANNO", b"beachape", false),
        chunk(b"APPL", &application, false),
        chunk(b"ID3 ", &id3_tag(100), false),
    ]
    .concat();
    let mut out = Vec::with_capacity(aiff.len() + tags.len());
    out.extend_from_slice(&aiff[..12]);
    out.extend_from_slice(&tags);
    out.extend_from_slice(&aiff[12..]);
    out.extend_from_slice(&chunk(b"AUTH", b"acme", false));
    let form = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&form.to_be_bytes());
    out
}

/// `flac` with `count` APPLICATION blocks of 1024 bytes after its
/// STREAMINFO block: small tag blocks.
pub fn with_small_flac_tags(flac: &[u8], count: usize) -> Vec<u8> {
    assert_eq!(flac[..4], *b"fLaC");
    let streaminfo_last = flac[4] & 0x80 != 0;
    let mut out = Vec::with_capacity(flac.len() + count * 1028);
    out.extend_from_slice(b"fLaC");
    out.push(flac[4] & 0x7F);
    out.extend_from_slice(&flac[5..42]);
    let body = [0_u8; 1024];
    for k in 0..count {
        out.extend_from_slice(&flac_block(2, streaminfo_last && k + 1 == count, &body));
    }
    out.extend_from_slice(&flac[42..]);
    out
}

/// `aiff` with `count` `NAME` chunks of 2 bytes after its `FORM` header.
pub fn with_small_aiff_tags(aiff: &[u8], count: usize) -> Vec<u8> {
    let name = chunk(b"NAME", b"ll", false);
    let mut out = Vec::with_capacity(aiff.len() + count * name.len());
    out.extend_from_slice(&aiff[..12]);
    for _ in 0..count {
        out.extend_from_slice(&name);
    }
    out.extend_from_slice(&aiff[12..]);
    let form = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&form.to_be_bytes());
    out
}

/// `wav` with `count` `LIST` chunks of 512 bytes before its `data` chunk.
pub fn with_small_wav_tags(wav: &[u8], count: usize) -> Vec<u8> {
    let list = chunk(b"LIST", &[&b"INFO"[..], &[0; 508]].concat(), true);
    let mut out = Vec::with_capacity(wav.len() + count * list.len());
    out.extend_from_slice(&wav[..36]);
    for _ in 0..count {
        out.extend_from_slice(&list);
    }
    out.extend_from_slice(&wav[36..]);
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

/// The dense forms of tags of review round 5 (finding 5-1): containers of
/// fewer than 4096 bytes, each with as many tags as it can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dense {
    /// A FLAC VORBIS_COMMENT block of comments of one byte ("=").
    FlacComments,
    /// A WAV `LIST` chunk of form `INFO`, of subchunks of 2 bytes.
    WavInfo,
    /// An AIFF `COMT` chunk of comments of one byte.
    AiffComments,
    /// An AIFF `ID3 ` chunk of ID3v2 `TIT2` frames of one byte of text.
    AiffId3,
}

/// One container of `dense`, as its bytes in the file.
fn dense_container(dense: Dense) -> Vec<u8> {
    match dense {
        Dense::FlacComments => {
            let count = 800_u32;
            let mut body = Vec::from(0_u32.to_le_bytes());
            body.extend_from_slice(&count.to_le_bytes());
            for _ in 0..count {
                body.extend_from_slice(&1_u32.to_le_bytes());
                body.push(b'=');
            }
            flac_block(4, false, &body)
        }
        Dense::WavInfo => {
            let mut body = Vec::from(*b"INFO");
            for _ in 0..400 {
                body.extend_from_slice(&chunk(b"ICMT", b"a", true));
            }
            chunk(b"LIST", &body, true)
        }
        Dense::AiffComments => {
            let count = 445_u16;
            let mut body = Vec::from(count.to_be_bytes());
            for _ in 0..count {
                // A time stamp, a marker of 0, a count of 1 and its byte.
                body.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1, b'a']);
            }
            chunk(b"COMT", &body, false)
        }
        Dense::AiffId3 => {
            let mut frames = Vec::new();
            for _ in 0..330 {
                // `TIT2` of 2 bytes: the text encoding, and one byte.
                frames.extend_from_slice(b"TIT2");
                frames.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, b'a']);
            }
            let mut tag = id3_tag(0);
            let size = frames.len() as u32;
            let synchsafe = [21, 14, 7, 0].map(|shift| ((size >> shift) & 0x7F) as u8);
            tag[6..10].copy_from_slice(&synchsafe);
            tag.extend_from_slice(&frames);
            chunk(b"ID3 ", &tag, false)
        }
    }
}

/// The size of one container of `dense` in the file.
pub fn dense_container_bytes(dense: Dense) -> usize {
    dense_container(dense).len()
}

/// `plain`, a FLAC, a WAV or an AIFF of the format of `dense`, with
/// `count` containers of `dense` before its audio.
pub fn with_dense_tags(plain: &[u8], dense: Dense, count: usize) -> Vec<u8> {
    let one = dense_container(dense);
    let tags = one.repeat(count);
    let mut out = Vec::with_capacity(plain.len() + tags.len());
    match dense {
        Dense::FlacComments => {
            out.extend_from_slice(&plain[..42]);
            out.extend_from_slice(&tags);
            out.extend_from_slice(&plain[42..]);
            // The last-block bit moves from STREAMINFO to the last tag
            // block, when STREAMINFO had it.
            if plain[4] & 0x80 != 0 && count > 0 {
                out[4] &= 0x7F;
                out[42 + (count - 1) * one.len()] |= 0x80;
            }
        }
        Dense::WavInfo => {
            out.extend_from_slice(&plain[..36]);
            out.extend_from_slice(&tags);
            out.extend_from_slice(&plain[36..]);
            let riff = (out.len() - 8) as u32;
            out[4..8].copy_from_slice(&riff.to_le_bytes());
        }
        Dense::AiffComments | Dense::AiffId3 => {
            out.extend_from_slice(&plain[..12]);
            out.extend_from_slice(&tags);
            out.extend_from_slice(&plain[12..]);
            let form = (out.len() - 8) as u32;
            out[4..8].copy_from_slice(&form.to_be_bytes());
        }
    }
    out
}

/// `flac` with a SEEKTABLE block of `points` seek points after its
/// STREAMINFO block, each of 18 bytes, in ascending order.
pub fn with_seektable(flac: &[u8], points: usize) -> Vec<u8> {
    assert_eq!(flac[..4], *b"fLaC");
    let streaminfo_last = flac[4] & 0x80 != 0;
    let mut body = Vec::with_capacity(points * 18);
    for point in 0..points as u64 {
        body.extend_from_slice(&(point * 4096).to_be_bytes());
        body.extend_from_slice(&(point * 100).to_be_bytes());
        body.extend_from_slice(&4096_u16.to_be_bytes());
    }
    let mut out = Vec::with_capacity(flac.len() + body.len() + 4);
    out.extend_from_slice(b"fLaC");
    out.push(flac[4] & 0x7F);
    out.extend_from_slice(&flac[5..42]);
    out.extend_from_slice(&flac_block(3, streaminfo_last, &body));
    out.extend_from_slice(&flac[42..]);
    out
}

/// `aiff` with an SSND offset and block size of `align`: `align` bytes of
/// zeros before its samples.
pub fn with_block_aligned_ssnd(aiff: &[u8], align: u32) -> Vec<u8> {
    let ssnd = (12..aiff.len())
        .find(|at| aiff[*at..].starts_with(b"SSND"))
        .expect("an SSND chunk");
    let size = u32::from_be_bytes(aiff[ssnd + 4..ssnd + 8].try_into().unwrap());
    let mut out = Vec::from(&aiff[..ssnd + 8]);
    out[ssnd + 4..ssnd + 8].copy_from_slice(&(size + align).to_be_bytes());
    out.extend_from_slice(&align.to_be_bytes());
    out.extend_from_slice(&align.to_be_bytes());
    out.extend_from_slice(&vec![0; align as usize]);
    out.extend_from_slice(&aiff[ssnd + 16..]);
    let form = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&form.to_be_bytes());
    out
}

/// The step sizes of IMA ADPCM (IMA Recommended Practices for Enhancing
/// Digital Audio Compatibility, 1992).
const IMA_STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// The change of the step index for each nibble of IMA ADPCM.
const IMA_INDEX_CHANGE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// An AIFC of Apple's IMA ADPCM (`ima4`) of `channels` at `rate`, and what
/// a decoder gives for it, as `f32`.  Each block of each channel is 34
/// bytes: a preamble of the top 9 bits of the predictor and a step index
/// of 7 bits, then 64 samples in nibbles, the low nibble first (Apple
/// QuickTime IMA ADPCM, https://wiki.multimedia.cx/index.php/Apple_QuickTime_IMA_ADPCM).
/// The blocks of the channels take turns.  The frames are a whole number
/// of blocks of 64.
pub fn ima4_aifc(channels: &[Vec<f32>], rate: u32) -> (Vec<u8>, Vec<Vec<f32>>) {
    let frames = channels[0].len();
    assert!(frames.is_multiple_of(64), "whole blocks");
    let mut decoded = vec![Vec::with_capacity(frames); channels.len()];
    // The state of each channel: the predictor and the step index.
    let mut state = vec![(0_i32, 0_i32); channels.len()];
    let mut ssnd = vec![0; 8];
    for block in 0..frames / 64 {
        for (channel, input) in channels.iter().enumerate() {
            let (mut predictor, mut index) = state[channel];
            // The preamble keeps the top 9 bits of the predictor.
            predictor = i32::from((predictor as i16) & !0x7F);
            ssnd.extend_from_slice(&(((predictor as u16) & 0xFF80) | index as u16).to_be_bytes());
            let mut nibbles = [0_u8; 64];
            for (k, nibble) in nibbles.iter_mut().enumerate() {
                let sample = step(input[block * 64 + k], 16);
                let step_size = IMA_STEPS[index as usize];
                let difference = sample - predictor;
                let sign = if difference < 0 { 8 } else { 0 };
                // The magnitude of 3 bits whose ((2m + 1) step) / 8 is
                // nearest to the difference.
                let magnitude = (0..8)
                    .min_by_key(|m| ((2 * m + 1) * step_size / 8 - difference.abs()).abs())
                    .unwrap_or(0);
                *nibble = (sign | magnitude) as u8;
                let change = ((2 * magnitude + 1) * step_size) >> 3;
                predictor = if sign == 8 {
                    predictor - change
                } else {
                    predictor + change
                };
                predictor = predictor.clamp(-32_768, 32_767);
                index = (index + IMA_INDEX_CHANGE[magnitude as usize]).clamp(0, 88);
                decoded[channel].push(predictor as f32 / 32_768.0);
            }
            for pair in nibbles.chunks(2) {
                ssnd.push(pair[0] | (pair[1] << 4));
            }
            state[channel] = (predictor, index);
        }
    }
    let mut comm = Vec::new();
    comm.extend_from_slice(&(channels.len() as u16).to_be_bytes());
    comm.extend_from_slice(&((frames / 64) as u32).to_be_bytes());
    comm.extend_from_slice(&16_u16.to_be_bytes());
    comm.extend_from_slice(&extended(rate));
    comm.extend_from_slice(b"ima4");
    comm.extend_from_slice(&[0, 0]);
    let mut body = Vec::from(*b"AIFC");
    for (id, chunk) in [
        (b"FVER", Vec::from(0xA280_5140_u32.to_be_bytes())),
        (b"COMM", comm),
        (b"SSND", ssnd),
    ] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        body.extend_from_slice(&chunk);
    }
    let mut out = Vec::from(*b"FORM");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    (out, decoded)
}

/// `aiff`, an AIFF or AIFC, with its `SSND` chunk moved before `COMM`.
pub fn with_ssnd_first(aiff: &[u8]) -> Vec<u8> {
    let chunk_at = |id: &[u8; 4]| {
        let mut at = 12;
        loop {
            let size = u32::from_be_bytes(aiff[at + 4..at + 8].try_into().unwrap()) as usize;
            let end = at + 8 + size + (size & 1);
            if aiff[at..at + 4] == *id {
                return (at, end);
            }
            at = end;
        }
    };
    let (ssnd, ssnd_end) = chunk_at(b"SSND");
    let (comm, _) = chunk_at(b"COMM");
    assert!(comm < ssnd, "COMM before SSND");
    [
        &aiff[..comm],
        &aiff[ssnd..ssnd_end],
        &aiff[comm..ssnd],
        &aiff[ssnd_end..],
    ]
    .concat()
}

/// The kinds of ADPCM in a WAV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavAdpcm {
    /// IMA ADPCM (format 0x0011).
    Ima,
    /// Microsoft ADPCM (format 0x0002).
    Ms,
}

/// The adaptation of the step of Microsoft ADPCM for each nibble.
const MS_ADAPTATION: [i32; 16] = [
    230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
];

/// A WAV of `kind` ADPCM of `channels` (1 or 2) at `rate`, in blocks of
/// 256 bytes for each channel, and what a decoder gives for it, as `f32`.
/// The frames of a block: IMA, 505, of which the first is the preamble's
/// predictor; Microsoft, 500, of which the first 2 are the preamble's
/// samples.  The input is cut to whole blocks.  The predictor of
/// Microsoft ADPCM is 0 (coefficients 256 and 0) in each block.
pub fn adpcm_wav(channels: &[Vec<f32>], rate: u32, kind: WavAdpcm) -> (Vec<u8>, Vec<Vec<f32>>) {
    let count = channels.len();
    let align = 256 * count;
    let per_block = match kind {
        WavAdpcm::Ima => (8 * (align - 4 * count)) / (4 * count) + 1,
        WavAdpcm::Ms => (8 * (align - 7 * count)) / (4 * count) + 2,
    };
    let blocks = channels[0].len() / per_block;
    let mut data = Vec::new();
    let mut decoded = vec![Vec::new(); count];
    let mut ima_index = vec![0_i32; count];
    for block in 0..blocks {
        let input = |channel: usize, k: usize| step(channels[channel][block * per_block + k], 16);
        let mut nibbles: Vec<Vec<u8>> = vec![Vec::new(); count];
        match kind {
            WavAdpcm::Ima => {
                for channel in 0..count {
                    let mut predictor = input(channel, 0);
                    let mut index = ima_index[channel];
                    data.extend_from_slice(&(predictor as i16).to_le_bytes());
                    data.extend_from_slice(&[index as u8, 0]);
                    decoded[channel].push(predictor as f32 / 32_768.0);
                    for k in 1..per_block {
                        let size = IMA_STEPS[index as usize];
                        let difference = input(channel, k) - predictor;
                        let sign = if difference < 0 { 8 } else { 0 };
                        let magnitude = (0..8)
                            .min_by_key(|m| ((2 * m + 1) * size / 8 - difference.abs()).abs())
                            .unwrap_or(0);
                        nibbles[channel].push((sign | magnitude) as u8);
                        let change = ((2 * magnitude + 1) * size) >> 3;
                        predictor = if sign == 8 {
                            predictor - change
                        } else {
                            predictor + change
                        };
                        predictor = predictor.clamp(-32_768, 32_767);
                        index = (index + IMA_INDEX_CHANGE[magnitude as usize]).clamp(0, 88);
                        decoded[channel].push(predictor as f32 / 32_768.0);
                    }
                    ima_index[channel] = index;
                }
                // Groups of 4 bytes (8 nibbles, the low one first) of each
                // channel in turn.
                for group in 0..(per_block - 1) / 8 {
                    for channel_nibbles in &nibbles {
                        for pair in channel_nibbles[group * 8..group * 8 + 8].chunks(2) {
                            data.push(pair[0] | (pair[1] << 4));
                        }
                    }
                }
            }
            WavAdpcm::Ms => {
                let mut state: Vec<(i32, i32, i32)> = (0..count)
                    .map(|channel| (16, input(channel, 1), input(channel, 0)))
                    .collect();
                // The predictor, then each field of each channel in turn.
                data.extend(std::iter::repeat_n(0_u8, count));
                for field in 0..3 {
                    for (delta, sample1, sample2) in &state {
                        let value = [*delta, *sample1, *sample2][field];
                        data.extend_from_slice(&(value as i16).to_le_bytes());
                    }
                }
                for (channel, (_, sample1, sample2)) in state.iter().enumerate() {
                    decoded[channel].push(*sample2 as f32 / 32_768.0);
                    decoded[channel].push(*sample1 as f32 / 32_768.0);
                }
                for k in 2..per_block {
                    for (channel, (delta, sample1, sample2)) in state.iter_mut().enumerate() {
                        let predicted = (*sample1 * 256) / 256;
                        let wanted = (input(channel, k) - predicted) as f32 / *delta as f32;
                        let nibble = (wanted.round() as i32).clamp(-8, 7);
                        let value = (predicted + nibble * *delta).clamp(-32_768, 32_767);
                        *sample2 = *sample1;
                        *sample1 = value;
                        *delta = (MS_ADAPTATION[(nibble & 0x0F) as usize] * *delta / 256).max(16);
                        nibbles[channel].push((nibble & 0x0F) as u8);
                        decoded[channel].push(value as f32 / 32_768.0);
                    }
                }
                // Mono: two nibbles of one channel in a byte, the high one
                // first.  Stereo: the left channel high, the right low.
                if count == 1 {
                    for pair in nibbles[0].chunks(2) {
                        data.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
                    }
                } else {
                    for (left, right) in nibbles[0].iter().zip(&nibbles[1]) {
                        data.push((left << 4) | right);
                    }
                }
            }
        }
        data.resize((block + 1) * align, 0);
    }
    let mut fmt = Vec::new();
    let tag: u16 = match kind {
        WavAdpcm::Ima => 0x0011,
        WavAdpcm::Ms => 0x0002,
    };
    fmt.extend_from_slice(&tag.to_le_bytes());
    fmt.extend_from_slice(&(count as u16).to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&((rate as usize * align / per_block) as u32).to_le_bytes());
    fmt.extend_from_slice(&(align as u16).to_le_bytes());
    fmt.extend_from_slice(&4_u16.to_le_bytes());
    match kind {
        WavAdpcm::Ima => {
            fmt.extend_from_slice(&2_u16.to_le_bytes());
            fmt.extend_from_slice(&(per_block as u16).to_le_bytes());
        }
        WavAdpcm::Ms => {
            fmt.extend_from_slice(&32_u16.to_le_bytes());
            fmt.extend_from_slice(&(per_block as u16).to_le_bytes());
            fmt.extend_from_slice(&7_u16.to_le_bytes());
            for (one, two) in [
                (256, 0),
                (512, -256),
                (0, 0),
                (192, 64),
                (240, 0),
                (460, -208),
                (392, -232),
            ] {
                fmt.extend_from_slice(&(one as i16).to_le_bytes());
                fmt.extend_from_slice(&(two as i16).to_le_bytes());
            }
        }
    }
    let mut body = Vec::from(*b"WAVE");
    body.extend_from_slice(&chunk(b"fmt ", &fmt, true));
    body.extend_from_slice(&chunk(
        b"fact",
        &((blocks * per_block) as u32).to_le_bytes(),
        true,
    ));
    body.extend_from_slice(&chunk(b"data", &data, true));
    let mut out = Vec::from(*b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    (out, decoded)
}
