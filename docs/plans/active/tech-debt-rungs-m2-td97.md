# Tech debt rungs, M2 step 11: TD-97, the page's memory check of `e2e save`

## Purpose

D11 of `xtask e2e save` checks that a save of a 10-minute song grows the page's WebAssembly memory by at most 1 MiB more than a save of a 1-minute song (MEM-4; 3 MiB since the maintainer's decision of 2026-11-11, Decision log).  In Safari, the check fails at times with no change of the code: CI run 43008561411 failed it for MP3.  CI's `macos` job runs `e2e save --browser safari`, and step 12 of the parent plan makes that job a required check.  So a flaky check would block merges.

This step finds the cause of the flake with measurements, then fixes the measure or the cause.  The check must stay sound: it must still fail when a save grows the page's memory with the length of the song.  It must not fail because of when the page's start-up growth lands.

This plan is step 11 of M2 of `plans/active/tech-debt-rungs.md`.

## Context and orientation

### The decisions that this step follows

* The maintainer asked on 2026-11-11 to fix TD-97 now, and to make the `macos` job a required check (step 12).  The orchestrator changes the ruleset; no agent of this step changes it.
* Replaced by the maintainer's decision of 2026-11-11 (Decision log): `WASM_BOUND_BYTES` is 3 MiB.  Before it, the rule was: do not widen `WASM_BOUND_BYTES` (step 11 of the parent plan, and the fix line of TD-97).
* Decision 0024 is the review rule, through the `review-rounds` skill.  The review base is `b94f5bc`.
* Question 8 of the parent plan: no retry of a test or a scenario.  Only a WebDriver session start may try once more.

### The code today

* `xtask/src/e2e/save.rs`:
  * `run` restores each planted session once first, so the first decode and the stem overviews do not overlap a save.  Then it calls `save_once` for each format (`formats`) and each length (`SECONDS`, 60 s and 600 s).
  * `restore` makes the session current with a script, then reloads the page (`p.reload()`).  It waits for the line `store: session <id> is current`, and gives the log mark of the new page.  So each save is on a new page.
  * `save_once` waits for the probe line of the new page, then `p.idle(3.0)`.  Then it calls `window.__e2e_heap()`, and then it reads `wasm_before` with the script `WASM_MEMORY` (`memory.buffer.byteLength` of `window.wasmBindings`).  It clicks the save (`click_save`), waits for the offer, and reads `wasm_after` with `AFTER_SAVE`.  `growth(wasm_before, wasm_after)` gives `Save::wasm`.
  * `save_checks` compares, for each format, the `wasm` of the 600 s save with the 60 s save: `l - s <= WASM_BOUND_BYTES`.  The test of the bound is in the `mod tests` of the file, near line 1740.
  * `WASM_BOUND_BYTES` is 1 MiB (3 MiB since `7dc0388`).  Its doc says that the save's reader holds 3 spans of 65,536 frames of each lane (`SAVE_AHEAD`), whatever the length of the song.
* `xtask/e2e/save_spy.js`: `__e2e_heap` runs `gc()` 4 times, with a task of 20 ms after the first 3, in Chrome (`--js-flags=--expose-gc`).  A collection queues the finalizers of wasm-bindgen's JS objects, which free their Rust objects in a later task.  The spy also calls `__e2e_heap` at each write of a save (`__e2e_start_heap`, `__e2e_stop_heap`).  In Safari, `__e2e_heap` gives `null` at once and collects nothing.  So in Safari, those frees come only when Safari's own collector runs.
* `app/src/lib.rs`, near line 707: the page logs `page wasm <n> MB <when>` at some stages, rounded to 1 MB.  That is too coarse for this step.
* `xtask/src/e2e/old_session.rs` reads the page's memory twice, about 2 s apart, and keeps the larger value.  This is a known pattern for a late growth.

### The facts about WebAssembly memory

* A memory grows in pages of 64 KiB, and never shrinks.  Its `buffer.byteLength` after a save is the high-water mark of the page, not of the save ([MDN, `WebAssembly.Memory.prototype.grow()`](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/Memory/grow)).
* So a growth that the page does after `wasm_before` and before the save counts as growth of the save.  Growth that the page did before `wasm_before` does not.  And a save that allocates into free space that the page start left does not grow the memory at all.

### The data

* CI run 43008561411, job 130482081111, `e2e save --browser safari` on `macos-latest`.  Growth of each save, in bytes, as 60 s / 600 s: wav 7,471,104 / 7,340,032; flac 7,536,640 / 7,405,568; alac 7,405,568 / 7,405,568; mp3 5,636,096 / 7,536,640 (failed); aac 7,274,496 / 7,536,640.  The log is in the orchestrator's scratchpad, `m2/fail-130482081111.log`.
* Step 9 of `plans/completed/tech-debt-rungs-m2.md` (Surprises, 2026-11-11): 6 runs on the maintainer's Mac in Safari 27.2 gave 30 pairs.  Each growth was near 5.8 MB or near 7.3 MB.  One pair failed: aac 5,767,168 / 7,274,496.
* M8 of `plans/completed/stream-from-storage-m8.md`, in Chrome: each save grew the memory by 3.5 to 3.9 MB, and a 10-minute save by -0.13 to +0.25 MiB more than a 1-minute save.
* So in Safari the growth has two levels, about 1.5 to 1.9 MB apart, that do not depend on the length of the song or the format.  The check fails when the 1-minute save takes the low level and the 10-minute save the high one.

### The leads

The cause is not known.  Measure before you choose one of these.  Each one is a lead, not a fact.

