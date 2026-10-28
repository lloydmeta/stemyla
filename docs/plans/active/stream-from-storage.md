# Stream a song from browser storage

## Purpose

On an iPhone 17 Pro, a separation of a song longer than about 1.7 minutes stops the tab (MEM-4).  The page and the worker keep whole songs in memory, so memory grows with the length of the song.  After this plan, browser storage (OPFS) holds a song's audio.  That is the encoded source, the decoded song, the mix that the worker separates and the six stems.  Memory holds only blocks, a read-ahead buffer and overviews, so the peak does not grow with the song.  The GPU also keeps the model's weights as f16, which lowers the fixed part by about 350 MB.

Target: a 30-minute song opens, separates, plays and saves on an iPhone 17 Pro, and no step stops the tab.  Its separation takes about 49 to 85 minutes (see "Memory budget").

Decisions 0015 to 0020 (accepted) and 0021 (proposed) record the design choices.  Out of scope:

* f16 arithmetic on the GPU (the `shader-f16` feature).  It would change the stems.
* An AudioWorklet for playback.  The callback stays on the main thread (`engine/src/live.rs`).
* A screen wake lock during a separation.  iOS can suspend a locked page, so M10 runs with Auto-Lock set to Never.

## Context and orientation

### Evidence of the breach (2026-10-25)

The maintainer separated a song of about 5.1 minutes (48 chunks, at most 13.5 M frames) on an iPhone 17 Pro, iOS 26, at `beachape.com/stemyla`.  The build was `4a5b35673150`, from commit `4a5b356`.  The tab reloaded at chunk 2 each time.  The device log (`idevicesyslog`, libimobiledevice) gave:

```
17:32:23.941 kernel: memorystatus: com.apple.WebKit.WebContent [42518] exceeded mem limit: ActiveHard 2048 MB (fatal)
17:32:23.943 kernel: killing_specific_process pid 42518 [com.apple.WebKit.WebContent] (per-process-limit ...) 2097155KB
17:32:23.942 MobileSafari: WebProcessProxy::processDidTerminateOrFailedToLaunch: reason=Crash
```

