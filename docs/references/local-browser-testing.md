# Local browser testing

Running `xtask web-test` and `xtask e2e` on the maintainer's Mac (Intel, macOS 26.7.1, Safari 27.0), as of 2026-10-19.  Since 2026-11-10, the maintainer also runs them on an Apple silicon Mac (arm64).  The commands and their prerequisites are in `README.md` (browser tests); this is what those machines add.

## Which browser

* Safari is the local default.  `safaridriver` starts no session while the screen is locked: it waits, then gives "The session timed out while connecting to a Safari instance" (2026-11-11, TD-96).  So `web-test` and `e2e` with Safari on macOS read the lock first, and fail at once with a clear error.  The check is `ioreg -n Root -d1 -a | grep -A1 CGSSessionScreenIsLocked`, which gives nothing when the screen is unlocked.  Do not quit Safari to clear a timeout.  If sessions time out with the screen unlocked, use `--browser chrome`, and say in the PR that Safari is unverified.
* `web-test` runs each crate, also after a crate fails.  At the end, it prints one line for each crate, and names each crate that failed.
* In Safari, `web-test` sets `WASM_BINDGEN_NO_DEBUG=1` for the crate run of `stem-worker` only, and prints why (TD-93).  To run that crate by hand in Safari, set it too.
* A probe that leaves its WebDriver session open makes later sessions fail with "already paired with another WebDriver session".  `web-test` with Safari ends each `safaridriver` whose parent is gone before each crate run.  `e2e` with Safari ends them before each session start.  If a session does not start, they end such drivers, and try the session start once more.  `web-test` then runs the whole crate again, so a test that passed runs again.  No failed test or scenario runs again (TD-96).  End a probe's session yourself.
* Do not use the Mac during a Safari run.  A click in the automation window shows Safari's "Automation Interruption" alert.  A window over the test page hides the page, and a run can then stop with no failure (TD-96).  After a stop, find the cause with `/usr/bin/log show --last 1h --predicate 'process == "Safari" AND eventMessage CONTAINS "view visibility state changed"'`.  In zsh, `log` is a built-in command, so give the full path.
* `web-test` and `e2e` with Safari hold `caffeinate -u` while they run, so macOS does not report the user idle (TD-96).
* Chrome needs a Chrome for Testing chromedriver of Chrome's major version, in `CHROMEDRIVER`.  Take the build for the CPU of the Mac: `mac-x64` for an Intel Mac, `mac-arm64` for Apple silicon ([Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/)).  Keep it outside the repo and reuse it rather than downloading it again each session.  Chrome updates itself: on 2026-10-29 it went from 155 to 156 during a run, and chromedriver 155 still drove it, with a warning ("has not been tested with Chrome version 156").
* On macOS, each Chrome driver of `e2e`, and each Chrome crate run of `web-test`, gets its own folder `/tmp/stemyla-chrome/<pid>-<n>` (M5.1 of `plans/active/gpu-speed.md`).  `xtask` gives it to `chromedriver` in `MAC_CHROMIUM_TMPDIR`, so each session's profile (`org.chromium.Chromium.scoped_dir.*`) goes there, not in `$TMPDIR`.  `xtask` removes the folder at the end of the run, also after a failure.  A removal that fails prints the path.  A run that a signal ends leaves its folder.  The next Chrome run removes it, when no process names the folder.
* Safari keeps write-stream swap files and blob folders in `$DARWIN_USER_TEMP_DIR/com.apple.WebKit.Networking+com.apple.Safari`, the folder of `getconf DARWIN_USER_TEMP_DIR` (TD-103).  Each Safari `e2e` and `web-test` run lists that folder at its start and at its end.  It then prints one line, such as `browser data: Safari left 1 swap file (349.5 MB) and 0 blob folders (0.0 MB) in <folder>; xtask does not remove them (TD-103)`.  The line does not fail the run, since the person's own Safari can write there too.  `xtask` removes none of these files.  A whole Safari `web-test` leaves about 1,320 swap files (1.7 GB on 2026-11-14), from the test helpers and the app tests' fake write worker (TD-103).  `e2e save` leaves 349 MB from its planted sessions.  An `e2e` run with the model before M5.1 left 350 MB more.
* To free that disk, a person quits Safari, and then removes the `FileSystemWritableStream*` files and the `BlobRegistryFiles-*` folders in that folder by hand.  A quit alone removes none of them (2026-11-14).  An agent removes none: the person's own Safari keeps files there too.

