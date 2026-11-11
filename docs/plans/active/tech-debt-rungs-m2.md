# Tech debt rungs, M2: Safari and verification

## Purpose

This is the milestone plan for M2 of `plans/active/tech-debt-rungs.md`.  The parent's M2 section stays the criteria.  After M2, one command installs the dev tools at their pins, and the tools are at their new releases.  `xtask web-test` runs each crate in Safari and Chrome, and lists each crate that failed.  The Safari failures that the repo records have a cause and a fix, or a TD with the cause.  A macOS job in CI runs Safari and the real AAC encode.

Readers: the implementer, the review agents of M2's rounds, the orchestrator, and the maintainer at the review of PR #44.

## Context and orientation

The state at the start, on 2026-11-11, at `739833f` on branch `tech-debt/2026-11-11`.  The review base of M2 is `ee3728e`.  The machine is the maintainer's Mac: arm64, Apple M2 Pro, Safari 27.2, Chrome 157.

### The decisions that M2 follows

All are in the parent's "Decisions for the maintainer" (questions 5 to 8) and its Decision log, and in the orchestrator's brief of 2026-11-11.

* Q5: steps 1 to 10 of the parent's M2 only.  TD-7, TD-53, TD-57, TD-64 and TD-66 wait for a later rung.
* Q6: a macOS job on `macos-latest`, a standard runner, free in this public repo.  It is not a required check at first.
* Q7 (TD-93): `web-test` sets `WASM_BINDGEN_NO_DEBUG=1` for `stem-worker` in Safari only, and prints why.  Run `e2e selftest --browser safari` with the weights.  Write a draft wgpu issue as a text file in the orchestrator's scratchpad folder `m2/`, for the maintainer to file.  No agent posts to an outside project.
* Q8 (flakes): fix the cause.  Never retry a test or a scenario.  Only a WebDriver session start may end a stale session and try once more.  A cause outside the repo gets a clear error, a line in `references/local-browser-testing.md`, and a TD.
* Agent: `xtask setup`, not a Makefile (BUILD-1).
* Agent: step 4 goes before step 2, so a Safari and Chrome baseline of every crate comes before the new wasm-bindgen.

### The paths of this run

