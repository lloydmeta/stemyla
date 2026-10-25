# Stream a song from browser storage

## Purpose

On an iPhone 17 Pro, a separation of a song longer than about 2 minutes stops the tab (MEM-4).  The page and the worker keep whole songs in memory, so memory grows with the length of the song.  After this plan, browser storage (OPFS) holds a song's audio: the song, the mix that the worker separates and the six stems.  Memory holds only blocks and a read-ahead buffer, so the peak does not grow with the song.

Target: a 30-minute song opens, separates, plays and saves on an iPhone 17 Pro, and no step stops the tab.

Decisions 0015, 0016 and 0017 (all proposed) record the design choices.  Out of scope:

* f16 weights on the GPU.  They lower the fixed part of the memory by about 350 MB, but not the part that grows with the song.  After this plan, the fixed part fits (see "Memory budget").  A later plan can add them for headroom.
* An AudioWorklet for playback.  The callback stays on the main thread (`engine/src/live.rs`).

## Context and orientation

### Evidence of the breach (2026-10-25)

The maintainer separated a 5.2-minute song (48 chunks) on an iPhone 17 Pro, iOS 26, at `beachape.com/stemyla` (build `4a5b35673150`, commit `4a5b356`).  The tab reloaded at chunk 2 each time.  The device log (`idevicesyslog`, libimobiledevice) gave:

```
17:32:23.941 kernel: memorystatus: com.apple.WebKit.WebContent [42518] exceeded mem limit: ActiveHard 2048 MB (fatal)
17:32:23.943 kernel: killing_specific_process pid 42518 [com.apple.WebKit.WebContent] (per-process-limit ...) 2097155KB
17:32:23.942 MobileSafari: WebProcessProxy::processDidTerminateOrFailedToLaunch: reason=Crash
```

