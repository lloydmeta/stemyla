//! Live output through cpal.  The audio callback owns the [`Player`]; the UI
//! talks to it only through the shared atomics in [`SharedParams`].
//!
//! Natively, cpal calls back on its own audio thread.  In the browser, cpal's
//! default Web Audio backend calls back on the main thread, next to egui, so
//! the callback cost (published in `SharedParams`) matters there too.

use crate::player::Player;
use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::Ordering::Relaxed;
use web_time::Instant;

pub struct LiveOutput {
    stream: cpal::Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Describes the default output device, or explains why there is none.
pub fn probe_default_output() -> Result<String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("host {:?} has no default output device", host.id()))?;
    let config = device
        .default_output_config()
        .context("default output config")?;
    Ok(format!("{}: {config:?}", device_label(&device)))
}

/// Opens the default output device at the song's sample rate and starts
/// playing `player` into it.
pub fn start(mut player: Player) -> Result<LiveOutput> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no default output device"))?;
    let rate = player.song().sample_rate;

    // Prefer f32 at the song rate with at least two channels.
    let range = device
        .supported_output_configs()
        .context("listing output configs")?
        .filter(|r| r.sample_format() == cpal::SampleFormat::F32 && r.channels() >= 2)
        .find(|r| r.min_sample_rate() <= rate && rate <= r.max_sample_rate())
        .ok_or_else(|| anyhow!("device has no f32 stereo config at {rate} Hz"))?;
    let supported = range.with_sample_rate(rate);
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;

    let params = std::sync::Arc::clone(player.params());
    let mut left = vec![0.0f32; 8192];
    let mut right = vec![0.0f32; 8192];
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let t0 = Instant::now();
                let frames = data.len() / channels;
                let mut done = 0;
                while done < frames {
                    let n = (frames - done).min(left.len());
                    player.process(&mut left[..n], &mut right[..n]);
                    for i in 0..n {
                        let f = &mut data[(done + i) * channels..(done + i + 1) * channels];
                        f[0] = left[i];
                        f[1] = right[i];
                        for extra in f.iter_mut().skip(2) {
                            *extra = 0.0;
                        }
                    }
                    done += n;
                }
                let ns = t0.elapsed().as_nanos() as u64;
                params.last_callback_ns.store(ns, Relaxed);
                params.max_callback_ns.fetch_max(ns, Relaxed);
                params.callbacks.fetch_add(1, Relaxed);
                params.total_callback_ns.fetch_add(ns, Relaxed);
                params.last_callback_frames.store(frames as u64, Relaxed);
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .context("building output stream")?;
    stream.play().context("starting output stream")?;
    Ok(LiveOutput {
        stream,
        device_name: device_label(&device),
        sample_rate: rate,
        channels: config.channels,
    })
}

impl LiveOutput {
    /// Starts the output if the platform held it back.  Browsers create a Web
    /// Audio context in the "suspended" state when the page has had no click
    /// or key press yet (as when a song is restored from storage at page
    /// load), and Safari resumes one only from inside a user-gesture event
    /// handler.  Calling this while handling a click lets the audio play;
    /// on a running output it changes nothing.
    pub fn resume(&self) -> Result<()> {
        self.stream.play().context("resuming the audio output")
    }
}

fn device_label(device: &cpal::Device) -> String {
    match device.id() {
        Ok(id) => format!("{id:?}"),
        Err(e) => format!("unnamed device ({e})"),
    }
}