* The orchestrator's scratchpad folder: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/e7f95d63-1ad1-4b43-bc80-eeb341ac774e/scratchpad/m2/`.  This plan calls it `m2/`.
* The step 2 patch: `m2/step2-wasm-bindgen-0.2.130.patch`.  `git apply --check` passes at `808aa66` and at `739833f`.
* Chrome for Testing driver: `CHROMEDRIVER=/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.
* The weights for `e2e selftest`: `/Users/lloyd/Downloads/bs_roformer_sw.f16.safetensors`.  Never copy them into the repo (ASSET-1).

### The rules of the Mac

* `caffeinate -dimsu` runs for the whole run.  So macOS does not report the user idle, unless a person stops it.
* Never quit the maintainer's Safari.  One browser run at a time: Safari allows one automation session.
* On a `safaridriver` timeout, end the stale WebDriver sessions, and try once more.  If it fails again, stop the Safari step, report to the team leader, and do the steps that need no Safari.
* `wasm-bindgen-test-runner` breaks under `cargo test -q`.  Run it without `-q`.
* Only the orchestrator edits `CLAUDE.md`, `.claude/**` and the parent plan.  The implementer gives each such change to the team leader as a unified diff in `m2/`.

### The code today

* `xtask/src/setup.rs` (step 1, `808aa66`): `TRUNK`, `CARGO_MUTANTS`, `WASM_TARGET`, `tools(lock)`, and the tests `the_docs_and_ci_name_only_the_pins`.  `wasm-bindgen-cli` takes its version from `Cargo.lock`.
* `xtask/src/web_test.rs`: `run` builds four `cargo test --target wasm32-unknown-unknown` runs, one for each crate: `app`, `stem-worker`, `decode-worker` and `stem-protocol`.  It stops at the first crate that fails (`bail!`).  It sets the driver variables, `WASM_BINDGEN_TEST_TIMEOUT` and, for Chrome, a capabilities file.
* `xtask/src/drivers.rs`: `Browser`, `driver()` and `setup_hint()`.  Safari is `/usr/bin/safaridriver`.
* `xtask/src/webdriver.rs`: the WebDriver client of `xtask e2e`.  `Driver::start` and `Session::new` start a session.
* `xtask/src/e2e.rs` and `xtask/src/e2e/`: the scenarios.  The scenarios that need no GPU are those that CI's `browser-tests` job runs: `smoke`, `decode`, `playback`, `old-session`, `audio-resume`, `storage`, `flac` and `save`.
* `xtask/src/e2e/save.rs`: `aac_rule(os, offered)` decides if a run saves AAC.  On macOS, a page that offers no AAC fails the run.
* `xtask/src/publish.rs`: `xtask publish` builds with Trunk into a dist folder, and copies the build to the `gh-pages` tree (`prepare_site`, `finish_site`).
* `.github/workflows/ci.yml`: `TRUNK_VERSION` and `TRUNK_SHA256` in `env`, the action `.github/actions/setup-trunk` (the `x86_64-unknown-linux-gnu` release tarball), and the job `browser-tests` on `ubuntu-latest`.
* `test-support/src/lib.rs`: `Need` (`gpu`, `timestamps`, `f16`, `fixtures`), `parse`, and `skip`.  It reads `STEMYLA_REQUIRE` at run time.  A wasm test cannot read the environment at run time (TD-8).
* `worker/src/web/tests.rs`: `a_read_back_is_copied_into_wasm_memory_once` and `an_f16_variant_the_browser_rejects_keeps_f32_weights` log "skipped: no GPU adapter" and pass with no adapter (TD-8).
* `worker/src/web/tests/aac.rs`: `a_real_aac_decodes_to_the_source` logs a skip and passes where the browser has no AAC encoder (TD-92).
* `decoder/Cargo.toml`: `engine` with `default-features = false`.  `decoder/src/web/tests.rs` holds the 12 wasm tests of TD-95.
* `engine`: `Filter::apply` and `Resampler` in `engine/src/resample.rs` (TD-51).  `engine` has no wasm test today.

### Terms

* A crate run: one `cargo test --target wasm32-unknown-unknown` that `web-test` starts for one crate.
* A baseline: the pass and fail count of each crate run, and the name of each failed test, in one browser, at one commit.
* A stale session: a `safaridriver` session that an earlier run left open, so a new session fails with "already paired".

## Plan of work

Each step is one signed commit, with a test that fails first on an assert, then passes.  The steps run in this order: 1 (done), 4, the baseline, 2, 3, 5, 6, 7, 8, 9, 10.  The implementer updates Progress after each step.

### Step 1: `xtask setup` (done)

The orchestrator committed it in `808aa66`.  No work.

### Step 4: `web-test` runs each crate, then lists each crate that failed

* Change `xtask/src/web_test.rs`: run every crate run, and keep the result of each.  After the last crate run, print one line for each crate: its name, and `passed` or `failed`.  Exit with an error that names each failed crate.
* Put the summary in a pure function, for example `summary(results: &[(&str, bool)]) -> Result<String, String>`.
* The test: a failure of the first crate and a pass of the last crate give an error that names the first crate.  It fails first against a function that keeps only the last result.
* Acceptance: `cargo test -p xtask web_test` passes, and the new test fails on a summary that loses a failure before the last crate.

### The baseline at wasm-bindgen 0.2.129

* After step 4, run `cargo run -p xtask -- web-test --browser safari`, then `cargo run -p xtask -- web-test --browser chrome`.
* Record, in Surprises, a table with each crate's pass and fail counts in each browser, and the name of each failed test.
* Keep the full output in `m2/baseline-0.2.129-<browser>.log`.

### Step 2: wasm-bindgen 0.2.129 to 0.2.130 (DEPS-1)

* Apply `m2/step2-wasm-bindgen-0.2.130.patch` with `git apply`.  It moves `wasm-bindgen` to 0.2.130, `js-sys` and `web-sys` to 0.3.107, `wasm-bindgen-futures` to 0.4.80 and `wasm-bindgen-test` to 0.3.80.  It changes `Cargo.lock` and `THIRD-PARTY.md`.  `async-trait` leaves the dev-dependency table.
* These crates are in `SETTLED_CRATES` of the DEPS-1 check, so the caret requirements stay (Decision log).
* Run `cargo run -p xtask -- setup wasm-bindgen-cli`.  Then run `cargo run -p xtask -- setup --check`.
* Run the two `web-test` runs of the baseline again, and compare.  Record the table in Surprises.
* Acceptance: `check --tier push` passes.  Each crate in each browser gives the same results as the baseline.  A new failure is a stop of this step: keep 0.2.129, record why, and go on.

### Step 3: Trunk 0.21.14 to 0.22.1

* Trunk 0.22.0 locks the dist folder, and keeps the lock file there ([v0.22.0](https://github.com/trunk-rs/trunk/releases/tag/v0.22.0)).  Find the lock file's name in Trunk's source or in a build.
* `publish` copies no build.  Trunk's `--dist` is `target/pages/v/<id>`, which is the site tree itself (`xtask/src/publish.rs`, `main`).  So a lock file in the dist folder goes into `gh-pages` as it is.  A lock file beside the dist folder makes `prepare_site` stop, since `site/v` then holds more than `<id>`.  After the build, remove the lock file, or refuse the build, with a pure function that names the file.  The test of that function fails first when the lock file stays in the site tree.
* Step 3 changes the Trunk version in `README.md` and the other docs only through `xtask setup`.  `the_docs_and_ci_name_only_the_pins` fails a doc that names the old version: fix each one it names.
* Download `trunk-x86_64-unknown-linux-gnu.tar.gz` of the official v0.22.1 release from `https://github.com/trunk-rs/trunk/releases/download/v0.22.1/`, into a new empty folder in the scratchpad.  Hash it with `shasum -a 256`.  Put the hash in `TRUNK_SHA256` of `ci.yml`.  Do not run the binary.
* Change `TRUNK` in `xtask/src/setup.rs` and `TRUNK_VERSION` in `ci.yml` to 0.22.1.  `the_docs_and_ci_name_only_the_pins` keeps the two equal.
* Run `cargo run -p xtask -- setup trunk`.  Run `trunk build --release` in `app` with 0.21.14 first and with 0.22.1 after.  Compare the file names in `app/dist`, and check that `filehash = false` keeps the worker names.
* Acceptance: the two builds give the same file names, but the lock file.  The new test fails when `publish` would copy the lock file.  If 0.22.1 breaks a check and the cause is Trunk, keep 0.21.14, record why, and go on.

### Step 5: TD-95, the decode worker's tests in Safari (VERIFY-1)

* The facts: in Safari 27.2, 6 of the 12 wasm tests of `decode-worker` decode each file as `Undecodable`.  A WAV gives "no decoder for codec: unsupported feature: core (codec): unsupported audio codec".  Chrome passes all 12.  `e2e decode --browser safari`, a release build, passes.
* Find the cause by experiment in Safari, not by guess.  Each experiment changes one thing, such as the release profile, `WASM_BINDGEN_NO_DEBUG`, the features of `engine`, a test that decodes a WAV with no storage, or the order of the tests.  Record each experiment and its result in Surprises.
* Fix the test or the code, as the cause says.  The test that shows the fix fails first in Safari.
* Acceptance: `cargo run -p xtask -- web-test --browser safari` passes the 12 tests of `decode-worker`, and Chrome still passes them.  If the cause is outside the repo, record it in `references/browser-quirks.md`, and narrow TD-95 to it.  Else close TD-95.

### Step 6: TD-93, `WASM_BINDGEN_NO_DEBUG=1` for the stem worker in Safari (VERIFY-1)

* Add a pure function in `xtask/src/web_test.rs` that gives the extra variables of a crate run, from the browser and the package.  It gives `WASM_BINDGEN_NO_DEBUG=1` only for `stem-worker` in Safari.  `web-test` prints why: Safari 27.2 gives no `GPUAdapterInfo.subgroupMinSize`, and the debug glue asserts a number (TD-93).
* The tests: Chrome with `stem-worker`, and Safari with each other crate, get no such variable.  Safari with `stem-worker` gets it.  Each test fails first.
* Run `cargo run -p xtask -- e2e selftest --browser safari --weights /Users/lloyd/Downloads/bs_roformer_sw.f16.safetensors` on the release build, after `trunk build --release` in `app`.  Record its result in Surprises and in TD-93.
* Write the draft wgpu issue in `m2/wgpu-issue-subgroup-min-size.md`.  It names wgpu 30.0.1, `wgpu/src/backend/webgpu.rs` of the `trunk` branch (about line 884), Safari 27.2 on macOS, the value `undefined`, and the nearest issue [gfx-rs/wgpu#10882](https://github.com/gfx-rs/wgpu/issues/10882).  Do not post it.
* Acceptance: the two GPU tests pass in a Safari `web-test`.  The new test fails for Chrome or another crate when the function gives the variable there.  Narrow TD-93 to the upstream fix.

### Step 7: TD-8, `STEMYLA_REQUIRE` in a wasm test (VERIFY-1)

* A wasm test reads `STEMYLA_REQUIRE` at compile time, with `option_env!`.  Put the reading in `test-support`, so `skip` panics on a listed need on wasm32 too.  Add `test-support` as a wasm32 dev-dependency of `stem-worker`.  LAYER-1 allows `test-support` as a dev-dependency of each crate but `dsp` (`docs/invariants.md`, `docs/ARCHITECTURE.md`).
* `skip` prints with `eprintln!`, and on wasm32-unknown-unknown stderr goes nowhere.  So the wasm tests call `verdict`, log its `Ok` line on the console, and panic on its `Err`.  Or `test-support` gets a wasm32 console path.  Choose one, and record it.  The native test reaches the compile-time path through a pure function that takes the compile-time value and the run-time value.
* The two GPU tests of `worker/src/web/tests.rs` use it in place of `console_log!("skipped: ...")`.  Remove their two entries of `SKIP_ALLOWLIST` in `xtask/src/rules.rs`, or `allowlisted_skips_exist` fails.  Correct the reason "a wasm test cannot read STEMYLA_REQUIRE" of the `app` entries that keep a skip.
* `web-test` passes `STEMYLA_REQUIRE` of its own environment to the build of each crate run.  rustc records an `option_env!` variable in the dep-info file, and Cargo rebuilds a path crate when its value changes.  Show it once with `CARGO_LOG=cargo::core::compiler::fingerprint=info`, which reports `test-support` dirty after a change.  Record the output in Surprises.
* The test: a native test of the pure function fails first.  Then run Chrome with no WebGPU adapter and `STEMYLA_REQUIRE=gpu`, with the filter `a_read_back_is_copied`.  A filter goes to each crate run, and matches the test name without the crate prefix.  `web-test` writes `WASM_BINDGEN_TEST_WEBDRIVER_JSON` for Chrome on each run, so add a tested way to give Chrome more arguments, such as `--disable-webgpu`.  A GPU skip must fail the run.
* Update the docs that say a wasm test cannot read `STEMYLA_REQUIRE`: VERIFY-1 in `docs/invariants.md`, `docs/ARCHITECTURE.md` (the tests), and the doc comment and `ONLY_THE_WORDS` of `test-support/src/lib.rs`.  A change of `CLAUDE.md` goes to the team leader as a diff, for a step checkpoint before the commit.
* Acceptance: with `STEMYLA_REQUIRE=gpu` and no adapter, a GPU skip in a wasm test fails.  Remove that bullet of TD-8.

### Step 8: TD-51, `engine`'s tests on wasm32 (VERIFY-1)

* `wasm-bindgen-test-runner` runs no plain `#[test]`.  It prints "no tests to run!" and exits 0.  Add `wasm-bindgen-test` as a wasm32-only dev-dependency of `engine`, as the other crates take it, at their version after step 2.  It is already in `Cargo.lock` and in `SETTLED_CRATES`.  Mark each test of `engine/src/resample.rs` with `#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]` and `#[cfg_attr(not(target_arch = "wasm32"), test)]`.  Do not use `unsupported = test`: its proc macro must then resolve in a native build.  Add `wasm_bindgen_test_configure!(run_in_browser)` under wasm32, and a crate run `-p engine --lib`.
* A crate run whose test binary has no wasm test must fail `web-test`.  `web-test` captures the output of each crate run while it still prints it, and fails on the runner's marker "no tests to run!".  A filtered run with 0 tests passes: the runner then prints "filtered out", not the marker.  Test both cases.  Step 9's classifier reads the same captured output.
* No test today reaches a `usize` product in `Filter::apply` or `Resampler`.  `the_length_and_the_position_do_not_overflow_32_bits` checks only `output_len` and `Position`, and the other tests use short tones.  Add a test with input of more than `2^32 / phases` frames, at a rate pair that keeps the run short.  For example, 384 kHz to 44.1 kHz gives a step of 1280, so about 29 million frames.  Cover `Resampler` too, as TD-51 names it.  Keep the run inside the runner's 600 s, at opt-level 0.  The test also runs natively in CI's `cargo test --workspace`: record its native time and its wasm32 time in Surprises.
* The test: plant a `usize` product in `Filter::apply`, such as `i * M` in `usize`.  The engine crate run must fail on wasm32, and pass natively.  Record the plant and its failure in Surprises.  Do not commit the plant.
* Decide whether `THIRD-PARTY.md`'s browser-test command takes `-p engine`, since it lists the wasm32 dev-dependencies by crate.  Record the choice.
* Acceptance: the planted `usize` product fails on wasm32 in `web-test`, and the real code passes in Safari and Chrome.  Close TD-51.

### Step 9: the Safari flakes

* Measure first.  Run the whole Safari `web-test` 5 times.  Run each `e2e` scenario that needs no GPU 3 times in Safari (`trunk build --release` first).  Keep each output in `m2/flakes/`.
* Record a table in Surprises: each failure, the run, the test or scenario, and its cause.  The known failures are in "Context and orientation" of the parent plan.
* Fix each cause.  No test or scenario is retried.  Only a WebDriver session start may end a stale session and try once more, in `xtask/src/webdriver.rs`.  In `web-test`, `wasm-bindgen-test-runner` owns the session, and `web-test` sees only its exit status and output.  So `web-test` ends stale Safari sessions before each crate run.  It runs a crate again only when a pure classifier of the output shows that the session failed to start before any test ran.  The classifier has its own test.  Each fix has a test that fails first: a pure test where it can, else a Safari run that failed before the fix.
* A cause outside the repo, such as WebKit or `safaridriver`, gets a clear error, a line in `references/local-browser-testing.md`, and a TD.
* For the idle audio, try `caffeinate -u` for the length of an `e2e` run.  `caffeinate -dimsu` runs for this whole run, so a test of this fix stops it for one measure only if the orchestrator agrees.  Else record the fix as not verified.
* Acceptance: after the fixes, 5 Safari `web-test` runs and 3 runs of each scenario pass, or each failure left has a TD.

### Step 10: a macOS job in CI (TD-92)

* Add a job `macos` to `.github/workflows/ci.yml`, on `macos-latest`.  It is not in `needs` of `publish`, and it is not a required check.  Give it a `timeout-minutes`, with the reason in a comment.
* Its steps: the pinned Rust toolchain with `wasm32-unknown-unknown`, the cache, `cargo run --locked -p xtask -- setup wasm32 trunk wasm-bindgen-cli`, `sudo safaridriver --enable`, the Safari `web-test`, `trunk build --release`, and the Safari `e2e` scenarios that need no GPU.  Then `a_real_aac_decodes_to_the_source` and `e2e save` in Chrome.  `setup` with no names also installs the nightly and cargo-mutants, which the job does not need.  Trunk on macOS comes from `xtask setup` (`cargo install --locked`), since the release binary of the action is for Linux (Decision log).
* With no AAC encoder, the AAC test fails, not skips.  Add a need `aac` to `test-support` (`STEMYLA_REQUIRE=aac`), read at compile time as in step 7.  Set `STEMYLA_REQUIRE=aac` on the Chrome AAC step only, not on the job.  `a_real_aac_decodes_to_the_source` uses it in place of its console skip.  `e2e save` already fails on macOS when the page offers no AAC (`aac_rule`).
* The macOS `e2e save` step gets the same `timeout-minutes` as the Linux one.  `the_ci_step_limit_is_the_waits_limit` (`xtask/src/e2e/save.rs`) reads only the first `e2e save --browser chrome` step.  Make it check each such step.
* The test: a native test of `test-support` fails first for the word `aac`.  The job's YAML is code: commit it like any other code.
* Docs: the macOS job goes in `README.md` (browser tests) and `docs/harness.md`.  The behaviour of steps 4 and 6 goes in `references/local-browser-testing.md`.
* The orchestrator pushes, and reads the CI result.  The team leader fixes a failure of the job in a later commit, and records each run in Surprises.  GitHub states nothing about the runner's GPU, so the GPU tests may skip there.  The job does not set `gpu` in `STEMYLA_REQUIRE`.
* Acceptance: the job passes on the PR.  With no AAC encoder, the AAC test fails, not skips.  Narrow TD-92, or close it.

### Before review round 1

* Run `cargo run -p xtask -- check --tier push` and `cargo build -p xtask`.
* Give the orchestrator each diff of `.claude/**`, `CLAUDE.md` and the parent plan that no step checkpoint carried, in `m2/`.  The parent plan gets M2's Progress lines, the tool facts that changed, and the Decision log lines of this plan.

## Validation and acceptance

* `cargo run -p xtask -- check --tier push` passes on the maintainer's Mac.
* `cargo run -p xtask -- setup --check` passes.
* `cargo run -p xtask -- web-test --browser safari` and `--browser chrome` pass on the Mac.
* CI passes on the PR, the macOS job included.
* `docs/tech-debt.md` lists TD-51 and TD-95 under `## Closed`, or each has a narrowed entry with its cause.  TD-8 has no bullet of the wasm GPU tests.  TD-92 and TD-93 are narrowed or closed.

## Idempotence and recovery

* Each step is one commit with its own test, so a revert of one commit undoes one step.
* Step 2 is a patch.  If it fails, `git checkout -- Cargo.lock THIRD-PARTY.md` and the changed `Cargo.toml` files restore 0.2.129, and `xtask setup wasm-bindgen-cli` installs the old CLI again.
* Step 3: `xtask setup trunk` installs the pin of the tree, so a revert of the step also gives back 0.21.14 at the next setup.
* If a step's test shows a real breach, stop that step.  Record the breach as a new TD, and go on with the next step.
* A Safari run that `safaridriver` refuses after the stale sessions end: do the steps that need no Safari, and list the Safari checks under "Not verified".

## Progress

* [x] 2026-11-11: step 1, `xtask setup`, in `808aa66` (the orchestrator).
* [x] 2026-11-11: step 4, `web-test` runs each crate and lists the failed crates.  The new test `a_failure_before_the_last_crate_names_that_crate` first failed on its `assert_eq!` against a `summary` that kept only the last result: it gave `Ok` with no line of failed crates.
* [x] 2026-11-11: the baseline at wasm-bindgen 0.2.129 in Chrome, at `34f7f6c`: each crate passed (Surprises).  The Safari baseline is at 0.2.130, at `95f5351`: the known failures of TD-93 and TD-95, and one known flake (Surprises).
* [x] 2026-11-11: step 2, wasm-bindgen 0.2.130.  Chrome gives the same results as the baseline (Surprises).  The Safari comparison is owed.  The step changes no behaviour of the repo's code, so it adds no test.
* [x] 2026-11-11: step 3, Trunk 0.22.1.  The new test `trunk_s_lock_file_never_reaches_the_site` (`xtask/tests/publish.rs`) first failed on its first assert: `check_build` gave `Ok(())` for a build with `.trunk-lock`.  With the refusal and no removal, it failed on the assert "the lock file stays in the site tree".  It passes with both.
* [x] 2026-11-11: step 5, TD-95.  `a_planted_file_reads_back_as_written` (`decoder/src/web/tests.rs`) first failed in Safari 27.2 on its `assert_eq!`: the planted file held 1,835,008 bytes, not 4,044, and no `RIFF` at the start.  After the fix, the 13 tests of the `decode-worker` crate run pass in Safari 27.2 and in Chrome 157.  The crate run is the command that `web-test` runs, with the same runner and driver variables.  The full Safari `web-test` comes after step 6.
* [ ] Step 6: TD-93.
* [x] 2026-11-11: step 7, TD-8, `STEMYLA_REQUIRE` in a wasm test.  `a_word_of_the_build_is_required_with_no_run_time_value` and `the_words_of_the_build_and_of_the_run_are_both_required` (`test-support`) first failed on their `assert_eq!` against a `claimed` that kept only the run-time value.  `a_chrome_arg_goes_after_the_autoplay_policy` (`xtask/src/web_test.rs`) first failed on its `assert_eq!` against a function that dropped the extra arguments.  In Chrome with `--disable-gpu` and `STEMYLA_REQUIRE=gpu`, `a_read_back_is_copied_into_wasm_memory_once` fails (Surprises).  `stem-worker` now takes `test-support`, so the fixture graph of `xtask check` (`the_fixture_graph_is_the_real_graph`) takes it too.
* [x] 2026-11-11: step 8, TD-51.  `a_test_binary_with_no_wasm_test_is_found` first failed on `assert!(found_no_test(output))` against a stub that gave `false`.  `the_marker_fails_only_a_run_with_no_filter` first failed on `assert!(crate_run_passed(true, output, Some("the_length")))` against a function that ignored the filter.  Before the engine tests had the wasm attribute, a full Chrome `web-test` failed `engine` only, on "no tests to run!".  The planted `usize` product failed on wasm32 and passed natively (Surprises).  Chrome then passed every crate.  The Safari run is owed.
* [ ] Step 9: the Safari flakes.
* [ ] Step 10: the macOS job (TD-92).
* [ ] Review rounds.

Safari runs owed: none.  The Safari baseline at 0.2.129 is missing (Surprises).  The Safari baseline at 0.2.130 replaces both owed runs.

## Surprises and discoveries

* 2026-11-11, the Safari baseline at `34f7f6c`: each of the 4 crate runs failed at the session start, before a test ran.  `safaridriver` gave HTTP 500, "session not created: The session timed out while connecting to a Safari instance".  No `safaridriver` was left from an earlier run.  A direct probe of `safaridriver -p 4445` gave the same error.  The cause is the locked screen: at 08:53 JST, `ioreg -n Root -d1 -a` gave `CGSSessionScreenIsLocked` = true, while Safari ran (the orchestrator).  The log is `m2/baseline-0.2.129-safari.log`.  Step 4's summary listed the 4 failed crates.
* 2026-11-11, the Chrome baseline at `34f7f6c`, wasm-bindgen 0.2.129, Chrome 157 (`m2/baseline-0.2.129-chrome.log`), and step 2 at wasm-bindgen 0.2.130 (`m2/step2-0.2.130-chrome.log`).  No test failed, and no test was ignored:

  | Crate run | Test binary | 0.2.129 | 0.2.130 |
  | - | - | - | - |
  | `app` | `--lib` | 197 passed (192 s) | 197 passed (190 s) |
  | `app` | `--test web` | 14 passed | 14 passed |
  | `app` | `--test web_close` | 1 passed | 1 passed |
  | `app` | `--test web_output` | 4 passed | 4 passed |
  | `stem-worker` | `--lib` | 108 passed | 108 passed |
  | `decode-worker` | `--lib` | 12 passed | 12 passed |
  | `stem-protocol` | `--test web` | 31 passed | 31 passed |

* 2026-11-11, step 2: crates.io gives 2026-10-30 as the release date of `wasm-bindgen` 0.2.130, `js-sys` and `web-sys` 0.3.107, `wasm-bindgen-futures` 0.4.80 and `wasm-bindgen-test` 0.3.80.  None is yanked.
* 2026-11-11, step 3: Trunk names its lock file `.trunk-lock`, in the dist folder (`BUILD_LOCK_FILE` in `src/build.rs` at the tag `v0.22.1`, [commit 5ccc168](https://github.com/trunk-rs/trunk/commit/5ccc1688d9a313ebbc60af0fb5a0ede366b1803a)).  0.22.1 changes only the dependencies from 0.22.0.  The file stays after the build: it is empty, and `app/dist/.trunk-lock` is ignored by `.gitignore`.
* 2026-11-11, step 3: the SHA-256 of `trunk-x86_64-unknown-linux-gnu.tar.gz` of v0.22.1 is `2ab8d548fd5ed9cda50906f61fd8654204e7893bd52ebbfd90533862b7d71ee1`.  `shasum -a 256` of the download and the `digest` of the release asset in the GitHub API give the same value.  The tarball holds one file, `trunk`.  Nobody ran it.
* 2026-11-11, step 3: `trunk build --release` in `app`, at `5057fc4`, gives the same 26 paths in `app/dist` with 0.21.14 and with 0.22.1, and 0.22.1 adds `.trunk-lock` (`m2/step3-dist-0.21.14.txt`, `m2/step3-dist-0.22.1.txt`).  The workers keep their names (`stem-worker_loader.js`, `decode-worker_loader.js`), and the snippet folders keep their hashes.
* 2026-11-11, step 7, the Chrome runs with the filter `a_read_back_is_copied` (logs in `m2/step7-*.log`):

  | Chrome argument | `STEMYLA_REQUIRE` | Result of `a_read_back_is_copied_into_wasm_memory_once` |
  | - | - | - |
  | `--disable-webgpu` | `gpu` | passed: the adapter opened, so Chrome 157 has no such switch |
  | `--disable-gpu` | `gpu` | failed: "VERIFY-1: STEMYLA_REQUIRE has gpu, but this test cannot run: no GPU adapter: ... webgpu found no adapters" |
  | `--disable-gpu` | not set | passed (the runner shows no console line of a test that passes) |

  `web-test` listed `stem-worker` as the failed crate.  The other crates ran 0 tests, with "filtered out", and passed.
* 2026-11-11, step 7: Cargo builds `test-support` again when `STEMYLA_REQUIRE` changes.  The dep-info file holds `# env-dep:STEMYLA_REQUIRE=gpu`.  `CARGO_LOG=cargo::core::compiler::fingerprint=info` prints nothing in Cargo 1.99.1: the target is now `cargo::compiler::fingerprint`.  `CARGO_LOG=info` gives `stale: changed env "STEMYLA_REQUIRE"`, `Some("gpu") != None` and `dirty: FsStatusOutdated(StaleItem(ChangedEnv { var: "STEMYLA_REQUIRE", previous: Some("gpu"), current: None }))` for `test-support` (`m2/step7-fingerprint.log`).
* 2026-11-11, step 8: the plan said that a filtered run with 0 tests prints "filtered out", not the marker.  That is wrong for wasm-bindgen-test-runner 0.2.130: it removes the filtered tests first, then prints "no tests to run!" when none is left (`src/wasm_bindgen_test_runner.rs`, lines 173-238).  With the filter `the_length`, each of the 5 crate runs printed the marker (`m2/step8-engine-no-wasm-test.log`).  So the marker fails only a run with no filter.
* 2026-11-11, step 8: with no wasm test in `engine`, a full Chrome `web-test` gave "web-test: engine: a test binary has no wasm test", passed the 4 other crates, and failed with "failed crates: engine" (`m2/step8-engine-no-wasm-test-full.log`).
* 2026-11-11, step 8: the plan's rate pair, 384 kHz to 44.1 kHz, made `a_long_input_converts_with_a_32_bit_usize` take 43.6 s natively at opt-level 0.  Its cost is `2^32 / step` outputs times the taps, so the test takes 383,990 Hz to 8,379 Hz: gcd 19, 441 phases (`MAX_PHASES`), step 20,210, 512 taps.  9.74 million frames give 212,000 outputs.  It takes 2.8 s natively, and 2.3 s on wasm32 in Chrome 157 (`m2/step8-long-chrome.log`).  The engine crate run takes 14.9 s in Chrome.
* 2026-11-11, step 8, the plant: `let planted = i * self.step as usize; std::hint::black_box(planted);` in the output loop of `Filter::apply`.  Natively the test passed.  In Chrome it failed: "panicked at engine/src/resample.rs:322:27: attempt to multiply with overflow" (`m2/step8-planted-chrome.log`).  The plant is not committed.
* 2026-11-11, the Safari baseline at `95f5351` (after step 8), wasm-bindgen 0.2.130, Safari 27.2, with the screen unlocked (`m2/baseline-0.2.130-safari.log`).  The Safari baseline at 0.2.129 is missing: the screen was locked.  The M4 records at `69e63db` and `fc99e38` (TD-93, TD-95) serve as the old baseline.  Each failure below is one that the repo records already:

  | Crate run | Test binary | Result | Failed tests |
  | - | - | - | - |
  | `app` | `--lib` | 197 passed (202 s) | none |
  | `app` | `--test web` | 14 passed | none |
  | `app` | `--test web_close` | 1 passed | none |
  | `app` | `--test web_output` | 4 passed | none |
  | `stem-worker` | `--lib` | 105 passed, 3 failed | `a_read_back_is_copied_into_wasm_memory_once` and `an_f16_variant_the_browser_rejects_keeps_f32_weights` (TD-93); `a_stored_file_removed_after_it_was_opened_is_a_read_error` (the known Safari flake, `references/local-browser-testing.md`) |
  | `decode-worker` | `--lib` | 6 passed, 6 failed | the 6 tests of TD-95, the same names |
  | `stem-protocol` | `--test web` | 31 passed | none |
  | `engine` | `--lib` | 18 passed (13 s) | none |

* 2026-11-11, step 5, TD-95, in Safari 27.2 with the screen unlocked.  Each experiment ran the `decode-worker` crate run only, with the runner and `SAFARIDRIVER` that `web-test` sets (logs in `m2/step5/`):

  | Experiment | The one change | Result |
  | - | - | - |
  | e1 | none, at `3a567cd` | 6 passed, 6 failed, the 6 tests of TD-95.  A WAV gives "no decoder for codec", an M4A "no known format at the start of the file". |
  | e2 | a new test plants a WAV of 4,044 bytes with `plant_at`, and reads it back | failed: the file holds 1,835,008 bytes (28 pages of 64 KiB, the wasm memory), with no `RIFF` at the start |
  | e3 | `plant_at` writes `bytes.slice()`, a whole copy | 13 passed |

  The cause is in the test, not in the decode worker.  wasm-bindgen gives a `&[u8]` to JavaScript as a view into the wasm memory.  Safari writes the whole buffer of a view (`references/browser-quirks.md`, MEM-5).  So each planted file held the whole wasm memory.  The release build passed `e2e decode`, since the page writes whole arrays.  The 6 tests that passed decode no planted file: they read the stems as raw PCM and check no sample, or they end before a decode.  So the other experiments of the plan (the release profile, `WASM_BINDGEN_NO_DEBUG`, the features of `engine`, the order of the tests) were not needed.

## Decision log

* 2026-11-11, maintainer: the answers to questions 5 to 8 of the parent plan (see "The decisions that M2 follows").
* 2026-11-11, agent (orchestrator): the crates of step 2 are in `SETTLED_CRATES`, so their caret requirements stay.  The parent's step 2 asks for an exact pin of a release under a month old.  The DEPS-1 check exempts a settled crate from that.
* 2026-11-11, agent: step 7's compile-time reading goes in `test-support`, so one helper keeps the words for native and wasm tests (decision 0003).
* 2026-11-11, agent: step 10 adds a need `aac` to `test-support`, so the macOS job fails a real-encode test that skips.  Reason: the parent's step 10 asks that the AAC test fail, not skip, with no encoder.
* 2026-11-11, agent: `web-test` may run a Safari crate again only when a pure classifier shows that the session failed to start before any test ran.  Reason: the runner owns the session, and Q8 allows a second try only of a session start.
* 2026-11-11, agent: a crate run whose test binary has no wasm test fails `web-test`, on the runner's marker "no tests to run!".  A filtered run with 0 tests passes.  Reason: the runner passes a crate with no `wasm_bindgen_test`, so step 8 could pass with nothing tested.
* 2026-11-11, agent: a new `Plan` agent reviewed this plan in 2 rounds: 10 findings in round 1 and 2 in round 2, each fixed in the plan before its first commit.
* 2026-11-11, agent: the macOS job installs Trunk with `xtask setup`, not with the action.  Reason: the action takes the Linux release tarball only, and BUILD-1 allows `cargo install`.
* 2026-11-11, agent (orchestrator): the cause of the Safari timeouts is the locked screen, not an open Safari.  Never quit Safari.  End each `safaridriver` probe that an agent starts.  Do not try Safari again while the lock holds.
* 2026-11-11, agent (orchestrator): Progress keeps a list "Safari runs owed".  Before each Safari run, check the lock with `ioreg -n Root -d1 -a | grep -A1 CGSSessionScreenIsLocked`.  If the screen is locked, add the run to the list, and go on.
* 2026-11-11, agent (orchestrator): step 9 gets one more fix.  `web-test` and `e2e` with Safari on macOS check the screen lock first, and fail at once with a clear message.  A pure parse of the `ioreg` output gets tests.  A line goes in `references/local-browser-testing.md`.
* 2026-11-11, agent (orchestrator): the order while the lock holds is the Chrome baseline, step 2 (Chrome now, Safari owed), step 3, step 7, step 8 and step 10.  Steps 5, 6 and 9 wait for the lock to clear.  Step 6's pure function, its tests and the draft wgpu issue need no Safari, so they may go first.  After step 10, the implementer stops, so the orchestrator pushes and reads the CI run.
* 2026-11-11, agent (orchestrator): step 2 applies its patch with `--exclude=docs/plans/active/tech-debt-rungs.md`.  The hunk of the parent plan is in `m2/step2-parent-plan.diff`, for the orchestrator.  Reason: only the orchestrator edits the parent plan.
* 2026-11-11, agent: step 3 removes Trunk's lock file after the build, and `check_build` refuses a build that still holds it.  Reason: the removal keeps `publish` working with Trunk 0.22, and the refusal stops the file if a later change skips the removal.
* 2026-11-11, agent: step 7 adds `test_support::skip_line`, which gives the skip line or panics.  A wasm test logs the line with `console_log!`, and `skip` prints it on stderr.  Reason: one function keeps the panic and the words, and `test-support` needs no `web-sys` for a console path.
* 2026-11-11, agent: step 7 joins the words of the build and of the run (`test_support::claimed`).  A word in either is required.  Reason: a wasm test has only the value of the build, and a native run has both, which Cargo keeps equal.
* 2026-11-11, agent: step 7 gives Chrome more arguments with `web-test --chrome-arg <arg>`.  `--disable-gpu` takes the adapter away; `--disable-webgpu` does not.  `web-test` passes `STEMYLA_REQUIRE` to each crate run through its inherited environment, so it needs no code for it.
* 2026-11-11, agent: step 7 keeps `ONLY_THE_WORDS` of `test-support` as it is.  Reason: its text names the words, and says nothing about wasm.
* 2026-11-11, agent: step 8 fails the marker "no tests to run!" only in a `web-test` with no filter.  Reason: the runner prints the marker also when the filter removes each test of a binary, so a filtered run cannot tell the two apart.  CI and each acceptance run have no filter.
* 2026-11-11, agent: step 8 keeps `-p engine` out of the browser-test command of `THIRD-PARTY.md`.  Reason: the command and the shipped command share one package list in `third_party_args`, and `engine`'s crate run adds only test crates and the stretch features (`cute-dsp`, `timestretch` and their dependencies, `oxideav-aac`), which the page does not ship.  DEPS-1 covers what ships; DEPS-1's pins and `Cargo.lock` cover these crates already.
* 2026-11-11, agent: step 4 does not capture the output of a crate run.  Step 8 adds the capture, with its own tests.  Reason: step 4's acceptance is the summary only, and a small commit is easier to review.
* 2026-11-11, agent: step 5 fixes `plant_at` in the test, and closes TD-95.  The new test reads a planted file back, and fails in Safari only, as Chrome writes a view correctly.  Reason: the macOS job of step 10 runs Safari, so it keeps the check in CI.

## Outcomes and retrospective

* To be written at the end of M2.
