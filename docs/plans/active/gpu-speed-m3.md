# GPU speed, M3: spike on Safari's overhead

## Purpose

This is the milestone plan for M3 of `plans/active/gpu-speed.md`.  The parent's M3 section stays the criteria.

Safari 27.2 runs the same WGSL 1.4 to 1.7 times slower than Chrome 157 on the maintainer's M2 Pro.  M3 finds why, with measurements, and finds a kernel design that is fast in both browsers.  M4 (a larger register-tiled matmul) and M5 (a flash-style time-axis attention) start from that design.

M3 only measures.  The variants that it times live on a scratch branch in a scratch worktree, outside the commits of `gpu-speed/2026-11-12`.  The branch gets docs only: this plan, a `browser-quirks.md` entry if the cause is in WebKit, and (through the orchestrator) the parent plan's Surprises and Decision log.

Readers: the implementer, the orchestrator, and the team leaders of M4 and M5.

## Context and orientation

The state at the start, on 2026-11-13, at commit `1be7a18` on branch `gpu-speed/2026-11-12`.  The review base is `7ec4d06`.  It holds the orchestrator's CI commit `1be7a18` (TD-98), which needs a review.

### The decisions that M3 follows

* The maintainer, 2026-11-12: kernels stay within WebGPU's default limits (16 KiB of workgroup memory, 256 invocations) and default features.  No `subgroups`.  Never wait for a run on the Radeon or the iPhone.
* The parent plan: M3 only measures.  If it lands no code, it has no review rounds of its own.  But `1be7a18` still needs one review: then the team leader runs `/review-rounds 7ec4d06` from round 1 on the CI commit, to the stop rule.  If M3 lands code, the review base `7ec4d06` covers both.
* The parent plan: no baseline in `xtask/perf-baselines.json` changes before M4's `--raise` change.  A browser update only records its figures in Surprises.
* The orchestrator, 2026-11-13: the temporary variants can live in a scratch worktree or on a scratch branch, unless they are worth keeping.  Each kernel that lands fits the default limits (GPU-1) and passes `every_kernel_fits_the_default_limits`.
* The parent plan: each test doc says only what the test asserts, as a list.  The simplest experiment that answers the question.
* A fact from outside the repo goes in with its source and the date checked.

### What is known (parent plan, Context and Surprises)

* The linear product 5456 x 1536 x 256 (`freq qkv`): 6.03 s in Safari, 3.54 s in Chrome (the parent's Context; the orchestrator's brief of 2026-11-13 gives `v64`, f16 weights, for the 11 chunks of the 1-minute song).  A ratio of 1.70.
* `s64`, which keeps its sums in a loop-indexed private array, is 9.6 times slower in Safari.  `docs/ARCHITECTURE.md` says such an array was 5 to 6 times slower on Intel and Apple GPUs.
* Time-axis attention for each chunk: Safari fused 3.27 s (split 3.43 s); Chrome split 1.90 s (fused 2.26 s).
* M2's profile groups (GPU seconds for one separation, Chrome against Safari): time-axis attention 20.03 against 36.93, frequency-axis attention 1.81 against 2.73, linear products 22.02 against 35.32, other 2.10 against 3.20.  So "other" (softmax, rmsnorm, element-wise: few multiply-adds for each memory read) is also 1.5 times slower.
* Gaps between steps: 14 % of step time in Chrome, 1 % in Safari, with profiling.  So dispatch overhead does not explain Safari's loss.
* The kernels use 13 % to 20 % of the GPU's peak of about 6.8 TFLOP/s f32.

### The hypothesis, and the other candidates

The hypothesis, not proven: WebKit's WGSL compiler adds bounds checks to index accesses.  They cost most where a kernel does few multiply-adds for each memory read, and they stop a private array from going into registers.

A first read of WebKit's `Source/WebGPU/WGSL/BoundsCheck.cpp` (main, 2026-11-13, [raw file](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebGPU/WGSL/BoundsCheck.cpp)) says this.  Step 1 confirms it at a fixed commit.

