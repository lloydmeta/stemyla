# Replay of six old review rounds

This file records M5 of harness phase 3.  M5 runs six old review rounds of this repo again, in a test harness, with the review agents and the `review-rounds` skill of this branch.  It measures how many findings of each old round the new review finds again (x of n).  The criteria are in `docs/plans/active/harness-phase-3.md`, section "M5: replay old reviews" ([link](../plans/active/harness-phase-3.md#m5-replay-old-reviews)).  The milestone plan is `docs/plans/active/harness-phase-3-m5.md`.

The ground truth below was written from git only, before any case run.  The first commit of this file holds it: `git log --reverse --format='%h %cI' -- docs/reviews/replay.md | head -1` gives that commit and its time.  The dry run and the case runs start after that time.

How the ground truth was made:

* A ground-truth item is one finding of the old round whose fix commit changed code or tests for it.  Its ID is `<case>-<n>`.
* The sources are the fix commit's message and diff.  For a phase 2 case, they also include the Progress of `docs/plans/active/harness-phase-2.md` at the fix commit.
* A finding is left out when its fix changed only docs, comments or wording, or when the old round refuted it or left it with no change.  For `m2` and `m10`, round 1's findings are left out.
* Each line range is at the reviewed commit, which is the fix commit's parent.  A range includes the doc comment of its function.
* The severity is the one that the old round recorded.  When the old round recorded none, the severity is inferred and marked `(inferred)`.
* For a weak or missing test, "Where" gives both the function under test and the test.

## Ground truth

| Case | n |
| - | - |
| `pr9` | 6 |
| `m9` | 8 |
| `m11` | 1 |
| `m2` | 5 |
| `m10` | 2 |
| `m18a` | 4 |
| Total | 26 |

### pr9

| Base | Reviewed | Fix commit | Old round | Criteria |
| - | - | - | - | - |
| `11273fc` | `ba1b592` | `c01d4cd` | round 1 | `commits:11273fc..ba1b592` |

PR #9 has no plan.  The source is the message and the diff of `c01d4cd`.  The old round recorded no severities.

| ID | Severity | Where | Defect | Evidence |
| - | - | - | - | - |
| `pr9-1` | should-fix (inferred) | `xtask/src/e2e.rs`, `Page::open`, lines 193-217 | Chrome gets `--enable-unsafe-swiftshader` only with `--swiftshader`.  Without a usable GPU, as on CI, Chrome refuses WebGL, so `smoke`, `audio-resume` and `storage` fail. | `c01d4cd`, `xtask/src/e2e.rs` |
| `pr9-2` | should-fix (inferred) | `xtask/src/e2e.rs`, `Page::errors`, lines 342-349 | The "no page errors" check misses the app's `log::error!` lines.  eframe's logger sends them to `console.warn` as "ERROR: ...". | `c01d4cd`, `xtask/src/e2e.rs` (test `page_errors_include_the_apps_log_errors`) |
| `pr9-3` | should-fix (inferred) | `xtask/src/e2e.rs`, `smoke`, lines 399-476; the tempo check, lines 457-467 | After the click on the tempo slider, the check waits for any stats line.  That line can come from before the click, so the tempo check can fail on a correct page. | `c01d4cd`, `xtask/src/e2e.rs` |
| `pr9-4` | should-fix (inferred) | `xtask/src/webdriver.rs`, `http`, lines 23-43; `.github/workflows/ci.yml`, job `browser-tests`, lines 109-142 | `http` sets no read or write timeout on its socket, so a silent driver hangs the scenario.  The CI job has no `timeout-minutes`, so a hang holds the runner for GitHub's 6 hours. | `c01d4cd`, `xtask/src/webdriver.rs` (test `a_silent_driver_times_out`), `.github/workflows/ci.yml` |
| `pr9-5` | nit (inferred) | `xtask/src/e2e.rs`, `Page::finish`, lines 351-366; `.github/workflows/ci.yml`, job `browser-tests`, lines 109-142 | Only `finish` writes the scenario's log, so a scenario that stops early with an error leaves no log.  CI keeps no logs or screenshots of a failed run. | `c01d4cd`, `xtask/src/e2e.rs`, `.github/workflows/ci.yml` |
| `pr9-6` | nit (inferred) | `xtask/src/drivers.rs`, test helper `temp_dir`, lines 101-105 | `temp_dir` does not empty a folder that an earlier run with the same process ID left.  So a test can read files of an earlier run. | `c01d4cd`, `xtask/src/drivers.rs` |

n = 6.

### m9

| Base | Reviewed | Fix commit | Old round | Criteria |
| - | - | - | - | - |
| `2af33b1` | `59816dc` | `4b27829` | review round 1 | `docs/plans/active/harness-phase-2.md#M9: young crates are pinned and the licence list is a test (TD-7, part; DEPS-1, DOC-1; maintainer decision)` |

The Progress records 1 should-fix and 7 nits, all real and fixed.  It names the should-fix and 7 new test cases; `m9-2` to `m9-8` are those 7 cases.  All places are in `xtask/src/rules.rs`.

| ID | Severity | Where | Defect | Evidence |
| - | - | - | - | - |
| `m9-1` | should-fix | `third_party_matches`, lines 2099-2185 (the second table, lines 2143-2146); test `a_third_party_drift_is_named`, lines 4763-4897 | No test covers the second table of `THIRD-PARTY.md`.  A check that drops its comparison, or that gives its rows a wrong line offset, passes the tests. | `4b27829`, `xtask/src/rules.rs` |
| `m9-2` | nit | `sets_a_bound_of_1`, lines 1744-1759; test `pins_and_settled_crates_pass`, lines 4459-4484 | No test passes the requirements `~1` and `=1`.  So the tests do not fail a change to how `sets_a_bound_of_1` reads `~` or `=`. | `4b27829`, `xtask/src/rules.rs` |
| `m9-3` | nit | `is_settled_above_1`, lines 1732-1742, and `is_pin`, lines 1717-1730; test `a_young_crate_with_a_range_is_named`, lines 4486-4532 | No test has a pre-release in a second comparator, such as `>=1.0.0-rc.1, <2`.  So a check of the first comparator only passes.  No pin has a part that is not a number, or a second comparator. | `4b27829`, `xtask/src/rules.rs` |
| `m9-4` | nit | `third_party_matches`, the licence check, lines 2147-2162; test `a_third_party_drift_is_named`, lines 4763-4897 | No test has a crate without a licence that only the second command gives (a dev-only crate).  So a licence check of the first command's output only passes. | `4b27829`, `xtask/src/rules.rs` |
| `m9-5` | nit | `third_party_matches`, the command check, lines 2117-2133; test `a_third_party_drift_is_named`, lines 4763-4897 | No test puts the second command above the heading "## Browser tests only".  So a check that finds the second command anywhere in the file passes. | `4b27829`, `xtask/src/rules.rs` |
| `m9-6` | nit | `is_date`, lines 1761-1767; test `a_stale_settled_entry_is_named`, lines 4566-4586 | No test gives a `SETTLED_CRATES` date of more than 10 characters.  So a length check that admits a longer date passes. | `4b27829`, `xtask/src/rules.rs` |
| `m9-7` | nit | `young_crates_are_pinned_in`, lines 1777-1914 (the "read nothing" problem, lines 1900-1906); test `nothing_to_read_is_a_problem`, lines 4588-4603 | No test has a member whose only dependency comes from git.  So a "read nothing" test on all dependencies, not on registry dependencies, passes. | `4b27829`, `xtask/src/rules.rs` |
| `m9-8` | nit | `third_party_matches`, the `wgpu` and `eframe` guard, lines 2163-2175; test `a_third_party_drift_is_named`, lines 4763-4897 | No test gives a first command's output without `wgpu`, while the second command's output has it.  So a guard that reads the wrong output passes. | `4b27829`, `xtask/src/rules.rs` |

n = 8.

Left out: the DEPS-1 text in `docs/invariants.md` and the TD-7 text (docs only).

### m11

| Base | Reviewed | Fix commit | Old round | Criteria |
| - | - | - | - | - |
| `7cd8518` | `9cd4d3e` | `da675e4` | review round 1 | `docs/plans/active/harness-phase-2.md#M11: the audio callback does not allocate after warm-up (closes TD-15; RT-1, DEPS-1, DOC-1; maintainer decision, made 2026-10-19)` |

The Progress records 1 should-fix and 4 nits.

| ID | Severity | Where | Defect | Evidence |
| - | - | - | - | - |
| `m11-1` | should-fix | `engine/src/stretch/wsola_adapter.rs`, `WsolaStretcher::warm_up`, lines 77-97; the promise in `engine/src/stretch/mod.rs`, `Stretcher::warm_up`, lines 34-39 | `Stretcher::warm_up` promises that a push of up to 4,096 frames does not allocate.  wsola's warm-up covers only the player's 256-frame push, so at 8 and 16 kHz a push allocates. | `da675e4`, `engine/src/stretch/wsola_adapter.rs`, test `a_stretcher_keeps_its_warm_up_promise` in `engine/tests/realtime.rs` |

n = 1.

Left out: the RT-1 title, the comment on `try_with` in `engine/tests/realtime.rs`, and wsola's doc on `max_tempo` (docs or comments only).  Also 22,050 Hz in `RATES` (left with no change).

### m2

| Base | Reviewed | Fix commit | Old round | Criteria |
| - | - | - | - | - |
| `4691a18` | `c40611c` | `280e800` | review round 2 | `docs/plans/active/harness-phase-2.md#M2: every shader is checked against the default limits (closes TD-1; GPU-1)` |

The Progress records 7 findings for round 2, all real and fixed, with no severities.

| ID | Severity | Where | Defect | Evidence |
| - | - | - | - | - |
| `m2-1` | should-fix (inferred) | `xtask/src/docs.rs`, `without_closed`, lines 716-726, which `id_references` calls, lines 831-840 | In the ID files, the ID check leaves out the whole `## Closed` section, not only the ID that opens each bullet.  So the text of a closed bullet can name an undefined ID. | `280e800`, `xtask/src/docs.rs` (test `a_closed_section_elsewhere_is_checked`) |
| `m2-2` | nit (inferred) | `roformer-gpu/src/lib.rs`, test helper `limit_breaches`, lines 2000-2144; tests `limit_breaches_are_reported`, lines 2188-2229, and `too_many_buffers_are_reported`, lines 2361-2377 | No test gives a value equal to its limit.  So a comparison that fails at the limit, such as `>=` on the buffer count, passes. | `280e800`, `roformer-gpu/src/lib.rs` (test `values_at_the_limits_pass`) |
| `m2-3` | should-fix (inferred) | `roformer-gpu/src/lib.rs`, `limit_breaches`, the loop over entry points, lines 2086-2143; tests `limit_breaches_are_reported`, lines 2188-2229, and `workgroup_storage_is_counted_as_webgpu_counts_it`, lines 2331-2359 | No test has a breach in an entry point after the first.  So a check of the first entry point only, or one that takes the first entry point's workgroup variables for each, passes. | `280e800`, `roformer-gpu/src/lib.rs` (test `every_entry_point_is_checked`) |
| `m2-4` | should-fix (inferred) | `roformer-gpu/src/lib.rs`, test `every_kernel_fits_the_default_limits`, lines 2164-2186 | The test lists the shader files by hand.  Nothing fails when a new file in `roformer-gpu/src/shaders/` is not in that list, so the test never checks that shader. | `280e800`, `roformer-gpu/src/lib.rs` (test `every_shader_file_is_checked`) |
| `m2-5` | nit (inferred) | `roformer-gpu/src/lib.rs`, `limit_breaches`, the `maxBindGroups` check, lines 2073-2084 | The `maxBindGroups` message computes `max - 1` on a `u32`, which underflows when the limit is 0. | `280e800`, `roformer-gpu/src/lib.rs` |

n = 5.

Left out: the doc comment of `every_layout_matches_its_shaders` (comment only) and one sentence of `docs/tech-debt.md` (wording only).  Round 1's findings are also in that Progress.

### m10

| Base | Reviewed | Fix commit | Old round | Criteria |
| - | - | - | - | - |
| `7a99a88` | `5bd0298` | `d67db75` | review round 2 | `docs/plans/active/harness-phase-2.md#M10: the hand-written e2e messages match the protocol (TD-16, part; PROTO-1, DOC-1)` |

The Progress records 1 should-fix and 4 nits for round 2.  All places are in `xtask/src/rules.rs`.

| ID | Severity | Where | Defect | Evidence |
| - | - | - | - | - |
| `m10-1` | should-fix | `js_after_typeof`, lines 2489-2499, which `js_types` calls, lines 2527-2556; test `a_misspelt_type_is_named`, lines 5681-5728 | No case compares a type after an earlier, unrelated `typeof`.  So a `js_after_typeof` that skips every type after any `typeof` passes the tests. | `d67db75`, `xtask/src/rules.rs` |
| `m10-2` | nit | `rust_types`, lines 2608-2657; test `other_type_words_are_not_types`, lines 5730-5765 | No Rust case pins the bracket rule or the brace rule.  `c["state"] == "running"`, `f("type") == "x"` and `m!["type": "x"]` are not types, but a check that reads them passes. | `d67db75`, `xtask/src/rules.rs` |

n = 2.

Left out: a long doc comment line of `js_tokens` (comment only), and the regex literal and the false type in TD-16 (docs only).

### m18a

| Base | Reviewed | Fix commit | Old round | Criteria |
| - | - | - | - | - |
| `32b2731` | `c4eab24` | `e63469e` | review round 1 | `docs/plans/active/harness-phase-2.md#M18: the breaches the checks found (closes TD-12, TD-13, TD-14, TD-17; MEM-1, MEM-2, MEM-5, DOC-1; maintainer decision)` |

The Progress records 3 should-fix and 5 nits.  The JavaScript functions are in the `inline_js` blocks of their files.

| ID | Severity | Where | Defect | Evidence |
| - | - | - | - | - |
| `m18a-1` | should-fix | `app/src/store.rs`, JS `whole`, lines 34-39; test `a_view_is_written_whole`, lines 407-420, with `lloyd_in_a_larger_buffer`, lines 395-399 | The test writes one view, in the middle of its buffer.  So a `whole` that checks only the offset, or only the length, passes it. | `e63469e`, `app/src/store.rs` |
| `m18a-2` | should-fix | `app/src/store.rs`, JS `st_write`, lines 40-52, and `save_sources`, lines 196-224; test `sources_are_saved_from_their_files`, lines 428-445, with JS `start_spy`, lines 376-389 | The spy records "blob" for any `Blob`.  So a JS copy of the song's file, such as `new Blob([body])`, passes the test. | `e63469e`, `app/src/store.rs` |
| `m18a-3` | should-fix | `app/src/store.rs`, `save_sources`, lines 196-224, and JS `st_write`, lines 40-52 | A write that fails after the song's file leaves the session folder.  That session has no `meta`, so it is not listed and holds the file for good.  `st_write` does not abort its writer. | `e63469e`, `app/src/store.rs` (test `a_failed_save_leaves_no_session`) |
| `m18a-4` | nit | `app/src/store.rs`, test JS `start_spy`, lines 376-389; `worker/src/web/tests.rs`, JS `start_spy`, lines 79-95 | A second `start_spy` before `stop_spy` keeps the spy as the real `write`.  Then `stop_spy` cannot put back the real `write`. | `e63469e`, `app/src/store.rs`, `worker/src/web/tests.rs` |

n = 4.

Left out: the doc of `st_write`'s two types and two comments (docs or comments only), and the Safari step (not taken, done before the review).
