# GPU speed, M3: spike on Safari's overhead

## Purpose

This is the milestone plan for M3 of `plans/active/gpu-speed.md`.  The parent's M3 section stays the criteria.

Safari 27.2 runs the same WGSL 1.4 to 1.7 times slower than Chrome 157 on the maintainer's M2 Pro.  M3 finds why, with measurements, and finds a kernel design that is fast in both browsers.  M4 (a matmul with less workgroup memory for each unit of work) and M5 (a flash-style time-axis attention) start from that design.

M3 only measures.  The variants that it times live on a scratch branch in a scratch worktree, outside the commits of `gpu-speed/2026-11-12`.  The branch gets docs only: this plan, and a `browser-quirks.md` entry if the cause is in WebKit.  Through the orchestrator, it also gets the parent plan's Surprises and Decision log.

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

* The linear product 5456 x 1536 x 256 (`freq qkv`): 6.03 s in Safari, 3.54 s in Chrome (the parent's Context).  The orchestrator's brief of 2026-11-13 gives these for `v64`, f16 weights, and the 11 chunks of the 1-minute song.  A ratio of 1.70.
* `s64`, which keeps its sums in a loop-indexed private array, is 9.6 times slower in Safari.  `docs/ARCHITECTURE.md` says such an array was 5 to 6 times slower on Intel and Apple GPUs.
* Time-axis attention for each chunk: Safari fused 3.27 s (split 3.43 s); Chrome split 1.90 s (fused 2.26 s).
* M2's profile groups give GPU seconds for one separation, Chrome against Safari.  Time-axis attention: 20.03 against 36.93.  Frequency-axis attention: 1.81 against 2.73.  Linear products: 22.02 against 35.32.  Other: 2.10 against 3.20.  So "other" (softmax, rmsnorm, element-wise: few multiply-adds for each memory read) is also 1.5 times slower.
* Gaps between steps: 14 % of step time in Chrome, 1 % in Safari, with profiling.  So dispatch overhead does not explain Safari's loss.
* The kernels use 13 % to 20 % of the GPU's peak of about 6.8 TFLOP/s f32.

### The hypothesis, and the other candidates

The hypothesis, not proven: WebKit's WGSL compiler adds bounds checks to index accesses.  They cost most where a kernel does few multiply-adds for each memory read, and they stop a private array from going into registers.

A first read of WebKit's `Source/WebGPU/WGSL/BoundsCheck.cpp` (main, 2026-11-13, [raw file](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebGPU/WGSL/BoundsCheck.cpp)) says this.  Step 1 confirms it at a fixed commit.

* An index access with a constant index gets no check.
* Each other access gets a clamp, `__wgslMin(index, size - 1)`.  The size is the array's size for a fixed-size array, `arrayLength` for a runtime-sized array, and the size of a vector or a matrix.
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

* `roformer-gpu/src/lib.rs`: `TILES` has `i64` and `s64` (64 x 64, 4 x 4 outputs for each thread) and `i128x64` (8 x 4).  It also has `v64` (64 x 64, BK 16, vec4 reads, a pad of one vec4).  `Tile::source` generates `matmul_interleaved.wgsl`, `Tile::vec4_source` generates `matmul_vec4.wgsl`, and `Tile::f16_source` makes the f16 variant of each.  Fused attention is `attention_tiled.wgsl`, generated by `Attention` in `lib.rs`.
* `v64`'s inner loop (`matmul_vec4.wgsl`): for each `kv` in 0 to 3, it reads `as_[ab + c]` and `bs_[bb + c]`.  Here `ab = ty * SV + kv` and `bb = tx * SV + kv`.  Each read has a runtime index, so each gets a clamp in WebKit.  The K loop has two barriers for each 16 values of K.
* `roformer-gpu/src/bench.rs`: `measure` times each shape of `shapes()` with each kernel of `TILES` that passed `Gpu::check_kernels`, in f32 and f16.  It logs `bench: <shape> (<m>x<n>x<k>, x<batch>) <tile>, <format> weights: <rate> GFLOP/s (median of <n>, spread <s>%)`.  The kernels and formats take turns inside each round.  `attention_per_chunk` times split and fused attention for each chunk on both axes.
* `worker/src/web.rs`, `bench`: `?bench=<names>` times only those kernels (`parse_kernels`); `bench_rounds=<n>` sets the rounds (default 3); `bench_quick=1` sets no warm-up.
* `xtask/src/e2e.rs`, `bench`: `cargo run -p xtask -- e2e bench --browser <b> --query <q> --dist <dir> --out <dir>` opens the page and waits for `bench finished`.  The default query is `bench&bench_quick=1&mm=s64`; `--query` replaces it.  Chrome gets `--enable-unsafe-webgpu` and, without `--headed`, `--headless=new`.  No option adds another Chrome argument.
* `roformer-gpu/tests/speed.rs`: `bench_runs` (ignored) runs `bench::measure` natively, through wgpu and naga's MSL on Metal.  `Gpu` makes each shader with `create_shader_module` (`shader` in `lib.rs`), so naga adds its own bounds checks.  wgpu 30.0.1 has `Device::create_shader_module_trusted` and `ShaderRuntimeChecks` (`bounds_checks`, `force_loop_bounding`, and more).
* Tests of the tiles in `lib.rs` that a new variant can break.  `workgroup_bytes` reads only 1D `array<f32, N>` and `array<vec4<f32>, N>` declarations.  `bank_conflicts` wants `[2, 2, 1, 1]` for each vec4 tile.  A `hand_written` tile always gives `matmul.wgsl`.

### Terms

* Variant: a kernel that is timed in M3 only, on the scratch branch.
* Page: one browser session that runs `?bench` once.  Inside a page, the kernels take turns in each round.
* Set: the pages of one experiment, in the order Safari, Chrome, Safari, Chrome.
* Ratio: a variant's time over the time of its reference kernel in the same page.  The 4 % drift between sessions moves both alike, so a ratio is the main figure.  The browser ratio is Safari's time over Chrome's for the same kernel, from the pages of one set.
* An effect: a ratio whose distance from 1 is larger than the larger spread of the two figures, in both pages of a browser.  Else the result is "no effect seen", with the spread.  The bench logs each spread.  A comparison across pages is a toggle, headed against headless, or a control's browser ratio.  For it, the pages take turns, and each figure is divided by the FMA control of its page.  An effect must then be larger than 4 % plus the larger spread.

### The paths of this run

* The session scratchpad: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad`.  This folder belongs to one session, and is not kept.
* The scratch worktree: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/wt`, on the local branch `scratch/m3-variants` from `1be7a18`.  Never push it (GIT-1).
* The logs are in `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/runs/`.  Each page has three:
  * `<step>-<browser>-<n>/bench.log`: the bench log, in the folder that `e2e --out` names;
  * `<step>-<browser>-<n>.log`: a copy of that bench log;
  * `<step>-<browser>-<n>.stdout`: the output of `e2e`.
* The native runs of Step 5.2 have no browser part: `s5native-<n>.log`.  The check pages of Step 2 are in `smoke/` and `smoke-safari/`.
* Each figure in the tables names its log.  The scratchpad belongs to this session, so the tables of each step also go into this plan's Surprises at the end of the step.  The logs and `m3/findings.md` stayed in the session scratchpad, and are not kept.  This plan's Surprises hold the tables.
* Weights (Step 8 only): `/Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors` (git-ignored, ASSET-1).
* Chrome: `CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.
* Safari: through `safaridriver`, which `xtask` drives.  Never quit the maintainer's Safari.  Read `docs/references/local-browser-testing.md` first.
* Build the page of the worktree with `(cd /private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/wt/app && trunk build --release)`, and pass `--dist /private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/wt/app/dist` to `e2e`.  Run one browser GPU run at a time.  `caffeinate -dimsu` runs.

## Plan of work

The simplest experiments first: the sources, then two control kernels, then the variants.  Each step writes its figures to a table in `m3/findings.md` before the next step.  A step whose answer makes a later step pointless records that and skips it.

Not done, and why:

* No WebKit or Chromium build from source: hours of build for each, and the sources and the controls answer the question.
* No Metal frame capture of Safari's GPU process: it needs settings that change the maintainer's Safari.
* No `subgroups` (decision 1).

### Step 1: the sources, at fixed commits

Read these, and record each fact with the file, the commit SHA and the date checked.  Output: `m3/sources.md`, with a table of each access kind and the form that each compiler emits.  The kinds: fixed-size array with a runtime index, constant index, runtime-sized storage array, vector, private array, workgroup array.

* WebKit, `Source/WebGPU/WGSL`: `BoundsCheck.cpp`, and the Metal code generation in `Metal/` (how it emits `__wgslMin`, loops, workgroup zero-init and `arrayLength`).  Also where the WebGPU framework gives options to the Metal compiler (`Source/WebGPU/WebGPU/`).  Examples are `fastMathEnabled`, `mathMode` or `MTLCompileOptions`.
* Dawn and Tint (dawn.googlesource.com): the robustness transform and what it skips, and the MSL writer's loop code (for example a guard against an endless loop).  Also Dawn's Metal compile options (`src/dawn/native/metal/`), and the toggles `disable_robustness` and `dump_shaders` (`src/dawn/native/Toggles.cpp`).
* naga 30.0.1 (`naga/src/back/msl/`): its bounds-check policies, for Step 5.

### Step 2: the scratch worktree and its harness

1. `git worktree add /private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/wt -b scratch/m3-variants 1be7a18`.
2. On that branch only:
   * Each matmul variant of Step 4 is a new `TILES` entry, with its f16 variant, so that the bench times both formats.  `s64-c` needs a source of its own, since a `hand_written` tile always gives `matmul.wgsl`: for example a source field on `Tile`.
   * Each attention variant of Step 6 is a new source timed in `attention_per_chunk`, beside split and fused.
   * Each attention variant and each control kernel goes into `generated_sources`, so that the limit checks see it.  New `TILES` entries are there already.
   * Two control kernels, timed in `?bench` with their own `bench: control ...` lines, in every page of Steps 3 to 6:
     * FMA: generated with no loop.  Each thread reads its inputs from a buffer, runs 8 or more independent chains of straight-line `fma`, and writes one result.  No index access between the read and the write.
     * Copy: each thread copies vec4s from one storage buffer to another, at 64 MB for the memory rate.  Also a near-empty dispatch (4 KiB), timed directly, for the cost of a dispatch.  Each size sends the same number of dispatches for each submit.
   * An environment variable in the scratch `xtask`, for example `E2E_CHROME_ARGS`, adds arguments to Chrome in `e2e bench`.
3. In the worktree, `every_kernel_fits_the_default_limits` (GPU-1) and `sources_are_complete` pass.  The formula and bank-conflict tests (`workgroup_bytes`, `bank_conflicts`) are changed on the scratch branch to accept the variants; that is not a pass of GPU-1.  Each matmul variant passes `Gpu::check_kernels` in each browser, or the bench logs it as skipped and the table says so.  Each attention variant is compared with the CPU (`attention_cpu`, as for the current fused kernel) in each browser before it is timed.  Its workgroup memory is computed.  The current kernel has `buf` 2,112 and `red` 1,024 floats, 12,544 B.
4. Commit on the scratch branch after each step, unsigned commits allowed (it is never pushed or merged).

### Step 3: the controls

One set (Safari, Chrome, Safari, Chrome) of `?bench=v64&bench_rounds=5` with the two controls.  Then one more Chrome page with `--headed`, as Safari runs in a window.  Record, for each browser: the FMA rate (GFLOP/s) and the copy rate (GB/s) at each size.  Also the cost of a dispatch, and `v64`'s rate on each linear shape.

The copy kernel's storage index gets a clamp in both browsers, so the copy shows the memory path, not a code without checks.  Read the controls' browser ratios against `v64`'s browser ratio (about 1.7):

* Controls near 1 (within their spreads): the GPU's state does not differ.  The loss is in the code that each compiler makes for the real kernels.
* Controls near `v64`'s ratio: the GPU's state or the dispatch explains the loss, not the code.  Steps 4 to 6 still run, but each browser ratio is read against the controls' ratio.
* Between the two: both have a part.  Give the part of `v64`'s ratio that the controls explain, and read the later steps against the rest.
* A headed Chrome page that differs from the headless pages by more than the spread: record it, and use headed Chrome for the later steps.

### Step 4: matmul variants

One set of `?bench=i64,s64,v64,<variants>&bench_rounds=5`.  The variants, each generated like `v64`:

* `v64-u`: `v64` with the inner `kv` loop unrolled.  Each read is then `as_[ab + c]`, with `ab = ty * SV` the same in every K step and `c` a constant.  The same number of clamps, with a loop-invariant runtime part.
* `v64-clamp2`: `v64` with one more clamp on each workgroup read, `min(index, max(p.k, SA_LEN - 1u))` (`SB_LEN` for `bs_`).  It gives the same index for any `k`, and no compiler can remove it, since `p.k` comes from the uniform buffer.  This measures what a clamp costs in each browser, Safari included, which Step 5 cannot do.
* `v64-bar`: `v64` with one more pair of `workgroupBarrier()` for each K tile.  The same memory and layout, so this measures the cost of a barrier.
* `v128`: 128 x 128, 8 x 8 outputs for each thread, BK 8, vec4 reads, 16 x 16 threads.  With a pad of one vec4, it has (128 + 128) x (8 + 4) x 4 B = 12 KiB.  M4's start point.  Twice the multiply-adds for each workgroup read of `v64`.
* `v128-u`: `v128` with the form of `v64-u`.  Only if `v64-u` shows an effect.
* `s64-c`: `s64` with its sums in the same private array, but every index a constant (loops over the array unrolled).  This removes the clamp and the loop index together, so it cannot tell the two apart.  It shows if the array itself is slow, and Step 5's toggles tell the clamp's part.

Record each variant's ratio to its reference in each browser, and each browser ratio.  The references: `s64-c` to `s64`, `v128-u` to `v128`, each other variant to `v64`.  The shapes: `freq qkv` and `time qkv`, with f32 and f16 weights.  Also record each kernel's f16 / f32 ratio, and compare it between the browsers.  f16 also halves B's reads.  So only the difference between the browsers points to `unpack2x16float`.

### Step 5: the clamp's cost, alone

Tests that change only what a compiler adds, the cheapest first:

1. Chrome: pages of `?bench=v64,v64-u,s64,s64-c&bench_rounds=5`, taking turns twice (default, robustness off, zero-init off, default, robustness off, zero-init off): the default; `--enable-dawn-features=disable_robustness`; `--enable-dawn-features=disable_workgroup_init`.  Check each toggle's name in Dawn's `Toggles.cpp` first (Step 1).  If `dump_shaders` gives Tint's MSL, keep `v64`'s MSL in `m3/`.  Dawn can write it to its device log, not to the page's console that `e2e` reads.  Record where it went, or that it did not come.
2. Native, only if 1 does not settle the cost of the checks.  A scratch test in `roformer-gpu/tests/speed.rs` times `v64`, `v64-u`, `s64` and `s64-c` on 5456 x 1536 x 256 through wgpu on Metal.  It has its own pipeline code, since `Gpu` makes its modules itself.  Four forms, built in one process and timed in turns:
   * `ShaderRuntimeChecks::checked()`;
   * the same with `bounds_checks = false`;
   * the same with `force_loop_bounding = false`;
   * `checked()` with `PipelineCompilationOptions::zero_initialize_workgroup_memory = false`.

   Not `unchecked()`, which also turns off the other checks.

Record each ratio to the default.  Workgroup zero-init, if neither 1 nor 2 tests it: rule it in or out by arithmetic.  `v64` zeroes 10 KiB once for each workgroup, and stages 16 K tiles of 10 KiB for K = 256.

### Step 6: attention variants

At most two fused attention variants, from what Steps 3 to 5 found, each within the default limits (Step 2.3) and checked against the CPU.  For example: the index form that won in Step 4, applied to `buf` and `red`; or more outputs for each thread.  One set of `?bench=v64&bench_rounds=5` (the bench always times split and fused attention), with the variants timed beside them.  Record the time for each chunk on the time axis, and the ratio to fused, in each browser.  This informs M5; M5 designs the flash-style kernel.

### Step 7: the result

1. `m3/findings.md`: each table, each log's path, and the cause or the best-supported explanation.  An explanation names the steps whose figures support it, and each candidate that the figures rule out.  A candidate that no step tested stays "not tested".
2. The design that M4 and M5 start from: tile shape, outputs for each thread, the memory layout and index form.  Also the number of barriers, and the attention structure.  Each choice names its figure.  The parent's M4 gives 8 KiB for the 128 x 128 tile, without a pad; with `v64`'s pad of one vec4 it is 12 KiB.  The diff of item 3 corrects it, or says why the pad goes.
3. `m3/parent-m3.diff` for the orchestrator, `git apply --check` passing on HEAD.  It holds a Surprises entry with the summary table, and Decision log lines for the cause and the design.  It changes the M4 and M5 sections where the design changes their start point.
4. If the cause is in WebKit: an entry in `docs/references/browser-quirks.md` in the form of the other entries.  It gives what was seen, where and when, the code, and the rule.  Also `m3/webkit-bug.md`, a bug text for the maintainer to file.  The text gives a small WGSL case, the figures in both browsers, and the WebKit source lines with the commit.  Review round 1 put it in the repo as `docs/references/webkit-bug-workgroup-zero-init.md`.
5. `git -C /private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/wt format-patch 1be7a18..scratch/m3-variants -o /private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/variants/`.  M4 and M5 can start from it.  Then remove the worktree and the scratch branch.
6. If a variant or the bench harness is worth keeping on the branch, do not commit it: say so in the report, with the reason.  The team leader decides, and then this plan gets a code step with tests that fail first, and review rounds over the code.

### Step 8: the checks before the review round

* `STEMYLA_PERF_WEIGHTS=<weights> CHROMEDRIVER=<driver> cargo run -p xtask -- check --tier push` passes, with both perf gates run (the changed paths from `origin/main` include `roformer-gpu/`).  Progress gives both gates' verdicts.  A skip line is not a pass (VERIFY-1).
* `cargo build -p xtask` passes.
* `git diff --stat 7ec4d06..HEAD` shows no path outside `docs/`, `.github/`, `xtask/src/e2e/save.rs` (the CI commit) and the paths that the orchestrator applies.  The review fixes of the CI commit add `xtask/src/web_test.rs`, `xtask/src/main.rs` and `xtask/tests/e2e_cli.rs`.

## Validation and acceptance

* This plan's Surprises hold a table of each step's figures, in each browser, each with its log in `m3/runs/`.  `m3/findings.md` and the logs stayed in the session scratchpad.  The browser ratio of `v64` on `freq qkv` with f16 weights is measured again in Step 4 and compared with the parent's ratio, 1.70.
* The cause, or the best-supported explanation, names its evidence: the source lines (file, commit, date) and the figures.  It says how much of `v64`'s browser ratio it explains.
* Each candidate of "The hypothesis, and the other candidates" has a verdict, with the reason.  The verdict is ruled in, ruled out, "ruled by source" (fast math, for example) or "not tested".  "Ruled in" and "ruled out" follow the rule for an effect (Terms).
* The design for M4 and M5 is in the parent plan's Decision log (through the orchestrator), each choice with its figure.
* A WebKit cause has its `browser-quirks.md` entry and its bug text, `docs/references/webkit-bug-workgroup-zero-init.md`.
* The branch gets no code from M3, unless Step 7.6 adds a code step.  Step 8's checks pass.
* The review rounds of the CI commit (`/review-rounds 7ec4d06`) end by the stop rule of decision 0024.

## Idempotence and recovery

Each experiment writes only to the scratch worktree and `m3/`.  A session that stops reads Progress and this plan's Surprises, and goes on at the next open step.  The full tables of `m3/findings.md` were in the session scratchpad, and are not kept.  A page that a locked screen or a Safari timeout stops runs again.  A set that a browser update splits runs again in full, and Surprises records both versions.  `git worktree remove` and `git branch -D scratch/m3-variants` undo Step 2.

## Progress

* [x] Plan review round 1 (2026-11-13): 9 should-fix and 4 nits, each applied.
* [x] Plan review round 2 (2026-11-13): 5 should-fix and 3 nits, each applied.
* [x] Step 1 (2026-11-13): the sources.  `m3/sources.md` gives each fact with its file, lines and commit.  The table is in Surprises.
* [x] Step 2 (2026-11-13): the scratch worktree and its harness, on `scratch/m3-variants` (`9a30c16`, `4cd30a2`, `047fbb8`).  The six matmul variants (with f16) and `attention-u` pass `every_kernel_fits_the_default_limits`, `sources_are_complete` and `workgroup_memory_within_defaults`.  `STEMYLA_REQUIRE=gpu,f16 cargo test --release --test kernels` passes natively (19 tests, `check_kernels_accepts_all` included).  `Gpu::check_kernels` accepted each variant in Safari 27.2 and Chrome 157 (`runs/smoke-*/bench.log`).  `attention-u` matches `attention_cpu` natively (relative error 6.4e-8).
* [x] Step 3 (2026-11-13): the controls.  The controls are near 1 and `v64` is at 1.44 to 2.37, so the loss is in the code.  The loss falls as K grows: a fixed cost for each workgroup.  The table is in Surprises.
* [x] Step 4 (2026-11-13): matmul variants, and `v64-g4` after them (Step 4b).  No variant is faster than `v64` in either browser.  The `wg10k` control, with 10 KiB of workgroup memory and almost no work, takes 23 times as long in Safari for the whole dispatch.  The table is in Surprises.
* [x] Step 5 (2026-11-13): the clamp's cost, alone.  Chrome's toggles, and Chrome's MSL for `v64` timed through Metal directly with WebKit's forms put in one at a time.  WebKit's zero-init with its clamps explains Safari's fixed cost for each workgroup; the thread maximum shows no effect.  The table is in Surprises.
* [x] Step 6 (2026-11-13): attention variants.  `attention-u` is 3.6 times slower than fused in Safari and 1.33 times in Chrome.  Only one variant ran.  The table is in Surprises.
* [x] Step 7 (2026-11-13): the result.  `m3/findings.md` held each table, each log, the explanation, each candidate and the design.  It was in the session scratchpad and is not kept.  This plan's Surprises hold the figures.  `git apply --check` of `m3/parent-m3.diff` passes on `7f5dba6`, and `docs-check` passes with it applied.  Also the `browser-quirks.md` entry (`7f5dba6`), `m3/webkit-bug.md`, and `m3/variants/` (5 patches from `scratch/m3-variants`).  The worktree and the scratch branch are removed.  No variant is kept as code.
* [x] Step 8 (2026-11-13): the checks before the review round.  `check --tier push` passed with both perf gates run: `check: 15 passed in 445.6 s`.  Chrome took 4.60 s (bound 5.17 s), and Safari 7.10 s (bound 8.14 s).
* [x] Review round 1 (2026-11-13, `/review-rounds 7ec4d06`, on the CI commit 1be7a18 and the M3 docs): 8 should-fix and 8 nits, all fixed in `d616783`.  `web-test` exits with 75 only after the runner's hang, and CI runs a step again only on 75.  The CI job-limit check reads the retry steps.  The WebKit bug text moved to `references/webkit-bug-workgroup-zero-init.md`.
* [x] Review round 2 (2026-11-13): 6 should-fix and 7 nits, all fixed in `f4c8a4f`.  The harness prints a failed test as "... FAIL", so the hang check now finds it.  TD-99 records a stem-worker test that failed once on CI, with no cause found (the orchestrator's decision).
* [x] Review round 3 (2026-11-13): 3 should-fix and 8 nits, all fixed in `3454cb3`.  The end of `web-test`'s run is a pure function with a test.
* [x] The maintainer's requests before round 4 (2026-11-13): each browser `e2e` scenario has a time limit (`ec3932f`).  The review agents' guard refuses a wide search, and the skills ask for absolute paths and a check that no agent still runs (`3898433`).
* [x] Review round 4 (2026-11-13, late rule): 20 should-fix fixed in `45e8577`, 4 pre-existing mutants sent to TD-39, and 10 nits reported, not fixed.  A fake WebDriver drives a real `Page` in the tests.
* [x] Review round 5 (2026-11-13): 4 should-fix fixed in `4d54c81`, and 8 nits reported, not fixed.
* [x] Review round 6 (2026-11-13): 2 should-fix fixed in `a076656`; 1 should-fix and 1 nit reported, not fixed.
* [x] Review round 7 (2026-11-13): no fix.  1 pre-existing should-fix (TD-34 lists it) and 3 nits reported, not fixed.  It is the last round by decision 0024.  The log is `docs/reviews/gpu-speed-2026-11-12-m3.md`.

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

  So the clamp has the same form in both browsers.  Three differences remain.  Chrome removes the clamps that it can prove.  Safari sets workgroup memory to zero with one thread, byte by byte.  Safari gives Metal no maximum number of threads.

* 2026-11-13, Step 3: the controls, from one set of `?bench=v64&bench_rounds=5` (`m3/runs/s3-{safari,chrome}-{1,2}.log`, headed Chrome `m3/runs/s3h-chrome-1.log`).  Figures are medians of 5 rounds, the spread in brackets; the ratio is Safari's time over Chrome's, for set 1 and set 2.  The full table was in `m3/findings.md`, in the session scratchpad, and is not kept.

  | Figure | Safari 1 | Chrome 1 | Safari 2 | Chrome 2 | Ratio |
  | - | - | - | - | - | - |
  | FMA control, GFLOP/s (start) | 5,976 (5.1 %) | 6,450 (0.9 %) | 6,086 (4.1 %) | 6,445 (0.9 %) | 1.079, 1.059 |
  | Copy 64 MiB, GB/s (start) | 172.5 (1.3 %) | 179.9 (1.3 %) | 173.2 (1.3 %) | 178.9 (0.6 %) | 1.043, 1.033 |
  | Dispatch (copy 4 KiB), us (start) | 10.6 (9.5 %) | 5.0 (3.7 %) | 10.8 (6.6 %) | 5.0 (3.5 %) | 2.120, 2.160 |
  | `v64` `freq qkv` f16 (K 256), GFLOP/s | 1,218 (0.7 %) | 2,144 (0.4 %) | 1,218 (0.4 %) | 2,140 (1.0 %) | 1.761, 1.757 |
  | `v64` `time qkv` f16 (K 256), GFLOP/s | 1,165 (0.9 %) | 2,021 (0.4 %) | 1,159 (0.9 %) | 2,014 (1.3 %) | 1.735, 1.737 |
  | `v64` `freq ff down` f16 (K 1,024), GFLOP/s | 1,488 (0.9 %) | 2,153 (0.5 %) | 1,488 (1.1 %) | 2,160 (1.1 %) | 1.447, 1.452 |
  | `v64` `time scores` (K 64), GFLOP/s | 606 (0.9 %) | 1,437 (0.7 %) | 606 (0.9 %) | 1,429 (0.9 %) | 2.374, 2.360 |
  | Time-axis attention, fused, s per chunk | 3.53 (4.4 %) | 2.31 (0.6 %) | 3.52 (3.1 %) | 2.30 (1.5 %) | 1.528, 1.530 |

  The FMA and copy ratios are below 4 % plus the larger spread, so no effect is seen.  The GPU's state does not explain the loss.  A dispatch costs 6 us more in Safari, about 0.2 % of a `v64` dispatch on `freq qkv`.  Headed Chrome is within the spread of headless Chrome (FMA 6,475, `freq qkv` f16 2,153), so the later steps keep headless Chrome.  The f16 / f32 ratio of `v64` is 0.98 to 1.01 in both browsers, so `unpack2x16float` shows no effect.  The ratio of `freq qkv` f16, 1.76, is near the parent's 1.70.

  The ratio falls as K grows.  A least-squares fit of the time for each workgroup used 19 cores, and the shapes with 300 workgroups or more.  It gives Safari 8.5 us fixed plus 24.5 us for each 256 of K.  It gives Chrome 0.7 us plus 18.2 us.  So Safari has a fixed cost of about 7.8 us for each workgroup, and an inner loop 1.35 times slower.  At K = 256 the fixed cost is 55 % of the gap.

* 2026-11-13, Step 4: one set of `?bench=i64,s64,v64,v64-u,v64-clamp2,v64-bar,v128,v128-u,s64-c&bench_rounds=5` on `freq qkv`, `time qkv` and `freq ff down` (`m3/runs/s4-{safari,chrome}-{1,2}.log`), then one set of `?bench=v64,v64-g4` (`m3/runs/s4b-*.log`).  Ratio: the variant's time over its reference's time in the same page, the range over both pages, shapes and formats.  A star: an effect in each page of that browser.  The full table was in `m3/findings.md`, in the session scratchpad, and is not kept.

  | Kernel (reference) | Ratio in Safari | Ratio in Chrome | Browser ratio, `freq qkv` f16 |
  | - | - | - | - |
  | `v64` | | | 1.767, 1.768 |
  | `v64-u` (`v64`) | 1.49 to 1.73 * | 1.67 to 1.93 * | 1.413, 1.405 |
  | `v64-clamp2` (`v64`) | 1.034 to 1.055 * | 1.039 to 1.056 * | 1.740, 1.749 |
  | `v64-bar` (`v64`) | 1.003 to 1.015 | 0.997 to 1.006 | 1.777, 1.782 |
  | `v128` (`v64`) | 1.30 to 1.59 * | 1.71 to 1.86 * | 1.290, 1.283 |
  | `v128-u` (`v128`) | 1.50 to 1.58 * | 1.90 to 2.08 * | 1.030, 1.034 |
  | `s64-c` (`s64`) | 0.115 to 0.152 * | 0.996 to 1.009 | 1.491, 1.481 |
  | `i64` (`v64`) | 1.25 to 1.58 * | 1.44 to 1.50 * | 1.498, 1.494 |
  | `v64-g4` (`v64`), K 256 | 1.014 to 1.033 | 1.196 to 1.225 * | 1.496, 1.504 |
  | `v64-g4` (`v64`), K 1,024 | 1.339 to 1.354 * | 1.254 to 1.266 * | |

  | Control (start of the page) | Safari 1, 2 | Chrome 1, 2 | Ratio |
  | - | - | - | - |
  | `fma`, GFLOP/s | 6,108, 6,131 | 6,432, 6,450 | 1.053, 1.052 |
  | `fma64`, GFLOP/s | 5,807, 5,828 | 6,099, 6,104 | 1.050, 1.047 |
  | `wg10k`, us for 2,064 workgroups | 408.0, 410.2 | 17.9, 17.8 | 22.8, 23.0 |
  | `wg0`, us for 2,064 workgroups | 11.0, 11.0 | 6.3, 6.3 | 1.75, 1.75 |

  The reading.  In the `wg10k` control, the zero-init of 10 KiB costs Safari 0.19 us of GPU time for each workgroup.  That is 0.39 ms of the 1.53 ms that Safari loses on each `v64` dispatch of `freq qkv` (about 26 %).  Step 5.2 measured more in a real kernel: 0.56 ms for the zero-init alone, and 0.96 ms with the clamps.  One clamp on each workgroup read costs about 5 % in both browsers.  `v64` has it only in Safari, since Chrome's range analysis removes it (`m3/v64-chrome.msl`).  Barriers, `unpack2x16float` (f16 / f32 0.96 to 1.06 in both) and 64 live values (`fma64`) show no effect.  The unrolled form and the 8 x 8 tile are slower in both browsers.  `s64`'s loss in Safari is its loop-indexed private array: `s64-c` with constant indices runs at `i64`'s rate.  `v64-g4` lowers the browser ratio to 1.50 at K 256, but costs Chrome 20 %.  So it is not a design for M4.  In Chrome, each `v64-g4` ratio is over 1.19, and each spread is 4.6 % or less, so the row has a star.  In Safari, `v64-g4` is not faster than `v64`.  Its K 256 row has no star: in Safari 1, `freq qkv` f32 gives 1.014, inside its spread of 2.8 %.  At K 1,024 (`freq ff down`, `m3/runs/s4b-*.log`), `v64-g4` is slower in both browsers: 1.34 to 1.35 times `v64` in Safari, 1.25 to 1.27 in Chrome.  So its own cost is not the same in both browsers there.  In Safari 2, one round of `time qkv` was slow for every kernel (spreads to 126 %); the medians of 5 do not change.  Each Step 4 page logged "the bench finishes: FAIL".  `bench::summary` panicked after the timings, on the shorter list of shapes.  The scratch branch fixed it in `cc6cf54`.

* 2026-11-13, Step 5: the cost of what each compiler adds.  The full tables were in `m3/findings.md`, in the session scratchpad, and are not kept.

  5.1, Chrome's toggles, pages in turns twice (`m3/runs/s5{def,rob,wgi,rng}-chrome-{1,2}.log`).  The figure is the time ratio to the default page, divided by each page's FMA control.  It is for `v64`, `s64` and `s64-c` on `freq qkv`, `time qkv` and `freq ff down`:

  | Toggle | `v64` | `s64`, `s64-c` |
  | - | - | - |
  | `disable_robustness` | 0.95 to 0.99 | 0.94 to 0.99 |
  | `disable_workgroup_init` | 0.99 to 1.00 | 0.99 to 1.005 |
  | range analysis off (`--disable-dawn-features=enable_integer_range_analysis_in_robustness`) | 1.01 to 1.03 | 1.09 to 1.14 * |

  All clamps together (`disable_robustness`) cost Chrome 1 % to 6 % (no effect by the rule).  Chrome's zero-init of 10 KiB costs 9 us for 2,064 workgroups (`wg10k` 17.8 us, 8.7 us with it off).  With each clamp kept, as in Safari, `s64` is 11 % slower in Chrome, not 10 times.  So the clamp on a private array does not by itself put the array in memory.

  5.2, Metal directly.  Chrome's MSL for `v64` (`m3/v64-chrome.msl`) comes from `dump_shaders`.  Chrome writes it to its log file with `--enable-logging --log-file=<file>`.  It was compiled and timed through Metal (`m3/native/bench.swift`; logs `m3/runs/s5native-{1,2}.log`).  WebKit's forms were put in one at a time.  They are no thread maximum, WebKit's zero-init (thread 0, byte by byte), and a clamp on each workgroup read.  The time ratio to Chrome's MSL, M 5,456, medians of 7, spreads 0.1 % to 0.9 %:

  | Form | N 1,536, K 256 | N 256, K 1,024 | N 1,536, K 1,024 |
  | - | - | - | - |
  | Chrome's MSL | 1.948 ms | 1.282 ms | 7.294 ms |
  | no thread maximum | 1.000 | 1.001 | 1.000 |
  | WebKit's zero-init | 1.290 | 1.048 | 1.040 |
  | clamps | 1.020 | 1.018 | 1.019 |
  | zero-init and clamps | 1.491 | 1.133 | 1.131 |
  | all three | 1.491 | 1.134 | 1.135 |

  The thread maximum shows no effect.  WebKit's zero-init with the clamps adds 0.96 ms for 2,064 workgroups at each K.  That is a fixed cost for each workgroup, near Step 3's fit for Safari (0.85 ms).  At K 256 this is 0.49 of the 0.76 by which Safari's ratio is over 1 (64 %).  At K 1,024 (`freq ff down`) it is 0.133 of 0.447 (30 %).  The 64 % is the estimate of Chrome's MSL timed with WebKit's zero-init and clamps.  The other estimates of the fixed cost's part at K 256 are lower:
  * the `wg10k` control: about 26 % (Step 4);
  * `v64-g4` against `v64`: about 34 % (Step 4b).  The estimate uses K 256 only.  It assumes that `v64-g4`'s own extra cost is the same in both browsers.  At K 1,024 it is not (Step 4).  Safari's time with `v64-g4` at K 256 did not go down (1.014 to 1.033 times `v64`).  The browser ratio fell because Chrome got about 1.20 times slower;
  * Step 3's fit: 55 %.

  The rest is a K loop 1.35 times slower in Safari, which none of these forms gives.  Natively each form takes 5.35 ms for each 768 of K.

* 2026-11-13, Step 6: one set of `?bench=v64&bench_rounds=5` with `attention-u` (`m3/runs/s6-{safari,chrome}-{1,2}.log`).  It passed the check against `attention_cpu` in each page (relative error 6.3e-8).  Time-axis attention, seconds for each chunk, medians of 5:

  | Kernel | Safari 1, 2 | Chrome 1, 2 | Ratio to fused, Safari | Ratio to fused, Chrome | Browser ratio |
  | - | - | - | - | - | - |
  | split | 3.522, 3.534 | 1.852, 1.855 | 1.014, 1.018 | 0.808, 0.812 | 1.902, 1.905 |
  | fused | 3.472, 3.472 | 2.291, 2.284 | 1 | 1 | 1.516, 1.520 |
  | `attention-u` | 12.524, 12.415 | 3.024, 3.042 | 3.607, 3.576 | 1.320, 1.332 | 4.141, 4.081 |

  The unrolled form is slower in both browsers, as in Step 4.  The fixed cost of Step 5 is about 2 % of a fused dispatch in Safari (576 workgroups, each over 1,151 keys).  So fused attention's loss is the slower inner loop.

* 2026-11-13, Step 7: the best-supported explanation (Step 7 of `m3/findings.md`, which was in the session scratchpad and is not kept).  Safari's loss on `v64` has two parts.  First, a fixed cost for each workgroup.  WebKit sets workgroup memory to zero with thread 0, byte by byte.  With WebKit's clamps, this makes Chrome's MSL 1.49 times slower at K 256 (Step 5.2).  By that emulation, it is 64 % of `v64`'s loss at K 256 and 30 % at K 1,024.  The other estimates at K 256 are about 26 % (`wg10k`), about 34 % (`v64-g4`, from K 256 only) and 55 % (Step 3's fit).  In fused attention, it is about 2 % of a fused dispatch in Safari, and about 5 % of its loss.  The `v64-g4` estimate assumes that `v64-g4`'s own extra cost is the same in both browsers.  At K 256, Safari's time did not go down with `v64-g4`, and Chrome's became about 1.20 times longer.  Second, a K loop about 1.35 times slower, whose cause is not found.  It is the rest, and most of fused attention's loss of 1.52.  A dump of Safari's own MSL would settle the second part.  WebKit's toggle for it is `com.apple.WebKit.WebGPU.TogglePrintMetalCode`.  Where the GPU process writes it is not confirmed, so M3 did not try it.

* 2026-11-13, review round 2: a reviewer's `git worktree add` failed on a name in use, and a pipe hid the failure.  So a `sed` changed one line in another reviewer's worktree.  The reviewer reverted it, and `review check` printed "no change" (TD-34; as in M1 and M2).
* 2026-11-13, review round 3: the supply-chain reviewer still ran after its report.  It ran `find /` from the plan's scratchpad placeholder for 3.6 hours, and raised a macOS privacy prompt.
* 2026-11-13: the Safari perf gate at `eb81ed0` hung for 3 hours after "click Separate 6 stems".  A rerun passed (7.00 s, bound 8.14 s).

## Decision log

* 2026-11-13, agent (team leader): the variants live on a local scratch branch in a scratch worktree, and a patch of them goes to `m3/variants/`.  Why: M3 only measures, and M4 and M5 add their kernels with tests.
* 2026-11-13, agent (team leader): a ratio inside one page is the main figure.  Each experiment is one set of pages in the order Safari, Chrome, Safari, Chrome.  Why: the bench already takes turns inside each round, and the 4 % drift between sessions moves every kernel of a page alike (`plans/completed/gpu-speed-m2.md`, Surprises).
* 2026-11-13, agent (team leader): two control kernels (FMA and copy) run first.  Why: M2's "other" group is also 1.5 times slower in Safari.  If a kernel with no index access in its loop is slower too, the cause is not the bounds checks.

* 2026-11-13, agent (team leader), plan review round 1: `v64-2d` and `v64-bk32` are replaced by `v64-u`, `v64-clamp2` and `v64-bar`.  Why: the 2D form has the same number of clamps as an unrolled 1D form.  BK 32 without a pad adds 4-way bank conflicts (`bank_conflicts` in `lib.rs`).  Each new variant changes one thing.
* 2026-11-13, agent (team leader), plan review round 1: Chrome's toggles run before the native test, which runs only if they do not settle the cost.  Why: the toggles need no new pipeline code.

* 2026-11-13, agent (implementer), Step 2: three more controls run in every page beside FMA and copy.  `fma64` is the FMA control with 64 chains of 32, the same FMAs with 64 live values.  `wg10k` declares v64's 10 KiB of workgroup memory and does almost no work, at v64's grid on `freq qkv` (2,064 workgroups).  `wg0` is the same with no workgroup memory.  Why: Step 1 found two differences that FMA and copy do not test.  Safari sets workgroup memory to zero with one thread, byte by byte.  Safari gives Metal no thread maximum, which can limit the registers of each thread.
* 2026-11-13, agent (implementer), Step 2: `v128-u` is timed in the set of Step 4, and its figure is read only if `v64-u` shows an effect.  Why: a second set costs more than one more kernel in the first set.
* 2026-11-13, agent (implementer), Step 2: on the scratch branch, calibration chooses only among the four kernels of `main`.  So split attention uses the same matmul as on `main`.  A build with `M3_SHAPES` set times only the shapes that it names.
* 2026-11-13, agent (implementer), Step 2: `attention-u` is built now.  It is fused attention with each loop over `kk` and `k` unrolled.  So each read of `buf` and `red` there is a loop-invariant base plus a constant.  Step 6 times it only if Step 4 supports the unrolled form.
* 2026-11-13, agent (implementer), Step 4: `v64-g4` is added after the set of Step 4, in a set of its own.  Why: Step 3's fit and the `wg10k` control show a fixed cost for each workgroup in Safari.  A kernel that sets its memory to zero once for 4 tiles tests that cost in a real kernel.
* 2026-11-13, agent (implementer), Step 4: the pages built with `M3_SHAPES` time `freq ff down` (K 1,024) beside `freq qkv` and `time qkv`.  Why: a second K separates a fixed cost for each workgroup from a cost for each K step.
* 2026-11-13, agent (implementer), Step 5: 5.1 has a third Chrome form, range analysis off, and 5.2 times Chrome's MSL through Metal directly, not wgpu.  Why: range analysis off makes Chrome keep each clamp as Safari does.  Chrome's MSL lets one WebKit form go in at a time, and no wgpu option gives WebKit's zero-init or removes the thread maximum.
* 2026-11-13, agent (implementer), Step 6: only `attention-u` ran, not a second variant.  Why: no finding of Steps 3 to 5 gives an attention form that removes Safari's loss.  The fixed cost is about 2 % of a fused dispatch.
* 2026-11-13, agent (implementer), Step 7: the cause in WebKit is the zero-init of workgroup memory, so it gets a `browser-quirks.md` entry and `m3/webkit-bug.md`.  The design for M4: `v64`'s tile with fewer bytes of workgroup memory for each unit of work, no 8 x 8 tile, no unrolled loop.  The candidates are `v64` with BK 8, and a kernel with no workgroup memory.  For M5: the fused structure, each query block loaded once, no unrolled loop.  Each choice and its figure was in `m3/findings.md` and `m3/parent-m3.diff`.  `m3/findings.md` was in the session scratchpad and is not kept.  This plan's Surprises hold the figures.
* 2026-11-13, agent (implementer), review rounds 1 and 2: the review of the CI commit added code to `xtask`.  Round 1 changed `xtask/src/web_test.rs` and `xtask/src/main.rs`, and round 2 also `xtask/tests/e2e_cli.rs`.  Why: CI must run a step again only after the runner's hang, so `web-test` gives the hang its own exit code.  Step 8's path list names these paths.
* 2026-11-13, agent (orchestrator): the stem-worker failure of run 43152005940 is a pre-existing finding; with no cause found in one hour, it gets TD-99.
* 2026-11-13, maintainer: three changes before review round 4: the Bash guard refuses a search of the whole disk or home folder; each browser `e2e` scenario gets a time limit; the skills give absolute paths and check that no subagent still runs.  Why: a review agent ran `find /` for 3.6 hours and walked the Photos library, and a Safari `e2e perf` run hung for 3 hours.

* 2026-11-13, agent (team leader), close-out: the second mark in `load_and_separate` (`xtask/src/e2e.rs`) is named `separated_from`.  Why: a `KNOWN_MISSED` entry needs its code line once in its function, and `let from = p.mark();` was there twice.  The behaviour does not change.

## Outcomes and retrospective

Done on 2026-11-13.

What M3 found:

* Safari's loss on the same WGSL is not the GPU's state: the FMA and copy controls are near 1 in both browsers.
* WebKit sets each workgroup's memory to zero with one thread, byte by byte.  That is a fixed cost for each workgroup.  It explains 26 % to 64 % of `v64`'s loss at K 256, by the estimate.
* The rest is a K loop about 1.35 times slower in Safari.  Its cause was not found.  A dump of Safari's own MSL could settle it.
* Barriers, `unpack2x16float`, fast math, loop guards and the thread maximum are ruled out.  Clamps cost 2 % to 5 %.
* M4 and M5 start from these results.  No variant is kept as code.

What M3 did not do:

* It did not find the cause of the slower K loop.
* It did not land "docs only".  The review of the CI commit 1be7a18 found that its retry hid each failure, and its fixes added code to `xtask`.  The maintainer's requests before round 4 added the `e2e` time limits and the guard.

What to do differently:

* Never put a placeholder such as `<scratchpad>` in a plan or a brief.  A reviewer ran `find /` from one for 3.6 hours.
* Before a report, check each subagent's background work, not only its report.
* Build in a worktree with its own `--target-dir`.  A shared target folder left test binaries that pointed at a removed worktree.
* Give each review agent a worktree name of its own.  Rounds 2, 6 and 7 each had two reviewers in one worktree (TD-34).
* Review a CI commit with weak tests early, in its own range.  Most of the 7 rounds went to the CI commit and its fixes, not to M3.