## The perf gate

`cargo run -p xtask -- check --tier push` can run `e2e perf --gate` in Chrome, then in Safari (decision 0031).  It runs them when `STEMYLA_PERF_WEIGHTS` names the model file and the changed paths can change the speed.  Without the variable, it prints a skip line.

* Set `STEMYLA_PERF_WEIGHTS` and `CHROMEDRIVER` for the push check before a review round and before a push, on the M2 Pro whose baselines `xtask/perf-baselines.json` holds.
* The gate fails a run whose browser gives no `isFallbackAdapter` (`fallback adapter unknown`), with that reason.
* The gate adds about 3 minutes: one run in each browser (2.8 minutes on 2026-11-12: Chrome 70 s, Safari 99 s).
* Each `e2e perf` run has a time limit of 10 minutes, from when the run opens its page.  At the limit, the run fails with one line, and ends its WebDriver session and its driver.  So a hung Safari run does not hold Safari for hours (2026-11-13: one Safari run hung for 3 hours).  The limits of the other scenarios are in `README.md` (browser tests).
* Keep the screen unlocked and do not use the Mac during the Safari run.  A locked screen fails the Safari line at once.
* A Chrome update can stop the Chrome line until `CHROMEDRIVER` names a driver of the new version.  An old driver may still drive the new Chrome, with a warning.
* The gate compares across browser versions, and prints a line when the version is not the baseline's.  After a browser update, measure again with `e2e perf --runs 5 --update-baselines`, and record both figures.  A raise of the bound needs `--raise "<reason>"` with any browser version.  After a browser update, the reason names it, for example `--raise "Safari 27.2 to 27.3"`.

## The same-session comparison

`e2e perf --base-dist <dir>` compares the page of a base commit with the head page, in one session (M4 of `plans/active/gpu-speed.md`).  M4, M5 and M6 use it to show no loss and a gain of 3 % or more.

* Build the base page in a worktree, with its own target folder: `git worktree add --detach <wt> <base commit>`, then `(cd <wt>/app && CARGO_TARGET_DIR=<wt>/target trunk build --release)`.  Build the head page in `app/dist`.
* The oldest base is `59da862`.  A page from before it logs the chunk times with 1 decimal, and the comparison refuses its runs.  It also refuses a run on a fallback or a software adapter, and `--swiftshader` (GPU-3).
* Run `STEMYLA_PERF_WEIGHTS=<model> cargo run -p xtask -- e2e perf --browser <b> --weights <model> --base-dist <wt>/app/dist --runs 4`, in Chrome, then in Safari.  Each browser takes about 15 to 30 minutes.
* `--query` applies to both sides.  `--head-query <q>` sets each key of `<q>` on the head's query only.  A key of both takes the head's value, and a new key goes at the end.  For example, `--query "mm=v64k8,v64k8&attn=fused" --head-query "attn=freq"` runs the base with `attn=fused` and the head with `attn=freq`.  Use the same page for both sides to compare two forced policies.  The output gives the query of each side before the first run.
* A comparison fails a run whose `kernels:` line does not name the `attn` that its side's query forces.  `attn=auto` forces none.
* A comparison refuses `attn=split` and `attn=time` in `--query` or `--head-query`.  Both split the frequency axis, which is 2.2 to 3.6 times slower.  `perf` refuses `--head-query` without `--base-dist`.
* Use `--runs 8` when calibration has a near choice in that browser.  A near choice is two kernels or two attention policies within 10 % of the point where calibration flips.
* Take a head whose slow choice (+10 %) comes in 40 % of the runs.  With `--runs 4`, it can stop at "no change" after 2 blocks of fast runs, in about 13 % of comparisons.  With `--runs 8`, that is 0.6^8, 1.7 %.
* The runs go in blocks of base, head, head, base.  The figure of a run is its chunk lower quartile.  The output gives each run in its order with its kernels, each block with its change `r`, and the verdict.
* A block is a loss when `r` is over `T` (1 %), and a gain when it is under `-T`.  A block is spoiled when the two runs of a side chose the same kernels and differ by more than `N` (1.1 %).  A spoiled block does not count.  A side whose two runs chose different kernels does not spoil its block.
* When each side chose one set of kernels in each run, the comparison starts with `--runs` / 2 blocks, and runs one more at a time.  It stops at 4 blocks, or at `--runs` / 2 when that is more.  The verdict is the class of 2 blocks or more, with at most one block of another class.  A loss and a gain together give undecided.
* When the runs of a side chose different kernels anywhere, a slow choice is part of what the head does, not noise.  Then the comparison runs each block up to the most: 4, or `--runs` / 2 when that is more.  The verdict is the class of the mean `r` of the blocks that are not spoiled.  Fewer than 2 such blocks give undecided.  The output gives each kernel choice with its count, under "kernels differ".
* The verdict is a loss, a gain, no change, or undecided.  "3 % or more" follows a loss or a gain when the mean `|r|` of its blocks is 0.03 or more.  When the kernels differ, it uses the size of the mean `r`.  Undecided is not "no loss" and not a gain: run it again in a later session.
* If the browser version changes during a comparison, the output says so.  Run the comparison again in full.
* Remove the worktree after the comparison with `git worktree remove`.

