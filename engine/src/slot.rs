//! The player an audio output plays, swapped while the output keeps running.
//!
//! An output is opened once and kept: what it plays is whatever [`Player`]
//! sits in its [`PlayerSlot`].  Opening a song puts a new player in the slot
//! and hands the old one back to the caller, which frees it (and its
//! sources, which hold the old song's lanes) on its own thread.  The audio
//! callback never waits for the slot and never frees a player: if the slot is
//! empty, or the UI holds it at that moment, the callback renders silence.
//!
//! This also keeps a replaced song from leaking where the audio backend
//! never frees a stream's callback (cpal 0.18's Web Audio host keeps it in
//! a reference cycle): the callback holds only the slot, and the slot is
//! emptied before the stream is dropped.

use crate::player::Player;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use web_time::Instant;

/// Frames rendered per inner step; the callback's scratch buffers hold this
/// many.
pub const SCRATCH_FRAMES: usize = 8192;

/// The frames of each buffer that the browser's audio output asks for
/// (`live`, on wasm32).  At 44.1 kHz one buffer is about 69.7 ms.  cpal
/// queues two, so a change to the mix is heard up to about 139 ms later.
/// The maintainer chose this size on 2026-10-28 (decision 0017).  It is
/// here, not in `live`, because `xtask` and the page's tests read it
/// without the `cpal` feature.
pub const WEB_BUFFER_FRAMES: u32 = 3_072;

// One callback of the browser's output is one inner step.
const _: () = assert!(WEB_BUFFER_FRAMES as usize <= SCRATCH_FRAMES);

/// A shared, swappable [`Player`].  Clones share the same slot.
#[derive(Clone, Default)]
pub struct PlayerSlot(Arc<Mutex<Option<Player>>>);

/// The callback's stereo scratch buffers, allocated once per output so the
/// audio path does not allocate.
pub struct Scratch {
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            left: vec![0.0; SCRATCH_FRAMES],
            right: vec![0.0; SCRATCH_FRAMES],
        }
    }
}

