# GPU speed, M4: a matmul with less workgroup memory for each unit of work

## Purpose

This is the milestone plan for M4 of `plans/active/gpu-speed.md`.  The parent's M4 section and its "Validation and acceptance" stay the criteria.

Safari sets workgroup memory to zero with one thread, byte by byte (`references/browser-quirks.md`).  So in Safari, a kernel with fewer bytes of workgroup memory for each workgroup can be faster on the same grid.  M4 adds such matmul kernels as calibration candidates.  Each GPU keeps a candidate only if it measures it faster, so the Radeon and the iPhone keep their kernels.

M4 also lands three tools that M5 and M6 use:

* the rule "A change lands only with a shown gain" (Step 7), with the in-page `?bench` ratio for a kernel;
* the same-session comparison of a base page and a head page (`e2e perf --base-dist`, Step 5), with its noise measured;
* `--raise` for every raise of a baseline's bound, a new browser version included (Step 2).

Readers: the implementer, the team leader, the orchestrator, and the team leaders of M5 and M6.

## Context and orientation

The state at the start, on 2026-11-13, at commit `9769916` on branch `gpu-speed/2026-11-12`.  The review base is `9769916`.  The machine is the maintainer's Apple M2 Pro (19-core GPU), with Safari 27.2 and Chrome 157.

### The decisions that M4 follows

* The maintainer, 2026-11-12: kernels stay within WebGPU's default limits (GPU-1: 16 KiB of workgroup memory, 256 invocations).  No `subgroups`.  The perf gate runs on this M2 Pro only, with tolerances that are not too tight.  Never wait for a run on the Radeon or the iPhone.  Decision 0031 is accepted.
* The maintainer, 2026-11-13: never run a search of the whole disk or of the home folder.  Give every agent absolute paths.  Before a report, check that no agent or command still runs.
* The parent plan: before M4 changes a baseline, every raise of a bound needs `--raise`, a new browser version included.  A test of it fails first.  No baseline changes before that change lands.
* The parent plan: the WGSL of each current kernel stays the same byte for byte, checked by a test that fails first on a change.
* The parent plan: M4's first commit fixes the entry of `references/browser-quirks.md` that says that more work for each byte loses less.
* The parent plan: not a 128 x 128 tile with 8 x 8 outputs, not an unrolled inner loop, and no loop-indexed private array.  Each output keeps its sum order, so the f16 variant gives the f32 kernel's output bit for bit (decision 0022).
* The parent plan: the drift between sessions is up to about 5.7 %.
* Lessons of M3: a build in a worktree uses its own `--target-dir` (or `CARGO_TARGET_DIR`) inside that worktree.  One browser run or perf gate at a time on this Mac.  Late review rounds fix only blockers, significant should-fix and confirmed findings on a strict rule.

### What is known (parent plan, M3)

* Safari's loss on `v64` at K 256 has two parts.  A fixed cost for each workgroup, from the zero-init with its clamps: 26 % to 64 % of the loss, by the estimate.  And a K loop about 1.35 times slower, with no cause found.
* The `wg10k` control (10 KiB of workgroup memory, almost no work, 2,064 workgroups): 408 us in Safari, 17.9 us in Chrome.  `wg0` (no workgroup memory): 11.0 us and 6.3 us.  M3 measured the zero-init only at 10 KiB and at 0 KiB.  That its cost grows with the bytes comes from WebKit's source.
* M3 Step 5.2: WebKit's zero-init with its clamps adds 0.96 ms to `v64` on `freq qkv` (2,064 workgroups), of 3.52 ms in Safari.  BK 8 (6 KiB) can save about 40 % of the zero-init's part: 6 % to 11 % of a K 256 product, and 1 % to 5 % of a separation.  A kernel with no workgroup memory can save up to 27 % of a K 256 product.
* `v64-g4` and `v128` did more work for each byte of workgroup memory, and neither was faster in Safari.  So the rule is fewer bytes for each workgroup on the same grid, not more work for each byte.
* The baselines (`xtask/perf-baselines.json`): Chrome 157 4.70 s, Safari 27.2 7.40 s, each with a tolerance of 10 %.  Both chose `v64` for the large and the small products.  A Safari run of M2 chose `i64` for the small products once.
* The cost of a forced flip (M2 Step 5.3): Chrome `attn=fused` 2.1 %, `mm=v64,i64` 0 %; Safari `attn=freq` 9.5 %, `mm=v64,i64` 0 %.  Safari's calibration gave fused / split 0.975 to 0.992 in 26 calibrations.

### The code today

