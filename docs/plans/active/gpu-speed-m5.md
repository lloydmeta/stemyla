# GPU speed, M5: a flash-style time-axis attention

## Purpose

This is the milestone plan for M5 of `plans/active/gpu-speed.md`.  The parent's M5 section and its "Validation and acceptance" stay the criteria.

M5 has two parts, in this order:

1. Calibration chooses the time-axis attention policy with a timing that does not match the separation (TD-101).  Safari's calibration times split attention about 0.5 s for each chunk too low, and Chrome's about 0.26 s.  So Safari chooses the slower `freq` policy in about 10 of 22 calibrations.  M5 finds the cause, fixes the timing, and closes TD-101 by the goal "Each calibration choice is not slower".
2. Then M5 tries a new fused attention kernel for the time axis, as a third calibration candidate.  Its start point is `attention_tiled.wgsl`.  It lands only with a shown gain.

Readers: the implementer, the team leader, the orchestrator, and the team leader of M6.

## Context and orientation

The state at the start, on 2026-11-13, at commit `b7f980f` on branch `gpu-speed/2026-11-12`.  The review base is `b7f980f`.  The machine is the maintainer's Apple M2 Pro (19-core GPU), with Safari 27.2 and Chrome 157.

### The decisions that M5 follows

* The maintainer, 2026-11-12: kernels stay within WebGPU's default limits (GPU-1: 16 KiB of workgroup memory, 256 invocations).  No `subgroups`.  The perf gate runs on this M2 Pro only, with tolerances that are not too tight.  Never wait for a run on the Radeon or the iPhone.
* The maintainer, 2026-11-13: never search the whole disk or the home folder.  Give every agent absolute paths.  Before a report, check that no agent or command still runs.  `v64k8` stays with Safari's split choice until M5 closes TD-101.
* The orchestrator, for M5: first find why calibration times `freq` too low, in both browsers.  Then close TD-101 by the goal "Each calibration choice is not slower", with its cases.  Add a tool that forces a policy on the head runs only, with a test that fails first.  The policy names are `fused` and `freq`.  Never force `split`.
* The orchestrator: M5's first commit fixes TD-101's text: about 10 of 22 calibrations, and the same bias in Chrome.
* The orchestrator: a new kernel lands only by "A change lands only with a shown gain".  The reference is each policy that calibration chooses at M5's base in that browser: `freq` in Chrome, `fused` and `freq` in Safari.  Until TD-101 is fixed, a ratio against `freq` does not count.  If no change is faster in either browser, M5 lands no kernel and records why.
* The orchestrator: each flip-cost run pins every other choice in its `--query`.  The WGSL of each current kernel stays the same byte for byte (`current_kernels_keep_their_wgsl`).
* The parent: M5 keeps the fused kernel's structure: 64 queries of one head for each workgroup, 4 x 4 for each thread, online softmax.  No unrolled inner loop.  The candidate is checked against the CPU.  Its dispatches take their profile label from `attention_label`.
* Lessons of M3 and M4: a build in a worktree uses its own `CARGO_TARGET_DIR` inside that worktree.  One browser run or perf gate at a time.  If a Safari run fails on a lost WebDriver session, run it once again (the maintainer can use Safari).  Write the edge cases of a rule into the plan before the code.  Put each choice of an I/O function in a pure function with a test.

### What is known

* The time-axis policies (`AttentionPolicy`, `roformer-gpu/src/lib.rs`).  `fused`: fused attention on both axes.  `freq`: split attention on the time axis, fused on the frequency axis.  `split` also splits the frequency axis, which is 2.2 to 3.6 times slower, so no run forces it.
* A time-transformer step (`Job::run`, `roformer-gpu/src/net.rs`) runs the whole transformer layer for some bands: the norms, `qkv`, rope, gates, attention, the output product and the feed-forward products.  `per_seq` sets how many bands a step takes.  With `freq` a step takes 1 band of 1,151 frames: 62 steps for each layer, 744 for each chunk.  With `fused` it takes 4 bands: 16 steps for each layer (the last of 2 bands), 192 for each chunk.  So the policy also changes the M of each linear product of the time transformer: 1,151 with `freq`, 4,604 with `fused`.  `Job::layer_steps` makes the step list.
* `freq` holds a scores buffer of 8 x 1,151 x 1,151 floats, about 42 MB of GPU memory, that `fused` does not hold (`Job::new`).  `FUSED_MARGIN`'s doc gives this memory as its reason.  The tab totals of `memory-budget` (M1, M4): 799 MB in Chrome, which chooses `freq`, and 758 MB in Safari with `fused`.  Both are under `TAB_BUDGET_MB` (943 MB, MEM-4).
* Calibration (`bench::calibrate`).  It times the matmul kernels on the shapes of `shapes()`: the time transformer's at M 1,151 (1 band), for both policies, and `freq ff up` at M 5,456.  Then `attention_per_chunk` times only the attention part: `split_attention` (scores, softmax, P·v, gate) on 1 band, times 744, and `fused_attention` on 4 bands, times 186.  `time_rec` queues up to 64 submissions before each wait for the GPU (`WAIT_SECS` 0.05 s).  `decide` keeps the time axis fused unless split measured faster by more than `FUSED_MARGIN` (1.1).
* The separation (`Pace::run`, `worker/src/web.rs`) submits one step, then waits for the GPU (`Gpu::done`), for each step.
* Leads, from the parent and M2:
  * the linear products of the time transformer run at M 1,151 with `freq`, and at M 4,604 with `fused`.  Calibration times neither form against the other;
  * 552 more steps for each chunk with `freq` (744 against 192), each with a wait.  M2 measured the gaps between steps at 14 % of step time in Chrome (with `freq`) and 1 % in Safari (with `fused`), with profiling.  So the waits can explain most of Chrome's bias, but not Safari's 0.5 s;
  * the other dispatches of a step (norms, rope, gates) at the smaller size.
* The figures (M2, M4).  Safari, with `v64k8`: calibration times split 3.11 to 3.16 s and fused 3.42 to 3.51 s for each chunk.  Forced runs: `fused` 7.225 s and `freq` 7.47 s (medians of the chunk lower quartiles), so `freq` costs about 0.25 s for each chunk.  Chrome at M2: calibration split 1.90 s, fused 2.26 s.  A forced `fused` costs 2.1 % (about 0.10 s for each chunk).  So calibration's (split - fused) is too low by about 0.57 s in Safari and 0.26 s in Chrome.
* Fused attention today (`attention_tiled.wgsl`, generated by `attention_source` in `lib.rs`): 64 queries of one head for each workgroup of 256 threads, 4 x 4 for each thread, keys 64 at a time.  Q and K slices of 16 dimensions are staged transposed in `buf`, and each thread reads them as scalars: 8 reads for 16 multiply-adds.  Row maxima go through `red`.  Workgroup memory: 2,112 + 1,024 floats, 12.25 KiB.  It reloads the Q slices for each key tile.
* M3 (`plans/completed/gpu-speed-m3.md`, Surprises, Steps 4 and 6):
  * `i64` (scalar workgroup reads, 4 x 4) over `v64` (vec4 reads, 4 x 4) on the K 256 and K 1,024 products: 1.25 to 1.58 in Safari, 1.44 to 1.50 in Chrome.  The fused kernel reads as `i64` does.
  * Time-axis attention for each chunk: Safari fused 3.47 s, split 3.53 s; Chrome fused 2.29 s, split 1.85 s.
  * Safari's zero-init of workgroup memory is about 2 % of a fused dispatch.  An unrolled form was 3.6 times slower in Safari, 1.33 in Chrome.
* M4: the zero-init cost in Safari is not linear in the bytes: 6 KiB costs 0.257 of 10 KiB (`references/browser-quirks.md`).

### The code today

* `roformer-gpu/src/bench.rs`: `time_one`, `time_rec`, `WAIT_SECS`, `split_attention`, `fused_attention`, `attention_per_chunk`, `measure_attention` (`?bench`), `calibrate`, `decide`, `FUSED_MARGIN`, `ATTENTION_WARM`, `ATTENTION_ROUND`, `CALIBRATION` (4), `Choice`, `kernels_line`.  The line `kernels for this GPU, ...` gives calibration's split and fused times for each chunk.
* `roformer-gpu/src/lib.rs`: `AttentionPolicy` (`name`, `parse`), `attention_source`, `attention_label`, `Gpu::check_attention` and `Gpu::fused_ok`, `generated_sources`, `kernels_id`, and the tests `every_kernel_fits_the_default_limits`, `current_kernels_keep_their_wgsl` (`PINNED_WGSL`, `NEW_KERNELS`) and `workgroup_memory_within_defaults`.
* `roformer-gpu/src/net.rs`: `per_seq`, `Job::steps`, `Job::run`, `Job::transformer`.
* `roformer-gpu/tests/`: the native GPU tests check fused attention against `attention_cpu`.
* `worker/src/web.rs`: `Pace`, `ensure_kernels` (calibration runs after the model upload; a URL with both `mm=` and `attn=` skips it, "from the URL"), `apply_attention`, `bench`.
* `xtask/src/e2e.rs`: `Opts` (`--query`, `--base-dist`, `--runs`), `perf_compare`, `CompareSites`, `perf_run`.  `xtask/src/e2e/perf.rs`: `run_comparison`, `compare_refusal`, `profile_groups`, `MEASURE_KEYS`.  `--query` applies to both sides of a comparison.
* `docs/references/local-browser-testing.md`: "The same-session comparison" and "A kernel lands only with a shown gain".
* `docs/tech-debt.md`, TD-101.  `docs/ARCHITECTURE.md`, "Running it on the GPU", the bullet "What varies by GPU is measured, once".

### Terms

* Page, set, ratio, effect: as in Terms of `plans/completed/gpu-speed-m3.md`.  Base page, head page, run: as in Terms of `plans/completed/gpu-speed-m4.md`.
* A policy: a time-axis attention policy, by the name that calibration writes in the `kernels:` line.  `fused`, `freq`, and the candidate's name if it lands (Step 8).
* A forced run: an `e2e perf` run whose `--query` sets `mm=<large>,<small>` and `attn=<policy>`.  So calibration does not run, and every choice is pinned.
* The calibration bias of a browser: calibration's (split time - fused time) for each chunk, minus the separation's (forced `freq` - forced `fused`) for each chunk, from the chunk lower quartiles.  Today about -0.57 s in Safari and -0.26 s in Chrome.
* Choosable: from at least 12 calibrations of one page in one browser (`N`), a policy is choosable when either holds:
  * calibration chose it at least once.  A policy that calibration draws 22 % of the time or more shows up in 12 calibrations with a chance of 95 % (1 - 0.78^12);
  * the mean of calibration's ratio at the flip point (the faster fused kernel's time over split's), plus or minus 2 standard deviations over the `N` calibrations, crosses `FUSED_MARGIN`.  For two fused kernels, the same with their ratio and 1.
