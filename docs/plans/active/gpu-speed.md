# GPU speed on Apple Silicon, with no loss on other GPUs

## Purpose

On the maintainer's Apple M2 Pro, a separation in Safari is only a little faster than on the Intel Mac's Radeon Pro 5500M.  This plan makes separation faster on Apple GPUs, in Safari first, with no loss on the Radeon or the iPhone.  Each change is measured, and a perf gate stops a later change that makes separation slower.  The plan also replaces the `memory-budget` growth check, which fails at the default chunk, with a check that holds there.

## Context and orientation

The state on 2026-11-12, at `aa10c49` on `main`.  The machine is the maintainer's Mac: Apple M2 Pro with a 19-core GPU, 32 GB, macOS 27.2, Safari 27.2, Chrome 157.

How a chunk runs (`docs/ARCHITECTURE.md`, "From a song to six stems" and "Running it on the GPU"):

* `GpuNetwork::stems` (`worker/src/web.rs`) does the STFT on the CPU (`roformer::separate::analyse`).  Then it runs each GPU step through `Pace::run`, which submits one step and waits for the GPU (`Gpu::done`).  After the last mask step of a stem, it reads the mask back and applies it with the inverse STFT on the CPU (`roformer::separate::apply_masks`).
* The matmul kernels are the entries of `TILES` (`roformer-gpu/src/lib.rs`): `i64` and `s64` (64 x 64, 4 x 4 outputs for each thread), `i128x64` (8 x 4) and `v64` (64 x 64, vec4 reads).  Fused attention is `attention_tiled.wgsl`.  Each GPU times the candidates once, and keeps its choice in browser storage (`roformer-gpu/src/bench.rs`, `CALIBRATION`).
* The device is requested with `Limits::defaults()` (GPU-1, `docs/references/webgpu-limits.md`).
* `e2e memory-budget` (`xtask/src/e2e.rs`) separates a 1-minute and a 4-minute song, made from the `synth` demo song.  It runs at `chunk=16384&overlap=1`.  Its `growth_check` fails when the worker's WebAssembly memory grows by more than `MAX_GROWTH_MB` (4 MB) during a separation.

Measured on 2026-11-12 with the f16 weights, the 1-minute song (11 chunks), the default chunk and the default kernels (`--query x=1` and `--query profile=1`):

| | Chrome 157 | Safari 27.2 |
| - | - | - |
| Time for each chunk | 4.7 s | 7.1 s |
| Whole separation | 51.9 s | 78.2 s |
| Matmul rate | 1.34 TFLOP/s | 0.88 TFLOP/s |
| Kernels chosen | `v64,v64`, attention `freq` (time axis split) | `v64,v64`, attention fused |
| Time-axis attention, share of GPU time | 43 % (scores 7.7 s, P·v 7.8 s, softmax 4.4 s) | about 50 % (fused, 37.7 s with the frequency axis) |
| Gaps between steps, share of step time | 14 % with profiling | 1 % with profiling |
| CPU (STFT, inverse STFT), for each chunk | about 0.3 s | about 0.25 s |

The GPU's peak is about 6.8 TFLOP/s f32, so the kernels use 13 % to 20 % of it.  On the same shapes, Safari is 1.4 to 1.7 times slower than Chrome: for example, the linear product 5456 x 1536 x 256 takes 6.03 s against 3.54 s.  `s64`, which keeps its sums in a loop-indexed array, is 9.6 times slower in Safari.  The split time-axis attention writes and reads a 42 MB score buffer 744 times for each chunk.  Its softmax moves about 63 GB for each chunk at about 157 GB/s, near the bus limit of about 205 GB/s.

What the adapters offer (a probe page on 2026-11-12):

| | Safari 27.2 | Chrome 157 (`--enable-unsafe-webgpu`) |
| - | - | - |
| `maxComputeWorkgroupStorageSize` | 32,768 | 32,768 |
| `maxComputeInvocationsPerWorkgroup` | 1,024 | 1,024 |
| `shader-f16`, `timestamp-query` | yes | yes |
| `subgroups` | no | yes |
| `chromium-experimental-subgroup-matrix` | no | yes |

Facts from outside the repo, checked on 2026-11-12:

