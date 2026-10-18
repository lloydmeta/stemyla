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
//! `starts`).  cpal's schedule then catches up at one second a second, so
//! an output that is behind when a song arrives is replaced
//! ([`Unusable::Behind`]) rather than starting the song silent.
//!
//! An output that cannot play the next song ([`Unusable`]) is replaced.  In
//! the browser each one replaced still keeps its callback: an empty slot,
//! 64 KiB of scratch and cpal's own buffers (about 100 KiB), but no song.
//! Separated songs are all at 44.1 kHz, so a new rate is rare, but a slow
//! phone can be busy for over 250 ms while it lays out a long song's lanes,
//! so there an output may be replaced about once a song.

use crate::player::Player;
use crate::slot::{PlayerSlot, Scratch};
use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Duration;

pub struct LiveOutput {
    stream: cpal::Stream,
    slot: PlayerSlot,
    /// Set by the stream's error callback.
    failed: Arc<AtomicBool>,
    /// When the latest buffer is due to start, as [`nanos`] of the stream's
    /// clock ([`NOT_YET`] before the first callback).
    scheduled: Arc<AtomicU64>,
    /// The output's latency in nanoseconds, which cpal adds to a buffer's
    /// start in its `playback` time (see [`starts`]).
    latency: Arc<AtomicU64>,
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
    let scheduled = Arc::new(AtomicU64::new(NOT_YET));
    let in_callback_scheduled = Arc::clone(&scheduled);
    let latency = Arc::new(AtomicU64::new(0));
    let in_callback_latency = Arc::clone(&latency);
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let latency = Duration::from_nanos(in_callback_latency.load(Relaxed));
                let start = starts(info.timestamp(), latency);
                in_callback_scheduled.store(nanos(start), Relaxed);
                let length =
                    Duration::from_secs_f64((data.len() / channels) as f64 / f64::from(rate));
                if cfg!(target_arch = "wasm32") && is_late(start, info.timestamp().callback, length)
                {
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
    let out = LiveOutput {
        stream,
        slot,
        failed,
        scheduled,
        latency,
        device_name: device_label(&device),
        sample_rate: rate,
        channels: config.channels,
    };
    out.refresh_latency();
    Ok(out)
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
    /// The browser's schedule fell behind its clock while the page was
    /// busy (see `starts`), so a song would start silent for about as
    /// long; a new output starts on time.
    Behind,
}

/// What the output knows about itself when a song arrives.
#[derive(Clone, Copy, Debug)]
struct Health {
    failed: bool,
    poisoned: bool,
    on_default_device: bool,
    rate: u32,
    /// How long ago the latest buffer was due to play.
    behind: Duration,
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
    } else if health.behind > MAX_BEHIND {
        Err(Unusable::Behind)
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
        self.refresh_latency();
        let health = Health {
            failed: self.failed.load(Relaxed),
            poisoned: self.slot.is_poisoned(),
            on_default_device: cpal::default_host()
                .default_output_device()
                .is_some_and(|device| device_label(&device) == self.device_name),
            rate: self.sample_rate,
            // Natively cpal calls back on its own thread, which the page
            // being busy does not hold up.
            behind: if cfg!(target_arch = "wasm32") {
                behind(self.stream.now(), self.scheduled.load(Relaxed))
            } else {
                Duration::ZERO
            },
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
        self.refresh_latency();
        self.stream.play().context("resuming the audio output")
    }

    /// The output's clock, which stands still while the platform holds the
    /// output back (see [`LiveOutput::resume`]).
    pub fn clock(&self) -> Duration {
        self.stream
            .now()
            .saturating_duration_since(cpal::StreamInstant::ZERO)
    }

    /// Reads the Web Audio context's latency for the callback: its fixed
    /// `baseLatency` and its `outputLatency`, which changes with the device
    /// (Bluetooth earphones add a few hundred milliseconds) and reads 0
    /// until the context runs.  cpal reads it afresh for every buffer, but
    /// the callback cannot reach the context, so the page calls this every
    /// frame (and it runs when the output opens, with each song and each
    /// resume).  Natively the latency stays zero: there callbacks are not
    /// held up.
    pub fn refresh_latency(&self) {
        let latency = self.web_latency().unwrap_or(Duration::ZERO);
        self.latency
            .store(latency.as_nanos().try_into().unwrap_or(0), Relaxed);
    }

    #[cfg(target_arch = "wasm32")]
    fn web_latency(&self) -> Option<Duration> {
        let cpal::platform::StreamInner::WebAudio(web) = self.stream.as_inner();
        let secs = |name: &str| {
            js_sys::Reflect::get(web.audio_context().as_ref(), &name.into())
                .ok()
                .and_then(|v| v.as_f64())
                .filter(|s| s.is_finite() && *s >= 0.0)
                .unwrap_or(0.0)
        };
        Some(Duration::from_secs_f64(
            secs("baseLatency") + secs("outputLatency"),
        ))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn web_latency(&self) -> Option<Duration> {
        None
    }
}

impl Drop for LiveOutput {
    /// Frees the player before the stream goes, since the stream's callback
    /// may outlive it (see the module documentation).
    fn drop(&mut self) {
        drop(self.slot.replace(None));
    }
}

/// [`LiveOutput::scheduled`] before the stream's first callback.
const NOT_YET: u64 = u64::MAX;

/// How far behind its clock a kept output's schedule may be when a song
/// arrives: the song starts silent for about this long.
const MAX_BEHIND: Duration = Duration::from_millis(250);

fn nanos(at: cpal::StreamInstant) -> u64 {
    at.saturating_duration_since(cpal::StreamInstant::ZERO)
        .as_nanos()
        .try_into()
        .unwrap_or(NOT_YET - 1)
}

/// How long before `now` the latest buffer, due at `scheduled` (as
/// [`nanos`]), was due to play: zero while it is still to come, and
/// before the first callback.
fn behind(now: cpal::StreamInstant, scheduled: u64) -> Duration {
    if scheduled == NOT_YET {
        return Duration::ZERO;
    }
    Duration::from_nanos(nanos(now).saturating_sub(scheduled))
}

/// When the buffer starts on the context's clock: cpal's `playback` time is
/// that start plus the output's `latency`, and its `callback` time is the
/// clock with none.
///
/// A buffer that starts well before its callback ran is late (see
/// [`is_late`]).  cpal's Web Audio
/// host schedules each buffer right after the previous one; when the main
/// thread was blocked for longer than its two queued buffers (as while a
/// song loads), the schedule falls behind the context's clock, every late
/// buffer starts at once on top of the other, and the song would play
/// doubled at twice the speed until the schedule catches up.  Late buffers
/// are silent instead, so the song pauses for about as long as the block
/// and then goes on where it was.
fn starts(timestamp: cpal::OutputStreamTimestamp, latency: Duration) -> cpal::StreamInstant {
    timestamp
        .playback
        .checked_sub(latency)
        .unwrap_or(cpal::StreamInstant::ZERO)
}

/// Whether a buffer `length` long, due to start at `start`, was due over a
/// buffer's length before its callback ran at `callback`.  The margin
/// covers a latency reading up to a frame old: cpal's own reading, in
/// `playback`, is fresh, so one that dropped since (earphones unplugged)
/// would otherwise make every buffer look late, and silent, until the next
/// reading.
fn is_late(start: cpal::StreamInstant, callback: cpal::StreamInstant, length: Duration) -> bool {
    start.checked_add(length).is_some_and(|end| end < callback)
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
    fn a_buffer_is_late_only_when_due_over_its_length_before_its_callback() {
        let at = cpal::StreamInstant::from_millis;
        let length = Duration::from_millis(46);
        assert!(!is_late(at(1_000), at(1_000), length), "on time");
        assert!(
            !is_late(at(960), at(1_000), length),
            "a latency reading 40 ms too high (earphones just unplugged) still plays"
        );
        assert!(
            is_late(at(900), at(1_000), length),
            "a buffer and more behind"
        );
        assert!(
            is_late(at(1_000), at(3_000), length),
            "after a two-second block"
        );
    }

    #[test]
    fn a_buffer_starts_its_latency_before_its_playback_time() {
        let at = |callback_ms, playback_ms| cpal::OutputStreamTimestamp {
            callback: cpal::StreamInstant::from_millis(callback_ms),
            playback: cpal::StreamInstant::from_millis(playback_ms),
        };
        let late = |t: cpal::OutputStreamTimestamp, latency_ms| {
            starts(t, Duration::from_millis(latency_ms)) < t.callback
        };
        assert!(!late(at(1_000, 1_050), 0), "scheduled ahead, as usual");
        assert!(!late(at(1_000, 1_000), 0));
        assert!(late(at(3_000, 1_100), 0), "after a two-second block");
        // Bluetooth earphones: 200 ms of latency in the playback time.
        assert!(!late(at(1_000, 1_250), 200), "ahead, with latency");
        assert!(
            late(at(1_000, 1_150), 200),
            "started 50 ms ago, though its playback time is still to come"
        );
        assert_eq!(
            starts(at(1_000, 100), Duration::from_millis(200)),
            cpal::StreamInstant::ZERO,
            "never before the clock began"
        );
    }

    const HEALTHY: Health = Health {
        failed: false,
        poisoned: false,
        on_default_device: true,
        rate: 44_100,
        behind: Duration::ZERO,
    };

    #[test]
    fn a_schedule_is_behind_by_how_long_ago_its_latest_buffer_was_due() {
        let at = cpal::StreamInstant::from_millis;
        assert_eq!(behind(at(5_000), NOT_YET), Duration::ZERO, "no buffer yet");
        assert_eq!(behind(at(1_000), nanos(at(1_050))), Duration::ZERO, "ahead");
        assert_eq!(
            behind(at(3_000), nanos(at(1_100))),
            Duration::from_millis(1_900),
            "after a two-second block"
        );
        let waited = |ms| Health {
            behind: Duration::from_millis(ms),
            ..HEALTHY
        };
        assert_eq!(check(waited(100), 44_100), Ok(()), "a short wait is kept");
        assert_eq!(check(waited(2_000), 44_100), Err(Unusable::Behind));
    }

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