`runningboardd` sets the limit on each WebContent process: `Memory Limits: active 2048 inactive 2048`.  The JetsamEvent report of an earlier kill gave 2048 MB for the WebContent process but 63 MB for `com.apple.WebKit.GPU`, while 979 MB of GPU buffers were live.  So the tab pays for its GPU buffers ([WebKit bug 256703](https://bugs.webkit.org/show_bug.cgi?id=256703)), as MEM-4 says.  `docs/references/local-browser-testing.md` gives the commands.

The parts of the tab's memory just before the kill, with one copy of the app:

| Part | MB | Source |
| - | - | - |
| GPU buffers (parameters 699, chunk 280) | 979 | worker log |
| Page WebAssembly memory, after the song decoded | 325 | Web Inspector heap snapshot |
| Worker WebAssembly memory | 246 | worker log |
| egui's WebGL2 context | 88 | heap snapshot |
| WebKit, the JS heap and the rest | about 400 | 2048 less the parts above |

The kill comes when the first segment reaches the page, after chunk 2 (`Demixer::take_ready` keeps the padded border back).  `Assembly::place` in `app/src/assemble.rs` then makes all twelve song-length channels at once: 6 stems x 2 channels x 13.8 M frames x 4 bytes, about 660 MB.  The 30-second demo song needs about 63 MB there, and it separates.

### Memory budget

At 44.1 kHz, a stereo `f32` signal takes 44,100 x 60 x 2 x 4 bytes, about 21 MB per minute.  The fixed part (about 1730 MB) is the total at the kill less the parts that grow with the song.

| Design | Fixed | Per minute | Longest song |
| - | - | - | - |
| Now: song on the page, mix in the worker, six stems on the page | about 1730 MB | about 170 MB | about 1.9 minutes |
| Stems in storage only | about 1730 MB | about 42 MB | about 7.6 minutes |
| This plan: song, mix and stems in storage | about 1600 MB | a few MB (overviews) | no limit from memory |

These numbers come from one run.  M2 measures them on a computer, and M9 on the iPhone.

### The code today (commit `4a5b356`)

* `engine/src/song.rs`: `Samples` (an `Arc<Vec<f32>>`), `Stem` and `Song`.  Every stem of a song is in memory (MEM-1).
* `engine/src/decode.rs`: decodes a whole file into memory, up to `MAX_DECODED_BYTES` (decision 0013).
* `engine/src/player.rs`: the `Player` reads every stem at one position, sums, stretches and clamps.  `engine/src/slot.rs` and `engine/src/live.rs`: the output and its callback.  On the web, cpal calls the callback on the main thread and queues two buffers of 2,048 frames (about 186 ms).  The callback never waits or allocates (RT-1, decision 0007).
* `engine/src/mixdown.rs`: `Mix` and `StemPcm`, the sources of a save.  `app/src/export.rs`: `ExportJob` reads a source in blocks of 16,384 frames into a sink.  The sinks write JS arrays, so a saved file is whole in the JS heap.
* `app/src/separator.rs`: `Separator::separate` sends the whole mix as two `Float32Array`s.  It assembles the replies with `Assembly` (`app/src/assemble.rs`).
* `worker/src/web.rs`: `run` copies the mix into WebAssembly memory, feeds `Demixer` (`roformer/src/demix.rs`) a chunk at a time, and posts each ready segment (`post_segment`).  `mp3` and `flac` encode two whole channels sent by the page.
* `app/src/store.rs`: song sessions in OPFS.  `save_stems` writes the stems as 16-bit WAVs after a separation; `restore_id` reads a session back into memory.
* `app/src/lib.rs`: `load_song` builds the lanes, their peaks (`dsp::peaks`) and spectrograms.  `original_lane` and `mix_of_sources` give the mix to separate.

### Terms

* Block: a run of frames read or written at one time.
* Session file: one channel set of a song in OPFS, such as a stem or the mix, as PCM (decision 0016).
* Read-ahead buffer: memory that holds the next few seconds of every stem in front of the play head (decision 0017).
* Underrun: the callback needs frames that the read-ahead buffer does not hold.  It plays silence and counts the event.
* Overview: the peak columns of a lane, made while its session file is written, so the waveform never reads the whole file.

## Plan of work

Each milestone is one PR or less, with fmt, clippy, the tests and `docs-check` before each commit (GIT-2).

### M0: the plan, the decisions and the evidence

This file, decisions 0015 to 0017 as `proposed`, TD-54 for the breach, and the iPhone commands in `docs/references/local-browser-testing.md`.

* Acceptance: `cargo run -p xtask -- docs-check` passes.

### M1: spike: playback from OPFS on the iPhone

The page gets a query parameter `?streamtest`, beside `?bench` and `?selftest`.  It writes six 30-minute stereo session files (decision 0016) to a scratch folder in OPFS.  Then it plays them through a read-ahead buffer filled by async reads on the main thread (decision 0017).  It logs one line per measure:

* the time of each read of one second of all six stems: p50, p99 and the maximum;
* underruns in 5 minutes of playback, with a forced 100 ms block of the main thread every 3 seconds;
* the time from a seek to an unbuffered position to the first frame of sound: p50 and p99 of 50 seeks;
* whether a read of a file succeeds while the worker writes the same file through a sync access handle, and what it returns.

The scratch folder is removed at the end.  The maintainer publishes the build and runs it on the iPhone (GIT-1).

* Acceptance: a browser test runs `?streamtest` with a short song and checks each log line.  The iPhone numbers go in "Surprises and discoveries".
* Decision point for the maintainer: accept or change 0016 and 0017 on these numbers.  The proposed bar: no underrun, read p99 under 50 ms, seek p99 under 150 ms.  If concurrent reads fail, a stem plays only after its file is complete.

### M2: measure the tab's peak against the song's length

A new e2e scenario separates synthetic songs of 1 and 4 minutes.  It records the peak of page WebAssembly memory + worker WebAssembly memory + GPU live bytes from the `memory: ...` lines.  It prints both peaks and their difference.  It asserts nothing yet: the difference is the measure that the later milestones drive down.

* Acceptance: `cargo run -p xtask -- e2e memory-budget` prints two peaks and a difference larger than 200 MB on `main`.

### M3: the worker writes the stems to storage

`Request::Separate` names the session.  The worker opens one session file per stem with a sync access handle and appends each ready segment.  `Reply::Segment` no longer carries samples.  It says how many frames are stored.  The page stops using `Assembly` for a separation.  The wire format changes with `xtask e2e` in the same commit (PROTO-1).  The page keeps a stem lane empty until its file is complete, unless M1 showed that a file can be read while it is written.

* Acceptance: a browser test separates a short song and reads the stems back from OPFS, bit for bit equal to the stems of `main` after 16-bit rounding.  `e2e memory-budget` shows no page growth during the separation.

### M4: the worker reads the mix from storage

When a song opens, the page writes its mix as a session file.  `Request::Separate` names that file instead of carrying two arrays.  `Demixer` takes its input from a source of blocks, not a whole `Vec<f32>`, and reads one chunk with its overlap at a time.

* Acceptance: `Demixer` tests give the same output from a block source as from a whole mix.  `e2e memory-budget` shows no worker growth with the song's length.

### M5: a song decodes into storage

`engine::decode` and `Song::from_audio` write blocks to a sink instead of growing a `Vec`.  On the web, the sink is a session file.  The resampler of decision 0012 runs in blocks.  The overview of each lane is made in the same pass.

* Acceptance: a test decodes a WAV, a FLAC and an MP3 into a memory sink and a block sink, with the same samples.  A browser test opens a song and finds its session files and overviews in OPFS.

### M6: playback reads storage through the read-ahead buffer

A trait in `engine` gives the frames of a stem from a position: `Samples` in memory implements it, and so does a storage-backed source.  The `Player` reads through it.  The storage-backed source copies from the read-ahead buffer only, and counts underruns.  An async reader on the main thread keeps the buffer full in front of the play head, of the loop start, and after a seek (decision 0017).  The read-ahead buffer is made before playback starts, so the callback never allocates (RT-1).

* Acceptance: engine tests play a storage-backed source with a fake reader that is slow, stalls or fails; the output equals the memory source, or is silence with the underrun counted.  The allocation tests in `engine/tests/realtime.rs` cover the new source (RT-1).  A browser test plays a stored song and counts no underruns.

### M7: lanes, overviews and spectrograms from storage

`load_song` builds a lane from its overview, not from samples.  A spectrogram reads the blocks that it shows.  A restored session (`store::restore_id`) opens its session files without reading them whole.

* Acceptance: the UI snapshot tests pass with a storage-backed song.  A browser test restores a session and finds the page WebAssembly memory under a fixed bound.

### M8: saving reads from storage and writes to storage

`ExportJob` reads its source through the trait of M6.  A WAV sink writes an OPFS file, and the page downloads that file.  The worker encodes FLAC and MP3 from a session file in blocks and writes an OPFS file.  The page downloads it.

* Acceptance: the save tests of `app/src/export.rs` and `e2e flac` pass.  A browser test saves a 10-minute mix and finds no growth of the JS heap with the length.

### M9: the budget check fails growth, and the iPhone evidence

`e2e memory-budget` asserts that the 4-minute peak is no more than 32 MB above the 1-minute peak.  TD-11 and TD-54 close.  The maintainer publishes, then opens, separates, plays (with a loop and seeks) and saves a 30-minute song on the iPhone.  `idevicesyslog` shows no `exceeded mem limit` for WebContent.

* Acceptance: `cargo run -p xtask -- e2e memory-budget` passes.  The iPhone log and the Web Inspector peaks are in "Outcomes and retrospective".

## Validation and acceptance

* `cargo fmt --all --check`, the clippy commands of `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo run -p xtask -- docs-check`.
* `cargo run -p xtask -- web-test` and `cargo run -p xtask -- e2e memory-budget`.
* On the iPhone 17 Pro: the 30-minute run of M9, with `idevicesyslog` running.

## Idempotence and recovery

* Each milestone keeps the memory path working until the storage path replaces it.  A milestone can be reverted alone.
* Session files are new names under `sessions/<id>/`.  A session of an older build has no session files.  The page opens it from its stored source files, as today.
* `?streamtest` writes under one scratch folder and removes it.  A run that stops leaves it.  The next run removes it first.
* If storage is unavailable or full, the page tells the user and falls back to the memory path, with the song length that fits (decision 0015).

## Progress

* [x] 2026-10-25: investigation of the crash on the iPhone (see Context).
* [ ] M0: plan, decisions 0015 to 0017, TD-54, references.
* [ ] M1: spike on the iPhone.
* [ ] M2: `e2e memory-budget` measures.
* [ ] M3: stems to storage.
* [ ] M4: mix from storage.
* [ ] M5: decode into storage.
* [ ] M6: playback through the read-ahead buffer.
* [ ] M7: lanes and sessions from storage.
* [ ] M8: saving from and to storage.
* [ ] M9: budget asserted, iPhone evidence.

## Surprises and discoveries

* 2026-10-25: Web Inspector's memory timeline showed about 1.06 GB in a run that completed and about 2.0 GB in a run that was killed at 2048 MB.  It does not count all the GPU memory that iOS charges to the tab.  The JetsamEvent report and `idevicesyslog` give the number that counts.
* 2026-10-25: in one killed run, the heap snapshot held two windows with `window.wasmBindings`, each with an app instance (223 MB of WebAssembly memory and a WebGL2 context).  No other Safari tab was open.  A fresh tab held one.  The cause is not known.  A page kept by a reload is likely.  M1 reloads the page and counts the instances again.
* 2026-10-25: chunk times rose from 10.8 s to 18.7 s over 7 chunks of the demo song on the iPhone (0.58 to 0.40 TFLOP/s).  Heat is the likely cause.  It does not concern memory.

## Decision log

* 2026-10-25, maintainer: the fix moves song-length audio to storage, not to smaller in-memory formats, because those still break at a longer song.  The target is a 30-minute song.
* 2026-10-25, agent: f16 weights on the GPU stay out of this plan.  The fixed part fits without them, and the maintainer asked about their risk to separation quality.

## Outcomes and retrospective

Not started.