* At M5's base, by the M4 runs: `fused` and `freq` are choosable in Safari; `freq` alone in Chrome, which chose it in each run.
* Near, for the flip costs only (the parent's M4 rule), for three policies.  Calibration takes the faster of the fused kernels (`fused`, the candidate), as `decide` takes the faster matmul kernel.  Then it keeps that one unless split is faster by more than `FUSED_MARGIN`.  Two choices are near when:
  * the two fused kernels' times are within 10 % of each other;
  * the faster fused kernel's time over split's time is within 10 % of `FUSED_MARGIN` (0.99 to 1.21 at 1.1).

### The paths of this run

* The scratchpad of M5: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/`.  Write each path in full in each command and each brief.  The scratchpad is not kept, so each figure also goes into this plan's Surprises at the end of its step.
* The logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/<step>-<browser>-<n>/`.  Each figure in a table names its log.
* The base worktree of each comparison: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/base-wt`, detached at the base commit.  Build its page with `CARGO_TARGET_DIR=/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/base-wt/target`, then `trunk build --release` in its `app/`.  Its page is `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/base-wt/app/dist`.  Remove it after each comparison with `git worktree remove`.
* The head page: `/Users/lloyd/Documents/rustt/stemyla/app/dist`, from `(cd /Users/lloyd/Documents/rustt/stemyla/app && trunk build --release)`.
* Weights: `STEMYLA_PERF_WEIGHTS=/Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors`, also given as `--weights`.  Never commit it (ASSET-1).
* Chrome: `CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.  Safari: through `safaridriver`, which `xtask` drives.  Never quit the maintainer's Safari.  Read `docs/references/local-browser-testing.md` first.
* `caffeinate -dimsu` runs; never stop it.  Before each browser run, check with `ps -A -o pid,etime,command` that no other `e2e` or `check --tier push` runs.

## Plan of work

The simplest steps first.  Each step that changes behaviour starts with a test that fails on an assert.  A step whose result makes a later step pointless records that in Surprises and skips it.

Part 1 (Steps 1 to 6) fixes calibration before Part 2 (Steps 7 to 14) adds a candidate, since calibration chooses the candidate by the same timing.

### Not done, and why

The parent asks for each change from `attention_tiled.wgsl` with the figure that predicts its gain.  This table names each change.  Only C1 has a predicted gain larger than the spread of a `?bench` figure (Safari up to 4.4 %, Chrome up to 1.5 %, M3 Step 3).  So M5 builds C1 only.

| Change | Predictor | Predicted gain of the time-axis attention | Built |
| - | - | - | - |
| C1: vec4 reads of workgroup memory in the scores and the values loops, as `v64` reads | `i64` over `v64` (M3 Step 4): 1.25 to 1.58 in Safari, 1.44 to 1.50 in Chrome.  Step 7 measures it again on `time scores` (K 64) | an upper bound, if the K loops are the whole dispatch: 20 % to 37 % in Safari (0.69 to 1.28 s of fused's 3.47 s for each chunk, 9 % to 17 % of a 7.22 s chunk).  In Chrome, 31 % to 33 % of fused's 2.29 s: 0.70 to 0.76 s.  In the separation `freq` is about 0.10 s faster than `fused` for each chunk, so the gain over `freq` is about 0.60 to 0.66 s, 13 % to 14 % of a 4.70 s chunk.  The share of the K loops is not known; the softmax and the stores do not gain | yes, if Step 7 confirms the predictor in a browser |
| C2: no `red`, or 6 KiB of workgroup memory | the zero-init is about 2 % of a fused dispatch in Safari (M3 Step 6); M4's curve | at most 2 % in Safari, about 0 in Chrome: under the spread | no |
| C3: each Q block staged once | a 64 x 64 f32 Q block is 16 KiB, all of GPU-1's workgroup memory, with no room for K.  In registers, it is 256 floats for each thread | does not fit GPU-1 | no; so the cost of the reloads is not measured |
| C4: fewer barriers, or a larger K slice | `v64-bar` 1.00 (M3 Step 4) | about 0 | no |
| C5: an unrolled loop | `attention-u` 3.6 times slower in Safari, 1.33 in Chrome (M3 Step 6) | a loss | no (the parent) |
| C6: `exp2` with a scaled `alpha` | 1 `exp` for each 128 multiply-adds | under 1 % | no |

Also not done:

* No `subgroups`, and no f16 arithmetic (the parent's Decision log).
* No wider `FUSED_MARGIN` to hide the bias (M4's Decision log): it changes the choice on GPUs that this Mac cannot measure.  Step 5 can set it lower, only by its decision point.
* No dump of Safari's own MSL (the parent, i1-4).

### Step 1: TD-101's text (the first commit)

In `docs/tech-debt.md`, TD-101, write the title again: calibration times split attention too low, in Safari and Chrome.  Write the figures again:

* Safari chose split in about 10 of 22 calibrations of Steps 9 to 11 of `plans/completed/gpu-speed-m4.md`, the forced-`mm` runs included.
* Chrome has the same bias: calibration puts split about 0.26 s for each chunk too low against fused (M2 Step 5.3).  Chrome chooses split there, and split is the faster policy in Chrome.
* The fix: M5 of `plans/active/gpu-speed.md` finds the cause, and calibration times the time axis as the separation runs it.

Acceptance: `cargo run -p xtask -- docs-check` passes.  The commit holds only this file.

### Step 2: `--head-query`

A forced policy on the head runs only, for the goal's case 4.

1. Tests first, in `xtask/src/e2e.rs` or `xtask/src/e2e/perf.rs`, each failing on an assert:
   * `parses_scenario_and_options` parses `--head-query attn=freq` into `Opts`.
   * A pure function gives the query of each side, for example `side_query(query, head_query, side)`.  The base gets `--query`.  The head gets `--query` with each key of `--head-query` set to its value: a key of both takes the head value, and a new key is added.  Test: `--query "mm=v64k8,v64k8&attn=fused" --head-query "attn=freq"` gives the base `mm=v64k8,v64k8&attn=fused` and the head `mm=v64k8,v64k8&attn=freq`.  Also a head query with no `--query`, an empty `--head-query`, and a `--query` that starts with `?`.
   * `compare_refusal`, or a new pure refusal, refuses `--head-query` without `--base-dist`.  In a comparison, it also refuses `attn=split` and `attn=time` in `--query` or `--head-query`: both split the frequency axis (the goal's case 3).  A slim denylist, so a new policy name needs no change here.
2. The code: `perf_compare` passes each side its query.  The output names each side's query once, before the first run.
3. A test that each side runs its own query, as `each_side_of_a_comparison_runs_its_own_page` tests the pages.  It fails first with the two queries swapped.
4. Docs: `docs/references/local-browser-testing.md` ("The same-session comparison"), the `perf` paragraph of `README.md`, and the module doc of `xtask/src/e2e.rs`.

Acceptance: each new test fails first, then passes.  `cargo test -p xtask` passes.

### Step 3: the check of the goal, and its failure at `ab40862`

The check of "Each calibration choice is not slower", for one page and one browser:

1. The pins: the matmul policy that this page's calibration chooses most often in that browser, from the `kernels:` lines of its runs.  A second matmul policy is checked only with a reason in the Decision log.
2. One comparison for each unordered pair of policies `{p, q}` of the page, where `p` is choosable: `e2e perf --base-dist <the page> --dist <the page> --query "mm=<pins>&attn=<q>" --head-query "attn=<p>" --runs 4`.  One comparison gives both signs: a loss of the head is a gain of the base.
3. The check fails when a choosable policy is slower than another policy by the comparison: a loss when it is the head, a gain when it is the base.  Else it passes.  An undecided comparison runs again in a later session, as `local-browser-testing.md` says.  Twice undecided fails the check, unless the Decision log gives a reason.

The check must fail first (the goal's case 2).  Build the page of `ab40862` in the base worktree.  In Safari, both policies are choosable there (M4).  The pins are `mm=v64k8,v64k8`, which Safari chose in most M4 runs (Steps 9 to 11).  Run `p` = `freq`, `q` = `fused`.  The expected verdict is a loss of about 2.3 % to 3.4 % (M4 Step 10).  In Chrome, `freq` is the one choosable policy; run `p` = `freq`, `q` = `fused`, with `mm=v64,v64`.  A gain or no change is expected.

Also record each run's whole time and chunk median.  These forced runs are also evidence for Step 4.

Write the check in `docs/references/local-browser-testing.md`, as a new section after "A kernel lands only with a shown gain", for M6.

Acceptance: Safari's verdict at `ab40862` is a loss, in Surprises with each block.  An undecided verdict runs once again.  If it is then not a loss, the check cannot fail first: stop, and give the team leader the figures.

### Step 4: find the cause of the bias

Measure where the separation's cost of `freq` goes, with forced runs.  Today's tools give it; no code change.

1. At HEAD (the page of Step 2), in each browser, 4 forced runs with `--query "profile=1&mm=<pins>&attn=<policy>"`, in the order `fused`, `freq`, `freq`, `fused`.  Then 4 forced runs with no `profile=1`, in the same order.  Use `--runs 1` for each run.  The pins are as in Step 3.
2. From the runs without a profile: the separation's cost of `freq` for each chunk, `D` = the median of the `freq` chunk lower quartiles minus the median of the `fused` ones.
3. From the profile runs: the GPU seconds of each profile group (time-axis attention, frequency-axis attention, linear products, other), divided by the 11 chunks.  For each group, `freq` minus `fused`.  The gaps: (the chunk median with the profile) minus (the sum of the groups for each chunk), `freq` minus `fused`.  A profile adds time to each step, so the gap figure is an upper bound.
4. From the `kernels for this GPU` lines of 4 unforced runs in each browser whose `kernels:` line gives the pins: calibration's split and fused times for each chunk.  Calibration's split time depends on its own matmul choice (M4: 3.13 s with `v64k8`, 3.54 s with `v64`), so a run with other kernels does not count; run more until 4 count.  Compare calibration's split time with the profile's time-axis attention of `freq`, and calibration's fused time with that of `fused`.
5. A table in Surprises, for each browser: `D`, calibration's (split - fused), the bias, and each part of `D`.  The parts are: time-axis attention, the linear products, other, and the gaps.

The predictions, from "What is known":

* Chrome: most of the bias is in the gaps: 552 more waits for each chunk at about 0.5 ms.
* Safari: most of the bias is in the linear products of the time transformer: M 1,151 against M 4,604.  The gaps are 1 % or less in Safari.

The cause is each part whose share of the bias is larger than the noise of `D`.  The noise of `D` is the larger difference of the two runs of a policy.

Acceptance: the table, with the cause in each browser.  If no part explains at least half of the bias in a browser, record it.  Then Step 5 takes form (c), which times all the parts.

### Step 5: calibration times the time axis as the separation runs it

1. The form, by Step 4's table.  Take the simplest form whose parts cover the bias, to within item 6's limit, in both browsers.  Give the proof from the table in the Decision log.  Each form waits after each step, as `Pace::run` does, so it also meets the parent's M6 case 11.
   * (a) The attention timings wait after each step (one submission for each wait).  It covers the gaps only.
   * (b) Form (a), and each policy also times the linear products of the time transformer at its own M and step count: M 1,151 for `freq`, M 4,604 for `fused`.  It covers the gaps and the linear products.
   * (c) A whole time-transformer step of each policy, from one recording function that `Job::run` also uses for `Step::Time`.  It covers each part.  No copy of the step's dispatch list can drift from `net.rs`.
2. Tests first, each failing on an assert:
   * The step list: a pure function, for example `time_steps(frames, budget, fused) -> Vec<usize>` (the bands of each step), gives 62 steps of 1 band with `freq`, and 15 steps of 4 bands and 1 of 2 with `fused`, at 1,151 frames and the default budget.  `Job::layer_steps` calls it.  Calibration times each step of the list, the 2-band step too, or weights each step by its bands.  Today's 186 band-equivalents stay right for the work; the waits are 192.
   * The budget: the worker's default budget (`BUDGET`, `worker/src/web.rs`, 8 << 20 elements) moves into `roformer-gpu`, and the worker uses it from there; or `calibrate` takes the budget.  Calibration times the default chunk at the budget of the run.  A test pins that both use one value.
   * The timing waits after each step: for example, the timing function takes a count of submissions for each wait, and calibration passes 1.  Or a source test, as `chunk_times_have_two_decimals` reads the worker's source.
   * `CALIBRATION` is 5.
   * `roformer-gpu/tests/memory.rs`: calibration frees each GPU buffer that it makes (`Gpu::live_bytes` back to its value before), as `.claude/rules/gpu.md` asks for a new allocation (MEM-3).  `readbacks_checks_and_benchmarks_free_their_buffers` covers `attention_per_chunk` today, so this test is a guard that passes before the change.  Progress says so; it is not a test that fails first.
3. The code.  Calibration uses buffers and zero weights of its own, as `buffers` and `attn_bufs` in `bench.rs` do.  It does not use the loaded `Net` or a `Job`: `?bench` runs with no model, and a `Job` holds `x` and `xt` (about 73 MB each).  `calibrate` drops its matmul buffers (`b`, about 109 MB with f16 weights) before it times the time axis.  Then form (c) holds at most about 113 MB: `qkv` of 4 bands (28 MB), the scores of 1 band (42 MB), the feed-forward's hidden layer (19 MB), `o` (9.4 MB), `x` and `xn` (4.7 MB each), and the weights of one layer (about 4.5 MB).  Today's peak, with `b` and `attn_bufs` (87 MB) alive together, is about 196 MB.  The doc of `calibrate` gives the new peak.  It is less than a `Job`'s buffers at the default chunk (about 310 MB), which calibration frees before the separation makes them.  So the tab total of `memory-budget` does not move (MEM-4).
4. `attention_per_chunk` keeps its attention-only figures for `?bench` (`measure_attention`), so M3's and M4's figures stay comparable.  The doc of `roformer-gpu/tests/calibration.rs` follows the change.  The `kernels for this GPU` line gives the new figures, and says what they cover.
5. Do not commit yet.  Build the head page from the working tree.
6. The accuracy, in each browser: unforced runs of the head page, each a new calibration, until 12 of them give the pins of Step 4 in their `kernels:` line.  A run with other matmul kernels does not count.  For each run, read calibration's new (split - fused) for each chunk, and its ratio at the flip point.  The new bias is the mean of the new (split - fused) minus `D` of Step 4.  Two limits, in each browser:
   * the bias: its size is under half the size of `D`.  Where the size of `D` is under the noise of `D`, the limit is that noise;
   * the spread: where `|D|` over the median of the forced `fused` chunk lower quartiles is more than `T` (1 %), the mean ratio is more than 2 standard deviations from `FUSED_MARGIN`.  So calibration chooses the same policy each time there.
   * If the spread limit fails, the lever is longer timings: more rounds, or a longer `ATTENTION_ROUND`.  Record the extra seconds of calibration.
   * If the bias limit fails, take the next form of item 1, and measure again.  If form (c) fails, stop: give the orchestrator a checkpoint with the figures (the parent's case 6).  TD-101 then stays open.
7. The decision point of `FUSED_MARGIN`, before the commit.  By Terms, take the choosable policies of item 6's 12 calibrations, in each browser.  A browser has a wrong choice when a choosable policy is slower than the other by more than `T`, by Step 4's forced runs (`|D|` over the median of the forced `fused` chunk lower quartiles).  That covers both sides: `fused` choosable where split is faster, and `freq` choosable where fused is faster.  Chrome is predicted to have one: its ratio about 1.03 to 1.05, under 1.1, with `fused` 2.1 % slower.  If a browser has a wrong choice:
   * Option A (recommended): `FUSED_MARGIN` goes to 1.0, with a test that fails first on the old value.  Then do item 6 again with the new margin, so that its spread limit holds at 1.0.  The cost: a GPU whose new ratio is 1.0 to 1.1 then splits the time axis.  By its own timing, that is faster.  It holds about 42 MB more GPU memory at the default chunk (the scores of 1 band).  On this Mac, Chrome's tab total with `freq` is 799 MB, under `TAB_BUDGET_MB` (943 MB, MEM-4).  This Mac cannot measure the Radeon and the iPhone.
   * Option B: do not land the fix.  With `FUSED_MARGIN` at 1.1, that browser chooses its slower policy, and Step 6's comparison shows a loss there, which the parent forbids (gain case 5).  TD-101 stays open with the new figures.
   * Option A changes the choice and the memory on GPUs that this Mac cannot measure.  Option B leaves the goal unmet.  So the team leader gives the orchestrator a checkpoint with the figures, and waits for the decision.  The orchestrator records it in the Decision log, and the PR lists it under "Needs your attention".
8. Before the commit, with the head page of the working tree: `e2e selftest` passes in Safari and in Chrome.  If the form changes `net.rs` (form (c)), `e2e stored-stems` in Chrome gives the same hashes as at the commit before, with `attn=fused` and with `attn=freq`.
9. Commit the form, `CALIBRATION` 5 (one bump covers the margin too), and the decided `FUSED_MARGIN`.
10. Docs, in the same commit: `docs/ARCHITECTURE.md` (the bullet "What varies by GPU is measured, once", and the paragraph of 744 and 186 steps: 192 steps with fused), the docs of `calibrate` and `FUSED_MARGIN`, and TD-101.

Acceptance: the tests fail first, then pass (the guard of item 2 passes before and after).  `cargo test -p roformer-gpu --lib` and `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` pass.  The limits of item 6 hold in each browser, in Surprises.  The orchestrator's decision of item 7 is in the Decision log, if a browser had a wrong choice.  Item 8's runs pass before the commit.

### Step 6: the comparison of the fix, and the goal's check at its head

1. The same-session comparison: base = the commit before Step 5's commit, head = Step 5's commit.  `--runs 8` in each browser, since Safari has a near choice at the base.  No loss in either browser.  A gain is recorded; a gain is not needed, since the fix is a calibration fix, not a kernel.
2. The head's calibrations: the 8 head runs of item 1 in each browser, and 4 more unforced runs, so that `N` is 12 or more.  Record each policy with its count, and the mean and standard deviation of the ratio at the flip point.  This gives the choosable policies (Terms).
3. The check of Step 3 at the head page, in each browser.
4. Safari's baseline: only with a gain from item 1, run `e2e perf --runs 5 --update-baselines` in Safari.  A lower bound needs no `--raise`.  A higher bound is refused: keep the old one, and record it.  The baseline goes in the commit of this step.

Acceptance: no loss in either browser by item 1.  A loss that holds is a defect of Step 5: fix it, or revert Step 5's commit, and TD-101 stays open.  The check of item 3 passes in each browser.  Then TD-101 moves to `## Closed` in `docs/tech-debt.md` in the commit of this step, with what closed it.  If the check fails in a browser, TD-101 stays open with the figures of this step, and the team leader reports it to the orchestrator.  Part 2 starts after this step.

### Step 7: the predictor of C1

One set (Safari, Chrome, Safari, Chrome) of `e2e bench --query "bench=i64,v64&bench_rounds=5&mm=<pins>&attn=fused"` at the head of Step 6.  With both `mm=` and `attn=`, the page does not calibrate, so each page times attention with the same matmul kernels.  `attention_per_chunk` times each policy whatever `attn` is.  The bench also logs the attention figures of `measure_attention`.

* For each page: `i64` over `v64` on `time scores` (K 64, the scores of split attention), with the spreads.  C1's scores loop is K 16 for each slice, and its values loop is K 16 too, so `time scores` is the nearest shape.  Also on `time qkv` (K 256).
* C1 is built when `i64` over `v64` on `time scores` is an effect (M3's rule) in at least one browser.  Else C1 is not built: record the figures in Surprises, and skip Steps 8 to 13.

Acceptance: the table in Surprises, with the verdict.

### Step 8: the candidate

C1 is a new fused attention kernel for the time axis, with the fused kernel's structure.  Its name is `vfused`, for the kernel and for its policy.  The policy `vfused` puts `vfused` on the time axis, and the current fused kernel on the frequency axis.

The design:

* 64 queries of one head for each workgroup of 256 threads.  Thread (tx, ty) owns queries `ty + 16 i` and keys or dimensions `tx + 16 j`, as today.  Online softmax, as today.
* The scores loop reads vec4s along the head dimension: Q and K slices are staged as `[64 tokens][16 dims]`, 4 vec4s for each token, with `v64`'s pad of one vec4.  Each 4 dimensions: 4 Q vec4s and 4 K vec4s, 64 multiply-adds.
* The values loop reads vec4s along the keys, 16 keys at a time: P as `[64 queries][16 keys]`, and V staged transposed as `[64 dims][16 keys]`, with the same pad.
* `red` has two uses today: the row maxima, and the row sums in `FINISH`.  Both use a part of `buf`, not a separate `red`.  `buf` is free between the last barrier of the scores loop and the first stage of P, and after the last barrier of the key loop.  A barrier comes between the last read of the maxima and the first write of P.
* Each output keeps its sum order: dimensions in order for each score, keys in order for each output, one `fma` at a time for the components x, y, z, w.  So on a native GPU it gives the current fused kernel's output bit for bit.
* Workgroup memory: `buf` alone, 2 x 64 x (16 + 4) floats, 10 KiB.  That is less than the current kernel's 12.25 KiB, and within GPU-1.  Safari's zero-init cost is not linear in the bytes, so a layout over 10 KiB needs a reason in the Decision log.  No unrolled loop and no loop-indexed private array.
* `attention_tiled.wgsl` and `attention_source` do not change (the pin).  `vfused` has its own template, for example `shaders/attention_vec4.wgsl`, in `GENERATED_FROM`.

The code needs these, by `lib.rs` and `bench.rs` today:

* `AttentionPolicy` holds which kernel runs on the time axis, not only a `bool`.  `name` and `parse` give `split`, `fused`, `time`, `freq` as today, and `vfused`.  A stored choice of version 5 or older never parses as `vfused`, since `CALIBRATION` changes the key.
* `Gpu::check_attention` checks `vfused` against `attention_cpu` too, with its own flag beside `fused_ok`.  `vfused` is usable only when both flags are true, since its policy keeps the current fused kernel on the frequency axis.  So a GPU whose fused kernel fails gets `split`, as today.
* `net.rs` reads `attention.time` as a `bool` in `Job::new`, `Job::layer_steps` and `Job::transformer`.  Each takes the time-axis kernel instead: `vfused` runs 4 bands for each step, as `fused` does, and needs no scores buffer.  `Rec::attention` takes the kernel to run.  With Step 5's form (c), calibration times `vfused` through the shared recording function.
* `vfused` on the time axis always has the fused kernel on the frequency axis.  The type does not let split go with `vfused` on the frequency axis, so that combination has no name.
* `net.rs` changes, so `e2e stored-stems` in Chrome gives the same hashes before and after, with `attn=fused` and with `attn=freq`.
* Two paths know only `fused_ok` today, and each gets the new rule with a test that fails first: the check of a stored choice in `choose_kernels` (`worker/src/web.rs`, `c.attention == SPLIT || gpu.fused_ok`), and `apply_attention` (`?attn=`), whose message also names `vfused`.
* Its dispatches use `attention_label`, so `profile_groups` counts them as time-axis attention.
* `decide` takes the faster of `fused` and `vfused`, then compares it with split by `FUSED_MARGIN`.  Bump `CALIBRATION`, with a line in its doc.

Tests first, each failing on an assert:

* `every_kernel_fits_the_default_limits` names `vfused` (GPU-1).  `NEW_KERNELS` names it.  `workgroup_memory_within_defaults` gives its bytes.
* `AttentionPolicy`'s parse test: `vfused` round-trips; `split` and the others still parse.
* `decide`: `vfused` faster than `fused`, and not slower than split by more than `FUSED_MARGIN`, gives `vfused`; slower than `fused` gives `fused`; both slower than split by more than `FUSED_MARGIN` gives `freq`; `vfused` failed its check gives `fused` or `freq`; the fused kernel failed its check gives `split`.
* `workgroup_memory_within_defaults` gives `vfused`'s formula: 2 x 64 x (16 + 4) x 4 B = 10,240 B.
* A test that `vfused`'s dispatch label is `attention_label`'s text.
* The native GPU tests: `vfused` against `attention_cpu` on the time axis shape and on ragged shapes (a sequence that is not a multiple of 64 or of 4); and `vfused` equal to the current fused kernel bit for bit on the same inputs.  The doc says the claim is for the native test only, since Safari compiles with `MTLMathModeRelaxed`.
* `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passes on this Mac (Metal).

Do not commit on the branch yet.  Run `git add -N` on each new file, and write `git diff` to `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/candidate.patch`.  Check it with `git apply --check` in a clean worktree at HEAD.

Acceptance: the tests fail first, then pass, natively on Metal.

### Step 9: the gain rule for `vfused`

The rule of Step 7 of `plans/completed/gpu-speed-m4.md`, for an attention kernel:

1. Build the head page with the candidate.  Run one set of `e2e bench --query "bench=v64&bench_rounds=5&mm=<pins>&attn=fused"`, with the pins of Step 6.  Give `measure_attention` the figure of Step 5's form for each policy, `vfused` included, as calibration times it.  It keeps the attention-only figures too.
2. For each browser and page: `R` = the candidate's time for each chunk over the reference's, in Step 5's form: the figure on which calibration chooses (the parent's gain case 2).  The references are each policy that calibration chose at M5's base in that browser: `fused` and `freq` in Safari, `freq` in Chrome.  `S` is the largest spread of the figures in `R`.  The attention-only ratios are recorded too, as evidence.  If TD-101 is still open after Step 6, `R` against `freq` does not count.  Then a forced comparison decides against `freq`: `--query "mm=<pins>&attn=freq" --head-query "attn=vfused"`, with the head page as both pages; a gain is needed.
3. A gain in a browser: `R < 1` and `1 - R > S` against each reference, in both pages.  A loss: `R > 1 + S` in both pages.
4. `vfused` lands only with a gain in at least one browser.  Then each browser's comparison (Step 11) must show no loss, and each browser that chooses it must have its gain.
5. With no gain: `vfused` does not land.  Its figures go in Surprises, and its code to `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/vfused.patch`.  Skip Steps 10 to 13.

Acceptance: a table in Surprises of `R`, `S` and the verdict for each browser, page and reference.

### Step 10: the self-tests, then the commit of `vfused`

With the candidate page, before the commit (GPU-2): `cargo run -p xtask -- e2e selftest --browser <b> --weights /Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors --query attn=vfused`, in Safari and in Chrome.  Also with no query, with the kernels that each browser chooses.

Then commit `vfused` with its tests, `CALIBRATION`, its pinned hashes (move it from `NEW_KERNELS` to `PINNED_WGSL`), and the docs that name the kernels (`docs/ARCHITECTURE.md`, "Running it on the GPU"; the doc of `AttentionPolicy`).  Add the attention case to "A kernel lands only with a shown gain" in `docs/references/local-browser-testing.md`.

Acceptance: each run passes before the commit.  Progress names each run.

### Step 11: the comparison of `vfused`

Base = the commit before Step 10's commit.  `--runs 8` in each browser, since three policies can be near.  The outcomes, as Step 9 of `plans/completed/gpu-speed-m4.md` says: no change or a gain keeps `vfused`; a loss that holds is a defect, and the candidate is reverted if it is not fixed; twice undecided reverts it, unless the Decision log gives a reason.  A browser that chooses `vfused` must have its gain from Step 9.

Acceptance: the verdict of each browser, with `r` of each block and the kernels of each run, in Surprises.

### Step 12: the goal's check and the flip costs at the head

1. The goal's check of Step 3 at the head, in each browser, for each choosable policy of the three (Terms), from at least 12 head calibrations (Step 11's head runs, and more unforced runs if needed).
2. The cost of each forced flip between near choices (Terms), as Step 10 of `plans/completed/gpu-speed-m4.md` does.  Each run pins every other choice in its `--query`.  If a cost plus the drift (5.7 %) is over the tolerance (10 %), decide in the Decision log, as the parent says.

Acceptance: the check passes in each browser.  Each near flip's cost is in Surprises.

### Step 13: the baselines

Only for a browser whose calibration chose `vfused`: `e2e perf --runs 5 --update-baselines`, as Step 11 of `plans/completed/gpu-speed-m4.md` says.  A higher bound is refused; keep the old entry, and record it.

Acceptance: `xtask/perf-baselines.json` changes only for such a browser, with a lower bound.

### Step 14: the worker headroom of `memory-budget`

Step 5 changes calibration, which runs before the separation, so the free heap at the start can move.  By the parent's cases 1 to 4 of the headroom, as Step 12 of `plans/completed/gpu-speed-m4.md` says: 3 runs of `e2e memory-budget` in each browser at the head.  `roformer/` changes only if Step 5 changes `working_set`; then compute the smallest lost-cut rise again.  If each growth is 57 MB and the margin is in the band of the doc, `WORKER_HEADROOM_MB` stays.  Else follow cases 2 to 4.

Acceptance: `memory-budget` passes 3 times in each browser.  The growths and the margins are in Surprises.

### Step 15: the checks before review round 1

* `STEMYLA_PERF_WEIGHTS=/Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver cargo run -p xtask -- check --tier push` passes, with both perf gates run.  Progress gives both verdicts.  A skip line is not a pass (VERIFY-1).
* `cargo build -p xtask` passes.
* `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/parent-m5.diff` for the orchestrator, with `git apply --check` passing on HEAD.  It holds Surprises entries with the tables of Steps 3 to 7, 9, 11, 12 and 14, and Decision log lines for the cause, the fix, `FUSED_MARGIN` and the candidate.  It changes M6's case 11 if Step 5 changes what M6 must keep in step with `Pace`.
* Each change of `.claude/**` or `CLAUDE.md` that no step checkpoint carried, as a diff in `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/`.  For example, `.claude/rules/gpu.md` can name the attention case of the gain rule.

## Validation and acceptance

* TD-101's text is right (Step 1), and TD-101 is closed by the goal "Each calibration choice is not slower" (Step 6).
* `--head-query` forces a policy on the head runs only, and refuses `split`.  Its tests failed first (Step 2).
* The goal's check failed first on the page of `ab40862` in Safari (Step 3).  It passes at the head in each browser (Steps 6 and 12).
* The cause of the bias is in Surprises, by browser (Step 4).  Calibration's new bias and the spread of its ratio are in the limits of Step 5, item 6, in each browser.  The decision on `FUSED_MARGIN` is in the Decision log, if a browser had a wrong choice.
* The comparison of the fix shows no loss in either browser (Step 6).
* If `vfused` lands: its gain by Step 9's rule in each browser that chooses it, no loss in either browser by Step 11, the self-tests pass, and the flip costs are measured.  If not: the figures and the reason are in Surprises.
* `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passes on this Mac.
* The WGSL of each current kernel stays the same (`current_kernels_keep_their_wgsl`), and `NEW_KERNELS` is empty at the end.
* `memory-budget` passes 3 times in each browser (Step 14).
* `cargo run -p xtask -- check --tier push` passes, with both gates run.
* The review rounds (`/review-rounds b7f980f`) end by the stop rule of decision 0024.

## Idempotence and recovery

Each step is one commit or none, on the branch.  A session that stops reads Progress and Surprises, and goes on at the next open step.  Steps 3, 4, 6, 7, 9 and 11 to 14 measure.  Until Step 10's commit, the candidate is in `candidate.patch` in the scratchpad.  A browser run that a locked screen or a time limit stops runs again.  A comparison that a browser update splits runs again in full, and Surprises records both versions.  A revert of Step 5's or Step 10's commit gives back the old choice; `CALIBRATION` and `kernels_id` make each browser calibrate again.  `git worktree remove` undoes each base worktree.

## Progress

* [x] 2026-11-13: the plan written.
* [x] Plan review round 1 (2026-11-13): 2 blockers, 12 should-fix and 5 nits, each applied.
* [x] Plan review round 2 (2026-11-13): 1 blocker, 7 should-fix and 7 nits, each applied.
* [x] Step 1 (2026-11-13): TD-101's text, in its own commit.  `docs-check` passes.
* [x] Step 2 (2026-11-13): `--head-query`.  Each new test failed first on an assert:
  * `parses_scenario_and_options`: `left: Err("unknown option --head-query")`, `right: Ok((Some("attn=freq"), Some("mm=v64,v64")))`.
  * `the_head_query_sets_its_keys_on_the_query` (`side_query`, a stub): `left: ("", "")`, `right: ("mm=v64k8,v64k8&attn=fused", "mm=v64k8,v64k8&attn=freq")`.
  * `the_queries_of_a_comparison_never_split_the_frequency_axis` (`query_refusal`, a stub): `"--head-query" in ""`.
  * `each_side_of_a_comparison_runs_its_own_query`, with the two queries swapped: `left: "mm=v64k8,v64k8&attn=freq"`, `right: "mm=v64k8,v64k8&attn=fused"`.
  * Then `cargo test -p xtask` passes (802 unit tests).
* [x] Step 3 (2026-11-13): the check of the goal, in `docs/references/local-browser-testing.md`.  It fails first at `ab40862` in Safari: a loss of `freq` of 3 % or more.  Chrome gives a gain of `freq`, so no failure there.
* [x] Step 4 (2026-11-13): the cause of the bias, from forced and unforced runs at the head page (`e25402d`).  Safari: the linear products and the other dispatches of the time transformer.  Chrome: the waits between steps.  So Step 5 takes form (c).
* [x] Step 5 (2026-11-13 to 2026-11-14): form (c), `FUSED_MARGIN` 1.0 and 12 time-axis rounds, in one commit.  Each new test failed first on an assert, against the old behaviour:
  * `time_steps_of_a_full_chunk`, with `time_steps` giving no steps: `left: []`, `right: [1, 1, ...]` (62 ones).
  * `calibration_waits_after_each_step`, with `STEP_WAITS` set to `Covering`: `left: Covering`, `right: EachStep`.
  * `choice_keys_name_the_format` (it pins `CALIBRATION`, now 5), with `CALIBRATION` 4: `left: 4`, `right: 5`.
  * `the_run_and_calibration_share_one_budget` (a source test), on the worker's source at `253d775`: `s.max_elems.unwrap_or(roformer_gpu::net::BUDGET) in worker/src/web.rs`.
  * `calibration_frees_its_buffers` (`roformer-gpu/tests/memory.rs`) is a guard, not a test that fails first.  It passed with the old `calibrate` at `253d775` (with f32 and f16 weights), and passes with the new one.
  * `cargo test -p roformer-gpu --lib` (55) and `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` pass on the M2 Pro (Metal).
  * Item 6 at `FUSED_MARGIN` 1.1: the limits hold in both browsers (Surprises).  Item 7: Chrome had a wrong choice.  The maintainer chose option A (Decision log).
  * `fused_attention_margin` now asserts the margin 1.0.  With `FUSED_MARGIN` 1.1 it failed: `assertion failed: !time(1.0, 1.01)`.
  * `ATTENTION_ROUNDS` (12) sets the timed rounds of the time axis, 4 times the 3 of before.  Calibration takes about 24.5 s in Chrome and 23.9 s in Safari, against 15.0 s and 13.6 s at `253d775`: about 9.5 to 10.3 s more.
  * Item 6 again at 1.0, with 12 rounds: both limits hold in each browser, and item 7 has no wrong choice (Surprises).
  * Item 8, with the head page of the working tree on `1e1c2cc`: `e2e selftest --query selftest=stages` passes in Safari (`v64k8,v64`, `fused`, worst stage 69.4 dB) and in Chrome (`v64,v64`, `freq`, 69.4 dB).  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step5-selftest-<browser>/`.
  * Item 8: `e2e stored-stems` in Chrome gives the same six hashes at `1e1c2cc` and at the head, with `attn=fused` and with `attn=freq`.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step5-stored2-<base or head>-<policy>/`.  `stored-stems` took `attn` from `--query` only after a fix (Surprises).  Its test `stored_stems_takes_attn_from_the_query` failed first: `left: "split"`, `right: "fused"`.
* [x] Step 6 (2026-11-14): the comparison of the fix gives no change in Chrome and in Safari (`--runs 8`).  The head's 12 calibrations in each browser give one choosable policy: `freq` in Chrome and `fused` in Safari.  The check of Step 3 passes in each browser.  No gain, so no new Safari baseline.  TD-101 is closed.
* [x] Step 7 (2026-11-14): the predictor of C1.  `i64` over `v64` on `time scores` is an effect in both browsers: 1.31 to 1.35 in Chrome, and 0.84 in Safari, where vec4 reads are slower.  So C1 is built (Chrome), and Safari is predicted to lose.
* [x] Step 8 (2026-11-14): `vfused`, not committed: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/candidate.patch` (code only), which `git apply --check` accepts on a clean worktree at `1c232b6`.  `AttentionPolicy` is an enum of `split`, `fused`, `time`, `freq` and `vfused`; `FusedKernel` names the kernel of each axis; `Gpu::vfused_ok`; `decide` takes the faster fused kernel; `CALIBRATION` is 6.  Each new test failed first on an assert:
  * `every_kernel_fits_the_default_limits`, before `generated_sources` named `vfused`: `left: [..., "attention", "softmax.wgsl", ...]`, `right: [..., "attention", "vfused", "softmax.wgsl", ...]`.  Also `every_shader_file_is_checked` (the new template not in `GENERATED_FROM`), `current_kernels_keep_their_wgsl` (`vfused: no such source`) and `workgroup_memory_within_defaults` (`left: None`, `right: Some(10240)`).
  * `vfused_matches_cpu` (`Vec4 attention seq 1151 x4: index 0: got -0.025935719, want 0.0023692255`), `vfused_equals_the_tiled_kernel_bit_for_bit` (`2357248 outputs differ`) and `check_attention_accepts_all` (`vfused gives wrong results on this GPU (relative error 1.1e1)`), on the first kernel, whose threads raced (Surprises).
  * By mutation: `decide_takes_the_faster_fused_kernel`, with `decide` that never takes `vfused` (`left: Fused`, `right: VFused`); `attention_policy_names`, without `vfused` in `ALL` (`left: [..., "split"]`, `right: [..., "vfused"]`); `a_policy_is_usable_only_with_its_kernels`, with `vfused` usable on the tiled kernel's check alone (`left: [..., "vfused"]`, `right: [..., "freq"]`); `profile_labels_attention_by_its_shape`, with `vfused` not labelled by `attention_label` (`left: Some(1)`, `right: Some(2)`).
  * `the_worker_takes_a_policy_only_when_its_kernels_passed` (a source test), on the worker's source at `1c232b6`: `Some(p) if p.usable(...) => gpu.attention.set(p), in worker/src/web.rs`.  `choice_keys_name_the_format`, with `CALIBRATION` 5: `left: 5`, `right: 6`.
  * Then `cargo test -p roformer-gpu --lib` (58) and `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` pass on the M2 Pro (Metal), `vfused` equal to the tiled kernel bit for bit.
  * `e2e stored-stems` in Chrome gives the same hashes at `1c232b6` and with the candidate, with `attn=fused` and with `attn=freq`.  With `attn=vfused` it gives the hashes of `attn=fused`.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step8-stored-<side>-<policy>/`.
* [x] Step 9 (2026-11-14): the gain rule for `vfused`.  A gain in each browser, in both pages, against each reference: `R` 0.66 to 0.70, with `S` 2.3 % or less.  So `vfused` lands.
* [x] Step 10 (2026-11-14): with the candidate page, before the commit, `e2e selftest` passes in Safari and in Chrome, each with `--query attn=vfused` and with `--query selftest=stages` (the kernels that each browser chooses), worst stage 69.4 dB.  Safari chose `v64k8,v64` and `vfused`; Chrome `v64,v64` and `vfused`.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step10-selftest-<browser>-<attn or selftest>/`.  Then the commit of `vfused`, with `vfused` moved from `NEW_KERNELS` to `PINNED_WGSL` (`0xed9b_5bcb_7b74_65a1`).
* [x] Step 11 (2026-11-14): the comparison of `vfused` (`9d6d447`) against `d244d57`, `--runs 8`: a gain of 3 % or more in each browser, Chrome -22.4 % and Safari -20.3 % (the mean `r` of 4 blocks).  Each head run chose `vfused`.
* [x] Step 12 (2026-11-14): at the head, `vfused` is the one choosable policy in each browser (12 of 12 calibrations), and the check passes: a gain of 20 % to 23 % against `fused` and against `freq`.  No two choices are near, so no flip cost is measured.
* [x] Step 13 (2026-11-14): both browsers chose `vfused`, so both baselines are measured again with `e2e perf --runs 5 --update-baselines` at `9d6d447`.  Each bound is lower, so no `--raise`: Chrome 157 4.70 s to 3.59 s, Safari 27.2 7.38 s to 5.57 s.
* [x] Step 14 (2026-11-14): `memory-budget` passed 3 times in each browser at `50d3ff6`.  Each growth is 57 MB, so check 3's margin is 6.16 MB, and `WORKER_HEADROOM_MB` stays.  `roformer/` does not change.
* [x] Step 15 (2026-11-14): `check --tier push` passes at `0af5ef8`, with both perf gates run: Chrome 3.59 s (bound 3.95 s), Safari 5.58 s (bound 6.13 s).  `cargo build -p xtask` passes.  The parent's diff is `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/parent-m5.diff`, and the diff of `.claude/rules/gpu.md` is `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/claude-gpu-rules.diff`; `git apply --check` accepts each on HEAD.
* [x] Review round 1 fixes (2026-11-14, `eadebd9`).  Each new test failed first on an assert, against a stub or the old code, or is a guard that a mutant fails:
  * 1-1: `the_time_axis_is_timed_as_the_separation_runs_it` (`time_axis_round`, a stub: `left: []`) and `a_chunk_total_sums_each_call_times_its_count` (`per_chunk_total`, a stub: `assertion failed: close(per_chunk_total(&[(0.004, 744)]), 2.976)`).  `time_axis_per_chunk` and `attention_per_chunk` use both.  The guard `the_time_axis_adds_up_one_wait_for_each_step` (a fake clock) fails with `Covering` in the glue (`split: 2.4800000000000004, want 14.88`), with `time_rec` giving 1.0 (`split: 744`), and with the totals of the usable kernels left out (`left: 0`).
  * 1-59: `calibration_records_the_step_of_the_separation` (profile labels of `TimeStep` against `Job::run`, and against fixed lists): with `TimeStep::record` recording nothing, `left: []`.
  * 1-3: `check_fused` runs (133, 2).  `the_start_up_check_rejects_a_broken_vfused`, with the seam `plant_attention_source` a stub: `no rescale: ""`.
  * 1-50: `the_start_up_check_keeps_a_usable_policy`, with the seam a stub: `left: VFused`, `right: Split`.
  * 1-2: `a_run_must_run_the_policy_that_its_query_forces` (`forced_attn_problem`, a stub: an empty reason) and `a_comparison_fails_a_run_of_the_wrong_policy` (`run_comparison` with each side's query: `Ok(Comparison { ... })`).
  * 1-5, 1-16: `stored_stems_checks_the_workers_policy` (`stored_stems_attention_problem`, a stub: `""`); the guard `the_stored_stems_script_takes_its_attn` fails on the old script line (`a fixed attn in the script`).
  * 1-6: `a_choice_is_kept_only_at_the_default_budget` (`keep_choice`, a stub: `4194304`); `the_run_and_calibration_share_one_budget` now also needs `keep_choice` in the worker (`!roformer_gpu::bench::keep_choice(budget) in worker/src/web.rs`).
  * Each confirmed mutant, applied by hand in a scratch worktree with its own target folder, fails its test: 1-21, 1-31, 1-38, 1-43, 1-70, 1-77 (at their new places in `time_axis_round` and `per_chunk_total`), 1-50, 1-52, 1-58 (by `per_chunk_total`), 1-59, and the pre-existing 1-25, 1-40, 1-44 and 1-28 (`time_rec`).  The case (37, 2) in `check_fused` fails `the_start_up_check_rejects_a_broken_vfused` too.  Results: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/r1-mutant-results.tsv`, `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/r1-net-results.tsv`.
  * Of the 28 network mutants, the test of 1-59 kills 7 (1-17, 1-20, 1-26, 1-32, 1-46, 1-48, 1-71); 21 stay, in TD-8.  The calibration mutants that no test kills are in TD-102.
  * With the start-up check on (133, 2), `e2e selftest --query selftest=stages` passes in Safari and in Chrome, each with both fused kernels checked OK and `vfused` chosen.  `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release`, `cargo test -p xtask`, `cargo build -p xtask` and `check --tier commit` pass.
  * Docs: 1-4 (about 33 s), 1-7, 1-8, 1-9, 1-10, 1-11, 1-12, 1-13 and 1-14 (WGSL §8.5.1.3; `vfused`'s pinned hash changes with its header comment, now `0x672c_1875_5e66_f53f`).  1-14 and 1-15 change `.claude/rules/gpu.md` by `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/r1-gpu-rules.diff`, and the parent by `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/parent-r1.diff`.
* [x] Review round 2 fixes (2026-11-14).  Each new test failed first on an assert, against a stub, or is a guard that the named mutant fails (applied by hand in a scratch worktree with its own target folder; `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/r2-mutant-results.tsv`):
  * 2-1: `vfused`'s dispatches have the label `attention seq <S> x<N> vfused`; the tiled kernel's stay as they were, and `profile_groups` counts both as attention.  `calibration_records_the_step_of_the_separation` failed first on the old label (`left: [("attention seq 9 x28", 1), ...]` for `VFused`), as did `profile_labels_attention_by_its_shape` (`left: Some(2)`, `right: Some(1)`) and `each_profile_line_goes_to_its_group` (`left: 15.0`, `right: 8207.0`).  Mutants A (`Job::run` runs the tiled kernel for `vfused`) and B (`TimeStep::record` does) fail it: `left: [("attention seq 9 x28", 1), ...]`.  `TimeAxis::of_policies` maps each timing to its policy; `each_timing_goes_to_its_policy` failed first on a stub (`left: [[], [], []]`), and mutant C fails it (`left: [[1.0], [3.0], [2.0]]`).
  * 2-2: `auto` asks for no policy in `stored_stems_attention_problem`; without the case, `stored_stems_checks_the_workers_policy` fails (`left: Some("the worker runs attention vfused, not the asked auto")`).
  * 2-3: the policy check is in `stored_stems_checks`; `the_stored_stems_checks_pass_a_good_reply_and_fail_wrong_ones` failed first (`left: 15`, `right: 16`), and fails with the check replaced by `None` (`/replies/0/text: []`).
  * 2-4: `CompareSites::queries`, which `perf_compare` gives `run_comparison`; `each_side_of_a_comparison_runs_its_own_query` failed first on a stub (`left: ["", ""]`).
  * 2-9: `choices_to_store` decides whether and what to store; `a_choice_is_stored_only_at_the_default_budget` failed first on a stub (`left: Some("amd|a|5\ti64;split\n")`), and fails with `keep_choice` left out.  `the_run_and_calibration_share_one_budget` fails on the worker of `eadebd9` (`roformer_gpu::bench::choices_to_store( in worker/src/web.rs`).
  * 2-10: the fake-clock test expects each fused kernel's rounds only when it passed its check.
  * Docs: 2-5 (TD-8 names the 5 `timestamps` tests that run on the M2 Pro; TD-8 and TD-102 say which coverage needs `timestamps` in `STEMYLA_REQUIRE`), 2-6 (Decision log), 2-7 (the two new failures in the reference, `README.md` and `xtask/src/e2e.rs`), 2-8 (ARCHITECTURE.md).

## Surprises and discoveries

* 2026-11-13, Step 3: the check at the page of `ab40862`, base `attn=fused`, head `attn=freq`, the same page on both sides.  Each figure is a chunk lower quartile, in seconds, in the order B H H B.

  | Browser, pins | Block | Runs | `r` | Class |
  | - | - | - | - | - |
  | Safari 27.2, `v64k8,v64k8` | 1 | 7.22, 7.45, 7.45, 7.22 | +3.19 % | loss |
  | | 2 | 7.21, 7.28, 7.28, 6.98 | +2.61 % | loss, spoiled (3.30 %) |
  | | 3 | 7.00, 7.28, 7.28, 6.99 | +4.07 % | loss |
  | Chrome 157, `v64,v64` | 1 | 4.68, 4.62, 4.63, 4.68 | -1.18 % | gain |
  | | 2 | 4.68, 4.62, 4.62, 4.68 | -1.28 % | gain |

  * Safari: the verdict is a loss, 3 % or more.  `freq` is choosable there (M4), so the check fails, as the goal's case 2 needs.  Whole times: `fused` 77.0 to 79.7 s, `freq` 80.3 to 86.7 s.  Chunk medians: `fused` 6.99 to 7.23 s, `freq` 7.29 to 7.46 s.  Log: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step3-safari-1/output.log`.
  * Chrome: the verdict is a gain of `freq` (2 blocks), so the check passes.  Whole times: `fused` 51.9 to 52.9 s, `freq` 51.3 to 52.4 s.  Chunk medians: `fused` 4.69 to 4.71 s, `freq` 4.62 to 4.63 s.  `freq` is about 0.06 s faster for each chunk, not 0.10 s as M2 gave.  Log: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step3-chrome-1/output.log`.
  * Safari's two `fused` runs of block 2 were 7.21 and 6.98 s.  From then on, both policies ran about 0.22 to 0.23 s faster for each chunk.  The cause is not known; the GPU clock is a guess.
* 2026-11-13, Step 4: where the separation's cost of `freq` goes.  The page of `e25402d`, with the pins of Step 3.  In each browser, runs 1 to 4 have `profile=1` and runs 5 to 8 have none, each in the order `fused`, `freq`, `freq`, `fused`.  Runs 9 and later are unforced.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step4-<browser>-<n>/`, and the tables in `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step4-safari-table.txt` and `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step4-chrome-table.txt`.

  | Figure, seconds for each chunk | Safari 27.2 | Chrome 157 |
  | - | - | - |
  | Forced `fused` and `freq`, chunk lower quartiles (runs 5 to 8) | 6.99, 6.96; 7.28, 7.28 | 4.69, 4.69; 4.62, 4.62 |
  | `D`, the separation's cost of `freq` | +0.305 | -0.070 |
  | The noise of `D` | 0.030 | 0.00 (under the 0.01 of the logs) |
  | `D` over the forced `fused` | 4.37 % | 1.49 % |
  | Calibration's split, fused (unforced runs with the pins) | 3.074, 3.286 (5 runs) | 1.825, 2.183 (4 runs) |
  | Calibration's (split - fused) | -0.212 | -0.357 |
  | The bias: calibration's (split - fused) minus `D` | -0.517 | -0.287 |
  | Profile, time-axis attention, `freq`; `fused` | 3.062; 3.229 | 1.817; 2.278 |
  | Part: time-axis attention, `freq` - `fused` | -0.166 | -0.461 |
  | Part: linear products | +0.287 | +0.066 |
  | Part: other (norms, rope, gates, transposes) | +0.141 | +0.013 |
  | Part: frequency-axis attention | -0.002 | -0.003 |
  | Part: gaps, with the profile (an upper bound) | +0.240 | +0.437 |
  | Part: gaps, by difference (`D` minus the other parts) | +0.045 | +0.315 |
  | Calibration's attention against the profile's: (split - fused) minus the attention part | -0.046 | +0.104 |

  * The bias is the sum of the parts that calibration does not time, with a minus sign, and the error of its attention timing.  Safari: -0.046 - 0.287 - 0.141 + 0.002 - 0.045 = -0.517.  Chrome: +0.104 - 0.066 - 0.013 + 0.003 - 0.315 = -0.287.
  * Safari: the linear products (0.287 s, 56 % of the bias) and the other dispatches (0.141 s, 27 %) of the time transformer are the cause.  They run at M 1,151 with `freq` and at M 4,604 with `fused`, in 744 steps against 192.  Each is over the noise of `D`.  The gaps (by difference) and the error of the attention timing are each about 0.045 s, a little over the noise.  The prediction said the linear products; the other dispatches are a surprise.
  * Chrome: the gaps between steps are the cause: about 0.315 s by difference, 110 % of the bias, as predicted.  The linear products add 0.066 s.  Calibration's fused attention is 0.095 s lower than the profile's, which makes the bias smaller.
  * The gaps by difference take each group's part without the profile to be as with it.  A profile adds time to each step, so the profile's gaps are an upper bound.
  * Calibration's attention-only times match the profile's well: split 3.074 against 3.062 in Safari, 1.825 against 1.817 in Chrome.  So the attention timing is not the cause; the parts that it leaves out are.
  * Safari's calibration chose `fused` in each of 8 unforced runs today (ratio 1.061 to 1.081), and `v64k8,v64k8` in 5 of 8.  M4 had split in about 10 of 22.  Chrome chose `v64,v64` and `freq` in 4 of 4 (ratio 1.186 to 1.203).
* 2026-11-14, Step 5, item 6: the accuracy of form (c), from unforced runs of the head page (the working tree on `253d775`).  `D` and its noise are from Step 4.  The ratio is calibration's fused time over its split time.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step5-<browser>-<n>/`; tables: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step5-safari-calib.txt` and `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step5-chrome-calib.txt`.

  | Figure | Safari 27.2 | Chrome 157 |
  | - | - | - |
  | Runs with the pins, of all runs | 13 of 16 (`v64k8,v64k8`) | 13 of 14 (`v64,v64`) |
  | Policy chosen | `fused` in 13 of 13 | `fused` in 13 of 13 |
  | Calibration's split, fused (ranges, seconds for each chunk) | 4.96 to 5.06; 4.68 to 4.78 | 3.05 to 3.57; 3.11 to 3.45 |
  | Mean (split - fused) | +0.292 | -0.062 |
  | `D` | +0.305 | -0.070 |
  | The new bias, and its limit | -0.013, under 0.152: holds | +0.008, under 0.035: holds |
  | The old bias (Step 4) | -0.517 | -0.287 |
  | Ratio: mean, standard deviation | 0.9419, 0.0079 | 1.0203, 0.0200 |
  | Mean plus or minus 2 standard deviations | 0.926 to 0.958 | 0.980 to 1.060 |
  | The spread limit at `FUSED_MARGIN` 1.1 | holds | holds |
  | The spread limit at 1.0 | holds | fails: 0.020 from 1.0, under 2 SD (0.040) |

  * Form (c) removes the bias in both browsers: to 2.5 % of the old bias in Safari, and 3 % in Chrome.
  * Calibration with form (c) takes about 15.9 s in Chrome, against 15.0 s before (run 1).
  * Chrome's run 8 timed both policies about 0.4 s higher (split 3.57, fused 3.45), so its ratio was 0.966.  Without it, the standard deviation is about 0.013.
  * Item 7: Safari's one choosable policy is `fused`, the faster one: no wrong choice.  Chrome's one choosable policy is `fused`, which is 1.49 % slower than `freq` by Step 4's forced runs, over `T`: a wrong choice.  `freq` is not choosable in Chrome: 1.1 is outside 0.980 to 1.060.
* 2026-11-14, Step 5, item 6 again, at `FUSED_MARGIN` 1.0 with `ATTENTION_ROUNDS` 12 (the maintainer's option A).  The head page of the working tree on `1e1c2cc`.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step5b-<browser>-<n>/`; tables: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step5b-safari-calib.txt` and `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step5b-chrome-calib.txt`.

  | Figure | Safari 27.2 | Chrome 157 |
  | - | - | - |
  | Runs with the pins, of all runs | 13 of 21 (`v64k8,v64k8`) | 13 of 14 (`v64,v64`) |
  | Policy chosen | `fused` in 13 of 13 | `freq` in 13 of 13 |
  | Mean (split - fused) | +0.295 | -0.064 |
  | The new bias, and its limit | -0.010, under 0.152: holds | +0.006, under 0.035: holds |
  | Ratio: mean, standard deviation | 0.9412, 0.0052 | 1.0208, 0.0032 |
  | Mean plus or minus 2 standard deviations | 0.931 to 0.952 | 1.015 to 1.027 |
  | The spread limit at 1.0 | holds | holds |
  | Chunk lower quartiles of the runs | 6.96 to 7.00 | 4.62 to 4.64 |

  * The 12 rounds cut Chrome's standard deviation from 0.020 to 0.0032, and Safari's from 0.0079 to 0.0052.
  * Item 7: Safari's one choosable policy is `fused`, the faster one.  Chrome's one choosable policy is `freq`, the faster one.  No browser has a wrong choice.
  * Safari chose `v64k8,v64` for the small products in 8 of 21 runs.  The two kernels time within 1 % there (1.16 to 1.18 s), so it is noise, not a defect.
* 2026-11-14, Step 6: the same-session comparison of the fix.  Base: the page of `1e1c2cc`, whose `app/` is that of `a058eb6`, the commit before Step 5's commit.  Head: `745a62b`.  `--runs 8`, unforced.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step6-<browser>-1/output.log`.

  | Browser | Block `r` | Kernels, base | Kernels, head | Verdict |
  | - | - | - | - | - |
  | Chrome 157 | -0.11 %, -0.11 %, -0.22 %, +0.00 % | `v64,v64` x5, `v64,v64k8` x3; `freq` x8 | `v64,v64`, `freq` x8 | no change (mean `r` of 4 blocks) |
  | Safari 27.2 | -0.14 %, -0.07 %, -0.07 %, -0.07 % | `v64k8,v64` x4, `v64k8,v64k8` x4; `fused` x8 | `v64k8,v64k8` x5, `v64k8,v64` x3; `fused` x8 | no change (mean `r` of 4 blocks) |

  * The base already chose the faster policy in each of its runs today: `freq` in Chrome, and `fused` in Safari.  So the fix gives no gain here.  At the base, Safari chose `freq` in about 10 of 22 calibrations of M4, and in none of 8 + 8 of M5.
  * Chunk lower quartiles: Chrome 4.62 to 4.63 s on both sides; Safari 6.98 to 7.01 s on both sides.
* 2026-11-14, Step 6, item 2: the head's calibrations, from the 8 head runs of item 1 and 4 more unforced runs in each browser (`/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step6free-<browser>-<n>/`).

  | Browser | N | Policies | Matmul policies | Ratio: mean, SD | Mean plus or minus 2 SD | Choosable |
  | - | - | - | - | - | - | - |
  | Chrome 157 | 12 | `freq` x12 | `v64,v64` x9, `v64,v64k8` x3 | 1.0215, 0.0038 | 1.014 to 1.029 | `freq` |
  | Safari 27.2 | 12 | `fused` x12 | `v64k8,v64k8` x8, `v64k8,v64` x4 | 0.9418, 0.0034 | 0.935 to 0.949 | `fused` |

* 2026-11-14, Step 6, item 3: the check at the head page (`745a62b`), the same page on both sides, `--runs 4`.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step6check-<browser>-1/output.log`.

  | Browser, pins | Base, head | Block `r` | Verdict | The check |
  | - | - | - | - | - |
  | Chrome 157, `v64,v64` | `fused`, `freq` | -1.39 %, -1.39 % | gain of `freq` | passes: `freq` is not slower |
  | Safari 27.2, `v64k8,v64k8` | `freq`, `fused` | -3.98 %, -4.05 % | gain of `fused`, 3 % or more | passes: `fused` is not slower |

* 2026-11-14, Step 7: the predictor of C1.  One set of `e2e bench --query "bench=i64,v64&bench_rounds=5&mm=<pins>&attn=fused"` at `040dbea`, in the order Safari, Chrome, Safari, Chrome; pins `v64k8,v64k8` in Safari and `v64,v64` in Chrome.  The figure is `i64`'s time over `v64`'s (the rate of `v64` over the rate of `i64`), with the larger spread of the two.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step7-<browser>-<n>/`; table: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step7-table.txt`.

  | Page | `time scores` (K 64) | `time qkv` (K 256), f32 weights | `time qkv`, f16 weights |
  | - | - | - | - |
  | Safari 1 | 0.840 (1.2 %): effect, `v64` slower | 1.252 (1.5 %): effect | 1.265 (2.8 %): effect |
  | Chrome 2 | 1.350 (4.2 %): effect | 1.450 (1.0 %): effect | 1.479 (1.6 %): effect |
  | Safari 3 | 0.835 (1.4 %): effect, `v64` slower | 1.257 (1.3 %): effect | 1.260 (1.6 %): effect |
  | Chrome 4 | 1.312 (4.5 %): effect | 1.452 (1.1 %): effect | 1.492 (2.1 %): effect |

  * The verdict: C1 is built, by the rule of Step 7: `i64` over `v64` on `time scores` is an effect in Chrome.
  * In Safari, the effect on `time scores` goes the other way: `v64` takes 1.19 times `i64`'s time at K 64, though it takes 0.80 times at K 256.  So vec4 reads are predicted to help C1 in Chrome and to slow it in Safari.  C1's loops have K 16 for each slice, nearer K 64 than K 256.
  * The attention-only figures of `measure_attention` (time axis, seconds for each chunk): Safari split 3.06 to 3.09, fused 3.24 to 3.30; Chrome split 1.82 to 1.84, fused 2.17 to 2.19.

* 2026-11-14, Step 14: `memory-budget` at `50d3ff6`, 3 runs in Chrome 157 and 3 in Safari 27.2.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step14-<browser>-<n>/output.log`.

  | Browser | Runs passed | Worker growth in each separation | Check 3: budget, margin | Tab total (page, worker, decoder, GPU live) | Kernels |
  | - | - | - | - | - | - |
  | Chrome 157 | 3 of 3 | 57 MB | 63.16 MB, 6.16 MB | 756 MB (33, 87, 6, 630) | `v64,v64` or `v64,v64k8`, `vfused` |
  | Safari 27.2 | 3 of 3 | 57 MB | 63.16 MB, 6.16 MB | 758 MB (35, 87, 6, 630) | `v64k8,v64k8` or `v64k8,v64`, `vfused` |

  * Each growth is as at M1 and M4, so the parent's case 1 holds and `WORKER_HEADROOM_MB` stays.  Calibration's new peak (about 113 MB, freed before the separation) does not move the worker's heap.
  * Chrome's tab total fell from 799 MB with `freq` to 756 MB with `vfused`: `vfused` holds no scores buffer, as `fused` does not.  Both are under `TAB_BUDGET_MB` (943 MB, MEM-4).

* 2026-11-14, Step 13: the baselines, from 5 runs in each browser at `9d6d447`.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step13-<browser>/output.log`.

  | Browser | Chunk lower quartiles | Kernels | Baseline, bound | Before |
  | - | - | - | - | - |
  | Chrome 157 | 3.58, 3.59, 3.59, 3.60, 3.61 | `v64,v64` x4, `v64,v64k8` x1; `vfused` x5 | 3.59 s, 3.95 s | 4.70 s, 5.17 s |
  | Safari 27.2 | 5.58, 5.55, 5.57, 5.57, 5.58 | `v64k8,v64k8` x4, `v64k8,v64` x1; `vfused` x5 | 5.57 s, 6.13 s | 7.38 s, 8.12 s |

* 2026-11-14, Step 12: the head's calibrations, from the 8 head runs of Step 11 and 4 more unforced runs in each browser (`/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step12free-<browser>-<n>/`).  The ratios are calibration's `vfused` time over its `fused` time, and over its split time.

  | Browser | N | Policies | Matmul policies | `vfused` / `fused`: mean, SD | `vfused` / split: mean, SD | Choosable |
  | - | - | - | - | - | - | - |
  | Chrome 157 | 12 | `vfused` x12 | `v64,v64` x12 | 0.6492, 0.0019 | 0.6635, 0.0022 | `vfused` |
  | Safari 27.2 | 12 | `vfused` x12 | `v64k8,v64k8` x10, `v64k8,v64` x2 | 0.6983, 0.0028 | 0.6612, 0.0034 | `vfused` |

  * Calibration's times for each chunk: Chrome split 3.05 to 3.07 s, `fused` 3.12 to 3.14 s, `vfused` 2.03 to 2.04 s; Safari split 4.93 to 5.17 s, `fused` 4.71 to 4.95 s, `vfused` 3.28 to 3.45 s.
  * No two choices are near by Terms: the two fused kernels are 30 % to 35 % apart, and the faster one over split is 0.66, not 0.99 to 1.21.  So Step 12 measures no flip cost.  The forced runs of the check give the cost of each flip from `vfused` anyway: about 30 % in Chrome and 25 % to 31 % in Safari.

* 2026-11-14, Step 12: the check at the head page (`9d6d447`), `vfused` on the head, the same page on both sides, `--runs 4`.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step12check-<browser>-<policy>/output.log`.

  | Browser, pins | Base | Block `r` | Verdict | The check |
  | - | - | - | - | - |
  | Chrome 157, `v64,v64` | `fused` | -23.37 %, -23.37 % | gain, 3 % or more | passes |
  | Chrome 157, `v64,v64` | `freq` | -22.40 %, -22.29 % | gain, 3 % or more | passes |
  | Safari 27.2, `v64k8,v64k8` | `fused` | -20.14 %, -20.23 % | gain, 3 % or more | passes |
  | Safari 27.2, `v64k8,v64k8` | `freq` | -23.47 %, -23.42 % | gain, 3 % or more | passes |

* 2026-11-14, Step 11: the same-session comparison of `vfused`.  Base: the page of `1c232b6`, whose `app/` is that of `d244d57`, the commit before Step 10's commit.  Head: `9d6d447`.  `--runs 8`, unforced.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step11-<browser>-1/output.log`.

  | Browser | Block `r` | Kernels, base | Kernels, head | Chunk lower quartiles, base; head | Verdict |
  | - | - | - | - | - | - |
  | Chrome 157 | -22.55 %, -22.38 %, -22.38 %, -22.38 % | `v64,v64k8` x4, `v64,v64` x4; `freq` x8 | `v64,v64`, `vfused` x8 | 4.62 to 4.64; 3.59 | gain, 3 % or more |
  | Safari 27.2 | -20.26 %, -20.31 %, -20.46 %, -20.19 % | `v64k8,v64k8` x6, `v64k8,v64` x2; `fused` x8 | `v64k8,v64k8` x7, `v64k8,v64` x1; `vfused` x8 | 6.98 to 7.00; 5.54 to 5.58 | gain, 3 % or more |

  * Whole times: Chrome 51.1 to 51.2 s at the base, 39.7 to 39.8 s at the head; Safari 77.0 to 77.7 s, 61.1 to 61.6 s.
  * Each browser that chooses `vfused` has its gain of Step 9.

* 2026-11-14, Step 9: the gain rule for `vfused`.  One set of `e2e bench --query "bench=v64&bench_rounds=5&mm=<pins>&attn=fused"` with the candidate page (the working tree on `d244d57`), in the order Safari, Chrome, Safari, Chrome; pins as in Step 7.  `R` is `vfused`'s time for each chunk over the reference's, in Step 5's form (whole time-transformer steps with a wait after each, as calibration times them).  `S` is the largest spread of the figures in `R`.  Logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m5/runs/step9-<browser>-<n>/`.

  | Page | Whole steps, seconds for each chunk: split, `fused`, `vfused` | `R` against `fused` | `R` against `freq` (split) | `S` | Verdict |
  | - | - | - | - | - | - |
  | Safari 1 | 4.96, 4.72, 3.29 | 0.697 | 0.663 | 2.3 % | gain |
  | Chrome 2 | 3.05, 3.14, 2.04 | (0.650) | 0.669 | 1.7 % | gain |
  | Safari 3 | 4.96, 4.72, 3.28 | 0.695 | 0.661 | 2.3 % | gain |
  | Chrome 4 | 3.08, 3.11, 2.02 | (0.650) | 0.656 | 1.9 % | gain |

  * The references are each policy that calibration chose at M5's base: `fused` and `freq` in Safari, `freq` in Chrome.  The figure against `fused` in Chrome is evidence only.
  * The attention-only figures (`measure_attention`), seconds for each chunk: Safari `fused` 3.30 and 3.29, `vfused` 1.88 and 1.84 (0.57 and 0.56); Chrome `fused` 2.17 and 2.17, `vfused` 1.07 and 1.07 (0.49 and 0.49).
  * Step 7 predicted a loss in Safari, from `v64` against `i64` on `time scores`.  The predictor was wrong for this kernel: `vfused` takes 0.56 times the tiled kernel's attention time in Safari.  `vfused` also has 10 KiB of workgroup memory, not 12.25 KiB, and Safari sets it to zero at each dispatch (M3, M4).  M5 did not split the gain between the two causes.
  * The predicted upper bound of C1 was 20 % to 37 % of the attention time in Safari and 31 % to 33 % in Chrome.  The gain is 43 % to 44 % in Safari and 51 % in Chrome: more than the bound.  So the predictor's premise does not hold for this kernel.  M5 did not measure why.

* 2026-11-14, review round 1 (finding 1-4): calibration takes about 33 s on the M2 Pro at `eeb2697` (32.4 to 32.9 s, `step12free-<browser>-1`), against about 15 s before M5 (13.9 to 15.0 s, Step 4).  The 12 rounds of Step 5 added about 10 s, and `vfused`'s timings about 8.5 s more, over the 8 to 10 s that the maintainer accepted.  The orchestrator asks the maintainer whether to cut it; the timing does not change in round 1.

* 2026-11-14, Step 8: the first `vfused` gave wrong results on Metal (relative error 11).  Its P, its row maxima and its row sums were written as scalars into the components of a vec4 of `buf`, 4 threads to each vec4, as the plan's layout of P (`[64 queries][16 keys]`) needs.  With those writes serialised by barriers, each test passed, bit for bit too.  So threads that write different components of one vec4 in workgroup memory race on Metal, through naga's MSL.  WGSL allows it: "A write access to a component of a vector may access all of the memory locations associated with that vector" ([§8.5.1.3](https://www.w3.org/TR/WGSL/#component-reference-from-vector-memory-view)).  So the race is a defect of the kernel on any GPU, not of Metal (review round 1, finding 1-14).  The kernel now writes each vec4 whole, from one thread (Decision log).

* 2026-11-14, Step 5, item 8: `e2e stored-stems` did not read `--query`.  Its page script set `mm: 's64', attn: 'split'`, so a run with `--query attn=fused` ran split attention, and gave the same hashes as `attn=freq`.  Now it takes `attn` from `--query` (`stored_stems_attn`), and logs the policy.  With it, `fused` and `freq` give different hashes, and each is the same at the base and the head.

## Decision log

* 2026-11-13, agent (team leader): Part 1 (the calibration fix) comes before Part 2 (the candidate).  Why: calibration chooses the candidate by the same timing, and a ratio against `freq` does not count until TD-101 is fixed.
* 2026-11-13, agent (team leader): M5 builds one candidate, C1 (vec4 reads).  Why: C1 alone has a predicted gain over the spread of a `?bench` figure (`i64` over `v64`: 1.25 to 1.58 in Safari, 1.44 to 1.50 in Chrome).  C2 to C6 predict 2 % or less, a loss, or do not fit GPU-1.
* 2026-11-13, agent (team leader): the reloads of Q are not measured.  Why: the parent measures them only before M5 removes them, and a Q block staged once does not fit GPU-1 (C3).
* 2026-11-13, agent (team leader), plan review round 1: Step 5 takes the simplest of three forms that Step 4's table supports, with the proof in the Decision log.  Each form waits after each step, as `Pace::run` does, for M6's case 11.  Why: PLANS.md asks for the simplest fix first.
* 2026-11-13, agent (team leader), plan review round 1: "choosable" comes from at least 12 calibrations, not from the 10 % near band.  Why: Chrome's ratio of 1.19 today is in the near band, so `fused` would be choosable there, and the check would fail whatever M5 does.
* 2026-11-13, agent (team leader), plan review round 1: the decision on `FUSED_MARGIN` comes before Step 5's commit, at a checkpoint for the orchestrator.  Why: a true timing is predicted to make Chrome choose `fused`, 2.1 % slower; the margin's reason is the 42 MB scores buffer of `freq`, so a change of it changes memory and choices on GPUs that this Mac cannot measure.
* 2026-11-13, agent (team leader), plan review round 2: the decision on `FUSED_MARGIN` uses the rule of Step 6's check ("choosable", from 12 calibrations with the pins), and Step 5 limits the spread of calibration's ratio.  Why: a mean under the margin with a wide spread still lets the slower policy be chosen.  Option B does not land the fix, since a loss in Step 6's comparison is a defect by the parent.

* 2026-11-13, agent (implementer), Step 4: the cause of the bias is the parts of a time-transformer step that calibration does not time.  In Safari, the linear products (0.287 s) and the other dispatches (0.141 s) at M 1,151 in 744 steps against M 4,604 in 192.  In Chrome, the waits between steps (about 0.315 s).  Why: Step 4's table; each part is over the noise of `D`.
* 2026-11-13, agent (implementer), Step 5: form (c), a whole time-transformer step of each policy through the function that `Job::run` uses.  Why, by Step 4's table and item 6's limit (half of `|D|`: 0.153 s in Safari, 0.035 s in Chrome): form (a) covers the gaps only, and leaves about -0.47 s in Safari.  Form (b) adds the linear products, and leaves about -0.517 + 0.045 + 0.287 = -0.185 s in Safari, over 0.153 s.  The other dispatches (0.141 s) are the rest, and only form (c) times them.
* 2026-11-14, the maintainer, Step 5 item 7: option A.  `FUSED_MARGIN` goes to 1.0, with time-axis timings about 4 times longer, so that the choice is stable.  It is a product change that the maintainer accepted: a GPU where split is faster now splits the time axis, and holds about 42 MB more GPU memory (the parent's Decision log, `1e1c2cc`).
* 2026-11-14, agent (implementer), Step 8: `vfused` does not use the plan's layout of P and V.  P is `[16 keys][16 ty]`, each a vec4 of the queries `ty + 16i`, and V is `[16 keys][16 tx]`, each a vec4 of the dimensions `tx + 16j`.  The row maxima and sums are a vec4 of each thread's 4 rows at `[ty][tx]`.  Why: the plan's layout needs 4 threads to write the components of one vec4, which WGSL lets race, and which raced on Metal (Surprises).  The new layout keeps the thread's ownership, the sum orders (so the bits) and 10 KiB, and has the same reads for each multiply-add: 2 vec4s for 16.
* 2026-11-14, agent (implementer), Step 5: the longer timing is 12 rounds of `ATTENTION_ROUND` (0.3 s), not longer rounds.  Why: more rounds give a median that one slow round (Chrome's run 8 of the first item 6) does not move, and they interleave the two policies more often.
* 2026-11-13, agent (implementer), Step 5: calibration times the time axis at the budget of the run, as the plan says.  A run with `?max_elems=` thus keeps a choice that it timed at its own budget.  Why: `?max_elems=` is a tool for tests, and the stored key does not name a budget today.
* 2026-11-14, agent (implementer), review round 1 (finding 1-6), replaces the line above on a stored choice: a choice calibrated at a budget other than `net::BUDGET` is used for that run only, and not stored (`choices_to_store`).  A run with `?max_elems=` uses a stored choice of the default budget if one exists, else it calibrates at its own budget.  Why: the stored key names no budget, so a stored choice must be the default budget's.
* 2026-11-14, maintainer: calibration takes about 33 s on the M2 Pro at M5's head (about 15 s before M5): the 12 rounds of the time axis and the third policy, `vfused`.  The maintainer accepts it, and no cut is needed (review round 1, finding 1-4).
* 2026-11-14, agent (orchestrator): the pinned hash of `vfused` changes in the round 1 fixes (a comment of its header, finding 1-14).  `vfused` is new in this PR, so no current kernel of `b7f980f` changes.

## Outcomes and retrospective

Not yet.
