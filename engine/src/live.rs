//! Live output through cpal.  The output is opened once and kept; what it
//! plays is the [`Player`] in its [`PlayerSlot`], which [`LiveOutput::play`]
//! swaps.  The UI talks to the playing player only through the shared
//! atomics in [`crate::SharedParams`].
//!
//! Natively, cpal calls back on its own audio thread.  In the browser, cpal's
//! default Web Audio backend calls back on the main thread, next to egui, so
//! the callback cost (published in `SharedParams`) matters there too.
//!
//! cpal 0.18's Web Audio host never frees a stream's callback (each of its
//! `ended` handlers holds a reference to itself, and dropping the stream
//! only closes the context), so whatever the callback owns lives as long as
//! the page.  The callback here owns only the slot and its scratch buffers,
//! and the slot is emptied when the output is dropped, so a replaced song is
//! freed either way.
//!
//! Because the output stays open, audio already handed to the platform
//! still plays after a swap: in the browser cpal queues two buffers of
//! 2,048 frames (93 ms at 44.1 kHz), plus the output's latency, of the old
//! song (closing the context used to cut it off).  A kept output also
//! outlives main-thread blocks, such as the one while a song loads, so in
//! the browser a buffer due before its callback ran is left silent (see
//! `is_late`).
//!
//! An output that cannot play the next song ([`Unusable`]) is replaced.  In
//! the browser each one replaced still keeps its callback: an empty slot,
//! 64 KiB of scratch and cpal's own buffers, but no song.  Separated songs
//! are all at 44.1 kHz, so that happens rarely.

use crate::player::Player;
use crate::slot::{PlayerSlot, Scratch};
use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

pub struct LiveOutput {
    stream: cpal::Stream,
    slot: PlayerSlot,
    /// Set by the stream's error callback.
    failed: Arc<AtomicBool>,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

static OPENED: AtomicU64 = AtomicU64::new(0);

/// How many outputs [`start`] has opened in this process (or page).
pub fn opened() -> u64 {
    OPENED.load(Relaxed)
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
/// playing `player` into it.  On failure the player comes back with the
/// error, for the caller to play silently.
pub fn start(player: Player) -> Result<LiveOutput, (anyhow::Error, Box<Player>)> {
    match open(player.song().sample_rate) {
        Ok(out) => {
            drop(out.slot.replace(Some(player)));
            Ok(out)
        }
        Err(e) => Err((e, Box::new(player))),
    }
}

/// Opens the default output device at `rate` and starts it, playing
/// silence until it is given a player.
fn open(rate: u32) -> Result<LiveOutput> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no default output device"))?;

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

    let slot = PlayerSlot::new();
    let in_callback = slot.clone();
    let mut scratch = Scratch::default();
    let failed = Arc::new(AtomicBool::new(false));
    let on_error = Arc::clone(&failed);
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                if cfg!(target_arch = "wasm32") && is_late(info.timestamp()) {
                    data.fill(0.0);
                } else {
                    in_callback.render_interleaved(data, channels, &mut scratch);
                }
            },
            move |err: cpal::Error| {
                eprintln!("audio stream error: {err}");
                if is_fatal(err.kind()) {
                    on_error.store(true, Relaxed);
                }
            },
            None,
        )
        .context("building output stream")?;
    stream.play().context("starting output stream")?;
    OPENED.fetch_add(1, Relaxed);
    Ok(LiveOutput {
        stream,
        slot,
        failed,
        device_name: device_label(&device),
        sample_rate: rate,
        channels: config.channels,
    })
}

/// Why a kept output cannot play the next song, so a new one must be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unusable {
    /// The stream reported an error that ends it (not an overload, a move
    /// to the new default device, or a refused real-time priority).
    Failed,
    /// A callback panicked; on some hosts that ends the audio thread for
    /// good.
    Panicked,
    /// The default device is another one now (natively, as when headphones
    /// are plugged in); a new output per song used to follow it.
    DeviceChanged,
    /// The song is at another sample rate than the output.
    OtherRate,
}

/// What the output knows about itself when a song arrives.
#[derive(Clone, Copy, Debug)]
struct Health {
    failed: bool,
    poisoned: bool,
    on_default_device: bool,
    rate: u32,
}

/// Whether an output in this state can play a song at `song_rate`.
fn check(health: Health, song_rate: u32) -> Result<(), Unusable> {
    if health.failed {
        Err(Unusable::Failed)
    } else if health.poisoned {
        Err(Unusable::Panicked)
    } else if !health.on_default_device {
        Err(Unusable::DeviceChanged)
    } else if health.rate != song_rate {
        Err(Unusable::OtherRate)
    } else {
        Ok(())
    }
}