* `roformer-gpu/src/lib.rs`: `Tile` and `TILES` (`i64`, `i128x64`, `s64`, `v64`).  `Tile::vec4_source` fills `shaders/matmul_vec4.wgsl`.  It asserts that each tile load fills the 256 threads: `(bm * bk / 4) % 256 == 0`.  So BK 8 on a 64 x 64 tile (128 vec4s for each tile) needs a change there.  `Tile::f16_source` makes the f16 variant with `replace_once` on the B declaration and on the one B read line of `b_get`.  `kernel_sources` and `kernels_id` (FNV-1a) cover each matmul kernel, its f16 variant and fused attention.
* The tests of the tiles in `lib.rs`: `bank_conflicts` wants `[2, 2, 1, 1]` for each vec4 tile; `workgroup_memory_within_defaults` checks the formula `4 * (bm + bn) * (bk + pad)` for a vec4 tile; `generated_sources` and its `want` list name each source; `every_kernel_fits_the_default_limits` checks each one with naga; `every_shader_file_is_checked` wants each shader file in `SHADER_FILES` or `GENERATED_FROM`.  `.claude/rules/gpu.md` lists these.
* `roformer-gpu/src/bench.rs`: `shapes()`, `measure` (`?bench`), `calibrate`, `decide`, `still_in`, and `CALIBRATION` (3).  Calibration times the large products on `time qkv`, `time ff down` and `freq ff up`, and the small products on `time gates` and `time p·v`.  It weights each shape by `per_chunk`.  `decide` takes the plain fastest kernel of each kind.  The `time p·v` product is not vec4-whole, so a vec4 kernel runs it as `i64` under its own name.
* `?bench=<names>&bench_rounds=<n>` (`worker/src/web.rs`) times only those kernels.  Each bench line is `bench: <shape> (<m>x<n>x<k>, x<batch>) <tile>, <format> weights: <rate> GFLOP/s (median of <n>, spread <s>%)`.  `cargo run -p xtask -- e2e bench --browser <b> --query <q> --dist <dir> --out <dir>` opens the page and waits for `bench finished`.
* `worker/src/web.rs` logs each chunk as `chunk <done>/<total>: <s> s; ...` with one decimal (`{:.1}`).  `chunk_times` in `perf.rs` reads it with `number_after`, which reads any decimals.
* `xtask/src/e2e/perf.rs`: `PerfResult` (the chunk times, `chunk_median`, `chunk_lower_quartile`, `whole_secs` from the line `separator: 6 stems in <s> s`, the kernels), `gate`, `tolerance_pct`, `new_entry` and `update_refusal`.  `update_refusal` lets a raise of the bound through without `--raise` when the browser version differs.  The test `a_raise_over_the_tolerance_needs_a_reason` asserts that.  `e2e perf --dist <dir>` runs a page from another folder.  Each run writes `<out>/perf-<browser>-<n>.json`.
* The text on `--raise` today: `.claude/rules/gpu.md` ("Speed"), `README.md` (the `perf` paragraph), `docs/references/local-browser-testing.md` (the perf lines), the module doc of `xtask/src/e2e.rs`, and the doc of `update_refusal`.
* The M3 harness for the controls is not on the branch.  It is in patches 0001 and 0003 of `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m3/variants/`.  Patch 0001 adds `roformer-gpu/src/m3.rs` (`EXTRAS`, `extra_source`, `controls`), `Kernel::Extra`, `Kernels::extra_pipeline`, `Rec::extra`, a `pub(crate)` `bench::time_rec`, and the two `controls` calls in `bench` of `worker/src/web.rs`.  Patch 0003 adds `wg_source` and the `wg10k` and `wg0` controls.  Port only these parts.
* `xtask/src/e2e.rs`, `WORKER_HEADROOM_MB` (-17 MB) and its doc: the proof of M1.  The parent's "Validation and acceptance" gives the cases of its measurement.

### Terms

* Page, set, ratio and effect: as in Terms of `plans/completed/gpu-speed-m3.md`.  A page is one browser session that runs `?bench` once.  A set is the pages of one experiment, in the order Safari, Chrome, Safari, Chrome.  A ratio is a time over the time of its reference kernel in the same page.  An effect is a ratio whose distance from 1 is larger than the larger spread of the two figures, in both pages of a browser.
* Candidate: a new `TILES` entry with its f16 variant.  M4 has two: `v64k8` and `r64` (Step 6).
* The reference kernel: the kernel that calibration chose at the base for that kind of product, in that browser.  At `9769916` it is `v64` for both kinds in both browsers.
* Calibration total: for one kind of product (large or small), the sum over calibration's shapes of that kind of `per_chunk x flops / rate`.  It is what `calibrate` compares.
* Base page and head page: the page built from the commit before a change, and the page built from the commit with it.
* A run: one separation of `e2e perf`, in a new browser profile.

### The paths of this run

* The scratchpad of M4: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/`.  It belongs to one session, and is not kept.  So each figure also goes into this plan's Surprises at the end of its step.
* The logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/runs/<step>-<browser>-<n>/`.  Each figure in a table names its log.
* The scratch worktree of Step 4: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/wt`, on the local branch `scratch/m4-controls`.  Never push it (GIT-1).  Its target folder: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/wt/target`.
* The base worktree of each same-session comparison: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/base-wt`, detached at the base commit.  Build its page with `CARGO_TARGET_DIR=/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/base-wt/target`, then `trunk build --release` in its `app/`.  Its page is `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/base-wt/app/dist`.  Remove it after each comparison with `git worktree remove`.
* The head page: `/Users/lloyd/Documents/rustt/stemyla/app/dist`, from `(cd /Users/lloyd/Documents/rustt/stemyla/app && trunk build --release)`.
* Weights: `STEMYLA_PERF_WEIGHTS=/Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors`, also given as `--weights`.  Git ignores it.  Never commit it (ASSET-1).
* Chrome: `CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.  Safari: through `safaridriver`, which `xtask` drives.  Never quit the maintainer's Safari.  Read `docs/references/local-browser-testing.md` first.
* Run one browser run or perf gate at a time.  `caffeinate -dimsu` runs; never stop it.  Before each browser run, check with `ps -A -o pid,etime,command` that no other `e2e` or `check --tier push` runs.

## Plan of work

The simplest steps first.  Each step that changes behaviour starts with a test that fails on an assert.  A step whose result makes a later step pointless records that in Surprises and skips it.

Not done, and why:

