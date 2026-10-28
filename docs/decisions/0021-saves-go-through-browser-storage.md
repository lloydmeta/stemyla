# 0021: saves go through browser storage, and an MP3 is encoded from 16 bits

Status: proposed (2026-10-29, by the agent)

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
* When browser storage fails, a song of at most `app::MEMORY_PATH_MAX_FRAMES` frames saves through the JS sinks, as before (decision 0015).  A longer song does not save, and the status line says why.

## Consequences

* The JS heap of a save holds one block of 1 MiB, whatever the song's length.  `xtask e2e save` measures it.
* A FLAC save holds no file in the worker.  An MP3 save still holds about 3.5 times its file in the stem worker's WebAssembly memory (TD-59).  `rusty_mp3` keeps its frames until `finish()`.
* Take an MP3 of a mix in which no sample passes full scale.  It differs from the MP3 of the 32-bit mix by the 16-bit rounding, about -96 dB.  That is far under the noise of a 192 kbps MP3.
* A save needs free browser storage for its WAV.  A FLAC or MP3 save also needs room for its output beside the WAV.  A save of 30 minutes writes about 318 MB.
* A download reads its file after the click, and the page cannot know when it ends.  A download of a long file that starts to read after more than 10 minutes can fail.
* The download of an OPFS `File` through an object URL is checked in Chrome only (TD-60).  The team asked the orchestrator for a save of each format in M10's iPhone run.
