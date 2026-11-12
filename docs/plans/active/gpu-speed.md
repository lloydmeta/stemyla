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

Baselines for each adapter and browser are in a committed file.  The gate fails when the median is more than the tolerance above the baseline.  The tolerance comes from at least 5 runs in each browser, and is never under 10 %, so that the gate is not a nuisance (the maintainer, 2026-11-12).  A run that is faster lowers the baseline only through an explicit update.  `check --tier push` runs the gate on a change to `roformer/`, `roformer-gpu/` or `worker/` when the weights are set; else it prints that the gate did not run.  Acceptance: tests of the parse and of the gate fail first.  The baselines hold the M2 Pro in Safari and Chrome, with the spread in Surprises.

### M3: spike on Safari's overhead

Find why Safari runs the same WGSL 1.4 to 1.7 times slower than Chrome.  Time variants of `v64` and of fused attention in `?bench`, in each browser: more outputs for each thread (8 x 8), and index forms that need no clamp.  The result is a table in Surprises and the design of M4 and M5.  A cause in WebKit gets an entry in `docs/references/browser-quirks.md`.  This milestone only measures: if it lands no code, it has no review rounds.

### M4: a larger register-tiled matmul

A new `TILES` entry, with its f16 variant, from M3's result.  The start point is a 128 x 128 tile, 8 x 8 outputs for each thread and vec4 reads: (128 + 128) x 8 x 4 B = 8 KiB of workgroup memory.  Each output keeps its sum order, so its f16 variant gives the f32 kernel's output bit for bit (decision 0022).  Calibration times it with the others; bump `CALIBRATION`.  Acceptance: `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` on this Mac.  `e2e selftest --query mm=<kernel>` passes in Safari and Chrome.  The perf gate passes, and the baselines go down if the GPU chooses it.

### M5: a flash-style time-axis attention

A new fused attention kernel for the time axis (1,151 tokens, 8 heads of 64), from M3's result.  It keeps the softmax state on chip, so no score buffer goes to memory.  It loads each query block once.  It is a third `AttentionPolicy` candidate beside split and the current fused kernel.  It is checked against the CPU, as the current fused kernel is.  Acceptance: as M4, with `--query attn=<policy>`.

### M6: overlap on the worker

Two changes in `worker/src/web.rs`, each measured:

* `Pace` keeps up to 2 steps in flight: it submits a step before it waits for the step before.  The step limit and the errors still stop a run.
* The CPU work of one chunk runs while the GPU runs the next: the STFT of chunk n + 1 and the inverse STFT of chunk n.

The stems stay the same byte for byte: only the order of the work changes.

Memory, from M1 (`plans/completed/gpu-speed-m1.md`, Step 6):

* The worker's working set is the function in `roformer::separate`, which `memory-budget` and the worker's wasm test use.  M6 adds the buffers that it holds as named parts of that function, in the same commit as the code.  The next chunk's input, spectra and features are about 42.5 MB at the default chunk.
* `Demixer::end_chunk` moves the read position.  So the next chunk's input cannot be read before every stem of this chunk is accepted.  M6 gives the demixer a read cursor apart from the accept cursor.
* The limit is MEM-4: the tab total of `memory-budget` stays at most `TAB_BUDGET_MB` (943 MB).  If it does not, M1's Step 6 names the next cuts, in order of cost.

Acceptance: `e2e stored-stems` gives the same hashes before and after.  The perf gate in Chrome shows the gain, and Safari is not slower.  `memory-budget` passes with M1's checks, with M6's parts in the working-set function.

## Validation and acceptance

* `e2e perf` passes its gate in Safari and Chrome on this Mac, with the new baselines in the committed file.
* `e2e memory-budget` passes in Safari and Chrome at the default chunk.
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

None.  Safari has Develop > Allow Remote Automation on.  The weights and the arm64 chromedriver are on this Mac, outside the repo (ASSET-1); the kick-off prompt names their paths.  Runs on the Radeon Mac and the iPhone are optional, after the PR: the run never waits for them (the maintainer, 2026-11-12).

#### Preflight

The skill's preflight, and:

* `caffeinate -dimsu` runs for the length of the run.
* `cargo run -p xtask -- setup --check` passes.
* The weights file and the chromedriver of the kick-off prompt exist.

#### Decision points

See "Decisions for the maintainer".

#### Review rule

Decision 0024, through the `review-rounds` skill.  M3 has no review rounds if it lands no code.

#### Kick-off prompt

```text
/orchestrate docs/plans/active/gpu-speed.md.  Weights: ~/Downloads/bs_roformer_sw.f16.safetensors.  CHROMEDRIVER: ~/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver.
```

#### Extra stop conditions

None beyond the skill's.  The orchestrator decides these cases, records them in the Decision log, and goes on:

* A kernel that is not faster on this Mac in either browser does not land.  Record the figures in Surprises.
* A cause of Safari's overhead in WebKit gets a `browser-quirks.md` entry and a draft WebKit bug in the PR for the maintainer to file.
* A browser update between runs changes the baselines.  Measure again, and record both figures.

### Decisions for the maintainer

