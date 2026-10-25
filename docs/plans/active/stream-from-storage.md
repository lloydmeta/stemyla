# Stream a song from browser storage

## Purpose

On an iPhone 17 Pro, a separation of a song longer than about 1.7 minutes stops the tab (MEM-4).  The page and the worker keep whole songs in memory, so memory grows with the length of the song.  After this plan, browser storage (OPFS) holds a song's audio.  That is the encoded source, the decoded song, the mix that the worker separates and the six stems.  Memory holds only blocks, a read-ahead buffer and overviews, so the peak does not grow with the song.  The GPU also keeps the model's weights as f16, which lowers the fixed part by about 350 MB.

Target: a 30-minute song opens, separates, plays and saves on an iPhone 17 Pro, and no step stops the tab.  Its separation takes about 49 to 85 minutes (see "Memory budget").

Decision 0015 (proposed) and decisions 0016, 0017 and 0018 (accepted) record the design choices.  Out of scope:

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
* On the web, cpal calls the callback on the main thread.  It queues two buffers of 2,048 frames: about 93 ms, plus the output's latency.  The callback never waits or allocates (RT-1, decision 0007).  When it falls behind, `live.rs` leaves buffers silent (`plays_alone`).
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
* Read-ahead buffer: memory that holds the next few seconds of every stem in front of the play head (decision 0017).
* Underrun: the callback needs frames that the read-ahead buffer does not hold.  It plays silence and counts the event.
* Late buffer: a buffer that starts after its time, because the main thread was busy.  It starts at once.  cpal's two chains make late buffers two at a time (a late pair).  `live.rs` fills one of each late pair, and leaves silent the one that would play over it.  This is not an underrun.
* Overview: the peak columns of a lane, made while its session file is written, so the waveform never reads the whole file.

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
  2. When the team reports, it starts a new intent reviewer.  The reviewer compares the milestone with this plan's Purpose, the milestone's section and the later milestones.  Real findings go back to the team leader, for at most 3 reviews, to the stop rule.
  3. It records the milestone in Progress with the facts the team reported, commits that (signed), and pushes the branch.  It updates PR #39's description with `gh pr edit`.  It waits for the CI run to end before the next team starts.
* The stop rule, for every review loop: stop at the first review with no blocker and no significant should-fix.  Significant means that a milestone would fail as written, two parts contradict each other, a measure would mislead, or the implementer would do wrong work.  Wording and polish are nits.
* It decides a question that this plan does not answer, records it in the Decision log, and lists it in the PR description.  It stops and waits for the maintainer only when:
  * a rule in `docs/invariants.md` would break;
  * a decision point of this plan needs the maintainer: M9's f16 default, and M10's iPhone run;
  * a step needs a person: publishing (GIT-1) and a run on the iPhone;
  * auto mode refuses an action (it reports, and does not try the same action another way).
* At the end, with M10 done and CI green, it writes the retrospective and moves this plan to `plans/completed/`.  It fixes the docs that name the old path, commits, pushes, and marks PR #39 ready.  The maintainer reviews and merges (GIT-1).

A team leader, a general-purpose subagent for each milestone:

1. For M3, M5, M6, M7 and M8, it writes `docs/plans/active/stream-from-storage-m<n>.md` as `plans/PLANS.md` says, with more detail than this plan.  It adds its line to `docs/README.md` in the same commit.  New subagents review it, to the stop rule, for at most 3 reviews.  M4, M9 and M10 work from their sections here.
2. It starts an implementer with the milestone plan's path (or this plan's section), and continues it with SendMessage for fixes.
3. It runs three review rounds with `/review-rounds`.  The base is the commit before the milestone's first commit, and the criteria are the milestone's section.  M3's round 1 passes `--baseline` (the phase 3 gate in `plans/active/harness-engineering.md`).
4. After round 3, it copies `target/review/run/iphone-crashes-2026-10-25/log.md` to `docs/reviews/iphone-crashes-2026-10-25-m<n>.md`, and removes the run folder.  So the next milestone's round 1 starts with no log (TD-37).
5. It writes a Progress line after each step and each round, and commits it.  It reports in at most 300 words: status, commits, each acceptance check and its result, changes to the plan, and questions.

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
* the same, with a forced 30 ms block of the main thread every 3 seconds.  That is shorter than one buffer (about 43 to 46 ms), cpal's slack at each 'ended' event;
* the time from a seek to an unbuffered position to the first frame of the new position: p50 and p99 of 50 seeks;
* whether a read of a file succeeds while a worker writes the same file through a sync access handle, and what it returns.

