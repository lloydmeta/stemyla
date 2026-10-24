# 0013: a decoded file holds at most 1 GiB of samples

Status: accepted (2026-10-25, by the maintainer)

## Context

The app adds FLAC input (`plans/active/flac.md`).  A FLAC frame of 65,535 frames of silence in stereo takes about 20 bytes, and decodes to 524,280 bytes of `f32` ([RFC 9639](https://www.rfc-editor.org/rfc/rfc9639.html), sections 8.2 and 9.2.3).  So a 1 MB file can hold about 25 GB of audio.  `engine::decode` grew each channel with no limit.  WebAssembly on wasm32 has 4 GiB at most, and iOS stops the tab at 2 GB (MEM-4).  A very long WAV or MP3 also stops the tab, but its file is then as large as its audio.

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
