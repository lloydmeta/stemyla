# GPU speed, M1: `memory-budget` at the default chunk (TD-11)

## Purpose

This is the milestone plan for M1 of `plans/active/gpu-speed.md`.  The parent's M1 section stays the criteria.

`e2e memory-budget` fails its worker growth check of 4 MB at the default chunk, the chunk that users run.  The worker grows by one chunk's working set, about 95 MB, and then stays flat.  So the check tests the wrong thing.  M1 replaces it with four checks that MEM-4 needs.  M1 also finds where the 95 MB goes, buffer by buffer, and cuts the cheap parts.  The cuts give room to M6, which holds a second chunk's input while the GPU runs.  The stems stay the same byte for byte.

Readers: the implementer, the review agents of M1's rounds, the orchestrator, and the team leader of M6.

## Context and orientation

The state at the start, on 2026-11-12, at commit `22cdd74` on branch `gpu-speed/2026-11-12`.  The review base of M1 is `22cdd74`.

### The decisions that M1 follows

* The maintainer, 2026-11-12: "a good, real solution" to the failed growth check.  Do not widen `MAX_GROWTH_MB` to make it pass.
* The orchestrator, 2026-11-12 (parent's Decision log): replace the check with four checks.  These are flat after warm-up, no growth with the song, a working-set budget from the chunk's buffer sizes with measured headroom, and a tab total for the iPhone (MEM-4).  First find where the growth goes, with evidence.  Then cut cheap parts so that M6 has room.
* No new allocator, crate or module without the orchestrator.  A tolerance gets a proof from the spread of several runs.
* Runs on the Radeon Mac or the iPhone are not needed.

### The code today

* `xtask/src/e2e.rs`, `memory_budget`: it separates a 1-minute and a 4-minute song (`BUDGET_MINUTES`), each in a new browser, with the query `chunk=16384&overlap=1&mm=s64`.  `growth_check` fails it when the page or the worker grows by more than `MAX_GROWTH_MB` (4 MB) from the separation's start to its end.  `worker_growth` reads the worker's lines `memory: worker wasm N MB at the separation's start` and `... at the separation's end`.  `memory_peak` gives the peak of page wasm, worker wasm, decoder wasm and GPU live bytes; nothing asserts it.  `open_growth_check`, `shown_growth_check`, `no_fallback_check` and `no_mix_write_check` stay as they are.  `old_session.rs` also uses `MAX_GROWTH_MB`; M1 does not change it.
* `worker/src/web.rs`, `GpuNetwork::stems`: one chunk.  It calls `roformer::separate::analyse`, which gives both spectra and the features.  It makes a `Job` if the frame count changes, calls `Job::set_input(&feats)`, drops the features, and runs each step through `Pace::run`.  After the last mask step of each stem it reads the mask (`Job::mask`) and calls `roformer::separate::apply_masks`.  It returns the six stems together.
* `worker/src/web.rs`, `separate_chunks`: for each chunk, `Demixer::next_input` gives `x`, `net.stems(index, &x)` gives the six stems, `Demixer::accept` adds them, and `Demixer::take_ready` gives a `Segment` for `emit`.  `x` lives to the end of the loop body.  The worker logs `chunk {done}/{total}: ...; memory: worker wasm N MB, ...` after each chunk (`memory_text`).  `memory_text` names the weight format: `f16 weight matrices` or `f32 weights`.
* `worker/src/web/mix.rs`, `StoredMix::read`: it copies the span's bytes into WebAssembly memory, then converts them to two channels.
* `roformer-gpu/src/net.rs`, `Job::set_input`: it makes a normalised copy of the features (`out`, `t * FEATURES` floats), then writes it to the GPU in one `write`.  Each band's rows are one block of `out`, at `t * g.offset + j * t * w`.  `Gpu::read` copies a read-back into WebAssembly memory once (`mapped_floats`).  wgpu 30's web `write_buffer` passes a view of WebAssembly memory (`src/backend/webgpu.rs`, `write_buffer`), so it makes no copy there.
* `roformer/src/separate.rs`, `apply_masks`: for each channel, it clones the whole spectrum, multiplies it by the mask, and calls `roformer::stft::istft`.
* `roformer/src/demix.rs`, `Demixer::accept`: it adds all stems of a chunk to the running sums `result`, then adds the window to `counter`, then moves `pos`.  `take_ready` drains the sums but keeps their capacity.  A song is padded only when its frames are more than `2 x border`, 588,800 at the default chunk.  So at the default chunk a song has at most 2 chunks, or at least 5.
* `worker/src/web/tests.rs`: a test-only global allocator, `PeakAllocator`, keeps `LIVE` and `PEAK` heap bytes for each wasm test.  It counts exactly, so a run gives the same count each time.  `separation_peak` and `a_stored_mix_keeps_the_worker_memory_flat` use it with `FakeNetwork`.  M1 uses this allocator; it adds no allocator.
* Other users of the APIs that M1 touches: `roformer/tests/reference.rs`, `roformer-gpu/tests/reference.rs`, `roformer::separate::separate_chunk` and `xtask/src/roformer_cli.rs` call `analyse`, `apply_masks`, `Demixer::accept` or `Job::set_input`.  Their signatures and results must not change.

### The buffers of one chunk at the default chunk

The default chunk `roformer::CHUNK` is 588,800 samples, so `T` = 1,151 frames.  `BINS` = 1,025, `FEATURES` = 4,100, `N_FFT` = 2,048, `HOP` = 512, 6 stems, overlap 2.  Bytes, from the code above:

| Buffer | Formula | Bytes | Lives |
| - | - | - | - |
| Chunk input `x` | 2 x CHUNK x 4 | 4,710,400 | the whole chunk loop |
| Bytes of the span read from the mix file | 2 x CHUNK x 4, at most | 4,710,400 | inside `StoredMix::read` |
| Span read from the mix file | 2 x CHUNK x 4, at most | 4,710,400 | inside `next_input` |
| Reflect-padded last chunk (`reflect_right`) | CHUNK x 4 | 2,355,200 | inside `next_input`, last chunk |
| Spectra, both channels | 2 x T x BINS x 8 | 18,876,400 | from the STFT to the last stem |
| Features | T x FEATURES x 4 | 18,876,400 | from the STFT to the upload |
| Normalised input in `set_input` | T x FEATURES x 4 | 18,876,400 | inside `set_input` |
| One mask | T x FEATURES x 4 | 18,876,400 | one stem |
| Masked spectrum, one channel | T x BINS x 8 | 9,438,200 | one channel of one stem |
| `istft` sums `out` and `env` | 2 x (N_FFT + HOP x (T - 1)) x 4 | 4,726,784 | one channel |
| Output being collected, one channel | HOP x (T - 1) x 4 | 2,355,200 | end of one channel |
| One stem, stereo | 2 x HOP x (T - 1) x 4 | 4,710,400 | until `accept` |
| Six stems | 6 x 4,710,400 | 28,262,400 | until `accept` |
| Demixer sums `result` | 6 x 2 x CHUNK x 4 | 28,262,400 | from chunk 1 to the end |
| Demixer `counter`, `window` | 2 x CHUNK x 4 | 4,710,400 | from chunk 1 to the end |
| Window clone in `accept` | CHUNK x 4 | 2,355,200 | inside `accept` |
| Segment of `take_ready` | 6 x 2 x (CHUNK / overlap) x 4 | 14,131,200 | until `emit` ends |
| PCM bytes in `StemWriter::put` | 4 x CHUNK / overlap | 1,177,600 | inside `put` |

An estimate of today's peak, from chunk 2 on, at the end of the last stem's inverse STFT of channel 1: demixer 32.97 MB, `x` 4.71, spectra 18.88, five stems 23.55, mask 18.88, masked channel 9.44, `istft` sums 4.73, channel 0's output 2.36 and channel 1's output being collected 2.36.  That is about 117.9 MB of live heap.  The upload phase is about 94.3 MB: demixer, `x`, spectra, features and the normalised input.  Step 1 measures the real figures.

The worker's WebAssembly memory today, in Chrome, at the default chunk: 30 MB at the separation's start, 58 MB before chunk 1, 92 MB after chunk 1, 125 MB after chunk 2 to the end (parent's Context).  WebAssembly memory never shrinks, so it shows the high-water mark of the heap, not the live bytes.  The heap that is already free at the separation's start takes part of the working set.  So the growth (95 MB) is less than the working set (about 118 MB).

### The paths of this run

* Weights: `~/Downloads/bs_roformer_sw.f16.safetensors`, outside the repo (ASSET-1).
* Chrome: `CHROMEDRIVER=~/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.
* Safari: through `safaridriver`, which `xtask` drives.  Never quit the maintainer's Safari.  Read `docs/references/local-browser-testing.md` first.
* Build the page with `(cd app && trunk build --release)` before each e2e run.  Run `cargo run -p xtask --release -- e2e <scenario> --browser safari|chrome --weights <path>`.  Run one browser GPU run at a time.

### Terms

* Working set: the live heap bytes that one chunk's separation holds at its peak, above the worker's live heap before the separation.  `PeakAllocator` measures it.
* Warm-up: chunks 1 and 2.  The demixer's sums reach their size at chunk 1.  Chunk 2 is the first chunk that runs with them live.
* Headroom: the worker's WebAssembly growth during a separation less the working set.  It is the allocator's overhead and fragmentation and the page rounding (64 KiB pages), less the heap that is free at the separation's start.  So it can be less than 0.  It depends on the weight format, because the model load leaves the free heap.

## Plan of work

The simplest fix that the maintainer allows is the check alone, with no cut.  The orchestrator also asks for room for M6, so M1 cuts too.  The cuts are in order of cost.  Each cut keeps each output float the same, because each sum keeps its terms and their order.

Each step's commit holds the doc changes that the step makes true (DOC-1).  Step 9 lists them.

### Step 0: the baselines (measure only)

1. Build the page at `22cdd74`.  Run `e2e stored-stems --weights <path>` in Chrome and in Safari.  Record the six SHA-256 hashes of each browser in Surprises.  If the browsers differ, each later run compares with its own browser's hashes (`--hashes`).
2. Run `e2e memory-budget --browser chrome --weights <path> --query x=1`.  `x=1` gives the defaults, as the parent's Context does.  The run fails its growth check.  Record the worker's figure after each chunk, the four peaks and the time of each song in Surprises.

### Step 1: the working-set function and its measure

1. Add a pure function in `roformer/src/separate.rs` that gives the worker's working set for `(chunk, overlap)`.  It returns named parts in bytes for each of three phases of a chunk: the upload, the masks, and the time between two chunks (the demixer's end of the chunk, the emit and the next read).  It also returns the peak, the largest phase.  `roformer` is the lowest crate that `stem-worker` and `xtask` both take (LAYER-1).  Its doc says that it describes the worker's design at this commit, and that a change of that design changes it in the same commit.  A native test checks its parts at `CHUNK` and overlap 2 against the table above.  The test fails first on an assert, with a stub that returns 0.
2. Split the CPU work of `GpuNetwork::stems` from its GPU work, with no change of behaviour.  A small trait inside `worker/src/web.rs` gives the GPU half: upload a chunk's features, then give each stem's mask in turn.  `GpuNetwork` implements it with `Job` and `Pace`.  A function runs the CPU half over the trait.
3. Add a wasm test in `worker/src/web/tests.rs`, `a_chunk_of_the_default_length_holds_its_working_set`.  It separates a stored mix of at least 5 chunks at `CHUNK` and overlap 2 through `separate_song`, with a fake GPU half.  For example, 3 x 294,400 frames give 5 chunks.  The fake allocates what the real one holds.  During the upload, it holds the normalised input; from Step 4, it calls the real pure normalisation function of Step 4.  It gives one mask of `T x FEATURES` floats for each stem, as `mapped_floats` does.
4. The test measures the peak of each phase with `PeakAllocator`.  The fake GPU half sets `PEAK` to `LIVE` at each phase boundary that it sees, and reads `PEAK` at the next one.  It sees three phases: the upload, the masks, and the time between two chunks' calls (the demixer's end of the chunk, the emit and the next read).  The function gives its peak for the same three phases.  For each phase of chunks 2 and 3, the test asserts that the measured peak above the separation's start is within `SMALL` of the function's, on both sides.  `SMALL` is the sum of the small buffers that the function does not name: the FFT plan and scratch, the two `hann` windows, the log strings and the `Segment` and `Vec` headers.  The test's doc names each one, with its size.  So the test fails when a phase holds a buffer that the function does not name, and when the function names a buffer that the phase no longer holds.  This holds also for a phase that is not the largest.
5. Run it in Chrome (`cargo run -p xtask -- web-test a_chunk_of_the_default_length_holds_its_working_set`, then the `-- --nocapture` form that `local-browser-testing.md` gives).  Record in Surprises the measured live bytes of each phase, beside the table's parts.  This is the evidence of where the growth goes.  If a phase does not agree, find the buffer and add it to the function and the table before a cut.  Record the test's time.

Each later cut first changes the function's parts.  Then the test fails on its assert, because the code still holds the old buffer.  The cut makes it pass.

### Step 2: cut 1, each stem goes to the demixer as it is made

* Saves five stems at the peak: 23.55 MB.
* `Demixer` gets `accept_stem(stem, &Stereo)` and `end_chunk()`.  `accept` calls them, so its callers do not change.  The window clone goes: the edge fades apply by index.
* `Network::stems` gives each stem to the caller as it is made, for example through a callback.  `separate_chunks` gives it to `accept_stem`.  `FakeNetwork` and `RewritingNetwork` follow.
* The mask drops before the stem goes to the demixer.
* `end_chunk` moves `pos`.  So the next chunk's input cannot be read before every stem of this chunk is accepted.  M6 needs that order: it gets a read cursor apart from the accept cursor.  Record this for M6 in Surprises.
* Tests: a native test that `accept_stem` for each stem, then `end_chunk`, gives the same sums and segments as `accept`, bit for bit.  It runs over `STREAM_CASES` (`roformer/src/demix.rs`), so the first and the last chunk, whose fades are 1, are in it.  It fails first on an assert, with a stub.  Run `e2e stored-stems --hashes <Step 0's>` in Chrome after the step.

### Step 3: cut 2, the mask applies one frame at a time

* Saves the masked spectrum of one channel: 9.44 MB.
* `roformer::stft` gets an inverse STFT that takes each frame from a function.  `istft(&Spectrum)` calls it, so its result does not change.  `apply_masks` multiplies each frame by the mask into a frame buffer, and gives it to that function.  It clones no spectrum.
* Tests: a native test that the new `apply_masks` equals the old one bit for bit, on a random spectrum and mask.  Keep the old one as a test helper for the comparison.  A test of the frame function fails first on an assert, with a stub.  Run `e2e stored-stems --hashes` in Chrome after the step.

### Step 4: cut 3, `set_input` writes one band at a time

* Saves the normalised input at the upload, less one band: about 16.5 MB.  The widest band group sets the size of the band buffer.
* `Job::set_input` normalises one band's rows into a buffer of one band, and writes it to the GPU at the band's offset.  Then it goes to the next band.  Each band's sum of squares keeps its terms and their order, so each float on the GPU is the same.  The change stays inside `set_input`; its signature and its callers do not change.
* Factor the normalisation of one band into a pure function over the features, the band's gains and a buffer.  The fake GPU half of Step 1 calls it, so it holds what the real one holds.
* Tests: a native test, with no GPU, that the bands of the pure function, put together at their offsets, equal the normalised input of today's code, bit for bit.  Keep today's code as a test helper for the comparison.  It fails first on an assert, with a stub.  Run `e2e stored-stems --hashes` in Chrome after the step.
* A larger cut also drops the features, and builds each band from the spectra.  It saves 18.88 MB more at the upload, but changes the worker and reads the spectrum's layout again.  Do it only if Step 6 needs it.

### Step 5: cut 4, the chunk input drops after the STFT

* Saves `x`: 4.71 MB, during the masks.
* `Network::stems` takes `x` by value, and the CPU half drops it after the STFT.
* Do this step only if it is a small change.  Else record why in the Decision log, and keep `x` in the function's parts.

### Step 6: the room for M6, and a decision

After Steps 2 to 5, the estimated mask phase is about 80.2 MB: demixer 32.97, spectra 18.88, mask 18.88, `istft` sums 4.73, and 4.71 of outputs.  The estimated upload phase is about 73.1 MB: demixer, spectra, features and one band (2.38).  Without Step 5, `x` adds 4.71 MB to each: 84.9 and 77.8 MB.

M6 runs the STFT and the upload of chunk n + 1 while chunk n's last mask applies.  So M6's phase holds chunk n's mask phase and chunk n + 1's largest part.  That part is at the STFT's end: its `x`, spectra and features, 42.5 MB.  So M6 needs about 80.2 + 42.5 = 122.7 MB, or 84.9 + 42.5 = 127.4 MB without Step 5.  M6 adds about 42.5 MB to M1's peak.

The trigger for a larger cut is MEM-4, not a target of this plan.  Take the tab total of Step 0, which is before the cuts, so the test is conservative.  Add the sizes of the buffers that M6 adds, from the table.  If the sum is at most `TAB_BUDGET_MB`, no larger cut is needed.  Else decide after Step 8, with its tab total.  Then run Step 8 again after a larger cut, since the cut changes the function and the growth.  The larger cuts in order of cost:

1. Drop the features at the upload (Step 4's larger cut): 18.88 MB.
2. Read each mask in blocks of frames, and apply each block before the next read: most of 18.88 MB.  It costs more read-backs for each stem and an inverse STFT that keeps its sums between blocks.

Record the figures and the decision in the Decision log.  Give the orchestrator a diff of the parent's M6 section with M6's part, as Step 9 says.

### Step 7: the four checks of `memory-budget`

Pure functions in `xtask/src/e2e.rs`, each with a unit test that fails first on the case that it catches:

1. Flat after warm-up: from the worker's `chunk 2/N` line to the end, the worker's WebAssembly memory grows by at most `MAX_GROWTH_MB`.  The function reads `worker wasm` from each `chunk k/N` line with k of 2 or more, and from the separation's end line.  It fails when a later value is more than `MAX_GROWTH_MB` above chunk 2's, or a line is missing.  Test cases: flat figures pass; a rise of 4.1 MB at chunk 5 fails; no chunk 2 line fails.
2. No growth with the song: the 4-minute song's worker growth, from the separation's start to its end, is at most `MAX_GROWTH_MB` above the 1-minute song's.  Test cases: equal growths pass; 4.1 MB more fails; a missing growth fails.
3. A working-set budget: the worker's growth during a separation is at most the function's peak at `roformer::CHUNK` and `roformer::OVERLAP`, plus `WORKER_HEADROOM_MB`.  The headroom is measured at the default chunk and overlap with f16 weights only (Step 8), and it can be less than 0.  So the check prints "not checked" with the reason, and does not fail, in two cases.  One is a query with `chunk=` or `overlap=`.  The other is a log with `f32 weights`.  Test cases: growth at the budget passes; 0.1 MB over fails; a query with `chunk=16384` gives "not checked"; a log with `f32 weights` gives "not checked".
4. A tab total: the sum of the four peaks (`MemoryPeak::total`: page wasm, worker wasm, decoder wasm, GPU live) is at most `TAB_BUDGET_MB`.  The sum of the peaks is at least the peak of the sum, so the check is safe.  `TAB_BUDGET_MB` is 943 MB: the 1,100 MB of M10 of `plans/completed/stream-from-storage.md`, less an MP3 save's frames (141 MB) and a save's page memory (16 MB).  `memory-budget` runs no save, so those parts are not in its total.  The 1,100 MB leaves about 950 MB under the iPhone's 2,048 MB for WebKit, the JS heap and WebGL, which no check measures (TD-11).  The 1,100 MB assumes f16 weights.  With `f32 weights` in the log, the GPU alone holds about 979 MB, so the check prints "not checked: the budget assumes f16 weights (TD-63)" and does not fail.  Test cases: 943 passes; 943.1 fails; a missing part fails; a log with `f32 weights` gives "not checked".

Then:

* `memory_budget` uses the four checks.  The worker's part of `growth_check` goes; the page's part stays.  Each check prints its figures and its margin, and the function's parts, so that a failure names the buffer to look at.
* The scenario's default query gives the defaults: no `chunk`, `overlap` or `mm`.  Why: users run the defaults, and `s64` is 9.6 times slower in Safari (parent's Context).
* Fix the doc of `MemoryPeak::total`: the sum of the peaks is at least the peak of their sum, not at most.  Change the doc of `MAX_GROWTH_MB` to list the checks that use it.
* Initial `WORKER_HEADROOM_MB`: from Step 0's run and Step 1's test.  Step 8 sets it.

### Step 8: the headroom, measured

1. Run `memory-budget` 3 times in Chrome and 3 times in Safari at the default chunk, with f16 weights, after Steps 2 to 7.  Each run gives two songs, so 12 growths.
2. For each growth, take the headroom: the growth less the function's peak.  It can be less than 0.  Record each headroom, its browser and its song in Surprises.
3. Set `WORKER_HEADROOM_MB` to the largest headroom, rounded up to the next whole MB.  Write that proof in the constant's doc, with the spread (largest less smallest).
4. The wasm test of Step 1 guards the CPU half, phase by phase.  Check 3 guards what the wasm test cannot see: the real `Job` and `Gpu::read`, and the allocator in a real run.  A lost buffer raises the growth by the rise of the largest phase, not by its size.  For each cut, compute from the function the rise of the peak if that cut is lost.  For example, a lost cut 3 makes the upload phase about 89.6 MB against a mask phase of 80.2, a rise of about 9.4 MB.  The worker logs whole MB, so a growth (end less start) has an error of up to 1 MB.  So the spread, plus the rounding of step 3, plus 1 MB, must be less than the smallest rise of a lost cut in the GPU half (cut 3).  If it is not, record the figures, and decide in the Decision log before Step 9.
5. Also record the figures of checks 1, 2 and 4 in each run.  Checks 1 and 2 must be under `MAX_GROWTH_MB` in every run.  If one is not, do not widen the constant: find the cause first.

### Step 9: the docs and the debt

Each in the commit of the step that makes it true (DOC-1):

* `xtask/src/e2e.rs` (Step 7): the module doc of `memory-budget`, the doc of `memory_budget`, `MAX_GROWTH_MB` and `MemoryPeak::total`.
* `README.md` (Step 7): the line on `memory-budget` and its growth check.
* `docs/ARCHITECTURE.md`, memory on phones (Steps 2 to 5 and 7): the worker's figures at the default chunk, the parts of its working set, the cuts, and the four checks.
* `docs/invariants.md`, MEM-4, "Enforced by" (Steps 1 and 7): the four checks and the new wasm test.
* `docs/tech-debt.md` (Step 7 or 8):
  * TD-11: remove the bullet on the chunk.  Change its "Fix:" line: the tab total is done; the JS heap and the WebGL textures are left.  Change the bullet on the features that `GpuNetwork::stems` drops: the wasm test now sees the CPU half's working set, with a fake GPU half that calls the real normalisation.  Keep what still has no check: the real `Job` and `Gpu::read` in a run without a GPU, and the JS heap.
  * TD-54: change its "Fix:" line, since TD-11's assertion at the default chunk is done.
* Parent plan (`docs/plans/active/gpu-speed.md`): only the orchestrator edits it.  The team leader gives a diff of its M6 section at the checkpoint before review round 1.  It says that M6 adds its second chunk's input as a named part of the working-set function, gets a read cursor apart from the accept cursor, and stays under `TAB_BUDGET_MB`.

### Before review round 1

* Run `e2e stored-stems --hashes <Step 0's>` in Chrome and in Safari on the last commit.
* Run `memory-budget` once in each browser on the last commit.  Both pass.
* Run `a_chunk_of_the_default_length_holds_its_working_set` in Chrome and in Safari.
* `cargo run -p xtask -- check --tier push` and `cargo build -p xtask` pass.

## Validation and acceptance

* `cargo run -p xtask --release -- e2e memory-budget --browser chrome --weights <path>` passes at the default chunk, with the four checks.  The same in Safari.
* `cargo run -p xtask --release -- e2e stored-stems --browser chrome --weights <path> --hashes <Step 0's>` passes after each CPU change.  The same in Safari at the end.
* `cargo run -p xtask -- web-test a_chunk_of_the_default_length_holds_its_working_set` passes in Chrome and Safari.  It failed on its assert after each change of the function's parts, before the cut.
* `cargo test -p xtask` runs each check's tests.  Each failed first on its case.
* `cargo test -p roformer` and `cargo test -p roformer-gpu --lib` run the equality tests of Steps 2 to 4.
* `cargo run -p xtask -- check --tier push` passes.
* TD-11 has no bullet on the chunk.

## Idempotence and recovery

Each step is one commit or more, signed, on `gpu-speed/2026-11-12`.  A session that stops reads Progress and goes on at the next open step.  A cut that fails `stored-stems` is reverted with `git revert`, and the next cut goes on.  The e2e runs write nothing to the repo.  A run that a locked screen or a Safari timeout stops runs again; `local-browser-testing.md` gives the causes.

## Progress

* [x] Step 0: the baselines (2026-11-12, at `32a25fc`, whose code is the code of `22cdd74`).
* [x] Step 1: the working-set function and its measure (2026-11-12).  First failures: `the_working_set_at_the_default_chunk_has_the_sizes_of_its_buffers` at `assert_eq!(part(moment, name), bytes, ...)` ("demixer sums at the stub", left 0, right 28262400), and `the_working_set_follows_the_chunk_and_the_overlap` at `assert_eq!(part(upload, "chunk input"), 2 * chunk * 4)` (left 0), with a stub that gives no parts.  `a_chunk_of_the_default_length_holds_its_working_set` at `assert!(held.abs_diff(phase.bytes()) <= SMALL, ...)` ("the upload phase of chunk 2 holds 75436204 bytes, and the function gives 94312400"), with a fake GPU half that held no normalised input.
* [x] Step 2: each stem goes to the demixer as it is made (2026-11-12).  First failures: `stems_one_at_a_time_give_the_sums_of_a_whole_chunk_exactly` at `assert_eq!(sums_of(&each), sums_of(&whole), ...)` ("n=15 chunk=10 overlap=2: the sums after chunk 0", left `(0, [])`), with stubs of `accept_stem` and `end_chunk`; `a_chunk_cannot_end_before_each_stem_comes` and `a_stem_cannot_come_twice_in_a_chunk` did not panic.  After the function's change, `a_chunk_of_the_default_length_holds_its_working_set` failed at `tests.rs:3347` ("the masks phase of chunk 2 holds 117900810 bytes, and the function gives 94311384").  After the cut it passed, and `stored-stems --hashes` passed in Chrome.
* [x] Step 3: the mask applies one frame at a time (2026-11-12).  First failure: `frames_from_a_function_give_the_inverse_of_the_whole_spectrum` at `assert_eq!(asked, (0..s.frames).collect::<Vec<_>>(), "each frame once, in order")` (left `[]`), with a stub of `istft_frames` that gave zeros.  After the function's change, `a_chunk_of_the_default_length_holds_its_working_set` failed at `tests.rs:3360` ("the masks phase of chunk 2 holds 94348666 bytes, and the function gives 84873184").  After the cut it passed, and `stored-stems --hashes` passed in Chrome.
* [x] Step 4: `set_input` writes one band at a time (2026-11-12).  First failure: `bands_normalised_one_at_a_time_equal_the_whole_input` at `assert_eq!(g.to_bits(), w.to_bits(), "float {k}")` ("float 0", left 0, right 3211480212), with a stub of `normalise_band` that gave zeros.  After the function's change, `a_chunk_of_the_default_length_holds_its_working_set` failed at `tests.rs:3360` ("the upload phase of chunk 2 holds 94312610 bytes, and the function gives 77811664").  After the fake called the real `normalise_band` it passed.  `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passed on the M2 Pro (Metal), and `stored-stems --hashes` passed in Chrome.
* [x] Step 5: the chunk input drops after the STFT (2026-11-12).  After the function's change, `a_chunk_of_the_default_length_holds_its_working_set` failed at `tests.rs:3377` ("the upload phase of chunk 2 holds 77812066 bytes, and the function gives 75436000").  After `Network::stems` took `x` by value and the CPU half dropped it after `analyse`, it passed.  The 109 wasm tests of `stem-worker` passed in Chrome, and `stored-stems --hashes` passed in Chrome.
* [ ] Step 6: the room for M6.
* [ ] Step 7: the four checks.
* [ ] Step 8: the headroom.
* [ ] Step 9: the docs and the debt.
* [ ] Review rounds.

## Surprises and discoveries

* 2026-11-12, plan review round 1: at the default chunk a song has at most 2 chunks or at least 5, because a song is padded only above `2 x border` frames (`Demixer::new`).
* 2026-11-12, Step 0: `e2e stored-stems --weights <path>` passed in Chrome 157 and in Safari 27.2.  Both browsers give the same six SHA-256s, so each later run compares with one list.  The list for `--hashes`: `55b981ea1ae82ddae4b0e6c5b5479de3447cf6bea51095f1ac07415cdf7a7348,7f0b47d66796b8346b2e07012e1c65abec4a0c622a365a5f772312ea728f687d,6f97eccacdd66503a20256282a82b6a5e014b70fff60818abe023a8636b1bf28,f1b79d872435cdb931306a0c33228905c2d6b3ddfd33e9f1cb55720f52d0b624,87fc0cc23bdefac194dfe5f0268a1684a78928f74c630f3d8d664cdb78189d8a,3166e0de310c930f2b40f75c2631e4524b7115c64f90d4ef540102af6da68f90`.  `stored-stems` sets its own options (`mm: 's64', attn: 'split', chunk: 16384, overlap: 2`), so it tests the CPU path at 17 segments of a 3-second clip, not at the default chunk.
* 2026-11-12, Step 0: `e2e memory-budget --browser chrome --query x=1` fails at the 1-minute song, as the plan expects.  `Page::finish` stops the run at the first failed song, so the 4-minute song did not run.  The worker's lines: 30 MB at the separation's start, 58 MB at `separating 11 chunks`, 92 MB after chunk 1, 125 MB after chunks 2 to 11, 125 MB at the end.  `FAIL: the worker grew by Some(95.0) MB during the separation, at most 4 MB`.  The four peaks: page wasm 33 MB, worker wasm 125 MB, decoder wasm 6 MB, GPU live 673 MB, sum 837 MB.  The separation took 51.5 s (11 chunks, 4.7 s each).  The page grew by 0 MB during the open and the separation, and by 18 MB to show the stems.
* 2026-11-12, Step 1: where the growth goes, measured by `a_chunk_of_the_default_length_holds_its_working_set` in Chrome 157 (`-- --nocapture`), in live heap bytes above the separation's start, for chunks 1 to 5.  Upload: 63,695,004, then 94,312,604 for chunks 2 to 5; the function gives 94,312,400 (+204).  Masks: 87,283,204, then 117,900,804; the function gives 117,863,384 (+37,420).  Between (chunks 1 to 4): 68,301,148 each; the function gives 68,300,800 (+348).  So each phase agrees with the table's parts, and no buffer is missing.  The masks phase's 37,420 bytes are the FFT plans, buffers and `hann` windows.  Chunk 1 is lower by the demixer's sums and counter, which `accept` makes at chunk 1.  The test took 23.6 s in Chrome (the whole binary run, with the build of the mix file).
* 2026-11-12, Step 1: the between phase's peak today is the demixer's `accept`, with the six stems and the window clone: 68.3 MB.  The emit (53.0 MB) and the next read (42.4 MB) are less.  The plan's table had these parts, not this order.
* 2026-11-12, Step 2: after cut 1, the wasm test measured (Chrome) upload 94,312,610 (+210 over the function), masks 94,348,666 (+37,282), between 52,992,402 (+402) for chunks 2 to 5.  The upload phase (94,312,400) is now the peak, 1,016 bytes over the masks phase (94,311,384).  The between phase's peak is the emit: the demixer, `x`, the segment and the PCM bytes.
* 2026-11-12, Step 2, for M6: `Demixer::end_chunk` moves `pos`, and `next_input` reads at `pos`.  So the next chunk's input cannot be read before each stem of this chunk is accepted and the chunk ends.  `end_chunk` asserts that each stem came, and `accept_stem` that none came twice.  M6 needs a read cursor apart from the accept cursor to read chunk n + 1 while chunk n's stems come.
* 2026-11-12, Step 3: after cut 2, the wasm test measured (Chrome) masks 84,910,466 (+37,282 over the function's 84,873,184).  Upload (94,312,610) and between (52,992,402) did not change.
* 2026-11-12, Step 4: after cut 3, the wasm test measured (Chrome) upload 77,812,066 (+402 over the function's 77,811,664).  The masks phase (84,910,466 measured, 84,873,184 from the function) is now the peak.  The band buffer is `T x 516` floats (2,375,664 bytes): the widest band has 129 bins.
* 2026-11-12, Step 5: after cut 4, the wasm test measured (Chrome) upload 75,436,210 (+210 over 75,436,000), masks 80,200,066 (+37,282 over 80,162,784), between 48,282,002 (+402 over 48,281,600).  The upload phase's peak is now the STFT's end, where `x` still lives beside the spectra and the features (75.4 MB).  The plan's estimate (73.1 MB) left `x` out there.  `analyse` takes `&x`, and its signature must not change, so `x` drops after it returns.  The peak of the working set is the masks phase, 80.2 MB, as the plan estimated.

## Decision log

* 2026-11-12, agent (team leader): `memory-budget` runs at the defaults, with no `chunk`, `overlap` or `mm` in its query.  Why: users run the defaults; `s64` is 9.6 times slower in Safari.
* 2026-11-12, agent (team leader): the evidence of where the growth goes comes from `PeakAllocator`, the test allocator that `worker/src/web/tests.rs` has.  Why: it counts live heap bytes exactly, needs no GPU, and runs in CI.  The orchestrator forbids a new allocator.
* 2026-11-12, agent (team leader): the working-set function is in `roformer::separate`, and describes the design at each commit.  Why: `stem-worker` and `xtask` both take `roformer` (LAYER-1), so one function gives the test's bound and the e2e budget.  A two-sided test keeps it true.
* 2026-11-12, agent (team leader): the tab budget is 943 MB: the 1,100 MB of M10 of `plans/completed/stream-from-storage.md`, less the save's parts.  Why: it is the budget already derived for MEM-4, and `memory-budget` runs no save.
* 2026-11-12, agent (team leader): the headroom is measured with f16 weights only, and check 3 says "not checked" with f32 weights.  Why: the free heap at the start depends on the model load, and the f16 file is the default.
* 2026-11-12, agent (team leader, plan review round 1): cut 3 writes `set_input` one band at a time, and keeps the features.  Why: it saves almost as much at the upload, stays inside `set_input`, and changes no caller.  Dropping the features waits for Step 6.
* 2026-11-12, agent (team leader): "M1's budget" for M6 means the same function, with M6's part added, and a tab total under `TAB_BUDGET_MB`.  Why: a budget fixed at M1's design would fail M6 for a buffer that M6 needs, and MEM-4 is the real limit.
* 2026-11-12, agent (team leader, plan review round 2): checks 3 and 4 print "not checked" with `f32 weights`, and check 3 also with a `chunk=` or `overlap=` query.  Why: the headroom and the 943 MB are measured or derived for f16 weights at the default chunk only.
* 2026-11-12, agent (team leader, plan review round 2): the wasm test asserts each phase on both sides.  Why: a cut that lowers a phase that is not the largest would else fail nothing.
* 2026-11-12, agent (team leader): a larger cut waits for Step 6's figures.  Why: each costs more code or more read-backs; the cheaper cuts may be enough.
* 2026-11-12, agent (implementer): `working_set` gives each phase as a list of moments, each with its parts, and a phase's bytes are its largest moment.  Why: a cut changes which moment of a phase is the largest (the between phase goes from `accept` to the emit at cut 1), and the function then picks it with no other change.
* 2026-11-12, agent (implementer): the test's phase boundaries are the entry and the exit of `Network::stems` and the exit of `GpuHalf::upload`.  So the upload phase holds the STFT too.  Why: the fake sees these boundaries with no change of the CPU half, and the STFT's end is less than the upload's peak.
* 2026-11-12, agent (implementer): `GpuHalf::mask(0)` runs the shared layers (`Job::steps` less its mask steps) before stem 0's mask steps.  Why: the GPU runs the same steps in the same order as before, and `upload` stays a plain write.
* 2026-11-12, agent (implementer): the TD-11 bullet on the features, the MEM-4 "Enforced by" line and the ARCHITECTURE text change in Step 1's commit, not in Step 7.  Why: DOC-1; the wasm test sees the features drop from Step 1.
* 2026-11-12, agent (implementer): `Network::stems` gives each stem through a callback `each(stem, stereo)`, and `separate_chunks` gives it to `Demixer::accept_stem`.  The fake networks give their six stems through it.  Why: the plan's example; it keeps the order of the work and needs no new type.
* 2026-11-12, agent (implementer): `Demixer` keeps `accepted: Vec<bool>`, and `end_chunk` fails a chunk with a stem missing.  Why: with stems one at a time, a lost stem would else give wrong sums with no error.
* 2026-11-12, agent (implementer): `istft_frames` gives the frame function the inverse FFT's own input buffer, and `istft` copies each frame into it.  Why: no frame buffer is added, and the floats are the same (a test against a copy of the old `istft`).
* 2026-11-12, agent (implementer): `widest_band` is in `roformer`, beside `band_groups`, and `normalise_band` is in `roformer_gpu::net`.  Why: the working-set function (`roformer`) and `Job::set_input` need the same width; the normalisation belongs to the GPU network's input.
* 2026-11-12, agent (implementer): the TD-11 bullet says that the fake GPU half calls the real normalisation, in Step 4's commit.  Why: DOC-1.
* 2026-11-12, agent (implementer): Step 5 is done: a small change (`x` by value through `Network::stems`, one `drop`).

## Outcomes and retrospective

Not yet.
