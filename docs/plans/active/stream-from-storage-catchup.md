# Stream from storage, catch-up after M7

## Purpose

This is the plan of the catch-up between M7 and M8 of `plans/active/stream-from-storage.md` (the parent plan).  Its scope comes from the parent's Decision log entries of 2026-10-27 and 2026-10-28.  After the catch-up:

* M3 to M6 each have a review round 4 on their round 3 fixes, and more rounds to the stop rule.
* Each confirmed should-fix that M3 to M6 left as debt (mostly TD-39, and TD-56) is fixed, or is kept with a reason per item.
* No route decodes a song on the page that is longer than the memory path allows (TD-58, decision 0015).
* `.cargo/mutants.toml` has no `exclude_re` list of wasm32-only functions.  Each such function has `#[cfg_attr(mutants, mutants::skip)]` (TD-33).
* The stem worker's extra memory in Safari is explained, and removed if it is a needless buffer (MEM-1, MEM-4).
* The browser's audio output has buffers of 3,072 frames, 1.5 times today's (the maintainer's choice).  One buffer is about 69.7 ms at 44.1 kHz, and cpal queues two, about 139 ms.

## Context and orientation

The state at the start, on 2026-10-28, at commit `d420932` on the branch `iphone-crashes-2026-10-25` (PR #39).  CI is green on it.  The parent plan's "The code today" and "Terms", and the plans of M3 to M7 (`stream-from-storage-m3.md` to `-m7.md`), apply.  The review logs of M3 to M7 are `docs/reviews/iphone-crashes-2026-10-25-m<n>.md`.

* Round 3 fixes of each milestone, which no round checked: M3 `860cb04`, M4 `87b3d30`, M5 `05905c2`, M6 `817258b`.
* Deferred confirmed should-fix (verdict confirmed or pre-existing, action not a fix), counted from the logs on 2026-10-28:
  * M3: 1-47, 1-80, 1-99, 1-103, 1-139, 2-30, 2-41, 2-49, 2-58.  Each is a missed mutant in an e2e scenario (`memory_budget`, `separate`, `stored_stems`, `run` of `xtask/src/e2e.rs`).
  * M4: 1-30, 1-35, 1-41, 1-53 (`memory_budget`, `roformer_cli::run`, `separate`, `stored_stems`).
  * M5: 1-29, 1-68, 1-72 (`drop_and_open` and `run` of `xtask/src/e2e/decode.rs`, `memory_budget`), and 3-2 (TD-56, which M7 closed).
  * M6: 1-27 (`StemApp::log_audio_stats`), 1-29 (`run` of `xtask/src/e2e.rs`), 1-40 (`main` of `app/src/main.rs`), 2-14 (`smoke`).
  * M3 1-2 and M4 1-1 concern the parent plan's text, which only the orchestrator edits.  The team checks that the parent plan now says what the code does, and records the result.
* The page-decode routes (TD-58): `open_on_the_page` (`app/src/lib.rs`), for a drop or a pick, reads each chosen file whole (`picker::read`) and calls `Song::load_files`.  `on_the_page_of` calls `store::restore_id`, which decodes the stored sources whole (`rebuild`, `app/src/store.rs`).  Each `Decode::OnThePage` leads to one of them: no session or no storage; a decode worker that cannot start, stops or replies `decode-unstored`; and `Kept::MixBusy` (`store::keep_decoded`).  D2 of the M7 plan (more than about 20 sources, `store_meta::fits_read_ahead`) leads to them directly.  The only bound is `engine::decode::MAX_DECODED_BYTES` (decision 0013).  `MEMORY_PATH_MAX_FRAMES` (`app/src/lib.rs`, 100 s at 44.1 kHz) bounds a separation on the memory path, classic drums' route (`classic_route`) and an incomplete stem (`stem_lanes`), not an open.
* `.cargo/mutants.toml` has `exclude_globs` (whole files that compile for wasm32 only) and `exclude_re` (wasm32-only functions in shared files).  cargo-mutants 27.1.0 does not apply `exclude_re` to a mutant that deletes a field from a struct expression (TD-33).  It honours `#[cfg_attr(mutants, mutants::skip)]`, which needs no crate (M7 review 5-2).
* The stem worker's memory: the worker logs `memory: ...` after each chunk.  In the maintainer's Safari run on the Mac (2026-10-28, a 3.5-minute song, 34 chunks), worker wasm was 57 MB at the start, 133 MB after chunk 1, and 170 MB from chunk 2 to the end.  Chrome's e2e (M4, M6, M7) shows 57 MB at the start, the end and the peak.
* The audio output: `engine/src/live.rs` opens cpal's output with `BufferSize::Default`.  cpal 0.18.2's Web Audio host then uses 2,048 frames (`DEFAULT_BUFFER_SIZE` in `src/host/webaudio/mod.rs`), and accepts `BufferSize::Fixed(n)` for any `n` from 1 (`SupportedBufferSize::Range { min: 1, max: FrameCount::MAX }`).  It queues two buffers and calls the callback on the main thread.  `plays_alone` decides which late buffer to fill.  `engine/tests/realtime.rs` runs callbacks of 128, 512, 4,096 and 10,000 frames.  `app/src/streamtest.rs` copies cpal's schedule with `FRAMES_PER_BUFFER = 2048`.  `StemApp::audio_stats_lines` (since B; it was `log_audio_stats`) gives `audio stats: <n> callbacks of <frames> frames ...`.

## Plan of work

### How the team works

The team leader wrote this plan, has it reviewed, starts the agents, runs the review rounds and commits Progress.  Reviewers and validators are read only.  One implementer at a time edits the tree: a new `general-purpose` subagent for each of A (fixes), B, C, D, E (a fix, if any) and F, in that order; F is last.  E's measurements run in a detached git worktree under `target/`, so they do not touch the tree.  No agent starts further agents, except the reviewers that `review-rounds` starts.

Before each commit, the implementer runs the checks of GIT-2 (`CLAUDE.md`, Commands).  It commits signed (`git commit -S`), stages files by name, and does not push.  Each commit leaves the docs true (DOC-1).  Write each test first and see it fail, then write the code.  A workaround or a known gap gets a `docs/tech-debt.md` entry in the same commit.  No agent edits `CLAUDE.md`, `.claude/` or the parent plan.  The team leader sends a needed change to `CLAUDE.md` or `.claude/` to the orchestrator.  It lists in its report each sentence of the parent plan that a change makes wrong.

The stop rule, for every review loop: stop at the first review with no blocker and no significant should-fix.  Significant means that a step would fail as written, two parts contradict, a measure would mislead, or the implementer would do wrong work.

### Decisions of this plan

* D1, round 4 of an old milestone.  `review-rounds` reviews the branch since a base, so it cannot review one old commit.  So round 4 runs by hand, in the same form: a new `reviewer` on `<fix>^..<fix>`, with the milestone's plan as criteria and the round 3 claims that the commit fixes.  It checks that each fix is right and has a test that fails without it.  It also reads the code at `HEAD`, to see that later milestones did not undo a fix and that a fix did not break them.  A validator reverts a fix by hand to confirm a claim that its test does not fail.  A `validator` tries to disprove each blocker and should-fix.  A round 4 has no mutants run.  The reviewers and validators build `HEAD` in the target folder `target/review/catchup-a-target`.  The IDs are `4-<n>` in each milestone's own log, and the reviewer of M<n> names its worktrees `target/review/4-catchup-m<n>-...`.  The rows go into the milestone's log with the round `4`, the commit of the fixes and the action.  The rounds table gets a row with `-` for mutants.  If round 4 confirms a blocker or a significant should-fix, a round 5 reviews the fix commit, to the stop rule.
* D2, a deferred row.  Each row gets one of these actions:
  * Fixed, which comes first: its logic moves into a pure function with a native test, as M7 did for `old_session::memory_mb`.  Or a native test runs the code, as `xtask/tests/e2e_cli.rs` does: for example, a scenario run with no driver must fail before a browser starts, so a body replaced with `Ok(())` fails the test.  The new test must fail with the row's mutant; the implementer applies the mutant by hand and records the result.
  * Kept, only when no native test can kill the mutant: a scenario's top-level `run` that only a browser can drive stays, if CI's `browser-tests` job runs that scenario.  The row says which job and scenario run it.
  * Kept with a reason, only when no native test can kill the mutant: a body that needs the GPU and the model, which CI does not have, or an entry point that needs a window (`main` of `app/src/main.rs`).  Its checks become pure functions with tests, and the row says what is left.
  The action goes in a new row of the same log, with the round `catch-up` and the finding's ID.  TD-39 shrinks to what is left, with a reason for each item.
* D3, the page-decode limit.  The principle: the page never holds more decoded audio than a separation of one source on the memory path holds at its limit.  That separation holds `LANE_BUDGET` stereo lanes of `MEMORY_PATH_MAX_FRAMES` frames: the source and the 6 stems, so 7 (about 250 MB); the implementer confirms the count from the code.  One pure function, such as `memory_path_frames(memory_limit, lanes)`, gives the limit in frames: `min(memory_limit, LANE_BUDGET * memory_limit / lanes)`.  `memory_limit` is `MEMORY_PATH_MAX_FRAMES`, or less in a test.  A mono source counts as a full lane, which is conservative, since its two sides share one buffer.
  * `lanes` counts what the page holds in memory after a separation on the memory path: the sources in memory (all of them on a page-decode route; a stored source stays in OPFS), plus 1 for the mix input if it is not one of those, plus the 6 stems.  With 7: one source gets 100 s, two sources 77 s, a drop of 6 stem files (13 lanes) about 54 s, and 21 sources (28 lanes) 25 s.  So most songs on the D2 route of the M7 plan are refused, and a song of many sources gets a shorter memory path than today.  That is a lasting product choice, so the implementer writes it as decision 0020, `proposed`, and the team leader lists it for the orchestrator.
  * Every check of the memory path's length and its status texts use that one function, each with a native test: the page-decode routes, `separation_path`, the check after a failed mix write (`if frames <= self.memory_limit`), `after_unstored`, `classic_route`'s `ReadFirst`, the two `stem_lanes` calls (with `MEMORY_PATH_MAX_FRAMES` in `restore_stored`, and with `self.memory_limit` after a storage-path separation), the native `let limit = MEMORY_PATH_MAX_FRAMES` of the classic route and the status text of `ClassicRoute::TooLong`, `refusal_text` and `too_long_text`.  The implementer lists them in Progress.  A test covers a song of 21 sources (25 s with a budget of 7).
  * The limit is in frames at `SONG_RATE`, not in bytes at the source's rate.  `engine/src/decode.rs` already refuses from the header, else stops at a limit (`Decoder::within`, `passes`, `frames_within`), but in bytes at the source's rate, and privately.  `engine` gets a public entry point that takes a limit in frames at `SONG_RATE`: it scales by each source's rate from its header, or counts the frames that `decode_into` (`engine/src/song.rs`) gives.  Tests at 22.05 kHz and 48 kHz show the limit in song frames.
  * Before the whole read, each route reads the head of each file (as `head_of` does) and refuses a WAV or a FLAC whose header length passes the limit.  The session's `meta` gives the length when it has a `mix` or `decoded` line.  An MP3 states only an estimate, and a FLAC or a WAV can state no length.  So the route also refuses, before the read, any file whose header states no length and that is larger than a stated cap, and the decode stops at the limit.  The implementer sets the cap from the largest file that a song within the limit can be, with room for tags such as cover art, and records it.  So the page never holds much more than the limit.
  * The limit is a parameter, as `StemApp::memory_limit` is, so a browser test can use a short limit and needs no 100 s file.
  * The refusal's status line says that the song is too long to open in this browser now, gives the limit and the reason (for example, storage is full or unavailable), and says what to do (free browser storage, or open a shorter song).
  * `Kept::MixBusy` first.  The implementer reads why `keep_decoded` gives it, and what the decoded source files need.  The session can perhaps open from its stored sources while a separation holds the mix file, or the page can perhaps wait for the mix and keep the decode.  If one of these works, the implementer does it, and that trigger no longer decodes on the page.  Else MixBusy gets the limit like the other triggers.  The choice and its reason go in the Decision log.
* D4, `mutants::skip`.  Each function that `exclude_re` names gets `#[cfg_attr(mutants, mutants::skip)]`.  Where a pattern names an impl (`Inbox::`) or a module (`web_tests::`, `separation_tests::`), the attribute goes on the impl or the module; cargo-mutants 27.1.0 honours it there.  The workspace gets `check-cfg` for `cfg(mutants)` in `[workspace.lints.rust] unexpected_cfgs`, since each crate has `[lints] workspace = true`.  So the build has no warning.  The `exclude_globs` list of whole wasm32-only files stays, since it names files, not functions.  Check: `cargo mutants --list` (cargo-mutants 27.1.0) shows no mutant of these functions, struct-field deletions included, and the same number of mutants of every other function as before.  The implementer compares the two lists by function name, since the new attribute lines move the line numbers, and keeps the counts and the difference in Progress.  The `exclude_re` patterns are not anchored, so they may also hide native functions of the same name, and the attribute on the wasm32 variant of `base_latency` or `main` removes mutants that TD-33 says stay in.  The implementer explains each difference, and updates TD-33.  It does not add the attribute to `base_latency` or `main`, which have native variants.  Each new wasm32-only function of B, C, E and F gets the attribute too.  No crate is added (DEPS-1).
* D5, the Safari figure.  If the extra memory is a needless buffer, it goes (MEM-1), with a test where a native test or a browser test can see it.  If it is needed, `docs/ARCHITECTURE.md` (memory on phones) and this plan say what it is and how large, so M10 counts it.  The figure for M10 is the larger of Chrome's and Safari's.
* D6, the output buffer (the maintainer's size, 3,072 frames; see Decision log).  On wasm32, `live.rs` asks cpal for `BufferSize::Fixed(3072)`, from one named constant, so a later change is one line.  One buffer is then about 69.7 ms at 44.1 kHz, the queue of two about 139 ms plus the output latency.  The page sees a seek at the next callback, in at most about 70 ms (mean about 35 ms), not 46 ms (mean about 23 ms).  Mute, solo and tempo changes are heard up to about 139 ms later.  The implementer checks what depends on the length, such as `MAX_BEHIND` (250 ms) and the text "over two buffers" in `live.rs`, and records the result.  Native output keeps `BufferSize::Default`.  The player's scratch buffers and the RT-1 test cover callbacks of 3,072 frames: `CALLBACK_FRAMES` in `engine/tests/realtime.rs` gets 3,072, and the implementer checks every sizing that assumes 2,048.  `plays_alone` already takes the real length.  Its tests and the schedule simulation `run` in `live.rs` use `2_048.0 / 44_100.0`; they keep a case at 2,048, since native output keeps the default, and add cases at 3,072.  `CALLBACK` in `app/src/playback_tests.rs` (2,048, "46 ms") follows.  `?streamtest`'s `FRAMES_PER_BUFFER` becomes 3072.  The `audio stats:` line already names the callback's frames, but `audio_stats` in `xtask/src/e2e.rs` reads only the tempo and the playhead.  A pure parse of the frames gets a unit test, and `e2e playback` and `e2e audio-resume` fail unless the frames are 3,072.  The rough seek bound of decision 0017 grows from about 136 ms to about 159 ms, past the 150 ms seek p99 bar that M10 measures against by about 9 ms.  The docs that say 2,048 frames or 93 ms of queue change, except records of past measures.  Decision 0017 keeps its text and gets a dated note with the new figures.  If the orchestrator sends another size, the constant and these figures change.

### A: round 4 of M3 to M6

1. Start one `reviewer` for each milestone, as D1 says, in one message.
2. Start one `validator` for each blocker and should-fix, at most 15 at once.
3. One implementer fixes the confirmed findings with tests, in one commit "catch-up round 4 fixes", and appends the round 4 rows to each log.
4. If round 4 confirms a blocker or a significant should-fix, a round 5 reviews the fix commit, to the stop rule.

Acceptance: each log has its round 4 rows and a rounds row.  Each confirmed finding has a fix commit and a test that fails without the fix.

### B: the deferred should-fix

The implementer checks the list in "Context" against the logs, and handles each row as D2 says.  For M5 3-2, it names the M7 test that closes TD-56 and runs it.  It updates TD-39.

Acceptance: each listed row has a `catch-up` row with its action.  Each "fixed" row names its test and the mutant that the test now kills.  TD-39 lists only what is left, each with a reason.

### C: the page-decode limit (TD-58)

The implementer handles MixBusy first, then adds the limit to `open_on_the_page` and `on_the_page_of` (and `rebuild`), as D3 says.  Tests that fail first:

* native tests of `memory_path_frames` and of each check and status text that uses it;
* browser tests (`app/src/decode_tests.rs` or `app/src/store.rs`) that each route refuses a long song with the status line, and reads no more of the file than its header where D3 says so;
* a test of the MixBusy choice.

Acceptance: the tests pass; `web-test` passes in Chrome.  TD-58 is closed, or keeps only what remains.  TD-54's bullets that cite TD-58, and MEM-1's and MEM-4's gap lists, follow.

### D: `mutants::skip`

As D4 says.  Acceptance: the two `--list` counts in Progress, clippy and the build with no `unexpected_cfgs` warning, TD-33 and the comment in `.cargo/mutants.toml` updated.

### E: the stem worker's memory in Safari

A measurer works in its own worktree.  It runs `e2e memory-budget` or `separate` in Safari and in Chrome with the model.  It finds what allocates the extra memory, and proposes a fix if it is needless.  Then an implementer applies the fix (D5), or writes the docs.  Safari stays open; a timed-out session is reported, and the rest goes on.

Acceptance: the figures per chunk in both browsers, the cause by file and function, and the fix with Safari's figure after it, or the docs.

### F: the output buffer

As D6 says.  Acceptance: `cargo test -p engine` (the RT-1 test and `plays_alone`'s tests), `web-test` in Chrome (the streamtest test), and `e2e playback` and `e2e audio-resume` in Chrome, with `callbacks of 3072 frames` in the audio stats line, which the e2e checks read.

### G: review rounds

After A to F, `cargo build -p xtask`, then `/review-rounds` with the base `d420932` and this plan as criteria, rounds 1, 2 and 3, and more to the stop rule.  The mutants run starts first, and the reviewers start only after its baseline passes.  After the last round, the log goes to `docs/reviews/iphone-crashes-2026-10-25-catchup.md`, the run folder goes, and that is committed.

## Validation and acceptance

* `cargo fmt --all --check`, the clippy commands of `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib`, `cargo run -p xtask -- docs-check`.
* `cargo run -p xtask -- web-test --browser chrome`, `e2e playback` and `e2e audio-resume` in Chrome.
* `cargo mutants --list` before and after D.
* The Safari and Chrome worker figures of E.

## Idempotence and recovery

* Each part is one or more commits that can be reverted alone.  A, B and C touch the same files, so they run in that order.
* Round 4's target folder is `target/review/catchup-a-target`.  Remove it after round 4 and any round 5.
* E's worktree is `target/catchup-e`.  Remove it with `git worktree remove --force target/catchup-e` before the review rounds.
* A review round that stops leaves `target/review/run/iphone-crashes-2026-10-25/`.  Remove it only after its log is copied.

## Progress

* [x] 2026-10-28: plan written.  The round 4 reviewers of M3 to M6 and E's measurer started at the same time (see Decision log).
* [x] 2026-10-28: plan review round 1: 4 should-fix (D3's bound, the parent plan, D6's queue length, D1's test check) and 4 nits, fixed.
* [x] 2026-10-28: plan review round 2: 5 should-fix (D3's header limit and lane budget, D2's entry point, D1's target folder, F's check of the frames) and 3 nits, fixed.
* [x] 2026-10-28: plan review round 3: 4 should-fix (D3's limit in song frames, the read before the refusal, the lanes counted; D6's seek bound) and 4 nits, fixed.
* [x] 2026-10-28: plan review round 4: 2 should-fix (D3: one limit function for every memory-path check, and the worked figures with the lanes counted) and 3 nits, fixed.
* [x] 2026-10-28: plan review round 5: 2 should-fix (D3 counts only the lanes in memory, and the list of checks was not complete) and 2 nits, fixed.
* [x] 2026-10-28: plan review round 6: no blocker and no significant should-fix (the stop rule); 4 nits fixed.
* [x] 2026-10-28: A: round 4 of M3 to M6.  Confirmed: M3 4-1 and M4 4-1 and M5 4-1 (weak tests); pre-existing: M3 4-2 (a successful open did not say that held stems are not kept).  M6 had nits only.  All fixed with tests that fail first, and the nits fixed (`51e9a5e`, logs `806caa3`).  Round 5 on `51e9a5e` (one reviewer, every mutant run in Chrome): no findings, so the loop stops.
  * 2026-10-28: round 4 fixes in `51e9a5e`, and the round 4 rows in the logs of M3 to M6.  Each test failed first: M3 4-1 with `true` to `refusal_text`; M3 4-2 before the fix; M4 4-1 with `Stop::Failed` and with no end check; M5 4-1 with each of the five guards off (each fails one test); M5 4-2 before the fix.  web-test in Chrome: app lib 137, worker 68, all passed.  Workspace tests: 778 passed.
* [x] 2026-10-28: B: 21 deferred rows.  19 fixed with native tests that kill the row's mutant (no-driver runs of `separate`, `memory-budget`, `stored-stems`, `smoke` and `run`; `MODEL_RUN_SECS`; `holds_or_error`; `roformer_refuses_bad_input_before_the_model`; `audio_stats_lines`).  M6 1-40 (`main`) kept: it needs a window.  M5 3-2 closed by M7's token tests.  TD-39 shrunk (`e1e4da2`, logs `cd272d2`).
  * 2026-10-28: the list of Context matches the logs: 21 rows, and M3 1-2 and M4 1-1.  Native tests in `e1e4da2`, and a `catch-up` row for each in the logs of M3 to M6.  Each mutant was applied by hand, and its test failed with it.  `xtask/tests/e2e_cli.rs` runs `smoke`, `separate`, `memory-budget`, `stored-stems` and `old-session` with no driver.  Each `Ok(())` body and the `Ok(())` `run` of `xtask/src/e2e.rs` fail them.  `a_run_of_the_model_may_take_four_hours` fails with `4 + 3600` and `4 / 3600` in `MODEL_RUN_SECS`.  `a_wait_ends_on_its_line_or_a_page_error` fails with `&&` in `holds_or_error`.  `roformer_refuses_bad_input_before_the_model` fails with an `Ok(())` `run`.  `the_audio_stats_line_comes_every_2_s_while_playing` fails with `vec![]` in `audio_stats_lines` (the old `log_audio_stats`).  Kept: M6 1-40, `main` of `app/src/main.rs`, which needs a window; `a_folder_opens_its_stems_or_says_why_not` checks its folder argument.  M5 3-2: the four token tests of `3d335f4` pass in Chrome.  M3 1-2 and M4 1-1: the parent plan now says what the code does; it does not name the `.part` files.  TD-39 lists only what is left.  web-test in Chrome: app lib 137, worker 68, decode-worker 10, all passed.  Workspace tests: 787 passed.
* [ ] C: the page-decode limit (TD-58).
* [ ] D: `mutants::skip`.
* [ ] E: the stem worker's memory in Safari.
* [ ] F: the output buffer.
* [ ] G: review rounds, and the log in `docs/reviews/`.

## Surprises and discoveries

None yet.

## Decision log

* 2026-10-28, team leader: D1, because `review-rounds` reviews a branch range, not one old commit.  D2, because the brief allows a browser-only `run` only where CI runs it.  D3, because decision 0015 bounds the memory path, and many lanes of 100 s do not fit MEM-4.  D4, because the attribute also removes struct-field mutants (M7 review 5-2).  D5, because M10 needs the larger figure.  D6, because the maintainer asked for about twice the headroom.
* 2026-10-28, orchestrator, on the team leader's report of F's cost: F goes on as briefed, and last of the implementer steps.  With two queued buffers of 4,096 frames, the queue is about 186 ms, and decision 0017's rough seek bound is about 182 ms, past M10's 150 ms seek p99 bar.  The buffer size is one named constant in `engine/src/live.rs`, so a later change is one line.  The orchestrator sends word if the maintainer chooses 2,048 or another size.
* 2026-10-28, maintainer (through the orchestrator): the browser's output buffer is 3,072 frames, not 4,096.  This replaces the size in the entry above.  At 44.1 kHz one buffer is about 69.7 ms, and the queue of two about 139 ms, plus the output latency.  The worst delay of a mute, solo or tempo change is about 139 ms; the page sees a seek in at most about 70 ms.  Decision 0017's rough seek bound moves from about 136 ms to about 159 ms (136 + 69.7 - 46.4), past M10's 150 ms seek p99 bar by about 9 ms.  The size stays one named constant.
* 2026-10-28, team leader: the round 4 reviewers and E's measurer started before the plan's review ended.  Both are read only, or work in their own worktree.  Their criteria are the milestone plans and the brief, not this plan's text.  So they did not wait.

## Outcomes and retrospective

Not started.