`runningboardd` sets the limit on each WebContent process: `Memory Limits: active 2048 inactive 2048`.  The JetsamEvent report of an earlier kill gave 2048 MB for the WebContent process.  It gave 63 MB for `com.apple.WebKit.GPU`, while 979 MB of GPU buffers were live.  So the tab pays for its GPU buffers ([WebKit bug 256703](https://bugs.webkit.org/show_bug.cgi?id=256703)), as MEM-4 says.  `docs/references/local-browser-testing.md` gives the commands.

The parts of the tab's memory just before the kill, with one copy of the app:

| Part | MB | Source |
| - | - | - |
| GPU buffers (parameters 699, chunk 280) | 979 | worker log |
| Page WebAssembly memory, after the song decoded | 325 | Web Inspector heap snapshot |
| Worker WebAssembly memory | 246 | worker log |
| egui's WebGL2 context | 88 | heap snapshot |
| WebKit, the JS heap and the rest | about 400 | 2048 less the parts above |

The kill comes when the first segment reaches the page, after chunk 2.  (`Demixer::take_ready` keeps the padded border back.)  `Assembly::place` in `app/src/assemble.rs` then makes all twelve song-length channels at once.  That is 6 stems x 2 channels x 13.5 M frames x 4 bytes, about 650 MB.  The 30-second demo song needs about 63 MB there, and it separates.

### Memory budget

At 44.1 kHz, a stereo `f32` signal takes 44,100 x 60 x 2 x 4 bytes, about 21 MB per minute.  Before the kill, three such copies of the 5.1-minute song held WebAssembly memory, about 110 MB each:

* the decoded song on the page;
* the mix that `load_song` makes on the page (`mix_of_sources`).  It is freed after use, but its pages stay in WebAssembly memory;
* the worker's copy of the mix.

The fixed part is at most the total less these copies: 2048 - 3 x 110, about 1720 MB.  It is less if part of the stems was resident at the kill.  The six stems add 127 MB per minute.  These are estimates from one run.  M2 measures them on a computer, and M10 on the iPhone.

| Design | Fixed | Per minute | Longest song |
| - | - | - | - |
| Now: song, mix and stems in memory | about 1720 MB | about 190 MB | about 1.7 minutes |
| Stems in storage only | about 1720 MB | about 63 MB | about 5.2 minutes |
| All audio in storage | about 1730 MB | none (overviews have a fixed size) | no limit from memory |
| This plan: all audio in storage, f16 weights | about 1380 MB | none (overviews have a fixed size) | no limit from memory |

The fixed part adds a read-ahead buffer (at most about 32 MB, decision 0017).  f16 weights take the parameters from 699 MB to about 350 MB (decision 0018).  The plan leaves about 630 MB of margin under 2048 MB.  The decode worker of M5 adds its own WebAssembly memory; M5 measures it.

A 30-minute song has about 272 chunks.  At the iPhone's measured 10.8 to 18.7 s per chunk, its separation takes about 49 to 85 minutes.

### The code today (commit `4a5b356`)

* `app/src/picker.rs`: `read` copies a whole chosen file into WebAssembly memory.  `app/src/store.rs`: `read` copies a whole stored file.
* `engine/src/decode.rs`: `decode_bytes` takes a whole encoded file and decodes it into memory, up to `MAX_DECODED_BYTES` (decision 0013).
* `engine/src/song.rs`: `Samples` (an `Arc<Vec<f32>>`), `Stem` and `Song`.  Every stem of a song is in memory (MEM-1).
* `engine/src/player.rs`: the `Player` reads every stem at one position, sums, stretches and clamps.  `engine/src/slot.rs` and `engine/src/live.rs`: the output and its callback.
* On the web, cpal calls the callback on the main thread.  It queues two buffers of 3,072 frames (`engine::slot::WEB_BUFFER_FRAMES`, the maintainer's size since the catch-up after M7): about 139 ms, plus the output's latency.  The callback never waits or allocates (RT-1, decision 0007).  When it falls behind, `live.rs` leaves buffers silent (`plays_alone`).
* `engine/src/mixdown.rs`: `Mix` and `StemPcm`, the sources of a save.  `app/src/export.rs`: `ExportJob` reads a source in blocks of 16,384 frames into a sink.  The sinks write JS arrays, so a saved file is whole in the JS heap.
* `app/src/export.rs`: `MixFollower` and `mix_mid` make a song-length mono `mid` for the mix lane.  `app/src/spectrogram.rs`: `lane_signal` makes a song-length signal for each lane.  `classic_drums` runs HPSS (`engine/src/hpss.rs`) on a whole-song mix and adds an in-memory stem.
* `app/src/separator.rs`: `Separator::separate` sends the whole mix as two `Float32Array`s.  It assembles the replies with `Assembly` (`app/src/assemble.rs`).
* `worker/src/web.rs`: `run` copies the mix into WebAssembly memory and feeds `Demixer` (`roformer/src/demix.rs`) a chunk at a time.  It posts each ready segment (`post_segment`).  `mp3` and `flac` encode two whole channels from the page.
* `app/src/store.rs`: song sessions in OPFS.  `save_stems` writes `sessions/<id>/stems/<stem>.wav` as 16-bit WAVs after a separation.  `restore_id` reads a session back into memory.
* `roformer/src/weights.rs`: the model file is f16 (`bs_roformer_sw.f16.safetensors`).  The loader converts each value to f32.  F32 and BF16 files load too.
* `roformer-gpu/src/net.rs`: `Net::load_store` uploads the weights as f32, about 699 MB.  `norm` scales its values at upload.  The band-split `gamma` is scaled and applied on the CPU.  The weight matrices and the biases are not scaled (`stacked` with a scale of 1.0, `tensor`).  The matrix kernels are in `roformer-gpu/src/shaders/`: `matmul.wgsl`, `matmul_vec4.wgsl` and `matmul_interleaved.wgsl`.
* `app/src/lib.rs`: `load_song` builds the lanes, their peaks (`dsp::peaks`) and spectrograms.  `original_lane` and `mix_of_sources` give the mix to separate.

### Terms

* Block: a run of frames read or written at one time.
* Session file: one stereo signal of a song in OPFS: a decoded source, the mix or a stem (decision 0016).
* Read-ahead buffer: memory that holds the next few seconds of every stored lane in front of the play head (decision 0017).
* Underrun: the callback needs frames that the read-ahead buffer does not hold.  It plays silence and counts one underrun for each callback that misses frames.
* Late buffer: a buffer that starts after its time, because the main thread was busy.  It starts at once.  cpal's two chains make late buffers two at a time (a late pair).  `live.rs` fills one of each late pair, and leaves silent the one that would play over it.  This is not an underrun.
* Overview: the peak columns of a lane, made in a pass over its session file, in blocks, so the page needs no samples of the lane to draw its waveform.

## Plan of work

Each milestone is one or more commits on the branch, with fmt, clippy, the tests and `docs-check` before each commit (GIT-2).

Each new OPFS write passes a whole typed array or a Blob (MEM-5).  Each milestone that adds a write path also adds a spy test on that write.  The test fails on a view into a larger buffer.  In the worker, the spy covers `FileSystemSyncAccessHandle.write`.

### How the work runs (M3 to M10)

From M3 on, agents do the work in one session that the maintainer starts.  Each agent reads only what it needs and writes its state to the repo, so a new agent can take over from the files.  The model is `plans/completed/harness-phase-3.md`, "How the work runs".  A "team" is a team leader and its subagents, not Claude Code's agent-teams feature.

All the work goes on the branch `iphone-crashes-2026-10-25`, in PR #39.  There is no other branch or PR.  M0 to M2 are done on it (see Progress).

Before the start, the maintainer:

1. Lets commits be signed with no dialog for the whole run.  Tick "Save in Keychain" in pinentry-mac, or set `default-cache-ttl` and `max-cache-ttl` to 604800 in `~/.gnupg/gpg-agent.conf`, run `gpgconf --reload gpg-agent`, and sign once.  Restore the values after the run.
2. Keeps the Mac awake (`caffeinate -dims` in another terminal), and keeps `autoContinueAtUsageLimit` on.
3. Starts Claude Code at the repo root in auto mode (`claude --permission-mode auto`), and gives the prompt at the end of this section.

The orchestrator, which is the main session:

* First, a preflight.  It stops at once, and tells the maintainer, if one check fails:
  * the branch is `iphone-crashes-2026-10-25`, and `git status --porcelain` is empty;
  * `gh auth status` passes, and PR #39 is open;
  * a signed object can be made with no dialog: `git commit-tree -S HEAD^{tree} -p HEAD -m preflight` (no branch moves);
  * `cargo mutants --version` gives 27.1.0;
  * the model file and the chromedriver are at the paths in the maintainer's local notes (auto-memory);
  * `app/dist` builds (`cd app && trunk build --release`).
* At the start, it asks the maintainer the question of M5's decision point (decision 0019) once, so M5 does not wait.  It records the answer as decision 0019, accepted.
* It reads this plan, `git log`, the CI status, the teams' reports and `docs/reviews/`.  It does not read code or diffs; it asks a subagent for a fact from them.  For each milestone, in order:
  1. It starts a team leader with the milestone's name, this plan's path and the decisions so far.
  2. When the team reports, it starts a new intent reviewer.  The reviewer compares the milestone with this plan's Purpose, the milestone's section and the later milestones.  Real findings go back to the team leader, to the stop rule.
  3. It records the milestone in Progress with the facts the team reported, commits that (signed), and pushes the branch.  It updates PR #39's description with `gh pr edit`.  It waits for the CI run to end before the next team starts.
* The stop rule, for every review loop: stop at the first review with no blocker and no significant should-fix.  Significant means that a milestone would fail as written, two parts contradict each other, a measure would mislead, or the implementer would do wrong work.  Wording and polish are nits.
* It decides a question that this plan does not answer, records it in the Decision log, and lists it in the PR description.  It stops and waits for the maintainer only when:
  * a rule in `docs/invariants.md` would break;
  * a decision point of this plan needs the maintainer: M10's iPhone run (M9's f16 default was decided in advance, see Decision log);
  * a step needs a person: publishing (GIT-1) and a run on the iPhone;
  * auto mode refuses an action (it reports, and does not try the same action another way).
* At the end, with M10 done and CI green, it writes the retrospective and moves this plan to `plans/completed/`.  It fixes the docs that name the old path, commits, pushes, and marks PR #39 ready.  The maintainer reviews and merges (GIT-1).

A team leader, a general-purpose subagent for each milestone:

1. It writes `docs/plans/active/stream-from-storage-m<n>.md` as `plans/PLANS.md` says, with more detail than this plan.  It adds its line to `docs/README.md` in the same commit.  New subagents review it, to the stop rule.  It does not edit this plan (see Decision log).
2. It starts an implementer with the milestone plan's path (or this plan's section), and continues it with SendMessage for fixes.
3. It runs review rounds with `/review-rounds` to the stop rule: at least rounds 1, 2 and 3, and more until a round has no blocker and no significant should-fix.  It fixes each confirmed blocker and significant should-fix with a test.  It does not leave one as debt when a browser test on this Mac can check it.  The base is the commit before the milestone's first commit, and the criteria are the milestone's section.  M3's round 1 passes `--baseline` (the phase 3 gate in `plans/active/harness-engineering.md`).
4. After the last round, it copies `target/review/run/iphone-crashes-2026-10-25/log.md` to `docs/reviews/iphone-crashes-2026-10-25-m<n>.md`, and removes the run folder.  So the next milestone's round 1 starts with no log (TD-37).
5. It writes a Progress line in its milestone plan after each step and each round, and commits it.  It reports in at most 300 words: status, commits, each acceptance check and its result, changes to the plan, and questions.

For every agent in the run:

* Start a subagent with a named `subagent_type`, never as a fork.  Keep at most 15 subagents running at once.
* One team runs at a time, since the teams share one branch and one working tree.  The deepest agents are two layers below the main session.
* Keep the turn open until each background command and each nested agent has finished.
* Commit only signed.  If signing fails, stop and report.
* Do not merge, rebase onto another branch, or make another branch or PR.  Push only `iphone-crashes-2026-10-25`.
* If `review check` fails, stop the round.  A new subagent reads `git reflog`, the stash and the changed files.  If only this branch changed, or only the editor's `vscode-merge-base` line in `.git/config`, the round starts again.  Otherwise the orchestrator stops for the maintainer.
* Browser tests run in Chrome (`--browser chrome`) with the chromedriver of the local notes; Safari's driver refused sessions on 2026-10-25.

The prompt for the maintainer to start the run:

> Run docs/plans/active/stream-from-storage.md as its section "How the work runs (M3 to M10)" says, from M3, on the branch iphone-crashes-2026-10-25 and PR #39.  Do the preflight first.  Then ask me decision 0019's question once, and run the milestones in order as orchestrator.  Stop only for the reasons that section names.

### M0: the plan, the decisions and the evidence

This file, decisions 0015 to 0017 as `proposed`, TD-54 for the breach, and the iPhone commands in `docs/references/local-browser-testing.md`.

* Acceptance: `cargo run -p xtask -- docs-check` passes.

### M1: spike: playback from OPFS on the iPhone

The page gets a query parameter `?streamtest`, beside `?bench` and `?selftest`.  It writes the files of a 30-minute separated song to a scratch folder in OPFS (decision 0016).  These are six 16-bit stems, one 32-bit float original lane and one 16-bit classic-drums stem.  Then it plays them through a read-ahead buffer filled by async reads on the main thread (decision 0017).  It plays at tempo 1 and at `MAX_TEMPO` (1.5).  It logs one line per measure:

* the time of each read of one second of all eight files: p50, p99 and the maximum;
* underruns and late buffers, counted apart, in 5 minutes of playback;
* the same, with a forced 30 ms block of the main thread every 3 seconds.  That is shorter than one buffer (about 43 to 46 ms), cpal's slack at each 'ended' event.  M1 ran with 2,048-frame buffers; since 2026-10-28 a buffer is 3,072 frames, about 70 ms;
* the time from a seek to an unbuffered position to the first frame of the new position: p50 and p99 of 50 seeks;
* whether a read of a file succeeds while a worker writes the same file through a sync access handle, and what it returns.

The scratch folder is removed at the end.  The maintainer publishes the build and runs it on the iPhone (GIT-1).

* Acceptance: a browser test runs `?streamtest` with a short song and checks each log line.  The MEM-5 spy covers the scratch writes.  The iPhone numbers go in "Surprises and discoveries".
* Decision point for the maintainer: accept or change 0016 and 0017 on these numbers.  The proposed bar: no underrun at either tempo, read p99 under 50 ms, seek p99 under 150 ms.  If concurrent reads fail, a stem plays only after its file is complete.

### M2: measure the tab's peak against the song's length

A new e2e scenario separates synthetic songs of 1 and 4 minutes, opened as WAV files.  It records the peak of three parts from the `memory: ...` lines: page WebAssembly memory, worker WebAssembly memory and GPU live bytes.  It prints both peaks and their difference.  It asserts nothing yet.  The difference is the measure that the later milestones drive down.

* Acceptance: `cargo run -p xtask -- e2e memory-budget` prints two peaks and a difference larger than 200 MB on `main`.

### M3: the worker writes the stems to storage

`Request::Separate` names the session.  The worker opens one session file per stem with a sync access handle and appends each ready segment.  On the storage path, the worker sends `Reply::Stored`, which says how many frames are stored, and no `Reply::Segment`.  `Reply::Segment` keeps its samples for the memory path only (M3 plan, D1).  The page stops using `Assembly` for a separation in storage.  The worker writes a 44-byte WAV header with sizes of 0, and sets the RIFF and data sizes after the last segment.  The worker cannot take `engine::wav` (LAYER-1).  The wire format changes with `xtask e2e` in the same commit (PROTO-1).  A stem lane stays empty until its file is complete, unless M1 showed that a file can be read while it is written.

* If OPFS is unavailable, or a write fails for lack of space, the page tells the user (decision 0015).  It uses today's memory path, with `Assembly`.  It refuses a song longer than the length that fits.  That length comes from M2's measure plus about 950 MB for the parts M2 does not measure (as in M10).  Since the catch-up after M7, the length shrinks with the lanes the page holds: 100 s for one source, `min(limit, 7 × limit / lanes)` for more (`app::memory_path_frames`, `app::held_lanes`, decision 0020).
* Acceptance: a browser test separates a short song and reads the stems back from OPFS.  They equal the stems of `main` bit for bit, after 16-bit rounding.  The MEM-5 spy covers the stem writes.  `e2e memory-budget` shows no page growth during the separation.  A browser test makes OPFS writes fail.  It checks the message, a short song on the memory path, and the refusal of a long song.

### M4: the worker reads the mix from storage

When the user presses Separate on the storage path, the page writes the song's mix as a session file, `mix.f32`, if the session has no usable one: one with the song's frames, the `f32le-1` tag and the right size (M4 plan, D1 and D2).  `Request::Separate` names that file.  The memory path still carries two arrays.  `Demixer` takes its input from a source of blocks, not a whole `Vec<f32>`.  It reads one chunk with its overlap at a time.

* Acceptance: `Demixer` tests give the same output from a block source as from a whole mix.  The MEM-5 spy covers the mix write.  `e2e memory-budget` shows no worker growth with the song's length.

### M5: a song decodes from storage into storage

The encoded source is never whole in memory.  A chosen file is saved to OPFS as a `Blob`, as today.  Then a decoder reads it in blocks and writes session files.  On the main thread, OPFS reads are async, and the decoder needs sync reads.  So decoding runs in a worker that reads the `Blob` with `FileReaderSync`.  `stem-worker` cannot take `engine` (LAYER-1), so this is a new worker of the page side.

* Decision 0019 (accepted): a new page-side worker crate that takes `engine` decodes in blocks.
* `engine::decode` and `Song::from_audio` read from a source of blocks and write blocks to a sink, not a `Vec`.  The resampler of decision 0012 runs in blocks.  The overview of each lane is made in a second pass over its session file, in blocks (M5 plan, D5).
* A restored session decodes its stored sources the same way (`store::read` stops reading whole files).  A restore that decodes on the page still reads each file whole, one file at a time, in 1 MiB slices, within the memory path's limit (decision 0020).
* A song of one source decodes into `sessions/<id>/mix.f32`, in M4's format (raw `f32` little-endian, interleaved stereo, 44.1 kHz), and writes M4's `meta` line `mix<TAB><frames><TAB>f32le-1`.  That file is the decoded source and the mix (decision 0016), so Separate finds it and writes no second file.  The original lane of M6 and M7 reads it too.
* A song of several sources gets its `mix.f32` and its `meta` line from the source session files, in blocks, before M7 removes the samples from the page.  Today the page writes it from the samples in memory (M4 plan, D3).
* No step rewrites a session's `mix.f32` while a separation names that session.  Safari does not see a rewrite of the same size in the same second (TD-55).
* The decode worker logs its WebAssembly memory in a `memory: ...` line, as the other two do.
* Acceptance: a test decodes a WAV, a FLAC and an MP3 from a block source.  A memory sink and a block sink get the same samples.  A browser test opens a song and finds its session files and overviews in OPFS.  For a song of one source, it finds `mix.f32` and its `meta` line, and Separate then writes no second mix file.  The MEM-5 spy covers the decode writes.  `e2e memory-budget` shows no growth of the page or the decode worker for a WAV source, from the start of the open to the decode's end line.  After that line, the page still reads the session files back for playback and lanes until M6 and M7 (TD-54), as after M3's separation.

### M6: playback reads storage through the read-ahead buffer

A trait in `engine` gives the frames of a stem from a position.  `Samples` in memory implements it, and so does a storage-backed source.  The `Player` reads through it.  The storage-backed source copies from the read-ahead buffer only, and counts underruns.  An async reader on the main thread keeps the buffer full in front of the play head (decision 0017).  It also fills the frames after the loop start, and starts a read at once after a seek.  The read-ahead buffer is made before playback starts, so the callback never allocates (RT-1).

* Acceptance: engine tests play a storage-backed source with a fake reader that is slow, stalls or fails.  The output equals the memory source, or is silence with the underrun counted.  After a seek, the old position plays until the new block arrives.  The allocation tests in `engine/tests/realtime.rs` cover the new source (RT-1).  A browser test plays a stored song and counts no underruns.
* No failure removes `mix.f32` while the shown song plays from the session or an open reads it.  `forget_mix` waits until then, on `unstored` (M4) and on `decode-unstored` with a kept mix (M5).  For a song of one source, `mix.f32` is the only session file of the source (M6 plan, D10).

### M7: lanes, spectrograms and classic drums from storage

* `load_song` builds a lane from its overview, not from samples.  It no longer makes a song-length mix.
* A spectrogram reads the blocks that it shows.  `lane_signal`, `MixFollower` and `mix_mid` stop making song-length signals.
* Classic drums runs HPSS in blocks with overlap, and writes its stem as a session file.
* After M5, `write_mix` runs only for a song decoded on the page, the fallback when the decode worker cannot decode it.  It reads the song's samples in memory.  M7 makes that fallback write `mix.f32` without a song-length signal on the page, or limits it to the memory path's length.
* Fix TD-56 if M7's open path allows it: a late drop or pick can show over a newer song.
* After every separation on the storage path, the decode worker makes each stem's overview from its WAV, in blocks, as M5 does for the sources.  The request is a new decode message in `stem-protocol` (PROTO-1).  The classic-drums stem gets its overview the same way.  `stem-worker` cannot take `engine::overview` (LAYER-1).
* When the page drops a song's samples, it keeps the song's lists of lane files, which playback reads (M6).
* Before M7, lanes that need more than the read-ahead cap played from memory (`Playback::start`).  Since M7, a stored lane never plays from memory.  With no read-ahead buffer, it plays silence and the status line says so.  A song of more than about 20 sources and no `stem` lines opens wholly in memory, decoded on the page (TD-58).  Since the catch-up after M7, that route keeps the memory path's limit for its lanes, so such a song opens only up to about 25 s.
* A restored session opens its session files without reading them whole.  A session with stem WAVs but no stem overviews (every session before M7) gets them the same way, once.  A session of M3's or M4's layout decodes its sources as in M5.
* Acceptance: the UI snapshot tests pass with a storage-backed song.  An HPSS test gives the same output in blocks as whole, within a stated tolerance.  A browser test restores a session of the old layout and finds the page WebAssembly memory under a fixed bound.

### M8: saving reads from storage and writes to storage

Since M7, `ExportJob` reads its source in blocks from the session files, through `LaneStore` and `BlockReader` (M7 plan, D3).  It stops at the first failed read.  It makes no second read-ahead buffer, because only one may live (M6 plan, D9).  M8 removes the song-length sinks.  Today a WAV save keeps the whole file in the JS heap.  A FLAC or MP3 save builds two song-length `Float32Array`s (`JsChannels::new`, about 635 MB for 30 minutes).  A WAV sink writes an OPFS file, and the page downloads that file.  The worker encodes FLAC and MP3 from the WAV save file in blocks and writes an OPFS file (decision 0021).  The page downloads it.

* Acceptance: the save tests of `app/src/export.rs` and `e2e flac` pass.  The MEM-5 spy covers the WAV, FLAC and MP3 writes.  A browser test saves a 10-minute mix and finds no growth of the JS heap with the length.

### M9: f16 weights on the GPU

The GPU keeps each weight matrix of an f16 file as f16, two values in each `u32` (decision 0018).  Norms and biases stay f32.  A matrix with an odd number of values gets one value of padding.

* Each matrix kernel gets an f16-weight variant beside its f32 kernel.  The variant unpacks the weights with `unpack2x16float` and computes in f32.
* Products of two activations (the attention scores, P·v) keep the f32 kernels.  So does an F32 or BF16 file.
* Each generated variant goes into `generated_sources` and the `want` list of `every_kernel_fits_the_default_limits`.  Each new shader file goes into `SHADER_FILES` or `GENERATED_FROM` (`.claude/rules/gpu.md`).  Calibration measures the f16 variants for the weight products.  `CALIBRATION` in `roformer-gpu/src/bench.rs` goes up by one.
* naga 30.0.1 accepts `unpack2x16float` only with `Capabilities::SHADER_FLOAT16_IN_FLOAT32`.  wgpu grants it from `DownlevelFlags::SHADER_F16_IN_F32`.  Metal has the flag; llvmpipe without `shader-f16` does not.
* The GPU-1 checker adds that capability, with a reason and a test.  An adapter without the flag keeps the f32 path.
* `?bench` times the kernels before and after.  A browser test checks that Safari and Chrome accept the shaders.

* Acceptance: a test runs each changed kernel with f16 weights and with today's f32 weights on the same GPU.  The outputs match bit for bit.  If an adapter flushes f16 subnormals, the test reports how many values differ, and the maintainer decides.  `STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release` and the e2e `selftest` pass against the CPU (GPU-2).  `every_kernel_fits_the_default_limits` passes with the new capability (GPU-1).  A test without the downlevel flag gets the f32 path.  The PR names the adapter of a real GPU, not SwiftShader (GPU-3).  The worker log shows the parameters at about 350 MB.
* Decision point, decided in advance by the maintainer: f16 becomes the default when the Mac's GPU gives the same stems bit for bit and `?bench` shows no slower separation.  M10's iPhone run checks the iPhone later.

### M10: the budget check, and the iPhone evidence

`e2e memory-budget` asserts two things on a computer:

* the 4-minute peak is no more than 4 MB above the 1-minute peak.  Extrapolated to 30 minutes, that is under 40 MB.  At the default chunk the stem worker grows by about 66 MB over its first two chunks, to one chunk's working set, then stays flat (TD-11).  So the check of the stem worker's growth during a separation compares its memory after chunk 2 with its memory at the end, not at the start;
* each peak of page WebAssembly memory, the two workers' WebAssembly memory and GPU live bytes is under a budget.  The budget is the fixed part measured at the default chunk, less the f16 saving, plus the read-ahead cap (`MAX_READ_AHEAD_BYTES`, 32 MiB), the decode worker's measured peak and the MP3 encoder's frames (TD-59), plus 10%.  M2 measured at `chunk=16384` and before the read-ahead buffer and the decode worker existed, so its fixed part is about 300 MB too low.  With f32 weights at the default chunk the peak was about 1,141 MB (the catch-up after M7, E); the f16 saving of about 350 MB brings it to about 790 MB.  An MP3 save of a 30-minute song holds about 141 MB in the stem worker (TD-59), on top of its 123 MB, which brings it to about 930 MB.  The budget is at most 1100 MB.  The decode worker's peak includes an `overviews` and a `classic` request (M7).  The stem worker's memory and the GPU's chunk buffers depend on the chunk length, not on the browser.  `memory-budget` runs at the default chunk (588,800 samples), as the iPhone does, not at the e2e scenarios' `chunk=16384`.  At the default chunk the stem worker peaks at 123 MB, and the GPU holds about 280 MB of one chunk's activations above the parameters (the catch-up after M7, E).  M7's `memory-budget` peak of 828 MB used `chunk=16384`, so it is too low by about 300 MB.

The 1100 MB leaves about 950 MB under the iPhone's limit for WebKit, the JS heap and WebGL.  These parts are not measured.  TD-11 and TD-54 close only when two things are done.  M8 removes the song-length save sinks (done in M8; the save's memory path keeps them for a song of at most 100 s, and TD-59 keeps an MP3's frames).  The page-decode routes refuse a song longer than the memory path allows (TD-58; done in the catch-up after M7).  Else each stays open with what remains.

The maintainer publishes, then opens, separates, plays and saves a 30-minute song on the iPhone.  It saves a stem as WAV, FLAC and MP3, and plays each file from the Files app (TD-60).  The song is at 44.1 or 48 kHz, because the decode limit (decision 0013, 1 GiB of 32-bit stereo) refuses a 30-minute song above about 74.6 kHz, so at 88.2 or 96 kHz.  The tab stays in the foreground, with Auto-Lock set to Never and the phone on a charger.  Playback includes a loop, seeks, and mute and solo changes with the spectrograms shown; the run counts underruns.  The run records the delay from a seek to its first new frame, p50 and p99, with and without a loop (decision 0017).  `idevicesyslog` shows no `exceeded mem limit` for WebContent.

* Acceptance: `cargo run -p xtask -- e2e memory-budget` passes.  The iPhone log and the Web Inspector peaks are in "Outcomes and retrospective".

## Validation and acceptance

* `cargo fmt --all --check`, the clippy commands of `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo run -p xtask -- docs-check`.
* `cargo run -p xtask -- web-test` and `cargo run -p xtask -- e2e memory-budget`.
* On the iPhone 17 Pro: the 30-minute run of M10, with `idevicesyslog` running.

## Idempotence and recovery

* Each milestone keeps the memory path working.  It stays as the fallback when storage is unavailable or full (decision 0015, M3).  A milestone can be reverted alone.
* The stem WAVs keep their path, `sessions/<id>/stems/<stem>.wav`, and are the stem session files (decision 0016).  The decoded sources, the mix and the overviews are new files under `sessions/<id>/`.  M7 says how an older session gets them.
* `?streamtest` writes under one scratch folder and removes it.  A run that stops leaves it.  The next run removes it first.
* If storage is unavailable or full, the page tells the user.  It falls back to the memory path, with the song length that fits (decision 0015).

## Progress

* [x] 2026-10-25: investigation of the crash on the iPhone (see Context).
* [x] 2026-10-25: M0: plan, decisions 0015 to 0017, TD-54, references (commit `4e076dd`).
* [x] 2026-10-25: review round 1 of M0: 7 findings confirmed and fixed, 5 nits fixed.
* [x] 2026-10-25: review round 2: 6 findings confirmed and fixed, 6 nits fixed.
* [x] 2026-10-25: review round 3: 4 findings confirmed and fixed, 5 nits fixed.  The maintainer accepted the amendment of 0018.
* [x] 2026-10-25: M1 code: `?streamtest` (`app/src/streamtest.rs`) and `the_spike_logs_each_measure_and_leaves_no_files`.  In Chrome on the Mac, a 0.2-minute run gave reads of p50 3.4 ms and p99 4.9 ms, and no underruns.
* [x] 2026-10-25: M1 review round 1: 1 blocker and 7 should-fix fixed.  The audio context starts in the tap, the schedule follows cpal's, and the tests check the counts.
* [x] 2026-10-25: M1 review round 2: 7 should-fix fixed.  The timer clock follows cpal's slack of one buffer, and the tests check the read, seek, late and concurrent numbers.
* [x] 2026-10-25: M1 review round 3: 1 blocker and 2 should-fix fixed.  The tests check the p99 text and late buffers without a block, and playback stops when the audio context stops.
* [x] 2026-10-25: M1 iPhone numbers (build `2baf7ff46c5b`, `st_play=60`): see "Surprises and discoveries".  The maintainer accepted decisions 0016 and 0017.
* [x] 2026-10-25: M2: `e2e memory-budget`.  In Chrome on the Mac (AMD Radeon Pro 5500M, `chunk=16384&overlap=1`).  1 minute: 966 MB (page wasm 177, worker wasm 57, GPU live 732).  4 minutes: 1558 MB (727, 99, 732).  The 4-minute peak is 592 MB above, about 197 MB per minute.
* [x] 2026-10-25: run of M3 to M10 started.  Preflight passed.  The maintainer accepted decisions 0015 and 0019, and decided M9's default in advance.
* [x] 2026-10-26: M3: stems to storage (`2c7440a..e2d554c`, plan `stream-from-storage-m3.md`, reviews `docs/reviews/iphone-crashes-2026-10-25-m3.md`).  In Chrome on the Mac (Radeon Pro 5500M), `e2e stored-stems --hashes` passes: the six stem files equal the memory path's stems after 16-bit rounding.  `e2e memory-budget`: the page grows 0 MB during the separation; peaks 966 MB at 1 minute, 1517 MB at 4 minutes.  The MEM-5 spy, the fallback tests and `web-test` (Chrome and Safari) pass.  The memory path refuses a song over 100 s (M3 plan, D3).  Do not publish a build from M3 until M7: after a storage-path separation and on each reload, the page reads the six stems back into memory (TD-54), so a long song can stop the iPhone's tab again and again.
* [x] 2026-10-26: M3 intent review: 1 significant should-fix (this plan's M3 text contradicted D1), fixed here.
* [x] 2026-10-26: M4: mix from storage (`f10fbb7..57bb956`, plan `stream-from-storage-m4.md`, reviews `docs/reviews/iphone-crashes-2026-10-25-m4.md`).  `Demixer` reads a `MixSource` one span at a time, and gives the same output from a block source as from a whole mix.  The page writes `mix.f32` in whole 1 MiB blocks (MEM-5 spy).  M3's six stem hashes are unchanged.  `e2e memory-budget` in Chrome on the Mac: the page and the worker grow 0 MB during a separation; the worker's peak is 57 MB at 1 and 4 minutes (M3: 99 MB at 4 minutes); peaks 966 MB and 1475 MB.  TD-55: Safari does not see a same-size rewrite of the mix in the same second.  In round 3, auto mode refused a reviewer's `sed` edits for a mutation test in its own worktree; the reviewer did not retry, and no finding depends on them.
* [x] 2026-10-26: M5: decode from and into storage (`83dfdde..0c78eba`, plan `stream-from-storage-m5.md`, reviews `docs/reviews/iphone-crashes-2026-10-25-m5.md`).  The new crate `decode-worker` (`decoder/`, decision 0019) decodes and resamples in blocks into session files, and makes the overviews in a second pass.  Block decode equals whole decode bit for bit for WAV, FLAC and MP3.  `e2e decode` (in CI): the session files and overviews equal the native decode.  A song of one source decodes into `mix.f32`, and Separate writes no second mix.  `e2e memory-budget` in Chrome on the Mac: the page and the decode worker grow 0 MB during the open, and the page and the stem worker 0 MB during the separation.  Peaks 972 MB and 1,481 MB; the decode worker 6 MB.  The 509 MB difference is the page's read-back (TD-54).  M3's stem hashes are unchanged.  TD-56: a late drop or pick can show over a newer song (pre-existing).  The orchestrator applied the edits of `CLAUDE.md` and `.claude/rules/` that the crate needs, and wrote the round 2 and 3 findings files, because the Write tool refuses them to a subagent.
* [x] 2026-10-27: M6: playback through the read-ahead buffer (`6be302b..a422b36`, plan `stream-from-storage-m6.md`, reviews `docs/reviews/iphone-crashes-2026-10-25-m6.md`).  The player reads each lane through `StemSource`; stored lanes read from a read-ahead buffer that async reads on the main thread fill (decision 0017).  Engine tests with a slow, stalled or failing reader: the output equals memory bit for bit, or is silence with the underruns counted.  After a seek, the old position plays until its block arrives (engine and browser); with each read delayed 200 ms, seek to sound was about 570 ms.  The RT-1 allocation test plays stored lanes through stall, failed-lane, held-seek, loop and end-of-song phases.  `e2e playback` (in CI): 7 of 7 lanes from storage, 24 MiB read-ahead, 0 underruns, read p99 6.1 ms.  `memory-budget`: peaks 997 MB and 1,505 MB; every growth check 0 MB; M3's stem hashes unchanged.  Three playback bugs from the review rounds (2-1, 3-1, 3-2) are fixed with tests.  CI's `test` job has a 45-minute limit.  TD-57: a moved loop can briefly underrun.
* [x] 2026-10-28: M7: lanes, spectrograms, classic drums and sessions from storage (`c4f3d6a..61af40c`, plan `stream-from-storage-m7.md`, reviews `docs/reviews/iphone-crashes-2026-10-25-m7.md`).  A shown song is lane audio: samples, or a session file and its overview.  The page no longer reads a stored song back (TD-54's read-back).  The decode worker makes stem overviews after each separation and once for an older session, and runs classic drums in blocks (block HPSS equals whole HPSS bit for bit).  Saves read stored lanes in blocks and stop at the first failed read.  Every open has a token, so TD-56 is closed.  `e2e old-session` (new, in CI): page memory 40.3 MB at 1 minute and 40.4 MB at 5 minutes, bound 64 MB; the `c4f3d6a` build fails it (201.7 and 879.0 MB), and so does a build whose worker refuses `overviews`.  `memory-budget`: 828 MB peak at 1 and 4 minutes.  `playback`: 7 of 7 lanes, 0 underruns, read p99 6.3 ms.  UI snapshots pass with a stored song.  Code review ran 8 rounds to the stop rule.  TD-58 now covers every page-decode route, with no length limit; the catch-up team fixes it.
* [x] 2026-10-28: Catch-up after M7 (`edc233d..df2908c`, plan `stream-from-storage-catchup.md`, reviews `docs/reviews/iphone-crashes-2026-10-25-catchup.md`, 7 rounds).  Round 4 of M3 to M6 confirmed 4 should-fix, all fixed; round 5 found none.  19 of 21 deferred should-fix are fixed with native tests; TD-39 keeps the rest.  Every page-decode route refuses a song past the memory path's limit, which shrinks with the lanes held (decision 0020, proposed).  It reads file heads or `meta` first, then one file at a time in 1 MiB slices.  `Kept::MixBusy` is gone: a decode waits for the mix write.  `#[cfg_attr(mutants, mutants::skip)]` replaces `exclude_re` (TD-33).  The stem worker's 170 MB came from the default chunk, not Safari; a double read-back copy and a kept feature buffer went, so it peaks at 123 MB with the stems unchanged.  The browser's output buffer is 3,072 frames.
* [x] 2026-10-29: M8: saving from and to storage (`8c3fcac..30e96c9`, plan `stream-from-storage-m8.md`, reviews `docs/reviews/iphone-crashes-2026-10-25-m8.md`, 3 rounds).  A save writes a 16-bit WAV save file in 1 MiB blocks; the stem worker encodes FLAC and MP3 from it in blocks into a save file; the page offers the file (decision 0021, proposed).  `e2e save` (new, in CI): the JS heap of a save rises 1.35 to 2.23 MB for each format at 1 and 10 minutes; the `c3952ad` build fails it (95.2 MB more for a WAV, 190.5 MB for a FLAC or an MP3 at 10 minutes).  `e2e flac`: a stored encode equals the arrays encode byte for byte.  When storage fails, a song of at most 100 s saves through the JS sinks.  TD-59: an MP3 save holds about 3.3 to 3.5 times its file in the stem worker.  TD-60: the download is checked in Chrome only.  The intent review passed with 3 nits.
* [ ] M9: f16 weights on the GPU.
* [ ] M10: budget asserted, iPhone evidence.

## Surprises and discoveries

* 2026-10-25: Web Inspector's memory timeline showed about 1.06 GB in a run that completed.  It showed about 2.0 GB in a run that iOS killed at 2048 MB.  It does not count all the GPU memory that iOS charges to the tab.  The JetsamEvent report and `idevicesyslog` give the number that counts.
* 2026-10-25: in one killed run, the heap snapshot held two windows with `window.wasmBindings`.  Each had an app instance: 223 MB of WebAssembly memory and a WebGL2 context.  No other Safari tab was open.  A fresh tab held one.  The cause is not known.  A page kept by a reload is likely.  The plan does not act on it.
* 2026-10-25, M1 on the iPhone 17 Pro, iOS 26 (build `2baf7ff46c5b`, four runs of 60 s, 2.86 GB of files):
  * No underruns at tempo 1 or 1.5, with or without a 30 ms block every 3 s.  No late buffers either.
  * Reads of one second of eight files: p50 22 to 23 ms; p99 26 ms with no block, 52 to 59 ms with blocks.
  * Seeks: p50 9 ms, p99 35 ms.  Writes: 2.86 GB in 1.9 to 8.4 s.
  * A file that a worker writes through a sync access handle reads correctly in Safari (26 of 26).  Chrome on the Mac failed 2 of 29 reads with `NotReadableError`, so a stem plays only once its file is complete (M3).
* 2026-10-25: a first iPhone run with Web Inspector's Timeline recording had 11 to 23 late buffers a minute, with the same reads.  The run without the recording had none, so the recording caused them, not storage.  Late buffers come from a busy main thread; read-ahead does not help them.  Safari on the Mac had 0 to 2 a minute, and Chrome none.
* 2026-10-25: chunk times rose from 10.8 s to 18.7 s over 7 chunks of the demo song on the iPhone (0.58 to 0.40 TFLOP/s).  Heat is the likely cause.  It does not concern memory.
* 2026-10-28, the catch-up after M7: the stem worker's 57/133/170 MB in the maintainer's Safari run came from the default chunk (588,800 samples), not from Safari.  Chrome gives the same at that chunk.  The e2e scenarios use `chunk=16384`, so no browser test had measured the chunk the iPhone uses.  At the default chunk the GPU also holds about 979 MB, against about 732 MB at `chunk=16384`.

## Decision log

* 2026-10-25, maintainer: the fix moves song-length audio to storage, not to smaller in-memory formats, because those still break at a longer song.  The target is a 30-minute song.
* 2026-10-25, agent: f16 weights on the GPU stay out of this plan.  The fixed part fits without them, and the maintainer asked about their risk to separation quality.
* 2026-10-25, maintainer: f16 weights on the GPU join this plan as M9, because they save memory (decision 0018).  This replaces the entry above.
* 2026-10-25, agent, after review round 1: the encoded source is read in blocks too (M5).  Classic drums, the mix lane's mid and the spectrogram signals move to blocks (M7).  The budget check (now M10) adds an absolute budget.
* 2026-10-25, maintainer: decisions 0016 and 0017 accepted on the M1 numbers.
* 2026-10-25, maintainer: all the work goes on `iphone-crashes-2026-10-25` and PR #39.  M2 needs no review rounds.  M3 to M10 run with an orchestrator and a team per milestone ("How the work runs").
* 2026-10-25, maintainer: no reload check in M1.  Nothing in this plan acts on its result, and M10's iPhone run shows any kill.
* 2026-10-25, agent, after review round 3: the decode worker's memory joins the checks.  M10 states the device conditions for a run of up to 85 minutes.  A wake lock is out of scope.
* 2026-10-25, agent, after review round 2: the memory path stays as the fallback (M3).  M9 adds f16 kernel variants and the naga capability.  M10's growth tolerance falls to 4 MB.  The spike reads the original lane too.
* 2026-10-25, maintainer, at the start of the M3 to M10 run: decision 0015 accepted.  Teams may reword MEM-1 and LAYER-1 as decisions 0015 and 0019 need, in the milestone that needs it, and keep or extend their tests.  Decision 0019 accepted: a new page-side worker crate decodes.  M9 makes f16 the default when the Mac gives the same stems bit for bit and no slower separation.
* 2026-10-25, maintainer: each milestone works in its own plan file, `plans/active/stream-from-storage-m<n>.md`, M4, M9 and M10 included.  Its Progress, Decision log and surprises go there.  Only the orchestrator edits this plan, with one Progress line for each milestone and the decisions that later milestones need.
* 2026-10-26, agent (M3): on the storage path the worker sends `stored` and `unstored` replies, and `segment` keeps its samples for the memory path only.  The memory path refuses a song over 100 s on every device, because a page cannot read its memory limit (M3 plan, D1 and D3).
* 2026-10-26, agent (M4): the page writes `mix.f32` (raw f32 little-endian, with a `meta` line) when Separate is pressed on the storage path, not when a song opens.  A song never separated costs no storage, and one path covers new songs and M3's sessions.  Every failure of the mix file sends `unstored` (M4 plan, D1 to D7).
* 2026-10-26, agent (orchestrator): M5 measures growth from the start of the open to the decode's end line, as M3 measured it during the separation.  The read-back for playback and lanes after the decode stays until M6 and M7 remove it, and M10 asserts the whole.  The maintainer asked the orchestrator to decide such questions and not to wait for them.
* 2026-10-26, agent (M5): the overviews come from a second pass over each session file, not from the decode pass.  The decode limit of decision 0013 stays.  M6 changes `forget_mix`, and M7 changes `write_mix` (M5 plan, Decision log).
* 2026-10-26, agent (orchestrator): the orchestrator makes the edits of `CLAUDE.md` and `.claude/` that an accepted decision needs (DOC-1), because subagents do not edit them on another agent's request.  It writes the files of `review-rounds` that the Write tool refuses to a subagent.
* 2026-10-26, agent (orchestrator, after M5's intent review): the decode worker makes the stem overviews from the stem WAVs in blocks, after each separation and once for an older session (M7).  It already has `engine::overview` and sync reads, and LAYER-1 keeps `engine` out of `stem-worker`.
* 2026-10-27, agent (M6): stored lane files travel with the stems, and the player no longer holds `Song`.  A hold (silence after a jump while the new block loads) is not an underrun.  MEM-1 is reworded in M7, when the page drops the samples (M6 plan, Decision log).
* 2026-10-27, agent (orchestrator, after M6's intent review, from `engine/src/source.rs` `next_run` and `app/src/playback.rs`): a seek waits for the read in flight and up to 2 blocks, 4 with the loop's start, or 6 past the loop's end.  Decision 0017 is amended.  M10 measures the delay on the iPhone, because M1's reads were not of one block.
* 2026-10-27, agent (orchestrator, after M6's intent review): M10's budget adds the read-ahead cap and the decode worker, which M2 did not measure.  M8 reads the session files in blocks and makes no second read-ahead buffer.
* 2026-10-27, maintainer (during M7): review rounds continue past round 3 until a round has no blocker and no significant should-fix.  A confirmed blocker or significant should-fix that an earlier round skipped gets a fix.  So M7 runs more rounds when it needs them.  After M7, a catch-up team runs a further round on each of M3 to M6, since each round 3 had confirmed fixes that no round checked.  It also fixes the confirmed should-fix that those milestones left as TD-39 or TD-56, or records why a fix cannot be checked.
* 2026-10-28, agent (orchestrator, after M7's intent review): the catch-up team also does these, before M8.  The page-decode routes (`open_on_the_page`, and `rebuild` on a restore) get the memory path's length limit of decision 0015, with a status line; today they decode a whole song on the page when storage is full or the mix is busy (TD-58).  It replaces the `exclude_re` list of wasm32-only functions with `#[cfg_attr(mutants, mutants::skip)]`, which needs no crate (M7 review 5-2).  It finds what takes the stem worker from 57 MB to 170 MB in Safari, and M10's budget uses Safari's figure.
* 2026-10-28, agent (orchestrator): publishing no longer risks TD-54's read-back, since M7's page reads only WAV headers, overviews and `meta` on open, restore and after a separation.  The orchestrator's M7 intent review checked each path (PR #39).  The save sinks (M8) and the page-decode routes (TD-58) still cost memory with the song's length.
* 2026-10-28, maintainer (through the orchestrator): the browser's output buffer is 3,072 frames, for about twice the audio callback's headroom.  The queue of two is about 139 ms.  Mute, solo and tempo changes are heard up to about 139 ms later.  Decision 0017's rough seek bound moves from about 136 ms to about 159 ms, about 9 ms past the 150 ms seek p99 bar.  M10 measures the seek delay on the iPhone.  If it fails the bar, M10's report gives the options (a smaller seek run, or 2,048 frames) and the maintainer decides.
* 2026-10-28, agent (orchestrator, after the catch-up's intent review): the entry above that says M10's budget uses Safari's stem worker figure is replaced.  The figure depends on the chunk length, not the browser (Surprises).  M10's `memory-budget` runs at the default chunk.  Its budget uses the stem worker's 123 MB and the GPU's chunk buffers at that chunk, not M2's fixed part.  The page-decode routes of that entry no longer grow with the song: they keep the memory path's limit (decision 0020).
* 2026-10-28, agent (orchestrator): review rounds stop at the first round with no blocker and no significant should-fix, judged by the orchestrator, not by the reviewer's label.  A rule or decision that fails in a case a user can reach, a contradiction, a misleading measure or later work gone wrong is significant.  An edge case no real song reaches, wording and a weak test of a minor path are nits (the maintainer, 2026-10-28).
* 2026-10-28, agent (orchestrator): VERIFY-1's allowlist of wasm tests that may skip silently gains `a_read_back_is_copied_into_wasm_memory_once`, which skips with no GPU adapter (catch-up E, TD-8).  The rule is unchanged; its list was out of date.
* 2026-10-28, maintainer: decision 0020 is accepted ("20 is fine").
* 2026-10-28, maintainer: the seek bound of about 159 ms with 3,072-frame buffers is accepted ("seek delay is fine").  M10 still measures the seek delay on the iPhone.
* 2026-10-28, maintainer: the third allowlisted skip of VERIFY-1 (the entry above) is accepted ("third test is fine").
* 2026-10-29, agent (orchestrator, after M8's intent review): M10's iPhone run saves a stem in each format, because the download of an OPFS file is checked in Chrome only (TD-60).  M10's budget counts the MP3 encoder's frames in the stem worker (TD-59), about 141 MB for 30 minutes.  The estimate becomes about 930 MB, under the 1100 MB budget.

## Outcomes and retrospective

Not started.