* An index access with a constant index gets no check.
* Each other access gets a clamp, `__wgslMin(index, size - 1)`: the array's size for a fixed-size array, `arrayLength` for a runtime-sized array, the size for a vector or a matrix.
* The pass does not look at the address space: workgroup, private, function and storage arrays all get the same clamp.

Chrome's compiler, Tint, also clamps indices by default (its robustness transform).  So the clamp alone does not explain a difference.  The difference can be in where each compiler clamps, in what else each one emits, or in the Metal compile options of each browser.  The other candidates:

* Workgroup memory set to zero at the kernel's start.
* The cost of a barrier.
* Loop unrolling, and the code that each compiler emits around loops.
* The cost of `unpack2x16float` (f16 weights).
* Fast math, or other options that each browser gives to the Metal compiler.
* Loop bounding: a guard that a compiler adds to each loop against an endless loop (wgpu calls it `force_loop_bounding`).
* The GPU's state in each browser: clock, power and queue priority.  `e2e` runs Chrome headless (`--headless=new`, `Page::open` in `xtask/src/e2e.rs`) and Safari in a window, which can also change it.  A kernel with no index access in its loop tells this apart from the compiler.
* Dispatch overhead.  The gaps above rule it out for the time between steps, but not for the start of each dispatch.

### The code today

* `roformer-gpu/src/lib.rs`: `TILES` has `i64` and `s64` (64 x 64, 4 x 4 outputs for each thread), `i128x64` (8 x 4) and `v64` (64 x 64, BK 16, vec4 reads, a pad of one vec4).  `Tile::source` generates `matmul_interleaved.wgsl`, `Tile::vec4_source` generates `matmul_vec4.wgsl`, and `Tile::f16_source` makes the f16 variant of each.  Fused attention is `attention_tiled.wgsl`, generated by `Attention` in `lib.rs`.
* `v64`'s inner loop (`matmul_vec4.wgsl`): for each `kv` in 0 to 3, it reads `as_[ab + c]` and `bs_[bb + c]`, with `ab = ty * SV + kv` and `bb = tx * SV + kv`.  Each read has a runtime index, so each gets a clamp in WebKit.  The K loop has two barriers for each 16 values of K.
* `roformer-gpu/src/bench.rs`: `measure` times each shape of `shapes()` with each kernel of `TILES` that passed `Gpu::check_kernels`, in f32 and f16, and logs `bench: <shape> (<m>x<n>x<k>, x<batch>) <tile>, <format> weights: <rate> GFLOP/s (median of <n>, spread <s>%)`.  The kernels and formats take turns inside each round.  `attention_per_chunk` times split and fused attention for each chunk on both axes.
* `worker/src/web.rs`, `bench`: `?bench=<names>` times only those kernels (`parse_kernels`); `bench_rounds=<n>` sets the rounds (default 3); `bench_quick=1` sets no warm-up.
* `xtask/src/e2e.rs`, `bench`: `cargo run -p xtask -- e2e bench --browser <b> --query <q> --dist <dir> --out <dir>` opens the page and waits for `bench finished`.  The default query is `bench&bench_quick=1&mm=s64`; `--query` replaces it.  Chrome gets `--enable-unsafe-webgpu` and, without `--headed`, `--headless=new`.  No option adds another Chrome argument.
* `roformer-gpu/tests/speed.rs`: `bench_runs` (ignored) runs `bench::measure` natively, through wgpu and naga's MSL on Metal.  `Gpu` makes each shader with `create_shader_module` (`shader` in `lib.rs`), so naga adds its own bounds checks.  wgpu 30.0.1 has `Device::create_shader_module_trusted` and `ShaderRuntimeChecks` (`bounds_checks`, `force_loop_bounding`, and more).
* Tests of the tiles in `lib.rs` that a new variant can break: `workgroup_bytes` reads only 1D `array<f32, N>` and `array<vec4<f32>, N>` declarations; `bank_conflicts` wants `[2, 2, 1, 1]` for each vec4 tile; a `hand_written` tile always gives `matmul.wgsl`.

### Terms