* No `?bench` harness for the controls on the branch.  The controls only predict BK 8's gain; the candidates' own `?bench` ratios decide.  So the controls run on a scratch branch, as in M3.
* No dump of Safari's own MSL: it changes the maintainer's Safari settings (parent, Decision log, i1-4).
* No `subgroups` (the maintainer's decision 1).

### Step 1: the quirk entry (the first commit)

In `docs/references/browser-quirks.md`, entry "Safari sets workgroup memory to zero with one thread", the bullet "The code here" says that a kernel that does more work for each byte of workgroup memory loses less in Safari.  M3 measured against it: `v64-g4` and `v128` did more work for each byte, and neither was faster in Safari.  Write it again:

* The cost grows with the bytes of workgroup memory and the number of workgroups, by WebKit's source.  M3 measured it only at 10 KiB and at 0 KiB.
* More work for each byte did not help in Safari: `v64-g4` (1.014 to 1.033 times `v64` at K 256) and `v128` (1.30 to 1.59).
* M4 of `plans/active/gpu-speed.md` tries fewer bytes for each workgroup on the same grid.

Acceptance: `cargo run -p xtask -- docs-check` passes.  The commit holds only this file.

### Step 2: every raise of a bound needs `--raise`

1. In `xtask/src/e2e/perf.rs`, change the test `a_raise_over_the_tolerance_needs_a_reason` first.  A raise with a new browser version and no reason is refused.  The refusal text names the old and the new version, and asks for a reason that names the browser update.  Run it, and see it fail on its assert.
2. Change `update_refusal`: a new bound over the old bound needs a reason that is not blank, for any browser version.  A bound that is not higher needs none.
3. Fix each text of "The code today" that says that a new browser version needs no reason: the doc of `update_refusal`, its refusal message ("with the same browser version"), the doc bullet of `a_raise_over_the_tolerance_needs_a_reason` ("passes with a new browser version"), the module doc of `xtask/src/e2e.rs`, `README.md` and `docs/references/local-browser-testing.md`.  After a browser update, the reason names it, for example `--raise "Safari 27.2 to 27.3"`.
4. `.claude/rules/gpu.md`, "Speed", says "With the same browser version, a raise of the bound also needs `--raise`".  Its new text: "A raise of the bound also needs `--raise`, with any browser version."  Give the orchestrator a step checkpoint with this diff in `m4/`.  Wait until it applies the diff.  Then the commit of this step adds `.claude/rules/gpu.md` by name.

Acceptance: the changed test fails before the change and passes after it.  `cargo test -p xtask` passes.  `cargo run -p xtask -- docs-check` passes.

### Step 3: the WGSL of each current kernel, pinned

1. Add a test in the tests of `roformer-gpu/src/lib.rs`, for example `current_kernels_keep_their_wgsl`.  It takes each named source of `generated_sources()` (the matmul kernels and their f16 variants, fused attention) and of `SHADER_FILES` (`softmax.wgsl`, `rmsnorm.wgsl`, `elementwise.wgsl`).  It computes the FNV-1a hash of each, and compares it with a pinned value, by name.  A name with no pinned value is allowed only if it is on a list of the new candidates' names, which starts empty.  So each current kernel is pinned, and a new kernel must be named.  Step 7's commit moves each candidate that lands from that list to a pinned value, its f16 variant included.  At the end of M4 the list is empty.  Use the FNV-1a of `kernels_id`; move it to a function that both use, with no change of `kernels_id`'s value.
2. The test fails first: put one changed byte in a comment of `shaders/matmul_vec4.wgsl`.  Run the test, and see it fail on its assert for `v64` and `v64-f16`.  Remove the change.  Record the failed run in Progress.
3. Its doc says what it asserts, as a list.  It also says why: calibration protects the Radeon and the iPhone from a new candidate, not from a changed old kernel.

So a candidate must not change `matmul_vec4.wgsl`, `matmul_interleaved.wgsl` or `matmul.wgsl`, or the code that fills them for the current tiles.

Acceptance: the test fails on the planted byte and passes without it.  `cargo test -p roformer-gpu --lib` passes.

### Step 4: the `wg` controls (scratch only)

Does the zero-init cost go down with the bytes, at the same grid?  M3 measured only 10 KiB and 0 KiB.  The parent asks for this control first.  It is evidence for the quirk entry and a prediction of `v64k8`'s gain.  It is not a gate: Step 6 builds `v64k8` in any case, and Step 7's own ratios decide.

1. `git worktree add /private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/wt -b scratch/m4-controls HEAD`.
2. On that branch only, port by hand from patches 0001 and 0003 the parts that "The code today" names.  Patch 0003 does not apply on 0001 alone: its context holds patch 0002's `attention-u`, and its indices (`WG10K = 4`, `WG0 = 5`) count it.  Leave out `attention-u`, the variants and the FMA and copy controls.  Give `wg_source` the count of vec4s as an argument, not a fixed 640.  `EXTRAS` holds three controls, each with almost no work, at `v64`'s grid on `freq qkv` (24 x 86 workgroups, 2,064):
   * `wg10k`: 640 vec4s (10 KiB), as M3's;
   * `wg6k`: 384 vec4s (6 KiB), as `v64k8` has;
   * `wg0`: no workgroup memory.
3. Build with the worktree's own target folder.  Run one set (Safari, Chrome, Safari, Chrome) of `e2e bench --query "bench=v64&bench_rounds=5" --dist <the worktree's app/dist>`.
4. Record, for each browser and page: the time of each control, and `(wg6k - wg0) / (wg10k - wg0)`.  If the zero-init cost is linear in the bytes, this is about 0.6 in Safari.

The reading: `(wg10k - wg6k)` for each dispatch, against `v64`'s time on `freq qkv`, is the predicted gain of `v64k8` there.  Add the figures, and whether the cost is linear in the bytes, to the quirk entry of Step 1, in a commit of this step.

Then `git worktree remove` the worktree, and `git branch -D scratch/m4-controls`.  Keep its patch in `m4/controls.patch` (`git format-patch`).

Acceptance: the table in Surprises, each figure with its log.  The quirk entry gives the measured cost at 6 KiB.

### Step 5: the same-session comparison

The tool that shows "no loss" and a gain of 3 % or more, for M4, M5 and M6.  The goal and the cases are in the parent's "Validation and acceptance".  The design:

* `cargo run -p xtask -- e2e perf --browser <b> --weights <w> --base-dist <dir> --runs <n>`.  The base runs use the page in `<dir>`, and the head runs use `--dist` (default `app/dist`).  Both use HEAD's `xtask`.  `--runs` is the count of each side: even, and 4 or more.  `--base-dist` refuses `--gate` and `--update-baselines`.  `--query` applies to both sides.
* The base worktree: `git -C /Users/lloyd/Documents/rustt/stemyla worktree add --detach /private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/base-wt <base commit>`.  Build its page as "The paths of this run" says.
* The order: blocks of 4 runs, B H H B.  Each run writes `<out>/perf-<browser>-<base or head>-<n>.json`.  The output gives each run in its order, with the kernels that it chose.
* The figure of each run is its chunk lower quartile, as the gate reads it.  The groups of slow chunks go into the whole time, but not into the lower quartile.  In M2's runs, groups of slow chunks added about 9 % to a Chrome run's whole time, and the lower quartile stayed at 4.70 s (`plans/completed/gpu-speed-m2.md`, Surprises, Step 5.1).  The output also gives the chunk median and the whole time of each run.
* The chunk times are logged to 0.1 s today: steps of 1.4 % to 2.1 %, too coarse for 3 %.  So the first change of this step logs them to 0.01 s (`{:.2}` in the chunk line of `worker/src/web.rs`), with a parse test of a line with 2 decimals.  It lands before the A/A session, so each later base page has it.
* The change of a block: `r = (H1 + H2) / (B1 + B2) - 1`.  In B H H B, a drift that multiplies each run's time by `1 + d i` (run `i`) cancels.  A drift of 1 % for each run and a true loss of 3 % give `r` = 3 %.
* A spoiled block: a side whose two runs in the block differ by more than `N` (the parent's case 4: the runs of one side differ by more than the noise).  A spoiled block does not count.
* The class of a block: a loss when `r > T`, a gain when `r < -T`, else no change.  `T` and `N` are constants (below).  Each compare uses `EPSILON` of `perf.rs`.
* The verdict:
  * Start with 2 blocks (`--runs 4`).  If the 2 blocks are not spoiled and have the same class, that class is the verdict.
  * Else the tool runs one more block, up to 4 blocks (`--runs 8`).  The verdict is the class of 2 blocks that are not spoiled, when no other block that is not spoiled has another class.
  * Else the verdict is undecided.
  * "3 % or more" is printed when the mean `|r|` of the blocks of the verdict's class is 0.03 or more.  The constant's doc says that a true change of exactly 3 % gets it about half of the time.
* So one slow run, or one change of the GPU's state, spoils only its block, and the other blocks decide.  A verdict never rests on one block.
* Undecided is not "no loss" and not a gain.  Run the comparison again in a later session.  If it is undecided again, the change does not land, unless the Decision log records an explicit reason.

Order of the work:

1. The chunk line with 2 decimals, and its parse test (above).  Commit it.
2. Pure functions in `xtask/src/e2e/perf.rs`, with tests that fail first: the order of the runs, the change of a block, a spoiled block, and the verdict from the runs of both sides.  The tests use `T` and `N` by name and a fixed synthetic noise, so they run again after the constants are set.  The cases:
   * a uniform loss of 3.5 % of the head gives a loss and "3 % or more"; one of 2.5 % gives a loss with no "3 % or more" (if `T` is under 2.5 %);
   * a uniform loss of 3 % with a drift of 1 % for each run gives a loss;
   * a uniform gain of 3.5 % gives a gain and "3 % or more";
   * one slow head run of +10 %, the other runs the same as the base, spoils its block, and gives no change after one more block;
   * a slow H1 in block 1 and a slow H2 in block 2, each +6 %, spoil both blocks, and do not give a loss;
   * one block that is a loss and one that is no change give one more block; a third block of no change gives no change;
   * blocks of three classes, or fewer than 2 blocks that are not spoiled after 4 blocks, give undecided;
   * a uniform change of 1.5 x `T` gives that class, and one of 0.5 x `T` gives no change.
3. The option `--base-dist` in `xtask/src/e2e.rs`, with a test of its refusals (`--gate`, `--update-baselines`, an odd `--runs` or one under 4).
4. The A/A session, to set `T` and `N`.  Build a base worktree at HEAD (the same commit, with the chunk line of item 1).  Run `--runs 6` in Chrome, then in Safari: 3 blocks in each browser.  Record `r` of each block, and the difference of the two runs of each side in each block.  Then:
   * `N` = the larger of 1 % and twice the largest measured difference of the two runs of a side in a block.
   * `T` = the larger of 1 % and twice the largest measured `|r|`, rounded up to 0.5 %.
   * `T` must be 2 % or less, so that a uniform 3 % loss is a loss with a margin.  If it is more, decide in the Decision log.  The options are blocks of more runs, or another figure.
   * Each constant's doc gives this proof, as the doc of `WORKER_HEADROOM_MB` does.
5. The check of the constants: a second A/A comparison, as the tool runs it (`--runs 4`), in each browser, after they are set.  It must give "no change".  This is not the session that set them.  If it gives a loss, a gain or undecided: set `T` and `N` again by the rule from both sessions' figures, or decide in the Decision log.  Do not use the tool until a new A/A comparison gives "no change".
6. Write how to run it in `docs/references/local-browser-testing.md`, beside the perf lines, and in the `perf` paragraph of `README.md` and the module doc of `xtask/src/e2e.rs`.

Acceptance: each new test fails first, then passes.  The second A/A comparison gives "no change" in both browsers.  The figures of both A/A sessions are in Surprises.

### Step 6: the candidates

Two candidates, each a new `TILES` entry with its f16 variant.

* `v64k8`: `v64` with BK 8.  64 x 64 outputs, 4 x 4 for each thread, vec4 reads, a pad of one vec4: `SV = 3`, and (64 + 64) x 12 x 4 B = 6 KiB of workgroup memory.  Two barriers for each K tile, as `v64` has.  Each tile is 128 vec4s, so the 256 threads load both tiles in one step: threads 0 to 127 load A, and threads 128 to 255 load B.  A group of 32 threads is all on one side, so the branch does not diverge in a SIMD group.  `vec4_source` keeps its assert for a tile of 256 vec4s or more, and makes this stage only for a tile of 128.  `v64`'s generated text does not change (Step 3).
* `r64`: a register-tiled kernel with no workgroup memory.  The same grid and thread map as `v64`: 64 x 64 outputs for each workgroup, thread (tx, ty) owns rows `ty + 16 i` and columns `tx + 16 j`.  For each 4 values of K, each thread reads its 4 A vec4s and 4 B vec4s from storage, and adds the products as `v64` does: component x, then y, z, w, one `fma` at a time.  No barrier.  It has its own template, for example `shaders/matmul_vec4_direct.wgsl`, in `GENERATED_FROM`.  Its `a_get` and `b_get` are the same text as in `matmul_vec4.wgsl`, so `f16_source` changes its B read with `replace_once`.  A field of `Tile`, for example `direct: bool`, selects it.  The code needs these, by `lib.rs` today:
  * `r64` has `vec4: true`.  So `MatMul::kernel` sends a product that is not vec4-whole to the fallback, and `bench::measure` maps it the same way.
  * `Tile::source` tests `direct` before `vec4`.  `f16_source` takes the vec4 texts for it.
  * The new template has `const KV: u32 = 1u;`, so that `a_get(ty + 16u * i, k0, row0, a_base)` reads row `ty + 16 i` at K offset `k0`, and `k0` steps by 4.
  * `workgroup_memory_within_defaults` has a branch for `direct` (0 bytes).  `bank_conflicts` leaves `direct` tiles out of its vec4 loop.

For each candidate:

1. The tests first, each failing on an assert before the code:
   * The `want` list of `every_kernel_fits_the_default_limits` names it and its f16 variant (GPU-1).  `generated_sources` takes it from `TILES` with no edit.  The list of new names of Step 3's test names it.
   * `workgroup_memory_within_defaults`: 6,144 B for `v64k8`; 0 for `r64`.
   * `bank_conflicts`, by the test's model: `v64k8` with its pad gives `[2, 2, 1, 1]`, as `v64` does.  With no pad (`SV = 2`) it gives `[1, 1, 1, 2]`, not `v64`'s `[1, 1, 1, 4]`.  So the unpadded `want` depends on the tile, and this assert fails first.  The doc says that the pad of one vec4 still gives 2-way stores and reads with no conflict.
   * The native GPU tests (`roformer-gpu/tests/`) check each `TILES` entry against the CPU.  Add a test that each candidate gives `v64`'s output bit for bit, in f32 and f16.  Use the calibration shapes and the ragged shapes of `matmul_vec4_variants` (`roformer-gpu/tests/kernels.rs`): a K that is a multiple of 4 but not of 16 is where `v64` adds zero terms, and `fma(0, 0, acc)` is `acc`.  The reason, in the test's doc: each output adds an explicit `fma` chain in K order in the three kernels, and no form changes that order.  The claim is for the native test only: Safari compiles with `MTLMathModeRelaxed` (M3, Step 1).  If the native GPU gives a difference, record it in Surprises and decide in the Decision log; decision 0022 needs only the f16 variant to equal its own f32 kernel.
2. The code.  Bump `CALIBRATION` to 4, with a line in its doc: version 4 adds the candidates of M4.  Change its test.
3. `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passes on this Mac (Metal).
4. Do not commit on the branch yet.  Step 7 decides which candidate lands.  Run `git add -N` on each new file, for example `roformer-gpu/src/shaders/matmul_vec4_direct.wgsl`, then write `git diff` to `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m4/candidates.patch`, so that a new session can go on.  Check it with `git apply --check` in a clean worktree at HEAD.

Acceptance: the tests above fail first and then pass, natively on Metal.

### Step 7: the rule "A change lands only with a shown gain", for a kernel

The goal and the cases are in the parent's "Validation and acceptance".  The rule for a kernel candidate:

1. Build the head page with the candidates in the working tree.  Run one set (Safari, Chrome, Safari, Chrome) of `e2e bench --query "bench=v64,<the candidates>&bench_rounds=5"`.  The bench times f32 and f16 weights.  If the reference kernel (Terms) of a browser is not `v64` (`i64` for the small products, for example), add it.
2. For each browser, page and weight format, and for each kind (large, small): `R` = the candidate's calibration total over the reference kernel's.  `S` = the largest spread of the bench lines that go into `R`.  Give also the ratio of each shape, so that a total does not hide one slower shape.  The `time p·v` product is not vec4-whole: each vec4 kernel takes the same `i64` timing there (`bench.rs`, `measure`), with no spread on its line.  So that term is the same on both sides of `R`, and `S` comes from the other lines.  `time gates` has 18 workgroups.  So a gain on the small kind is not expected; the large kind decides.
3. A gain on a kind in a browser: `R < 1` and `1 - R > S`, in both pages.  A loss: `R > 1 + S` in both pages.  Else no effect seen.  This is the rule for an effect of M3's Terms, for a ratio in one page.
4. A candidate lands only with a gain on a kind, with f16 weights, in at least one browser.  The f32 figures are recorded.  Then each browser's same-session comparison (Step 9) must show no loss.
5. A candidate with no gain in either browser does not land.  Its figures go in Surprises, and its code goes to `m4/<name>.patch`.  If both candidates gain in the same browser, both land: calibration chooses between them.

Then commit each candidate that passes item 4, with its tests, `CALIBRATION`, its pinned hashes (Step 3), and the docs that name the kernels (`docs/ARCHITECTURE.md`, "Running it on the GPU", and the doc of `TILES`).  Write the rule in `docs/references/local-browser-testing.md`, beside the same-session comparison, for M5 and M6.

Acceptance: a table in Surprises of `R`, `S` and the ratio of each shape, for each candidate, browser, page, format and kind, with the verdict.  If no candidate lands, Steps 8 to 11 are skipped, with the reason in Progress.

### Step 8: the self-tests

For each candidate that landed: `cargo run -p xtask -- e2e selftest --browser <b> --weights /Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors --query mm=<kernel>`, in Safari and in Chrome.  Also with no `mm`, the kernels that each browser chooses.

Acceptance: each run passes.  Progress names each run.

### Step 9: the same-session comparison of the kernels

The base is the commit before Step 7's commit.  Run Step 5's tool with `--runs 4`, in Chrome, then in Safari.  A browser chose a candidate when the `kernels:` line of any head run names it.

* No change or a gain in both browsers: the candidate stays.
* A loss that holds in a browser: a defect.  First look at the kernels that the head runs chose.  A calibration that chose the candidate where it is slower is a defect to fix.  If it is not fixed, the candidate is reverted, with the figures in Surprises (the parent's "Extra stop conditions").
* Undecided: as Step 5 says.  Undecided is not "no loss".  Twice undecided, the candidate does not land, unless the Decision log records an explicit reason.
* A browser that chose a candidate must have a gain for it by Step 7's rule in that browser (the parent's case 1).  If it has none, the candidate is reverted, unless the orchestrator accepts it with a reason.  That acceptance goes in the Decision log, and in the PR under "Needs your attention".
* A candidate that fails this step is reverted with a revert commit, and Progress names it.  Then Steps 10 and 11 use the kernels that are left.

Acceptance: the verdict of each browser, with `r` of each block and the kernels of each run, in Surprises.

### Step 10: the cost of each forced flip between near choices

After the new candidates, as M2 Step 5.3 did.  "Near" is the parent's: a fused / split ratio within 10 % of `FUSED_MARGIN` (0.99 to 1.21), or two matmul kernels whose calibration totals are within 10 % of each other.  Read them from the `kernels for this GPU` lines of Step 9's head runs.

1. For each near choice, run 2 forced runs (`e2e perf --runs 1 --query mm=<large>,<small>` or `--query attn=<policy>`) and 2 unforced runs, in turns: unforced, forced, forced, unforced.  The cost is (the forced median - the unforced median) of the chunk lower quartiles, over the unforced median.
2. If a cost plus the drift (5.7 %) is over the tolerance (10 %), decide in the Decision log.  The options: a wider tolerance with its proof, given with `--raise` after Step 2; or a gate key that names the kernels.  Safari's `attn=freq` (9.5 % in M2) is near if its fused / split is 0.99 or more.

Acceptance: the cost of each near flip, and the decision if one is needed, in Surprises and the Decision log.

### Step 11: the baselines

Only for a browser whose calibration chose a new candidate (the parent: "The baselines go down if the GPU chooses it"):

* `cargo run -p xtask -- e2e perf --browser <b> --weights <w> --runs 5 --update-baselines`.  A lower bound needs no `--raise`.  A higher bound is refused: do not give `--raise` for it, since Step 9 showed no loss.  Record the figures in Surprises, and keep the old entry.
* An update in a fast session takes up to about 6 % of the margin (the parent).  Step 10 allows for that drift.

Acceptance: `xtask/perf-baselines.json` changes only for such a browser, with a lower bound, and `entry_problems` passes.  Else it does not change, and Progress says why.

### Step 12: the worker headroom of `memory-budget`

By the parent's cases 1 to 4 of the headroom:

1. Run `cargo run -p xtask -- e2e memory-budget --weights <w>` 3 times in Chrome and 3 times in Safari, at HEAD.  Record the worker's growth of each run, and the margin of check 3.
2. Compute again the smallest lost-cut rise of cut 3 from the current `roformer::separate::working_set` (9.44 MB at M1).  M4 does not change `working_set`, so it can stay.  Say so with the figure.
3. Each growth the same as M1's (57 MB, noise 1 MB) and the margin in the band of the doc: the constant stays.  Record the figures in Surprises.
4. Else follow cases 2 to 4: the wasm test passes, a probe of the live heap at each phase finds no held buffer, and the constant goes midway between the noise and the rise.  If the noise is not less than the rise, decide in the Decision log.  Its doc, the figures in `the_worker_growth_must_fit_the_working_set_budget` and a Decision log line change in the same commit.

Acceptance: `memory-budget` passes 3 times in each browser.  The growths and the margins are in Surprises.

### Step 13: the checks before review round 1

* `STEMYLA_PERF_WEIGHTS=/Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver cargo run -p xtask -- check --tier push` passes, with both perf gates run.  Progress gives both verdicts.  A skip line is not a pass (VERIFY-1).
* `cargo build -p xtask` passes.
* `m4/parent-m4.diff` for the orchestrator, with `git apply --check` passing on HEAD.  It holds Surprises entries with the tables of Steps 4, 5, 7, 9, 10 and 12, and Decision log lines for the rules of Steps 5 and 7 and for each candidate.  One line says that the chunk times have 2 decimals, which reverses the parent's "Not taken: chunk times in 2 decimals".  It also makes the M5 and M6 sections and "Validation and acceptance" point to the rules: Step 7 of this plan and `e2e perf --base-dist`.
* Each change of `.claude/**` or `CLAUDE.md` that no step checkpoint carried, as a diff in `m4/`.

## Validation and acceptance

* The quirk entry no longer says that more work for each byte loses less (Step 1).
* `update_refusal` refuses each raise of the bound without a reason, a new browser version included.  The changed test failed first (Step 2).
* The test of Step 3 failed on a planted byte, and passes.  Each candidate that landed has pinned hashes, and the list of new names is empty.
* The chunk line has 2 decimals, with a parse test.
* The same-session comparison runs with `e2e perf --base-dist`.  Its pure tests failed first.  `T` is 2 % or less, and the second A/A comparison gives "no change" in both browsers (Step 5).
* Each candidate that lands has a gain by Step 7's rule, with f16 weights, in each browser that chose it, or the orchestrator's acceptance in the Decision log and the PR.  Each browser's comparison of Step 9 shows no loss.
* `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passes on this Mac.  `e2e selftest` passes in Safari and Chrome, with `--query mm=<kernel>` for each new kernel and with the kernels that each browser chooses.
* The perf gate passes in both browsers (Step 13).  The baselines change only for a browser that chose a new kernel, and only down.
* The flip costs and the headroom are measured again, with the figures in Surprises (Steps 10 and 12).
* `cargo run -p xtask -- check --tier push` passes, with both gates run.
* The review rounds (`/review-rounds 9769916`) end by the stop rule of decision 0024.

## Idempotence and recovery

Each step is one commit or none, on the branch.  A session that stops reads Progress and Surprises, and goes on at the next open step.  Steps 4 to 12 measure.  Until Step 7's commit, the candidates are in `m4/candidates.patch`.  A browser run that a locked screen or a time limit stops runs again.  A set or a comparison that a browser update splits runs again in full, and Surprises records both versions.  A candidate is reverted with its commit; `CALIBRATION` and `kernels_id` then make each browser calibrate again.  `git worktree remove` and `git branch -D scratch/m4-controls` undo Step 4.  `git worktree remove` undoes each base worktree.

## Progress

* [x] Plan review round 1 (2026-11-13): 2 blockers, 7 should-fix and 3 nits, each applied.
* [x] Plan review round 2 (2026-11-13): 1 blocker, 5 should-fix and 5 nits, each applied.
* [x] Step 1 (2026-11-13): the quirk entry, commit "Quirks: fewer bytes for each workgroup, not more work for each byte".  `docs-check` passed.
* [x] Step 2 (2026-11-13): every raise of a bound needs `--raise`.  The changed test `a_raise_over_the_tolerance_needs_a_reason` failed first on its assert (`"157.0.7000.0" in ""`, perf.rs:1306), then passed.  `cargo test -p xtask` and `docs-check` passed.  The orchestrator applied the change of `.claude/rules/gpu.md`; this step's commit adds it.
* [x] Step 3 (2026-11-13): the WGSL of each current kernel, pinned by `current_kernels_keep_their_wgsl` (12 sources; `NEW_KERNELS` empty).  `fnv1a` and `FNV_OFFSET` serve the test and `kernels_id`, whose value stays `0xd4f493a491195bc8` (printed before and after).  With one changed byte in the first comment of `matmul_vec4.wgsl`, the test failed on its assert: `v64: hash 0xb6bd57ba6e7f0898, pinned 0x2bf06de948042584` and `v64-f16: hash 0xe871347a10b57f98, pinned 0xbb9577526ab450c6`.  Without the byte, `cargo test -p roformer-gpu --lib` passed (51 tests).
* [x] Step 4 (2026-11-13): the `wg` controls, on the scratch branch `scratch/m4-controls` (removed), patch in `m4/controls.patch`.  One set, `runs/s4-{safari,chrome}-{1,2}.log`.  The quirk entry gives the figures.
* [ ] Step 5: the chunk line in 2 decimals, the same-session comparison, and its A/A sessions.
  * [x] 5.1 (2026-11-13): the worker logs each chunk time with 2 decimals.  The test `chunk_times_have_two_decimals` failed first on its assert (`"chunk {done}/{total}: {:.2} s; {}", in worker/src/web.rs`), then passed with `cargo test -p xtask --lib` (779 tests).
  * [x] 5.2 and 5.3 (2026-11-13): the pure functions of the comparison in `xtask/src/e2e/perf.rs` (`BLOCK_ORDER`, `Block`, `compared`, `compare_refusal`), and `e2e perf --base-dist` in `xtask/src/e2e.rs`.  The 5 new tests failed first on their asserts, with stubs: `a_block_runs_b_h_h_b_and_gives_its_change` (`left: [Base, Head, Base, Head]`), `a_uniform_change_gives_its_class` (`left: None`), `a_slow_run_spoils_only_its_block` (no block spoiled), `mixed_blocks_need_more_or_are_undecided` (`left: None`) and `a_comparison_refuses_a_gate_an_update_and_odd_runs` (`"--gate" in ""`).  Then `cargo test -p xtask --lib` passed (784 tests).  `CHANGE_T` and `SPOIL_N` are 2 % until the A/A session sets them.
* [ ] Step 6: the candidates.
* [ ] Step 7: the gain rule, and the candidates that land.
* [ ] Step 8: the self-tests.
* [ ] Step 9: the same-session comparison of the kernels.
* [ ] Step 10: the cost of each forced flip.
* [ ] Step 11: the baselines.
* [ ] Step 12: the worker headroom.
* [ ] Step 13: the checks before review round 1.

## Surprises and discoveries

* 2026-11-13, Step 4: the `wg` controls, one set of `?bench=v64&bench_rounds=5` (Safari 27.2, Chrome 157.0.8103.51, the same in each page; logs `m4/runs/s4-{safari,chrome}-{1,2}.log`).  Each control has almost no work, at `v64`'s grid on `freq qkv` (24 x 86 workgroups, 2,064).  Times in us for each dispatch, medians of 5, at the start and the end of each page.  Ratio: `(wg6k - wg0) / (wg10k - wg0)`.

  | Page | `wg10k` | `wg6k` | `wg0` | Ratio |
  | - | - | - | - | - |
  | Safari 1, start | 414.1 (8.9 %) | 114.5 (0.9 %) | 10.7 (10.0 %) | 0.257 |
  | Safari 1, end | 408.2 (6.8 %) | 113.0 (1.8 %) | 10.8 (8.7 %) | 0.257 |
  | Safari 2, start | 410.2 (5.2 %) | 114.0 (1.8 %) | 11.3 (3.7 %) | 0.257 |
  | Safari 2, end | 409.7 (1.7 %) | 113.5 (1.3 %) | 11.1 (4.1 %) | 0.257 |
  | Chrome 1, start | 17.8 (14.7 %) | 14.0 (0.7 %) | 6.3 (6.3 %) | 0.670 |
  | Chrome 1, end | 17.9 (1.3 %) | 14.2 (1.9 %) | 6.3 (2.9 %) | 0.681 |
  | Chrome 2, start | 18.2 (26.1 %) | 14.2 (1.9 %) | 6.3 (4.8 %) | 0.664 |
  | Chrome 2, end | 17.9 (3.0 %) | 14.3 (8.6 %) | 6.3 (0.6 %) | 0.690 |

  The cost is not linear in the bytes: a linear cost gives 0.6.  In Safari, 6 KiB costs about a quarter of what 10 KiB costs.  The prediction for `v64k8` on `freq qkv` in Safari: `wg10k - wg6k` is 296 to 299 us of `v64`'s 3.53 to 3.57 ms (f16, 1,216 and 1,202 GFLOP/s), a gain of about 8.3 % to 8.4 %.  In Chrome: 3.6 to 4.0 us of 1.99 ms, about 0.2 %.  The cause of the curve is not known.  One guess, not checked: a core holds fewer workgroups at once with 10 KiB than with 6 KiB, and each waits for its own zero-init.

## Decision log

* 2026-11-13, agent (team leader), plan review round 2: the same-session comparison reads the chunk lower quartile of each run, and the worker logs the chunk times to 0.01 s.  This reverses the parent's line "Not taken: chunk times in 2 decimals" (intent review of M2), which rested on a clean whole time.  Why: in M2's Step 5.1 runs, groups of slow chunks added about 9 % to a Chrome run's whole time, and the lower quartile stayed at 4.70 s.  With slow runs about 3 in 10, the whole time would give a false loss or gain in about 16 % of comparisons.  At 0.1 s, the lower quartile has steps of 1.4 % to 2.1 %, too coarse for 3 %.  For the orchestrator to accept.
* 2026-11-13, agent (team leader): the comparison is code (`e2e perf --base-dist`), with pure tests, not a procedure done by hand.  Why: M4, M5 and M6 use it about 10 times in two browsers, and the order of the runs and the ranks are easy to get wrong by hand (VERIFY-1).
* 2026-11-13, agent (team leader), plan review rounds 1 and 2: the verdict reads blocks of B H H B, and needs 2 blocks of the same class that are not spoiled.  Not the fastest runs of each side.  Why: with a drift of 1 % for each run, the fastest runs are the early ones, and a true loss of 3 % gave a rank change of only 2 %.  A block cancels a linear drift.  One slow run or one change of the GPU's state spoils only its block (the parent's cases 2 and 4).
* 2026-11-13, agent (team leader), plan review round 1: `T` comes from an A/A session, base and head of the same commit, at the block size that the comparisons use.  A second A/A comparison checks it.  Why: the noise of this rule in one session was not measured, and a check from the same session that set `T` passes by construction.
* 2026-11-13, agent (team leader), plan review round 1: the `wg` controls run on a scratch branch, do not land, and do not gate `v64k8`.  Why: they only predict BK 8's gain, and Step 7's own ratios decide.  The parent asks for them first, and their figures go into the quirk entry.
* 2026-11-13, agent (team leader), plan review rounds 1 and 2: the pin of Step 3 covers each generated source and each shader file, not only the kernels that calibration chooses, and each candidate that lands.  Why: calibration protects none of them once they are current, and M5 and M6 use the pin.
* 2026-11-13, agent (team leader), plan review round 2: a candidate is committed when it has an in-page gain (Step 7), and reverted when Step 9 finds a loss, twice undecided, or a browser that chose it with no gain.  Why: Step 9's base is the commit before the candidate.

* 2026-11-13, agent (implementer), Step 5.1: the chunk line keeps PROTO-1 with no change of `stem-protocol`.  Why: the line is the text of a `Reply::Log` message, whose encoder and decoder do not change.  The one writer of the text is `worker/src/web.rs`.  The one reader is `chunk_done` and `chunk_times` in `xtask/src/e2e.rs` and `xtask/src/e2e/perf.rs`.  The test `chunk_times_have_two_decimals` reads the writer's format in `worker/src/web.rs`, and parses a line of 2 decimals in the same commit.  `xtask` does not take `stem-protocol`, so a shared format function there would add a dependency for one log line.
* 2026-11-13, agent (implementer), Step 5.2: the verdict takes the class of 2 blocks or more that are not spoiled, with at most one block of another class, and never with a loss and a gain together.  Why: the plan's case "a loss, then two blocks of no change, gives no change" needs one block of another class to be allowed.  A loss and a gain together say that the noise is larger than `T`, so they give undecided.
* 2026-11-13, agent (implementer), Step 5.2: the test of a uniform loss of 3 % with a drift uses a drift of 1 % for each run only when `SPOIL_N` is 4 % or more; else a quarter of `SPOIL_N`.  Why: B1 and B2 of a block are 3 runs apart, so a drift of 1 % for each run puts them 3 % apart, and a smaller `N` spoils each block.  The drift still cancels in `r`.
* 2026-11-13, agent (implementer), Step 5.2: a spoiled block is one whose two runs of a side differ by more than `N` of the faster run.

## Outcomes and retrospective

Not yet.