* On M1 and M2 GPUs, an f16 FMA runs at the f32 rate ([metal-benchmarks](https://github.com/philipturner/metal-benchmarks)).  So f16 arithmetic gives no speed on Apple GPUs.
* `subgroups` is in about 0.8 % of Safari and iOS browsers ([web3dsurvey](https://web3dsurvey.com/webgpu/features/subgroups)).
* Subgroup matrices are a Chrome experiment ([Dawn](https://dawn.googlesource.com/dawn/+/refs/heads/main/docs/dawn/features/subgroup_matrix.md)).
* WebKit's WGSL compiler adds bounds checks on index accesses ([WebKit commit](https://www.mail-archive.com/webkit-changes@lists.webkit.org/msg210152.html)).  No source gives their cost.

The worker's WebAssembly memory at the default chunk, Chrome, 2026-11-12: 30 MB at the separation's start, 58 MB before chunk 1, 92 MB after chunk 1, and 125 MB after chunk 2 to the end.  So it grows by one chunk's working set, about 95 MB, and then stays flat.  A spectrum, the features and one stem's mask are each about 18.9 MB at 1,151 frames.  TD-11 records this, with a fix that was not done.

## Plan of work

Six milestones, in order.  Each new kernel is a candidate that each GPU's calibration may choose.  A GPU that measures it slower keeps its old kernel, so the Radeon and the iPhone do not get slower.  Every kernel fits the default limits (GPU-1).

### M1: `memory-budget` at the default chunk (TD-11)

`e2e memory-budget` runs at the default chunk, the size that users run.  It checks what MEM-4 needs, not a growth of 4 MB:

* Flat after warm-up: the worker's WebAssembly memory grows by at most `MAX_GROWTH_MB` from the end of chunk 2 to the separation's end.
* No growth with the song: the 4-minute song's worker growth is at most `MAX_GROWTH_MB` above the 1-minute song's.
* A working-set budget: the worker's growth during a separation is at most a budget.  A function computes the budget from the chunk's buffer sizes, with measured headroom.
* A tab total: page, worker and decode-worker WebAssembly memory and GPU live bytes stay under a budget for the iPhone (MEM-4).

First find where the 95 MB goes, buffer by buffer.  Then cut the cheap parts, for example a masked spectrum held whole when one frame at a time does.  The cut must leave room for M6, which holds a second chunk's input.  Acceptance: a test of each pure check fails first on the case it catches.  `memory-budget` passes in Safari and Chrome at the default chunk.  TD-11 loses its bullet on the chunk.

### M2: the perf scenario and its gate

A new scenario, `e2e perf`, separates the 1-minute song at the default chunk with the f16 weights.  It writes a result for the browser: the adapter, the browser version, the kernels chosen, the median time for each chunk (chunk 2 to the end), the whole time and the matmul rate.  With `--query profile=1` it adds GPU time by group: time-axis attention, frequency-axis attention, linear products, other.

Baselines for each adapter and browser are in a committed file.  The gate fails when the chunk lower quartile (the lower quartile of chunks 2 to the end, by nearest rank) is more than the tolerance above the baseline; the result still gives the median.  The tolerance comes from at least 5 runs in each browser, and is never under 10 %, so that the gate is not a nuisance (the maintainer, 2026-11-12).  A run that is faster lowers the baseline only through an explicit update.  `check --tier push` runs the gate on a change to `roformer/`, `roformer-gpu/` or `worker/` when the weights are set; else it prints that the gate did not run.  Acceptance: tests of the parse and of the gate fail first.  The baselines hold the M2 Pro in Safari and Chrome, with the spread in Surprises.

### M3: spike on Safari's overhead

Find why Safari runs the same WGSL 1.4 to 1.7 times slower than Chrome.  Time variants of `v64` and of fused attention in `?bench`, in each browser: more outputs for each thread (8 x 8), and index forms that need no clamp.  The result is a table in Surprises and the design of M4 and M5.  A cause in WebKit gets an entry in `docs/references/browser-quirks.md`.  This milestone only measures: if it lands no code, it has no review rounds.

### M4: a matmul with less workgroup memory for each unit of work

A new `TILES` entry, with its f16 variant, from M3's result (`plans/active/gpu-speed-m3.md`, Decision log).  Safari sets workgroup memory to zero with one thread, byte by byte, so its cost grows with the bytes and the number of workgroups (`references/browser-quirks.md`).  The start point is `v64`'s 64 x 64 tile, 4 x 4 outputs for each thread and vec4 reads.  It has fewer bytes of workgroup memory for each unit of work.  Two candidates:

* `v64` with BK 8 (6 KiB with the pad of one vec4), with two barriers for each K tile, as `v64` has;
* a register-tiled kernel with no workgroup memory.  M3 did not measure a kernel of this kind.

Not a 128 x 128 tile with 8 x 8 outputs.  M3 measured it 1.30 to 1.59 times slower than `v64` in Safari, and 1.71 to 1.86 times in Chrome.  It also takes 12 KiB with the pad.  Not an unrolled inner loop, and no loop-indexed private array.  Each output keeps its sum order, so its f16 variant gives the f32 kernel's output bit for bit (decision 0022).  Calibration times it with the others; bump `CALIBRATION`.  Acceptance: `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` on this Mac.  `e2e selftest --query mm=<kernel>` passes in Safari and Chrome.  The perf gate passes.  The same-session comparison of "Validation and acceptance" shows the gain and no loss in either browser.  The baselines go down if the GPU chooses it.

Before M4 changes a baseline, it makes every raise of a bound need `--raise`, a new browser version included.  A test of it fails first (intent review of M2, i1-4).  No milestone changes a baseline before that change lands.  Until then, a browser update, a patch build included, only records its figures in Surprises.  The reason names the browser update when that is the cause.

After a new candidate (M4 and M5), measure again the cost of each forced flip between near choices, as M2 Step 5.3 did.  Near means within 10 % of the point where calibration flips:

* for attention, a fused / split ratio within 10 % of `FUSED_MARGIN` (1.1);
* for matmul kernels, which `decide` takes as the plain fastest, within 10 % of each other.

The drift between sessions is 4 % (`plans/completed/gpu-speed-m2.md`, Surprises).  If a flip's cost plus that drift is over the tolerance, decide in the Decision log.  The options are a wider tolerance with its proof, or a gate key that names the kernels.

### M5: a flash-style time-axis attention

A new fused attention kernel for the time axis (1,151 tokens, 8 heads of 64), from M3's result.  It keeps the softmax state on chip, so no score buffer goes to memory.  It loads each query block once.  It is a third `AttentionPolicy` candidate beside split and the current fused kernel.  M5's milestone plan defines "near" for three attention candidates, for the flip costs of M4.  It keeps the fused kernel's structure: 64 queries of one head for each workgroup, 4 x 4 for each thread, and online softmax.  It does not unroll its inner loops.  M3 measured an unrolled form 3.6 times slower in Safari and 1.33 times in Chrome.  It is checked against the CPU, as the current fused kernel is.  Its dispatches take their profile label from `attention_label`, so `e2e perf`'s profile groups count it as time-axis attention, not "other".  Acceptance: as M4, with `--query attn=<policy>`.

### M6: overlap on the worker

Goal: the GPU does not wait for the CPU, and the CPU does not wait for the GPU, where the work allows.  Two changes in `worker/src/web.rs`, each measured on its own:

* `Pace` keeps up to 2 steps in flight.  The step limit and the errors still stop a run.
* The CPU work runs while the GPU works: the STFT of the next chunk, and the application of each stem's mask.

M6's milestone plan designs the order of the work and the read-back.  It must meet these cases, each with a test that fails first where a test can see it:

1. The stems are the same byte for byte: `e2e stored-stems` gives the same hashes before and after.
2. No mask is read after a later stem's mask steps wrote the buffer: `Job` has one `mask` buffer, which every stem's mask steps write.
3. The CPU holds at most one mask.  Six masks (113 MB at the default chunk) leave about 7 MB under `TAB_BUDGET_MB` in Chrome, too little for the JS heap, which no check counts (TD-11).
4. A wait for a mask does not also wait for later steps.  In WebGPU, `mapAsync` resolves only after the queue work submitted before the call.  `Gpu::read` submits, maps and waits in one call today.
5. The next chunk's input can be read before every stem of this chunk is accepted, and the read does not move where the stems are accepted.  Today `next_input` reads at `pos`, which only `Demixer::end_chunk` moves during a separation.
6. `roformer::separate::working_set` and the phases of the worker's wasm test describe the overlap, with each part in the phases where it is live.  Added parts alone only raise check 3's budget and test nothing.
7. Buffers keep fixed sizes from chunk to chunk.  A new size left holes in the heap and grew the worker by 19 MB in M1 (`plans/completed/gpu-speed-m1.md`, Step 8).
8. `memory-budget` passes its four checks, with the tab total at most `TAB_BUDGET_MB` (MEM-4).  If it does not, M1's Step 6 names the next cuts, in order of cost.
9. The same-session comparison of "Validation and acceptance" shows the gain in Chrome and no loss in Safari.  The perf gate passes in both.

## Validation and acceptance

* `e2e perf` passes its gate in Safari and Chrome on this Mac, with the new baselines in the committed file.  The gate is a backstop: at 10 % and 0.1 s per chunk, it stops a large loss only.
* The same-session comparison shows each gain and "no loss" for M4, M5 and M6, in each browser.  Goal: a verdict that a real change of 3 % or more in either browser cannot escape, and that noise does not make.  M4's milestone plan designs the rule, and M5 and M6 use it.  The cases:
  1. The base page is built in a worktree and given to `e2e perf` with `--dist`, and the head page is HEAD's.  Base and head runs take turns in one session, as M2 did for M1 (`plans/completed/gpu-speed-m2.md`, Surprises), and each run is recorded in its order.
  2. Inside a run, the noise only adds time: groups of slow chunks.  Between sessions, and maybe between runs, the GPU's state can also move by about 4 % (Safari 7.10 s against 7.40 s with the same code).  So no verdict rests on one run.
  3. A uniform 3 % loss in either browser is found.  The figures can be the chunk lower quartile, the chunk median and the whole time ("6 stems in", 0.1 s on about 50 to 80 s).
  4. A loss or a gain holds only when it repeats.  When the verdict depends on one run, or the runs of one side differ by more than the noise, more pairs run.
  5. A loss that holds is a defect of the milestone, or a reason to drop its candidate (Extra stop conditions).  A loss that does not repeat is recorded, not acted on.
  6. A gain lowers the baseline with `--update-baselines`, after M4's `--raise` change.  An update in a fast session also takes up to about 4 % of the margin; M4's flip check allows for that drift.
* `e2e memory-budget` passes in Safari and Chrome at the default chunk.  Its check 3 headroom (`WORKER_HEADROOM_MB`) is mostly heap that is free at the separation's start, which the model load and calibration set.  M4 and M5 change calibration, and M6 changes `working_set`, so each can move it.  Goal: the constant stays a measured figure with a margin that is not too tight, and still catches a lost cut.  The milestone plans of M4, M5 and M6 design the measurement, with these cases:
  1. The headroom is measured again at the end of M4, M5 and M6, and when check 3's margin is under the noise or over the smallest lost-cut rise.
  2. A measurement follows proof that the cause is free heap: the wasm test passes, and a probe of the live heap at each phase (as in M1 Step 8) finds no held buffer.  A held buffer is a defect to fix.
  3. The smallest lost-cut rise is computed again from the current `working_set`, for the GPU half's cut 3 (9.44 MB at M1).  The wasm test checks the CPU half's cuts 1, 2 and 4.
  4. The constant goes midway between the noise (2.16 MB at M1) and that rise, by the rule in its doc.  If the noise is not less than the rise, decide in the Decision log.  Its doc, the figures in `the_worker_growth_must_fit_the_working_set_budget` and a Decision log line change in the same commit.
* `e2e selftest --weights <model>` passes in Safari and Chrome with the kernels that each browser chooses.
* `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passes on this Mac (Metal).
* `cargo run -p xtask -- check --tier push` passes, and CI passes on the PR.

## Idempotence and recovery

Each milestone is commits on one branch.  A milestone can stop after any commit, and the next session reads Progress.  A new kernel is a candidate, so a revert of its commit leaves the old choice.  `CALIBRATION` makes each browser time its kernels again after a change.  A session that stops in a perf run runs it again; the scenario writes nothing to the repo except through an explicit update of the baselines.

### How the work runs

#### Mode

Orchestrated mode (decision 0023), with the `orchestrate` skill: a plan of 6 milestones, with decision points in more than one.  One team leader for each milestone.  The maintainer agreed to the mode on 2026-11-12.

#### Branch and PR

The branch is `gpu-speed/2026-11-12`, from `main` at `aa10c49`.  The maintainer allowed a push of the branch and a draft PR into `main` on 2026-11-12.  The orchestrator pushes between milestones and review rounds, as the skill says.  It never pushes `main` or `gh-pages`, and never merges (GIT-1).

#### Maintainer steps

None.  Safari has Develop > Allow Remote Automation on.  The weights are in `third_party/models/`, which git ignores (ASSET-1), and the arm64 chromedriver is outside the repo; the kick-off prompt names their paths.  Runs on the Radeon Mac and the iPhone are optional, after the PR: the run never waits for them (the maintainer, 2026-11-12).

#### Preflight

The skill's preflight, and:

* `caffeinate -dimsu` runs for the length of the run.
* `cargo run -p xtask -- setup --check` passes.
* The weights file and the chromedriver of the kick-off prompt exist.
* `STEMYLA_PERF_WEIGHTS` names the weights file and `CHROMEDRIVER` the chromedriver, in the environment of each `check --tier push` and of each team.  From M3, each push check before a review round runs both perf gates.  A milestone's Progress gives both gates' verdicts; a skip line is not a pass (VERIFY-1).

#### Decision points

See "Decisions for the maintainer".

#### Review rule

Decision 0024, through the `review-rounds` skill.  M3 has no review rounds if it lands no code.

#### Kick-off prompt

```text
/orchestrate docs/plans/active/gpu-speed.md.  Weights: /Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors.  CHROMEDRIVER: ~/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver.
```

#### Extra stop conditions

None beyond the skill's.  The orchestrator decides these cases, records them in the Decision log, and goes on:

* A kernel that is not faster on this Mac in either browser does not land.  Record the figures in Surprises.
* A cause of Safari's overhead in WebKit gets a `browser-quirks.md` entry and a draft WebKit bug in the PR for the maintainer to file.
* A browser update between runs changes the speed.  Until M4's `--raise` change lands, record both sets of figures in Surprises, and change no baseline.  If the update fails the gate before then, the gate's failure is recorded with the version, and the milestone goes on with the same-session comparison for its verdict.  After that change, update the baselines with `--raise`, naming the browser update.

### Decisions for the maintainer

1. Which WebGPU limits and features may the new kernels use?  Options: the default limits only; the adapter's limits with a fallback kernel (amends GPU-1); also `subgroups` (Chrome only).  Recommendation: the default limits only, because the measured bottlenecks fit in them.  Answer (2026-11-12): the default limits only.
2. How strict is the perf gate?  Options: a gate on this M2 Pro, with Radeon and iPhone runs at the end; a Radeon run before each kernel default; numbers only.  Recommendation: the M2 Pro gate.  Answer (2026-11-12): the M2 Pro gate, with tolerances that are not too tight.  Do not wait on the maintainer for anything on the Radeon or the iPhone.
3. Does the worker overlap GPU steps (up to 2 in flight)?  Recommendation: yes, measured, and dropped if Safari gains nothing and Chrome gains under 5 %.  Answer (2026-11-12): yes, measured.
4. May the run push and keep a draft PR?  Answer (2026-11-12): yes, a draft PR into `main`.
5. `memory-budget` at the default chunk.  The maintainer asked on 2026-11-12 for "a good, real solution" to the failed growth check.  The design (M1) is an agent decision, below.

## Progress

* [x] 2026-11-12: baselines measured (Context), the maintainer's decisions recorded, the plan written.
* [x] 2026-11-12: M1: `memory-budget` at the default chunk (`22cdd74..156324a`, plan `plans/completed/gpu-speed-m1.md`, reviews `docs/reviews/gpu-speed-2026-11-12-m1.md`, 8 rounds).  `e2e memory-budget` runs at the defaults with four checks: flat after chunk 2, no growth with the song, a working-set budget (`roformer::separate::working_set` plus a measured headroom of -17 MB), and a tab total of at most 943 MB (MEM-4).  It passes in Chrome and Safari.  Four bit-identical cuts and a read fix took the worker's working-set peak from 117.86 to 80.16 MB and its growth from 95 to 57 MB; the tab total is 799 MB (Chrome) and 758 MB (Safari).  `stored-stems` hashes did not change.  Stats: 8 rounds, 51 agents, 2,863,709 tokens, 107.2 wall minutes, 0 interventions; rounds 4 to 8 fixed 6 findings, all in one new GPU test, its doc and TD-8.  Debt: TD-11 lost its chunk bullet; TD-8 gained a line on what the new GPU tests do not check.
* [x] 2026-11-12: M2: the perf scenario and its gate (`91923ae..9ddc32f`, plan `plans/completed/gpu-speed-m2.md`, reviews `docs/reviews/gpu-speed-2026-11-12-m2.md`, 5 rounds).  `e2e perf` separates the 1-minute song at the default chunk, and gates the chunk lower quartile against `xtask/perf-baselines.json`.  The M2 Pro baselines are 4.70 s (Chrome 157) and 7.40 s (Safari 27.2), each from 5 runs, with a tolerance of 10 %.  `check --tier push` runs the gate in Chrome, then Safari, when `STEMYLA_PERF_WEIGHTS` names the model (decision 0031, proposed).  The gate fails closed on a fallback adapter, a software adapter and a query that changes the work.  Safari is not slower after M1 (7.40 s at `22cdd74` and at HEAD).  Stats: 5 rounds, 42 agents, 2,704,055 tokens, 90.0 wall minutes, 0 interventions; rounds 4 and 5 fixed 2 findings, both in the round 3 fixes.  Debt: TD-10 and TD-16 name what the gate does not check.
* [ ] M3: spike on Safari's overhead.
* [ ] M4: a matmul with less workgroup memory for each unit of work.
* [ ] M5: a flash-style time-axis attention.
* [ ] M6: overlap on the worker.

## Surprises and discoveries

* 2026-11-12: the session runs Opus 5.5 (`claude-opus-5-5`, the system prompt) at effort `high` (`modelSettings` for `claude-opus-5-5` in `~/.claude/settings.json`, over `effortLevel: xhigh`).
* 2026-11-12: Safari has `timestamp-query`, so `?profile=1` gives GPU times in Safari as in Chrome.
* 2026-11-12, M1: the worker's WebAssembly memory at the default chunk is now 87 MB from chunk 1 to the end, a growth of 57 MB (95 MB before).  The tab total is 799 MB in Chrome and 758 MB in Safari.  After the cuts the heap has little free space.  So a buffer of a new size, such as a shorter last chunk, grew the worker by 19 MB until M1 read and padded each chunk at one size (`plans/completed/gpu-speed-m1.md`, Step 8).  M6 keeps its buffers at fixed sizes for the same reason.
* 2026-11-12: Chrome's calibration chose split attention on the time axis (split 1.90 s, fused 2.26 s for each chunk).  Safari's chose fused (split 3.43 s, fused 3.27 s).  `docs/ARCHITECTURE.md` says Apple GPUs choose fused, which holds only for Safari.
* 2026-11-12, M2: the perf baselines of the M2 Pro, from 5 runs in each browser (`xtask/perf-baselines.json`, `plans/completed/gpu-speed-m2.md`).  The gate reads the chunk lower quartile: the lower quartile of chunks 2 to N, by nearest rank.  Chrome 157: 4.70 s, spread 0 %, tolerance 10 %.  Safari 27.2: 7.40 s, spread 0 %, tolerance 10 %.  On the chunk median, the first 5 runs gave tolerances of 22 % and 28 %: the chunk times have a floor and groups of slow chunks.  The cost of a forced kernel flip: Chrome `attn=fused` 2.1 %, `mm=v64,i64` 0 %; Safari `attn=freq` 9.5 %, `mm=v64,i64` 0 %.  Profile groups (GPU seconds, one pass for each dispatch): Chrome time-axis attention 20.03 s (43.6 %), frequency-axis attention 1.81 s (3.9 %), linear products 22.02 s (47.9 %), other 2.10 s (4.6 %); Safari 36.93 s (47.2 %), 2.73 s (3.5 %), 35.32 s (45.2 %), 3.20 s (4.1 %).

* 2026-11-12, M1 round 4 and M2 round 5: twice a wire reviewer changed a line in another reviewer's worktree, then reverted it.  In M2 it came after a failed `git worktree add` on a name in use, when the next commands of the chain ran in the wrong folder.  The validators ran each claim again in their own worktrees, and `review check` printed "no change".  TD-34 covers edits in worktrees; review agents should stop a command chain on a failed `worktree add`.

* 2026-11-13, M3: why Safari 27.2 is slower than Chrome 157 on the same WGSL, on the M2 Pro.  The Surprises of `plans/active/gpu-speed-m3.md` hold the tables.  The logs and `m3/findings.md` stayed in the session scratchpad.  The controls are near 1 (FMA 1.05 to 1.08, copy 1.03 to 1.04), so the GPU's state does not explain it.  `v64`'s browser ratio is 1.76 on `freq qkv` with f16 weights (the parent's 1.70).  It falls as K grows: 2.4 at K 64, 1.76 at K 256, 1.45 at K 1,024.

  | Part of Safari's loss | Evidence | Share of `v64`'s loss |
  | - | - | - |
  | WebKit sets workgroup memory to zero with thread 0, byte by byte | Chrome's MSL with WebKit's zero-init and clamps, timed through Metal: 1.49 times at K 256, 1.13 at K 1,024 | the emulation's estimate: 64 % at K 256, 30 % at K 1,024.  Other estimates at K 256: 55 % (Step 3's fit), about 34 % (`v64-g4`), about 26 % (the `wg10k` control).  The `v64-g4` estimate assumes that `v64-g4`'s own extra cost is the same in both browsers.  Safari's time with `v64-g4` at K 256 did not go down (1.014 to 1.033 times `v64`).  The browser ratio fell because Chrome got about 1.20 times slower |
  | Bounds clamps | 2 % to 5 % in each browser (`v64-clamp2`, robustness off in Chrome) | in the row above |
  | A K loop about 1.35 times slower | Step 3's fit; not given by any tested form | the rest; most of fused attention's 1.52 |
  | Ruled out | barriers, `unpack2x16float`, the GPU's state, the thread maximum of Safari's MSL, a load of 64 registers; fast math and the loop guard by source | |

  Two variants are slower than `v64` in both browsers.  An unrolled inner loop: 1.49 to 1.93.  128 x 128 with 8 x 8 for each thread: 1.30 to 1.86.  At K 256, four tiles for each workgroup (`v64-g4`) is slower in Chrome (1.20), and not faster in Safari (1.014 to 1.033).  At K 1,024 it is slower in both: Safari 1.34 to 1.35, Chrome 1.25 to 1.27.  `s64`'s 10 times in Safari is its loop-indexed private array: with constant indices it runs at `i64`'s rate.  Time-axis attention in Safari: fused 3.47 s for each chunk, split 3.53 s; in Chrome 2.29 s and 1.85 s.

## Decision log

* 2026-11-12, maintainer: the answers to "Decisions for the maintainer", 1 to 4.
* 2026-11-12, agent (orchestrator): `memory-budget`'s growth check is replaced, not widened (M1).  Why: at the default chunk the worker grows by one chunk's working set and then stays flat, so a growth of 4 MB tests the wrong thing.  The checks that MEM-4 needs are flat after warm-up, no growth with the song, a working-set budget and a tab total.
* 2026-11-12, agent (orchestrator): no f16 arithmetic.  Why: on M1 and M2 GPUs, f16 FMA runs at the f32 rate, and decision 0022 keeps f32 sums.
* 2026-11-12, agent (orchestrator): no `subgroups` and no subgroup matrices.  Why: Safari 27.2 offers neither, and the maintainer chose the default limits and features (decision 1).
* 2026-11-12, agent (orchestrator): the Radeon and the iPhone are protected by calibration, not by required runs.  Each new kernel is a candidate that a GPU keeps only if it measures faster there.
* 2026-11-12, agent (orchestrator): M1's worker headroom is -17 MB, not the team's -22 MB.  Why: -22 MB left a margin of 1.16 MB, which the maintainer's "not too tight" rules out.  The measured noise is 1 MB (the log's whole MB), and 2.16 MB was taken for it.  -17 MB is midway between that 2.16 MB and the smallest loss that it must catch (9.44 MB, a lost cut).
* 2026-11-12, agent (orchestrator), intent review of M1: M6 holds at most one mask on the CPU, and defines `WorkingSet`'s phases again for the overlap (finding i1-1).  Check 3's headroom is measured again only by the cases in "Validation and acceptance", never widened (findings i1-2, i2-2, i3-1, i3-2, i4-2).
* 2026-11-12, agent (orchestrator), intent review of M1, round 4: rounds 1 to 4 each found a new case in M6's overlap bullet and in the headroom bullet.  So both are now a goal and a list of cases, and the milestone plans of M4 to M6 design the rest (orchestrate skill, "The intent review").
* 2026-11-12, agent (orchestrator): from M1 review round 6, each test doc in this plan's diffs says only what its test asserts, as a list.  Why: rounds 4 and 5 each found a new overclaim in one test's doc.
* 2026-11-12, agent (team leader of M2, accepted by the orchestrator): the perf gate reads the chunk lower quartile, not the chunk median.  Why: on the median, 5 runs in each browser gave tolerances of 22 % (Chrome) and 28 % (Safari), over M2's limit of 20 %.  The chunk times have a floor and groups of slow chunks from interference, which only adds time, and a slower kernel moves every chunk.  On the same runs the lower quartile gives 10 % and 11 % (Chen and Revels, [Robust benchmarking in noisy environments](https://arxiv.org/abs/1608.04295)).
* 2026-11-12, agent (team leader of M2): Safari's tolerance stays at 10 %, though a forced `attn=freq` run costs 9.5 %.  Why: calibration splits the time axis only when split is more than `FUSED_MARGIN` (1.1) times faster, and 26 Safari calibrations gave fused / split 0.975 to 0.992 (`plans/completed/gpu-speed-m2.md`, Decision log).
* 2026-11-12, agent (team leader of M2): decision 0031 amends decision 0026 and GIT-2: tier `push`, when asked, runs `e2e perf --gate` in Chrome, then Safari, when `STEMYLA_PERF_WEIGHTS` names the model and the paths change `roformer/`, `roformer-gpu/`, `worker/` or a workspace file.  For "Needs your attention" in the PR.

* 2026-11-12, maintainer: decision 0031 is accepted.
* 2026-11-12, agent (orchestrator), intent review of M2: the gate is a backstop, and M4 to M6 show gain and no loss by a same-session comparison of base and head (i1-2).  The preflight sets `STEMYLA_PERF_WEIGHTS`, and a skip is not a pass (i1-1).  M4 and M5 measure flip costs again (i1-3).  M4 makes every raise need `--raise` before it changes a baseline (i1-4), not a review round now, since the finding is not significant.  M5 uses `attention_label` (i1-5).  Not taken: chunk times in 2 decimals (i1-2's code part), since the whole time already has 0.1 s on about 50 to 80 s; `m4a/` and `protocol/` in `PERF_FOLDERS` (i1-6), refuted in M2 round 1 (1-8).
* 2026-11-12, agent (orchestrator), intent review of M2, round 2: a loss or a gain compares the fastest runs of base and head, since the noise only adds time (i2-1).  "Near" choices are measured from calibration's flip point (i2-2).  M6 case 9 uses the same-session comparison (i2-3).  No baseline changes before M4's `--raise` change (i2-4).  The drift figures cite `plans/completed/gpu-speed-m2.md` (i2-5).
* 2026-11-12, maintainer: the weights file moved to `third_party/models/`, which git ignores (`/third_party/`, `*.safetensors`).  The kick-off prompt and `STEMYLA_PERF_WEIGHTS` give its absolute path, since review worktrees hold no ignored files.
* 2026-11-12, agent (orchestrator), intent review of M2, round 3: rounds 1 to 3 each found a new case in the same-session comparison.  So it is now a goal and a list of cases, and M4's milestone plan designs the rule (orchestrate skill, "The intent review").  This replaces the rule of round 2 (i2-1).  Until M4's `--raise` change, a browser update changes no baseline (i3-1).  M5 defines "near" for three attention candidates (i3-5).
* 2026-11-13, agent (team leader of M3): the cause of Safari's fixed cost for each workgroup is WebKit's zero-init of workgroup memory, with its clamps (`references/browser-quirks.md`).  Why: Chrome's MSL with that zero-init and those clamps, timed through Metal, is 1.49 times slower at K 256, and Safari's `v64` is 1.76 times.  The rest, a K loop 1.35 times slower, has no cause found.
* 2026-11-13, agent (team leader of M3): M4 starts from `v64`'s tile with less workgroup memory for each unit of work.  The candidates are `v64` with BK 8, and a kernel with no workgroup memory, which M3 did not measure.  Why: the zero-init cost grows with the bytes.  Also, 128 x 128 with 8 x 8 for each thread was slower in both browsers (1.30 to 1.86).  Barriers cost nothing (`v64-bar` 1.00); unrolled loops cost 1.49 to 1.93.
* 2026-11-13, agent (team leader of M3): M5 keeps the fused kernel's structure, loads each query block once and does not unroll.  Why: an unrolled form was 3.6 times slower in Safari, and the zero-init is about 2 % of a fused dispatch.

## Outcomes and retrospective

Not yet.
