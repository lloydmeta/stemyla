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
//! [`WEB_BUFFER_FRAMES`] frames (3,072, about 139 ms at 44.1 kHz), plus the
//! output's latency, of the old song (closing the context used to cut it
//! off).  A kept output also
//! outlives main-thread blocks, such as the one while a song loads.  cpal
//! then schedules buffers in the past, which the Web Audio API starts at
//! once, on top of one another.  So in the browser a buffer that would
//! play over one already filled is left silent (see `plays_alone`), and
//! the song goes on at its own speed, not doubled.  Behind its clock,
//! though, cpal has no buffer queued ahead, and every further hold-up of
//! the page is a gap until its schedule catches up: at one second a
//! second, or never while the page stays busy for over two buffers at a
//! time.  An output that is behind when a song arrives is therefore
//! replaced ([`Unusable::Behind`]).
//!
//! An output that cannot play the next song ([`Unusable`]) is replaced.  In
//! the browser each one replaced still keeps its callback: an empty slot,
//! 64 KiB of scratch and cpal's own buffers (about 120 KiB), but no song.
//! Every song is at 44.1 kHz (decision 0012), so a new rate is rare, but a slow
//! phone can be busy for over 250 ms while it lays out a long song's lanes,
//! so there an output may be replaced about once a song.

