//! Decoding audio files into per-channel `f32` buffers with Symphonia.

use anyhow::{Context, Result, anyhow};
use std::io::Cursor;
use std::path::Path;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

/// Decoded audio: one `Vec<f32>` per channel, all the same length.
#[derive(Clone, Debug)]
pub struct Audio {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

/// Decodes a file on disk (WAV or MP3).
pub fn decode_path(path: &Path) -> Result<Audio> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_owned);
    decode_source(Box::new(file), ext.as_deref())
        .with_context(|| format!("decoding {}", path.display()))
}

/// Decodes an in-memory file, for example one dropped into the web app.
/// `ext` (such as `"mp3"`) helps the format probe.
pub fn decode_bytes(bytes: Vec<u8>, ext: Option<&str>) -> Result<Audio> {
    decode_source(Box::new(Cursor::new(bytes)), ext)
}

fn decode_source(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Audio> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| anyhow!("unsupported or unrecognised format: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow!("no audio track"))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("track has no audio codec parameters"))?
        .clone();
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| anyhow!("no decoder for codec: {e}"))?;

    let mut sample_rate = params.sample_rate.unwrap_or(0);
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let mut planes: Vec<Vec<f32>> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow!("reading packet: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                if sample_rate == 0 {
                    sample_rate = buf.spec().rate();
                }
                buf.copy_to_vecs_planar::<f32>(&mut planes);
                if channels.is_empty() {
                    channels = vec![Vec::new(); planes.len()];
                }
                for (dst, src) in channels.iter_mut().zip(planes.iter()) {
                    dst.extend_from_slice(src);
                }
            }
            // A corrupt frame is skipped, as players do.
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(anyhow!("decoding packet: {e}")),
        }
    }
    if channels.is_empty() || channels[0].is_empty() {
        return Err(anyhow!("file decoded to no audio"));
    }
    Ok(Audio {
        sample_rate,
        channels,
    })
}