* Variant: a kernel that is timed in M3 only, on the scratch branch.
* Page: one browser session that runs `?bench` once.  Inside a page, the kernels take turns in each round.
* Set: the pages of one experiment, in the order Safari, Chrome, Safari, Chrome.
* Ratio: a variant's time over the time of its reference kernel in the same page.  The 4 % drift between sessions moves both alike, so a ratio is the main figure.  The browser ratio is Safari's time over Chrome's for the same kernel, from the pages of one set.
* An effect: a ratio whose distance from 1 is larger than the larger spread of the two figures, in both pages of a browser.  Else the result is "no effect seen", with the spread.  The bench logs each spread.  For a comparison across pages (a toggle, headed against headless, a control's browser ratio), the pages take turns, each figure is divided by the FMA control of its page, and an effect must be larger than 4 % plus the larger spread.

### The paths of this run

* The scratch worktree: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/wt`, on the local branch `scratch/m3-variants` from `1be7a18`.  Never push it (GIT-1).
* The logs: `<scratchpad>/m3/runs/<step>-<browser>-<n>.log`, where `<scratchpad>` is the folder above `m3/`.  Each figure in the tables names its log.  The scratchpad belongs to this session, so the tables of each step also go into this plan's Surprises at the end of the step.  A new session finds the figures there.
* Weights (Step 8 only): `/Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors` (git-ignored, ASSET-1).
* Chrome: `CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.
* Safari: through `safaridriver`, which `xtask` drives.  Never quit the maintainer's Safari.  Read `docs/references/local-browser-testing.md` first.
* Build the page of the worktree with `(cd <wt>/app && trunk build --release)`, and pass `--dist <wt>/app/dist` to `e2e`.  Run one browser GPU run at a time.  `caffeinate -dimsu` runs.

## Plan of work

The simplest experiments first: the sources, then two control kernels, then the variants.  Each step writes its figures to a table in `m3/findings.md` before the next step.  A step whose answer makes a later step pointless records that and skips it.

Not done, and why: no WebKit or Chromium build from source (hours of build for each, and the sources and the controls answer the question); no Metal frame capture of Safari's GPU process (it needs settings that change the maintainer's Safari); no `subgroups` (decision 1).

### Step 1: the sources, at fixed commits

Read these, and record each fact with the file, the commit SHA and the date checked.  Output: `m3/sources.md`, with a table of each access kind (fixed-size array with a runtime index, constant index, runtime-sized storage array, vector, private array, workgroup array) and the form that each compiler emits.

* WebKit, `Source/WebGPU/WGSL`: `BoundsCheck.cpp`; the Metal code generation in `Metal/` (how it emits `__wgslMin`, loops, workgroup zero-init and `arrayLength`); and where the WebGPU framework gives options to the Metal compiler (`Source/WebGPU/WebGPU/`, for example `fastMathEnabled`, `mathMode` or `MTLCompileOptions`).
* Dawn and Tint (dawn.googlesource.com): the robustness transform and what it skips; the MSL writer's loop code (for example a guard against an endless loop); Dawn's Metal compile options (`src/dawn/native/metal/`); the toggles `disable_robustness` and `dump_shaders` (`src/dawn/native/Toggles.cpp`).
* naga 30.0.1 (`naga/src/back/msl/`): its bounds-check policies, for Step 5.

### Step 2: the scratch worktree and its harness

1. `git worktree add <scratchpad>/m3/wt -b scratch/m3-variants 1be7a18`.
2. On that branch only:
   * Each matmul variant of Step 4 is a new `TILES` entry, with its f16 variant, so that the bench times both formats.  `s64-c` needs a source of its own, since a `hand_written` tile always gives `matmul.wgsl`: for example a source field on `Tile`.
   * Each attention variant of Step 6 is a new source timed in `attention_per_chunk`, beside split and fused.
   * Each attention variant and each control kernel goes into `generated_sources`, so that the limit checks see it.  New `TILES` entries are there already.
   * Two control kernels, timed in `?bench` with their own `bench: control ...` lines, in every page of Steps 3 to 6:
     * FMA: generated with no loop.  Each thread reads its inputs from a buffer, runs 8 or more independent chains of straight-line `fma`, and writes one result.  No index access between the read and the write.
     * Copy: each thread copies vec4s from one storage buffer to another, at 64 MB for the memory rate.  Also a near-empty dispatch (4 KiB), timed directly, for the cost of a dispatch.  Each size sends the same number of dispatches for each submit.
   * An environment variable in the scratch `xtask`, for example `E2E_CHROME_ARGS`, adds arguments to Chrome in `e2e bench`.