use crate::player::Player;
use crate::slot::{PlayerSlot, Scratch, WEB_BUFFER_FRAMES};
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
    match open(player.sample_rate()) {
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
    let config = cpal::StreamConfig {
        buffer_size: buffer_size(cfg!(target_arch = "wasm32")),
        ..supported.into()
    };
    let channels = config.channels as usize;

    let slot = PlayerSlot::new();
    let in_callback = slot.clone();
    let mut scratch = Scratch::default();
    let failed = Arc::new(AtomicBool::new(false));
    let on_error = Arc::clone(&failed);
    let scheduled = Arc::new(AtomicU64::new(NOT_YET));
    let in_callback_scheduled = Arc::clone(&scheduled);
    let base = Arc::new(AtomicU64::new(0));
    let in_callback_base = Arc::clone(&base);
    let mut schedule = Schedule::new(cfg!(target_arch = "wasm32"));
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let length =
                    Duration::from_secs_f64((data.len() / channels) as f64 / f64::from(rate));
                let base = Duration::from_nanos(in_callback_base.load(Relaxed));
                let (start, fill) = schedule.next(info.timestamp().callback, length, base);
                in_callback_scheduled.store(nanos(start), Relaxed);
                if fill {
                    in_callback.render_interleaved(data, channels, &mut scratch);
                } else {
                    data.fill(0.0);
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
    // Read where cpal reads it, before the stream plays (and so before the
    // first callback, which in the browser runs later on this thread).
    base.store(
        base_latency(&stream).as_nanos().try_into().unwrap_or(0),
        Relaxed,
    );
    stream.play().context("starting output stream")?;
    OPENED.fetch_add(1, Relaxed);
    Ok(LiveOutput {
        stream,
        slot,
        failed,
        scheduled,
        device_name: device_label(&device),
        sample_rate: rate,
        channels: config.channels,
    })
}

/// The buffer size that the output asks cpal for: [`WEB_BUFFER_FRAMES`]
/// in the browser, and the platform's own size natively.
fn buffer_size(in_browser: bool) -> cpal::BufferSize {
    if in_browser {
        cpal::BufferSize::Fixed(WEB_BUFFER_FRAMES)
    } else {
        cpal::BufferSize::Default
    }
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
    /// busy, so until it catches up no buffer is queued ahead and every
    /// hold-up of the page is a gap in the song (see `plays_alone`); a new
    /// output starts a buffer ahead.
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
        match check(health, player.sample_rate()) {
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

    /// The output's clock, which stands still while the platform holds the
    /// output back (see [`LiveOutput::resume`]).
    pub fn clock(&self) -> Duration {
        self.stream
            .now()
            .saturating_duration_since(cpal::StreamInstant::ZERO)
    }
}

/// The stream's Web Audio context's `baseLatency`, which cpal adds to the
/// first buffer's start (see [`first_start`]); zero natively, where it does
/// not.
#[cfg(target_arch = "wasm32")]
fn base_latency(stream: &cpal::Stream) -> Duration {
    let cpal::platform::StreamInner::WebAudio(web) = stream.as_inner();
    let secs = js_sys::Reflect::get(web.audio_context().as_ref(), &"baseLatency".into())
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|s| s.is_finite() && *s >= 0.0)
        .unwrap_or(0.0);
    Duration::from_secs_f64(secs)
}

#[cfg(not(target_arch = "wasm32"))]
fn base_latency(_: &cpal::Stream) -> Duration {
    Duration::ZERO
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
/// arrives: until it catches up, about this long, no buffer is queued
/// ahead, so any hold-up of the page is a gap in the song.
const MAX_BEHIND: Duration = Duration::from_millis(250);

/// How coarse a browser's audio clock may be: `currentTime` moves in
/// 128-frame blocks (2.9 ms at 44.1 kHz), and Firefox rounds it to 1 ms
/// when blocks are shorter than that
/// ([MDN](https://developer.mozilla.org/docs/Web/API/BaseAudioContext/currentTime)).
/// With `privacy.resistFingerprinting`, Firefox rounds it to 16.667 ms,
/// more than this allows for, so there a late buffer may be left silent
/// where it would have played alone, or play over part of the one before.
const CLOCK_GRAIN: Duration = Duration::from_millis(4);

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

/// cpal's schedule as its data callback sees it: when each buffer starts,
/// and which buffers to fill.
struct Schedule {
    /// Whether this is the browser, where late buffers start at once (see
    /// [`plays_alone`]).  Natively every buffer is filled.
    in_browser: bool,
    first: Option<cpal::StreamInstant>,
    buffers: u32,
    audible_until: cpal::StreamInstant,
}

impl Schedule {
    fn new(in_browser: bool) -> Self {
        Self {
            in_browser,
            first: None,
            buffers: 0,
            audible_until: cpal::StreamInstant::ZERO,
        }
    }

    /// For the buffer `length` long called back at `callback` (with the
    /// context's `base` latency): when cpal starts it, and whether to fill
    /// it rather than leave it silent.
    fn next(
        &mut self,
        callback: cpal::StreamInstant,
        length: Duration,
        base: Duration,
    ) -> (cpal::StreamInstant, bool) {
        let first = *self
            .first
            .get_or_insert_with(|| first_start(callback, base, length));
        let start = nth_start(first, length, self.buffers);
        self.buffers = self.buffers.saturating_add(1);
        let alone = plays_alone(start, callback, length, self.audible_until);
        if let Some(end) = alone {
            self.audible_until = end;
        }
        (start, alone.is_some() || !self.in_browser)
    }
}

/// When cpal's Web Audio host starts the first buffer, called back at
/// `callback`: `baseLatency` and one buffer `length` ahead of the clock.
fn first_start(
    callback: cpal::StreamInstant,
    base: Duration,
    length: Duration,
) -> cpal::StreamInstant {
    callback.checked_add(base + length).unwrap_or(callback)
}

/// When it starts buffer `n` (from 0): each right after the one before,
/// whatever the clock says, since cpal never moves its schedule back to
/// the clock.  Worked out here rather than from cpal's `playback` time,
/// which adds `outputLatency` as it is when the buffer is filled (Bluetooth
/// earphones change it by a few hundred milliseconds), so no reading of
/// it here can be out of date.  When cpal fails to queue a buffer it does
/// not move its schedule on, so from then on this is a buffer late; it
/// reports the stream as invalid too, so the output is then replaced
/// ([`Unusable::Failed`]) before its schedule matters.
fn nth_start(first: cpal::StreamInstant, length: Duration, n: u32) -> cpal::StreamInstant {
    length
        .checked_mul(n)
        .and_then(|after| first.checked_add(after))
        .unwrap_or(first)
}

/// When a buffer `length` long, due to start at `start` and called back at
/// `callback`, ends if it plays alone: `None` when it would start before
/// `audible_until`, the end of the latest buffer filled.
///
/// The Web Audio API starts a buffer due in the past at once.  When the
/// main thread was blocked for longer than cpal's two queued buffers (as
/// while a song loads), both are called back late, both start at once, and
/// filling both would play the song doubled at twice its speed.  Filling
/// only the buffers that play alone keeps it at its own speed: one of each
/// late pair while the schedule catches up, and one a frame while the page
/// stays busy for over two buffers at a time (when it never catches up).
/// A healthy stream fills each buffer right after the one before.
///
/// A clock read up to [`CLOCK_GRAIN`] early (an eighth of a buffer at most,
/// for short buffers) still counts as after the latest end, while the other
/// buffer of a late pair, called back within a few milliseconds, would play
/// over nearly all of it.  A late buffer really starts once cpal has
/// started it, after the callback's work and at the next 128-frame block,
/// so the latest end worked out here is early by that much, and a buffer
/// filled after a late one may overlap it by as long, plus the grain.
fn plays_alone(
    start: cpal::StreamInstant,
    callback: cpal::StreamInstant,
    length: Duration,
    audible_until: cpal::StreamInstant,
) -> Option<cpal::StreamInstant> {
    let from = start.max(callback);
    let grain = CLOCK_GRAIN.min(length / 8);
    (from.checked_add(grain).unwrap_or(from) >= audible_until)
        .then(|| from.checked_add(length).unwrap_or(from))
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
    fn the_browser_asks_for_its_buffer_size_and_native_keeps_the_default() {
        assert_eq!(buffer_size(true), cpal::BufferSize::Fixed(3_072));
        assert_eq!(buffer_size(false), cpal::BufferSize::Default);
    }

    /// The buffer sizes of the tests: cpal's default in the browser, which
    /// native outputs may also use, and the browser's size now.
    const SIZES: [u32; 2] = [2_048, WEB_BUFFER_FRAMES];

    /// How long a buffer of `frames` frames plays at 44.1 kHz.
    fn length_of(frames: u32) -> Duration {
        Duration::from_secs_f64(f64::from(frames) / 44_100.0)
    }

    #[test]
    fn a_buffer_plays_alone_unless_it_starts_before_the_latest_one_ends() {
        let at = cpal::StreamInstant::from_millis;
        let plus = |t: cpal::StreamInstant, d: Duration| t.checked_add(d).unwrap();
        let ms = Duration::from_millis;
        for frames in SIZES {
            let length = length_of(frames);
            let next = plus(at(1_000), length);
            assert_eq!(
                plays_alone(next, at(1_000), length, next),
                Some(plus(next, length)),
                "on time, right after the one before ({frames})"
            );
            assert_eq!(
                plays_alone(at(1_000), at(3_000), length, at(1_000)),
                Some(plus(at(3_000), length)),
                "due two seconds ago, it starts as it is called back ({frames})"
            );
            let first_end = plus(at(3_000), length);
            assert_eq!(
                plays_alone(next, at(3_000), length, first_end),
                None,
                "the other late buffer would start at once on top of it ({frames})"
            );
            assert_eq!(
                plays_alone(next, at(3_010), length, first_end),
                None,
                "nor when the first took 10 ms to fill ({frames})"
            );
            assert!(
                plays_alone(
                    at(1_000),
                    plus(at(3_000), length - ms(3)),
                    length,
                    first_end
                )
                .is_some(),
                "a clock read 3 ms short still counts as after the latest end ({frames})"
            );
            assert_eq!(
                plays_alone(
                    at(1_000),
                    plus(at(3_000), length - ms(5)),
                    length,
                    first_end
                ),
                None,
                "5 ms short is the other buffer of a late pair ({frames})"
            );
        }
        let short = Duration::from_secs_f64(2_048.0 / 192_000.0);
        let first_end = plus(at(3_000), short);
        assert_eq!(
            plays_alone(at(1_000), at(3_008), short, first_end),
            None,
            "a buffer of 10.7 ms at 192 kHz allows a grain of 1.3 ms, not 4 ms"
        );
    }

    const MS: u64 = 1_000_000;
    /// A Web Audio render block, 128 frames at 44.1 kHz.
    const BLOCK: u64 = 2_902_494;

    /// Runs a [`Schedule`] in the browser against cpal's two buffers in
    /// flight, `frames` frames at 44.1 kHz each.  The page can call back at
    /// `free(t)` a buffer that ended at `t`, reads the clock in steps of
    /// `step`, and is busy for `cost` with each callback, after which the
    /// Web Audio API starts the buffer at the next render block.  Returns
    /// when each buffer was called back and when each one filled really
    /// plays, until `until`, in nanoseconds.
    fn run(
        frames: u32,
        free: impl Fn(u64) -> u64,
        step: u64,
        cost: u64,
        until: u64,
    ) -> (Vec<u64>, Vec<(u64, u64)>) {
        let at = cpal::StreamInstant::from_nanos;
        let length = length_of(frames);
        let mut schedule = Schedule::new(true);
        let mut ends = [0, 0];
        let mut called = Vec::new();
        let mut filled = Vec::new();
        let mut busy_until = 0;
        loop {
            let next = usize::from(ends[1] < ends[0]);
            let now = free(ends[next]).max(busy_until);
            if now > until {
                return (called, filled);
            }
            busy_until = now + cost;
            called.push(now);
            let (start, fill) = schedule.next(at(now / step * step), length, Duration::ZERO);
            let plays = nanos(start).max((now + cost).div_ceil(BLOCK) * BLOCK);
            ends[next] = plays + nanos_of(length);
            if fill {
                filled.push((plays, ends[next]));
            }
        }
    }

    fn nanos_of(length: Duration) -> u64 {
        length.as_nanos() as u64
    }

    /// The most that one buffer filled plays over the one filled before.
    fn most_overlap(filled: &[(u64, u64)]) -> u64 {
        filled
            .windows(2)
            .map(|w| w[0].1.saturating_sub(w[1].0))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn a_healthy_stream_fills_every_buffer_back_to_back() {
        for frames in SIZES {
            let (called, filled) = run(frames, |t| t, 1, 0, 3_000 * MS);
            assert_eq!(filled.len(), called.len(), "{frames}");
            assert!(filled.windows(2).all(|w| w[1].0 == w[0].1), "{frames}");
        }
    }

    #[test]
    fn natively_every_buffer_is_filled() {
        let at = cpal::StreamInstant::from_millis;
        let length = Duration::from_millis(46);
        let mut schedule = Schedule::new(false);
        assert!(schedule.next(at(1_000), length, Duration::ZERO).1);
        assert!(schedule.next(at(3_000), length, Duration::ZERO).1);
        assert!(
            schedule.next(at(3_000), length, Duration::ZERO).1,
            "a late pair, which in the browser would be filled once"
        );
    }

    fn blocked(t: u64) -> u64 {
        if (1_000 * MS..2_000 * MS).contains(&t) {
            2_000 * MS
        } else {
            t
        }
    }

    #[test]
    fn after_a_block_the_song_goes_on_at_its_own_speed() {
        // An exact clock, one that moves a render block at a time (a buffer is
        // 16 or 24 blocks, so it never reads short) and one rounded to 2 ms,
        // with callbacks that take a while.
        for frames in SIZES {
            let buffers = (900 * MS) as f64 / nanos_of(length_of(frames)) as f64;
            let once_a_buffer = buffers.floor() as usize..=buffers.ceil() as usize;
            for (step, cost) in [(1, 0), (BLOCK, MS / 3), (BLOCK, MS), (2 * MS, MS / 3)] {
                let (called, filled) = run(frames, blocked, step, cost, 4_000 * MS);
                let overlap = most_overlap(&filled);
                assert!(
                    overlap <= 4 * MS + cost + BLOCK,
                    "{overlap} ns overlap ({frames})"
                );
                let catching_up = |t: &u64| (2_000 * MS..2_900 * MS).contains(t);
                let calls = called.iter().filter(|t| catching_up(t)).count();
                let fills = filled.iter().filter(|f| catching_up(&f.0)).count();
                // Called back twice a buffer while the schedule catches up, but
                // filled once a buffer, from the end of the block on.
                assert!(
                    once_a_buffer.contains(&fills),
                    "{fills} filled in 900 ms, not {once_a_buffer:?} ({frames}, {step}, {cost})"
                );
                assert!(
                    calls + 1 >= 2 * fills,
                    "{calls} called back, {fills} filled ({frames})"
                );
                let resumed = filled.iter().find(|f| f.0 >= 2_000 * MS).unwrap().0;
                assert!(
                    resumed <= 2_000 * MS + cost + BLOCK,
                    "resumed at {resumed} ({frames})"
                );
                let played: u64 = filled
                    .iter()
                    .filter(|f| f.0 >= 2_000 * MS)
                    .map(|f| f.1 - f.0)
                    .sum();
                assert!(
                    played >= 1_900 * MS,
                    "{played} ns of the 2 s after the block ({frames})"
                );
            }
        }
    }

    #[test]
    fn a_page_too_busy_for_the_schedule_to_catch_up_still_plays() {
        // A frame of 150 ms is longer than two buffers of either size.
        let frame = |t: u64| t.div_ceil(150 * MS) * 150 * MS;
        for frames in SIZES {
            assert!(2 * length_of(frames) < Duration::from_millis(150));
            let (called, filled) = run(frames, frame, 1, 0, 3_000 * MS);
            let overlap = most_overlap(&filled);
            assert!(overlap <= 4 * MS + BLOCK, "{overlap} ns overlap ({frames})");
            let later = |t: &u64| *t >= 1_500 * MS;
            let calls = called.iter().filter(|t| later(t)).count();
            let fills = filled.iter().filter(|f| later(&f.0)).count();
            assert!(
                (9..=11).contains(&fills) && calls >= 2 * fills,
                "one of the two buffers a frame: {fills} of {calls} in 1.5 s ({frames})"
            );
        }
    }

    #[test]
    fn a_late_pair_fills_one_buffer_however_long_filling_takes() {
        for frames in SIZES {
            let per_second = (1_000 * MS / nanos_of(length_of(frames))) as usize;
            for cost in [5 * MS, 15 * MS, 30 * MS] {
                let (called, filled) = run(frames, blocked, 1, cost, 3_000 * MS);
                let overlap = most_overlap(&filled);
                assert!(
                    overlap <= 4 * MS + cost + BLOCK,
                    "{overlap} ns overlap ({frames})"
                );
                let after = |t: &u64| *t >= 2_000 * MS;
                let calls = called.iter().filter(|t| after(t)).count();
                let fills = filled.iter().filter(|f| after(&f.0)).count();
                assert!(
                    2 * fills >= per_second && 2 * fills <= calls + 2,
                    "{fills} filled of {calls} called back, filling for {cost} ns ({frames})"
                );
            }
        }
    }

    #[test]
    fn buffers_follow_one_another_from_the_first_whatever_the_clock() {
        let at = cpal::StreamInstant::from_millis;
        let length = Duration::from_millis(46);
        // cpal's first buffer: baseLatency (10 ms) and a buffer ahead.
        let first = first_start(at(1_000), Duration::from_millis(10), length);
        assert_eq!(first, at(1_056));
        assert_eq!(nth_start(first, length, 0), first);
        assert_eq!(nth_start(first, length, 10), at(1_516));
        assert_eq!(
            behind(at(3_500), nanos(nth_start(first, length, 11))),
            Duration::from_millis(1_938),
            "buffer 11, due at 1,562 ms, after a block to 3,500 ms"
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
