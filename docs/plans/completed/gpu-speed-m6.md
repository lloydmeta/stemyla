# GPU speed, M6: overlap on the worker

## Purpose

This is the milestone plan for M6 of `plans/completed/gpu-speed.md`.  The parent's M6 section, with its goal "M6 measures before it builds" and its cases 1 to 11, and its "Validation and acceptance" stay the criteria.

Goal (the parent): the GPU does not wait for the CPU, and the CPU does not wait for the GPU, where the work allows.  Two changes in `worker/src/web.rs` can do this, and each one is measured on its own:

* P: `Pace` keeps up to 2 steps in flight.  The step limit and the errors still stop a run.
* O: the CPU work runs while the GPU works: the next chunk's input read and STFT, and the application of each stem's mask.

M6 first measures what each change can save, at M6's base, with runs without `profile=1`.  It builds a change only when that figure predicts a gain of 3 % or more in one browser.  If it builds neither, M6 lands no code but Step 1's test helpers.

Readers: the implementer, the team leader, the orchestrator, and the intent reviewers of M6.

## Context and orientation

The state at the start, on 2026-11-15, at commit `2543abc` on branch `gpu-speed/2026-11-12`.  The review base is `2543abc`.  The machine is the maintainer's Apple M2 Pro (19-core GPU), with Safari 27.2 and Chrome 157.

### The decisions that M6 follows