3. In the worktree, `every_kernel_fits_the_default_limits` (GPU-1) and `sources_are_complete` pass.  The formula and bank-conflict tests (`workgroup_bytes`, `bank_conflicts`) are changed on the scratch branch to accept the variants; that is not a pass of GPU-1.  Each matmul variant passes `Gpu::check_kernels` in each browser, or the bench logs it as skipped and the table says so.  Each attention variant is compared with the CPU (`attention_cpu`, as for the current fused kernel) in each browser before it is timed, and its workgroup memory is computed (the current kernel: `buf` 2,112 and `red` 1,024 floats, 12,544 B).
4. Commit on the scratch branch after each step, unsigned commits allowed (it is never pushed or merged).

### Step 3: the controls

One set (Safari, Chrome, Safari, Chrome) of `?bench=v64&bench_rounds=5` with the two controls.  Then one more Chrome page with `--headed`, as Safari runs in a window.  Record, for each browser: the FMA rate (GFLOP/s), the copy rate (GB/s) at each size, the cost of a dispatch, and `v64`'s rate on each linear shape.

The copy kernel's storage index gets a clamp in both browsers, so the copy shows the memory path, not a code without checks.  Read the controls' browser ratios against `v64`'s browser ratio (about 1.7):

* Controls near 1 (within their spreads): the GPU's state does not differ, and the loss is in the code that each compiler makes for the real kernels.
* Controls near `v64`'s ratio: the GPU's state or the dispatch explains the loss, not the code.  Steps 4 to 6 still run, but each browser ratio is read against the controls' ratio.
* Between the two: both have a part.  Give the part of `v64`'s ratio that the controls explain, and read the later steps against the rest.
* A headed Chrome page that differs from the headless pages by more than the spread: record it, and use headed Chrome for the later steps.

### Step 4: matmul variants

One set of `?bench=i64,s64,v64,<variants>&bench_rounds=5`.  The variants, each generated like `v64`:

* `v64-u`: `v64` with the inner `kv` loop unrolled.  Each read is then `as_[ab + c]`, with `ab = ty * SV` the same in every K step and `c` a constant.  The same number of clamps, with a loop-invariant runtime part.
* `v64-clamp2`: `v64` with one more clamp on each workgroup read, `min(index, max(p.k, SA_LEN - 1u))` (`SB_LEN` for `bs_`).  It gives the same index for any `k`, and no compiler can remove it, since `p.k` comes from the uniform buffer.  This measures what a clamp costs in each browser, Safari included, which Step 5 cannot do.
* `v64-bar`: `v64` with one more pair of `workgroupBarrier()` for each K tile.  The same memory and layout, so this measures the cost of a barrier.
* `v128`: 128 x 128, 8 x 8 outputs for each thread, BK 8, vec4 reads: (128 + 128) x (8 + 4) x 4 B = 12 KiB with a pad of one vec4, 16 x 16 threads.  M4's start point.  Twice the multiply-adds for each workgroup read of `v64`.
* `v128-u`: `v128` with the form of `v64-u`.  Only if `v64-u` shows an effect.
* `s64-c`: `s64` with its sums in the same private array, but every index a constant (loops over the array unrolled).  This removes the clamp and the loop index together, so it cannot tell the two apart.  It shows if the array itself is slow, and Step 5's toggles tell the clamp's part.

Record each variant's ratio to its reference (`s64-c` to `s64`, `v128-u` to `v128`, each other variant to `v64`) in each browser, on `freq qkv` and `time qkv` with f32 and f16 weights, and each browser ratio.  Also record each kernel's f16 / f32 ratio, and compare it between the browsers: f16 also halves B's reads, so only the difference between the browsers points to `unpack2x16float`.

### Step 5: the clamp's cost, alone

Tests that change only what a compiler adds, the cheapest first:

