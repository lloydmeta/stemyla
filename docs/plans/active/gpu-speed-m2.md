# GPU speed, M2: the perf scenario and its gate

## Purpose

This is the milestone plan for M2 of `plans/active/gpu-speed.md`.  The parent's M2 section stays the criteria.

M3 to M6 change the kernels and the worker to make separation faster.  Each change needs a measure, and a later change must not make separation slower without a failure.  M2 adds a scenario, `xtask e2e perf`, that separates the 1-minute synthetic song and writes a result.  M2 also adds a gate: it compares the result with a committed baseline for the browser and the adapter.  `check --tier push` runs the gate on this M2 Pro when the model is named.

Readers: the implementer, the review agents of M2's rounds, the orchestrator, and the team leaders of M3 to M6.

## Context and orientation

The state at the start, on 2026-11-12, at commit `91923ae` on branch `gpu-speed/2026-11-12`.  The review base of M2 is `91923ae`.

### The decisions that M2 follows

* The maintainer, 2026-11-12: the perf gate runs on this M2 Pro, in Safari and Chrome, only.  Its tolerances must not be "too tight to be annoying".  Never wait for a run on the Radeon or the iPhone.
* The parent plan: the tolerance comes from at least 5 runs in each browser, and is never under 10 %.  A faster run lowers a baseline only through an explicit update.  The baselines file is committed and names no machine path.
* The orchestrator, 2026-11-12:
  * The context baselines (Chrome 4.7 s, Safari 7.1 s for each chunk) are from before M1, which changed the worker's CPU path.  Measure the baselines again at M2's start, at HEAD.
  * The scenario uses the 1-minute synthetic song of `memory-budget` (`long_song`, ASSET-1), at the default chunk.
  * Each run starts a new browser profile, so calibration runs each time.  The result records the kernels that each run chose.  The tolerance must absorb a flip between near-equal kernels, or the gate compares like with like.
  * The result records the adapter name and `isFallbackAdapter`.  This also serves GPU-3 (TD-10): narrow TD-10.  A run on SwiftShader or on a fallback adapter is never a pass or a baseline.
  * The design of how `check --tier push` runs the gate is the team's, within decision 0026.  Keep the added time in proportion: one run in each browser for the gate.  The 5-run spread sets the tolerance.
  * Each test doc says only what the test asserts, as a list (parent's Decision log).
  * No new crate.  The simplest design that meets the M2 section.

### The code today

* `xtask/src/e2e.rs`, `memory_budget`: the flow that M2 reuses.  `long_song(out, minutes)` writes the song.  `weights_mount` mounts the model's folder.  `Page::open` starts a new WebDriver session with `--enable-unsafe-webgpu` (Chrome).  The page waits for `separator: `, gets the song (`drop_files`), waits for `song kept in browser storage`, gets the model (`drop_model`), waits for `model loaded`, clicks Separate (`o.separate_x`), and waits for `separation_shown`.  `query(o, "")` gives the page URL's query.  `chunk_done(line)` reads `k` from a `chunk k/N: ...` line.  `number_after(line, key)` reads a number after a key.
* What the worker and the page log during a run (from M1's logs at `0c56d0a`, f16 weights, the 1-minute song):
  * `separator: kernels: matmul v64,v64, attention freq (calibrated; f16 weights)` (Chrome), and `... attention fused ...` (Safari): the kernels chosen (`roformer_gpu::bench::kernels_line`).
  * `separator: model loaded in 12.5 s, saved in browser storage (apple metal-3; own WGSL kernels at ...)` (Chrome), and `(apple apple apple; ...)` (Safari): the adapter text (`gpu_text` in `worker/src/web.rs`, from the JS `gpu_info`: `vendor`, `architecture` and `description` of `GPUAdapterInfo`).  `gpu_text` also gives the `selftest: GPU ...` and `bench: GPU ...` lines.
  * `separator: chunk 2/11: 4.7 s; so far ..., 1.34 TFLOP/s on average ...; memory: ...`: one line for each chunk.  The time has one decimal.
  * `separator: separation: 11242 GPU steps, ..., 1.33 TFLOP/s on average ...`: the matmul rate of the whole separation (`Pace::summary`).
  * `separator: 6 stems in 52.0 s`: the whole separation (the page).
  * `memory: ... (f16 weight matrices) ...`: the weight format.
  * With `?profile=1`: `profile: <label>: <secs> s GPU in <n> dispatches (<p>%)`, then `profile: total GPU time ...` and `profile: GPU busy ...` (`roformer_gpu::profile_report`, `log_profile`).
* Measured after M1 (M1's logs, `0c56d0a`, one run each): Chrome 157, 4.7 s for each chunk, 52.0 s in all, 1.33 TFLOP/s.  Safari 27.2, 7.4 s for each chunk, 81.3 s in all, 0.84 TFLOP/s.  So Safari is 0.3 s slower for each chunk than the parent's Context (7.1 s).  Step 5 gives the real baselines.
* `roformer-gpu/src/lib.rs`, `Rec::push`: with a profiling device, it keeps one label for each dispatch.  A matmul gets `matmul_label`: `matmul {m}x{n}x{k} x{batch} {tile}`, with ` f16` for an f16 variant.  Every other kernel gets its name only: `attention`, `softmax`, `rmsnorm`, `rope`, `gate`, `transpose`, `glu`.  So the profile cannot tell time-axis attention from frequency-axis attention.  `roformer-gpu/tests/kernels.rs` and `roformer-gpu/tests/memory.rs` read the key `softmax` of `take_profile()`.
* `xtask/src/webdriver.rs`, `Session::new`: it reads `sessionId` from the new session's reply and discards `capabilities`.  W3C WebDriver gives `capabilities.browserVersion` there ([W3C WebDriver, New Session](https://www.w3.org/TR/webdriver2/#new-session)).
* `xtask/src/check.rs`: `lines()` gives the 13 lines of tier `push`, which `the_push_tier_runs_the_lines_of_claude_md` (`xtask/tests/rules.rs`) keeps equal to `CLAUDE.md`'s Commands.  `push_choices(paths)` runs the GPU test line only when a path is in `GPU_FOLDERS` or is a workspace file, or no path is known.  `with_gpu_require(plan, require)` changes a plan from the environment; it is pure and tested.  `never_run()` lists `cargo run -p xtask -- e2e <scenario>` with the reason that CI runs the browser tests.  For tier `push`, `changed_paths` gives the diff from the merge base with `origin/main`.  So on this branch every push run sees M1's `roformer/` paths.
* Decision 0026: "`xtask check --tier push` runs the fmt, clippy, test, `docs-check` and Trunk build lines ...  It does not run `serve`, `web-test`, `e2e` or `publish`."  Its reason is that CI runs the browser tests.  CI has no GPU, so CI cannot run the perf gate.
* TD-10 (`docs/tech-debt.md`): GPU-3 is not checked.  Its fix: "log `isFallbackAdapter` and the adapter name in selftest and e2e results".  GPU-3 in `docs/invariants.md`: "Enforced by: prose only."

### Facts from outside the repo, checked on 2026-11-12

* `GPUAdapterInfo.isFallbackAdapter` came in Chrome 136.  `GPUAdapter.isFallbackAdapter` was removed in Chrome 140.  Chrome had shipped no fallback adapter then, so the attribute was always false on users' devices ([New in WebGPU 136](https://developer.chrome.com/blog/new-in-webgpu-136), [New in WebGPU 140](https://developer.chrome.com/blog/new-in-webgpu-140)).  So `isFallbackAdapter` alone does not find SwiftShader.  The scenario also refuses an adapter text with `swiftshader`, and the `--swiftshader` option.
* Safari's support of `GPUAdapterInfo.isFallbackAdapter` is not known.  Step 1 records what Safari 27.2 gives.

### The paths of this run

* Weights: `~/Downloads/bs_roformer_sw.f16.safetensors`, outside the repo (ASSET-1).
* Chrome: `CHROMEDRIVER=~/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.
* Safari: through `safaridriver`, which `xtask` drives.  Never quit the maintainer's Safari.  Read `docs/references/local-browser-testing.md` first.
* Build the page with `(cd app && trunk build --release)` before each e2e run.  Run one browser GPU run at a time.

### Terms

* Run: one new browser session that opens the song, loads the model, calibrates and separates once.
* Chunk median: the median of the times of chunks 2 to N of one run, from the `chunk k/N: X s` lines.  Chunk 1 is left out: it is the warm-up of M1.
* Key: the browser name (`safari` or `chrome`), the adapter text, and the host's chip (`sysctl -n machdep.cpu.brand_string`, for example `Apple M2 Pro`; `unknown` where the command fails).  The baselines file has at most one entry for each key.  The adapter text alone is the same on each Apple silicon Mac (`apple metal-3`, `apple apple apple`).
* Chunk lower quartile: the lower quartile of the times of chunks 2 to N of one run, by nearest rank: sort the times, and take the value at index floor((n - 1) / 4).  For 10 chunks, that is the third fastest.  The gate reads it, not the chunk median (Decision log, Step 5).
* Baseline: the median of the chunk lower quartiles of the runs of an update, for one key.
* Tolerance: a percentage above the baseline.  A gated run fails when its chunk lower quartile is more than the baseline times (1 + tolerance / 100).

## Plan of work

The simplest design that the M2 section allows: pure functions over the log that the app already writes, one new scenario that reuses the flow of `memory-budget`, and a JSON file that `serde_json` (already a dependency) reads.  The gate in tier `push` is an opt-in line, like the GPU test line's `STEMYLA_REQUIRE`.

Each step's commit holds the doc changes that the step makes true (DOC-1).  Step 6 lists them.  Each new test fails first on an assert, with a stub; Progress records the first failure.

### Step 1: the worker names the adapter and `isFallbackAdapter`

* A new JS function beside `gpu_info` in `worker/src/web.rs`, `gpu_fallback(pref)`: it gives `"yes"`, `"no"` or `"unknown"` from `adapter.info.isFallbackAdapter`.  Do not change `gpu_info`: the calibration key `choice_key` uses its text, and a new text makes every user's GPU calibrate again.
* `gpu_text` adds `fallback adapter <yes|no|unknown>` after the adapter text: `apple metal-3, fallback adapter no; own WGSL kernels ...`.  So the `model loaded`, `selftest: GPU` and `bench: GPU` lines all carry it.
* Evidence: the worker's text has no test without a browser and a GPU.  The e2e runs of Step 5 are the evidence: Step 3's parse fails each run whose `model loaded` line has no fallback text.  Record each browser's value in Surprises.

### Step 2: profile labels name the axis of attention

* `Rec::softmax` sets its label to `softmax {rows}x{len}`.  The fused attention sets its label to `attention seq {seq} x{nseq}`.  Each through a pure function beside `matmul_label`, with a unit test like `matmul_labels_name_the_f16_variant`.
* Change `roformer-gpu/tests/kernels.rs` and `roformer-gpu/tests/memory.rs`, which read the key `softmax`, to the new label.  `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passes on the M2 Pro (Metal).
* No kernel and no choice changes, so `CALIBRATION` does not change.  The labels exist only on a profiling device.

### Step 3: the scenario `e2e perf`

Pure functions in a new file `xtask/src/e2e/perf.rs`, each with a unit test on a log made of the lines above:

1. `perf_result(log, browser, version) -> Result<PerfResult, String>`.  `PerfResult` holds: the browser and its version, the adapter text, the fallback value, the kernels line's choice (`matmul v64,v64, attention freq`), the weight format, the time of each chunk, the chunk median, the whole time, the matmul rate, and the profile groups (empty without `profile=1`).  It fails, with a message that names the missing line, when a line is missing.  Test cases: a full log gives each value; the chunk median of 2 to N leaves chunk 1 out; a log without the fallback text, without the kernels line, or without a chunk line fails.
2. `profile_groups(log, frames, bands) -> Groups`: GPU seconds in four groups.  Time-axis attention: a matmul whose `m` and (`n` or `k`) are `frames`, a softmax whose `len` is `frames`, an attention whose `seq` is `frames`.  Frequency-axis attention: the same with `bands`.  Linear products: each other matmul, the attention gates' matmul (`m` 1,151, `n` 8) included.  Other: each other label.  `frames` is `roformer::CHUNK / roformer::HOP + 1` (1,151); `bands` is the number of bands of `roformer::band_groups`.  Test cases: one line of each kind goes to its group; the sum of the groups equals the total line.
3. `never_a_pass(result, opts) -> Option<String>`: the reason that a run cannot pass or be a baseline.  The fallback value is `yes`; the adapter text holds `swiftshader` (any case); `--swiftshader` or `--headed` is set (the baselines are headless); the weight format is not f16; the query has `chunk=`, `overlap=`, `mm=`, `attn=` or `weights=`.  Test cases: one for each reason, and a run on `apple metal-3` with no query gives `None`.
4. `gate(result, baseline) -> Verdict`: `Fail` when the chunk median is more than the baseline times (1 + tolerance / 100); else `Pass`.  The verdict text gives the median, the baseline, the tolerance and the margin.  It adds a line when the browser version or the kernels differ from the baseline's, and a line "faster than the baseline by more than the tolerance: update the baseline" when the median is under the baseline times (1 - tolerance / 100).  A run with `profile=1` is not gated, since profiling runs one pass for each dispatch: the verdict is `NotGated` with the reason.  Test cases: at the bound passes; 0.01 s over fails; a profile run is not gated; a new browser version passes with the note.
5. `tolerance_pct(medians) -> f64`: with the baseline `b` (the median of `medians`) and the slowest median `w`, the tolerance is the larger of 10 and 2 x (w - b) / b x 100, rounded up to a whole percent.  It refuses fewer than 5 medians.  Test cases: 5 equal medians give 10; a slowest median 6 % over gives 12; 4 medians are refused.
6. The baselines file, `xtask/perf-baselines.json`: a list of entries.  Each holds the key, the browser version, the date, the commit (short SHA), the order, kernels and chunk median of each run, the baseline, the tolerance, and the reason of a raise (empty if none).  `read_baselines` and `with_entry(baselines, entry)` (which replaces the entry of the same key) are pure.  Test cases: a round trip keeps each field; `with_entry` replaces only the entry of its key.  A test reads the committed file: each entry has a tolerance of at least 10, and no value holds a machine path (`/Users/`, `/home/`, `~/`).
7. `update_refusal(old, new, raise) -> Option<String>`: an update may not raise a baseline above the old baseline times (1 + old tolerance / 100), unless the browser version differs from the old entry's, or `--raise "<reason>"` is given.  The reason goes in the entry.  Why: a raise without a reason hides the slowdown that the gate must stop.  Test cases: a lower baseline passes; a raise over the bound is refused; the same raise with a new browser version or a reason passes.

Then:

* `Session::new` keeps `capabilities.browserVersion`, through a pure function on the reply, with a test.
* The scenario `perf` in `xtask/src/e2e.rs` (dispatch, usage line, module doc).  It takes `--runs N` (default 1), `--gate` and `--update-baselines`.  Each run is a new `Page` and so a new browser profile.  The flow is `memory_budget`'s for the 1-minute song.  Factor the shared part into one function, if that is a small change; else keep a short copy and record why.
* Each run writes `<out>/perf-<browser>-<n>.json` (the `PerfResult`) and prints one summary line.  `<out>` is `target/e2e/perf`, outside the repo.
* With a baseline for the key, each run is gated.  Without one, the scenario prints "no baseline for <key>: not gated"; with `--gate`, that fails.  `never_a_pass` fails a run with `--gate` or `--update-baselines`, and else prints its reason.
* `--update-baselines` needs `--runs 5` or more, no reason from `never_a_pass` in any run, and no refusal from `update_refusal`.  It writes the entry of the key with `with_entry`, from the runs.  It is the only path that changes the file.
* The scenario reads the host's chip once, with `sysctl`, for the key.

### Step 4: the gate in `check --tier push`

* A pure function `with_perf_gate(plan, asked: Tier, paths, weights: Option<&str>)` in `xtask/src/check.rs`, like `with_gpu_require`.  `asked` is the tier that `check` was asked for (`main` passes its `tier`).  `plan.tier` cannot tell it: `plan()` sets `Tier::Push` also for a strict path and for no graph.  `weights` is `STEMYLA_PERF_WEIGHTS`, the model's path.
* It adds no line when `asked` is `Tier::Commit`, also when a strict path moves a commit up to tier `push`.  Why: `roformer-gpu/src/lib.rs` and `worker/src/web.rs` are strict paths, so most commits of M3 to M6 run tier `push`.  The gate then adds 3 to 4 minutes and needs an unlocked screen at each commit.
* With tier `push` asked, and a changed path in `roformer/`, `roformer-gpu/` or `worker/`, or a workspace file, or no path known:
  * `weights` set: two Run lines, `cargo run -p xtask -- e2e perf --gate --browser chrome --weights <path>`, then the same with `--browser safari`.  Build the argv as a list, not with `Check::root`, since a path can hold spaces.  A browser that cannot run fails its own line: `e2e` already fails with no `CHROMEDRIVER`, and on a locked screen.
  * `weights` not set: one skip line, "the perf gate did not run: STEMYLA_PERF_WEIGHTS does not name the model".
* Else one skip line with the paths' reason.
* The gate lines come after the Trunk line, which builds `app/dist`.  The order is Chrome, then Safari, as in Step 5.
* No `--release`: the timed work runs in the browser, and the push tier already builds the debug `xtask`.  If the debug build makes `long_song` slow (over 30 s), record it, and decide in the Decision log.
* Tests in `check.rs`, each failing first: tier `push` with GPU paths and the variable runs both lines, in that order, after the Trunk line; without the variable, one skip line with the reason; a docs-only change skips with the paths' reason; `asked` `Tier::Commit` with a strict path adds no line, run or skip.
* Rename and extend `check_never_runs_the_browser_or_publish`: `plan()` never runs an `e2e` line, and only `with_perf_gate` with the variable set adds `e2e perf --gate`.
* Why not a word of `STEMYLA_REQUIRE`: the variable that names the weights is already the claim, so a second switch adds nothing.  `test-support` does not change.
* This changes decision 0026, which says that tier `push` runs no `e2e`.  Its reason is that CI runs the browser tests; CI has no GPU, so it cannot run this gate.  Write decision 0031, `proposed`: tier `push`, when asked, runs `e2e perf --gate` in Chrome and Safari when `STEMYLA_PERF_WEIGHTS` is set and the paths change GPU or worker code.  It says that an agent sets the variable for the push check before a review round and before a push, on this M2 Pro.  It names the cost (one run in each browser, about 3.5 minutes in all) and the unlocked screen.  Give 0026 the line "Amended by 0031 (proposed)".  The orchestrator lists 0031 under "Needs your attention".
* The texts that the gate makes false change in this step's commit (DOC-1): the module doc of `check.rs` ("No check is a browser test"); GIT-2 in `docs/invariants.md` ("No tier runs a browser test"); the doc of `the_push_tier_runs_the_lines_of_claude_md` ("and only those"); the reason of the `e2e <scenario>` line in `never_run` (name 0031 and the perf gate); the test's new name in GIT-2's "Enforced by" (`docs-check` checks test names); the comment of the `check --tier push` line in `CLAUDE.md`; `.claude/rules/gpu.md`.  The last two go to the orchestrator as a step checkpoint before this step's commit (texts in Step 6).  Wait until it applies them.  The commit adds them by name.

### Step 5: the baselines, measured

1. Build the page.  Run `cargo run -p xtask -- e2e perf --runs 5 --update-baselines --browser chrome --weights <path>`, then the same in Safari.  One browser at a time, Chrome first, as the gate runs them.
2. Record in Surprises, for each browser: each run's order, chunk median, kernels, adapter text, fallback value and browser version; the key; the baseline, the spread ((slowest - fastest) / baseline) and the tolerance.  If the medians rise with the order (thermal drift), record it, and decide in the Decision log before the commit.
3. The cost of a kernel flip, measured: if the 5 runs of a browser chose the same kernels, run once more with the other near choice forced.  For Safari, `--query attn=freq`: time axis split, frequency axis fused (calibration chose fused on the time axis, split 3.43 s against fused 3.27 s).  Not `attn=split`, which also splits the frequency axis, a change that calibration never makes (`decide` in `roformer-gpu/src/bench.rs`).  For Chrome, `--query attn=fused`, and the second-best matmul from the calibration line if it is within 10 % of `v64`.  These runs are not gated (`never_a_pass` gives the reason).  The cost is the difference of the chunk medians, as a percentage of the baseline.  If a run of the 5 flipped, the spread holds the cost; record which case applies.  The tolerance must be more than the cost.  If it is not, decide in the Decision log: raise the tolerance to cover it, or key the baseline on the kernels.
4. If a tolerance is over 20 %, record the figures and decide in the Decision log before the commit.  A gate that loose would miss a slowdown of one kernel group.
5. Commit `xtask/perf-baselines.json`.  Then run `cargo run -p xtask -- check --tier push` with `STEMYLA_PERF_WEIGHTS` and `CHROMEDRIVER` set.  Both gate lines pass, after the GPU tests and the Trunk build.  Record each margin to the bound in Surprises.
6. Run `e2e perf --query profile=1` once in each browser, and record the four groups in Surprises.  M3 and M5 use them.
7. Run `e2e selftest --weights <path>` once in each browser.  Check that its `selftest: GPU` line has the fallback text (TD-10).

### Step 6: the docs and the debt

Each in the commit of the step that makes it true (DOC-1):

* `xtask/src/e2e.rs` module doc and `README.md`'s browser-test list (Step 3): the scenario, its options, its result file, its key and its gate.
* `docs/references/local-browser-testing.md` (Step 4): a section on the perf gate.  It gives the variable, the time it adds (one run in each browser, about 3.5 minutes in all), and that the screen stays unlocked and the Mac unused during the Safari run.  It also says that a Chrome update can stop the Chrome line until `CHROMEDRIVER` names a driver of the new version; an old driver may still drive it, with a warning.
* `docs/decisions/0031-*.md`, its line in `docs/README.md`, and the "Amended by" line of 0026 (Step 4).
* The texts of Step 4's last bullet (Step 4).
* `docs/invariants.md`, GPU-3, "Enforced by" (Step 4): `e2e perf` refuses a fallback or SwiftShader adapter as a pass or a baseline.  The `model loaded`, `selftest` and `bench` lines name the adapter and `isFallbackAdapter`.
* `docs/tech-debt.md`, TD-10 (Step 1, then Step 4): the log part of its fix is done.  Left: nothing ties a new default kernel to a pass on a named real GPU; the PR still names it.
* `CLAUDE.md` (Step 4, through the orchestrator): no new line.  The comment of the `check --tier push` line names the gate, for example `# before each push and each review round: the fmt, clippy, test, docs-check and Trunk lines; the perf gate when STEMYLA_PERF_WEIGHTS names the model`.  The line `e2e <scenario>` already covers `e2e perf`.
* `.claude/rules/gpu.md` (Step 4, through the orchestrator): one bullet: "Speed: `e2e perf` gates the lower quartile of the chunk times against `xtask/perf-baselines.json`.  Lower a baseline only with `--update-baselines` from 5 runs or more."
* Parent plan (through the orchestrator, at the checkpoint before review round 1): a Surprises line with the baselines, the spread, the tolerance and the cost of a flip of each browser, and the profile groups.

### Before review round 1

* `cargo run -p xtask -- check --tier push` with `STEMYLA_PERF_WEIGHTS` and `CHROMEDRIVER` set passes, and runs both gate lines.  Without the variable, it prints the skip line.
* `cargo build -p xtask` passes.

## Validation and acceptance

* `cargo test -p xtask` runs the tests of Steps 3 and 4.  Each failed first on its case, with a stub (Progress).
* `cargo test -p roformer-gpu --lib` runs the label tests of Step 2.  `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passes on the M2 Pro.
* `xtask/perf-baselines.json` has an entry for Chrome and one for Safari on this M2 Pro (`Apple M2 Pro` in the key), each from 5 runs, with a tolerance of at least 10 %, and no machine path (a test reads the committed file).
* `cargo run -p xtask -- e2e perf --gate --browser chrome --weights <path>` passes, and the same in Safari.
* The spread, the tolerance and the measured cost of a kernel flip of each browser are in Surprises, and in the parent's Surprises.
* `cargo run -p xtask -- check --tier push` passes, with and without `STEMYLA_PERF_WEIGHTS`.

## Idempotence and recovery

Each step is one commit or more, signed, on `gpu-speed/2026-11-12`.  A session that stops reads Progress and goes on at the next open step.  The e2e runs write only under `target/e2e/perf`, except `--update-baselines`, which writes `xtask/perf-baselines.json`; `git checkout` of that file undoes an update.  A run that a locked screen or a Safari timeout stops runs again (`local-browser-testing.md`).  A browser update between runs changes the baselines: measure again with `--update-baselines`, and record both figures (parent's Extra stop conditions).

## Progress

* [x] Plan review round 1 (2026-11-12): 8 should-fix and 3 nits, each applied.
* [x] Plan review round 2 (2026-11-12): 2 should-fix (the asked tier; Safari's forced flip is `attn=freq`) and 3 nits, each applied.
* [x] Step 1: the worker names the adapter and `isFallbackAdapter` (2026-11-12).  No new test: the e2e runs of Step 5 are the evidence (Step 3's parse fails a `model loaded` line with no fallback text).  TD-10 now says what the log gives.
* [x] Step 2: profile labels name the axis of attention (2026-11-12).  First failure: `attention_labels_name_their_shapes` at `roformer-gpu/src/lib.rs:3704` (`assert_eq!(softmax_label(9208, 1151), "softmax 9208x1151")`, left `""`), with stubs that gave an empty label.  `cargo test -p roformer-gpu --lib` (50 passed) and `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` passed on the M2 Pro (Metal).  The three profile tests that read the softmax label passed with `STEMYLA_REQUIRE=gpu,f16,timestamps`.
* [x] Step 3: the scenario `e2e perf` (2026-11-12).  First failures, with stubs that gave empty or default values (`read_baselines` gave `Err("stub")`), each in `xtask/src/e2e/perf.rs`:
  * `a_full_log_gives_each_value` at `assert_eq!(r.browser, "chrome")` (line 198, left `""`).
  * `the_chunk_median_leaves_chunk_1_out` at `assert_eq!(r.chunk_median, 5.0)` (line 218, left 0.0).
  * `a_log_without_a_needed_line_fails` at its `matches!(&got, Err(e) if e.contains(text))` (line 251, "fallback adapter: Ok(PerfResult { .. })").
  * `each_profile_line_goes_to_its_group` at `assert_eq!(g.time_attention, 15.0)` (line 296, left 0.0).
  * `a_log_without_profile_lines_gives_no_groups` (line 310, left `Some(Groups { .. })`, right `None`).
  * `never_a_pass_names_each_reason` at `assert!(reason.contains(text))` (line 360, "fallback: \"\"").
  * `the_gate_fails_only_over_the_bound` at `assert!(text.contains(part))` (line 429, "5.50 s: ").
  * `the_tolerance_is_twice_the_spread_and_at_least_10` (line 470, left `Ok(0.0)`, right `Ok(10.0)`).
  * `an_entry_holds_the_runs_and_their_median` at `assert_eq!(e.key, key())` (line 493, left a key of empty texts).
  * `baselines_keep_each_field_through_a_round_trip` (line 518, left `Err("stub")`).
  * `with_entry_replaces_only_the_entry_of_its_key` (line 532, left the old Chrome entry).
  * `the_committed_baselines_hold_tolerances_and_no_machine_path` at `assert!(entries.is_ok())` (line 547, `Err("stub")`).
  * `a_raise_over_the_tolerance_needs_a_reason` at `assert!(reason.contains("--raise"))` (line 569, `""`).
  * `the_browser_version_comes_from_the_capabilities` at `xtask/src/webdriver.rs:750` (left `""`, right `"157.0.7000.0"`).
  * On the first try, two tests stopped on an `expect_err` and a `let ... else { panic!() }`, not on an assert.  They now assert.
  * One run of `e2e perf` in each browser passed with no baseline (Surprises).
* [x] Step 4: the gate in `check --tier push` (2026-11-12).  The orchestrator applied the change of `CLAUDE.md` and `.claude/rules/gpu.md` at the step checkpoint.  First failures, each in `xtask/src/check.rs`, with a stub that gave the plan unchanged:
  * `the_push_tier_runs_the_perf_gate_in_chrome_then_safari` at `assert_eq!(next, want)` (line 2018, left the skips of `setup` and `serve` after the Trunk line).
  * `without_the_model_the_perf_gate_is_one_skip_line` at `assert_eq!(reason, ...)` (line 2036, left the reason of `setup`).
  * `a_docs_change_skips_the_perf_gate_for_its_paths` at `assert!(skipped)` (line 2070, the skip of `setup`).
  * `only_the_perf_gate_runs_a_browser_test` (renamed from `check_never_runs_the_browser_or_publish`) at its last `assert_eq!` (line 2151, left `[]`).
  * `tier_commit_adds_no_perf_gate_line` passed with that stub.  With a second stub that did not read `asked`, it failed at `assert_eq!(gated, plain)` (line 2123, left a plan with the gate lines).
  * `check --tier push --dry-run` with `STEMYLA_PERF_WEIGHTS` set prints the two run lines after the Trunk line, Chrome first.  Without it, it prints the skip line.
* [x] Step 5: the baselines, measured (2026-11-12).  Baselines at `1a79cdd`, committed in `d45ce3e`: Chrome 4.70 s, Safari 7.40 s, each with a tolerance of 10 %.  The forced flips, the push check with the gate, the profile groups and the selftests are in Surprises.
  * 5.1 at `7265b9e` gave tolerances of 22 % and 28 % on the chunk median (Surprises), over the 20 % limit of 5.4.  The team leader chose the chunk lower quartile (Decision log).
  * First failures of the change: `the_lower_quartile_is_the_value_at_a_quarter_by_nearest_rank` at `assert_eq!(lower_quartile(&ten), 4.8)` (`perf.rs:571`, left 0.0), with a stub that gave 0.0.  With the tests on the lower quartile and the gate still on the median, `the_gate_fails_only_over_the_bound` failed at `assert!(text.contains(part))` (`perf.rs:823`, "chunk lower quartile 5.50 s: chunk median 4.70 s, ...").  `an_entry_holds_the_runs_and_the_median_of_their_lower_quartiles` failed at `assert_eq!(e.baseline, 5.0)` (`perf.rs:904`, left 4.7).
* [x] Step 6: the docs and the debt (2026-11-12).  Each doc changed in the commit of its step.  `xtask/src/e2e.rs` and `README.md` changed in Step 3 and with the lower quartile.  The perf gate section of `docs/references/local-browser-testing.md`, decision 0031, its `docs/README.md` line, 0026's "Amended by" line, GIT-2, GPU-3, `docs/working-with-agents.md` and TD-10 changed in Step 4.  TD-10 changed in Step 1 too.  The orchestrator applied the changes of `CLAUDE.md` and `.claude/rules/gpu.md` (Step 4, and the lower quartile).  The measured gate time (2.8 minutes) replaced "about 3.5 minutes" in Step 5.  The diff for the parent plan's Surprises is `m2/parent-surprises.diff` in the session's scratch folder.
* [x] Before review round 1 (2026-11-12, at `0d251df`).  `check --tier push` with `STEMYLA_PERF_WEIGHTS` and `CHROMEDRIVER` set passed (15 lines, 402.3 s): the gate passed in Chrome (4.70 s, margin 0.47 s) and in Safari (7.40 s, margin 0.74 s).  Without the variable, it passed (13 lines, 232.4 s) and printed the skip line "the perf gate did not run: STEMYLA_PERF_WEIGHTS does not name the model".  `cargo build -p xtask` passed.
* [ ] Review rounds.

## Surprises and discoveries

* 2026-11-12, plan: M1's logs at `0c56d0a` give the adapter texts `apple metal-3` (Chrome 157) and `apple apple apple` (Safari 27.2).  Neither names the chip.
* 2026-11-12, Step 3: `GPUAdapterInfo.isFallbackAdapter` is `false` in Safari 27.2 and in Chrome 157.0.8103.51 on the M2 Pro: both `model loaded` lines give `fallback adapter no`.
* 2026-11-12, Step 3: one trial run in each browser, before the baselines.  Chrome: chunk median 4.70 s, 52.1 s in all, 1.33 TFLOP/s, `matmul v64,v64, attention freq`.  Safari 27.2: chunk median 7.50 s, 82.2 s in all, 0.83 TFLOP/s, `matmul v64,i64, attention fused`.  So Safari's calibration chose `i64` for the small products in this run, not `v64` as in M1's logs (small: `i64` 1.20 s, `v64` 1.16 s there).  The near choice flips between runs.
* 2026-11-12, Step 5.1, at `7265b9e`: `e2e perf --runs 5 --update-baselines`, Chrome then Safari.  Key `Apple M2 Pro` for both; fallback adapter `no` in each run.
  * Chrome 157.0.8103.51, `apple metal-3`, `matmul v64,v64, attention freq` in all 5 runs.  Chunk medians in order: 4.75, 5.20, 4.70, 4.70, 4.70 s.  Baseline 4.70 s, spread (5.20 - 4.70) / 4.70 = 10.6 %, tolerance 22 %.
  * Safari 27.2, `apple apple apple`.  Runs 1, 2, 3 and 5 chose `matmul v64,v64, attention fused`; run 4 chose `matmul v64,i64, attention fused`.  Chunk medians in order: 7.40, 7.40, 8.40, 7.50, 7.40 s.  Baseline 7.40 s, spread 13.5 %, tolerance 28 %.
  * No drift with the order: the slow runs are run 2 (Chrome) and run 3 (Safari).
  * Both tolerances are over 20 %, so Step 5.4 stops for a decision.
  * The chunk times have two levels.  Most chunks are at a floor (Chrome 4.7 s, Safari 7.4 s), and others are 5 % to 25 % slower, in groups.  For example, Chrome run 2 had chunks 2 to 11 of 4.7, 5.7, 4.7, 4.7, 5.4, 5.6, 5.2, 4.7, 5.2 and 5.9 s.  A run's median rises when more than half of its chunks are slow.
  * Other statistics of the same runs, by the same tolerance rule: the minimum of chunks 2 to N gives 4.7 s in each Chrome run and 7.4 s in each Safari run, so a tolerance of 10 %.  The lower quartile gives Chrome 4.7 s in each run (10 %), and Safari 7.4, 7.4, 7.8, 7.4, 7.4 s (10.8 %, so 11 %).
  * The one natural flip (Safari run 4, `v64,i64`) had a median 1.4 % over the baseline.
  * Wall time for each update: Chrome 350 s, Safari 505 s.  That includes the debug `xtask` and `long_song`.
  * One `ps` sample during the Chrome runs gave WindowServer 48 % CPU; no other process was over 4 %.  The agent session ran beside each run, as it does in a push check.
* 2026-11-12, Step 5.1 again, at `1a79cdd`, on the chunk lower quartile.  Each run chose the same kernels in its browser, and gave fallback adapter `no`.
  * Chrome 157.0.8103.51, `apple metal-3`, Apple M2 Pro, `matmul v64,v64, attention freq`.  Chunk lower quartiles in order: 4.70, 4.70, 4.70, 4.70, 4.70 s (chunk medians 4.70, 4.70, 4.70, 4.70, 5.00 s).  Baseline 4.70 s, spread 0 %, tolerance 10 %.
  * Safari 27.2, `apple apple apple`, Apple M2 Pro, `matmul v64,v64, attention fused`.  Chunk lower quartiles: 7.40 s in each run (chunk medians 7.40 s in each run).  Baseline 7.40 s, spread 0 %, tolerance 10 %.
  * No drift with the order.  Runs 3 and 5 of Chrome again had groups of slow chunks (up to 5.9 s), and the lower quartile did not move.
  * Wall time for each update: Chrome 346 s, Safari 493 s.
* 2026-11-12, Step 5.3: the cost of a kernel flip, one forced run each at `1a79cdd`, not gated (`never_a_pass`: "the query sets attn=" or "mm=").  The 5 runs of each browser chose the same kernels, so each near choice was forced.
  * Chrome `attn=fused` (time axis fused): chunk lower quartile 4.80 s, +0.10 s, 2.1 % over the baseline.
  * Chrome `mm=v64,i64`: 4.70 s, 0 %.
  * Safari `attn=freq` (time axis split): 8.10 s in two runs, each with all 10 chunks at 8.0 to 8.1 s.  +0.70 s, 9.5 % over the baseline.  The bound is 7.40 x 1.10 = 8.14 s, so the margin is 0.04 s, under the log's 0.1 s step.
  * Safari `mm=v64,i64`: 7.40 s, 0 %.
* 2026-11-12, Step 5.5, at `d45ce3e`: `check --tier push` with `STEMYLA_PERF_WEIGHTS` and `CHROMEDRIVER` set passed, 15 lines in 406.6 s.  The two gate lines ran after the GPU tests and the Trunk build.  Chrome: chunk lower quartile 4.70 s, bound 5.17 s, margin 0.47 s (10.0 %), line time 70.2 s.  Safari: 7.40 s, bound 8.14 s, margin 0.74 s (10.0 %), line time 99.4 s.  So the gate added 2.8 minutes.  The debug `xtask` and `long_song` fit in those line times, well under 30 s.
* 2026-11-12, Step 5.6: one `e2e perf --query profile=1` run in each browser, GPU seconds by group (one pass for each dispatch, so not gated).
  * Chrome: time-axis attention 20.03 s (43.6 %), frequency-axis attention 1.81 s (3.9 %), linear products 22.02 s (47.9 %), other 2.10 s (4.6 %); total 45.96 s.  Chunk lower quartile 5.00 s with profiling.
  * Safari: time-axis attention 36.93 s (47.2 %), frequency-axis attention 2.73 s (3.5 %), linear products 35.32 s (45.2 %), other 3.20 s (4.1 %); total 78.18 s.  Chunk lower quartile 7.50 s with profiling.
  * Chrome's time-axis group (20.03 s) matches the parent's Context (scores 7.7 s, P.v 7.8 s, softmax 4.4 s).
* 2026-11-12, Step 5.7: `e2e selftest --weights <path>` passed in Chrome and Safari (worst stage 69.4 dB).  Each `selftest: GPU` line has the fallback text: `apple metal-3, fallback adapter no` and `apple apple apple, fallback adapter no` (TD-10).

## Decision log

* 2026-11-12, agent (team leader): the gate is keyed on the browser, the adapter text and the host's chip, not on the kernels.  The tolerance absorbs a flip between near-equal kernels, with a proof in Step 5.  Why: a key on the kernels gives "no baseline" after a flip, and the gate then checks nothing.
* 2026-11-12, agent (team leader): the gate compares across browser versions, and notes a new version.  Why: Chrome updates about each 4 weeks; a key on the version would leave the gate off after each update.
* 2026-11-12, agent (team leader): the baselines come from Step 5's 5 runs, at a commit whose code on the non-profiling path is the code of `91923ae`, except one log line.  Why: the orchestrator asked for baselines at M2's start; the scenario that measures them exists only after Step 3.
* 2026-11-12, agent (team leader): `STEMYLA_PERF_WEIGHTS` set is the claim that the gate runs; no new word of `STEMYLA_REQUIRE`.  Why: one switch is enough, and `test-support` does not change.
* 2026-11-12, agent (team leader): the profile labels of softmax and fused attention get their shapes.  Why: the M2 section asks for time-axis and frequency-axis attention as groups, and a label of the kernel's name alone cannot split them.
* 2026-11-12, agent (team leader, plan review round 1): the key holds the host's chip from `sysctl`.  Why: the adapter text is the same on each Apple silicon Mac, so an M1 would fail and an M4 would always pass.
* 2026-11-12, agent (team leader, plan review round 1): the gate runs only when tier `push` is asked, not when a strict path moves a commit up to it.  Why: most commits of M3 to M6 touch strict paths; the maintainer's limit is one run in each browser at a push.
* 2026-11-12, agent (team leader, plan review rounds 1 and 2): `with_perf_gate` takes the asked tier, the paths and the weights.  A browser that cannot run fails its own `e2e` line.  Why: `e2e` already fails with no `CHROMEDRIVER` and on a locked screen, and `Plan` has no failed state.
* 2026-11-12, agent (team leader, plan review round 1): the gate line runs the debug `xtask`.  Why: a release `xtask` builds Burn and `roformer` again at each push, and the timed work is in the browser.
* 2026-11-12, agent (team leader, plan review round 1): the cost of a kernel flip is measured with the other choice forced, not estimated from the calibration line.  Why: the calibration times cover only sample shapes, and attention flips at `FUSED_MARGIN`, not at a free 10 %.
* 2026-11-12, agent (team leader, plan review round 1): an update may not raise a baseline over its tolerance without a new browser version or a stored reason.  Why: a raise hides the slowdown that the gate must stop.
* 2026-11-12, agent (team leader, plan review round 1): no new `CLAUDE.md` line; the comment of the `check --tier push` line names the gate.  Why: `CLAUDE.md` is at 96 of 100 lines, and the `e2e <scenario>` line covers the command.

* 2026-11-12, agent (implementer): `perf_result` reads the weight format from the `kernels:` line (`(calibrated; f16 weights)`), not from the `memory:` line.  Why: the line that gives the kernels also gives the format, so one line holds both.
* 2026-11-12, agent (implementer): `profile_groups` gives `None` for a log with no `total GPU time` line, and `PerfResult.groups` is then `None`.  The gate reads `groups` to find a profile run.  Why: one field tells both what the run measured and whether it is gated.
* 2026-11-12, agent (implementer): a pure `new_entry` builds the entry of an update from its runs.  It refuses a profile run, and runs of more than one browser, adapter or browser version.  Why: the plan's rule "no reason from `never_a_pass`" does not cover a profile run, whose times are one pass for each dispatch.
* 2026-11-12, agent (implementer): with `--update-baselines`, a run over the old bound prints the gate's text and does not fail.  `update_refusal` decides the update.  Why: a new browser version or a `--raise` reason can make a slower baseline correct.
* 2026-11-12, agent (implementer): `memory_budget` and `perf` share two small functions, `open_song` and `load_and_separate`.  Each returns the log's mark that `memory_budget` used before, so its checks read the same lines.

* 2026-11-12, agent (implementer): GPU-3's "Enforced by" names the test `never_a_pass_names_each_reason`, not the function `never_a_pass`.  Why: `docs-check` reads each name in that line as a test name.
* 2026-11-12, agent (implementer): the skip line of the perf gate shows `--browser chrome|safari --weights $STEMYLA_PERF_WEIGHTS`.  Why: one skip line stands for both run lines.
* 2026-11-12, agent (team leader), Step 5: the gate, the baseline, `tolerance_pct`, `update_refusal` and the "faster than the baseline" note read the chunk lower quartile, not the chunk median.  `PerfResult` keeps the chunk median.  The other rules stay: at least 10 %, 2 x the slowest run's excess, and a Decision log line over 20 %.  Why: the noise only adds time, and a slower kernel moves every chunk, the floor too.  Under one-sided noise, a low quantile is the robust estimate ([Chen and Revels, "Robust benchmarking in noisy environments", 2016](https://arxiv.org/abs/1608.04295)).  The lower quartile, not the minimum, so that one fast chunk cannot move it.  The 5 runs of 5.1: Chrome 22 % on the median against 10 % on the lower quartile; Safari 28 % against 11 %.
* 2026-11-12, agent (implementer), Step 5.3: both tolerances stay at 10 %, and the baseline is not keyed on the kernels.  Why: each measured flip cost is under 10 %: Chrome 2.1 % and 0 %, Safari 9.5 % and 0 %.  The Safari `attn=freq` margin is thin, 0.04 s, but its two runs gave 8.10 s with every chunk at 8.0 to 8.1 s.  The lower quartile moves only when 8 of 10 chunks are slow.  Calibration chose fused by 5 % in Safari (split 3.43 s, fused 3.27 s), so this flip is rare.  If a flip fails the gate, the gate's text names the other kernels, and an update with a `--raise` reason records it.
* 2026-11-12, agent (team leader), Step 5: Safari's tolerance stays at 10 %, though a forced `attn=freq` run (+9.5 %, 8.10 s) is 0.04 s under the bound (8.14 s).  Why: calibration splits the time axis only when split measures more than `FUSED_MARGIN` (1.1) times faster than fused (`decide` in `roformer-gpu/src/bench.rs`).  In 26 Safari calibrations in the M1 and M2 logs, fused / split was 0.975 to 0.992.  So a flip needs a calibration error of more than 10.8 %, against a usual spread of under 2.4 % (one round had 26.9 %).  A tolerance of 15 % would cover the flip, but would also let a 14 % slowdown pass.  A gate failure gives the run's kernels beside the baseline's, so a flip shows as the cause.  If a Safari gate fails with `attention freq`, run it again before any change of the tolerance.

## Outcomes and retrospective

Not yet.
