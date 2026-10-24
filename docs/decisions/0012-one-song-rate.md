# 0012: every song in the app is at 44.1 kHz

Status: accepted (2026-10-24, by the maintainer)

## Context

The separation model takes only 44.1 kHz (`roformer::SAMPLE_RATE`).  The app refused to separate a song at another rate.  Songs at 48 kHz are common.

## Options

* Convert each song to 44.1 kHz when the app opens it.
* Convert only the separation input.  Stems and the original lane then have different rates, so one of them must convert again.
* Convert in a worker.  `stem-worker` cannot take `engine` (decision 0005), and the worker's memory never shrinks.
* Use `rubato`, or the browser's `OfflineAudioContext`, for the conversion.

## Decision

`Song::from_audio` converts each stem to `engine::resample::SONG_RATE` (44,100), on the page thread.  The converter is a windowed-sinc resampler in `engine`, with 64 taps at 48 kHz.  It takes every standard rate.  A stem at 44.1 kHz stays as it is, with no copy.  The plan is `plans/active/48k-audio.md`.

## Consequences

* Separation, playback, storage, the worker and the wire format do not change.
* A download of the "original" lane is at 44.1 kHz, not at the rate of the source file.
* Opening a song at another rate freezes the page for longer: about 1.3 s for 5 minutes of 48 kHz audio in WebAssembly on V8.  A decode worker can remove both freezes later.
* Sound above 20 kHz can alias, but stays above 20 kHz.
* A test holds `SONG_RATE` equal to `roformer::SAMPLE_RATE`.