* L1, the start-up of the page (the orchestrator's first lead).  `save_once` reads `wasm_before` 3 s after the page shows its session.  If the page grows its memory by about 1.5 to 1.9 MB later than that, in some runs only, the growth falls into the save.
* L2, the save itself.  The order of the allocations of a save can differ between runs, for example with the timing of the worker's replies.  Rust's allocator on wasm32 asks `memory.grow` for a large block that fits no free hole.  So one shift of the place of one buffer moves the high-water mark by a whole buffer.  The steps of the data are 1,507,328 bytes (23 pages) and 1,900,544 bytes (29 pages).  One span of 65,536 frames of 2 channels of `f32` is 524,288 bytes, and 3 spans are 1,572,864 bytes (24 pages).
* L3, inputs that depend on the browser: the page's allocations depend on values that come from JS, such as animation frames, events and sizes.  Safari can give other values than Chrome.  (The module's `memory.grow(n)` sets the number of pages, not the engine, and `byteLength` is exact.)
* L4, the collector.  In Chrome, `__e2e_heap` collects garbage before `wasm_before` and at each write, so the finalizers free the Rust objects of dropped JS wrappers at known times.  In Safari, they run when Safari's collector runs.  So in some saves about 1.5 MB is free when the save starts, and in others it is not.  This fits the two levels and the lower, tighter numbers of Chrome.

* L5, the Save menu.  `click_save` opens the menu, waits 0.5 s, takes a screenshot, then clicks the item.  The draw of the menu can grow the memory after `wasm_before` and before the save.

What each lead predicts:

* L1: the growth comes at a time after the page loads, with no relation to the save.  A page with no save shows it too.
* L2: the growth comes at a fixed stage of the save, such as the allocation of the reader's spans, and its size changes between runs.
* L3: the growth differs between Safari and Chrome at the same stage, with the same history of the page.
* L4: the growth changes with the time of the collector.  The count of finalizer frees before the save differs between the two levels.
* L5: the growth comes between the two clicks.

### The rules of the Mac

* Before each Safari run, check the screen lock: `ioreg -n Root -d1 -a | plutil -extract IOConsoleUsers xml1 -o - - | grep -A1 CGSSessionScreenIsLocked`.  It gives `<false/>` or nothing when the screen is unlocked.  If it is locked, wait and report; do not run Safari.
* Run one browser run at a time.
* Give every wait a time limit.  Run each `e2e` command with the Bash tool's `timeout`, or in the background with a time limit.  A run of `e2e save` takes about 3.5 minutes; give it at most 15 minutes.
* End a hung run: its `xtask`, the runner and `safaridriver`.  Never quit Safari.
* The maintainer does not use the Mac during Safari runs.  A hidden page can hang a run (TD-96).  After a hang, read `/usr/bin/log show --last 1h --predicate 'process == "Safari" AND eventMessage CONTAINS "view visibility state changed"'`.
* Never search the file system from `/` or `~`.  Use the paths in this plan and in `docs/references/local-browser-testing.md`.
* Chrome needs `CHROMEDRIVER` (`docs/references/local-browser-testing.md`).  On this Mac (arm64), it is `/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.
* Build the page first: `(cd app && trunk build --release)`.  The `e2e` commands are in the root `README.md` (browser tests).

### The paths of this step

* The orchestrator's scratchpad: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/e7f95d63-1ad1-4b43-bc80-eeb341ac774e/scratchpad/m2/td97/`.  Put the logs of the measure runs there, as `measure/<browser>-<n>.log`, and the logs of the acceptance runs as `accept/<browser>-<n>.log`.
* Each change of `.claude/**`, `CLAUDE.md` or `plans/active/tech-debt-rungs.md` goes to the orchestrator as a unified diff in that folder.  No agent of this step edits those paths.

### Terms

* Level: one of the two values near which the growth of a save falls in Safari.
* Settled page: a page whose WebAssembly memory has not grown for a stated time, after its session is current.
* Start-up growth: growth of the page's memory after the page shows its session and before the click of the mix lane's Save.
* Level, after the fix: the values of the new measure, which the check compares.

## Plan of work

Each step that changes code is one commit, with `cargo run -p xtask -- check --tier commit` before it, signed (`git commit -S`).  Each new test fails first, on an assert.

### Step A: measure

Step A commits only the table in Surprises, and a measure that stays as a note of the run.  A measure that stays needs its own test (VERIFY-1).

1. Add a sampler that does not block the run, in `xtask/e2e/save_spy.js`.  The server injects the spy before the app on each page, so each reload starts it again, from the page's load.  When `window.wasmBindings` gives its memory, keep the `memory` object once.  Then a `setInterval` of 0.25 s reads `memory.buffer.byteLength` with no `await`, and keeps it with `performance.now()`.  So the sampler makes almost no garbage, which matters for L4.  Read its samples after the offer, and 3 s after the offer, with one script each.  Keep the run's waits as they are: `p.idle(3.0)` stays, so the measure does not move the click (a later click is itself a fix of L1).
2. Stamp each line of `xtask/e2e/console.js` with `performance.now()`, so the page's lines and the samples have the same clock.  Mark these times: the page's load, the `is current` line, the probe line, the read of `wasm_before`, each `gc()` of `__e2e_heap` (in Chrome), the click of the mix lane's Save, the click of the menu's item, and the offer.  `click_save` waits 0.5 s and takes a screenshot while the menu is open.  Give the growth between the two clicks on its own, since the menu's draw can grow the memory too (L5).
3. Log the memory at the save's own stages from the page: the allocation of the reader's spans, the first write, the first reply of the worker, and the end.  A temporary log line in `app/` is acceptable for the measure.
4. Count the finalizer frees of wasm-bindgen's objects before the save and during it, if the page can count them without a change of its behaviour (for example, a wrapper of `FinalizationRegistry` in the spy).  If it cannot, say so in Surprises.
5. Add two controls:
   * a new page, sampled for 20 s with no save;
   * a save after a 15 s idle of its page, in place of 3 s.
   Run the controls as a separate scenario or a temporary flag.  Do not commit them unless they stay with a test.