impl PlayerSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts `player` in the slot (or empties it, for `None`) and returns the
    /// player it held.  The caller drops what comes back, so the old player
    /// and its sources are freed on the caller's thread, after the slot is
    /// unlocked.  Waits at most for one audio callback to finish.
    pub fn replace(&self, player: Option<Player>) -> Option<Player> {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::replace(&mut *held, player)
    }

    /// Whether a callback panicked while rendering.  The slot then stays
    /// silent for good: on some hosts the panic has ended the audio thread,
    /// so the output must be replaced rather than reused.
    pub fn is_poisoned(&self) -> bool {
        self.0.is_poisoned()
    }

    /// Whether a player is in the slot.
    pub fn is_occupied(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// The audio callback: fills interleaved `data` (`channels` samples per
    /// frame, at least two) from the player in the slot, left and right in
    /// the first two channels and silence in any others, and publishes the
    /// callback's cost in the player's [`crate::SharedParams`].  Renders
    /// silence when the slot is empty, held by another thread, or poisoned
    /// ([`Self::is_poisoned`]); it never blocks.
    pub fn render_interleaved(&self, data: &mut [f32], channels: usize, scratch: &mut Scratch) {
        debug_assert!(channels >= 2, "the output has at least two channels");
        let mut held = match self.0.try_lock() {
            Ok(held) => held,
            Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => {
                data.fill(0.0);
                return;
            }
        };
        let Some(player) = held.as_mut() else {
            data.fill(0.0);
            return;
        };
        let t0 = Instant::now();
        let frames = data.len() / channels;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(scratch.left.len());
            player.process(&mut scratch.left[..n], &mut scratch.right[..n]);
            for i in 0..n {
                let f = &mut data[(done + i) * channels..(done + i + 1) * channels];
                f[0] = scratch.left[i];
                f[1] = scratch.right[i];
                for extra in f.iter_mut().skip(2) {
                    *extra = 0.0;
                }
            }
            done += n;
        }
        let ns = t0.elapsed().as_nanos() as u64;
        let params = player.params();
        params.last_callback_ns.store(ns, Relaxed);
        params.max_callback_ns.fetch_max(ns, Relaxed);
        params.callbacks.fetch_add(1, Relaxed);
        params.total_callback_ns.fetch_add(ns, Relaxed);
        params.last_callback_frames.store(frames as u64, Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SharedParams;
    use crate::player::PlayerConfig;
    use crate::song::{Song, Stem};
    use crate::source::{MemorySource, StemSource};
    use crate::stretch::Varispeed;
    use std::sync::Weak;
    use std::sync::mpsc;
    use std::time::Duration;

    fn song(value: f32, frames: usize) -> Arc<Song> {
        Arc::new(Song {
            sample_rate: 44_100,
            frames,
            stems: vec![Stem {
                name: "lloyd".into(),
                left: vec![value; frames].into(),
                right: vec![value; frames].into(),
            }],
        })
    }

    fn playing(song: Arc<Song>) -> Player {
        let params = Arc::new(SharedParams::new(song.stems.len()));
        params.playing.store(true, Relaxed);
        Player::with_config(
            song,
            params,
            Box::new(Varispeed::new()),
            PlayerConfig::default(),
        )
    }

    /// Renders one callback of `frames` stereo frames into a buffer that
    /// starts full of a marker value, so untouched samples show.
    fn callback(slot: &PlayerSlot, frames: usize, channels: usize) -> Vec<f32> {
        let mut data = vec![7.0; frames * channels];
        slot.render_interleaved(&mut data, channels, &mut Scratch::default());
        data
    }

    /// A playing player of one memory lane, and a `Weak` of that lane's
    /// source, which holds the lane's samples.
    fn sourced(value: f32, frames: usize) -> (Player, Weak<dyn StemSource>) {
        let source: Arc<dyn StemSource> = Arc::new(MemorySource::new(
            vec![value; frames].into(),
            vec![value; frames].into(),
        ));
        let weak = Arc::downgrade(&source);
        let params = Arc::new(SharedParams::new(1));
        params.playing.store(true, Relaxed);
        let player = Player::with_sources(
            44_100,
            frames,
            params,
            Box::new(Varispeed::new()),
            PlayerConfig::default(),
            vec![source],
        );
        (player, weak)
    }

    /// The player holds its sources, not the song: the replaced player's
    /// sources are freed on the caller's thread, not in the callback.
    #[test]
    fn replacing_the_player_frees_the_previous_song() {
        let slot = PlayerSlot::new();
        let (first, first_weak) = sourced(0.25, 20_000);
        assert!(slot.replace(Some(first)).is_none());
        callback(&slot, 512, 2);
        assert!(first_weak.upgrade().is_some(), "the slot keeps it alive");

        let (second, second_weak) = sourced(0.5, 20_000);
        let previous = slot.replace(Some(second));
        assert!(previous.is_some(), "the old player comes back");
        callback(&slot, 512, 2);
        assert!(first_weak.upgrade().is_some(), "freed by the caller");
        drop(previous);
        assert!(first_weak.upgrade().is_none(), "the first song is freed");

        drop(slot.replace(None));
        assert!(second_weak.upgrade().is_none(), "emptying frees the second");
        assert!(!slot.is_occupied());
    }

    #[test]
    fn a_clone_shares_the_slot_without_keeping_its_song() {
        let slot = PlayerSlot::new();
        let in_callback = slot.clone();
        let (player, weak) = sourced(0.25, 20_000);
        drop(slot.replace(Some(player)));
        assert!(in_callback.is_occupied());
        drop(slot.replace(None));
        assert!(weak.upgrade().is_none(), "a clone left alive holds no song");
    }

    #[test]
    fn an_empty_slot_renders_silence_in_every_channel() {
        let data = callback(&PlayerSlot::new(), 300, 4);
        assert!(data.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_held_slot_renders_silence_without_waiting() {
        let slot = PlayerSlot::new();
        drop(slot.replace(Some(playing(song(0.25, 20_000)))));
        let held = slot.0.lock().expect("lock");
        let (tx, rx) = mpsc::channel();
        let audio = slot.clone();
        let thread = std::thread::spawn(move || tx.send(callback(&audio, 512, 2)));
        let data = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the callback returns while the slot is held");
        drop(held);
        thread.join().expect("joined").expect("sent");
        assert!(data.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_poisoned_slot_stays_silent_and_still_frees_its_players() {
        let slot = PlayerSlot::new();
        drop(slot.replace(Some(playing(song(0.25, 20_000)))));
        assert!(!slot.is_poisoned());
        let poisoner = slot.clone();
        let panicked = std::thread::spawn(move || {
            let _held = poisoner.0.lock().expect("lock");
            panic!("a render that panics");
        })
        .join();
        assert!(panicked.is_err());
        assert!(slot.is_poisoned());
        assert!(callback(&slot, 64, 2).iter().all(|v| *v == 0.0));

        let next = song(0.25, 20_000);
        let next_weak = Arc::downgrade(&next);
        assert!(slot.replace(Some(playing(next))).is_some(), "the old one");
        assert!(slot.is_poisoned(), "for the owner to replace the output");
        assert!(callback(&slot, 64, 2).iter().all(|v| *v == 0.0));
        drop(slot.replace(None));
        assert!(next_weak.upgrade().is_none());
    }

    #[test]
    fn a_player_fills_left_and_right_and_silences_the_rest() {
        let slot = PlayerSlot::new();
        let s = song(0.25, 20_000);
        let mut reference = playing(Arc::clone(&s));
        drop(slot.replace(Some(playing(s))));
        // More frames than the scratch holds, so the inner loop runs twice.
        let frames = SCRATCH_FRAMES + 100;
        let data = callback(&slot, frames, 3);
        let [l, r] = reference.render(frames);
        for i in 0..frames {
            assert_eq!(data[i * 3], l[i], "left {i}");
            assert_eq!(data[i * 3 + 1], r[i], "right {i}");
            assert_eq!(data[i * 3 + 2], 0.0, "third channel {i}");
        }
        assert!(l.iter().all(|v| (*v - 0.25).abs() < 1e-6));
    }

    #[test]
    fn a_callback_publishes_its_cost_in_the_players_params() {
        let slot = PlayerSlot::new();
        let player = playing(song(0.25, 20_000));
        let params = Arc::clone(player.params());
        drop(slot.replace(Some(player)));
        callback(&slot, 256, 2);
        callback(&slot, 128, 2);
        assert_eq!(params.callbacks.load(Relaxed), 2);
        assert_eq!(params.last_callback_frames.load(Relaxed), 128);
    }
}
