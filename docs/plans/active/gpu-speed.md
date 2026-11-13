# GPU speed on Apple Silicon, with no loss on other GPUs

## Purpose

On the maintainer's Apple M2 Pro, a separation in Safari is only a little faster than on the Intel Mac's Radeon Pro 5500M.  This plan makes separation faster on Apple GPUs, in Safari first, with no loss on the Radeon or the iPhone.  Each change is measured, and a perf gate stops a later change that makes separation slower.  The plan also replaces the `memory-budget` growth check, which fails at the default chunk, with a check that holds there.

## Context and orientation

The state on 2026-11-12, at `aa10c49` on `main`.  The machine is the maintainer's Mac: Apple M2 Pro with a 19-core GPU, 32 GB, macOS 27.2, Safari 27.2, Chrome 157.

How a chunk runs (`docs/ARCHITECTURE.md`, "From a song to six stems" and "Running it on the GPU"):

* `GpuNetwork::stems` (`worker/src/web.rs`) does the STFT on the CPU (`roformer::separate::analyse`).  Then it runs each GPU step through `Pace::run`, which submits one step and waits for the GPU (`Gpu::done`).  After the last mask step of a stem, it reads the mask back and applies it with the inverse STFT on the CPU (`roformer::separate::apply_masks`).
* The matmul kernels are the entries of `TILES` (`roformer-gpu/src/lib.rs`): `i64` and `s64` (64 x 64, 4 x 4 outputs for each thread), `i128x64` (8 x 4), `v64` (64 x 64, vec4 reads) and, since M4, `v64k8` (`v64` with BK 8, 6 KiB of workgroup memory).  Fused attention is `attention_tiled.wgsl`.  Each GPU times the candidates once, and keeps its choice in browser storage (`roformer-gpu/src/bench.rs`, `CALIBRATION`).
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

A new `TILES` entry, with its f16 variant, from M3's result (`plans/completed/gpu-speed-m3.md`, Decision log).  Safari sets workgroup memory to zero with one thread, byte by byte, so its cost grows with the bytes and the number of workgroups (`references/browser-quirks.md`).  The start point is `v64`'s 64 x 64 tile, 4 x 4 outputs for each thread and vec4 reads.  It has fewer bytes of workgroup memory for each workgroup, on the same grid.  More work for each byte is not the rule: `v64-g4` and `v128` did more work for each byte, and neither was faster in Safari.  Two candidates:

* `v64` with BK 8 (6 KiB with the pad of one vec4), with two barriers for each K tile, as `v64` has.  If the cost is linear in the bytes, M3's Step 5.2 puts its gain at about 6 % to 11 % of a K 256 product (40 % of 0.56 to 0.96 ms, on 3.52 ms), and 1 % to 5 % of a separation.  `vec4_source` asserts that each tile load fills 256 threads, so BK 8 needs a change there;
* a register-tiled kernel with no workgroup memory.  M3 did not measure a kernel of this kind.  Step 5.2 bounds its gain at up to 27 % of a K 256 product.

In Safari, M3 measured the zero-init only at 10 KiB and at 0 KiB; that its cost grows with the bytes comes from WebKit's source.  So M4 first times a 6 KiB `wg` control and each candidate in `?bench`.  The `wg10k` control is not on the branch: it is patch 0003 of M3's variants, on the control harness of patch 0001, which stayed in the session scratchpad (Step 7.5 of `plans/completed/gpu-speed-m3.md`), so M4 adds it.  The in-page ratio decides the gain, and the same-session comparison decides that there is no loss.

Not a 128 x 128 tile with 8 x 8 outputs.  M3 measured it 1.30 to 1.59 times slower than `v64` in Safari, and 1.71 to 1.86 times in Chrome.  It also takes 12 KiB with the pad.  Not an unrolled inner loop, and no loop-indexed private array.  Each output keeps its sum order, so its f16 variant gives the f32 kernel's output bit for bit (decision 0022).  Calibration times it with the others; bump `CALIBRATION`.  M4's first commit fixes the entry of `references/browser-quirks.md` that says more work for each byte loses less.  Acceptance: `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` on this Mac.  `e2e selftest --query mm=<kernel>` passes in Safari and Chrome.  The perf gate passes.  The change meets "A change lands only with a shown gain" ("Validation and acceptance").  The baselines go down if the GPU chooses it.

