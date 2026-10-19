# Local browser testing

Running `xtask web-test` and `xtask e2e` on the maintainer's Mac (Intel, macOS 26.7.1, Safari 27.0), as of 2026-10-19.  The commands and their prerequisites are in `README.md` (browser tests); this is what that machine adds.

## Which browser

* Safari is the local default.  `safaridriver` failed every session for part of 2026-10-18 ("The session timed out while connecting to a Safari instance") and worked again later that day.  Try Safari first; if a session times out, use `--browser chrome` and say in the PR that Safari is unverified.
* A probe that leaves its WebDriver session open makes later sessions fail with "already paired with another WebDriver session".  End stale sessions before retrying.
* Chrome needs a Chrome for Testing chromedriver of Chrome's major version, for mac-x64, in `CHROMEDRIVER` ([Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/)).  Keep it outside the repo and reuse it rather than downloading it again each session.

## Known local failures

Check `main` before treating any of these as a regression:

* Safari keeps audio suspended while macOS reports the user idle (`references/browser-quirks.md`), so `e2e smoke` and `audio-resume` fail on any commit then.
* In `e2e storage`, Safari fails two cases Chrome passes: the stalled-download case gets WebKit's "AbortError: Fetch is aborted" instead of the abort reason, and the `?model_url=` download case sometimes times out.
* In a full Safari `web-test`, `an_output_left_behind_by_a_busy_page_is_replaced` logs "skipped: the audio context is suspended" because the test before it leaves the context suspended; run it alone with `--nocapture` to see it run.  `a_stored_file_removed_after_it_was_opened_is_a_read_error` is flaky in Safari; Chrome passes it.

## Running a single browser test

`wasm-bindgen-test-runner` breaks under `cargo test -q`; run it without `-q`.

## The model

`e2e selftest`, `separate` and `worker-check` need the model weights (`--weights <path>`), which are never in the repo (ASSET-1).  The PyTorch reference fixtures from `tools/roformer_export.py` are not on this machine, so the reference tests skip, and `STEMYLA_REQUIRE=fixtures` makes them fail (TD-8).