1. Which WebGPU limits and features may the new kernels use?  Options: the default limits only; the adapter's limits with a fallback kernel (amends GPU-1); also `subgroups` (Chrome only).  Recommendation: the default limits only, because the measured bottlenecks fit in them.  Answer (2026-11-12): the default limits only.
2. How strict is the perf gate?  Options: a gate on this M2 Pro, with Radeon and iPhone runs at the end; a Radeon run before each kernel default; numbers only.  Recommendation: the M2 Pro gate.  Answer (2026-11-12): the M2 Pro gate, with tolerances that are not too tight.  Do not wait on the maintainer for anything on the Radeon or the iPhone.
3. Does the worker overlap GPU steps (up to 2 in flight)?  Recommendation: yes, measured, and dropped if Safari gains nothing and Chrome gains under 5 %.  Answer (2026-11-12): yes, measured.
4. May the run push and keep a draft PR?  Answer (2026-11-12): yes, a draft PR into `main`.
5. `memory-budget` at the default chunk.  The maintainer asked on 2026-11-12 for "a good, real solution" to the failed growth check.  The design (M1) is an agent decision, below.

## Progress

* [x] 2026-11-12: baselines measured (Context), the maintainer's decisions recorded, the plan written.
* [x] 2026-11-12: M1: `memory-budget` at the default chunk (`22cdd74..156324a`, plan `plans/completed/gpu-speed-m1.md`, reviews `docs/reviews/gpu-speed-2026-11-12-m1.md`, 8 rounds).  `e2e memory-budget` runs at the defaults with four checks: flat after chunk 2, no growth with the song, a working-set budget (`roformer::separate::working_set` plus a measured headroom of -17 MB), and a tab total of at most 943 MB (MEM-4).  It passes in Chrome and Safari.  Four bit-identical cuts and a read fix took the worker's working-set peak from 117.86 to 80.16 MB and its growth from 95 to 57 MB; the tab total is 799 MB (Chrome) and 758 MB (Safari).  `stored-stems` hashes did not change.  Stats: 8 rounds, 51 agents, 2,863,709 tokens, 107.2 wall minutes, 0 interventions; rounds 4 to 8 fixed 6 findings, all in one new GPU test, its doc and TD-8.  Debt: TD-11 lost its chunk bullet; TD-8 gained a line on what the new GPU tests do not check.
* [ ] M2: the perf scenario and its gate.
* [ ] M3: spike on Safari's overhead.
* [ ] M4: a larger register-tiled matmul.
* [ ] M5: a flash-style time-axis attention.
* [ ] M6: overlap on the worker.

## Surprises and discoveries

* 2026-11-12: the session runs Opus 5.5 (`claude-opus-5-5`, the system prompt) at effort `high` (`modelSettings` for `claude-opus-5-5` in `~/.claude/settings.json`, over `effortLevel: xhigh`).
* 2026-11-12: Safari has `timestamp-query`, so `?profile=1` gives GPU times in Safari as in Chrome.
* 2026-11-12, M1: the worker's WebAssembly memory at the default chunk is now 87 MB from chunk 1 to the end, a growth of 57 MB (95 MB before).  The tab total is 799 MB in Chrome and 758 MB in Safari.  After the cuts the heap has little free space.  So a buffer of a new size, such as a shorter last chunk, grew the worker by 19 MB until M1 read and padded each chunk at one size (`plans/completed/gpu-speed-m1.md`, Step 8).  M6 keeps its buffers at fixed sizes for the same reason.
* 2026-11-12: Chrome's calibration chose split attention on the time axis (split 1.90 s, fused 2.26 s for each chunk).  Safari's chose fused (split 3.43 s, fused 3.27 s).  `docs/ARCHITECTURE.md` says Apple GPUs choose fused, which holds only for Safari.

## Decision log

* 2026-11-12, maintainer: the answers to "Decisions for the maintainer", 1 to 4.
* 2026-11-12, agent (orchestrator): `memory-budget`'s growth check is replaced, not widened (M1).  Why: at the default chunk the worker grows by one chunk's working set and then stays flat, so a growth of 4 MB tests the wrong thing.  The checks that MEM-4 needs are flat after warm-up, no growth with the song, a working-set budget and a tab total.
* 2026-11-12, agent (orchestrator): no f16 arithmetic.  Why: on M1 and M2 GPUs, f16 FMA runs at the f32 rate, and decision 0022 keeps f32 sums.
* 2026-11-12, agent (orchestrator): no `subgroups` and no subgroup matrices.  Why: Safari 27.2 offers neither, and the maintainer chose the default limits and features (decision 1).
* 2026-11-12, agent (orchestrator): the Radeon and the iPhone are protected by calibration, not by required runs.  Each new kernel is a candidate that a GPU keeps only if it measures faster there.
* 2026-11-12, agent (orchestrator): M1's worker headroom is -17 MB, not the team's -22 MB.  Why: -22 MB left 1.16 MB above the noise, which the maintainer's "not too tight" rules out.  -17 MB is midway between the noise (2.16 MB) and the smallest loss that it must catch (9.44 MB, a lost cut).
* 2026-11-12, agent (orchestrator): from M1 review round 6, each test doc in this plan's diffs says only what its test asserts, as a list.  Why: rounds 4 and 5 each found a new overclaim in one test's doc.

## Outcomes and retrospective

Not yet.