## A kernel lands only with a shown gain

The rule for a new matmul kernel (M4 of `plans/active/gpu-speed.md`, Step 7).  M5 and M6 use it too.

* Build the head page with the candidate.  Run one set of pages, in the order Safari, Chrome, Safari, Chrome: `cargo run -p xtask -- e2e bench --browser <b> --query "bench=v64,<candidate>&bench_rounds=5" --dist app/dist --out <dir>`.  Add the kernel that calibration chooses at the base, if it is not `v64`.
* For each page, weight format and kind (large, small): `R` is the candidate's calibration total over the reference kernel's.  The reference kernel is the kernel that calibration chooses for that kind at the base, in that browser.  A calibration total is the sum over calibration's shapes of that kind of `per_chunk x flops / rate`.  `S` is the largest spread of the bench lines that go into `R`.  Give the ratio of each shape too.
* A gain on a kind in a browser: `R < 1` and `1 - R > S`, in both pages.  A loss: `R > 1 + S` in both pages.  Else no effect is seen.
* The candidate lands only with a gain on a kind, with f16 weights, in at least one browser.  Then each browser's same-session comparison must show no loss, and each browser that chose it must have a gain for it.
* An attention kernel (M5): run `e2e bench --query "bench=v64&bench_rounds=5&mm=<pins>&attn=fused"` in the same order.  `R` is the candidate's time for each chunk over the reference policy's, from the line `bench: time axis, whole steps with a wait after each`, as calibration times them.  The reference policies are each policy that calibration chooses at the base in that browser.  `S` is the largest spread of the figures in `R`.  The attention-only lines are evidence only.  The rule of a gain and a loss is the same as for a matmul kernel.

## Each calibration choice is not slower

The check of the goal "Each calibration choice is not slower", for one page and one browser (M5 of `plans/active/gpu-speed.md`).  M6 uses it too.

* A policy is a time-axis attention policy, by the name in the `kernels:` line: `fused`, `freq` or `vfused`.  The check never forces `split` or `time`, which split the frequency axis.
* The ratio at the flip point of a calibration is the faster fused kernel's time over its split time, from its `kernels for this GPU` line.  The flip point is `FUSED_MARGIN`.
* For the two fused kernels, the ratio is `vfused`'s time over `fused`'s, and the flip point is 1.
* A policy is choosable when 12 or more calibrations of the page in that browser show either of these:
  * calibration chose it at least once;
  * the mean ratio at the flip point, plus or minus 2 standard deviations, crosses the flip point.
* The pins: the matmul policy that the page's calibration chooses most often in that browser, from the `kernels:` lines of its runs.  Check a second matmul policy only with a reason in the plan's Decision log.
* For each unordered pair `{p, q}` of policies where `p` is choosable, run one comparison with the page on both sides: `e2e perf --base-dist <page> --dist <page> --query "mm=<pins>&attn=<q>" --head-query "attn=<p>" --runs 4`.  One comparison gives both signs: a loss of the head is a gain of the base.
* The check fails when a choosable policy is slower than another policy: a loss when it is the head, a gain when it is the base.  Else it passes.
* An undecided comparison runs again in a later session.  Twice undecided fails the check, unless the plan's Decision log gives a reason.

## Known local failures

Check `main` before treating any of these as a regression:

