# 0021: saves go through browser storage, and an MP3 is encoded from 16 bits

Status: accepted (2026-10-29, by the maintainer)

## Context

Until M8 of `plans/active/stream-from-storage.md`, a save kept its whole file in the JS heap of the page.  A WAV save made one `Uint8Array` of the file (`JsWav`).  A FLAC or MP3 save made two song-length `Float32Array`s (`JsChannels`), about 635 MB for 30 minutes.  The page transferred them to the stem worker.  The worker kept the encoded file until its reply.  iOS stops a tab at 2 GB (MEM-4), and the JS heap counts.  Decision 0015 moves the song, the mix and the stems to browser storage, with the memory path as the fallback.  M8 does the same for saves.

## Options

* Keep the JS sinks.  The JS heap of a save grows with the song.
* Write each format's input to browser storage as `f32`, for the worker to encode.  A FLAC or MP3 save then writes twice the bytes of a 16-bit WAV: 635 MB against 318 MB for 30 minutes.  The page also has two write paths.
* Write every save as a 16-bit WAV save file first.  The WAV is the download of a WAV save, and the input of a FLAC or MP3 save.  The worker reads it in blocks, encodes it, and writes its own save file in blocks.

## Decision

The third option (D1 to D8 of `plans/active/stream-from-storage-m8.md`).

* The page writes every save as a 16-bit WAV save file, `saves/<page>-<n>.wav` (`app::export::StoredWav`).  It writes blocks of 1 MiB through one reused JS array, with at most one write in flight.  `<page>` is the page's start time in milliseconds, and `<n>` counts the page's saves.
* For a FLAC or MP3 save, the stem worker reads the WAV in blocks and encodes it.  It writes `saves/<page>-<n>.flac` or `.mp3` through a sync access handle.  It removes the WAV at the end, also when the encode fails.  The messages name the files (`stem_protocol::Encode::Stored`, `Encoded::Stored`, `Encoded::Failed`), and no samples cross.
* The page offers a save file as a `Blob` of its `File`, of the format's type.  So no bytes enter the JS heap or WebAssembly memory.
* The lifetime of save files: the page removes an offered file 10 minutes after the offer.  It removes the file of a failed or dropped save at once.  At its start, it removes each file older than one hour.  Reset removes the folder with the rest.
* An MP3 is encoded from the WAV's 16-bit samples, not from the 32-bit mix.  A FLAC was 16-bit already, so it stays bit for bit the same.
* When browser storage fails, a song of at most `app::MEMORY_PATH_MAX_FRAMES` frames saves through the JS sinks, as before (decision 0015).  A longer song does not save, and the status line says why.  This flat limit is an exception to decision 0020.  The JS sinks are in the JS heap, and a song on the memory path is already within 0020's limit (D8 of the M8 plan).

## Consequences

* The JS heap of a save holds one block of 1 MiB, whatever the song's length.  `xtask e2e save` measures it.
* A FLAC save holds no file in the worker.  An MP3 save still holds about 3.5 times its file in the stem worker's WebAssembly memory (TD-59).  `rusty_mp3` keeps its frames until `finish()`.
* Take an MP3 of a mix in which no sample passes full scale.  It differs from the MP3 of the 32-bit mix by the 16-bit rounding, about -96 dB.  That is far under the noise of a 192 kbps MP3.
* A save needs free browser storage for its WAV.  A FLAC or MP3 save also needs room for its output beside the WAV.  A save of 30 minutes writes about 318 MB.
* A download reads its file after the click, and the page cannot know when it ends.  A download of a long file that starts to read after more than 10 minutes can fail.
* The download of an OPFS `File` through an object URL is checked in Chrome only (TD-60).  The team asked the orchestrator for a save of each format in M10's iPhone run.

## Note of 2026-10-29 (proposed, for a person)

Status of this note: proposed.  The text above is the accepted decision; this note does not change it until a person accepts the note.

* Since commit `f28ebff` (review round speed 3 of `plans/active/stream-from-storage-m8.md`), the page writes blocks of 2 MiB, not 1 MiB.  When each frame of the page holds the main thread for long, as CI's paints in software do, a write takes about two frames, whatever its size up to 8 MiB.  So fewer writes make a save faster.  4 MiB made D10's measure of the JS heap vary by about 4 MB, which is its bound.
* Since review round speed 3, a save's reader also reads ahead.  So the JS heap holds the bytes of the reads in flight until each is copied into WebAssembly memory.  The JS heap of a save holds the write's array of 2 MiB and the browser's copy of the write in flight.  It also holds the reads in flight: 2 reads of 65,536 frames of each lane since review round speed 4.  In `e2e save` (3 lanes), the JS heap measure of D10 of the M8 plan rose by 2.4 to 4.5 MB during a save, of 1 minute or 10.  That measure is the largest reading at a write or at the end of the save, after a garbage collection, less the reading before.  It does not grow with the song.  `docs/ARCHITECTURE.md` (memory on phones) gives the whole memory of a save.
