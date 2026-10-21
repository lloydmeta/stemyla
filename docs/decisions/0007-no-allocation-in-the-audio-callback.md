# 0007: no allocation in the audio callback

Status: accepted (2026-10-19, by the maintainer)

## Context

RT-1 said that the audio callback never waits and never frees a player.  It did not say that the callback must not allocate.  The default stretcher allocated in the callback on every call: wsola 0.1.0's `pull` returned a new `Vec`, and wsola's input buffer grew over the first 1.5 s of play.  Chipmunk mode called `dsp::double_speed`, which returns a new `Vec`, and a loop of a few frames grew the player's `anchors` queue (TD-15).  An allocator can take a lock or fault a page, so its time has no bound, and a callback that misses its deadline is heard as a click.

## Options

* (a) No allocation and no free after warm-up.  Warm-up is the work that `Player::with_config` does, on the thread that makes the player.  A test counts allocations in the callback.
* (b) No allocation at all.  This cannot hold, since a new player must make its buffers.
* (c) Allocation allowed, and measured only.  This leaves TD-15 open.

## Decision

(a), chosen by the maintainer on 2026-10-19.

## Consequences

* `Stretcher::warm_up` makes each stretcher's buffers reach their largest size before the player enters the slot.  For wsola, this is a push of about 650 KB of silence and a reset, on the thread that opens a song.
* wsola is 0.2.0, whose `pull` gives an iterator and does not allocate.
* `engine/tests/realtime.rs` runs the callback under a counting allocator, through tempo, chipmunk, seek, loop and mix changes, for each stretcher that the app ships.
* RT-1 covers only the stretchers that the app ships.  `timestretch` and `cute-dsp` are for benches only.
* The test runs natively.  The WebAssembly build is not counted (TD-25).