1. Chrome: pages of `?bench=v64,v64-u,s64,s64-c&bench_rounds=5`, taking turns twice (default, robustness off, zero-init off, default, robustness off, zero-init off): the default; `--enable-dawn-features=disable_robustness`; `--enable-dawn-features=disable_workgroup_init`.  Check each toggle's name in Dawn's `Toggles.cpp` first (Step 1).  If `dump_shaders` gives Tint's MSL, keep `v64`'s MSL in `m3/`.  Dawn can write it to its device log, not to the page's console that `e2e` reads; record where it went, or that it did not come.
2. Native, only if 1 does not settle the cost of the checks: a scratch test in `roformer-gpu/tests/speed.rs`, with its own pipeline code (`Gpu` makes its modules itself), times `v64`, `v64-u`, `s64` and `s64-c` on 5456 x 1536 x 256 through wgpu on Metal.  Four forms, built in one process and timed in turns: `ShaderRuntimeChecks::checked()`; the same with `bounds_checks = false`; the same with `force_loop_bounding = false`; `checked()` with `PipelineCompilationOptions::zero_initialize_workgroup_memory = false`.  Not `unchecked()`, which also turns off the other checks.

Record each ratio to the default.  Workgroup zero-init, if neither 1 nor 2 tests it: rule it in or out by arithmetic.  `v64` zeroes 10 KiB once for each workgroup, and stages 16 K tiles of 10 KiB for K = 256.

### Step 6: attention variants

At most two fused attention variants, from what Steps 3 to 5 found, each within the default limits (Step 2.3) and checked against the CPU.  For example: the index form that won in Step 4, applied to `buf` and `red`; or more outputs for each thread.  One set of `?bench=v64&bench_rounds=5` (the bench always times split and fused attention), with the variants timed beside them.  Record the time for each chunk on the time axis, and the ratio to fused, in each browser.  This informs M5; M5 designs the flash-style kernel.

### Step 7: the result

1. `m3/findings.md`: each table, each log's path, and the cause or the best-supported explanation.  An explanation names the steps whose figures support it, and each candidate that the figures rule out.  A candidate that no step tested stays "not tested".
2. The design that M4 and M5 start from: tile shape, outputs for each thread, the memory layout and index form, the number of barriers, and the attention structure.  Each choice names its figure.  The parent's M4 gives 8 KiB for the 128 x 128 tile, without a pad; with `v64`'s pad of one vec4 it is 12 KiB.  The diff of item 3 corrects it, or says why the pad goes.
3. `m3/parent-m3.diff` for the orchestrator, `git apply --check` passing on HEAD: a Surprises entry with the summary table, Decision log lines for the cause and the design, and the M4 and M5 sections changed where the design changes their start point.
4. If the cause is in WebKit: an entry in `docs/references/browser-quirks.md` in the form of the other entries (what was seen, where and when; the code; the rule), and `m3/webkit-bug.md`, a bug text for the maintainer to file.  The text gives a small WGSL case, the figures in both browsers, and the WebKit source lines with the commit.
5. `git -C <wt> format-patch 1be7a18..scratch/m3-variants -o <scratchpad>/m3/variants/`.  M4 and M5 can start from it.  Then remove the worktree and the scratch branch.
6. If a variant or the bench harness is worth keeping on the branch, do not commit it: say so in the report, with the reason.  The team leader decides, and then this plan gets a code step with tests that fail first, and review rounds over the code.

### Step 8: the checks before the review round

* `STEMYLA_PERF_WEIGHTS=<weights> CHROMEDRIVER=<driver> cargo run -p xtask -- check --tier push` passes, with both perf gates run (the changed paths from `origin/main` include `roformer-gpu/`).  Progress gives both gates' verdicts.  A skip line is not a pass (VERIFY-1).
* `cargo build -p xtask` passes.
* `git diff --stat 7ec4d06..HEAD` shows no path outside `docs/`, `.github/`, `xtask/src/e2e/save.rs` (the CI commit) and the paths that the orchestrator applies.

## Validation and acceptance

