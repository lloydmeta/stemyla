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
fn extended(rate: u32) -> [u8; 10] {
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
