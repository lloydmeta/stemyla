# Local browser testing

Running `xtask web-test` and `xtask e2e` on the maintainer's Mac (Intel, macOS 26.7.1, Safari 27.0), as of 2026-10-19.  Since 2026-11-10, the maintainer also runs them on an Apple silicon Mac (arm64).  The commands and their prerequisites are in `README.md` (browser tests); this is what those machines add.

## Which browser

* Safari is the local default.  `safaridriver` starts no session while the screen is locked: it waits, then gives "The session timed out while connecting to a Safari instance" (2026-11-11, TD-96).  So `web-test` and `e2e` with Safari on macOS read the lock first, and fail at once with a clear error.  The check is `ioreg -n Root -d1 -a | grep -A1 CGSSessionScreenIsLocked`, which gives nothing when the screen is unlocked.  Do not quit Safari to clear a timeout.  If sessions time out with the screen unlocked, use `--browser chrome`, and say in the PR that Safari is unverified.
* In Safari, `web-test` sets `WASM_BINDGEN_NO_DEBUG=1` for the crate run of `stem-worker` only, and prints why (TD-93).  To run that crate by hand in Safari, set it too.
* A probe that leaves its WebDriver session open makes later sessions fail with "already paired with another WebDriver session".  `web-test` and `e2e` with Safari end each `safaridriver` whose parent is gone before they start.  If a session does not start, they end such drivers, and try the session start once more.  They try no test or scenario again (TD-96).  End a probe's session yourself.
* `web-test` and `e2e` with Safari hold `caffeinate -u` while they run, so macOS does not report the user idle (TD-96).
* Chrome needs a Chrome for Testing chromedriver of Chrome's major version, in `CHROMEDRIVER`.  Take the build for the CPU of the Mac: `mac-x64` for an Intel Mac, `mac-arm64` for Apple silicon ([Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/)).  Keep it outside the repo and reuse it rather than downloading it again each session.  Chrome updates itself: on 2026-10-29 it went from 155 to 156 during a run, and chromedriver 155 still drove it, with a warning ("has not been tested with Chrome version 156").

## Known local failures

Check `main` before treating any of these as a regression:

* Safari keeps audio suspended while macOS reports the user idle (`references/browser-quirks.md`), so `e2e smoke` and `audio-resume` fail on any commit then.  The `caffeinate -u` of `web-test` and `e2e` is to stop this, but nobody saw it with an idle user (TD-96).
* In `e2e storage`, the `?model_url=` download case sometimes timed out in Safari 27.0.  It passed in each of 7 runs in Safari 27.2 on 2026-11-11.
* `e2e save` can fail its check of the page's WebAssembly memory in Safari: 1 of 6 runs on 2026-11-11 (TD-97).
* In a full Safari `web-test`, `an_output_left_behind_by_a_busy_page_is_replaced` logs "skipped: the audio context is suspended" because the test before it leaves the context suspended; run it alone with `--nocapture` to see it run.

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