* The maintainer, 2026-11-12: kernels stay within WebGPU's default limits (GPU-1).  The perf gate runs on this M2 Pro only, with tolerances that are not too tight.  Never wait for a run on the Radeon or the iPhone.  The worker overlaps GPU steps (up to 2 in flight), measured.
* The maintainer, 2026-11-13 and 2026-11-14: never search the whole disk.  Give every agent absolute paths.  Before a report, check that no agent or command still runs.  `FUSED_MARGIN` is 1.0, and calibration takes about 33 s.
* The parent's Decision log: M6's base is the commit before M6's first commit.  M6's first commit moves the worker tests' `plant` and `plant_at` to sync access handles, with a truncate first (Step 1).  A figure that decides a build comes from runs without `profile=1`.  A change whose predicted gain is under 3 % in both browsers is not built.  Cases 10 and 11 apply to each built change.
* Browser runs (the parent's Preflight): one browser run or perf gate at a time, across all agents (TD-105).  Run each browser test through `xtask web-test` with a filter (TD-104).  No agent quits Safari or deletes its swap files (TD-103).  No reviewer or validator runs `e2e perf` or `memory-budget`.  A finding that needs such a run names it, and the team leader runs it after the round's agents end.  While a run that decides a verdict goes on, the team leader starts no agent.
* Lessons of M1 to M5.1: a build in a git worktree uses its own `CARGO_TARGET_DIR` in that worktree.  A Safari run that loses its session runs once more; then stop and report.  Write the edge cases of a rule into the plan before the code.  Put each choice of an I/O function in a pure function with a test.  Proportion (`CLAUDE.md`): a new module, crate or product change needs the maintainer's choice; the team leader asks the orchestrator.

### What is known

* A separation at the default chunk (1,151 frames) runs 470 GPU steps for each chunk with `vfused` (192 on the time axis), and 1,022 with `freq`.  Both browsers choose `vfused` since M5.  Baselines: 3.59 s for each chunk in Chrome, 5.57 s in Safari (`xtask/perf-baselines.json`).
* The parent's estimate of what P can save: M5 Step 4's cost of each wait, from the gaps by difference of profile runs, is 0.57 ms in Chrome and 0.08 ms in Safari.  So the waits of 470 steps are about 7.5 % of a chunk in Chrome and 0.7 % in Safari; the profile's upper bound gives 10 % and 3.7 %.  These figures come from `profile=1` runs, so no build rests on them.
* The CPU work of a chunk: the STFT and the inverse STFTs, about 0.3 s in Chrome and 0.25 s in Safari (the parent's Context, before M5).  That is 4.5 % to 8.4 % of M5's chunk times.  It also has the input read (`Demixer::next_input`, from browser storage on the storage path), the demixer's sums and the emit of a segment.
* The order today (`separate_chunks`, `chunk_stems` and `GpuNetwork` in `worker/src/web.rs`): read the input (`next_input`), start the chunk timer `tc`, STFT (`analyse`), upload (`Job::set_input` writes `split_in`), then for each stem: its steps (the shared steps with stem 0), the mask read (`Job::mask`, through `Gpu::read`), `apply_masks`, `accept_stem`.  Then `end_chunk`, the emit, and the chunk line `"chunk {done}/{total}: {:.2} s; {}"`.  Nothing of the CPU runs while the GPU works, and the GPU runs nothing while the CPU works.
* `Pace::run` records one step (`Job::run`), waits for all submitted work (`Gpu::done`, `onSubmittedWorkDone`), collects the profile, and checks the errors and the step limit (5 s, 60 s for the first step of each kind).  `Pace::wall` sums each step's wall time; only `log_profile` prints it, once for each separation.
* `Gpu::read` makes a staging buffer, submits a copy into it, maps it (`mapAsync`), copies it into wasm memory, and destroys it.  In WebGPU, `mapAsync` resolves only after the queue work that was submitted before the call ([WebGPU, `mapAsync`](https://www.w3.org/TR/webgpu/#dom-gpubuffer-mapasync)).  `writeBuffer` and `submit` run in the order of the calls on the queue timeline ([WebGPU, queue timeline](https://www.w3.org/TR/webgpu/#queue-timeline)).  `onSubmittedWorkDone` resolves when all the work submitted before the call is done ([WebGPU, `onSubmittedWorkDone`](https://www.w3.org/TR/webgpu/#dom-gpuqueue-onsubmittedworkdone)).
* The GPU buffers that steps share (`Job`, `roformer-gpu/src/net.rs`): `split_in` (the input; only `Step::Split` reads it), `x`, `xt` (the final-normalised state, which each `Step::Mask` reads), `h` and `y` (which the mask steps write, and the transformer steps write `h` too) and one `mask` buffer, which the mask steps of every stem write (case 2).  A chunk has 386 shared steps and 14 mask steps for each stem at the default chunk.  So a chunk's input can go to `split_in` after the chunk before has submitted its `Split` step.  A chunk's other steps can start only after the last mask step of the chunk before was submitted.
* `Demixer` (`roformer/src/demix.rs`): `next_input` reads the chunk at `pos`.  Only `end_chunk` moves `pos`, after `accept_stem` took each stem.  Each input has the chunk's size, padded in place (case 7).
* Calibration (`roformer-gpu/src/bench.rs`): `time_axis_per_chunk` times whole time-transformer steps with the function that `Job::run` uses, and waits after each step (`STEP_WAITS` is `Waits::EachStep`), as `Pace::run` does.  `CALIBRATION` is 6.
* Memory (M1, M5): the worker's working-set peak is 80.16 MB (`roformer::separate::working_set`), its growth 57 MB, and check 3's margin 6.16 MB with `WORKER_HEADROOM_MB` -17 MB.  The tab total is 756 MB (Chrome), 758 MB (Safari), and 799 MB with `freq` in Chrome; `TAB_BUDGET_MB` is 943 MB.  A spectrum pair, the features and one mask are each about 18.9 MB at 1,151 frames.  The smallest lost-cut rise is 9.44 MB.
* The test helpers: `plant` and `plant_at` in `worker/src/web/tests.rs`, and `plant_at` in `decoder/src/web/tests.rs`, write through `createWritable`.  So each leaves a Safari swap file (TD-103).  `plant` in `writer/src/web/tests.rs` uses a sync access handle: `truncate(0)`, a write of `bytes.slice()` at 0, `flush` and `close`.  The three test crates run in a dedicated worker (`run_in_dedicated_worker`).

### Terms

* Base page, head page, run, block, `r`, `T` (1 %), `N` (1.1 %), verdict: as in "The same-session comparison" of `docs/references/local-browser-testing.md`.
* M6's base: the commit of this plan.  Its page is the page of `2543abc`: this commit changes only docs.
* A step in flight: a step that the worker submitted and whose end it has not yet awaited.
* The chunk time: the figure of the chunk line.  Today, from after `next_input` to the line.  After Step 3, from the end of the line of the chunk before (chunk 1: from just before its `next_input`) to this line.
* The paced time of a chunk: the sum of `Pace::run`'s wall time over the chunk's steps.
* A CPU piece: one part of a chunk's CPU work that runs in one call: the next chunk's input read (`read_ahead`), its `analyse` with its upload, and each stem's `apply_masks` with its `accept_stem`.
* O1 and O2, the two forms of O.  O1: each stem's mask is applied while the GPU runs the first 2 steps of the next stem (Step 5a).  O2: O1, and also the next chunk's input read and STFT under the last stem's steps (Step 5b).  O1 is the simplest form: it needs no change of the input read, of `Demixer` or of the working set.
* A predicted gain: the share of the chunk time (the median of chunks 2 to N, by the new form of Step 3) that a change takes off, from Step 2's figures.

### The paths of this run

* The scratchpad of M6: `<scratch>/m6/`.  Write each path in full in each command and each brief.  The scratchpad is not kept, so each figure also goes into this plan's Surprises at the end of its step.
* The logs: `<scratch>/m6/runs/<step>-<browser>-<n>/`.  Each figure in a table names its log.
* The measuring worktree of Step 2: `<scratch>/m6/measure-wt`, detached at M6's base, with `<scratch>/m6/measure.patch` applied.  Build its page with `CARGO_TARGET_DIR=<scratch>/m6/measure-wt/target`, then `trunk build --release` in its `app/`.
* The base worktree of each comparison: `<scratch>/m6/base-wt`, detached at the base commit, built the same way with its own `target`.  Its page is `.../m6/base-wt/app/dist`.  Remove each worktree after its use with `git worktree remove`.
* The head page: `app/dist`, from `(cd app && trunk build --release)`.
* Weights: `$STEMYLA_PERF_WEIGHTS`, the main checkout's `third_party/models/bs_roformer_sw.f16.safetensors`, also given as `--weights`.  Never commit it (ASSET-1).  `$CHROMEDRIVER`, the driver of Chrome for Testing 157 (mac-arm64).  Set both for each `check --tier push` and each `e2e` run.
* `<scratch>` is the session's scratchpad folder, which the brief of each agent names.
* `caffeinate -dimsu` runs; never stop it.  Before each browser run, check with `ps -A -o pid,etime,command` that no other `e2e`, `web-test` or `check --tier push` runs.  Read `docs/references/local-browser-testing.md` first.

## Plan of work

The simplest steps first.  Each step that changes behaviour starts with a test that fails on an assert.  A step whose result makes a later step pointless records that in Surprises and skips it.

### Not done, and why

* No figure from `profile=1` decides a build (the parent).  `profile=1` runs one pass for each dispatch, and its read-back sits in the chunk time but not in the paced time.
* No more than 2 steps in flight, for P and for O: the maintainer's answer (the parent's "Decisions for the maintainer", item 3).  A deeper queue while a CPU piece runs is item 2 of "Decisions for the maintainer" below, and it is open only on the condition there.
* No second `mask` buffer on the GPU.  The order of Step 5 copies each mask out before the next stem's steps write it, so one buffer stays enough (case 2).
* No change of the WGSL, of the kernels or of calibration's candidates.
* No split of a CPU piece into smaller parts (for example `apply_masks` by channel).  It changes `roformer/`, and Step 2 first shows whether the pieces as they are give 3 %.

### The two `Gpu` methods that P and O need

`Gpu::done` and `Gpu::read` are `async fn`s.  So `on_submitted_work_done` and `map_async` run only at the first poll of the future, not at the call.  A form such as `let f = gpu.done(); job.run(next); f.await` gives a signal that covers both steps.  Then 2 steps are never in flight.  So the measuring patch, Step 4 and Step 5 use two new methods in `roformer-gpu/src/lib.rs`, each a plain `fn` that does its WebGPU call at once:

* `Gpu::done_signal(&self) -> Signal`: calls `on_submitted_work_done` now, and gives a future that resolves when the work submitted before the call is done.
* `Gpu::read_start(&self, buf, offset, len) -> PendingRead`: makes the staging buffer, submits the copy and calls `map_async` now.  `PendingRead::finish(self).await` gives the floats, then unmaps and destroys the staging buffer (MEM-3).  `Gpu::read` becomes `read_start` then `finish`.

The measuring patch takes the same two methods, so the prototype of P measures 2 steps in flight.  On the branch, the commit of the first built change (Step 4, or Step 5a if P is not built) adds them.  Each method gets a native test (it needs a GPU; `STEMYLA_REQUIRE=gpu`):

* `done_signal`: submit step A, take its signal, submit step B.  Poll the device with `PollType::Wait` for the submission index of A only, then read a flag of the signal with no await (`Signal::is_done`).  It reads true.  A signal that registers its callback only when awaited has set none, so the flag reads false every time.  No timing is in the test.
* `read_start`: after the read starts, a submitted copy or dispatch overwrites the source buffer.  Poll for the submission index of the read's copy only, then read `PendingRead::is_mapped` with no await: true.  `finish` then gives the old contents.  A deferred copy fails the contents check, and a deferred `map_async` fails the flag check.  A `queue.write_buffer` does not prove the order, since native wgpu holds that write until the next `submit`.
* `Gpu::poll` waits for all submitted work natively, so neither test awaits through it.

### Step 1: the test helpers use sync access handles (the first commit)

1. A test first, in each of `worker/src/web/tests.rs` and `decoder/src/web/tests.rs`: a spy on `createWritable` around `plant` and `plant_at` counts 0 calls.  The worker has a spy (`start_writable_spy`).  The decoder gets one in its inline JS, of the same form.  Each test fails first, on an assert, with the count of today's helpers.
2. A guard of the truncate: plant a longer file, then a shorter one at the same path, and read back the shorter size.  Run it once with a helper that has no `truncate(0)`, and record that failure (a test that passes before and after proves nothing alone).  `a_mix_file_changed_after_the_open_gives_no_old_samples` must still pass.
3. The code: each `plant` and `plant_at` writes as `plant` of `writer/src/web/tests.rs` does: `createSyncAccessHandle`, `truncate(0)`, a write of `bytes.slice()` at 0 (a whole array, MEM-5; copied before the first `await`), `flush` and `close`, in `try` / `finally`.
4. The saving and the regression check: one whole Safari `cargo run -p xtask -- web-test --browser safari` before the change and one after, each with its `browser data:` line.  The run after must pass.  Some worker tests read Safari's `lastModified` after a write in place (`worker/src/web/tests.rs`, near lines 3664 and 3712).  Also one whole `cargo run -p xtask -- web-test --browser chrome` after the change, which must pass.  Give the counts, the sizes and the verdicts in Surprises.
5. `docs/tech-debt.md`, TD-103: remove `plant` and `plant_at` of the two files from "What leaks in test runs only", and give the new count of a whole Safari `web-test`.
6. `cargo run -p xtask -- check --tier commit`, then commit, signed.

Acceptance: the spy tests fail first, then pass.  The guard's failure without the truncate is recorded.  The whole Safari and Chrome `web-test` runs pass after the change.  The commit holds only the two test files and `docs/tech-debt.md`.

### Step 2: the measurement at M6's base (no commit)

The figures that decide each build, from runs without `profile=1`.  `<pins>` is `v64,v64` in Chrome and `v64k8,v64` in Safari (the `kernels:` lines of M5.1's gates); check them against the `kernels:` line of the first unforced run, and use that line if it differs.

1. The measuring patch, `<scratch>/m6/measure.patch`, on M6's base.  It stays off the branch (the parent).  It holds the two methods above with their native tests, and adds these fields after the text of each chunk line, so `perf_result` still reads the chunk time:
   * `input <s>`: the time of the chunk's `next_input`, which is outside today's chunk time;
   * `paced <s>` and `steps <n>`: the chunk's paced time and step count;
   * `shared <s>`, and `stem<k> <s>` for each stem: the paced time of the shared steps (386 at the default chunk) and of each stem's mask steps (14 each);
   * `reads <s>`: the time of the six `Job::mask` calls;
   * `stft <s>`: `analyse` with the upload;
   * `apply<k> <s>` for each stem: its `apply_masks` with `accept_stem`;
   * the paced time of each window of 2 steps that Step 5 puts a CPU piece under: `first2<k>` for k = 1 to 5, the first 2 steps of stem k; `m5w2` and `m5w3`, steps 3 and 4 and steps 5 and 6 of stem 5; `head2`, `Split` and the first `Time` step of the chunk;
   * `rest <s>`: `end_chunk`, the emit and the rest of the chunk;
   * `inflight <n>`: the most steps in flight during the chunk.
   It also adds a query key, `pace_ahead=1`, the prototype of P.  With it, `Pace` submits a step, takes its `done_signal`, and awaits the signal of the step before, so 2 steps are in flight.  It awaits each signal before each mask read, so the read stays correct.  The key goes in `protocol/src/lib.rs` (`mod field`) and in `MEASURE_KEYS` of `xtask/src/e2e/perf.rs`, since it changes what a run measures, so a gate refuses it.
2. Build the measuring page in the measuring worktree.  Then, in each browser, one at a time:
   * 4 runs: `STEMYLA_PERF_WEIGHTS=<weights> cargo run -p xtask -- e2e perf --browser <chrome|safari> --weights <weights> --runs 4 --dist <the measuring page> --query "mm=<pins>&attn=vfused" --out <scratch>/m6/runs/2-<browser>-fields`.  For each field: the median over chunks 2 to N of each run, then the median of the 4 runs.
   * The prototype of P: `STEMYLA_PERF_WEIGHTS=<weights> cargo run -p xtask -- e2e perf --browser <b> --weights <weights> --base-dist <the measuring page> --dist <the measuring page> --query "mm=<pins>&attn=vfused" --head-query "pace_ahead=1" --runs 4 --out <scratch>/m6/runs/2-<browser>-pace`.  The verdict and the mean `r` of the blocks.  The head runs' `inflight` field must read 2, else the prototype is wrong: fix it and run again.
   * `<weights>` is `$STEMYLA_PERF_WEIGHTS`, the main checkout's `third_party/models/bs_roformer_sw.f16.safetensors`.
3. P's predicted gain in a browser: the size of the comparison's mean `r`, if its verdict is a gain; else 0.  It is the effect of P itself, at the base, with no profile.
4. O's predicted gains in a browser, from the field medians.  Each CPU piece hides at most the paced time of the 2 steps that run under it in Step 5's lists.
   * First the bound: `(input + stft + apply0 + ... + apply5) / (input + chunk time)`.  If it is under 3 % in both browsers, O is not built.
   * O1: `(min(apply0, first21) + ... + min(apply4, first25)) / chunk time`.
   * O2's addition to O1: `(min(input, m5w2) + min(stft, m5w3) + min(apply5, head2)) / (input + chunk time)`.  It is built only with O1, and only if this addition alone is 3 % or more.
   * Also give the bound at an unlimited depth, for item 2 of "Decisions for the maintainer": the same sums with the paced time of the whole span under each piece (`stem<k+1>` for `apply<k>`, `stem5` for `input` and `stft`, `shared` for `apply5`).
5. The other figures for Surprises: the CPU share (the chunk time minus `paced` and `reads`, over the chunk time), and the parent's estimate of the waits, beside the figures above.
6. The decision, for each change: build it when its predicted gain is 3 % or more in at least one browser.  Write the figures and the verdict in Surprises, and a Decision log line.
   * O1 and O2 at a depth of 2 under 3 % in both browsers, but a bound at an unlimited depth 3 % or more in one: give the orchestrator a checkpoint with the figures for item 2 of "Decisions for the maintainer", and wait.
   * Nothing is built: M6 lands no more code.  Skip Steps 3 to 9, and do Step 10.
   * P is not built: skip Step 4.  Step 5 keeps 2 steps in flight only while a CPU piece runs, and else paces as today.
   * O1 is not built: skip Steps 3 and 5.  Cases 3, 5, 6, 7 and 10 do not apply; cases 2 and 4 still do (the parent), and Step 4 meets them.
   * O2 is not built: skip Steps 3 and 5b.  Cases 5, 6, 7 and 10 do not apply to O1 (Step 5a).
   * In order: Step 3 (only if O2 is to be built; case 10 puts it before the first change), P (Step 4), O1 (Step 5a), O2 (Step 5b), each measured against the commit before it.  O2's addition is measured again on O1's commit, with the measuring fields on that tree: the same formula must still give 3 % or more, else O2 is not built, and Step 3 stays as a fix of the timer.

Acceptance: the table of the field medians and the predicted gains in each browser, with the logs, in Surprises.  The measuring patch is in the scratchpad, and the branch has no commit of it.

### Step 3: the chunk time covers the input read (case 10; only if O2 is built)

1. A source test first, in the form of `chunk_times_have_two_decimals` (`xtask/src/e2e/perf.rs`): the worker's chunk timer starts at the end of the line of the chunk before, and chunk 1's just before its first `next_input`.  It fails first on the old form, on an assert.
2. The code: in `separate_chunks`, the timer `tc` is set before the first `next_input`, and set again just after each chunk line.  The line keeps its format, `"chunk {done}/{total}: {:.2} s; {}"`.
3. `cargo run -p xtask -- check --tier commit`, then commit, signed.  This commit is the oldest base of M6's comparisons.
4. Build its parent's page in the base worktree.  Then, in each browser: `STEMYLA_PERF_WEIGHTS=<weights> cargo run -p xtask -- e2e perf --browser <b> --weights <weights> --base-dist <scratch>/m6/base-wt/app/dist --runs 4 --out <scratch>/m6/runs/3-<browser>`.  If the verdict is not "no change", measure the baselines again with `e2e perf --browser <b> --weights <weights> --runs 5 --update-baselines --raise "the chunk time covers the input read"`, in a commit of its own.

Acceptance: the source test failed first, then passes.  The verdicts are in Surprises, and the baselines are new if a verdict was not "no change".

### Step 4: P, two steps in flight (only if P is built)

The design:

* `Pace` keeps at most 2 steps in flight.  For each step, it records and submits the step, and takes its `done_signal` at once, so that signal covers this step and the steps before it only.  Then, when 2 steps are in flight, it awaits the signal of the older one.
* A step's time runs from the later of its submission and the end of the step before, to its own end, as the worker sees the ends.  The step limit (5 s; 60 s for the first step of each kind) checks that time.  `take_errors` and `collect_profile` run after each awaited end, and stop the run as today.
* Before each mask read, `Pace` awaits each step in flight (`drain`).  So the copy of the mask is submitted after the last mask step of its stem, and before any step of the next stem (case 2).  The map waits for no later step, since none was submitted (case 4).
* Calibration in step (case 11): `time_rec` (`roformer-gpu/src/bench.rs`) gets a sliding window: with `STEP_WAITS` at 2 in flight, it submits a step, takes its `done_signal`, and awaits the signal of the step 2 back.  Today a `Waits` value of 2 would give batches of 2 with a wait for both, not a window.  `time_axis_per_chunk` times the steps with the window, and `CALIBRATION` goes to 7.

Tests first, each failing on an assert:

* A pure function gives the order of a list of steps with up to `n` in flight, for example `pace_order(steps, n) -> Vec<Event>`, with `Submit(i)`, `Signal(i)`, `Await(i)` and `Read(stem)`.  With `n` = 2: each `Signal(i)` comes right after `Submit(i)`, and no `Submit(i + 2)` comes before `Await(i)`.  Each `Read(stem)` comes after `Await` of each step submitted before it, and before the first `Submit` of the next stem's steps.  `Pace` follows its order.
* The step time: a pure function gives each step's time from the times of the submissions and the ends, with the case of a step that waited behind the one before.
* The window of `time_rec`: a pure function gives its order of submissions and awaits for a window of 2, as `pace_order` does.  `calibration_waits_after_each_step` changes with `STEP_WAITS`, and `CALIBRATION` is 7.

Then:

1. Before the commit: `e2e selftest --browser <b> --weights <weights>` passes in Safari and Chrome.  `e2e stored-stems --browser <b> --weights <weights>` gives the same hashes as at the base, with `--query attn=vfused` and with `--query attn=freq`, in Chrome and Safari (case 1).  The base hashes come from the base page (Step 3's commit, or M6's base if Step 3 is skipped), in the same browser.
2. `cargo run -p xtask -- check --tier commit`, then commit, signed.
3. The comparison (case 9), in each browser: `e2e perf --browser <b> --weights <weights> --base-dist <the page of the commit before> --runs 4 --out <scratch>/m6/runs/4-<browser>`.  A gain of 3 % or more in one browser, and no loss in either.  Else revert the commit, and give the figures to Surprises (the parent's gain case 4).

Acceptance: the tests failed first, then pass, the two native tests with `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release`.  The self-tests, the hashes and the verdicts are in Surprises.

### Step 5: O, the CPU works while the GPU works (only if O1 or O2 is built)

The notation: `S(i)` submits step i and takes its signal; `W(i)` awaits it.  `M(k)` stands for the 14 mask steps of stem k, run with a window of 2, as Step 4 (or one at a time, as today, when P is not built).  At most 2 steps are in flight at any time.

#### Step 5a: O1, each mask applied under the next stem's first steps

One chunk n, as a list of events, for k = 0 to 5:

1. `M(k)` to its last step (the shared steps first, for k = 0); `W` of each step in flight.
2. `read_start` of the mask of stem k: the copy and `map_async`, before any later submission (cases 2 and 4).
3. For k = 0 to 4: `S` of the first 2 steps of `M(k + 1)`.  For k = 5: nothing.
4. `finish` of the read of stem k: its mask goes into wasm memory.  The CPU then holds this mask only, since the mask of stem k - 1 dropped in its apply (case 3).
5. `apply_masks` and `accept_stem` of stem k, while the 2 submitted steps run.  The mask drops.  For k = 5, no step is in flight, as today.
6. For k = 0 to 4: `M(k + 1)` goes on from its third step.

O1 changes no input read, no `Demixer`, no `working_set` and no time-axis step.  So cases 5, 6, 7, 10 and 11 do not apply to it, and the CPU holds one mask, as today.  Its tests first, each failing on an assert:

* Cases 2 and 4: a pure order function gives the list above.  Its test: each `read_start` of stem k comes after the last `W` of `M(k)`, and before the first `S` of `M(k + 1)`.  No `S` comes between a copy and its `map_async`.  At most 2 steps are in flight at each event.  The native tests of the two methods cover the GPU side.
* Case 3: the wasm test's fake GPU half counts the masks that the CPU holds, and fails over 1.  `GpuHalf` and the fake change with the order, so the fake records it.

Then, as Step 4, items 1 to 3: the self-tests, the hashes of case 1, `check --tier commit`, the commit, and the comparison against the commit before, with `--out <scratch>/m6/runs/5a-<browser>`.

#### Step 5b: O2, the next chunk's input under the last stem (only if O2 is built)

O2 adds to O1.  For k = 5 of chunk n, item 1 runs `M(5)` with its CPU pieces in its windows:

1. After apply 4 (under the first 2 steps of `M(5)`): the read of chunk n + 1's input (`read_ahead`), under the next window of 2 steps of `M(5)`.
2. `analyse` of chunk n + 1 and its upload to `split_in` (`set_input`), under the next window of 2 steps of `M(5)`.  Chunk n's `Split` was submitted long before, and the queue keeps the order, so chunk n's `Split` read its own input.  The features drop after the upload.
3. `M(5)` to its last step; the read of mask 5 as O1 items 2 and 4.  Then `S` of `Split` and of the first `Time` step of chunk n + 1.
4. Apply 5 of chunk n under those 2 steps.  Then `end_chunk` and the emit of chunk n, and chunk n + 1 goes on from its third step.  `separate_chunks` takes no `next_input` for chunk n + 1.

The edge cases, each written into the code's doc and a test:

* The last chunk has no next input: items 1 to 3 submit nothing of a next chunk, and apply 5 runs with no step in flight, as in O1.
* `GpuNetwork::upload` makes a new `Job` when the frame count changes.  With the read-ahead, that would drop chunk n's buffers while its steps and copy are in flight.  The frame count stays fixed, since `next_input` and `read_ahead` pad each input to the chunk's size.  The upload asserts that the frame count equals the `Job`'s while a chunk is in flight.
* The first `Time` step of chunk n + 1 runs under apply 5, with a window of 2.  That is 1 of 192 time-axis steps of a chunk, and calibration times both policies' first steps alike.  So `STEP_WAITS` does not change for O2; Step 8's bias check is the guard (case 11).

The parts of the design that the cases need:

* Case 5: `Demixer` gets `read_ahead()`, a read of the next chunk's input that does not move `pos`.  `accept_stem` and `end_chunk` still use `pos`.  A test in `roformer/src/demix.rs`: a song separated with the read ahead gives each stem the same samples, bit for bit, as with `next_input` alone, and the read gives the same input as `next_input` after `end_chunk`.  It fails first with a read that moves `pos`.
* Cases 2 and 4: O1's order function takes the new events, and its test also checks that the upload comes after chunk n's `Split`, and that no step of chunk n + 1 is submitted before the `read_start` of mask 5.
* Case 6: `roformer::separate::working_set` and the phases of `a_chunk_of_the_default_length_holds_its_working_set` follow the list, with each moment's parts:
  * the read-ahead and STFT moment (items 1 and 2): this chunk's spectra, the next chunk's span bytes and input, its spectra, its features and the band buffer of `set_input`;
  * the moment of stem 5's inverse STFT (item 4): today's parts of that moment and the next chunk's spectra.
  The test measures each phase's peak against its estimate, as today.  A part listed where it is not live would raise check 3's budget by up to 18.9 MB, more than the smallest lost-cut rise (9.44 MB), so check 3 would stop catching a lost cut.
* Case 7: each new buffer has a fixed size from chunk to chunk: the next chunk's input is padded as `next_input` pads it.  `memory-budget`'s flat check covers it.
* The memory cost: the next chunk's spectra (18.9 MB) at the peak of the masks phase, and the next input and features at the STFT moment.  The tab total with `freq` in Chrome would be about 799 + 19 = 818 MB, under `TAB_BUDGET_MB` (943 MB); Step 7 measures it.

Step 3 landed before the first change (case 10).  Then, as Step 4, items 1 to 3, with `--out <scratch>/m6/runs/5b-<browser>`.

#### The step limit during a CPU piece (Steps 5a and 5b)

While a CPU piece runs, the worker sees no end, since the piece is synchronous.  So the ends of the steps in flight show after the piece.  For each step, the step limit takes off the part of the piece that lies inside that step's time, as Step 4 defines the step time.  For the second step, whose time starts at the end of the first as the worker sees it, that part is 0.  `take_errors` runs after each awaited end, after the piece, and before the next mask is copied or applied.

Acceptance of Step 5: the tests failed first, then pass, the native ones with `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release`.  The self-tests, the hashes and the verdicts are in Surprises.

### Step 6: the landing rule and the baselines

* Each built change (P, O1, O2) lands only with its gain of 3 % or more in Chrome or Safari, and no loss in either, by its comparison against the commit before it (case 9).  A last comparison of M6's head against the oldest base (Step 3's commit, or M6's base if Step 3 is skipped) gives the whole gain.
* For each browser with a gain: `e2e perf --browser <b> --weights <weights> --runs 5 --update-baselines`, then `check --tier commit` and a commit of its own.  A lower bound needs no `--raise`.
* The perf gate passes in both browsers at the head.

### Step 7: memory (case 8, and the headroom)

1. `e2e memory-budget --browser <b> --weights <weights>` 3 times in each browser at the head, with the kernels that each browser chooses, and 3 times in Chrome with `--query attn=freq`.  It passes its four checks each time.  The tab total stays at most `TAB_BUDGET_MB` (MEM-4).
2. The headroom, by the parent's cases 1 to 4: if each growth matches the new peak with the margin in the band of the doc, `WORKER_HEADROOM_MB` stays.  Else the wasm test passes and a probe of the live heap at each phase finds no held buffer (case 2).  Compute the smallest lost-cut rise again from the new `working_set` (case 3).  Set the constant midway between the noise and that rise, with its doc, the figures of `the_worker_growth_must_fit_the_working_set_budget` and a Decision log line, in one commit (case 4).
3. If `memory-budget` fails, M1's Step 6 names the next cuts, in order of cost.

### Step 8: calibration at the head (case 11)

1. The check of "Each calibration choice is not slower" on the head page alone (the parent's case 5 of that goal), as `docs/references/local-browser-testing.md` says.
2. The bias of `freq` against `fused`, as Step 5, item 6 of `plans/completed/gpu-speed-m5.md` does, in each browser: forced runs of `fused` and `freq` with the pins (4 each, in the order `fused`, `freq`, `freq`, `fused`), for `D` and its noise.  Then 12 unforced calibrations whose `kernels:` line gives the pins.  The bias is the mean of calibration's (split - fused) minus `D`.  Its size stays under half of `|D|`, or under the noise of `D` where `|D|` is smaller.  Else it is a defect of M6: fix the timing (case 11), or revert P.
3. Each run's logs go in `<scratch>/m6/runs/8-<browser>-<n>/` (`--out`).

### Step 9: the checks before review round 1

* `cargo run -p xtask -- check --tier push`, with `$STEMYLA_PERF_WEIGHTS` set and `$CHROMEDRIVER` set to the driver of Chrome for Testing 157 (mac-arm64), passes, with both perf gates run.  Progress gives both verdicts.  A skip line is not a pass (VERIFY-1).
* `cargo build -p xtask` passes.
* `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passes.
* Docs in the commits of the changes: `docs/ARCHITECTURE.md` ("Running it on the GPU": the pacing and the order of a chunk), the docs of `Pace`, `chunk_stems` and `working_set`, and TD-11's wasm-test bullet.
* `<scratch>/m6/parent-m6.diff` for the orchestrator, with `git apply --check` passing on HEAD.  It holds a Surprises entry with the tables of Steps 1, 2 and 4 to 8, and Decision log lines for each build decision.

### Step 10: no build (only if Step 2 builds neither)

1. `STEMYLA_PERF_WEIGHTS=... CHROMEDRIVER=... cargo run -p xtask -- check --tier push` (with the paths of Step 9) passes, with both perf gates run, since Step 1's commit is code.  Progress gives both verdicts.
2. `<scratch>/m6/parent-m6.diff` for the orchestrator: a Surprises entry with Step 1's counts and Step 2's table, its Outcomes text, and a Decision log line.  The Surprises entry names the measuring patch (the parent).
3. The parent's headroom case 1 ("measured again at the end of M6") does not apply: Step 1 changes only test code, so the worker's heap does not change.  The Decision log gives this reason.
4. The review rounds, by item 1 of "Decisions for the maintainer".  Then the plan moves to `completed/`.

## Validation and acceptance

* Step 1: the spy tests failed first and pass; the truncate guard failed without the truncate; whole Safari and Chrome `web-test` runs pass; a whole Safari `web-test` leaves fewer swap files, in Surprises; TD-103 no longer names the two helpers.
* Step 2: the table of each browser, with the predicted gain of P and of O, and the decision for each change, in Surprises and the Decision log.
* For each built change: its tests failed first, then pass; `e2e stored-stems` gives the base's hashes with `attn=vfused` and `attn=freq` in each browser (case 1); its comparison shows a gain of 3 % or more in one browser and no loss in either (case 9).
* Case 10 (if O2 is built): the source test failed first, and Step 3's comparison is in Surprises.
* Case 8: `memory-budget` passes 3 times in each browser, and 3 times with `attn=freq` in Chrome (Step 7).
* Case 11: the choice check passes on the head page, and the bias of `freq` against `fused` is in its limit, in each browser (Step 8).
* `cargo run -p xtask -- check --tier push` passes, with both gates run.
* The review rounds (`/review-rounds 2543abc`) end by the stop rule of decision 0024.

## Idempotence and recovery

Each step is one commit or none, on the branch.  A session that stops reads Progress and Surprises, and goes on at the next open step.  Step 2 measures only; its patch and worktree stay in the scratchpad.  A browser run that a locked screen or a time limit stops runs again.  A comparison that a browser update splits runs again in full, and Surprises records both versions.  A revert of Step 4's or Step 5's commit gives back the old order; a `CALIBRATION` bump makes each browser calibrate again.  `git worktree remove` undoes each worktree.

## Decisions for the maintainer

1. If Step 2 builds neither change, does Step 1's commit get review rounds?  The parent says that M6 then has no review rounds, and also puts the change of the test helpers in M6's first commit.  Options: a) no review rounds, as the parent says; b) the review rounds on Step 1's commit alone.  Recommendation: b), since the commit is code and VERIFY-1 asks for a check of it; the rounds are small.  This is a conflict in the parent, so the orchestrator decides.  Answer (2026-11-15, the orchestrator, parent `ced888f`): b).  The parent's "no review rounds" meant a milestone that lands no code.
2. Open only if Step 2 finds O1 and O2 under 3 % at a depth of 2 in both browsers, and a bound of 3 % or more at an unlimited depth in one.  May O queue more than 2 steps while a CPU piece runs?  Options: a) no, and O is not built; b) up to the steps that cover the piece (for example the 14 mask steps of a stem), with the step limit as Step 5 says.  Recommendation: from Step 2's figures.  The cost of b): the GPU holds up to about 0.1 to 0.2 s of queued work, so other work, the screen included, waits longer for the GPU (the doc of `Pace`).  The maintainer's answer of 2026-11-12 allows 2 in flight.  Step 2 hit the condition: O1 at an unlimited depth gave 3.20 % in Chrome and 2.05 % in Safari, an upper bound.  Answer (2026-11-15, the maintainer, through the orchestrator): a).  At most 2 steps are in flight, and O1 is not built.

## Progress

* [x] 2026-11-15: the plan written, at `2543abc`.
* [x] 2026-11-15: plan review round 1: 1 blocker, 7 should-fix and 2 nits, each applied.
* [x] 2026-11-15: plan review round 2: 1 blocker, 4 should-fix and 2 nits, each applied.
* [x] 2026-11-15: Step 1: the test helpers use sync access handles.  The spy tests `plant_helpers_open_no_write_stream` failed first in Chrome, with 2 calls (worker) and 1 call (decoder) of `createWritable` (`<scratch>/m6/step1-fail-spy.log`).  The guard `plant_helpers_truncate_a_longer_file` failed with helpers that had no `truncate(0)`: it read `[2, 2, 2, 1, 1, 1, 1, 1]`, not `[2, 2, 2]`, in both crates (`<scratch>/m6/step1-fail-truncate.log`).  Then all four tests passed (`<scratch>/m6/step1-pass.log`).  The whole Safari and Chrome `web-test` runs after the change passed (Surprises).
* [x] 2026-11-15: Step 2: the measurement at M6's base, with `<scratch>/m6/measure.patch` on `5ad3f82`.  The native tests of `done_signal` and `read_start` failed first with forms that ask WebGPU only at the await, on their flag asserts (`<scratch>/m6/step2-fail-native.log`).  Then `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passed with the patch (`<scratch>/m6/step2-gpu-suite.log`).  P is to be built; O1 and O2 are not, at a depth of 2.  O1's bound at an unlimited depth is 3.20 % in Chrome, so item 2 of "Decisions for the maintainer" is open: a checkpoint for the orchestrator (Surprises, Decision log).
* [x] 2026-11-15: Step 3 skipped: O2 is not built (Step 2).
* [x] 2026-11-15: Step 4: P, 2 steps in flight, in `406e819`.  Its comparison against M6's base page shows a gain of 3 % or more in Chrome and in Safari (Surprises).  Done before the commit:
  * The tests failed first, each on an assert (`<scratch>/m6/step4-fail-gpu.log`): `a_done_signal_covers_the_work_before_its_call` and `a_started_read_gives_the_contents_at_its_start` on their flags, with forms that ask WebGPU only at the await.  `the_pace_keeps_2_steps_in_flight_and_drains_before_each_read` found 1 step in flight at most, with a window that ignored its depth.  `a_step_that_waited_behind_the_step_before_is_not_charged_for_it` failed with the old step time.  `calibration_keeps_the_steps_in_flight_of_the_separation` read `InFlight(1)`, and `choice_keys_name_the_format` read `CALIBRATION` 6.  `a_window_of_1_awaits_each_step_at_once` passed before and after: it pins today's order.
  * `the_worker_paces_as_calibration_times` failed on the worker of M6's base (no window, `<scratch>/m6/step4-fail-source.log`), and on a worker with no drain before the read of a mask (`<scratch>/m6/step4-fail-source-drain.log`).
  * `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passed (`<scratch>/m6/step4-gpu-suite.log`).
  * `e2e selftest` passed in Chrome and Safari, with the default query (`mm=s64`, stages) and with the full chunk and the pins (`selftest=1&mm=<pins>&attn=vfused`) (`<scratch>/m6/runs/4-selftest-*`).
  * `e2e stored-stems` gave the hashes of M6's base page in Chrome and Safari, with `attn=vfused` and with `attn=freq` (`<scratch>/m6/runs/4-stems-*`; Surprises).
* [x] 2026-11-15: Step 5a skipped: O1 is not built (the maintainer's answer to item 2).
* [x] 2026-11-15: Step 5b skipped: O2 is not built (Step 2).
* [x] 2026-11-15: Step 6: P lands (Step 4's comparisons, and Safari's again with `--runs 8`).  The baselines: Chrome 3.59 s to 3.43 s (`d3408f5`), Safari 5.57 s to 5.37 s (`cf2e935`).  The perf gates of Step 9 are in Progress there.
* [x] 2026-11-15: Step 7: `memory-budget` passed 3 times in each browser and 3 times in Chrome with `attn=freq`.  `WORKER_HEADROOM_MB` stays (Surprises).
* [x] 2026-11-15: Step 8: calibration at the head.  The choice check passes in both browsers.  The bias of `freq` against `fused` holds in Chrome.  In Safari it fails the limit on the planned set of 4 forced runs.  The orchestrator accepts it (i1-3 of the intent review, the parent's Decision log).
* [x] 2026-11-15: Step 9: the checks before review round 1, at `cf2e935` with this plan's change (`<scratch>/m6/step9-push.log`).
  * `check --tier push` passed 15 lines, with both perf gates.  Chrome's gate passed: 3.45 s against the bound 3.77 s (baseline 3.43 s), `matmul v64,v64, attention vfused`.  Safari's gate passed: 5.40 s against 5.91 s (baseline 5.37 s), `matmul v64k8,v64k8, attention vfused`.  Its line `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passed.
  * `cargo build -p xtask` passed.
  * The docs: `docs/ARCHITECTURE.md` ("Running it on the GPU" and "Short steps") and the doc of `Pace` changed in `406e819`.  P changes neither `chunk_stems` nor `working_set`, and TD-11's wasm-test bullet (`chunk_stems` with a fake GPU half) stays true.
  * `<scratch>/m6/parent-m6.diff` for the orchestrator; `git apply --check` passes on HEAD.  No change of `CLAUDE.md` or `.claude/` is needed.
* [x] 2026-11-15: Step 10 skipped: P is built.
* [x] 2026-11-15: the fixes of review round 1 (`<scratch>/m6/r1/fixes.txt`).  Each new or changed test failed first on an assert, against a mutant (`<scratch>/m6/r1/fail-*.log`):
  * 1-1: `Pace` and `time_rec` run their steps through `Window::step` and `Window::drain` (a `Steps` trait).  `pace_order` drives them too.  `the_pace_keeps_2_steps_in_flight_and_drains_before_each_read` failed against four mutants.  Two are a dropped older step (`fail-1-1-a-drop-older.log`) and no await in the round (`fail-1-1-b-no-await-in-round.log`).  The others are an await of each step at once (`fail-1-1-c-await-at-once.log`) and a signal before the submission (`fail-1-1-d-signal-before-submit.log`).
  * 1-1: `StepClock` keeps the end of the last step.  `the_step_clock_charges_each_step_from_the_end_of_the_step_before` failed without its update (`fail-1-1-e-no-last-end.log`).
  * 1-1: `the_worker_paces_as_calibration_times` pins that `Pace`, `OnGpu`, `Checks` and `time_rec` use the shared parts.  It failed against a `Pace::run` that drops the older step, a step time without `StepClock` and a signal in `OnGpu::submit`.  It also failed against `time_rec` with a window of 1 or no `Window::step` (`fail-1-1-f-*.log` to `fail-1-1-j-*.log`).
  * 1-9: the error text names the newer step in flight too.  `each_end_tells_whether_a_newer_step_is_in_flight` failed against an end that always tells true (`fail-1-9-newer-always-true.log`).  That test drives only `Window`, not the worker.  Since review round 2, `a_rejection_names_the_newer_step_only_when_one_is_in_flight` and `the_worker_paces_as_calibration_times` cover the worker side.
  * `time_rec_counts_the_steps_of_a_round` failed against `-` to `+` in the round loop of `Covering` and of `InFlight`, and `/` to `*`.  It also failed against a count of 2 for each step (`fail-1-tr-*.log`).
  * The four signal tests failed against `Gpu::done` as `()`, `is_mapped` as `true`, `is_done` as `true` and `Signal::wait` as `()` (`fail-1-13-*.log`, `fail-1-16-*.log`, `fail-1-17-*.log`, `fail-1-24-*.log`).
  * 1-2 to 1-8 and 1-10: the docs and the test helpers.
  * `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passed.  `e2e selftest` passed in Chrome and Safari.  `e2e stored-stems --query attn=vfused` in Chrome gave the six hashes of M6's base (`<scratch>/m6/r1/stems-chrome-vfused.hashes`).  The worker and decoder tests passed in Safari, and the plant tests in Chrome (`<scratch>/m6/r1/web-test-*.log`).
* [x] 2026-11-15: the fixes of review round 2 (`<scratch>/m6/r2/fixes.txt`).  Each new or changed test failed first on an assert, against a mutant (`<scratch>/m6/r2/fail-*.log`):
  * 2-1: the text of a rejection is `roformer_gpu::rejected_text`.  `a_rejection_names_the_newer_step_only_when_one_is_in_flight` failed against `if !newer` in it (`fail-2-1-b-not-newer.log`).
  * 2-1: `the_worker_paces_as_calibration_times` pins that `OnGpu::end` gives `newer` to `Checks::ended`, and `Checks::ended` to `rejected_text`.  It failed against `false` in each place (`fail-2-1-a-end-false.log`, `fail-2-1-c-ended-false.log`).
  * 2-2: `time_rec_counts_the_steps_of_a_round` counts the calls of `record`.  It and the source test failed against an empty `Timed::submit` (`fail-2-2-a-submit-noop.log`).
  * 2-3: the source test pins the whole bodies of `Pace::run`, `Pace::drain` and `time_rec`'s `InFlight` arm, and the drain of `GpuNetwork::mask`.  It failed against a drain after each step of `Pace::run` (`fail-2-3-a-run-drains.log`).  It also failed against one in `time_rec`'s round loop (`fail-2-3-b-time-rec-drains.log`), and in the step loop of `mask` (`fail-2-3-c-mask-drains-in-loop.log`).
  * 2-4 to 2-7: the docs, TD-102 and the comment before the drain of the mask.
  * `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passed.  The worker makes the same error text through `rejected_text`, and changes a comment, so no browser run.
* [x] 2026-11-15: the fixes of review round 3 (`<scratch>/m6/r3/fixes.txt`).  Each new or changed test failed first on an assert, against a mutant (`<scratch>/m6/r3/fail-*.log`):
  * 3-1: `an_error_at_the_end_of_a_step_stops_the_run` drives `Window` with steps whose end fails.  It failed against a `Window::step` that drops the error (`fail-3-1-a-step-drops-err.log`), and the same in `Window::drain` (`fail-3-1-b-drain-drops-err.log`).
  * 3-1: the checks at the end of a step are `roformer_gpu::step_check`, with the same texts.  `a_step_check_gives_the_errors_first_then_the_limit` failed against `errors.get(1)` (`fail-3-1-d-second-error.log`).  It also failed against `dt > limit * 1e9` (`fail-3-1-e-limit-times-1e9.log`), and against no limit of 60 s for the first step of a kind (`fail-3-1-f-no-first-step-limit.log`).
  * 3-1: `the_worker_paces_as_calibration_times` pins that `Checks::ended` returns `step_check` with `gpu.take_errors()`.  It failed against no errors (`fail-3-1-c-no-take-errors.log`) and against a dropped result (`fail-3-1-g-ended-drops-result.log`).
  * 3-5: the source test pins the whole body of `Pace::all`, and a drain before each read of `selftest`.  It failed against `Pace::all` with no drain (`fail-3-5-a-all-no-drain.log`), and against the removal of each of the 4 drains of `selftest` (`fail-3-5-b-selftest-no-drain-*.log`).
  * 3-2: `step_check` removes the shadow of `first`.  3-3, 3-4 and 3-6: the docs, the comment before the drain of the mask, and this plan.
  * `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passed.  `e2e selftest` passed in Chrome and Safari (`<scratch>/m6/r3/selftest-*.log`).
* [x] 2026-11-15: the fixes of review round 4.  `a_step_check_gives_the_errors_first_then_the_limit` gained a step on the limit and a limit of 120 s.  It failed against `dt >= limit` (4-2) and against a fixed 60 s for a first step (4-1) (`<scratch>/m6/r4/fail-4-*.log`).
* [x] 2026-11-15: review round 5 found 1 nit and no should-fix, so it is the last round (decision 0024).  Five rounds: 34 agents, 2,166,247 tokens, 52.6 wall minutes, 0 interventions.  Rounds 4 and 5 fixed 2 findings, both cases of the test of `step_check`.  The log is `docs/reviews/gpu-speed-2026-11-12-m6.md`.
* [x] 2026-11-15: finding i1-2 of the intent review of M6: `WORKER_HEADROOM_MB` follows M6's growth of 55 MB, and goes from -17 to -19 MB (Surprises, Decision log).  `the_worker_growth_must_fit_the_working_set_budget` failed first with -17 MB, on its new assert: a lost cut 3 less the taken noise (62.28 MB) passed the budget of 63.16 MB (`<scratch>/m6/i1-2/fail-headroom-17.log`).  With -19 MB it passed.  `memory-budget` passed 3 times in each browser at the head.
* [x] 2026-11-15: review rounds 6 to 9, after the intent review.  Round 6 reviewed the fix of i1-2 and the close-out.  Rounds 6 to 8 each fixed one significant doc finding: the verdict on Safari's bias (i1-3), and the cause and the fix of TD-106.  Round 9 found 2 nits, both repeats, so it is the last round (decision 0024).
* [x] 2026-11-15: all nine rounds: 48 agents, 2,767,767 tokens, 68.7 wall minutes, 0 interventions.

## Surprises and discoveries

* Step 1, 2026-11-15, Safari 27.2 and Chrome 157 on the M2 Pro.  A whole Safari `web-test` at M6's base left 1,320 swap files, 1,697.2 MB, and 9 blob folders (`<scratch>/m6/step1-safari-before.log`).  After the change, it left 1,149 swap files, 1,549.5 MB, and 9 blob folders (`<scratch>/m6/step1-safari-after.log`).  So the change saves 171 swap files, 147.7 MB, for each whole Safari run.  Each crate passed in both runs: the stem worker 111 tests before and 113 after, the decode worker 13 before and 15 after.  The whole Chrome `web-test` after the change passed in each crate (`<scratch>/m6/step1-chrome-after.log`).  The tests that read Safari's `lastModified` after a write in place (`a_mix_file_written_again_after_the_last_read_ends_with_unstored`, `the_end_check_sees_a_mix_file_written_again_at_its_size`) passed in Safari.

* Step 2, 2026-11-15, Chrome 157.0.8103.51 and Safari 27.2 on the M2 Pro, at M6's base with the measuring patch, no `profile=1`.  Each field is the median over chunks 2 to 11 of a run, then the median of 4 runs (`<scratch>/m6/runs/2-<browser>-fields/`).  The full table: `<scratch>/m6/step2-table.md`.  The patch: `<scratch>/m6/measure.patch`; the branch has no commit of it.
  * The pins: Chrome's unforced run chose `v64,v64` (`<scratch>/m6/runs/2-chrome-unforced/`), as planned.  Safari's unforced run chose `v64k8,v64k8`, not `v64k8,v64` (`<scratch>/m6/runs/2-safari-unforced/`).  So Safari's runs used `mm=v64k8,v64k8`, by the rule of Step 2.  Its small products were near: `v64` 1.18 s, `v64k8` 1.16 s.

  | Field (median) | Chrome | Safari |
  | - | - | - |
  | chunk | 3.630 s | 5.603 s |
  | input | 5.2 ms | 6.0 ms |
  | paced (470 steps) | 3.425 s | 5.383 s |
  | shared | 3.223 s | 5.087 s |
  | stem0 to stem5, each | 33.3 to 33.6 ms | 48.8 to 49.8 ms |
  | reads (6) | 18.7 ms | 42.7 ms |
  | stft | 32.0 ms | 28.3 ms |
  | apply0 to apply5, each | 23.2 to 23.4 ms | 23.0 ms |
  | first21 to first25, each | 6.0 ms | 9.0 to 10.0 ms |
  | m5w2, m5w3 | 4.0, 4.9 ms | 6.0, 7.0 ms |
  | head2 | 15.9 ms | 22.0 ms |
  | rest | 12.1 ms | 11.0 ms |
  | inflight | 1 | 1 |

  | Predicted gain | Chrome | Safari |
  | - | - | - |
  | P: verdict and mean `r` (`<scratch>/m6/runs/2-<browser>-pace/`) | gain, 3 % or more; -4.37 % (blocks -4.55 %, -4.19 %) | gain, 3 % or more; -3.22 % (blocks -3.17 %, -3.26 %; block 1, -2.34 %, spoiled at 1.45 %) |
  | O's bound | 4.86 % | 3.07 % |
  | O1, depth 2 | 0.83 % | 0.82 % |
  | O2's addition, depth 2 | 0.68 % | 0.62 % |
  | O1's bound, unlimited depth | 3.20 % | 2.05 % |
  | O2's addition's bound, unlimited depth | 1.55 % | 1.02 % |
  | CPU share | 5.13 % | 3.15 % |
  | The parent's estimate of the waits, from `profile=1` | 7.5 % (bound 10 %) | 0.7 % (bound 3.7 %) |

  * The head runs of P's prototype read `inflight 2` in each chunk, and the base runs `inflight 1`.
  * P gains 3.2 % in Safari, against the parent's estimate of 0.7 %.  The waits cost more in Safari than the gaps of the profile showed.
  * The CPU pieces are small: the STFT with its upload is 28 to 32 ms, not the 0.25 to 0.3 s of the parent's Context.  Each stem's apply with its inverse STFT is 23 ms.  So the CPU work of a chunk is about 0.17 s.  Each piece is larger than the 2 steps under it (6 to 16 ms), so the steps bound O at a depth of 2.
  * O2's addition at an unlimited depth takes `min(input + stft, stem5)`, since the input read and the STFT share the span of stem 5.  A min for each gives 1.66 % in Chrome and 1.02 % in Safari.  Either way it is under 3 %.  So O2 is not built at any depth, and Step 3 is skipped.
  * Safari's worker clock gives whole milliseconds, so its fields are on a grid of 1 ms.  O's bound in Safari (3.07 %) is near 3 %, but Chrome's 4.86 % already passes the bound, so no decision hangs on it.

* 2026-11-15: the maintainer chose a) for item 2 of "Decisions for the maintainer": at most 2 steps in flight.  So O1 is not built, by its figures of Step 2 (0.83 % in Chrome, 0.82 % in Safari at a depth of 2).  O2 is not built at any depth (Step 2).  The steps left after Step 4 are 6, 7, 8 and 9.
* Step 4, 2026-11-15, `e2e stored-stems` (`<scratch>/m6/runs/4-stems-*`): the base page is M6's base, `5ad3f82`, and the head page is the tree of Step 4.  The scenario runs `mm=s64` on both sides.  The six SHA-256 of each pair are the same, in Chrome and in Safari:
  * `attn=vfused`: bass `04b942ca...`, drums `bcfb3645...`, other `771b34e0...`, vocals `f1b79d87...`, guitar `5f9f7bd6...`, piano `3166e0de...`.
  * `attn=freq`: the same six in each browser, base and head (`<scratch>/m6/runs/4-stems-<browser>-freq-<side>.hashes`).
  * The scenario took `mm=s64` although the query gave each browser's pins.  So the pins in the query changed nothing; both sides ran the same kernels.

* Step 4, 2026-11-15, the comparison of item 3 (`<scratch>/m6/runs/4-<browser>/`), with no query.  The base page is M6's base (`5ad3f82`), whose page is the page of `0c2e554`, the commit before Step 4: Steps 1 and 2 changed only tests and docs.
  * Chrome 157: gain, 3 % or more.  Blocks: -4.18 %, -4.70 % (spoiled, 1.67 %), -4.45 %; mean `r` of the unspoiled blocks -4.32 %.  Each run chose `v64,v64` and `vfused`.  The head's chunk lower quartile was 3.43 to 3.47 s, against 3.59 to 3.65 s.
  * Safari 27.2: gain, 3 % or more, by the mean `r` of 4 blocks, since the kernels differ: -2.80 %, -4.10 %, -3.93 %, -3.67 %; mean -3.63 %.  The base chose `v64k8,v64k8` 5 times and `v64k8,v64` 3 times, the head `v64k8,v64` 5 times and `v64k8,v64k8` 3 times (the near choice of the small products).  The head's chunk lower quartile was 5.37 to 5.39 s, against 5.54 to 5.63 s.
  * So P lands: a gain of 3 % or more in both browsers, and no loss (case 9).  Step 2's prototype predicted 4.37 % and 3.22 %.

* Step 6, 2026-11-15:
  * Safari's comparison of `406e819` against `5ad3f82`'s page, again with `--runs 8`, since Safari has a near matmul choice (`<scratch>/m6/runs/6-safari-runs8/`): gain, 3 % or more.  Blocks -3.59 %, -3.76 %, -3.68 %, -3.76 %; mean `r` -3.70 %.  Each side chose `v64k8,v64k8` 5 times and `v64k8,v64` 3 times.  With `--runs 4` (Step 4): gain, -3.63 %.  Both comparisons run 4 blocks, since the kernels differ.
  * Only P is built, so the whole gain of M6's head against M6's base is Step 4's comparison: 4.32 % in Chrome, and 3.63 % and 3.70 % in Safari.  It did not run again.
  * The baselines, from 5 unforced runs each (`<scratch>/m6/runs/6-<browser>-baseline/`): Chrome 3.43 s (was 3.59 s), bound 3.77 s (was 3.95 s); Safari 5.37 s (was 5.57 s), bound 5.91 s (was 6.13 s).  Each is a lower bound, so no `--raise`.  One Chrome run chose `v64,i64` (3.52 s), and two Safari runs `v64k8,v64`.
* Case 1 (Step 4): `e2e stored-stems` always runs `mm=s64`.  So its hashes check P's order with that kernel.  P changes no kernel; the kernels' own checks are the start-up checks and `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release`.
* Step 7, 2026-11-15, at the head (`cf2e935`, the page of `406e819`), with the kernels that each browser chooses (`<scratch>/m6/runs/7-<case>-<n>/`): each of the 9 runs of `memory-budget` passed its four checks.

  | Case | Tab total (margin under 943 MB) | Worker growth (bound) | 4-minute peak over 1-minute |
  | - | - | - | - |
  | Chrome, 3 runs | 756 MB (187 MB) each | 55 MB (63.16 MB) each song | 0 MB |
  | Safari, 3 runs | 758 MB (185 MB) each | 55 MB (63.16 MB) each song | 0 MB |
  | Chrome, `attn=freq`, 3 runs | 799 MB (144 MB) each | 55 MB (63.16 MB) each song | 0 MB |

  * The tab totals are M5's (756, 758 and 799 MB): P adds no buffer.
  * The worker grew 55 MB in each of the 18 separations, not M1's 57 MB.  So the measured headroom is -25.16 MB, with a spread of 0, and the margin under the bound is 8.16 MB.  It stays inside the band of `WORKER_HEADROOM_MB`'s doc: above the taken noise (2.16 MB), and a lost cut 3 (55 + 9.44 = 64.44 MB) still fails the check.  So the constant stays (Step 7, item 2).  P adds no CPU buffer of a chunk's size.  This run did not measure the growth at M6's base, so the cause of the 2 MB is not known.  Finding i1-2 of the intent review changed the constant later (the entry of i1-2, below).

* Step 8, 2026-11-15, the head page (`406e819`), `CALIBRATION` 7.  The calibrations: each unforced run of the head page in Steps 4, 6 and 8 (`<scratch>/m6/runs/4-<browser>/perf-*-head-*`, `6-<browser>-baseline/`, `6-safari-runs8/perf-safari-head-*`, `8-<browser>-calib/`; tables `<scratch>/m6/runs/8-<browser>-calib.table`).
  * The pins, the matmul policy chosen most often: Chrome `v64,v64` (12 of 14 calibrations; also `v64,i64` and `v64,v64k8` once each).  Safari `v64k8,v64k8` (14 of 24; `v64k8,v64` 10).
  * Choosable policies: `vfused` alone, in both browsers.  Each calibration chose it.  The faster fused kernel over split: Chrome 0.710 (sd 0.003), Safari 0.690 (sd 0.002), far from the flip point 1.0.  `vfused` over `fused`: Chrome 0.638 (sd 0.005), Safari 0.691 (sd 0.002), far from 1.
  * The choice check (`<scratch>/m6/runs/8-<browser>-choice-vfused-<q>/`), with the pins, the head page on both sides, the head `attn=vfused`:

  | Pair | Chrome | Safari |
  | - | - | - |
  | `vfused` against `fused` | gain, 3 % or more: -23.76 %, -24.23 % | gain, 3 % or more: -21.35 %, -21.17 % |
  | `vfused` against `freq` | gain, 3 % or more: -19.51 %, -19.53 % | gain, 3 % or more: -21.14 %, -21.29 % |

  So the choosable policy is not slower than another: the check passes in both browsers.
  * The bias of `freq` against `fused` (Step 8, item 2).  Forced runs with the pins, in the order `fused`, `freq`, `freq`, `fused` (`<scratch>/m6/runs/8-<browser>-bias-<n>-<policy>/`); chunk lower quartiles:

  | Figure | Chrome | Safari, set 1 | Safari, set 2 | Safari, 8 runs |
  | - | - | - | - | - |
  | `fused` | 4.54, 4.53 s | 6.82, 6.83 s | 7.04, 6.84 s | median 6.835 s |
  | `freq` | 4.27, 4.27 s | 6.85, 6.87 s | 6.97, 6.86 s | median 6.865 s |
  | `D`, its noise | -0.265 s, 0.010 s | +0.035 s, 0.020 s | -0.025 s, 0.200 s | +0.030 s, 0.220 s |
  | Calibration's (split - fused), mean of the pinned runs | -0.309 s (12) | +0.002 s (14) | +0.002 s | +0.002 s |
  | The bias, and its limit | -0.044 s, under 0.133 s: holds | -0.033 s, over 0.0175 s: fails | +0.027 s, under 0.200 s: holds | -0.028 s, under 0.220 s: holds |

  * In Safari, P made `freq` and `fused` almost equal: `D` is 0.5 % of a chunk, under `T` (1 %), against about 0.3 s at M5.  The first set's `D` (+0.035 s) changed its sign in the second set (-0.025 s).  So `D` in Safari is under its noise, and a limit of half of `D` (0.0175 s, 0.26 % of a chunk) is under what 2 runs of a policy can measure.
  * In Chrome, `freq` is now faster than `fused` in the separation (`D` -0.265 s), and calibration sees the same (-0.309 s).  Before P, `freq` paid more waits (744 time-axis steps against 192).

* Finding i1-2 of the intent review, 2026-11-15, Chrome 157 and Safari 27.2 on the M2 Pro, at the default chunk with f16 weights (`<scratch>/m6/i1-2/runs/<side>-<browser>-<n>/` and `.stdout`).
  * M6's base page (`5ad3f82`, whose page equals `2543abc`'s), 1 run in Chrome: the worker grew 55 MB for each song, as at the head.  It logged 31 MB at the separation's start and 86 MB at its end, as the head does.  So the 2 MB came before M6, not with P or its fixes.
  * M5's Step 14 (`50d3ff6`) logged 30 and 87 MB (57 MB).  The `memory-budget` runs of M5.1 (2026-11-14) already logged 31 and 86 MB (55 MB).  The worker logs whole MB, so the true shift is less than 2 MB.
  * `web-test a_chunk_of_the_default_length_holds_its_working_set` passed in Chrome at the head (`<scratch>/m6/i1-2/wasm-test-chrome.log`).  It checks the live heap of each phase against `working_set`.  A held buffer adds to the growth, and the growth went down.  So no held buffer explains the change.
  * Cut 3's rise is still 9.44 MB (89.60 against 80.16 MB): `roformer/` has no change since M1's end (`156324a`).
  * The head (`69241d0` with this fix, which changes only `xtask`), with `WORKER_HEADROOM_MB` -19 MB: each of the 6 runs passed its four checks.

  | Runs | Worker growth (budget) | Check 3's margin | Check 1's margin | Tab total (margin under 943 MB) |
  | - | - | - | - | - |
  | Chrome, 3 | 55 MB (61.16 MB) each song | 6.16 MB | 4.0 MB | 756 MB (187 MB) |
  | Safari, 3 | 55 MB (61.16 MB) each song | 6.16 MB | 4.0 MB | 758 MB (185 MB) |

  * Each run logged 31 MB at the separation's start and 86 MB at its end.  The 4-minute song grew as much as the 1-minute song (check 2's margin 4.0 MB).
  * Check 3's margin is 6.16 MB again, as at M1.  A lost cut 3 (64.44 MB) is 3.28 MB over the budget, more than the taken noise (2.16 MB).

## Decision log

* 2026-11-15, agent (team leader): P's predicted gain is the same-page comparison of a prototype under a query key, in the measuring patch.  Why: no figure without `profile=1` gives the GPU's idle time between steps; the prototype gives P's effect itself at the base, with the noise rule of the comparison.
* 2026-11-15, agent (team leader): O's predicted gain is the sum over its CPU pieces of `min(piece, the paced time of the 2 steps under it)`, from the measuring patch's fields, after a first bound of all the CPU work.  Why: O needs a new order of the chunk, too large for a prototype in a measuring patch.  The maintainer allows 2 steps in flight, so a piece hides at most 2 steps (plan review round 1, finding 1), and those steps' own times, not a mean, bound it (round 2, finding 3).  The comparison of Step 5 decides the landing.
* 2026-11-15, agent (team leader): O is split into O1 (each mask applied under the next stem's first 2 steps) and O2 (the next chunk's input read and STFT under the last stem).  O1 is built first, and O2 only with its own predicted 3 %.  Why: O1 needs no read-ahead, no `Demixer` change, no chunk-time change and no working-set change (plan review round 2, finding 5; PLANS.md, "Proportionate").
* 2026-11-15, agent (team leader): P and O use two new `Gpu` methods that make their WebGPU call at once (`done_signal`, `read_start`).  Why: `Gpu::done` and `Gpu::read` are `async fn`s, so a signal taken through them covers the later step too (plan review round 1, finding 2).

* 2026-11-15, agent (implementer): Step 1's commit also changes `docs/references/local-browser-testing.md`, which gives the count of a whole Safari `web-test` (1,320 files).  Why: the new count makes that line wrong, and DOC-1 fixes a doc in the same commit.  Item 6 of Step 1 named only the two test files and `docs/tech-debt.md`.

* 2026-11-15, agent (implementer): by Step 2's rule, P is built (Step 4): its prototype gains 4.37 % in Chrome and 3.22 % in Safari.  O1 and O2 are not built at a depth of 2: 0.83 % and 0.68 % in Chrome, 0.82 % and 0.62 % in Safari.  O1's bound at an unlimited depth is 3.20 % in Chrome, so the checkpoint of item 2 of "Decisions for the maintainer" applies, and the implementer waits.  O2's bound at an unlimited depth is under 3 % in both browsers (1.55 %, 1.02 %), so O2 is not built in either answer, and Step 3 is skipped.
* 2026-11-15, agent (implementer): Safari's pins in Step 2 are `v64k8,v64k8`, from the `kernels:` line of its first unforced run, not `v64k8,v64` (Step 2's rule).
* 2026-11-15, agent (implementer): the measuring patch's `Signal` and `PendingRead` hold a clone of the device, so neither holds a borrow of `Gpu`.  `Signal::wait` and `PendingRead::finish` poll the device natively, as `Gpu::done` did.  `Gpu::done` is `done_signal().wait()`, and `Gpu::read` is `read_start(..).finish()`.

* 2026-11-15, agent (implementer): `pace_order`, `Window`, `step_secs` and `STEPS_IN_FLIGHT` are in `roformer-gpu/src/lib.rs`, not in the worker.  Why: the worker's code compiles for wasm32 only, so its tests run only in a browser.  In `roformer-gpu`, the tests run natively, and `Pace` and `bench::time_rec` use the same `Window`.  So one function gives the order of both (Step 4's first and third tests).  A source test (`the_worker_paces_as_calibration_times`) pins that `Pace` takes that window and drains before each read of a mask.
* 2026-11-15, agent (implementer): `Waits::EachStep` becomes `Waits::InFlight(n)`, and `STEP_WAITS` is `InFlight(STEPS_IN_FLIGHT)`.  Calibration's log lines now say "whole steps, 2 in flight", and `docs/references/local-browser-testing.md` names that line.  Why: the old text "with a wait after each" is no longer true.
* 2026-11-15, agent (implementer): `Pace::all` drains at its end, and the self-test drains before each of its reads.  Why: a step left in flight would have its errors and its time checked only at the next step, or never.
* 2026-11-15, agent (implementer): the self-test also ran the full chunk with the pins, beyond Step 4's `e2e selftest` with the default query.  Why: the default query runs only the stages, and the full chunk is the path that reads each mask after a drain.

* 2026-11-15, agent (implementer): Safari's bias in Step 8 is judged on the 8 forced runs (both sets), and holds there: -0.028 s, under the noise of `D`, 0.220 s.  By Step 8's rule on the first set alone, it fails: -0.033 s, over 0.0175 s.  Why: the first set's `D` changed its sign on a repeat, so it is under its noise, and the limit is then that noise (M5's rule).  The plan says that a failed bias is a defect of M6, to fix in the timing or by a revert of P.  This decision is the team leader's, so the report names it.
* 2026-11-15, agent (team leader): Safari's bias holds on the 8 forced runs, and M6 keeps P with no change of the timing.  Superseded by i1-3 of the intent review (the parent's Decision log): the bias fails its limit on the planned set, and the orchestrator accepts it.  Why: `D` changed its sign on the repeat, so it is under its noise, and M5's rule then takes that noise (0.220 s) as the limit.  `|D|` is about 0.5 % of a chunk, under `T` (1 %).  Calibration gives the two policies as equal (+0.002 s).  So neither choice costs more than the noise.  Neither `fused` nor `freq` is choosable on this Mac, since `vfused` is about 21 % ahead.

* 2026-11-15, agent (implementer of review round 1): a `Steps` trait gives the submission, the signal and the end of a step.  `Window::step` and `Window::drain` drive it in one order, for `Pace`, `time_rec` and `pace_order`.  `submit` and `signal` are two calls, so the shared function sets their order.  Why: finding 1-1; each caller had its own copy of the loop, and only `pace_order` was tested.  `Pace` keeps the window apart from its checks (`Checks`), so a step's end can borrow the checks while the window is borrowed.

* 2026-11-15, agent (implementer), finding i1-2 of the intent review: `WORKER_HEADROOM_MB` is -19 MB, not -17 MB.  Why: the worker grows 55 MB, not 57 MB.  With -17 MB, a lost cut 3 (64.44 MB) is only 1.28 MB over the budget, under the taken noise (2.16 MB).  By M1's rule, the margin goes midway between the taken noise and the smallest lost-cut rise (9.44 MB, cut 3): -25.16 + (2.16 + 9.44) / 2 = -19.36, rounded to -19 MB.  So the budget is 61.16 MB, 6.16 MB above 55 MB.  A lost cut 3 is 3.28 MB over it, more than the taken noise.  M6's base also gives 55 MB, and the wasm test passes.  So the change came before M6 and is not a held buffer, and case 4's rule applies.

## Outcomes and retrospective

What M6 achieved, on the M2 Pro with Chrome 157 and Safari 27.2:

* P lands: the worker keeps 2 GPU steps in flight.  The same-session comparison against M6's base gives a gain of 4.32 % in Chrome and 3.63 % to 3.70 % in Safari, with no loss.  The baselines went from 3.59 s to 3.43 s (Chrome) and from 5.57 s to 5.37 s (Safari).
* Calibration times the time axis with the same 2 steps in flight (`CALIBRATION` 7).  The choice check passes in both browsers.  The bias of `freq` against `fused` holds in Chrome.  In Safari it fails its limit on the planned set, and the orchestrator accepts it (i1-3; TD-106).
* The stems are the same as at M6's base (`e2e stored-stems`), and `memory-budget` passes in both browsers.  The worker grows 55 MB, so `WORKER_HEADROOM_MB` is -19 MB, not -17 MB (finding i1-2 of the intent review).
* The test helpers `plant` and `plant_at` write in place: a whole Safari `web-test` leaves 171 fewer swap files (147.7 MB less).

What M6 did not do:

* O1 and O2, the CPU work under the GPU, are not built.  The CPU work of a chunk is about 0.17 s, not the 0.25 to 0.3 s that the parent assumed.  At 2 steps in flight, each piece hides at most 2 steps, under 1 % of a chunk.  The maintainer kept the limit of 2 steps in flight.
* Case 10 (the chunk time over the input read) does not apply, since O2 is not built.

What to do differently:

* Measure the CPU pieces before an estimate goes into a plan.  The parent's 0.25 to 0.3 s came from before M1's cuts and M5's kernels; a measuring patch of one hour gave the real figure.
* A prototype under a query key, compared on one page, is a cheap and exact predictor for a pacing change.  The profile's estimate of the waits was 0.7 % in Safari; the prototype gave 3.2 %.
* Put the loops that tests must reach in the native crate from the start.  Rounds 1 to 4 each found a part of `Pace` that only a source test could see.  One shared `Window` and pure checks (`step_check`, `rejected_text`) closed it.