The scratch folder is removed at the end.  The maintainer publishes the build and runs it on the iPhone (GIT-1).

* Acceptance: a browser test runs `?streamtest` with a short song and checks each log line.  The MEM-5 spy covers the scratch writes.  The iPhone numbers go in "Surprises and discoveries".
* Decision point for the maintainer: accept or change 0016 and 0017 on these numbers.  The proposed bar: no underrun at either tempo, read p99 under 50 ms, seek p99 under 150 ms.  If concurrent reads fail, a stem plays only after its file is complete.

### M2: measure the tab's peak against the song's length

A new e2e scenario separates synthetic songs of 1 and 4 minutes, opened as WAV files.  It records the peak of three parts from the `memory: ...` lines: page WebAssembly memory, worker WebAssembly memory and GPU live bytes.  It prints both peaks and their difference.  It asserts nothing yet.  The difference is the measure that the later milestones drive down.

* Acceptance: `cargo run -p xtask -- e2e memory-budget` prints two peaks and a difference larger than 200 MB on `main`.

### M3: the worker writes the stems to storage

`Request::Separate` names the session.  The worker opens one session file per stem with a sync access handle and appends each ready segment.  `Reply::Segment` no longer carries samples.  It says how many frames are stored.  The page stops using `Assembly` for a separation in storage.  The worker writes a 44-byte WAV header with sizes of 0, and sets the RIFF and data sizes after the last segment.  The worker cannot take `engine::wav` (LAYER-1).  The wire format changes with `xtask e2e` in the same commit (PROTO-1).  A stem lane stays empty until its file is complete, unless M1 showed that a file can be read while it is written.

* If OPFS is unavailable, or a write fails for lack of space, the page tells the user (decision 0015).  It uses today's memory path, with `Assembly`.  It refuses a song longer than the length that fits.  That length comes from M2's measure plus about 950 MB for the parts M2 does not measure (as in M10).
* Acceptance: a browser test separates a short song and reads the stems back from OPFS.  They equal the stems of `main` bit for bit, after 16-bit rounding.  The MEM-5 spy covers the stem writes.  `e2e memory-budget` shows no page growth during the separation.  A browser test makes OPFS writes fail.  It checks the message, a short song on the memory path, and the refusal of a long song.

### M4: the worker reads the mix from storage

When a song opens, the page writes its mix as a session file.  `Request::Separate` names that file instead of carrying two arrays.  `Demixer` takes its input from a source of blocks, not a whole `Vec<f32>`.  It reads one chunk with its overlap at a time.

* Acceptance: `Demixer` tests give the same output from a block source as from a whole mix.  The MEM-5 spy covers the mix write.  `e2e memory-budget` shows no worker growth with the song's length.

### M5: a song decodes from storage into storage

The encoded source is never whole in memory.  A chosen file is saved to OPFS as a `Blob`, as today.  Then a decoder reads it in blocks and writes session files.  On the main thread, OPFS reads are async, and the decoder needs sync reads.  So decoding runs in a worker that reads the `Blob` with `FileReaderSync`.  `stem-worker` cannot take `engine` (LAYER-1), so this is a new worker of the page side.

