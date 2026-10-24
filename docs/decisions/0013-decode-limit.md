# 0013: a decoded file holds at most 1 GiB of samples

Status: accepted (2026-10-25, by the maintainer)

## Context

The app adds FLAC input (`plans/active/flac.md`).  A FLAC frame of 65,535 frames of silence in stereo takes about 17 bytes, and decodes to 524,280 bytes of `f32` ([RFC 9639](https://www.rfc-editor.org/rfc/rfc9639.html), sections 8.2 and 9.2.3).  So a 1 MB file can hold about 30 GB of audio.  `engine::decode` grew each channel with no limit.  WebAssembly on wasm32 has 4 GiB at most, and iOS stops the tab at 2 GB (MEM-4).  A very long WAV or MP3 also stops the tab, but its file is then as large as its audio.

## Options

* A limit on the decoded samples of every format.
* A limit on FLAC only.  A long WAV or MP3 still stops the tab.
* A limit on the decoded size relative to the file's size.  A legitimate FLAC of silence would fail.
* No limit.  A small file can stop the tab.

## Decision

`engine::decode` stops with "the song is too long to open" when the decoded `f32` of all the file's channels passes `MAX_DECODED_BYTES`, 1 GiB.  The limit is the same for every format.

## Consequences

* A file is refused when it holds more than 50.7 minutes of 44.1 kHz stereo, or 23.3 minutes of 96 kHz stereo.  A file with more channels is refused sooner, because the decoder keeps every channel.
* A small crafted or damaged file cannot make the page reserve or decode more than the limit.
* The limit is on one file.  A song of several stems can still pass 2 GB on an iPhone; MEM-4 keeps that as it was.
* Added after review rounds 1 and 2, on 2026-10-25, by the agent; not part of the maintainer's acceptance:
  * While a channel grows past its reservation, a realloc holds the old and the new buffer.  So the heap can briefly hold about 1.5 times the limit.  WebAssembly memory never shrinks, so the page keeps that high-water mark.
  * A mono file counts as stereo, because `Song::from_audio` makes it stereo.  So a mono file decodes to at most half the limit.  Without this, the realloc of its one channel held 1.9 times the limit.
  * A WAV or FLAC whose header states more than the limit is refused before it decodes.  An MP3 without a Xing or VBRI tag states only an estimate, which a quiet start can make 10 times too long.  So an MP3 stops at the limit while it decodes.

Amended on 2026-10-25 by the agent, after review round 1: the Context said "about 20 bytes" and "about 25 GB".  The hand-made frames of the tests are 16 to 18 bytes, so it now says "about 17 bytes" and "about 30 GB".
