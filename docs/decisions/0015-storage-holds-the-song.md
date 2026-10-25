# 0015: browser storage holds a song's audio, and memory holds blocks

Status: proposed (2026-10-25, by the agent)

## Context

On an iPhone 17 Pro, iOS stops a tab at 2048 MB, GPU buffers included (MEM-4).  The page keeps the decoded song and six stems in memory, and the worker keeps a copy of the mix.  Together they grow by about 170 MB per minute of song.  A 5.2-minute song stopped the tab at chunk 2 on 2026-10-25 (`plans/active/stream-from-storage.md`, Context).  The fixed part is about 1730 MB, so the longest song that fits is about 1.9 minutes.

## Options

* Smaller formats in memory: 16-bit stems, f16 weights.  The longest song becomes about 6 minutes.  Each still grows with the song.
* Free the model from the GPU after a separation, and keep the stems in memory for playback.  Playback then fits about 9 minutes of `f32` stems.  The separation itself still grows.
* Storage holds the song, the mix and the stems.  Memory holds blocks, a read-ahead buffer and overviews.  The peak does not grow with the song.

## Decision

Storage holds a song's audio (proposed).  The song decodes into session files in OPFS.  The worker reads the mix from a session file and writes each stem to a session file.  Playback, lanes and saves read session files in blocks.  If storage is unavailable or full, the page uses the memory path and refuses a song longer than fits.

## Consequences

* MEM-1 changes: stem audio is a session file read in blocks, or `engine::Samples` on the memory path.  `invariants.md` changes with the milestone that makes the change.
* Every reader of samples goes through one block source: the player, the save jobs, the lanes and the spectrograms.
* A 30-minute song takes about 1.9 GB of storage for its stems at 16-bit (decision 0016), plus its source file.  Safari 17 and later gives an origin up to 60% of the disk ([WebKit](https://webkit.org/?p=14403)).  Storage that the browser evicts costs a new separation.
* Playback depends on reads that finish in time (decision 0017).
* The decode limit of decision 0013 protected memory.  On the storage path it protects storage.  A later decision can change it.
