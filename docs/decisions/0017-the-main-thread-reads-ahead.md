# 0017: the main thread reads session files ahead of the play head

Status: proposed (2026-10-25, by the agent)

## Context

On the web, cpal calls the audio callback on the page's main thread.  It queues two buffers of 2,048 frames: about 93 ms, plus the output's latency (`engine/src/live.rs`).  The callback never waits or allocates (RT-1).  Decision 0015 moves stems to OPFS session files.  On the main thread, OPFS reads are async: sync access handles exist only in dedicated workers ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemFileHandle/createSyncAccessHandle)).

## Options

* Async reads on the main thread fill a read-ahead buffer in memory.  The callback copies from the buffer only.  Nothing new is needed from the host.
* A worker reads with a sync access handle into a `SharedArrayBuffer` ring.  This needs cross-origin isolation (COOP and COEP headers).  GitHub Pages cannot set headers ([GitHub discussion 13309](https://github.com/orgs/community/discussions/13309)), so it needs a service-worker shim.
* A worker reads and transfers blocks to the page with `postMessage`.  Each block crosses a message queue that the main thread also serves.

## Decision

Async reads on the main thread fill a read-ahead buffer (proposed).  The buffer holds a few seconds of every stem in front of the play head, and the frames after the loop start.  A seek starts a read at once.  The old audio plays until the new block arrives.  If the callback needs frames that the buffer does not hold, it plays silence and counts an underrun.  M1 of `plans/active/stream-from-storage.md` measures the reads on an iPhone before this decision is accepted.

## Consequences

* Ten seconds of eight lanes is about 16 MB: six 16-bit stems, the 32-bit float original and a 16-bit classic-drums stem.  The frames after the loop start add at most the same again, so the buffer is at most about 32 MB.  The buffer is made before playback starts (RT-1).
* A main-thread block longer than the queued audio (about 93 ms) is still a gap, as today.  `live.rs` leaves that buffer silent.  Storage adds no new cause of a gap while reads finish within the read-ahead buffer.
* A seek to an unbuffered position starts after one read.  M1 measures that delay.
* Moving the callback to an AudioWorklet stays possible.  It would then read from the same buffer.