* `m3/findings.md` and this plan's Surprises hold a table of each step's figures, in each browser, each with its log in `m3/runs/`.  The browser ratio of `v64` on `freq qkv` with f16 weights is measured again in Step 4 and compared with the parent's ratio, 1.70.
* The cause, or the best-supported explanation, names its evidence: the source lines (file, commit, date) and the figures.  It says how much of `v64`'s browser ratio it explains.
* Each candidate of "The hypothesis, and the other candidates" is ruled in, ruled out, "ruled by source" (fast math, for example) or "not tested", with the reason.  "Ruled in" and "ruled out" follow the rule for an effect (Terms).
* The design for M4 and M5 is in the parent plan's Decision log (through the orchestrator), each choice with its figure.
* A WebKit cause has its `browser-quirks.md` entry and `m3/webkit-bug.md`.
* The branch gets no code from M3, unless Step 7.6 adds a code step.  Step 8's checks pass.
* The review rounds of the CI commit (`/review-rounds 7ec4d06`) end by the stop rule of decision 0024.

## Idempotence and recovery

Each experiment writes only to the scratch worktree and `m3/`.  A session that stops reads Progress and `m3/findings.md`, and goes on at the next open step.  A page that a locked screen or a Safari timeout stops runs again.  A set that a browser update splits runs again in full, and Surprises records both versions.  `git worktree remove` and `git branch -D scratch/m3-variants` undo Step 2.

## Progress

* [x] Plan review round 1 (2026-11-13): 9 should-fix and 4 nits, each applied.
* [x] Plan review round 2 (2026-11-13): 5 should-fix and 3 nits, each applied.
* [x] Step 1 (2026-11-13): the sources.  `m3/sources.md` gives each fact with its file, lines and commit.  The table is in Surprises.
* [x] Step 2 (2026-11-13): the scratch worktree and its harness, on `scratch/m3-variants` (`9a30c16`, `4cd30a2`, `047fbb8`).  The six matmul variants (with f16) and `attention-u` pass `every_kernel_fits_the_default_limits`, `sources_are_complete` and `workgroup_memory_within_defaults`.  `STEMYLA_REQUIRE=gpu,f16 cargo test --release --test kernels` passes natively (19 tests, `check_kernels_accepts_all` included).  `Gpu::check_kernels` accepted each variant in Safari 27.2 and Chrome 157 (`runs/smoke-*/bench.log`).  `attention-u` matches `attention_cpu` natively (relative error 6.4e-8).
* [ ] Step 3: the controls.
* [ ] Step 4: matmul variants.
* [ ] Step 5: the clamp's cost, alone.
* [ ] Step 6: attention variants.
* [ ] Step 7: the result.
* [ ] Step 8: the checks before the review round.
* [ ] Review rounds of the CI commit.

## Surprises and discoveries

* 2026-11-13, Step 1: the compiler sources, read on 2026-11-13 at WebKit `5447ffbd5817eb8a1de9c0540bb8a6fe37db555d`, Dawn `85b7901759827252adf542adf7a1d2858515b12e`, Chromium `dafa2a921b73b46a0cd9c687b3ae7724ce7983c2` (each `main`, 2026-11-12) and naga, wgpu-hal and wgpu-types 30.0.1.  Safari 27.2 and Chrome 157 ship from release branches, which can differ from `main`.  The full list with line numbers is `m3/sources.md`.  The implementer read the two WebKit facts in the last two rows again in the raw files.

  | Access or code | WebKit (Safari) | Tint (Chrome) | naga 30.0.1 |
  | - | - | - | - |
  | Fixed-size array, runtime index (private, function, workgroup) | `a[__wgslMin(i, N - 1)]` (`WGSL/BoundsCheck.cpp:220-261`) | `a[min(i, N - 1)]`; integer range analysis removes it when it proves `i <= N - 1` (`robustness.cc:233-290`, on by default in Chromium: `gpu/config/gpu_finch_features.cc:329-330`) | `a.inner[min(i, N - 1)]` |
  | Constant index into a fixed-size array | no clamp | no clamp | no clamp |
  | Runtime-sized storage array | clamp to `arrayLength - 1`, also for a constant index | clamp to `arrayLength - 1` | clamp from `_buffer_sizes` |
  | Vector | always a clamp, also for a constant index on a non-constant vector | a clamp, removed when proved | a clamp |
  | Turn the clamps off | no option (`WGSL/WGSL.cpp:147`) | `disable_robustness` | `bounds_checks = false` |
  | Loop guard | Apple8 (M2 Pro): a counter that is not `volatile` and never changes, so it folds away; Apple9 and later: a `volatile` counter (`MetalFunctionWriter.cpp:92-95`, `425-433`) | only on a loop that it cannot prove finite | on every loop (`force_loop_bounding`) |
  | Metal math | `MTLMathModeRelaxed`, fast functions (`WebGPU/ShaderModule.mm:47-72`) | `math_mode(relaxed)` on macOS 15 and later | Metal default |
  | Workgroup memory set to zero | thread 0 alone writes each byte, in a loop, then one barrier (`GlobalVariableRewriter.cpp:2552-2594`, `2666-2786`; `MetalFunctionWriter.cpp:1081-1095`) | each thread writes a part, then one barrier (`zero_init_workgroup_memory.cc:136-185`) | thread 0 writes each variable with `= {}`, then one barrier |
  | `[[max_total_threads_per_threadgroup]]` | not written, and not set on the pipeline descriptor (`MetalFunctionWriter.cpp:1787-1791`, `WebGPU/ComputePipeline.mm:43-55`) | written from the workgroup size (`printer.cc:428-434`) | written from the workgroup size |

  So the clamp has the same form in both browsers.  Three differences remain: Chrome removes the clamps that it can prove, Safari sets workgroup memory to zero with one thread, byte by byte, and Safari gives Metal no maximum number of threads.