* Safari keeps audio suspended while macOS reports the user idle (`references/browser-quirks.md`), so `e2e smoke` and `audio-resume` fail on any commit then.  The `caffeinate -u` of `web-test` and `e2e` is to stop this, but nobody saw it with an idle user (TD-96).
* In `e2e storage`, the `?model_url=` download case sometimes timed out in Safari 27.0.  It passed in each of 7 runs in Safari 27.2 on 2026-11-11.
* On 2026-11-11, before step 11 of M2, `e2e save` failed its check of the page's WebAssembly memory in Safari in 1 of 6 runs, with a bound of 1 MiB.  The bound is 3 MiB since then, from the spread measured in step 11 of M2.
* On the M2 Pro, `e2e weights --weights <the f16 model>` fails one check, "part 1: the f32 stems have M3's six hashes", in Chrome and in Safari (M5.1 of `plans/active/gpu-speed.md`, runs c2 and s3 on 2026-11-14).  Each other check of the scenario passes.  Of the six 16-bit WAV hashes of part 1, vocals (`f1b79d87...`) and piano (`3166e0de...`) equal `M3_HASHES` in `xtask/src/e2e/weights.rs`.  Bass (`55b981ea...`), drums (`7f0b47d6...`), other (`6f97ecca...`) and guitar (`87fc0cc2...`) differ.  The probable cause: `M3_HASHES` come from the f32 model file, and this Mac has only the f16 file.  Part 1 loads that file as f32, so its weights are the f16 values, and the stems differ.  Run c2 was at `71e0e5b`, whose diff from `8b16df7` changes only docs and `xtask/src/{drivers,web_test,webdriver}.rs`.  So the check also fails at `8b16df7`.  To pass it, give the f32 model file.
* In a Safari `web-test`, each audio context of the test page stays suspended, so 5 tests skip: the 3 busy-page tests of `app/tests/web_output.rs`, and 2 tests of `app/src/streamtest.rs`.  They skip alone too, so no earlier test causes it.  `wasm-bindgen-test-runner` gives `safaridriver` no capabilities, so Safari's autoplay rule holds the audio back (TD-96).  Before each skip, the test checks that a new audio context of the page stays suspended too.  To see the skip lines, run the command with `-- --nocapture` that TD-96 gives.  It keeps the time limit of `web-test`.  Chrome runs these tests.

## Downloads in tests

A test that clicks a download link saves the file in `~/Downloads`: headless Chrome downloads it.  Spy on the click, as `sv_spy_links` in `app/src/save_tests.rs` and `xtask/e2e/save_spy.js` do.  An offer of a save file is async, so keep the spy on until no offer is in flight.

## Running a single browser test

`wasm-bindgen-test-runner` breaks under `cargo test -q`; run it without `-q`.

## The model

`e2e selftest`, `separate`, `worker-check`, `memory-budget`, `stored-stems` and `weights` need the model weights (`--weights <path>`), which are never in the repo (ASSET-1).  The PyTorch reference fixtures from `tools/roformer_export.py` are not on either Mac, so the reference tests skip, and `STEMYLA_REQUIRE=fixtures` makes them fail (TD-8).

## Memory on an iPhone

Checked on 2026-10-25 with an iPhone 17 Pro on iOS 26, connected by USB, and libimobiledevice from Homebrew (`brew install libimobiledevice`).

* iOS stops a Safari tab at 2048 MB, GPU buffers included (MEM-4).  The tab reloads with no error that the page can catch.
* `idevicesyslog --no-colors -o <file>` writes the device log.  A kill shows as `memorystatus: com.apple.WebKit.WebContent [<pid>] exceeded mem limit: ActiveHard 2048 MB (fatal)`.  The log grows by about 35 MB a minute, so stop it after the run.
* `idevicecrashreport -k -e -f JetsamEvent <folder>` copies the JetsamEvent reports and keeps them on the device.  A report gives each process's `rpages` (pages charged to it) and `physicalPages` (its own pages).  iOS does not always write a report for one process over its limit.
* Web Inspector's memory timeline does not count all the GPU memory that iOS charges to the tab, so it reads low.  Its exported recording (Timelines, Export) holds the heap snapshots as JSON.  Each node is `[id, size, class, flags]`.  The largest `ArrayBuffer`s are the WebAssembly memories.
* A heap snapshot covers every page in the WebContent process.  Count the `Window` nodes with a `wasmBindings` property to count the app's instances.

