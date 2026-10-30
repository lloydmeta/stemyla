# 0017: the main thread reads session files ahead of the play head

Status: accepted (2026-10-25, by the maintainer)

## Context

On the web, cpal calls the audio callback on the page's main thread.  It queues two buffers of 2,048 frames: about 93 ms, plus the output's latency (`engine/src/live.rs`).  The callback never waits or allocates (RT-1).  Decision 0015 moves stems to OPFS session files.  On the main thread, OPFS reads are async: sync access handles exist only in dedicated workers ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemFileHandle/createSyncAccessHandle)).

## Options

* Async reads on the main thread fill a read-ahead buffer in memory.  The callback copies from the buffer only.  Nothing new is needed from the host.
* A worker reads with a sync access handle into a `SharedArrayBuffer` ring.  This needs cross-origin isolation (COOP and COEP headers).  GitHub Pages cannot set headers ([GitHub discussion 13309](https://github.com/orgs/community/discussions/13309)), so it needs a service-worker shim.
* A worker reads and transfers blocks to the page with `postMessage`.  Each block crosses a message queue that the main thread also serves.

## Decision

Async reads on the main thread fill a read-ahead buffer.  The buffer holds a few seconds of every stem in front of the play head, and the frames after the loop start.  A seek starts a read at once.  The old audio plays until the new block arrives.  If the callback needs frames that the buffer does not hold, it plays silence and counts an underrun.  M1 of `plans/completed/stream-from-storage.md` measured the reads on an iPhone 17 Pro on 2026-10-25.  Over four runs of 60 s, at tempo 1 and 1.5, there were no underruns and no late buffers.  The read p99 was 26 ms, and up to 59 ms with a 30 ms block every 3 s.  The seek p99 was 35 ms.

## Consequences

* Ten seconds of eight lanes is about 16 MB: six 16-bit stems, the 32-bit float original and a 16-bit classic-drums stem.  The frames after the loop start add at most the same again, so the buffer is at most about 32 MB.  The buffer is made before playback starts (RT-1).
* A main-thread block longer than the queued audio (about 93 ms) is still a gap, as today.  `live.rs` leaves that buffer silent.  Storage adds no new cause of a gap while reads finish within the read-ahead buffer.
* A seek to an unbuffered position starts after its block reads.  M1 measures that delay.
* Moving the callback to an AudioWorklet stays possible.  It would then read from the same buffer.

Amended on 2026-10-27 by the agent, after M6: a seek to an unbuffered position waits for the read in flight, then for the blocks of the target's run (32,768 frames each).  That is up to 2 blocks, up to 4 when the run needs the loop's start, and up to 6 for a target after the loop's end.  The page sees a seek at the next callback (about 46 ms) and polls every 10 ms.  M1 measured reads of one second of eight files, not of one block, so it gives no seek delay for this design.  As a rough bound, since a block holds less than one second of audio, a seek without a loop takes 3 reads at M1's p99 (about 80 ms) plus up to 56 ms, near M1's bar of 150 ms.  A seek with a loop can pass it.  The old position plays during the wait, so the wait is a delay, not a gap.  M10 measures the seek delay on the iPhone.  The decision is unchanged.

Note of 2026-10-28, by the agent: the maintainer chose a browser output buffer of 3,072 frames on 2026-10-28 (D6 of `plans/completed/stream-from-storage-catchup.md`).  `engine::slot::WEB_BUFFER_FRAMES` holds the size, and `engine/src/live.rs` asks cpal for it.  The text above gives the old figures of 2,048 frames.  At 44.1 kHz, one buffer is now about 69.7 ms, and the queue of two is about 139 ms, plus the output's latency.  A main-thread block longer than about 139 ms is a gap.  The page sees a seek in at most about 70 ms (mean about 35 ms), not 46 ms.  Mute, solo and tempo changes are heard up to about 139 ms later.  The rough seek bound above moves from about 136 ms to about 159 ms (136 + 69.7 - 46.4).  That is about 9 ms past the 150 ms seek p99 bar that M10 measures against.  The decision is unchanged.
