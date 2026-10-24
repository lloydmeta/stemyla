# Songs at 48 kHz and other standard rates

## Purpose

Today the app refuses to separate a song that is not at 44.1 kHz.  After this plan, the app converts each decoded song to 44.1 kHz when it opens the song.  Then a 48 kHz song, and a song at any other standard rate, separates as a 44.1 kHz song does.  A song at 44.1 kHz does not change: the app does not copy or convert it.

The separation model takes only 44.1 kHz.  Its config gives `"sample_rate": 44100`, `"stft_n_fft": 2048`, `"stft_hop_length": 512` and fixed bin counts for each band ([config](https://huggingface.co/lumabeat/bs-roformer-sw/raw/main/bs_roformer_sw_config.json), checked 2026-10-24).  At 48 kHz each bin is 23.4 Hz wide, not 21.5 Hz, so every band moves up by 8.8 %.  Thus the samples must be converted, not only labelled again.

## Context and orientation

The state on 2026-10-24, at commit `62c50ab` on branch `feature/48k-audio`:

* `app/src/lib.rs`: `SEPARATION_RATE` (44,100).  The Separate button refuses a song at another rate ("Separation needs 44.1 kHz audio").
* `engine/src/song.rs`: `Song::from_audio` converts each stem to the rate of the first stem, with `resample_linear`.  If a stem is at that rate, it keeps the stem's `Vec` (no copy).  `Song::load_files` decodes files and calls `from_audio`.  `Samples::from(Vec)` puts the `Vec` in an `Arc` and does not copy it.
* `app/src/lib.rs` `load_and_keep` (a song that the user opens) and `app/src/store.rs` `rebuild` (a stored session that the user opens again) both call `Song::load_files`.  `rebuild` then adds the stored stems with `with_original`, which takes the frame count of the source.  Browser storage keeps the source file as it was, at its own rate.
* `roformer/src/lib.rs`: `SAMPLE_RATE` (44,100).  `xtask/src/roformer_cli.rs` refuses other rates.  This plan does not change the CLI.
* `app/tests/web.rs`: the storage tests use `RATE = 8_000` and 800 frames for the source and the stems.
* `xtask/src/e2e.rs` `clip`: writes 2 s of the demo mix at 44.1 kHz for the `separate` scenario.

Terms:

* Rate: samples per second for each channel.
* Standard rate: a rate from 8 kHz to 384 kHz whose ratio to 44,100, in lowest terms L / M, has L of 441 or less.  This includes 8, 11.025, 12, 16, 22.05, 24, 32, 48, 88.2, 96, 176.4 and 192 kHz.
* Tap: one input sample that the filter weights and adds to make one output sample.

## Design

Decision 0012 records the choice: every song in the app is at 44.1 kHz.  The conversion is on the page thread, in `Song::from_audio`, where decoding already runs.

```
file at 8 to 384 kHz ------> decode --> engine::resample::to_rate --> Song at 44.1 kHz --> (no change after this)
file at 44.1 kHz ----------> decode --> the same Vec (no copy) -----> Song at 44.1 kHz
```

The resampler (`engine/src/resample.rs`):

* `pub const SONG_RATE: u32 = 44_100`.
* `pub fn to_rate(x: Vec<f32>, from: u32, to: u32) -> Result<Vec<f32>>`.  At `from == to` it returns `x` as it is.  Otherwise it makes a new `Vec` and drops `x`.  It refuses a rate outside 8 kHz to 384 kHz (review round 1).  It also refuses a rate whose L is more than 441.  A header can state any rate, and the table and the output grow with the rate.
* Rational polyphase with a Blackman-windowed sinc.  For 48 kHz to 44.1 kHz, L = 147 and M = 160.  Output frame `i` reads input frame `j = (i * M) / L` with table row `p = (i * M) % L`.  `Position::next` steps `j` and `p` from one output to the next, and `output_len` computes `ceil(n * L / M)` in `u64`.  So nothing overflows a 32-bit `usize` on wasm32, and the output does not drift.
* When the rate falls, the cutoff is half the output rate.  The half-width is `32 * from / 48 kHz` input frames, rounded up to an even number and at least 32: 64 taps at 48 kHz, 88 at 64 kHz, 256 at 192 kHz.  So the transition is about 4.1 kHz wide at every input rate.
* When the rate rises, the stop band starts at half the input rate, so no image of the input's band falls below it.  The half-width is 64 input frames, so the pass band ends at 45.7 % of the input rate, 14.6 kHz from 32 kHz (review round 1).
* Each table row is scaled to sum to 1.  The inner loop uses four accumulators.  Input outside the song counts as silence.

A scratch prototype (not in the repo) measured this filter on 2026-10-24.  From 48 kHz, the gain is within 0.002 dB up to 20 kHz.  Against an ideal sine, the SNR is 98.5 dB or more up to 18 kHz, 93.5 dB at 19.1 kHz and 89.9 dB at 20 kHz.  It swings by up to 13.4 dB between tones 500 Hz apart (review round 1, a 500 Hz sweep).  From 16 kHz to 32 kHz, the image of a tone near half the input rate is 60 dB or more below the input level.  For a tone in the pass band it is 72 dB or more below (review round 2).  A 23 kHz tone becomes a 21.1 kHz alias at -21.4 dB, above hearing.  This is the trade-off of SoX's `rate -a`, which allows aliasing above the pass band.  A 5-minute stereo song took 0.78 s native and 1.33 s in WebAssembly on V8.

Memory (MEM-1, MEM-4): `from_audio` converts one channel at a time and drops its input first.  For a 5-minute 48 kHz song, the peak during conversion is about 168 MB, against 741 MB when 7 separated lanes show.  The page reuses the space that it frees, so the high-water mark does not change.

Not taken:

* Conversion of the separation input only: stems and the original lane would then have different rates.
* Conversion in a worker: `stem-worker` cannot take `engine` (LAYER-1, decision 0005).  Also, a worker's WebAssembly memory never shrinks, so the space would stay unused.  Decoding stays on the page, so the page would still freeze.
* `rubato`: five major versions in 2026 and six new crates (DEPS-1), for about 80 lines of code that tests can check fully.
* The browser's resampler (`OfflineAudioContext`): web only, different in each browser, and native tests cannot check it.

## Plan of work

One PR, one signed commit for each milestone, then the fix commits of review rounds 1, 2 and 3.

### M0: this plan and decision 0012

Acceptance: `cargo run -p xtask -- docs-check` passes with the plan and the decision in the index.

### M1: `engine::resample`

Add the module with `to_rate` and `SONG_RATE`.  No caller changes.  Unit tests, in `cargo test -p engine`:

* `a_second_at_48k_is_a_second_at_44k`: 48,000 frames give 44,100.
* `the_pass_band_is_flat_to_20_khz`: tones every 500 Hz from 100 Hz to 20 kHz at 48 kHz keep their gain within 0.01 dB.  The SNR against an ideal sine is 95 dB or more up to 18 kHz, and 85 dB or more to 20 kHz.
* `an_alias_stays_above_20_khz`: a 23 kHz tone gives a 21.1 kHz alias at -20 dB or less.
* `upsampling_from_32k_keeps_a_tone`: frame count, and gain within 0.01 dB and SNR of 95 dB or more for 1, 10 and 14 kHz.
* `a_large_fall_keeps_the_pass_band` (192 kHz) and `every_standard_rate_converts`: in M1.  Review round 1 extended `a_large_fall_keeps_the_pass_band` to 64, 88 and 384 kHz.
* `a_rising_rate_makes_no_audible_image`, `input_outside_the_song_counts_as_silence`, `the_output_is_not_delayed`, `the_length_and_the_position_do_not_overflow_32_bits` and `a_rate_outside_8_to_384_khz_is_refused`: added in review round 1.  `the_same_input_gives_the_same_output` was removed, since no change to this deterministic code could make it fail.
* `the_same_rate_is_not_copied`: the output pointer is the input pointer.
* `an_odd_rate_is_refused`: 44,056 Hz gives an error.

### M2: songs open at 44.1 kHz

* `Song::from_audio` converts each stem to `SONG_RATE` with `to_rate`, and still keeps a stem at that rate as it is.  Remove `resample_linear`.
* `app/src/lib.rs`: `SEPARATION_RATE` becomes `engine::resample::SONG_RATE`.  The guard stays.
* `xtask/tests/`: `the_song_rate_is_the_model_rate` asserts `SONG_RATE == roformer::SAMPLE_RATE`.
* Engine tests: a 48 kHz WAV opens as a 44.1 kHz song with 44,100 frames for each second.  Stems at mixed rates all open at 44.1 kHz.  `a_44k_stem_is_not_copied` builds an `Audio` at 44.1 kHz and asserts that the stem has the same pointer and the same bits.
* `app/tests/web.rs`: `RATE` becomes 44,100 and the frame counts change with it, because an 8 kHz source now opens at 44.1 kHz.
* DOC-1: `README.md` (which songs open), the `engine` line and the flowchart in `docs/ARCHITECTURE.md`.

Acceptance: the tests above fail before the change and pass after it, and all the checks of GIT-2 pass.

### M3: a stored 48 kHz session opens again

`app/tests/web.rs`: `a_48k_session_reopens_with_its_stems`.  Store a 48 kHz source, store stems with the frame count of the converted song, and open the session again.  Every lane has the same frame count, and the song is at 44.1 kHz.  Run `cargo run -p xtask -- web-test`.

A session stored before this change can hold a source at mixed rates with stems.  The old `from_audio` opened a drop at the first stem's rate, and floored the length of the others.  Such a source can now open one frame longer than its stems.  So `rebuild` fits each stored stem to the source (review round 2, `stems_that_do_not_fit_the_source_are_fitted_to_it`).

### M4: the end-to-end separation of a 48 kHz clip

`xtask/src/e2e.rs` `clip`: convert the demo mix to 48 kHz with `to_rate` and write that WAV.  The page logs the decode and conversion times.  Run `cargo run -p xtask -- e2e separate` with the weights (`docs/references/local-browser-testing.md`).  Acceptance: the scenario passes with 6 stems.  Write the times in Progress.  Then move this plan to `docs/plans/completed/`.

### Review

Run `cargo build -p xtask`, then `/review-rounds` for rounds 1, 2 and 3 over the PR's commits.

## Validation and acceptance

* `cargo fmt --all --check`, each clippy command in `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `cargo run -p xtask -- docs-check` pass.
* `cargo run -p xtask -- web-test` passes, with `a_48k_session_reopens_with_its_stems`.
* `cargo run -p xtask -- e2e separate` passes on the 48 kHz clip.

## Idempotence and recovery

* Each milestone is one signed commit.  To undo one, revert it.
* To resume, read Progress, then `git log --oneline 62c50ab..HEAD`.
* Browser storage keeps source files as they were.  A revert of M2 opens them at their own rate again.

## Progress

* [x] 2026-10-24: design accepted by the maintainer (decision 0012).
* [x] 2026-10-24: M0: plan and decision 0012.  fmt, clippy, the workspace tests and `docs-check` pass.
* [x] 2026-10-24: M1: `engine::resample`, 9 unit tests.  In WebAssembly on V8, `to_rate` converts 5 minutes of 48 kHz stereo in 1.37 s.
* [x] 2026-10-24: M2: songs open at 44.1 kHz.  On the old `from_audio` (no conversion, the first stem's rate), `a_48k_wav_opens_at_44k`, `stems_at_mixed_rates_all_open_at_44k` and `a_stem_at_an_odd_rate_is_refused` fail; after M2 they pass.  `a_44k_stem_is_not_copied` passes before and after (it guards MEM-1).  The 8 kHz fixtures in `app/tests/web.rs`, `app/tests/web_close.rs` and `app/src/store.rs` are now at 44.1 kHz.  All checks pass, and `web-test` passes in Chrome (69 tests).  Safari refused the WebDriver session ("webdriver POST /session request failed"), so no test ran there.
* [x] 2026-10-24: M3: `a_48k_session_reopens_with_its_stems` passes in Chrome (`web-test --browser chrome`, 70 tests).  Before M2 it fails its first assertion, as the source opened at 48 kHz.  The reopened original lane has the same samples as the first open.
* [x] 2026-10-24: M4: `e2e separate --browser chrome` passes on a 2 s 48 kHz clip.  The page logged "song: decoded and converted to 44100 Hz in 0.03 s (88200 frames)", and six stems arrived in 1.7 s (Radeon Pro 5500M), with no page errors.  Safari was not tried, because its WebDriver session failed in M2.  After review round 1 renamed the log line, `e2e separate` passed again with "song: opened at 44100 Hz in 0.02 s (88200 frames)".
* [ ] Review rounds 1, 2 and 3.

## Surprises and discoveries

* 2026-10-24: DSP-1 allows only the methods `len`, `push` and `capacity`, so `dsp` cannot calculate `sin` or `cos`.  The resampler goes in `engine`.
* 2026-10-24: the storage tests in `app/tests/web.rs` store 8 kHz sources with stems of the same frame count.  At 44.1 kHz the source has more frames than the stems, so the fixture changes in M2.

* 2026-10-24, M1: on wasm32 `usize` has 32 bits, so `i * M` overflows after about 10 minutes at 48 kHz (2.5 minutes at 192 kHz).  `apply` steps the input position and the phase instead, and computes the length in `u64`.
* 2026-10-24, M1: the engine tests in `engine/src/song.rs` use rates such as 10 Hz and 100 Hz.  `to_rate` refuses those, so M2 changes the tests to 44.1 kHz.

## Decision log

* 2026-10-24, maintainer: every standard rate converts, not only 48 kHz.
* 2026-10-24, maintainer: a download of the "original" lane is at 44.1 kHz, not at the rate of the source file.
* 2026-10-24, maintainer: the conversion runs on the page thread.
* 2026-10-24, maintainer, on the agent's recommendation: 64 taps at 48 kHz.  96 taps would put the 23 kHz alias at -36 dB for about 1.5 times the time.
* 2026-10-24, maintainer: one PR with one commit for each milestone.
* 2026-10-24, agent: the page logs one time for decoding and conversion together, from `load_and_keep`.  `app` cannot time the two apart without timing inside `engine`, which has no logger.  M1 measured the conversion alone (1.37 s for 5 minutes of 48 kHz stereo).
* 2026-10-24, agent: the plan moves to `completed/` after the review rounds, not at the end of M4, since the rounds can still change it.

## Outcomes and retrospective

Not yet written.