* Decision point for the maintainer: a new page-side worker crate that takes `engine`, or another way to decode in blocks.  The choice gets decision 0019.
* `engine::decode` and `Song::from_audio` read from a source of blocks and write blocks to a sink, not a `Vec`.  The resampler of decision 0012 runs in blocks.  The overview of each lane is made in the same pass.
* A restored session decodes its stored sources the same way (`store::read` stops reading whole files).
* The decode worker logs its WebAssembly memory in a `memory: ...` line, as the other two do.
* Acceptance: a test decodes a WAV, a FLAC and an MP3 from a block source.  A memory sink and a block sink get the same samples.  A browser test opens a song and finds its session files and overviews in OPFS.  The MEM-5 spy covers the decode writes.  `e2e memory-budget` shows no growth of the page or the decode worker for a WAV source.

### M6: playback reads storage through the read-ahead buffer

A trait in `engine` gives the frames of a stem from a position.  `Samples` in memory implements it, and so does a storage-backed source.  The `Player` reads through it.  The storage-backed source copies from the read-ahead buffer only, and counts underruns.  An async reader on the main thread keeps the buffer full in front of the play head (decision 0017).  It also fills the frames after the loop start, and starts a read at once after a seek.  The read-ahead buffer is made before playback starts, so the callback never allocates (RT-1).

* Acceptance: engine tests play a storage-backed source with a fake reader that is slow, stalls or fails.  The output equals the memory source, or is silence with the underrun counted.  After a seek, the old position plays until the new block arrives.  The allocation tests in `engine/tests/realtime.rs` cover the new source (RT-1).  A browser test plays a stored song and counts no underruns.

### M7: lanes, spectrograms and classic drums from storage

* `load_song` builds a lane from its overview, not from samples.  It no longer makes a song-length mix.
* A spectrogram reads the blocks that it shows.  `lane_signal`, `MixFollower` and `mix_mid` stop making song-length signals.
* Classic drums runs HPSS in blocks with overlap, and writes its stem as a session file.
* A restored session opens its session files without reading them whole.  An older session has stem WAVs but no overviews.  The page makes its overviews from the WAVs in blocks, once, and decodes its sources as in M5.
* Acceptance: the UI snapshot tests pass with a storage-backed song.  An HPSS test gives the same output in blocks as whole, within a stated tolerance.  A browser test restores a session of the old layout and finds the page WebAssembly memory under a fixed bound.

### M8: saving reads from storage and writes to storage

`ExportJob` reads its source through the trait of M6.  A WAV sink writes an OPFS file, and the page downloads that file.  The worker encodes FLAC and MP3 from a session file in blocks and writes an OPFS file.  The page downloads it.

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
* Decision point for the maintainer: make f16 the default after the iPhone and the Mac show the same stems and no slower separation.

### M10: the budget check, and the iPhone evidence

`e2e memory-budget` asserts two things on a computer:

* the 4-minute peak is no more than 4 MB above the 1-minute peak.  Extrapolated to 30 minutes, that is under 40 MB;
* each peak of page WebAssembly memory, the two workers' WebAssembly memory and GPU live bytes is under a budget.  The budget is M2's measured fixed part, less the f16 saving, plus 10%.  It is at most 1100 MB.

The 1100 MB leaves about 950 MB under the iPhone's limit for WebKit, the JS heap and WebGL.  These parts are not measured.  TD-11 and TD-54 close.

The maintainer publishes, then opens, separates, plays and saves a 30-minute song on the iPhone.  The tab stays in the foreground, with Auto-Lock set to Never and the phone on a charger.  Playback includes a loop and seeks.  `idevicesyslog` shows no `exceeded mem limit` for WebContent.

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
* [ ] M3: stems to storage.
* [ ] M4: mix from storage.
* [ ] M5: decode from and into storage.
* [ ] M6: playback through the read-ahead buffer.
* [ ] M7: lanes, spectrograms, classic drums and sessions from storage.
* [ ] M8: saving from and to storage.
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

## Outcomes and retrospective

Not started.