impl LiveOutput {
    /// Plays `player` from the next callback on, and returns the player it
    /// replaces (if any) for the caller to drop.  When this output cannot
    /// play it (see [`Unusable`]), the player comes back with the reason,
    /// for the caller to open a new output.
    pub fn play(&self, player: Player) -> Result<Option<Player>, (Unusable, Box<Player>)> {
        let health = Health {
            failed: self.failed.load(Relaxed),
            poisoned: self.slot.is_poisoned(),
            on_default_device: cpal::default_host()
                .default_output_device()
                .is_some_and(|device| device_label(&device) == self.device_name),
            rate: self.sample_rate,
        };
        match check(health, player.song().sample_rate) {
            Ok(()) => Ok(self.slot.replace(Some(player))),
            Err(why) => Err((why, Box::new(player))),
        }
    }

    /// Plays silence from the next callback on, and returns the player that
    /// was playing (if any) for the caller to drop.
    pub fn stop(&self) -> Option<Player> {
        self.slot.replace(None)
    }

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

impl Drop for LiveOutput {
    /// Frees the player before the stream goes, since the stream's callback
    /// may outlive it (see the module documentation).
    fn drop(&mut self) {
        drop(self.slot.replace(None));
    }
}

/// Whether a buffer is due to start before the callback filling it ran.
/// cpal's Web Audio host schedules each buffer right after the previous
/// one; when the main thread was blocked for longer than its two queued
/// buffers (as while a song loads), the schedule falls behind the
/// context's clock, every late buffer starts at once on top of the other,
/// and the song would play doubled at twice the speed until the schedule
/// catches up.  Late buffers are silent instead, so the song pauses for
/// about as long as the block and then goes on where it was.
fn is_late(timestamp: cpal::OutputStreamTimestamp) -> bool {
    timestamp.playback < timestamp.callback
}

/// Whether a stream error means the stream no longer plays.  An overload
/// (xrun), a move to the new default device, or a refused real-time
/// priority leaves it playing.
fn is_fatal(kind: cpal::ErrorKind) -> bool {
    !matches!(
        kind,
        cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied
    )
}

fn device_label(device: &cpal::Device) -> String {
    match device.id() {
        Ok(id) => format!("{id:?}"),
        Err(e) => format!("unnamed device ({e})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::ErrorKind;

    #[test]
    fn only_errors_that_end_the_stream_retire_the_output() {
        for kind in [
            ErrorKind::Xrun,
            ErrorKind::DeviceChanged,
            ErrorKind::RealtimeDenied,
        ] {
            assert!(!is_fatal(kind), "{kind:?} leaves the stream playing");
        }
        for kind in [
            ErrorKind::DeviceNotAvailable,
            ErrorKind::StreamInvalidated,
            ErrorKind::HostUnavailable,
        ] {
            assert!(is_fatal(kind), "{kind:?} ends the stream");
        }
    }

    #[test]
    fn a_buffer_due_before_its_callback_is_late() {
        let at = |callback_ms, playback_ms| cpal::OutputStreamTimestamp {
            callback: cpal::StreamInstant::from_millis(callback_ms),
            playback: cpal::StreamInstant::from_millis(playback_ms),
        };
        assert!(!is_late(at(1_000, 1_050)), "scheduled ahead, as usual");
        assert!(!is_late(at(1_000, 1_000)));
        assert!(is_late(at(3_000, 1_100)), "after a two-second block");
    }

    const HEALTHY: Health = Health {
        failed: false,
        poisoned: false,
        on_default_device: true,
        rate: 44_100,
    };

    #[test]
    fn a_healthy_output_plays_a_song_at_its_rate() {
        assert_eq!(check(HEALTHY, 44_100), Ok(()));
    }

    #[test]
    fn an_output_that_cannot_play_the_song_says_why() {
        let cases = [
            (
                Health {
                    failed: true,
                    ..HEALTHY
                },
                44_100,
                Unusable::Failed,
            ),
            (
                Health {
                    poisoned: true,
                    ..HEALTHY
                },
                44_100,
                Unusable::Panicked,
            ),
            (
                Health {
                    on_default_device: false,
                    ..HEALTHY
                },
                44_100,
                Unusable::DeviceChanged,
            ),
            (HEALTHY, 48_000, Unusable::OtherRate),
        ];
        for (health, song_rate, why) in cases {
            assert_eq!(check(health, song_rate), Err(why), "{health:?}");
        }
    }
}
