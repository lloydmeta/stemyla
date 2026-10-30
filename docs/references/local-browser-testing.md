# Local browser testing

Running `xtask web-test` and `xtask e2e` on the maintainer's Mac (Intel, macOS 26.7.1, Safari 27.0), as of 2026-10-19.  The commands and their prerequisites are in `README.md` (browser tests); this is what that machine adds.

## Which browser

* Safari is the local default.  `safaridriver` failed every session for part of 2026-10-18 ("The session timed out while connecting to a Safari instance") and worked again later that day.  Try Safari first.  If a session times out, quit Safari and retry: on 2026-10-22, sessions timed out while Safari was open, also outside the sandbox, and worked once it was quit.  If they still time out, use `--browser chrome` and say in the PR that Safari is unverified.
* A probe that leaves its WebDriver session open makes later sessions fail with "already paired with another WebDriver session".  End stale sessions before retrying.
* Chrome needs a Chrome for Testing chromedriver of Chrome's major version, for mac-x64, in `CHROMEDRIVER` ([Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/)).  Keep it outside the repo and reuse it rather than downloading it again each session.  Chrome updates itself: on 2026-10-29 it went from 155 to 156 during a run, and chromedriver 155 still drove it, with a warning ("has not been tested with Chrome version 156").

## Known local failures

Check `main` before treating any of these as a regression:

* Safari keeps audio suspended while macOS reports the user idle (`references/browser-quirks.md`), so `e2e smoke` and `audio-resume` fail on any commit then.
* In `e2e storage`, Safari fails two cases Chrome passes: the stalled-download case gets WebKit's "AbortError: Fetch is aborted" instead of the abort reason, and the `?model_url=` download case sometimes times out.
* In a full Safari `web-test`, `an_output_left_behind_by_a_busy_page_is_replaced` logs "skipped: the audio context is suspended" because the test before it leaves the context suspended; run it alone with `--nocapture` to see it run.  `a_stored_file_removed_after_it_was_opened_is_a_read_error` is flaky in Safari; Chrome passes it.

## Downloads in tests

A test that clicks a download link saves the file in `~/Downloads`: headless Chrome downloads it.  Spy on the click, as `sv_spy_links` in `app/src/save_tests.rs` and `xtask/e2e/save_spy.js` do.  An offer of a save file is async, so keep the spy on until no offer is in flight.

## Running a single browser test

`wasm-bindgen-test-runner` breaks under `cargo test -q`; run it without `-q`.

## The model

`e2e selftest`, `separate`, `worker-check`, `memory-budget`, `stored-stems` and `weights` need the model weights (`--weights <path>`), which are never in the repo (ASSET-1).  The PyTorch reference fixtures from `tools/roformer_export.py` are not on this machine, so the reference tests skip, and `STEMYLA_REQUIRE=fixtures` makes them fail (TD-8).

## Memory on an iPhone

Checked on 2026-10-25 with an iPhone 17 Pro on iOS 26, connected by USB, and libimobiledevice from Homebrew (`brew install libimobiledevice`).

* iOS stops a Safari tab at 2048 MB, GPU buffers included (MEM-4).  The tab reloads with no error that the page can catch.
* `idevicesyslog --no-colors -o <file>` writes the device log.  A kill shows as `memorystatus: com.apple.WebKit.WebContent [<pid>] exceeded mem limit: ActiveHard 2048 MB (fatal)`.  The log grows by about 35 MB a minute, so stop it after the run.
* `idevicecrashreport -k -e -f JetsamEvent <folder>` copies the JetsamEvent reports and keeps them on the device.  A report gives each process's `rpages` (pages charged to it) and `physicalPages` (its own pages).  iOS does not always write a report for one process over its limit.
* Web Inspector's memory timeline does not count all the GPU memory that iOS charges to the tab, so it reads low.  Its exported recording (Timelines, Export) holds the heap snapshots as JSON.  Each node is `[id, size, class, flags]`.  The largest `ArrayBuffer`s are the WebAssembly memories.
* A heap snapshot covers every page in the WebContent process.  Count the `Window` nodes with a `wasmBindings` property to count the app's instances.