6. Run `e2e save --browser safari` 3 times and `e2e save --browser chrome` 2 times on the Mac, with the rules of the Mac.  Run each control twice in Safari and once in Chrome.  Keep each log in `m2/td97/measure/`.
7. Record a table in Surprises: for each run and each save, the memory at each time of item 2, the time and size of each growth, and the level.  Give the count of saves at each level, for each browser.  Compare it with the counts of step 9 (no sampler), so a measure that changes the levels is seen.  Say which lead the data supports, by the predictions above, with the evidence.
8. Run more Safari measure runs until at least one save at the low level has samples, up to 9 runs in all.  Step B needs them.  If 9 runs give no low-level save, stop and report to the team leader with the data.

Acceptance: the table in Surprises shows when and where the memory grew, at each level.  It names the cause, or narrows it to one or more of L1 to L4, with evidence.

### Step B: fix the measure or the cause

The choice, after the maintainer's answer (Decision log, 2026-11-11): D11 keeps the growth of `memory.buffer.byteLength` in each save, and `WASM_BOUND_BYTES` becomes 3 MiB, from the spread measured in step A.  No change to the page.  TD-97 closes; no TD for the Save menu's buffer.  This replaces option 1 below, which the maintainer refused, and the item "Do not widen `WASM_BOUND_BYTES`".

Replaced: the choice, after step A (Decision log, 2026-11-11): option 1 of Surprises.  The page counts its Rust heap with a counting global allocator on wasm32.  At the save's start, it logs the live bytes.  At the end, it logs the peak of the save above them.  `e2e save` compares those peaks of the 10-minute and the 1-minute saves, against `WASM_BOUND_BYTES`.  The memory's growth stays in the log, not checked.  The rest of this step stays as written, but where the choice changes it:

* The allocator is hand-written in `app`, about 30 lines, with no new crate (DEPS-1).  It wraps the default allocator.  Its counters are generic over the inner allocator, so a native test can check them.  It allocates nothing itself, and adds no wait (RT-1).
* The page gives the two values in one log line, as the worker's `memory: encode` line does.  A pure parser in `xtask/src/e2e/save.rs` reads it, with tests.
* A new decision record in `docs/decisions/`, `proposed`, says why the check measures the heap's requests and not the memory's growth.  The orchestrator lists it under "Needs your attention".
* TD-97 is narrowed, not closed, to the gap that is left: the check does not see a growth that comes from the allocator's layout alone.  The entry gives the evidence of step A.
* The short buffer of about 4 MB that the Save menu's draw makes (L5) is a cost of MEM-4 on an iPhone.  Find what makes it, if a short search can.  Else record it in a TD.

Choose the fix from the data of step A, and record the choice in the Decision log.  The candidates:

* If L5 holds: read `wasm_before` after the menu opens, just before the click of the item.  A test asserts that a growth that scales with the length, after that read, still fails the check.
* If L1 holds: read `wasm_before` from a settled page.  For example, poll the memory until it has not grown for a stated time, with a time limit, and fail with a clear message at the limit.  Or wait for a page line that ends the start-up.  A growth after the settled read still counts.
* If L4 holds: make the time of the frees the same in each save, in Safari too, or measure in a way that does not depend on it.  The page itself must not depend on the collector for a sound bound.
* If L2 or L3 holds: change the measure so that it does not depend on the order of the allocations, but still sees a growth with the length of the song.  For example, compare the high-water marks of the two saves on pages of the same history.  Or remove the cause in the page, if the cause is in the repo.
* If the cause is outside the repo, the measure must still become sound and stop the flake, since step 12 makes the job a required check.  Only the cause may stay in TD-97, narrowed, with the evidence.  Say so in `docs/references/browser-quirks.md` or `docs/references/local-browser-testing.md`.
* A measure that passes the bare pairs of growth of TD-97 is not a fix.  A minimum of reads, a subtraction of a level, or a rounding to a level hides the flake.

Whatever the fix, these hold:

* `WASM_BOUND_BYTES` is 3 MiB (Replaced by the maintainer's decision of 2026-11-11 (Decision log); it was "stays 1 MiB").
* The check still fails when a 10-minute save grows the page's memory by more than 3 MiB more than a 1-minute save (MEM-4).
* The measure is a pure function with unit tests in `xtask/src/e2e/save.rs`.  The browser run gives it its inputs.
* No retry of the scenario or of a save (question 8).
* Update every doc that states the measure in the same commit (DOC-1): the module doc of `xtask/src/e2e/save.rs`, the doc of `WASM_BOUND_BYTES`, the root `README.md` if it names the measure, `docs/invariants.md` (the gaps of MEM-4 and of VERIFY-1), `docs/references/local-browser-testing.md` (the TD-97 line), and `docs/tech-debt.md` (TD-97 to `## Closed`, or narrowed).

Acceptance:

* Replaced by the maintainer's decision of 2026-11-11 (Decision log), with the next two items: the two-level pairs of TD-97 and of run 43008561411 pass the bound of 3 MiB, and one byte more fails (`the_two_level_pairs_of_safari_pass_and_one_byte_more_fails`).  Literal cases of a save that keeps 1/30 or 1/18 of the song's 16-bit audio fail, also with a wider bound (`a_save_that_keeps_a_part_of_the_song_fails`).  Before it:
* No save at the low level came in step A, so no real samples of one exist.  So a unit test models the two layouts of Surprises, (a) and (b), from the measured values of step A.  In each, the page asks for the same heap in the 1-minute and the 10-minute save, and the memory's growth differs by about 1.9 MB.  The test asserts that the old measure gives a pair over the bound, and that the new measure passes.  It fails on the old measure, on an assert.  The two-level pairs of TD-97 and of run 43008561411 are in the test's doc, as the cases that the model stands for.
* (Replaced, see above.)  A unit test asserts that the check fails when the growth scales with the length: for example, a 10-minute save that grows the memory by 1 MiB and one 64 KiB page more than the 1-minute save, after the start-up growth has settled.
* (Replaced, see above.)  If the fix waits for a settled page, a unit test of the wait covers the stable case, the late growth case and the time limit.  Another test asserts that the check fails when a growth that scales with the length lands after the settled read and before the click.  So the wait does not hide it.

### Step C: acceptance runs

1. Build the page: `(cd app && trunk build --release)`.
2. Run `e2e save --browser safari` 6 times in a row on the Mac, with the rules of the Mac.  Each run passes every check.  A failure starts the count again after its cause is known and fixed.  A run that a hidden page hangs (TD-96, shown by the visibility log) does not count, and does not start the count again.  Any other hang starts the count again.
3. Replaced by the maintainer's decision of 2026-11-11 (Decision log): each 10-minute less 1-minute growth of the 6 runs is under 3 MiB, and the 5 pairs of each run are a table in Progress.  Before it: six passes alone are not proof: before the fix, about 1 run in 6 failed, so 6 passes in a row came about 1 time in 3 with no fix.  So, for each format, all 12 values of the new measure (6 runs, 2 lengths) span at most 1 MiB (one level, not two).  Give the 12 values of each format as a table in Progress.  Give the count of saves at each level before the fix (step A, the old measure) and after (the new measure).
4. Run `e2e save --browser chrome` once.  It passes.
5. Run `cargo run -p xtask -- check --tier push` and `cargo build -p xtask`.
6. Record each run, its log path and its D11 numbers in Progress.

Acceptance (Replaced by the maintainer's decision of 2026-11-11 (Decision log)): 6 Safari runs in a row pass, each 10-minute less 1-minute growth is under 3 MiB, the Chrome run passes, and `check --tier push` passes.  Met on 2026-11-11 at `7dc0388` (Progress).  Before it: 6 Safari runs in a row pass, the 12 values of each format span at most 1 MiB, the Chrome run passes, and `check --tier push` passes.  TD-97 is under `## Closed` in `docs/tech-debt.md`, or narrowed with a clear cause outside the repo.

### Step D: review rounds

The team leader runs `/review-rounds b94f5bc <round> "<criteria>"` from round 1, until the stop rule of decision 0024 holds.  The criteria are step 11 of M2 in the parent plan and this plan.  After the last round, the team leader moves this plan to `docs/plans/completed/`, and changes its line in `docs/README.md`.

## Validation and acceptance

* `cargo test -p xtask` passes, with the new tests of steps A and B.
* `cargo run -p xtask -- check --tier push` passes on the maintainer's Mac.
* `cargo run -p xtask -- e2e save --browser safari` passes 6 times in a row on the maintainer's Mac.
* `cargo run -p xtask -- e2e save --browser chrome` passes.
* CI's `macos` job passes on the PR (the orchestrator pushes and reads it).
* `docs/tech-debt.md` lists TD-97 under `## Closed`, or it has a narrowed entry with its cause.

## Idempotence and recovery

* A measure run changes no committed file.  Run it again at any time.
* Each step is one commit, so a revert of one commit undoes one step.
* A hung run: end its `xtask`, runner and `safaridriver`, check the screen lock and the visibility log, and run it again.  Never quit Safari.
* If the data of step A names no cause, or has no low-level save, add Safari runs up to 9 in all.  Then stop and report to the team leader with the data.

## Progress

* [x] 2026-11-11: this plan, after review by a `Plan` agent (round 1: 4 blockers, 5 should-fix and 3 nits; round 2: 4 should-fix; each fixed).
* [ ] Step A: measure, with the table in Surprises.
  * [x] 2026-11-11: the measure tools, in a worktree outside the repo (`m2/td97/measure-tree`, patch `m2/td97/measure.patch`).  No part is committed, so no test is owed.  They are the sampler, the stamps, the events, the finalizer count, a count of the Rust heap, a log of the font atlas, and the two controls.
  * [x] 2026-11-11: 2 Chrome runs and the 2 Chrome controls, 9 Safari measure runs (6 full runs, 2 of them controls of a 15 s idle; 1 run under load that stopped; 2 runs with no late read), 2 Safari page controls, and 3 Safari runs of HEAD.  Each log is in `m2/td97/measure/`.  The table is in Surprises.
  * [x] 2026-11-11: no save at the low level in 114 Safari saves (step A item 8).  The plan says to stop and report.  Step A stops here, and steps B and C wait for the team leader.
* [x] Step B: the fix, with its tests.  The team leader chose option 1 on 2026-11-11 (Decision log); the maintainer replaced it with a bound of 3 MiB the same day.
  * [x] 2026-11-11 (replaced by the next item, reverted): `app/src/heap_count.rs` counted the page's Rust heap on wasm32.  The page logs the heap line at each offer of the storage path.  `save_checks` compares the peaks; the memory's growth is logged.  Decision 0030 (`proposed`), TD-97 narrowed, and a new TD for the menu's buffer.  Each new test failed first on an assert:
    * `the_count_follows_each_call_and_keeps_the_peak`: against a `restart_peak` that did not set the peak, `assert_eq!(heap.peak(), 200)` got 4000.
    * `a_refused_allocation_counts_nothing`: against an `alloc` that counted a null pointer, the counts were (64, 64), not (0, 0).
    * `a_build_with_no_count_gives_none`: against a `restart_peak` that gave `Some(0)` on a native build.
    * `a_heap_line_gives_the_peak_above_the_start`: against a `heap_line` that gave `None`.
    * `a_growth_by_layout_alone_passes_the_heap_check`: against the old `save_checks`, layout (a) failed "wav: the 10-minute save grows the page's WebAssembly memory ... (Some(5373952.0) and Some(7471104.0) bytes)".
    * `a_heap_peak_that_grows_with_the_song_fails`: against the old `save_checks`, a peak 1 MiB and 64 KiB higher gave no failure (`left: 0, right: 1`).
    * `the_wasm_check_bounds_the_growth_of_the_long_save` became `the_heap_check_bounds_the_peak_of_the_long_save`.  It also asserts that a large memory growth alone passes.
  * [x] 2026-11-11: one Chrome run of the new build passed (`m2/td97/accept/chrome-pre.log`).  The peaks were 7,647,356 to 7,647,369 bytes, the same for 1 and 10 minutes to 0 bytes in each format.  Each heap line's start plus its peak is the absolute peak of step A's `chrome-2` (25,973,059 for WAV).  The live bytes at the save's start are about 1.7 MB above those at the item's click, so the step A peak above the click is 1.7 MB higher.
  * [x] 2026-11-11, after the maintainer's answer: the counting allocator, its line, its parser and their tests are removed; `app/` is as before `af135fb`.  D11 keeps the memory's growth, and `WASM_BOUND_BYTES` is 3 MiB.  TD-97 is closed, and the TD of the menu's buffer is dropped.  Decision 0030 is kept as `rejected`.  The new test `the_two_level_pairs_of_safari_pass_and_one_byte_more_fails` failed first on the bound of 1 MiB, at its first `assert!`: the CI pair, set on the ALAC saves, gave the failure "alac: the 10-minute save grows the page's WebAssembly memory by at most 1048576 bytes more than the 1-minute save (Some(5636096.0) and Some(7536640.0) bytes)".
* [x] Step C: 6 Safari runs in a row and one Chrome run pass, with the bound of 3 MiB.
  * [x] 2026-11-11, at `7dc0388`: 6 runs of `e2e save --browser safari` in a row passed each check, with the lock read before each (`m2/td97/accept2/safari-1.log` to `safari-6.log`).  None hung.  `e2e save --browser chrome` passed (`m2/td97/accept2/chrome-1.log`).  The growth of each save, in bytes, as 60 s / 600 s, and the 10-minute less the 1-minute growth:

| Run | WAV | FLAC | ALAC | MP3 | AAC |
| - | - | - | - | - | - |
| Safari 1 | 7,340,032 / 7,274,496 (-65,536) | 7,340,032 / 7,340,032 (0) | 7,208,960 / 7,471,104 (+262,144) | 7,274,496 / 5,701,632 (-1,572,864) | 7,405,568 / 7,405,568 (0) |
| Safari 2 | 7,405,568 / 7,405,568 (0) | 7,274,496 / 7,405,568 (+131,072) | 7,274,496 / 7,274,496 (0) | 7,340,032 / 7,340,032 (0) | 7,340,032 / 7,340,032 (0) |
| Safari 3 | 7,405,568 / 7,340,032 (-65,536) | 7,274,496 / 7,274,496 (0) | 7,405,568 / 7,340,032 (-65,536) | 7,405,568 / 7,274,496 (-131,072) | 5,636,096 / 7,340,032 (+1,703,936) |
| Safari 4 | 7,274,496 / 7,274,496 (0) | 7,536,640 / 7,536,640 (0) | 7,471,104 / 7,471,104 (0) | 7,274,496 / 7,340,032 (+65,536) | 7,405,568 / 7,274,496 (-131,072) |
| Safari 5 | 7,208,960 / 7,340,032 (+131,072) | 7,405,568 / 7,340,032 (-65,536) | 7,536,640 / 5,636,096 (-1,900,544) | 7,340,032 / 7,471,104 (+131,072) | 7,340,032 / 7,536,640 (+196,608) |
| Safari 6 | 7,340,032 / 7,471,104 (+131,072) | 7,471,104 / 7,405,568 (-65,536) | 6,160,384 / 7,340,032 (+1,179,648) | 7,274,496 / 5,832,704 (-1,441,792) | 7,405,568 / 7,274,496 (-131,072) |
| Chrome | 7,274,496 / 7,274,496 (0) | 7,405,568 / 7,274,496 (-131,072) | 7,405,568 / 7,274,496 (-131,072) | 7,274,496 / 7,471,104 (+196,608) | 7,471,104 / 7,143,424 (-327,680) |

  * The largest 10-minute less 1-minute growth in Safari is +1,703,936 bytes (AAC, run 3), under the bound of 3,145,728.  The bound of 1 MiB would have failed runs 3 and 6 (+1,703,936 and +1,179,648).  Unlike step A, 5 saves came at the low level (5,636,096 to 6,160,384).  So the low level is still there; the bound holds over it.  With these runs, 150 Safari pairs give a largest difference of 1,900,544 bytes, and single growths from 5,505,024 to 7,602,176, the same as before.  The Chrome run's largest difference is 327,680 in absolute value.
  * [x] 2026-11-11: `cargo run -p xtask -- check --tier push` (13 lines) and `cargo build -p xtask` passed (`m2/td97/check-push-2.log`).
  * [x] 2026-11-12, review round 1: `a_save_that_keeps_a_part_of_the_song_fails` pins the bound from above with literal values: 5,636,096 and 8,811,296 (1/30 kept at one level), and 7,602,176 and 10,797,024 (1/18 kept in the worst layout, 3,194,848 bytes more).  It failed first with `WASM_BOUND_BYTES` at 1024 MiB, on `assert_eq!(failed.len(), 1, ...)` for the first pair (`left: 0, right: 1`), and passes at 3 MiB.  The constant's doc, MEM-4, TD-97 and the Decision log give both thresholds.
* [x] Step C, replaced (the heap's peak, at `af135fb`): 6 Safari runs in a row and one Chrome run passed.
  * [x] 2026-11-11, at `af135fb`: 6 runs of `e2e save --browser safari` in a row passed each check, with the lock read before each (`m2/td97/accept/safari-1.log` to `safari-6.log`).  None hung.  `e2e save --browser chrome` passed (`m2/td97/accept/chrome-1.log`).  The 12 Safari values of the new measure of each format, the heap's peak above its live bytes at the save's start, in bytes:

| Format | The 12 peaks (6 runs, 60 s and 600 s) | Span | The old growths, 60 s / 600 s, of the 6 runs | Old pairs over 1 MiB |
| - | - | - | - | - |
| WAV | 7,647,361 each | 0 | 7,471,104 / 7,340,032; 7,471,104 / 7,405,568; 7,340,032 / 7,471,104; 7,405,568 / 7,208,960; 7,340,032 / 7,340,032; 7,274,496 / 7,471,104 | 0 |
| FLAC | 7,647,360 each | 0 | 7,340,032 / 7,405,568; 7,405,568 / 7,471,104; 7,405,568 / 7,340,032; 7,405,568 / 7,471,104; 7,340,032 / 7,208,960; 7,274,496 / 7,536,640 | 0 |
| ALAC | 7,647,356 each | 0 | 7,208,960 / 7,340,032; 7,536,640 / 7,340,032; 7,536,640 / 7,208,960; 7,471,104 / 7,405,568; 7,471,104 / 7,340,032; 7,274,496 / 7,471,104 | 0 |
| MP3 | 7,647,369 each | 0 | 7,405,568 / 7,536,640; 7,340,032 / 7,536,640; 7,471,104 / 7,274,496; 7,340,032 / 7,536,640; 7,340,032 / 7,274,496; 7,340,032 / 7,208,960 | 0 |
| AAC | 7,647,365 each | 0 | 7,405,568 / 7,471,104; 7,274,496 / 7,340,032; 7,340,032 / 7,274,496; 7,340,032 / 7,471,104; 7,405,568 / 7,274,496; 7,405,568 / 7,274,496 | 0 |

  * Levels: before the fix, step 9 and CI had 7 low saves in 70 (old measure).  In step A, 0 in 114 Safari saves.  In step C, the old growth (logged) had 0 low saves in 60; the new measure has one value for each format, in each run, in both browsers.  The Chrome run gave the same 5 peaks.  The values of step C span 0 bytes, so they are within 1 MiB.  But no low save came in step C either, so step C does not show the new check on a page at the low level.  Step B's model test covers that case.
  * [x] 2026-11-11: `cargo run -p xtask -- check --tier push` and `cargo build -p xtask` passed (`m2/td97/check-push.log`).
* [ ] Step D: the review rounds.

## Surprises and discoveries

* 2026-11-11, the CI log of run 43008561411 (job 130482081111), read again.  The page lines of the low MP3 save of 1 minute are the same as the lines of the high saves.  Its line `store: session ... is current` comes before `separator: no model cached in this browser`, but the high AAC save of 1 minute has the same order.  The log gives the growth of each save only, not the memory before and after.  So it cannot tell a higher memory before the save from a lower memory after it.  Each save clicked the item 0.8 to 1.4 s after the click of Save, and the run took 167 s to its last save.
* 2026-11-11, the measure (step A).  The sampler reads `memory.buffer.byteLength` each 0.25 s.  Events read it, the finalizer count, and the Rust heap's live bytes and peak, at each stage.  A temporary counting allocator in the page gives the Rust heap.  In bytes, over each log:

| Log | Saves | Growth (old measure) | Memory before | Memory after (high-water) | Growth with the menu open (L5) | Growth from the item's click | Rust heap peak above its live bytes at `before` | Finalizer frees |
| - | - | - | - | - | - | - | - | - |
| `chrome-1` | 10 | 7,274,496 to 7,602,176 | 21,954,560 to 22,151,168 | 29,360,128 to 29,687,808 | 1,835,008 to 2,162,688 | 5,242,880 to 5,701,632 | not measured | 0 |
| `chrome-2` | 10 | 7,274,496 to 7,602,176 | 22,020,096 to 22,151,168 | 29,425,664 to 29,622,272 | 1,835,008 to 2,031,616 | 5,308,416 to 5,636,096 | 9,395,484 to 9,397,752 | 0 |
| `chrome-control-idle15-1` | 10 | 7,143,424 to 7,471,104 | 22,020,096 to 22,216,704 | 29,294,592 to 29,556,736 | 1,835,008 to 2,097,152 | 5,308,416 to 5,505,024 | 9,395,484 to 9,397,656 | 0 |
| `safari-1` | 10 | 7,143,424 to 7,471,104 | 24,117,248 to 24,313,856 | 31,391,744 to 31,719,424 | 1,703,936 to 2,031,616 | 5,242,880 to 5,505,024 | 9,396,504 to 9,397,752 | 0 |
| `safari-2` | 10 | 7,208,960 to 7,602,176 | 24,117,248 to 24,313,856 | 31,522,816 to 31,719,424 | 1,835,008 to 2,097,152 | 5,373,952 to 5,570,560 | 9,396,504 to 9,397,752 | 0 |
| `safari-3` | 10 | 7,143,424 to 7,471,104 | 24,117,248 to 24,313,856 | 31,391,744 to 31,719,424 | 1,835,008 to 1,966,080 | 5,242,880 to 5,505,024 | 9,396,504 to 9,397,752 | 0 |
| `safari-4` | 10 | 7,274,496 to 7,602,176 | 24,117,248 to 24,313,856 | 31,457,280 to 31,719,424 | 1,835,008 to 2,031,616 | 5,373,952 to 5,636,096 | 9,396,600 to 9,397,752 | 0 |
| `safari-control-idle15-1` | 10 | 7,143,424 to 7,602,176 | 24,117,248 to 24,313,856 | 31,457,280 to 31,784,960 | 1,638,400 to 2,031,616 | 5,308,416 to 5,767,168 | 9,396,600 to 9,397,752 | 0 |
| `safari-control-idle15-2` | 10 | 7,143,424 to 7,602,176 | 24,117,248 to 24,313,856 | 31,457,280 to 31,719,424 | 1,835,008 to 2,097,152 | 5,308,416 to 5,636,096 | 9,396,504 to 9,397,752 | 0 |
| `safari-5-loaded12` | 4 | 7,274,496 to 7,471,104 | 24,182,784 to 24,313,856 | 31,588,352 to 31,653,888 | 1,835,008 to 1,966,080 | 5,439,488 to 5,505,024 | 9,396,600 to 9,397,752 | 0 |
| `safari-6-v3` | 10 | 7,208,960 to 7,602,176 | 24,117,248 to 24,313,856 | 31,457,280 to 31,719,424 | 1,835,008 to 2,097,152 | 5,177,344 to 5,636,096 | 9,396,504 to 9,397,752 | 0 |
| `safari-7-v3` | 10 | 7,274,496 to 7,602,176 | 24,117,248 to 24,313,856 | 31,457,280 to 31,784,960 | 1,703,936 to 2,097,152 | 5,308,416 to 5,636,096 | 9,396,504 to 9,397,752 | 0 |

* Levels: each of the 84 measured Safari saves and the 30 measured Chrome saves is at the high level.  The 30 saves of the 3 Safari runs of HEAD (`safari-head-<n>`, no measure) are at the high level too, from 7,208,960 to 7,536,640.  So the measure did not remove the low level: HEAD shows none today either.  Step 9 had 6 low saves in 60, and CI 1 in 10.  So no Safari save in 114 is low.  At 1 in 10, that has a chance of about 6 in 1,000,000.  So the condition that made the low level is not present on the Mac today.  In step 9, the 5 low saves of run 3 came in 2 groups (4 saves in a row, then 1).  Safari's log shows no change of page visibility in that run.
* L1, the start-up of the page: no growth from the first sample (0.25 s after the page loads) to `before`, in each save and in the 3 page controls of 20 s (2 in Safari, 1 in Chrome; 2 pages each).  The memory at 0.25 s is the memory at `before` (24.1 to 24.3 MB in Safari, 21.95 to 22.15 MB in Chrome).  A 15 s idle in place of 3 s gives the same values.  The data does not support L1.
* L4, the collector: the wrapper of `FinalizationRegistry` counted no finalizer call on any page, also in Chrome after each `gc()`.  So no Rust object waits for the collector to be freed.  The data does not support L4.
* L5, the Save menu: in each save, the memory grows by 1.6 to 2.2 MB about 0.1 to 0.2 s after the click of Save, with the menu open.  The Rust heap then gets a peak 3,958,034 to 3,958,130 bytes above its live bytes in Safari (3,906,842 to 3,907,958 in Chrome), and keeps only 51,934 to 53,050 bytes.  So the menu's draw makes a short buffer of about 4 MB.  The font atlas does not change after the page start (16,384 by 64 pixels in Safari at 2 pixels a point, 16,384 by 32 in Chrome).  So the buffer is not a growth of the atlas.  The gap of the old levels, 1.5 to 1.9 MB, is about the size of this growth.  So a low save can be one where the menu's buffer fits into free memory, but no low save came to show it.
* L2 and L3: the live bytes of the Rust heap at `before` are the same in each page of one length, to about 100 bytes (18,673,585 to 18,673,681 for 1 minute, 18,674,793 to 18,674,889 for 10 minutes, in Safari).  The save's peak above them is the same for 1 and 10 minutes, to 2.3 KB (9,395,484 to 9,397,752 bytes, in both browsers).  So the page asks for the same memory in each save.  The growth of the memory differs by up to 458,752 bytes between saves with the same requests.  That is the allocator's layout: the place of each free block that the page start and the menu leave.  Chrome and Safari give the same structure, about 2.2 MB more memory in Safari.
* The old measure is the high-water mark after the save less the memory before it.  The high-water mark depends on which free blocks the allocator has when the save starts.  The Rust heap's peak does not.
* A run under load (12 processes of `yes` on 12 CPUs) stopped at the ALAC save of 1 minute: "the save offered no file within its cap of 60 s".  Its 4 saves before were at the high level.  So load alone did not give the low level, and full load breaks the run.
* The build cross-talk: the worktree of the measure and the repo shared one `target/`.  Cargo's unit hashes do not hold the path of a workspace member.  So `cargo build -p xtask` and `trunk build` in the repo gave the worktree's code, with no rebuild, until each source file was touched.  The 3 HEAD runs came after that rebuild (`strings target/debug/xtask` and `app_bg.wasm` hold no measure code).  The later measure build used its own `target-measure/`.
* Why no fix of the measure is proven yet: a low save has about 1.8 MB less growth with the same requests.  Two layouts can give that.  (a) The page start left more free memory at `before`, so the high-water mark after the save is the same.  (b) The menu's buffer and the save's buffers fit better into the same free memory, so the high-water mark after the save is lower.  No low save came to tell (a) from (b).
* The options for step B, for the team leader:
  * Option 1 (the agent's recommendation): measure the Rust heap's peak in the save, above its live bytes at the save's start.  The page counts its heap with a counting global allocator on wasm32, and gives the 2 values to `e2e save`.  It does not depend on the layout, so (a) and (b) give one value.  Today, 1 and 10 minutes differ by at most 2.3 KB, so the bound of 1 MiB stays.  The cost is a change to the page, so a decision record (`proposed`).  It does not see a growth by fragmentation alone, so the memory's growth stays in the log.
  * Option 2: compare the high-water marks after the two saves (`wasm_after`), not the growths.  It needs no change to the page.  Today they spread by 393,216 bytes in Safari.  It holds for (a), and fails for (b).
  * Option 3: read `wasm_before` with the menu open (the plan's L5 candidate).  The save's growth then depends on whether the menu's buffer grew the memory.  So the two levels can stay, for (a) and for (b).
  * Option 4: measure more, for a low save.  The conditions of step 9 are not known.  The maintainer may have used the Mac then, which the rules forbid now.
  * Step B's acceptance asks for samples of a low save.  With option 1 or 2, the test inputs can model (a) and (b) from the measured values.  The CI pair and the step 9 pair stay in the test's doc.  That changes the acceptance, so it needs the team leader.

## Decision log

* 2026-11-11, maintainer: fix TD-97 now; make the `macos` job a required check after it (step 12 of the parent plan).
* 2026-11-11, agent: measure first, then choose the fix.  Reason: the cause is not known, and a fix of the measure that hides a real growth breaks MEM-4.
* 2026-11-11, agent: the acceptance adds one level for each format across the 6 Safari runs.  Reason: 6 passes in a row came about 1 time in 3 before any fix (plan review round 1).
* 2026-11-11, agent: the measure tools stay in a worktree outside the repo, with a patch.  Reason: the counting allocator and the log of the font atlas change the page, and step A commits no tool that has no test.
* 2026-11-11, agent: 2 of the 9 Safari measure runs read the samples just after the offer, with no idle of 3 s after it.  Reason: to keep the time between pages of HEAD, in case the idle removed the low level.  They gave the same levels.
* 2026-11-11, agent: step A stops after 9 Safari measure runs with no low save (step A item 8).  Steps B and C wait for the team leader.  Reason: step B's test needs samples of a low save, and the plan says to report then.
* 2026-11-11, agent (team leader): step B takes option 1, the Rust heap's peak above its live bytes at the save's start.  Reason: in each save, the page asks for the same heap, to 2.3 KB, for 1 and 10 minutes.  The memory's growth then depends on the allocator's layout, which no measure of `byteLength` can fix.  The peak does not depend on the layout, and it grows with a buffer that grows with the song (MEM-4).  Option 2 fails for layout (b), and option 3 keeps the two levels.  Option 4 waits for a condition that nobody can make.
* 2026-11-11, agent (team leader): step B's test models layouts (a) and (b) from the values of step A, in place of real samples of a low save.  Reason: 114 Safari saves gave none.  The CI and step 9 pairs stay in the test's doc.
* 2026-11-11, agent (team leader): TD-97 is narrowed to the growth by the allocator's layout alone, which the new check does not see.  Reason: the check measures the page's requests; a growth by layout is real memory on an iPhone, but it does not grow with the song in the data.
* 2026-11-11, maintainer: refuse option 1 and decision 0030.  Quote: "I think you should simply measure and choose some kind of sane tolerance.  If it _really_ really makes no sense (prove why), then we can think again.  And avoid this kind of unjustifiable overengineering."  And: "writing a custom allocator is insane overengineering".  This replaces the three option-1 lines above, and the rule not to widen `WASM_BOUND_BYTES`.
* 2026-11-11, agent: `WASM_BOUND_BYTES` is 3 MiB.  Reason: after step C, over 150 Safari pairs (the logs of `m2/td97/measure/`, `m2/td97/accept/`, `m2/td97/accept2/`, `m2/flakes/`, `m2/flakes/after/` and CI run 43008561411), the largest 10-minute less 1-minute growth is 1,900,544 bytes (1.81 MiB, the MP3 pair of CI run 43008561411).  The extremes of single growths, 5,505,024 (the ALAC save of 10 minutes of step 9, `m2/flakes/after/e2e-save-3.log`) and 7,602,176, are 2,097,152 bytes apart (2.00 MiB).  Over 30 Chrome pairs, the largest difference is 196,608, and 327,680 in absolute value.  3 MiB is 1 MiB above that gap.  Step A saw the same requests in each measured save, all at the high level.  So the allocator's layout is the likely cause; the bound does not depend on the cause.  The proof that the bound still sees a growth with the song: a minute of 16-bit stereo at 44.1 kHz is 10,584,000 bytes (the 60 s WAV is 10,584,044 bytes).  At one level, the 9 minutes more fail above 3,145,728 / 9 = 349,525 bytes a minute, about 1/30 of the song's 16-bit audio.  Take the worst layout of the extremes of single growths seen: the 1-minute save high and the 10-minute save low, 2,097,152 bytes apart.  There, they fail above (3,145,728 + 2,097,152) / 9 = 582,542 bytes a minute, about 1/18.
* 2026-11-11, agent (team leader): no TD for the Save menu's short buffer.  Reason: a one-off growth of about 2 MB for each page is no cost that a reader must act on.  The finding stays in Surprises.

## Outcomes and retrospective

* None yet.
