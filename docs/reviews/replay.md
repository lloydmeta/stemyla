# Replay of six old review rounds

This file records M5 of harness phase 3.  M5 runs six old review rounds of this repo again, in a test harness, with the review agents and the `review-rounds` skill of this branch.  It measures how many findings of each old round the new review finds again (x of n).  The criteria are in `docs/plans/active/harness-phase-3.md`, section "M5: replay old reviews" ([link](../plans/active/harness-phase-3.md#m5-replay-old-reviews)).  The milestone plan is `docs/plans/active/harness-phase-3-m5.md`.

The ground truth below was written from git only, before any case run.  The first commit of this file holds it: `git log --reverse --format='%h %cI' -- docs/reviews/replay.md | head -1` gives that commit and its time.  The dry run and the case runs start after that time.

After the ground truth, the file holds the rest of step 6 of the milestone plan.  These are the harness, the result, the cost, the reviewers' counts, the outside findings, the flags and the threats.

How the ground truth was made:

* A ground-truth item is one finding of the old round whose fix commit changed code or tests for it.  Its ID is `<case>-<n>`.
* The sources are the fix commit's message and diff.  For a phase 2 case, they also include the Progress of `docs/plans/active/harness-phase-2.md` at the fix commit.
* A finding is left out when its fix changed only docs, comments or wording.  A finding is also left out when the old round refuted it, or left it with no change.  For `m2` and `m10`, round 1's findings are left out.
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

The Progress records 1 should-fix and 7 nits, all real and fixed.  It names the should-fix and 7 new test cases; `m9-2` to `m9-8` are assumed to be those 7 cases.  All places are in `xtask/src/rules.rs`.

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

Added after the runs: the fix commit `4b27829` has 10 changes, and Progress records 8 findings.  So the match of the 7 nits to `m9-2` to `m9-8` is an assumption (`## Threats`).

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

## Harness

The test harness H is the folder /private/tmp/stemyla-harness/.  M3 made it, and `H/bin/make-harness.sh` built it again at `4ccf2e9`, the commit of the ground truth.  The agents and the skill ran as M3 froze them at `de34663`, with M4's change to steps 9 and 11 of the skill.  No case reaches step 11, and no check reads the new field of step 9.

Each case run used the model `claude-opus-5-5` with the effort `high`.  `STEMYLA_MUTANTS_MINUTES` was not set, so the mutants run had the default limit of 60 minutes.

H is under `/private/tmp`, which macOS clears at a restart.  So the full text of the two scripts follows.

`replay-clone.sh <case> <clone>` makes the clone of one case.  The clone holds only the history up to the reviewed commit, and the script checks that the fix commit is not in it:

```bash
#!/bin/bash
# Makes the clone of one M5 replay case (docs/plans/active/harness-phase-3-m5.md,
# step 3): H/<clone> on the branch replay-<case>, with only the history up to
# the reviewed commit.  Each git command after the clone runs as `git -C`, so
# no command runs in the main checkout.  Exits 1 if a check fails.
# Usage: replay-clone.sh <case> <clone>
set -euo pipefail

repo=/Users/lloyd/Documents/rustt/stemyla
harness=/private/tmp/stemyla-harness
min_free_gib=30

if [ $# -ne 2 ]; then
  echo "usage: replay-clone.sh <case> <clone>" >&2
  exit 1
fi
case_name=$1
clone_name=$2

# The case table of the plan: the reviewed commit and the fix commit.
case $case_name in
  pr9) reviewed=ba1b59227406443fd5ea6cb52a78a68991cf5d18 fix=c01d4cdfd6bf8c0c79c4e838daac8aa86591e0c2 ;;
  m9) reviewed=59816dc2f9cbccb079d08627608bee2f1cd68d87 fix=4b27829eb33ec6f98671af2af32043e8097971c5 ;;
  m11) reviewed=9cd4d3e3a2644cd9d14d3f6fd3c17133342856d3 fix=da675e4e853c29487521b1e785cf732bb2f22c66 ;;
  m2) reviewed=c40611ca36a945c2a791b0537fe633d8cc8a9fd5 fix=280e800c6db14a30c7ec94df7553a02d5b9f049e ;;
  m10) reviewed=5bd02987991ed94e6fbb2b21ddb1fd22809f5534 fix=d67db7511066c5e74195224be775b3b19292f763 ;;
  m18a) reviewed=c4eab24fc252be8f7a04655241a3b55d9c3abca8 fix=e63469ef19788570543a9e0e8e91172075b17d29 ;;
  *)
    echo "replay-clone: unknown case: $case_name" >&2
    exit 1
    ;;
esac

case $clone_name in
  "" | */* | .*)
    echo "replay-clone: the clone must be a plain folder name: $clone_name" >&2
    exit 1
    ;;
esac

tag=replay/$case_name
branch=replay-$case_name
clone=$harness/$clone_name

if [ -e "$clone" ]; then
  echo "replay-clone: $clone exists" >&2
  exit 1
fi

fail() {
  echo "replay-clone: check failed: $1" >&2
  exit 1
}

git -c advice.detachedHead=false clone -q --no-local --single-branch --no-tags --branch "$tag" "$repo" "$clone"
git -C "$clone" checkout -q -b "$branch"
git -C "$clone" tag -d "$tag" > /dev/null
git -C "$clone" remote remove origin
printf '%s\n' '!/Cargo.lock' >> "$clone/.git/info/exclude"

ignored=$(git -C "$clone" ls-files -ci --exclude-standard)
[ -z "$ignored" ] || fail "tracked files are ignored: $ignored"

head=$(git -C "$clone" rev-parse HEAD)
[ "$head" = "$reviewed" ] || fail "HEAD is $head, not $reviewed"

refs=$(git -C "$clone" for-each-ref --format='%(refname)')
[ "$refs" = "refs/heads/$branch" ] || fail "the refs are not only refs/heads/$branch: $refs"

if git -C "$clone" cat-file -e "$fix^{commit}" 2> /dev/null; then
  fail "the fix commit $fix is in the clone"
fi

status=$(git -C "$clone" status --porcelain)
[ -z "$status" ] || fail "the clone is not clean: $status"

free_kib=$(df -k "$harness" | awk 'NR == 2 { print $4 }')
[ "$free_kib" -ge $((min_free_gib * 1024 * 1024)) ] || fail "less than $min_free_gib GiB free: $free_kib KiB"

echo "replay-clone: $clone on $branch at $head; checks pass"
```

`replay-check.sh <clone> [<transcript>]` checks one case run.  It reads the transcript with `jq` only:

```bash
#!/bin/bash
# Checks one M5 replay case run (docs/plans/active/harness-phase-3-m5.md,
# step 3, item 4).  It prints one line for each check, `pass` or `FAIL` with
# the reason; the Tripwire can also print `CHECK`, for the team leader to
# resolve.  Denials, Cost and the reviewer counts print values only.
# It reads the transcript with jq only.  For bash 3.2 (macOS).
# Usage: replay-check.sh <clone> [<transcript>]
set -euo pipefail

harness=/private/tmp/stemyla-harness
guard=$harness/bin/read-only-bash.sh

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "usage: replay-check.sh <clone> [<transcript>]" >&2
  exit 1
fi
clone_name=$1
case $clone_name in
  "" | */* | .*)
    echo "replay-check: the clone must be a plain folder name: $clone_name" >&2
    exit 1
    ;;
esac
clone=$harness/$clone_name
transcript=${2:-$harness/$clone_name.jsonl}
time_file=$harness/$clone_name.time

if [ ! -d "$clone" ]; then
  echo "replay-check: no clone $clone" >&2
  exit 1
fi
if [ ! -f "$transcript" ]; then
  echo "replay-check: no transcript $transcript" >&2
  exit 1
fi

# jq over the whole transcript as one array.
tq() {
  jq -s "$@" "$transcript"
}

# One folder target/review/run/*/ in the clone, else none.
run=""
run_problem=""
shopt -s nullglob
runs=("$clone"/target/review/run/*/)
shopt -u nullglob
if [ ${#runs[@]} -eq 1 ]; then
  run=${runs[0]%/}
elif [ ${#runs[@]} -eq 0 ]; then
  run_problem="no run folder in $clone/target/review/run/"
else
  run_problem="${#runs[@]} run folders in $clone/target/review/run/"
fi
state=""
reviewed=""
if [ -n "$run" ]; then
  state=$run/1/state.json
  if [ -f "$state" ]; then
    reviewed=$(jq -r '.head // empty' "$state")
  fi
fi
findings=""
if [ -n "$run" ]; then
  findings=$run/1/findings.md
fi

echo "replay-check: clone $clone"
echo "replay-check: transcript $transcript"
echo "replay-check: run ${run:-none}; reviewed ${reviewed:-unknown}"

# Result.
result_line=$(tq -r '
  [.[] | select(.type == "result")] | last
  | if . == null then "FAIL: no result message"
    elif .is_error == false and .subtype == "success" then "pass"
    else "FAIL: is_error \(.is_error), subtype \(.subtype)"
    end')
echo "Result: $result_line"

# Load.
stderr_count=$(tq '[.[] | select(.isReplay == true) | select(tostring | contains("local-command-stderr"))] | length')
load_problems=""
if [ "$stderr_count" -ne 0 ]; then
  load_problems="$stderr_count isReplay messages hold local-command-stderr"
fi
if [ -z "$run" ]; then
  load_problems="${load_problems:+$load_problems; }$run_problem"
elif [ ! -f "$state" ]; then
  load_problems="${load_problems:+$load_problems; }no $state"
fi
if [ -z "$load_problems" ]; then
  echo "Load: pass"
else
  echo "Load: FAIL: $load_problems"
fi

# Plugin.
plugin_line=$(tq -r '
  [.[] | select(.type == "system" and .subtype == "init")] as $inits
  | [$inits[] | select((.plugin_errors // []) | length != 0)] as $bad
  | if ($inits | length) == 0 then "FAIL: no system/init message"
    elif ($bad | length) == 0 then "pass (\($inits | length) init messages)"
    else "FAIL: \($bad | length) of \($inits | length) init messages have plugin_errors: \([$bad[].plugin_errors] | tostring)"
    end')
echo "Plugin: $plugin_line"

# Mutants.
if [ -z "$run" ]; then
  echo "Mutants: FAIL: $run_problem"
elif [ ! -f "$run/1/mutants.txt" ]; then
  echo "Mutants: FAIL: no $run/1/mutants.txt"
else
  mutants=$(head -n 1 "$run/1/mutants.txt")
  case $mutants in
    "mutants: full:"* | "mutants: partial:"*) echo "Mutants: pass: $mutants" ;;
    *) echo "Mutants: FAIL: $mutants" ;;
  esac
fi

# Route.
if [ -z "$run" ]; then
  echo "Route: FAIL: $run_problem"
elif [ ! -f "$run/1/route.txt" ] || [ ! -f "$findings" ]; then
  echo "Route: FAIL: no route.txt or no findings.md in $run/1/"
else
  routed=$(sed -n 's/^  \([A-Za-z0-9_-]*\):.*/\1/p' "$run/1/route.txt" | sort -u)
  agent_rows=$(awk '
    /^## / { in_agents = ($0 == "## Agents"); next }
    in_agents && /^\|/ {
      cell = $0
      sub(/^\| */, "", cell)
      sub(/ *\|.*$/, "", cell)
      print cell
    }' "$findings")
  # The Agent types with at least one call that completed: a background call
  # with a completed task_notification, or a foreground call whose result is
  # neither async_launched nor an error.
  completed_types=$(tq -r '
    [.[] | select(.type == "system" and .subtype == "task_notification" and .status == "completed") | .tool_use_id] as $notified
    | [.[] | select(.type == "user") | . as $message
       | (.message.content | if type == "array" then .[] else empty end)
       | select(type == "object" and .type == "tool_result")
       | select(.is_error != true)
       | select((($message.tool_use_result | objects | .status) // "") != "async_launched")
       | .tool_use_id] as $foreground
    | ($notified + $foreground) as $done
    | [.[] | select(.type == "assistant") | .message.content[]?
       | select(.type == "tool_use" and .name == "Agent")
       | select(.id as $id | $done | index($id))
       | .input.subagent_type // "general-purpose"]
    | unique | .[]')
  route_problems=""
  if [ -z "$routed" ]; then
    route_problems="route.txt names no reviewer"
  fi
  for name in $routed; do
    if ! printf '%s\n' "$agent_rows" | grep -qx -- "$name"; then
      route_problems="${route_problems:+$route_problems; }$name has no row in ## Agents"
    fi
    if ! printf '%s\n' "$completed_types" | grep -qx -- "$name"; then
      route_problems="${route_problems:+$route_problems; }$name has no completed Agent call"
    fi
  done
  if [ -z "$route_problems" ]; then
    echo "Route: pass ($(printf '%s' "$routed" | tr '\n' ' ' | sed 's/ *$//'))"
  else
    echo "Route: FAIL: $route_problems"
  fi
fi

# Review only.
if [ -z "$run" ]; then
  echo "Review only: FAIL: $run_problem"
elif [ -z "$reviewed" ]; then
  echo "Review only: FAIL: no head in $state"
else
  now=$(git -C "$clone" rev-parse HEAD)
  if [ "$now" != "$reviewed" ]; then
    echo "Review only: FAIL: HEAD is $now, not $reviewed"
  elif [ -e "$run/log.md" ]; then
    echo "Review only: FAIL: $run/log.md exists"
  else
    echo "Review only: pass"
  fi
fi

# Tripwire, on subagent calls only.
# (a) Each subagent Bash command, with a flag that its result holds the
# guard's reason, one JSON object to a line.
bash_calls=$(tq -c '
  [.[] | select(.type == "user" and .parent_tool_use_id != null)
   | (.message.content | if type == "array" then .[] else empty end)
   | select(type == "object" and .type == "tool_result")
   | select(.content | tostring | contains("read-only guard:"))
   | .tool_use_id] as $refused
  | .[] | select(.type == "assistant" and .parent_tool_use_id != null)
  | .message.content[]?
  | select(.type == "tool_use" and .name == "Bash" and (.input.command | type) == "string")
  | {id, refused_by_hook: (.id as $id | $refused | index($id) != null), tool_input: {command: .input.command}}')
tripwire_hits=""
while IFS= read -r call; do
  [ -n "$call" ] || continue
  if printf '%s\n' "$call" | "$guard" 2> /dev/null; then
    continue
  fi
  if [ "$(printf '%s\n' "$call" | jq -r '.refused_by_hook')" = "false" ]; then
    tripwire_hits="$tripwire_hits$(printf '%s\n' "$call" | jq -r '"  FAIL (a) the guard refuses it, but its result lacks the guard reason: Bash \(.id): \(.tool_input.command | gsub("\\s+"; " ") | .[0:300])"')"$'\n'
  fi
done <<< "$bash_calls"

# (b) a hit and (c) a CHECK, from each subagent tool input.
own_tmp_folder="-private-tmp-stemyla-harness-$clone_name"
read -r -d '' path_program << 'JQ' || true
  def stop: "[^/\\s'\"`;|&()<>]*";
  def names($prefix): [scan($prefix + "(" + stop + ")") | .[0]];
  def own_tmp($p):
    ($p | ltrimstr("/private")) as $q
    | ("/tmp/stemyla-harness/" + $own) as $a
    | ("/tmp/claude-501/" + $own_tmp) as $b
    | $q == $a or ($q | startswith($a + "/")) or $q == $b or ($q | startswith($b + "/"));
  def hits:
    [ (if contains("rustt/stemyla") then "rustt/stemyla" else empty end),
      (if contains("target/review/run") then "target/review/run" else empty end),
      (if test("\\.claude/projects(?!/)") then ".claude/projects" else empty end),
      (names(".claude/projects/")[] | select(. != $own_tmp) | ".claude/projects/" + .),
      (names("stemyla-harness/")[] | select(. != $own and . != "bin") | "stemyla-harness/" + .),
      (names("claude-501/")[] | select(. != $own_tmp) | "claude-501/" + .) ];
  def checks:
    [ (if contains("../") then "../" else empty end),
      (if contains("~/") then "~/" else empty end),
      (if contains("$HOME") then "$HOME" else empty end),
      (if contains("/Users/") then "/Users/" else empty end),
      ([scan("(?<![\\w.~/-])(?:/private)?/tmp(?![\\w.-])" + stop + "(?:/" + stop + ")*")]
       | map(select(own_tmp(.) | not)) | unique | .[]) ];
  .[] | select(.type == "assistant" and .parent_tool_use_id != null)
  | .message.content[]? | select(.type == "tool_use")
  | . as $call
  | [.input.command, .input.file_path, .input.path, .input.pattern] | map(strings) as $inputs
  | ([$inputs[] | hits[]] | unique) as $hits
  | ([$inputs[] | checks[]] | unique) as $checks
  | ($inputs | join(" | ") | gsub("\\s+"; " ") | .[0:300]) as $text
  | if ($hits | length) > 0 then
      "  FAIL (b) \($hits | join(", ")): \($call.name) \($call.id): \($text)"
    elif ($checks | length) > 0 then
      "  CHECK (c) \($checks | join(", ")): \($call.name) \($call.id): \($text)"
    else empty end
JQ
path_findings=$(tq -r --arg own "$clone_name" --arg own_tmp "$own_tmp_folder" "$path_program")
if [ -n "$path_findings" ]; then
  tripwire_hits="$tripwire_hits$path_findings"$'\n'
fi
fail_count=$(printf '%s' "$tripwire_hits" | grep -c '^  FAIL' || true)
check_count=$(printf '%s' "$tripwire_hits" | grep -c '^  CHECK' || true)
if [ "$fail_count" -gt 0 ]; then
  echo "Tripwire: FAIL: $fail_count hits, $check_count calls to check"
elif [ "$check_count" -gt 0 ]; then
  echo "Tripwire: CHECK: $check_count calls to check"
else
  echo "Tripwire: pass"
fi
printf '%s' "$tripwire_hits"

# Denials: the dontAsk denials, each with its command.
tq -r '
  (map(select(.type == "assistant") | .message.content[]? | select(.type == "tool_use") | {key: .id, value: .}) | from_entries) as $calls
  | [.[] | select(.type == "system" and .subtype == "permission_denied" and .decision_reason_type == "mode")] as $denials
  | "Denials: \($denials | length) dontAsk",
    ($denials[]
     | ($calls[.tool_use_id] // {}) as $call
     | "  \(.tool_name) \(.tool_use_id) (agent \(.agent_id // "main")): \(($call.input.command // $call.input.file_path // ($call.input | tostring)) | gsub("\\s+"; " ") | .[0:300])")'

# Cost, printed, no check.
echo "Cost:"
tq -r '
  [.[] | select(.type == "assistant") | .message.content[]? | select(.type == "tool_use" and .name == "Agent")] as $agents
  | ([.[] | select(.type == "result")] | last) as $result
  | ($result.modelUsage // {} | [.[]]) as $models
  | [.[] | select(.type == "system" and .subtype == "task_notification" and .usage != null)] as $notices
  | [.[] | select(.type == "user") | . as $message
     | (.message.content | if type == "array" then .[] else empty end)
     | select(type == "object" and .type == "tool_result")
     | {id: .tool_use_id, total: ($message.tool_use_result | objects | .totalTokens)}
     | select(.total != null)] as $foreground
  | ($models | map(.inputTokens // 0) | add // 0) as $input
  | ($models | map(.outputTokens // 0) | add // 0) as $output
  | ($models | map(.cacheCreationInputTokens // 0) | add // 0) as $creation
  | ($models | map(.cacheReadInputTokens // 0) | add // 0) as $read
  | "  Agent calls: \($agents | length): \($agents | group_by(.input.subagent_type // "general-purpose") | map("\(.[0].input.subagent_type // "general-purpose") \(length)") | join(", "))",
    "  session tokens (modelUsage): input \($input), output \($output), cache creation \($creation), cache read \($read); sum \($input + $output + $creation + $read)",
    "  total_cost_usd: \($result.total_cost_usd // "none") (list price, costBasis \([$models[] | .costBasis // "none"] | unique | join(",")))",
    ($agents[]
     | .id as $id
     | ([$notices[] | select(.tool_use_id == $id)] | last | .usage.total_tokens) as $background
     | ([$foreground[] | select(.id == $id)] | last | .total) as $fg
     | "  final context: \(.input.subagent_type // "general-purpose") \($id): \(if $background != null then "\($background) (task_notification)" elif $fg != null then "\($fg) (foreground totalTokens)" else "none" end)")'
if [ ! -f "$time_file" ]; then
  echo "  wall time: no time file"
else
  start=$(sed -n 1p "$time_file")
  end_line=$(sed -n 2p "$time_file")
  if [ -z "$end_line" ]; then
    echo "  wall time: no end time in $time_file (start $start)"
  else
    exit_status=${end_line%% *}
    end=${end_line##* }
    seconds=$((end - start))
    echo "  wall time: $seconds s ($((seconds / 60)) min $((seconds % 60)) s); claude exit $exit_status"
  fi
fi
tq -r '
  [.[] | select(.type == "system" and .subtype == "permission_denied" and .decision_reason_type == "mode")] as $denials
  | [.[] | select(.type == "user" and .parent_tool_use_id != null)
     | (.message.content | if type == "array" then .[] else empty end)
     | select(type == "object" and .type == "tool_result")
     | select(.content | tostring | contains("read-only guard:"))] as $refusals
  | "  dontAsk denials: \($denials | length)",
    "  guard refusals: \($refusals | length)"'

# Each reviewer's counts, from findings.md.
echo "Counts:"
if [ -z "$findings" ] || [ ! -f "$findings" ]; then
  echo "  no findings.md"
else
  counts=$(awk '
    function flush(   n, i, parts, source) {
      if (sources == "") { return }
      n = split(sources, parts, /, */)
      for (i = 1; i <= n; i++) {
        source = parts[i]
        seen[source] = 1
        count[source, verdict]++
      }
    }
    /^### / { flush(); sources = ""; verdict = "none"; next }
    /^## / { flush(); sources = ""; verdict = "none"; next }
    /^\* sources: / { sources = substr($0, 12) }
    /^\* verdict: / {
      verdict = substr($0, 12)
      sub(/[^a-z-].*$/, "", verdict)
    }
    END {
      flush()
      for (source in seen) {
        line = sprintf("  %s: confirmed %d, pre-existing %d, refuted %d, unproven %d, unvalidated %d",
          source, count[source, "confirmed"], count[source, "pre-existing"],
          count[source, "refuted"], count[source, "unproven"], count[source, "unvalidated"])
        print line
      }
    }' "$findings" | sort)
  if [ -n "$counts" ]; then
    printf '%s\n' "$counts"
  else
    echo "  no findings"
  fi
fi
```

The case command (step 3 of the milestone plan).  The team leader ran it as one background Bash command, with the allow list as run.  The allow list is M3's settled list, with `Bash(rustc --version)` added after dry run 1:

```bash
date +%s > /private/tmp/stemyla-harness/<clone>.time
cd /private/tmp/stemyla-harness/<clone>
claude -p "/stemyla:review-rounds <base, full SHA> 1 \"<criteria>\" --review-only" \
  --setting-sources project,local --model claude-opus-5-5 --effort high \
  --agents ../agents.json --plugin-dir ../stemyla --permission-mode dontAsk \
  --allowedTools 'Read' 'Write' 'Agent' \
    'Bash(/private/tmp/stemyla-harness/bin/xtask *)' 'Bash(cargo *)' 'Bash(git *)' \
    'Bash(grep *)' 'Bash(find *)' 'Bash(sed *)' 'Bash(ls *)' 'Bash(cat *)' \
    'Bash(rm -rf target/review/*)' 'Bash(head *)' 'Bash(tail *)' 'Bash(wc *)' \
    'Bash(jq *)' 'Bash(sort *)' 'Bash(diff *)' 'Bash(echo *)' 'Bash(rustc --version)' \
  --output-format stream-json --verbose \
  < /dev/null > ../<clone>.jsonl
echo "$? $(date +%s)" >> /private/tmp/stemyla-harness/<clone>.time
```

The base and the criteria of each case are in its table under `## Ground truth`.

## Result

The comparers were one new general-purpose subagent for each case.  Each got the case's ground-truth table and the case's found findings and unvalidated nits, and read no file.  A finding ID such as `1-3` is the ID of the finding in the case run's `findings.md`.  An item of severity `nit` can also be found by an unvalidated nit, marked `found (unvalidated)`.

| Case | Found (the parent plan's count) | With `found (unvalidated)` |
| - | - | - |
| `pr9` | 0/6 | 0/6 |
| `m9` | 2/8 | 3/8 |
| `m11` | 1/1 | 1/1 |
| `m2` | 3/5 | 3/5 |
| `m10` | 1/2 | 1/2 |
| `m18a` | 1/4 | 1/4 |
| Total | 8/26 | 9/26 |

### pr9: 0/6

| ID | Result | By | Finding | Quote |
| - | - | - | - | - |
| `pr9-1` | missed | - | - | - |
| `pr9-2` | missed | - | - | - |
| `pr9-3` | missed | - | - | - |
| `pr9-4` | missed | - | - | - |
| `pr9-5` | missed | - | - | - |
| `pr9-6` | missed | - | - | - |

The comparer's notes:

* `pr9-2`: 1-16 is a missed mutant in `Page::errors`, but `pr9-2` is not a weak-test item.
* `pr9-3`: 1-25 is in `smoke` at line 448, outside the tempo check, and `pr9-3` is not a weak-test item.
* `pr9-4`: 1-1 is a weak test of `sends_json_and_reads_a_sized_reply`, not the socket timeout of `http`.
* `pr9-5`: 1-28 is a mutant of `Page::finish`, and `pr9-5` is not a weak-test item.
* `pr9-6`: no finding names `temp_dir`.

### m9: 3/8, or 2/8 without the unvalidated item

| ID | Result | By | Finding | Quote |
| - | - | - | - | - |
| `m9-1` | found | supply-chain-reviewer | 1-3 | "No test checks the second table ("Browser tests only").  If the check stops comparing that table with its command, every test still passes" |
| `m9-2` | found (unvalidated) | reviewer | 1-5 (nit) | "The design lists `~` and `=` with M of 1 or more as settled above 1.0, but no fixture uses them." |
| `m9-3` | missed | - | - | - |
| `m9-4` | found | docs-reviewer, supply-chain-reviewer | 1-2 | "Only a shipped crate without a licence is tested.  If the check reads only the first command's rows, every test still passes." |
| `m9-5` | missed | - | - | - |
| `m9-6` | missed | - | - | - |
| `m9-7` | missed | - | - | - |
| `m9-8` | missed | - | - | - |

The comparer's note on `m9-1`: 1-3 states only the half "drops its comparison".  The nit 1-4 is near the half "wrong line offset", but a nit cannot match a should-fix item.

### m11: 1/1

| ID | Result | By | Finding | Quote |
| - | - | - | - | - |
| `m11-1` | found | memory-reviewer | 1-3 | "The `Stretcher::warm_up` contract promises no allocation for pushes of up to 4,096 frames.  `WsolaStretcher` sizes its input only for 256-frame pushes" |

The comparer's note: 1-10, a mutant of `WsolaStretcher::warm_up`, does not match, since `m11-1` is not a weak-test item.

### m2: 3/5

| ID | Result | By | Finding | Quote |
| - | - | - | - | - |
| `m2-1` | found | reviewer, docs-reviewer | 1-3 | "the ID check leaves out the whole `## Closed` section.  A wrong ID in the text of a closed bullet (such as [an undefined TD number]) is never reported." |
| `m2-2` | found | supply-chain-reviewer | 1-5 | "No test checks the buffer-count limits at the edge.  The test uses 9 and 13 buffers only.  An off-by-one (`>=`) ... passes every test." |
| `m2-3` | missed | - | - | - |
| `m2-4` | found | reviewer, gpu-reviewer, supply-chain-reviewer, docs-reviewer | 1-1 | "'Every shader' rests on a hand list of three files ... A shader file that `Kernels::new` compiles but the list leaves out is never checked." |
| `m2-5` | missed | - | - | - |

The comparer's note on `m2-3`: 1-4 is in the same loop, but states another defect (read-only buffers, y and z).

### m10: 1/2

| ID | Result | By | Finding | Quote |
| - | - | - | - | - |
| `m10-1` | missed | - | - | - |
| `m10-2` | found | cargo-mutants; also the unvalidated nit of reviewer and supply-chain-reviewer | 1-10; 1-4 (nit) | 1-10: "no test fails with this mutant: xtask/src/rules.rs:2624:21: replace && with \|\| in rust_types".  1-4: "No test pins the rule that a `"type": "x"` key counts only in a brace group.  If `in_braces` is replaced with `true`, all 7 proto_tests pass." |

The notes:

* `m10-1`: 1-12 is in `js_included_types`, another function, and states another defect.
* `m10-2`: the comparer was not sure if mutant 1-10 (`rust_types`, line 2624, `&&` to `||`) is the bracket rule.  The team leader first read line 2624 at `5bd0298` as the `==` or `!=` test in `compares`, and marked no match.  That reading narrowed the written rule after the results were seen.  Step 6.2 says: "A missed mutant matches an item that is a weak test of the same function."  1-10 is in `rust_types`, and `m10-2` is a weak-test item of `rust_types`.  The written rule governs, so 1-10 matches.

### m18a: 1/4

| ID | Result | By | Finding | Quote |
| - | - | - | - | - |
| `m18a-1` | missed | - | - | - |
| `m18a-2` | found | memory-reviewer, reviewer | 1-3 | "The test passes for any Blob and never runs `load_and_keep`.  A Blob built in `load_and_keep` from a copy of the bytes passes every check." |
| `m18a-3` | missed | - | - | - |
| `m18a-4` | missed | - | - | - |

The comparer's note on `m18a-1`: the unvalidated nit 1-5 (reviewer) states its defect ("A `whole()` that checks only `byteOffset === 0` still passes it").  But a nit cannot match a should-fix item.

In `m18a` the harness denied every browser-test run, and only wasm tests in a browser reach its four items (`## Threats`).

### Total and bar

* The total, without the `found (unvalidated)` items (the parent plan's count): 8/26 = 0.31, with a 95% Wilson score interval ([Wilson, 1927](https://doi.org/10.1080/01621459.1927.10502953)) of 0.17 to 0.50.
* The total with the `found (unvalidated)` item `m9-2`: 9/26 = 0.35, with an interval of 0.19 to 0.54.  Only `m9-2` is found by an unvalidated nit alone.  This total is extra information, and the bar does not use it.
* The first reading was 7/26 = 0.27 (0.14 to 0.46), with no match for `m10-2` by cargo-mutants.  It was dropped, since it narrowed the written rule of step 6.2 (the notes of `m10`).
* The bar: a total of 0.8 or more, and each recorded `should-fix` or worse found.  The bar is not met.  The total is 0.31.  Of the recorded should-fix items, `m9-1`, `m11-1` and `m18a-2` are found, and `m10-1`, `m18a-1` and `m18a-3` are missed.  No item is a blocker.
* The inferred should-fix items do not enter the bar.  Of these, `pr9-1` to `pr9-4` and `m2-3` are missed, and `m2-1` and `m2-4` are found.

The found items by source.  An item with more than one source counts once for each source:

| Source | Found | Items |
| - | - | - |
| reviewer | 3 | `m2-1`, `m2-4`, `m18a-2` |
| supply-chain-reviewer | 4 | `m9-1`, `m9-4`, `m2-2`, `m2-4` |
| docs-reviewer | 3 | `m9-4`, `m2-1`, `m2-4` |
| memory-reviewer | 2 | `m11-1`, `m18a-2` |
| gpu-reviewer | 1 | `m2-4` |
| wire-reviewer | 0 | - |
| cargo-mutants | 1 | `m10-2` |

The unvalidated nits that match an item: 1-5 of `m9` (reviewer) finds `m9-2` alone.  1-4 of `m10` (reviewer and supply-chain-reviewer) also matches `m10-2`, which cargo-mutants finds.

Unvalidated nits that match no item are counted, not listed: `pr9` 6, `m9` 7, `m11` 4, `m2` 3, `m10` 5, `m18a` 4, a total of 29.

## Cost

The figures come from `replay-check.sh` on each clone.  The session tokens are the sum of input, output, cache creation and cache read tokens in the last `result`'s `modelUsage`, which covers the subagents.

The counted runs:

| Case | Clone | Agents (reviewers, validators) | Wall time (s) | Session tokens | Output tokens | `total_cost_usd` | `dontAsk` denials | Guard refusals | Mutants line |
| - | - | - | - | - | - | - | - | - | - |
| `pr9` | `replay-pr9-1` | 10 (6, 4) | 4,034 | 10,428,321 | 132,737 | 9.33 | 21 | 1 | partial: tested 56 (caught 11, missed 42, unviable 3), timeouts 1, not tested 262; stopped at the limit |
| `m9` | `replay-m9-1` | 7 (4, 3) | 1,554 | 6,404,096 | 89,278 | 5.79 | 12 | 6 | none: tested 0 ... not tested 106; exit 4: the baseline failed |
| `m11` | `replay-m11-2` | 8 (4, 4) | 1,496 | 17,011,037 | 181,201 | 10.13 | 36 | 4 | partial: tested 25 (caught 17, missed 7, unviable 1), timeouts 1, not tested 0; timeouts have no result |
| `m2` | `replay-m2-1` | 11 (4, 7) | 1,325 | 9,651,637 | 118,675 | 7.28 | 16 | 4 | none: tested 0 ... not tested 25; exit 4: the baseline failed |
| `m10` | `replay-m10-2` | 8 (4, 4) | 4,015 | 7,482,748 | 130,573 | 6.91 | 18 | 4 | partial: tested 124 (caught 119, missed 4, unviable 1), timeouts 8, not tested 130; stopped at the limit |
| `m18a` | `replay-m18a-1` | 7 (4, 3) | 972 | 6,266,253 | 87,615 | 5.10 | 23 | 2 | full: tested 1 (caught 1, missed 0, unviable 0), timeouts 0, not tested 0 |
| Total | | 51 | 13,396 (3.7 h) | 57,244,092 | 740,079 | 44.54 | 126 | 21 | |

Each agent's final context, from its last `task_notification`.  This figure is the size of the agent's last context, not the tokens that it used.  So it is not part of the session tokens.  It is the figure of the skill's `## Agents` table and of D4.  The ratio of tokens used to final context differs by agent.  So a sum of final contexts can misstate a ratio of tokens used by several times (TD-44).

* `pr9`: reviewer 115,520; gpu-reviewer 17,035; memory-reviewer 54,269; wire-reviewer 104,501; supply-chain-reviewer 97,524; docs-reviewer 60,035; validators 15,499, 19,670, 10,160 and 59,784.
* `m9`: reviewer 80,678; memory-reviewer 34,489; supply-chain-reviewer 72,062; docs-reviewer 77,978; validators 22,158, 21,418 and 21,235.
* `m11`: reviewer 98,176; memory-reviewer 111,087; supply-chain-reviewer 104,721; docs-reviewer 112,622; validators 16,492, 38,032, 31,617 and 88,880.
* `m2`: reviewer 80,279; gpu-reviewer 84,307; supply-chain-reviewer 60,061; docs-reviewer 73,898; validators 26,093, 50,662, 17,620, 25,974, 23,530, 27,598 and 12,949.
* `m10`: reviewer 76,628; wire-reviewer 69,228; supply-chain-reviewer 62,071; docs-reviewer 76,552; validators 24,935, 23,825, 18,321 and 48,454.
* `m18a`: reviewer 68,527; memory-reviewer 78,774; supply-chain-reviewer 69,675; docs-reviewer 73,272; validators 16,736, 43,748 and 40,437.

The runs that did not count:

| Run | Agents | Wall time (s) | Session tokens | `total_cost_usd` | Why it did not count |
| - | - | - | - | - | - |
| dry run 1 (`replay-m11-dry-1`) | 7 | 1,508 | 16,767,829 | 9.92 | a dry run |
| dry run 2 (`replay-m11-dry-2`) | 10 | 1,582 | 18,217,623 | 11.11 | a dry run |
| `m11` run 1 (`replay-m11-1`) | 7 | 1,658 | 18,520,919 | 10.27 | the `-p` session ended while the mutants validator ran, with no `findings.md` |
| `m10` run 1 (`replay-m10-1`) | 4 | 767 | 7,417,992 | 5.44 | a usage limit |
| Total | 28 | 5,515 | 60,924,363 | 36.74 | |

`total_cost_usd` is a list-price figure (`costBasis` `list`), not what the subscription pays.

The old round's reviewer tokens are not recorded, for each case.  No fix commit message (`c01d4cd`, `4b27829`, `da675e4`, `280e800`, `d67db75`, `e63469e`) records them.  No line of phase 2's plan on `main` records them (`git show` and `grep -i token`).  No agent read the old session transcripts: auto mode refused that read in plan review round 3, with the reason "PII Data Handling".

## Reviewers

The verdict counts of each source, summed over the six counted cases.  The counts are by source, so a merged finding with two sources counts once for each.  No finding has the verdict `unproven`.

| Source | Confirmed | Pre-existing | Refuted | Unvalidated |
| - | - | - | - | - |
| cargo-mutants | 46 | 1 | 6 | 0 |
| docs-reviewer | 12 | 1 | 0 | 15 |
| gpu-reviewer | 3 | 0 | 0 | 1 |
| memory-reviewer | 5 | 0 | 0 | 1 |
| reviewer | 9 | 1 | 0 | 11 |
| supply-chain-reviewer | 5 | 1 | 0 | 7 |
| wire-reviewer | 1 | 0 | 0 | 1 |

## Outside findings

An outside finding is a found finding (`confirmed` or `pre-existing`) that matches no ground-truth item of its case.  There are 61: `pr9` 43, `m9` 1, `m11` 5, `m2` 4, `m10` 6, `m18a` 2.  Mutant 1-10 of `m10` matches `m10-2`, so it is not an outside finding.  It is still on `main` (TD-42).  Each has the severity `should-fix`.  A new checker subagent compared each with `main` at `fa41c9f`.  Its states: 9 fixed on `main`, 2 gone and 50 still on `main`.  "(likely)" is the checker's word: it read the code and the tests, but did not run them.  The last column also gives the tech-debt entry of a finding that is still on `main`.

Rows of `pr9` mutants that share one state and one reason share one row.  The source of each `pr9` mutant is `cargo-mutants`, and its claim is "no test fails with this mutant".

The maintainer classes a sample of these findings as real or false after the PR is ready.

| Case | ID | Severity | Source | Claim | Checker's state |
| - | - | - | - | - | - |
| `pr9` | 1-1 | should-fix | reviewer, memory-reviewer, supply-chain-reviewer | `sends_json_and_reads_a_sized_reply` still passes when the client reads to the end of the stream, since the fake closes the socket after 30 s. | still on main: the test has a 120 s timeout.  So a client that reads to the end of the stream passes when the fake closes the socket at 30 s (TD-39) |
| `pr9` | 1-2 | should-fix, pre-existing | reviewer | The `selftest` scenario does not see a failed model load, so it waits the full 4 hours. | fixed on `main`, `03d3326` |
| `pr9` | 1-3 | should-fix | docs-reviewer | `docs/ARCHITECTURE.md` calls the WebDriver client "std-only", but it uses serde_json and anyhow. | still on `main`; fixed on this branch by M5 |
| `pr9` | 1-24, 1-41, 1-48 | should-fix | cargo-mutants | `+` to `*` or `-` in the arithmetic of `Session::set_viewport`. | fixed on `main`, `32b2731`: the arithmetic moved into `fit_viewport`, and `viewport_tests` assert the sizes |
| `pr9` | 1-17, 1-39 | should-fix | cargo-mutants | `\|\|` to `&&` in the end of the wait in `bench`, line 837. | gone: `03d3326` removed the clause in `bench` |
| `pr9` | 1-13 | should-fix | cargo-mutants | `Session::set_viewport` replaced with `Ok(())`. | still on main (likely): no test calls the `set_viewport` wrapper (TD-39) |
| `pr9` | 1-15, 1-16, 1-21, 1-23, 1-28, 1-32, 1-49 | should-fix | cargo-mutants | Mutants of the `Page` methods `wait_for`, `errors`, `idle`, `url`, `finish` and `click`. | still on main (likely): `Page` needs a live browser session (TD-39) |
| `pr9` | 1-10, 1-11, 1-14, 1-19, 1-20, 1-25, 1-26, 1-30, 1-35, 1-36, 1-38, 1-40, 1-43, 1-44, 1-45, 1-46, 1-47 | should-fix | cargo-mutants | Mutants in the scenarios and their helpers: `storage`, `audio_resume`, `bench`, `drop_model`, `smoke`, `storage_with_model`, `separate`, `worker_check` and `clip`. | still on main (likely): the scenarios run only in a browser, never in `cargo test` (TD-39) |
| `pr9` | 1-37, 1-42 | should-fix | cargo-mutants | `query` replaced with an empty string; `!` deleted in `Opts::weights`. | still on main (likely): `parses_scenario_and_options` never calls `weights()` or `query()` (TD-39) |
| `pr9` | 1-51 | should-fix | cargo-mutants | `<=` to `>` in `drip`; a request with no headers hangs. | still on main (likely): the drip test still sends a Host header (TD-39) |
| `pr9` | 1-12, 1-22, 1-31, 1-34 | should-fix | cargo-mutants | Mutants of `Driver::start`, `free_port` and `Session::execute`. | still on main (likely): no test starts a `Driver` or calls `Session::execute` (TD-39) |
| `pr9` | 1-18, 1-27 | should-fix | cargo-mutants | Mutants of `run` in `xtask/src/web_test.rs`. | still on main (likely): the tests of `xtask/src/web_test.rs` never call `run` (TD-39) |
| `pr9` | 1-50 | should-fix, pre-existing | cargo-mutants | `run` in `xtask/src/serve.rs` replaced with `Ok(())`. | still on main (likely): `xtask/tests/serve.rs` never calls `run` (TD-39) |
| `m9` | 1-1 | should-fix | reviewer | No test fails when the `well_formed` guard goes, so a line of `THIRD-PARTY.md` that is not well formed can become a row. | fixed on `main`, `7a99a88` (from `4b27829`): the fixture `"x v0.1.0 extra\|MIT\|"` kills the mutant |
| `m11` | 1-1 | should-fix | reviewer, memory-reviewer, docs-reviewer | No test reaches two branches of `app_stretcher_problems`: `"default"` in the features of an `engine =` line, and a manifest with no `engine =` line. | still on main (TD-40) |
| `m11` | 1-2 | should-fix | docs-reviewer | An agent wrote decision 0007 as accepted by the maintainer, but the maintainer chose only the rule, not the limits of its scope. | still on main: decision 0007 still says "accepted (by the maintainer)" (TD-45: only the maintainer can set the status of a decision, and the orchestrator's summary names it) |
| `m11` | 1-8 | should-fix | cargo-mutants | `-` to `+` in `WsolaStretcher::can_pull`; the stretcher's output changes for pushes over 256 frames. | still on main (likely): `a_stretcher_keeps_its_warm_up_promise` counts allocations only, and no test checks the output (TD-40) |
| `m11` | 1-10 | should-fix | cargo-mutants | `+` to `*` in `WsolaStretcher::warm_up`; the warm-up push becomes 722M frames. | still on main (likely): no test bounds the warm-up size (TD-40) |
| `m11` | 1-14 | should-fix | cargo-mutants | `==` to `!=` in `Player::drop_oldest_anchor_when_full`; the playhead shows the loop start for about 4 callbacks before each wrap. | still on main (likely): the loop tests check only that the playhead stays inside the loop (TD-40) |
| `m2` | 1-2 | should-fix | reviewer, gpu-reviewer | `every_layout_matches_its_shaders` pairs shaders, layouts and entry names from its own hand-copied table, not from `Kernels::new`.  So its doc comment and the docs claim more than it checks. | still on main (in part): `ad1b159` narrowed the claim.  The last sentence of the doc comment, "The entry points that `Kernels::new` names exist.", still claims too much, since the test checks a hand copy of the entry names (TD-41) |
| `m2` | 1-4 | should-fix | docs-reviewer, gpu-reviewer | No test covers the count of read-only storage buffers, the y-axis limit, or z in the invocation product. | still on main: no planted shader has one of these (TD-41) |
| `m2` | 1-6 | should-fix | docs-reviewer | GPU-1 says "The test parses every shader with naga", but both tests check only their own hand lists. | fixed on `main`, `ad1b159`; the rest is 1-2 |
| `m2` | 1-7 | should-fix | docs-reviewer, reviewer | GPU-1 says that only wgpu checks dispatch counts, but `Rec::push` asserts each one against 65,535. | fixed on `main`, `ad1b159` |
| `m10` | 1-1 | should-fix | reviewer, wire-reviewer | A plain Rust literal keeps `\n`, `\t` and `\r` as source text, so `\ntype` reads as `ntype`, and a misspelt type after it passes.  TD-16 does not name the gap. | still on main: `unescape_quotes` keeps `\n`, and TD-16 names only a `\n` after `//` (TD-16 gains a sentence) |
| `m10` | 1-2 | should-fix | docs-reviewer | TD-16 does not name regex literals, where a quote, `//` or backtick hides the types after it. | fixed on `main`, `7cd8518` |
| `m10` | 1-3 | should-fix | docs-reviewer | No test has a `SCRIPT_ONLY_TYPES` entry that only `xtask/src/e2e.rs` uses, so a stale check without the Rust half passes. | still on main: `the_script_only_list_is_checked` has no such entry (TD-42) |
| `m10` | 1-11 | should-fix | cargo-mutants | The match guard in `rust_type_value` replaced with `true`; `"type": format!("segmnt")` gives a false problem. | still on main (likely): no test has a macro other than `json!` after `"type":` (TD-42) |
| `m10` | 1-12 | should-fix | cargo-mutants | `!` deleted in `js_included_types`; the first string of an `includes` array is not checked. | still on main (likely): both `includes` cases put the misspelt type second (TD-42) |
| `m10` | 1-13 | should-fix | cargo-mutants | `+=` to `-=` in `js_tokens`; a newline before `/*` gives the wrong line. | still on main (likely): in a scratch copy, `js_is_lexed` passes with the mutant (TD-42) |
| `m18a` | 1-1 | should-fix | docs-reviewer, memory-reviewer | The doc comment of `load_and_keep` says that the bytes are never copied, but the picker copies each chosen file into wasm once. | fixed on `main`, `7541152` (from `e63469e`) |
| `m18a` | 1-2 | should-fix, pre-existing | supply-chain-reviewer, docs-reviewer | No automated check runs the file picker path, and no tech-debt entry records that. | still on main: no automated check drives the picker (TD-43) |

## Flags

A new flag subagent read `.claude/agents/*.md`, `.claude/skills/review-rounds/SKILL.md` and `## Ground truth`.  It found no flag, and no borderline text.  The skill holds only the procedure.  The texts that came closest:

* `.claude/agents/wire-reviewer.md:61`: "A field name or a default that one side changes and the other side does not (TD-16)".  The items of `m10` are test gaps in `js_after_typeof` and `rust_types`, not a field mismatch.
* `.claude/agents/supply-chain-reviewer.md:61`: "The licence and the registry of each new crate.  Check them with `cargo info <crate>` (TD-7)".  `m9-4` is a missing test case, not the licence of a new crate.
* `.claude/agents/baseline.md:17`: "For each test, ask: does it fail if the code is wrong?".  The six reviewer files hold, at line 56, "name a change to the code that the test must catch".  Each is a general method, and `baseline` ran in no case.

Caveat: the agents point to tech-debt IDs, and `docs/tech-debt.md` at a reviewed commit is part of the reviewed tree.  The flag subagent did not read it.

## Threats

The five threats of the parent plan:

* One run of a random process.  Each case has one counted run, with no tuning (D5).  Another run can find other findings.
* Six cases, most of them harness work in the main.  n is 26, so the interval is wide.  The GPU, memory and wire reviewers have few items to find.
* The harness leaves out the person's global `CLAUDE.md` in `~/.claude/`, which real rounds load.  The case command has `--setting-sources project,local`.
* The agent that wrote the agent files has read phase 2's Progress, which names the findings.  The flag subagent found no text that names an item (`## Flags`).  But the check cannot see a bias that names no item.
* The agents name `docs/invariants.md`, tech-debt IDs and files that do not exist at some replayed commits.  For example, the rule of `pr9`'s finding 1-3 says "docs/invariants.md is missing".

The threats that M5 found:

* Of the 8 case runs that were not dry runs, 5 ran `classic_drums_beat_the_raw_mix` in the mutants baseline.  It timed out in the unmutated tree in 3 (`m9`, `m2` and `m10` run 1), so no mutant ran there.  It passed in `pr9` and `m10` run 2.  The baselines of `m11` and `m18a` never ran it.  Of the 13 items of `m9` and `m2`, 10 are weak-test items that a missed mutant could have matched: `m9-1` to `m9-8`, `m2-2` and `m2-3`.  The other three are not weak-test items, so no missed mutant can match them.  `m2-1` and `m2-5` are defects in code.  `m2-4` is a gap in a test's own list of files, which no mutant of a function reaches.  TD-33 records the slow baseline.
* `pr9` and `m10` stopped the mutants run at the limit of 60 minutes, with 262 and 130 mutants not tested.
* Two runs ran again: `m11` run 1 (the `-p` session ended while the mutants validator ran) and `m10` run 1 (a usage limit).
* `dontAsk` denials: 126 over the six counted runs.  The harness denies a command that the allow list does not admit.  A real session asks a person, or auto mode decides.  The team leader found the full command of each denial in the transcript.  Each command counts once, in the first kind of the table that it matches, in the order of the table.

  | Kind | `pr9` | `m9` | `m11` | `m2` | `m10` | `m18a` | Total |
  | - | - | - | - | - | - | - | - |
  | heredoc | 1 | 0 | 6 | 0 | 4 | 0 | 11 |
  | multi-line command (such as `sed -i` with `a\`) | 1 | 3 | 4 | 0 | 6 | 0 | 14 |
  | `cd` | 2 | 3 | 1 | 5 | 2 | 1 | 14 |
  | variable or env prefix (`W=... &&`, `f=$(...)`, `CARGO_TARGET_DIR=... cargo`) | 1 | 2 | 10 | 3 | 1 | 6 | 23 |
  | `>` or `>>` write to a file | 3 | 0 | 2 | 2 | 1 | 1 | 9 |
  | `which` or `command -v` | 7 | 0 | 0 | 0 | 0 | 2 | 9 |
  | a path outside the clone (`~/.cargo`, `/Users/`, `find /`) | 0 | 0 | 8 | 1 | 0 | 7 | 16 |
  | another command not on the list (`awk`, `cp`, `comm`, `printf`, `for`, `sysctl`, `wasm-bindgen-test-runner`, `{ }`) | 6 | 4 | 5 | 5 | 4 | 6 | 30 |
  | Total | 21 | 12 | 36 | 16 | 18 | 23 | 126 |

  The effect in each case:

  * `pr9`: 7 `which` or `command -v` and one `CARGO_TARGET_DIR=... cargo run -p xtask -- web-test` were denied.  So no reviewer ran the browser tests.  Seven `cargo tree` pipes for the licence list (with `comm`, `awk`, `{ }` or a write) were denied.  Two mutant probes with `cd`, and two validator probes (a heredoc, a multi-line `sed`), were denied.  The items of `pr9` are in the e2e and WebDriver code, which runs only in a browser.
  * `m9`: 8 mutant or probe commands in worktrees were denied (3 multi-line `sed`, 3 `cd`, 2 `W=` prefixes).  4 reads with `awk` or `for` were denied.
  * `m11`: 10 writes of probe tests (6 heredocs, 4 multi-line `sed`), 10 commands with a variable prefix and 8 reads of the cargo registry were denied.  The one item of `m11` was found.
  * `m2`: 5 mutant probes with `cd`, 3 with a variable prefix and 2 shader writes with `printf >` were denied.
  * `m10`: 10 writes of probe tests to `rules.rs` were denied (4 heredocs, 6 multi-line `sed`).  `m10-1` is a missing test case in `rules.rs`, and was missed.
  * `m18a`: the harness denied every try to run the browser tests.  These were 2 `which chromedriver wasm-bindgen-test-runner`, a `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=... cargo test`, and 2 `CARGO_TARGET_DIR=... cargo run -p xtask -- web-test`.  They also include a wasm32 `cargo clippy` and an `echo "CHROMEDRIVER=$CHROMEDRIVER"`.  All four items of `m18a` are in JavaScript in `inline_js` blocks, which only wasm tests in a browser check.  `.cargo/mutants.toml` excludes `app/src/store.rs`, which is wasm32 only.  So no reviewer and no mutant could run code that reaches an item of `m18a`.

  So some probes and tests did not run, and the result can be low.
* The old round's reviewer tokens are not recorded, and auto mode refused the read of the old transcripts.
* The comparers got each finding's ID, sources, severity, kind, place and claim, with the evidence where it bears on the match.  The team leader made shorter the claims of nits about docs or wording only, which cannot match a code item.  A comparer was not sure if mutant 1-10 matches `m10-2`.  The team leader first marked no match, then applied the written rule of step 6.2, so 1-10 matches.
* The severities of `pr9` and `m2` are inferred, so those items do not enter the should-fix test of the bar.
* The ground truth is what the old round fixed, so the measure is recall only.  The runs found 61 outside findings, and the maintainer classes a sample of them as real or false.
* The items of `m9` rest on an assumption.  Progress at `4b27829` records 1 should-fix and 7 nits, but the fix commit has 10 changes.  `m9-2` to `m9-8` assume that the 7 nits are the 7 test cases that the message names.
* The ground truth can lack a finding of the old round.  The old round's own fix commits fixed three outside findings, so the old round likely found them:
  * `m9` 1-1, the `well_formed` guard, fixed by `4b27829` with a fixture.  Neither the commit message nor Progress names that change.
  * `m10` 1-2, regex literals in TD-16, fixed by `d67db75`.  That fix changed docs only.
  * `m18a` 1-1, the doc comment of `load_and_keep`, fixed by `e63469e`.  That fix changed docs only.

  By its rules, the ground truth leaves out a fix that changed docs only.  The ground truth was fixed before the runs, and stays as it is.
