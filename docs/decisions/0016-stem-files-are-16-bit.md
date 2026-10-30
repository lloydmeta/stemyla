# 0016: stem files hold 16-bit PCM, and the mix and source files 32-bit float

Status: accepted (2026-10-25, by the maintainer)

## Context

Decision 0015 puts a song's audio in OPFS session files.  `app/src/store.rs` already saves separated stems as 16-bit WAVs, and a restored session plays them.  A 30-minute song has six stereo stems: 1.9 GB at 16 bits, 3.8 GB at 32-bit float.  Playback reads all six stems and the original lane.  The six stems take about 1.1 MB per second at 16 bits, 2.1 MB per second at 32-bit float.  The 32-bit float original lane adds 0.35 MB per second, and a 16-bit classic-drums stem 0.18 MB per second.

## Options

* 16-bit PCM, as the stems are stored today.  The noise floor of one stem is about -96 dBFS, and of six stems summed about -88 dBFS.
* 32-bit float.  No rounding after the separation, twice the storage and twice the reads.
* 24-bit PCM.  Fewer bytes than float, but three-byte samples cost more to read and write in blocks.

## Decision

Stem files hold 16-bit PCM, little-endian, interleaved stereo.  A stem file is the stem's WAV, as today.  The mix that the worker separates is one file of 32-bit float, so the model's input does not change.  A 30-minute mix takes about 635 MB.  Each decoded source file is also 32-bit float.  For a song of one source file, that file is the mix.  Overviews are separate small files.

## Consequences

* A separation's output is rounded to 16 bits once, when the worker writes it.  Playback, the mix and a save read the rounded samples.  A restored session sounds the same as a new one.
* A save can never hold more than 16 bits of a stem.  Today's FLAC is 16-bit anyway (TD-52).
* The model separates the same `f32` mix as today.  M4 of the plan checks that its stems do not change.
* The block source of decision 0015 reads two sample formats.
