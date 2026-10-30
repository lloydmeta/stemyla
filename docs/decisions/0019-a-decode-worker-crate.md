# 0019: a page-side worker crate decodes a song in blocks

Status: accepted (2026-10-25, by the maintainer)

## Context

Decision 0015 keeps the encoded source out of memory: a decoder reads it in blocks and writes session files.  Symphonia reads from a sync source (`Read` and `Seek`).  On the main thread, OPFS and `Blob` reads are async.  Sync reads exist only in workers: `FileReaderSync` ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileReaderSync)) and `FileSystemSyncAccessHandle` ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemSyncAccessHandle)).  `stem-worker` cannot take `engine`, because `app` and `stem-worker` share only `stem-protocol` (LAYER-1).

## Options

* A new page-side worker crate in the Shells layer that takes `engine`.  Its WebAssembly holds only the decoder, the resampler and the overviews.
* The app's own WebAssembly, started a second time in a worker with a decode entry point.  No new crate, but the worker links egui and the whole app.
* Decode in `stem-worker`.  Decode and resampling move out of `engine` into a lower crate that `app` and `stem-worker` both take.  This breaks LAYER-1, and decode waits for the model and for separations.
* Decode on the main thread with async reads.  Symphonia has no push API, so the decoder must stop and start at each block.  Decoding a long song also blocks the audio callback's thread.

## Decision

The first option.  A new crate, a page-side worker, takes `engine`.  It reads the stored `Blob` or OPFS file in blocks, decodes and resamples it, and writes the session files and the overviews.

## Consequences

* The layers table in `docs/ARCHITECTURE.md` gets a row for the crate.  LAYER-1's wording and its tests cover a third WebAssembly module, which is on the page side.
* The page and the decode worker exchange messages.  Their encoders and decoders follow PROTO-1.
* The decode worker logs its WebAssembly memory in a `memory: ...` line.  M10's budget includes it.
* Trunk builds a third WebAssembly file, so the page downloads more code once.