Before M4 changes a baseline, it makes every raise of a bound need `--raise`, a new browser version included.  A test of it fails first (intent review of M2, i1-4).  No milestone changes a baseline before that change lands.  Until then, a browser update, a patch build included, only records its figures in Surprises.  The reason names the browser update when that is the cause.

After a new candidate (M4 and M5), measure again the cost of each forced flip between near choices, as M2 Step 5.3 did.  Near means within 10 % of the point where calibration flips:

* for attention, a fused / split ratio within 10 % of `FUSED_MARGIN` (1.1);
* for matmul kernels, which `decide` takes as the plain fastest, within 10 % of each other.

The drift between sessions is up to about 5.7 % ((7.40 - 7.00) / 7.00; Safari 7.00 s at `eb81ed0` against the 7.40 s baseline, with the same GPU code; `plans/completed/gpu-speed-m3.md`, Surprises).  If a flip's cost plus that drift is over the tolerance, decide in the Decision log.  The options are a wider tolerance with its proof, or a gate key that names the kernels.  Each forced and unforced run pins every other choice in its `--query` (for example `mm=v64k8,v64&attn=fused`), so that one flip is measured alone: M4's Safari matmul flips were mixed with the attention choice.

### M5: a flash-style time-axis attention

A new fused attention kernel for the time axis (1,151 tokens, 8 heads of 64), from M3's result.  It keeps the softmax state on chip, so no score buffer goes to memory.  It loads each query block once only if that fits the default limits (GPU-1): a 64 x 64 f32 query block alone is 16 KiB, and the current kernel stages Q again for each key tile.  M3 measured no cost for those reloads, and puts fused attention's Safari loss in the K loop.  Each byte or workgroup that it adds raises Safari's zero-init cost (`references/browser-quirks.md`).  So M5 measures the cost of the reloads before it removes them.  M3 found no change that makes fused attention faster in Safari.  So M5's milestone plan first names each change from `attention_tiled.wgsl`, with the figure that predicts its gain in each browser.  It times each in `?bench` before it lands.  If none is faster in either browser, M5 lands no kernel, and records why in Surprises.  It is a third `AttentionPolicy` candidate beside split and the current fused kernel.  M5's milestone plan defines "near" for three attention candidates, for the flip costs of M4.  It keeps the fused kernel's structure: 64 queries of one head for each workgroup, 4 x 4 for each thread, and online softmax.  It does not unroll its inner loops.  M3 measured an unrolled form 3.6 times slower in Safari and 1.33 times in Chrome.  It is checked against the CPU, as the current fused kernel is.  Its dispatches take their profile label from `attention_label`, so `e2e perf`'s profile groups count it as time-axis attention, not "other".  Acceptance: as M4, with `--query attn=<policy>`.  The gain is shown by the rule of Step 7 of `plans/completed/gpu-speed-m4.md`, against each time-axis policy that calibration chooses at M5's base in that browser: `freq` (the time axis split) in Chrome, `fused` and `freq` in Safari.  Until TD-101 is fixed, a ratio against split does not count in either browser, and `e2e perf --base-dist` decides there.  In Chrome, calibration puts fused about 0.3 s for each chunk above what a forced run costs (M2 Step 5.3), the same sign as in Safari.  So M5 looks for one cause in both browsers; a lead is that calibration queues up to 64 submissions for each wait, and the separation waits after each step (744 steps for each chunk with split, 186 with fused).  No loss is shown by `e2e perf --base-dist` (`docs/references/local-browser-testing.md`).  M4 found that Safari's calibration times split attention about 0.5 s for each chunk too low, at the base and with `v64k8` (Surprises, M4).  M5 finds the cause first, since a third candidate is chosen by the same timing.  M5's first commit fixes TD-101's text: about 10 of 22 calibrations, and the same bias in Chrome.  Each user keeps one calibration until the next `CALIBRATION` bump, so a Safari user who draws split keeps a separation about 1 % slower than before M4; the same bias can flip the iPhone, which this Mac cannot measure.  Acceptance also: TD-101 is closed, by the goal "Each calibration choice is not slower" in "Validation and acceptance".

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
9. Each of the two changes meets "A change lands only with a shown gain" ("Validation and acceptance").  The gain of 3 % or more and the absence of a loss come from `e2e perf --base-dist` (`docs/references/local-browser-testing.md`).  The perf gate passes in both browsers.
10. The chunk time covers the work that M6 moves.  Today the timer starts after `next_input`, so it leaves out the input read; a change that moves the next chunk's read or STFT across that edge gives a false gain or loss of up to the STFT's share (3 % to 6 %).  So before the first change, in its own commit, the chunk time runs from the end of the previous chunk's log line to this one's (chunk 1's from just before its `next_input`), and a source test (as `chunk_times_have_two_decimals`) fails first on the old form.  That commit is the oldest base of M6's comparisons.  It runs `e2e perf --base-dist --runs 4` against its parent in each browser; if that is not "no change", the baselines are measured again with `--raise "the chunk time covers the input read"`.
11. Calibration times the time axis as the separation runs it, after each change too.  A change that alters `Pace` alters calibration's timing in the same commit, and bumps `CALIBRATION` if the timing changes.  The check of the goal "Each calibration choice is not slower" runs again at M6's head, by its case 5.

## Validation and acceptance

* `e2e perf` passes its gate in Safari and Chrome on this Mac, with the new baselines in the committed file.  The gate is a backstop: at 10 % and 0.01 s per chunk, it stops a large loss only.
* A change lands only with a shown gain.  Goal: no change of M4, M5 or M6 lands on noise, or with a loss in either browser, or with a loss on a GPU that this Mac cannot measure.  M4's milestone plan designs the rule, and M5 and M6 use it.  The rule for a kernel is Step 7 of `plans/completed/gpu-speed-m4.md`, also in `docs/references/local-browser-testing.md`.  The cases:
  1. A kernel gain (M4, M5) can be under 3 % of a separation: BK 8's is 1 % to 5 %.  It is shown by the in-page `?bench` ratio of the candidate over the current default, in each browser that chooses it.
  2. The ratio covers the shapes on which calibration chooses (`bench.rs`), not only the shape with the largest predicted gain.
  3. The ratio is an effect by the rule in Terms of `plans/completed/gpu-speed-m3.md` for a ratio in one page: its distance from 1 is larger than the larger spread of the two figures, in both pages of each browser, as M3's Step 4 table used it.  The 4 % margin and the FMA control are for comparisons across pages only.
  4. A worker change (M6) has no `?bench` ratio.  It lands only if the same-session comparison shows a gain of 3 % or more in Chrome or Safari.  Else its figures go in Surprises, and it does not land.
  5. No change lands with a loss in either browser by the same-session comparison.
  6. Calibration protects the Radeon and the iPhone from a new candidate, not from a changed old kernel.  So the WGSL of each current kernel stays the same byte for byte, and a test that fails first on a change checks it.
* Each calibration choice is not slower (TD-101).  Goal: a user keeps one calibration until the next `CALIBRATION` bump, so no time-axis policy that calibration can choose on this Mac is slower than the best one by more than the noise, in either browser.  M5's milestone plan designs the check, and M6 runs it again.  The cases:
  1. A check of the mean over mixed runs is not enough: the mean hides a slow policy, and M4's loss of about 1 % in the split mode sits at `T`.  So each policy is forced alone, on the same page as the policy it is compared with, where `freq` costs 2.3 % (the forced flip alone) to 3.4 % (the median of all its runs) against `fused` (M4 Step 10).
  2. The check fails first: on the page of `ab40862`, it gives a loss for `freq` against `fused` in Safari.
  3. The policies are the names that calibration writes in the `kernels:` line: today `fused` or `freq`, and the name of M5's candidate if it lands.  Never force `split`: it also splits the frequency axis, which is 2.2 to 3.6 times slower fused, and adds about 4 % to 9 % to a run.
  4. A tool that forces a policy on the head runs only (for example a `--head-query`) gets a test that fails first.
  5. At M6's head the check runs on the head page alone, since M6's case 10 changes the chunk time, and a gain of M6 can hide a slow policy against an older base.
  6. If M5 finds no cause, calibration times the time axis as the separation runs it.  If that is not enough, the orchestrator decides in the Decision log and in the PR's "Needs your attention".
* The same-session comparison shows "no loss" for M4, M5 and M6 in each browser, and each gain of 3 % or more.  Goal: a verdict that a real change of 3 % or more in either browser cannot escape, and that noise does not make.  M4's milestone plan designs the rule, and M5 and M6 use it.  The rule is `e2e perf --base-dist` (Step 5 of `plans/completed/gpu-speed-m4.md`, and `docs/references/local-browser-testing.md`).  The cases:
  1. The base page is built in a worktree and given to `e2e perf` with `--dist`, and the head page is HEAD's.  Base and head runs take turns in one session, as M2 did for M1 (`plans/completed/gpu-speed-m2.md`, Surprises), and each run is recorded in its order.
  2. Inside a run, the noise only adds time: groups of slow chunks.  Between sessions, and maybe between runs, the GPU's state can also move by about 5 % (Safari 7.00 to 7.40 s with the same code).  So no verdict rests on one run.
  3. A uniform 3 % loss in either browser is found.  The figures can be the chunk lower quartile, the chunk median and the whole time ("6 stems in", 0.1 s on about 50 to 80 s).
  4. A loss or a gain holds only when it repeats.  When the verdict depends on one run, or the runs of one side differ by more than the noise, more pairs run.
  5. A loss that holds is a defect of the milestone, or a reason to drop its candidate (Extra stop conditions).  A loss that does not repeat is recorded, not acted on.
  6. A gain lowers the baseline with `--update-baselines`, after M4's `--raise` change.  An update in a fast session also takes up to about 6 % of the margin; M4's flip check allows for that drift.
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
* [x] 2026-11-13: M3: spike on Safari's overhead (`7ec4d06..4a2ae27`, plan `plans/completed/gpu-speed-m3.md`, reviews `docs/reviews/gpu-speed-2026-11-12-m3.md`, 7 rounds).  Safari sets workgroup memory to zero with one thread, byte by byte (`references/browser-quirks.md`; bug text in `references/webkit-bug-workgroup-zero-init.md`).  That explains 26 % to 64 % of `v64`'s loss at K 256; a K loop about 1.35 times slower is the rest, with no cause found.  M4 and M5 start from these results (their sections).  The reviews also covered the CI commit 1be7a18: `web-test` exits with 75 only after the runner's hang, and CI runs a step again only on 75 (TD-98).  After the maintainer's request of 2026-11-13, each browser scenario of `e2e` has a time limit, the review agents' guard refuses a wide search (`find /`), and the skills ask for absolute paths and a check that no agent still runs.  Stats: 7 rounds, 70 agents, 4,272,580 tokens, 117.1 wall minutes, 0 interventions in the rounds; between rounds 3 and 4 the maintainer sent 3 messages, on a `find /` of a round 3 reviewer that ran 3.6 hours and on a Safari gate that hung 3 hours.  Rounds 4 to 7 fixed 26 findings, all in the time limits, the guard and their tests.  Debt: TD-99 (a stem-worker test that failed once on CI) and TD-100 (the CI step limit ends `e2e save` first) are new; TD-34 lists the wide searches that still pass; TD-39 gained 4 missed mutants.
* [x] 2026-11-13: M4: a matmul with less workgroup memory for each unit of work (`9769916..ab40862`, plan `plans/completed/gpu-speed-m4.md`, reviews `docs/reviews/gpu-speed-2026-11-12-m4.md`, 5 rounds).  `v64k8` (`v64` with BK 8, 6 KiB of workgroup memory) is a calibration candidate (`CALIBRATION` 4).  Its in-page ratio on the large products is 0.944 to 0.946 in Safari, a gain, and 1.31 in Chrome, so Chrome keeps `v64`.  `r64`, with no workgroup memory, does not land.  The same-session comparison (`e2e perf --base-dist`, blocks of base, head, head, base; `T` 1 %, `N` 1.1 % from an A/A session) gives a gain of about 2.2 % to 2.4 % in Safari and no change in Chrome.  Safari's baseline went from 7.40 s to 7.38 s; in about 6 of 16 calibrations Safari chooses split attention, about 1 % slower than the base (TD-101, for M5).  In Safari, 6 KiB of workgroup memory costs 0.257 of what 10 KiB costs (`references/browser-quirks.md`).  The WGSL of each current kernel is pinned, every raise of a bound needs `--raise`, and the worker logs chunk times to 0.01 s.  Stats: 5 rounds, 43 agents, 2,668,176 tokens, 106.6 wall minutes, 0 interventions; 2 validators of round 3 failed on an API error (403 from Cloudflare) and ran again.  Rounds 4 and 5 fixed 2 findings, both tests of the comparison.  Debt: TD-101 is new.
* [ ] M5: a flash-style time-axis attention.
* [ ] M6: overlap on the worker.

## Surprises and discoveries

* 2026-11-12: the session runs Opus 5.5 (`claude-opus-5-5`, the system prompt) at effort `high` (`modelSettings` for `claude-opus-5-5` in `~/.claude/settings.json`, over `effortLevel: xhigh`).
* 2026-11-12: Safari has `timestamp-query`, so `?profile=1` gives GPU times in Safari as in Chrome.
* 2026-11-12, M1: the worker's WebAssembly memory at the default chunk is now 87 MB from chunk 1 to the end, a growth of 57 MB (95 MB before).  The tab total is 799 MB in Chrome and 758 MB in Safari.  After the cuts the heap has little free space.  So a buffer of a new size, such as a shorter last chunk, grew the worker by 19 MB until M1 read and padded each chunk at one size (`plans/completed/gpu-speed-m1.md`, Step 8).  M6 keeps its buffers at fixed sizes for the same reason.
* 2026-11-12: Chrome's calibration chose split attention on the time axis (split 1.90 s, fused 2.26 s for each chunk).  Safari's chose fused (split 3.43 s, fused 3.27 s).  `docs/ARCHITECTURE.md` says Apple GPUs choose fused, which holds only for Safari.
* 2026-11-12, M2: the perf baselines of the M2 Pro, from 5 runs in each browser (`xtask/perf-baselines.json`, `plans/completed/gpu-speed-m2.md`).  The gate reads the chunk lower quartile: the lower quartile of chunks 2 to N, by nearest rank.  Chrome 157: 4.70 s, spread 0 %, tolerance 10 %.  Safari 27.2: 7.40 s, spread 0 %, tolerance 10 %.  On the chunk median, the first 5 runs gave tolerances of 22 % and 28 %: the chunk times have a floor and groups of slow chunks.  The cost of a forced kernel flip: Chrome `attn=fused` 2.1 %, `mm=v64,i64` 0 %; Safari `attn=freq` 9.5 %, `mm=v64,i64` 0 %.  Profile groups (GPU seconds, one pass for each dispatch): Chrome time-axis attention 20.03 s (43.6 %), frequency-axis attention 1.81 s (3.9 %), linear products 22.02 s (47.9 %), other 2.10 s (4.6 %); Safari 36.93 s (47.2 %), 2.73 s (3.5 %), 35.32 s (45.2 %), 3.20 s (4.1 %).

* 2026-11-12, M1 round 4 and M2 round 5: twice a wire reviewer changed a line in another reviewer's worktree, then reverted it.  In M2 it came after a failed `git worktree add` on a name in use, when the next commands of the chain ran in the wrong folder.  The validators ran each claim again in their own worktrees, and `review check` printed "no change".  TD-34 covers edits in worktrees; review agents should stop a command chain on a failed `worktree add`.

* 2026-11-13, M3: why Safari 27.2 is slower than Chrome 157 on the same WGSL, on the M2 Pro.  The Surprises of `plans/completed/gpu-speed-m3.md` hold the tables.  The logs and `m3/findings.md` stayed in the session scratchpad.  The controls are near 1 (FMA 1.05 to 1.08, copy 1.03 to 1.04), so the GPU's state does not explain it.  `v64`'s browser ratio is 1.76 on `freq qkv` with f16 weights (the parent's 1.70).  It falls as K grows: 2.4 at K 64, 1.76 at K 256, 1.45 at K 1,024.

  | Part of Safari's loss | Evidence | Share of `v64`'s loss |
  | - | - | - |
  | WebKit sets workgroup memory to zero with thread 0, byte by byte | Chrome's MSL with WebKit's zero-init and clamps, timed through Metal: 1.49 times at K 256, 1.13 at K 1,024 | the emulation's estimate: 64 % at K 256, 30 % at K 1,024.  Other estimates at K 256: 55 % (Step 3's fit), about 34 % (`v64-g4`), about 26 % (the `wg10k` control).  The `v64-g4` estimate assumes that `v64-g4`'s own extra cost is the same in both browsers.  Safari's time with `v64-g4` at K 256 did not go down (1.014 to 1.033 times `v64`).  The browser ratio fell because Chrome got about 1.20 times slower |
  | Bounds clamps | 2 % to 5 % in each browser (`v64-clamp2`, robustness off in Chrome) | in the row above |
  | A K loop about 1.35 times slower | Step 3's fit; not given by any tested form | the rest; most of fused attention's 1.52 |
  | Ruled out | barriers, `unpack2x16float`, the GPU's state, the thread maximum of Safari's MSL, a load of 64 registers; fast math and the loop guard by source | |

  Two variants are slower than `v64` in both browsers.  An unrolled inner loop: 1.49 to 1.93.  128 x 128 with 8 x 8 for each thread: 1.30 to 1.86.  At K 256, four tiles for each workgroup (`v64-g4`) is slower in Chrome (1.20), and not faster in Safari (1.014 to 1.033).  At K 1,024 it is slower in both: Safari 1.34 to 1.35, Chrome 1.25 to 1.27.  `s64`'s 10 times in Safari is its loop-indexed private array: with constant indices it runs at `i64`'s rate.  Time-axis attention in Safari: fused 3.47 s for each chunk, split 3.53 s; in Chrome 2.29 s and 1.85 s.

* 2026-11-13, M4 (`plans/completed/gpu-speed-m4.md`, Surprises, holds each table with its logs).  Safari 27.2 and Chrome 157.0.8103.51 on the M2 Pro, the same in each set and comparison.

  Step 4, the `wg` controls (almost no work, at `v64`'s grid of 2,064 workgroups), us for each dispatch, the range over 2 pages of each browser, at the start and the end:

  | Control | Safari | Chrome |
  | - | - | - |
  | `wg10k` (10 KiB) | 408.2 to 414.1 | 17.8 to 18.2 |
  | `wg6k` (6 KiB) | 113.0 to 114.5 | 14.0 to 14.3 |
  | `wg0` (none) | 10.7 to 11.3 | 6.3 |
  | `(wg6k - wg0) / (wg10k - wg0)` | 0.257 in each | 0.664 to 0.690 |

  So the zero-init cost is not linear in the bytes: in Safari, 6 KiB costs about a quarter of what 10 KiB costs (`references/browser-quirks.md`).

  Step 5, the same-session comparison (`e2e perf --base-dist`): `T` = 1 % and `N` = 1.1 %, from an A/A session (table below), checked by a second A/A comparison.

  Step 7, the in-page ratios (`?bench`, 2 pages of each browser), the candidate's calibration total over `v64`'s, with f16 weights:

  | Candidate | Safari large | Safari small | Chrome large | Chrome small |
  | - | - | - | - | - |
  | `v64k8` (BK 8, 6 KiB) | 0.944 to 0.946, gain | 0.993, none | 1.311 to 1.318, loss | 1.006, none |
  | `r64` (no workgroup memory) | 1.202 to 1.208, loss | 0.981 to 0.982, none | 2.537 to 2.545, loss | 1.058, none |

  `v64k8` is 0.91 to 0.92 times `v64` on the K 256 products in Safari, and 1.01 to 1.02 on `time ff down` (K 1,024).  `r64` is 0.57 times `v64` on `time gates` in Safari, which the small total, mostly `time p·v`, does not show.

  Steps 5, 9, 10 and 12 (`r`: the change of a block; base and head of one commit for an A/A):

  | Step | Chrome | Safari |
  | - | - | - |
  | 5, first A/A, `r` of 3 blocks; runs of a side apart | +0.11, -0.11, 0.00 %; up to 0.21 % | +0.14, -0.27, -0.20 %; up to 0.54 % |
  | 5, second A/A (`--runs 4`) | 0.00, 0.00 %: no change | +0.34, +0.07 %: no change |
  | 9, `v64k8` (base `a5e6a4c`, head `a238db5`) | 0.00, -0.21 %: no change; head `v64,v64` | spoiled, -2.44, -2.37 %: gain; head `v64k8` |
  | 10, cost of a forced flip | small products: +0.5 % (`v64k8`), +0.6 % (`i64`), +1.8 % (`s64`) | split attention against fused, with `v64k8`: +3.4 % (medians 7.47 and 7.225 s) |
  | 11, baseline | no change (4.70 s) | 7.40 s to 7.38 s, from 5 runs |
  | 12, `memory-budget`, 3 runs | worker growth 57 MB in each; check 3's margin 6.16 MB | the same |

  In Safari, with `v64k8`, calibration times split attention at 3.11 to 3.16 s and fused at 3.42 to 3.51 s for each chunk: at `FUSED_MARGIN`.  It chose split in 3 of 11 runs of Steps 9 and 10, and in 3 of the 5 runs of Step 11.  Those runs took 7.38 to 7.52 s, against 7.21 to 7.36 s with fused.  At the base, calibration gave fused / split 0.97, and a forced split cost 9.5 % (M2).  So in both, calibration's split time is about 0.5 s for each chunk too low.

  Step 12: `memory-budget` passed 3 times in Chrome and 3 times in Safari at `29d8567`.  The worker grew by 57 MB in each of the 12 separations, as at M1, so check 3's margin is 6.16 MB in each (budget 63.16 MB).  The tab total was 799 MB in Chrome and 758 to 801 MB in Safari, at most 943 MB.  `roformer/` has no change since M1, so `working_set`'s peak is still 80.16 MB and the smallest lost-cut rise is still 9.44 MB.  `WORKER_HEADROOM_MB` stays -17 MB.

* 2026-11-13, M4 review round 1: Safari's comparison of `v64k8` ran again with `--runs 8` and the rule for kernels that differ (base `a5e6a4c`).  Verdict: a gain, the mean `r` of 4 blocks -2.15 %, with split attention in 1 of 8 head runs (`plans/completed/gpu-speed-m4.md`, Surprises).

* 2026-11-13, CI run 43314717211 at eb81ed0: the macOS job failed in `trunk build`, because the `wasm-opt` that Trunk downloaded could not load `libbinaryen.dylib`.  The runs after it passed with no change, so it was the runner's download.  If it comes back, it gets a TD.

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
* 2026-11-13, agent (team leader of M3): M5 keeps the fused kernel's structure, measures the cost of the query reloads before it removes them, and does not unroll.  Why: an unrolled form was 3.6 times slower in Safari, and the zero-init is about 2 % of a fused dispatch.

* 2026-11-13, agent (orchestrator), intent review of M3, round 1: M5 loads each query block once only if it fits GPU-1, and measures the reloads first (i1-1).  M4's rule is fewer bytes for each workgroup on the same grid, with a 6 KiB `wg` control first, since `v64-g4` and `v128` did more work for each byte and were not faster (i1-2).  The drift between sessions is up to about 5 % (i1-3).  M3's range starts at 1be7a18 (i1-5).
* 2026-11-13, agent (orchestrator), intent review of M3, round 1 (i1-4): no milestone looks for the cause of Safari's slower K loop.  A dump of Safari's MSL changes the maintainer's Safari settings (`com.apple.WebKit.WebGPU.TogglePrintMetalCode`), so it goes to the PR's "Needs your attention".
* 2026-11-13, agent (orchestrator), intent review of M3, round 2: a kernel gain under 3 % is shown by its in-page `?bench` ratio; the same-session comparison shows no loss (i2-1).  M5 names each change from `attention_tiled.wgsl` with its predicted gain, and lands no kernel if none is faster (i2-2).  BK 8's gain is 6 % to 11 % of a K 256 product (i2-3).  The drift is up to about 5.7 % (i2-6).  The quirk entry of `browser-quirks.md` that still says more work for each byte loses less (i2-4) goes to M4's team, which cites it, as its first doc fix.
* 2026-11-13, agent (orchestrator), intent review of M3, round 3: rounds 1 to 3 each found a new case in the rule for a gain (i1-2, i2-1, i3-1, i3-2).  So it is now the goal "A change lands only with a shown gain", with cases, and M4's milestone plan designs the rule.  M6 case 9 uses it (i3-1).  Each current kernel's WGSL stays the same byte for byte, since calibration does not protect the Radeon and the iPhone from a changed old kernel (i3-3).  M4's first commit fixes the quirk entry (i3-5).  The `wg10k` control needs patch 0001 too (i3-4).
* 2026-11-13, agent (orchestrator), intent review of M3, round 4 (i4-1): case 3 of the gain goal uses M3's rule for a ratio in one page, not the rule for comparisons across pages, which would reject a real gain of BK 8.
* 2026-11-13, agent (orchestrator), M4: the worker logs the chunk times with 2 decimals, and the same-session comparison reads each run's chunk lower quartile.  This reverses "Not taken: chunk times in 2 decimals" (intent review of M2).  Why: groups of slow chunks add about 9 % to a Chrome run's whole time and leave the lower quartile, and at 0.1 s the lower quartile moves in steps of 1.4 % to 2.1 %.
* 2026-11-13, agent (team leader of M4): the same-session comparison is `e2e perf --base-dist`: blocks of base, head, head, base; `r = (H1 + H2) / (B1 + B2) - 1` for each block; a block whose two runs of a side differ by more than `N` is spoiled.  The verdict is the class of 2 blocks or more that are not spoiled, with at most one block of another class and not a loss and a gain together, up to 4 blocks; else undecided.  `T` = 1 % and `N` = 1.1 %, from an A/A session, checked by a second one.  When the runs of a side chose different kernels (review rounds 1 and 2), every block runs: 4, or `--runs` / 2 when that is more.  A block is spoiled only when the two runs of a side chose the same kernels and differ by more than `N`.  The verdict is the class of the mean `r` of the blocks that are not spoiled.  A comparison uses `--runs 8` where calibration has a near choice.
* 2026-11-13, agent (team leader of M4): a kernel lands only with a gain on a kind (large or small) with f16 weights, in at least one browser: its calibration total over the reference kernel's, `R`, is under 1 by more than the largest spread, in both pages.  Then each browser's comparison shows no loss, and each browser that chose it has the gain (`plans/completed/gpu-speed-m4.md`, Step 7).
* 2026-11-13, agent (implementer of M4): `v64k8` lands (`CALIBRATION` 4).  Safari chooses it; Chrome keeps `v64`.  `r64` does not land: no gain in either browser.
* 2026-11-13, agent (implementer of M4): the tolerance of the gate stays 10 %, and `FUSED_MARGIN` stays 1.1.  Why: Safari's flip to split attention costs 3.4 %, 9.1 % with the drift.  A wider margin would change the time-axis choice on GPUs that this Mac cannot measure.  The cause of calibration's low split time in Safari goes to M5.
* 2026-11-13, agent (orchestrator): v64k8 stays though Safari's calibration chooses split attention in about 6 of 16 calibrations, about 1 % slower than the base; M5 finds why calibration times split too low.  The comparison of review round 1 (`--runs 8`) gives a gain in Safari: the mean `r` of 4 blocks is -2.15 % (TD-101).

* 2026-11-13, agent (orchestrator), intent review of M4, round 1: M5's acceptance closes TD-101: each attention policy that Safari's calibration chooses at M5's head is not slower than fused at `a5e6a4c` (i1-1).  M5's in-page reference is each policy that calibration chooses at its base, and a Safari ratio against split does not count until TD-101 is fixed (i1-2).  M6 first makes the chunk time cover the input read (case 10, i1-3).  Each flip run pins the other choices (i1-4).  Safari draws split in about 10 of 22 calibrations of Steps 9 to 11, with the forced-`mm` runs; the 6 of 16 left them out.
* 2026-11-13, agent (orchestrator), intent review of M4, round 2: M5's TD-101 check forces each chosen policy on the head with a new `--head-query`, since the mean of mixed runs hides a slow policy (i2-1).  Calibration's bias toward split shows in Chrome too, so no ratio against split counts in either browser, and M5's first commit fixes TD-101's text (i2-2).  M6 keeps calibration's timing in step with `Pace`, and checks TD-101 again (case 11, i2-3).  Case 10's false change is up to 3 % to 6 %, and its commit measures itself (i2-4, i2-5).
* 2026-11-13, agent (orchestrator), intent review of M4, round 3: rounds 1 to 3 each found a new case in M5's TD-101 acceptance (i1-1, i2-1, i3-1).  So it is now the goal "Each calibration choice is not slower", with cases, which M5 designs and M6 runs again: each policy is forced alone on one page, the check fails first on `ab40862`, the names are `fused` and `freq`, never `split` (i3-2), and at M6's head it runs on the head page alone (i3-3).
* 2026-11-13, agent (orchestrator), intent review of M4, round 4 (i4-1): the check of "Each calibration choice is not slower" covers M5's candidate too, since calibration chooses it by the same timing.  The cost of `freq` is 2.3 % to 3.4 %.
## Outcomes and retrospective

Not yet.