## Decision log

* 2026-11-13, agent (team leader): the variants live on a local scratch branch in a scratch worktree, and a patch of them goes to `m3/variants/`.  Why: M3 only measures, and M4 and M5 add their kernels with tests.
* 2026-11-13, agent (team leader): a ratio inside one page is the main figure, and each experiment is one set of pages in the order Safari, Chrome, Safari, Chrome.  Why: the bench already takes turns inside each round, and the 4 % drift between sessions moves every kernel of a page alike (`plans/completed/gpu-speed-m2.md`, Surprises).
* 2026-11-13, agent (team leader): two control kernels (FMA and copy) run first.  Why: M2's "other" group is also 1.5 times slower in Safari.  If a kernel with no index access in its loop is slower too, the cause is not the bounds checks.

* 2026-11-13, agent (team leader), plan review round 1: `v64-2d` and `v64-bk32` are replaced by `v64-u`, `v64-clamp2` and `v64-bar`.  Why: the 2D form has the same number of clamps as an unrolled 1D form, and BK 32 without a pad adds 4-way bank conflicts (`bank_conflicts` in `lib.rs`).  Each new variant changes one thing.
* 2026-11-13, agent (team leader), plan review round 1: Chrome's toggles run before the native test, which runs only if they do not settle the cost.  Why: the toggles need no new pipeline code.

* 2026-11-13, agent (implementer), Step 2: three more controls run in every page beside FMA and copy.  `fma64` is the FMA control with 64 chains of 32, the same FMAs with 64 live values.  `wg10k` declares v64's 10 KiB of workgroup memory and does almost no work, at v64's grid on `freq qkv` (2,064 workgroups).  `wg0` is the same with no workgroup memory.  Why: Step 1 found two differences that FMA and copy do not test.  Safari sets workgroup memory to zero with one thread, byte by byte.  Safari gives Metal no thread maximum, which can limit the registers of each thread.
* 2026-11-13, agent (implementer), Step 2: `v128-u` is timed in the set of Step 4, and its figure is read only if `v64-u` shows an effect.  Why: a second set costs more than one more kernel in the first set.
* 2026-11-13, agent (implementer), Step 2: on the scratch branch, calibration chooses only among the four kernels of `main`, so split attention uses the same matmul as on `main`.  A build with `M3_SHAPES` set times only the shapes that it names.
* 2026-11-13, agent (implementer), Step 2: `attention-u` is built now: fused attention with each loop over `kk` and `k` unrolled, so each read of `buf` and `red` there is a loop-invariant base plus a constant.  Step 6 times it only if Step 4 supports the unrolled form.

## Outcomes and retrospective

Not yet.
