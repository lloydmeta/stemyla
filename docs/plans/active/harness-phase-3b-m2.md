# Harness phase 3b, M2: less mutants noise (the known-missed list)

## Purpose

This is the milestone plan for M2 of `plans/active/harness-phase-3b.md`.  It gives more detail than the parent plan's M2 section, which stays the criteria.  M2 starts with a measurement of the mutants noise that is left after `.cargo/mutants.toml` of PR #39.  The measurement found that the noise that the parser filter of the parent plan removes is small.  So this plan cuts that filter, and keeps the known-missed list (Decision log).

After M2:

* `xtask review mutants <round>` splits the missed mutants of cargo-mutants in two files of the round folder.  `missed.txt` holds the lines for the validator.  `known.txt` holds each line that a known-missed entry matches, with the entry's tech-debt ID.
* The mutants line counts the known lines, as `, known <k>`.
* The `review-rounds` skill gives the validator only `missed.txt`.  It logs each line of `known.txt` as a finding with the verdict `known` and `repeat-of` the TD, with no validator.
* The list holds the mutants of TD-39 and TD-66 that PR #39's review rounds sent to tech debt.  A test fails when an entry names a closed TD, or code that is gone.

Readers: the implementer of M2, the review agents of M2's rounds, and the orchestrator.

## Context and orientation

The state at the start of M2, on 2026-10-31, at commit `759abfa` on branch `harness-phase-3b`:

* `xtask/src/review.rs` (3,556 lines): `mutants` (near line 1308) runs `cargo mutants --in-diff <round>/mutants.diff --output <round>` under a limit, then writes one line to `<round>/mutants.txt` and prints it.  `classify` and `impl Display for Line` (near line 1038 and 1092) make the line: `mutants: <class>: tested <n> (caught <c>, missed <m>, unviable <u>), timeouts <t>, not tested <n>`, then `; <reason>` if there is one.  `clear_old_output` removes the old `mutants.out`, `mutants.out.old` and `mutants.txt` of the round.  `prepare_mutants` reads the round's `state.json` (`since`, `head`) and writes `mutants.diff`.
* cargo-mutants writes `<round>/mutants.out/missed.txt`: one line for each missed mutant, as `<file>:<line>:<col>: <mutation>`.  For example, `xtask/src/e2e/save.rs:503:5: replace save_once -> Result<Save> with Ok(Default::default())`.  An operator mutation ends with `in <function>`, such as `replace * with + in save_once`.  Two mutants can have the same text in one function, at other lines: `replace || with && in note_run` at lines 254 and 256 of `0789e6b:xtask/src/e2e/weights.rs`.
* The tests of the mutants run are in `xtask/src/review.rs` (near line 2957 to 3546).  `fake_cargo` and `round_1_of_a_rust_change` make a temporary repo and a fake `cargo` that writes `mutants.out`.  `COUNTED` and `NOTHING` pin the counts of the line.
* `.claude/skills/review-rounds/SKILL.md`, step 6: item 3 reads `<run>/$1/mutants.out/missed.txt`.  Item 4 makes a finding of each line: source `cargo-mutants`, severity `should-fix`, kind `weak-test`, rule `VERIFY-1`.  Item 5 gives `repeat-of` only to a finding that matches a `refuted` finding of an earlier round.  So a confirmed mutant that went to TD-39 comes back in each round that touches its file.  `REVIEW_SKILL_TEXT` in `xtask/tests/rules.rs` (near line 1131) pins lines of the skill.  The skill and its pins are the orchestrator's to edit.
* `docs/tech-debt.md`: TD-33 says what the mutants run cannot test.  TD-39 lists the e2e and WebDriver code that only a browser reaches, such as `save_once`, `save_playing`, `click_save` and `restore` of `xtask/src/e2e/save.rs`.  TD-66 lists `note_run` and `run` of `xtask/src/e2e/weights.rs`.
* `docs/working-with-agents.md` describes the round (the list near line 79) and the `review` tools (Tools table, near line 154).
* No new crate is needed (DEPS-1).  The code uses the standard library and `git`.

Terms:

* Missed line: one line of cargo-mutants' `missed.txt`.
* Known-missed entry: one row of the list.  It names a TD, a file, a mutation text and a code line.
* Known line: a missed line that an entry matches.

### The measurement (parent plan, M2, first bullet)

Source: the findings tables of `docs/reviews/iphone-crashes-2026-10-25-m8.md`, `-m9.md` and `-catchup.md`, which hold 22 rounds.  Each round's commit has `6d0a39d` as an ancestor (`git merge-base --is-ancestor`).  Each row with `cargo-mutants` in its sources is one missed line: 261 rows, in 13 of the 22 rounds.  The class of each row comes from the code at the round's commit (`git show <commit>:<file>`), and from its verdict and action.

| Class | Lines | Verdicts | Where |
| - | - | - | - |
| 1. A file that today's `.cargo/mutants.toml` leaves out | 68 | all refuted | `app/src/save_tests.rs`, M8 round 1 |
| 2. A wasm32-only item with no `mutants::skip` | 11 | all refuted | M8 round 1: `save_type`, `storage_full_text`, `can_encode` (2), `drop_save_sources` (`app/src/lib.rs`), `StoredWav::path` (2, in `mod stored` of `app/src/export.rs`).  Catch-up round 1: the wasm32 variant of `mapped_floats` (4, `roformer-gpu/src/lib.rs`), which has no attribute by the rule of `.cargo/mutants.toml` |
| 3. A wasm32 block in a shared function | 0 | | |
| 4. A known-missed mutation (went to TD-39 or TD-66) | 29 | confirmed or pre-existing | M8 21, M9 6, catch-up 2.  11 of them repeat the function and mutation of a row of an earlier round: `save_once` 5, `save_playing` 4, `click_save` 2 |
| 5. The rest | 153 | 118 confirmed or pre-existing, 35 refuted | the refuted: `protocol/src/lib.rs` 16 (native code that only the wasm32 tests reach), constants in `xtask/src/e2e/save.rs` 10, and 9 equivalent mutants |

What the counts show:

* Class 1 is gone already: 68 of the 114 refuted lines.  193 lines are left.
* The parser filter of the parent plan removes class 2 and class 3: 11 lines (6% of 193), in 2 of 22 rounds.  4 of them are there by the design of `.cargo/mutants.toml`.
* The known-missed list removes class 4: 29 lines (15%), in 10 rounds.  11 of them came back from an earlier round of the same milestone.
* The largest refuted class left is in class 5: `protocol/src/lib.rs`.  No filter by `cfg` finds it, because that code compiles natively.  M2 records it in TD-33.
* The validator gets all of a round's missed lines in one prompt.  So the noise costs part of one agent's tokens, and a log row for each line.  It does not cost an agent for each line.

## Plan of work

### How the team works

* The team leader writes this plan and starts one implementer (`general-purpose`) with its path.  The implementer does steps 1 to 4.  Each step brings its own tests, written first and seen to fail.  Each step is one or more signed commits, after the checks of GIT-2, so each commit passes `docs-check`.
* The implementer does not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.  Step 4 gives the team leader one diff file for the orchestrator.  The diff holds the skill, its pins in `xtask/tests/rules.rs`, and the lines of `docs/working-with-agents.md` that describe what the skill does.  So the docs land in the same commit as the skill that they describe (DOC-1; M1's retrospective).
* Tests run on temporary repos and fixture text only, never on this repo's `target/review/`.  No test needs `refs/remotes/origin/pr/39`, since CI does not have it.  The fixture lines come from the logs above, and the code lines from `git show` of the round's commit, copied into the test.

### Step 1: the list and its match

Add `xtask/src/review/known_missed.rs`, with `mod known_missed;` in `xtask/src/review.rs`.

* `pub struct Known { pub debt: &'static str, pub file: &'static str, pub mutation: &'static str, pub code: &'static str }`.  `debt` is a TD ID.  `file` is a path from the repo root.  `mutation` is the text of a missed line after `<file>:<line>:<col>: `.  `code` is the line of the file at `<line>`, with the spaces at its ends removed.
* `pub const KNOWN_MISSED: &[Known]`: empty in step 1.  Step 3 fills it.
* `pub fn split<'a>(missed: &'a str, list: &[Known], code_at: impl Fn(&str, usize) -> Option<String>) -> Split<'a>`.  `Split` has `to_validate: Vec<&'a str>` and `known: Vec<(&'a str, &'static str)>`: each known line with its TD.  The order of each list is the order of `missed`.  `split` ignores empty lines.
* A line is known when all of these hold: it parses as `<file>:<line>:<col>: <mutation>`; an entry has the same `file` and the same `mutation`; and `code_at(file, line)`, trimmed, equals the entry's `code`.  Else it goes to the validator.  This holds also for a line that does not parse, and for a file that `code_at` cannot read.  So a wrong or old entry sends a line to the validator, and never hides a new mutant.
* The `code` test keeps two mutants with the same text in one function apart, when they are on different lines.  Two such mutants on the same line match the same entry.  The doc comment says so.

Tests of step 1, in `known_missed.rs`, with a fixture list (not `KNOWN_MISSED`) and a fake `code_at`:

* The line of finding 2-17 of round 2 of M8 (`xtask/src/e2e/save.rs:503:5: replace save_once -> Result<Save> with Ok(Default::default())`) is known, with its TD.  Its code at `9c69f22` is `restore(p, seconds)?;`.
* The lines of findings 1-41 and 1-54 of round 1 of M8 (`xtask/src/e2e/save.rs:427:39: replace * with + in save_once`, and `* with /`) go to the validator.  They are in the same function as 2-17.  `2fdecda` fixed them with tests.  Their code at `f28d22d` is `let item_y = o.item_y + o.item_dy * row as f64;`.
* The same mutation and file, but another code line, goes to the validator.  So does the same mutation in another file.
* Two lines `replace || with && in note_run` at two lines of one file: an entry with the code of one of them makes only that one known.
* A line whose file `code_at` cannot read, and a line with no `<line>:<col>: ` part, go to the validator.
* Order and empty lines: the output keeps the order of the input, and has no empty line.

### Step 2: `review mutants` writes `missed.txt` and `known.txt`

* After cargo-mutants ends, `mutants` reads `<round>/mutants.out/missed.txt`, if it exists.  It calls `split` with `KNOWN_MISSED`.  `code_at` reads the file at the round's `head` with `git show <head>:<file>` (the commit that cargo-mutants copied), and takes the line.
* It writes `<round>/missed.txt` (the lines to validate) and `<round>/known.txt` (each line, a tab, and its TD).  Each line ends with a newline.  It writes both files also when they are empty, and also when cargo-mutants wrote no `missed.txt`.  It does not change anything in `mutants.out/`.
* `clear_old_output` also removes an old `missed.txt` and `known.txt` of the round.
* The mutants line gets `, known <k>` after `not tested <n>`, and before the reason.  `<k>` is the number of known lines.  The line is the same in `mutants.txt` and on stdout.  It is `known 0` when the run made no `missed.txt`, also for the class `none`.
* `split` takes its list as an argument, so the tests of this step give a fixture list.  `mutants` uses `KNOWN_MISSED`; a seam such as `mutants_with(…, list)` lets the run test give its own list.
* Tests of step 2, in `xtask/src/review.rs`:
  * The fake cargo writes a `mutants.out/missed.txt` with two lines, one on the fixture list.  The repo's committed file holds the entry's code at that line.  After the run, `missed.txt` holds the other line, `known.txt` holds the known line and its TD, and the line ends `not tested 0, known 1`.
  * A run whose fake writes no `missed.txt` gives two empty files and `known 0`.
  * The old `missed.txt` and `known.txt` go before a new run (the test of `clear_old_output`).
  * Each test that pins the line (`COUNTED`, `NOTHING` and the others) gets `, known <k>`.  These are the "log format test" of the parent plan.  The rounds table of the log keeps its columns, since the line is one cell.

### Step 3: the entries, and the test that keeps them true

* Fill `KNOWN_MISSED` from the 29 rows of class 4 above.  For each row, find the mutant at `759abfa` with `cargo mutants --list --line-col=true -f <file>` (or the option that cargo-mutants 27.1.0 gives for line and column).  Take its mutation text and its code line.  Each entry has the TD of its row's action.
* Leave out a row whose mutant `--list` no longer shows, or whose row went to a fix, not only to tech debt.  The five rows at `2c7c29b:xtask/src/e2e/save.rs:156` say "TD-39; fc5fa97 review round speed 1 fixes": check the fix commit, and leave them out if it added a test.  Record each entry and each row left out in Surprises.
* Each entry is one mutant, so one row of the logs gives at most one entry.  A row that repeats an earlier row gives no new entry.
* A test, `each_known_missed_entry_is_open_and_in_the_code`, in `known_missed.rs`, reads this repo's files:
  * Each `debt` is an open TD: a `### <debt>: ` heading in `docs/tech-debt.md`, and no bullet `* <debt>:` under its `## Closed`.  So closing the TD makes the test fail until its entries go.
  * Each `file` exists, and has a line whose trimmed text is `code`.
  * The function that `mutation` names (the word after `replace ` up to ` ->`, ` with` or `(`, or the word after ` in `) has a `fn <name>` line in `file`.
  * No two entries are the same.
* A test with the real list and the line of 2-17 and its code: the line is known, with TD-39.  The lines of 1-41 and 1-54 go to the validator.  This is the acceptance of the parent plan, with the real list.

### Step 4: the docs, and the diff for the orchestrator (DOC-1)

The implementer commits:

* `docs/tech-debt.md`: in TD-33, a bullet: `xtask review mutants` sends the lines of `KNOWN_MISSED` (`xtask/src/review/known_missed.rs`) to `known.txt`, not to the validator.  A second bullet: native code that only the wasm32 tests reach, such as the JS readers of `protocol/src/lib.rs`, shows as missed; the validator refutes it (16 lines in the measurement).  A third: a new wasm32-only item without `mutants::skip` still shows as missed (11 lines in the measurement), since M2 has no parser filter.  In TD-39 and TD-66, one sentence each: their mutants are on the list.
* `docs/working-with-agents.md`, Tools table, the `review mutants` cell: it writes `missed.txt` and `known.txt`, and the mutants line counts the known lines.  A changelog line for M2.
* `docs/harness.md`: if its row for `xtask review mutants` names what the run gives the validator, add the list.

The implementer gives the team leader one diff file, not a commit, with:

* `.claude/skills/review-rounds/SKILL.md`, step 6:
  * Item 3 reads `<run>/$1/missed.txt`, not `mutants.out/missed.txt`.  If the file does not exist, the mutants run is from an older `xtask`: read `mutants.out/missed.txt`.
  * A new item after item 4: each line of `<run>/$1/known.txt` becomes a finding with the next ID.  Its fields are those of item 4.  Its verdict is `known`, its `repeat-of` is the TD after the tab, and it gets no validator.  Its action in the log is "known: <TD>".
  * Step 9: the verdict list of a finding adds `known`.  Step 10: "Do not fix a `known` finding.  Its gap is in its TD."  Step 11's PR text lists each `known` finding with its TD.
* `xtask/tests/rules.rs`: the new lines of `REVIEW_SKILL_TEXT`, such as "Read `<run>/$1/missed.txt`" and "Its verdict is `known`", with the doc comment.
* `docs/working-with-agents.md`, "Review a change", item 4: the validator gets the missed mutants but the known ones, which the agent logs as `known` with their TD.  The "Check:" line names each `known` finding.

The implementer checks the diff in a scratch worktree: `git apply --check`, then `cargo test -p xtask --test rules` passes with the diff, and fails with only the pins.  The team leader puts the diff in the scratchpad and gives it to the orchestrator at the checkpoint, before round 1.  So round 1 runs with the new skill, and its mutants run is the first live use.

### Step 5: review rounds

`cargo build -p xtask`, then `/review-rounds 759abfa <round> "<criteria>"` for rounds 1, 2 and 3, and more until a round has no blocker and no significant should-fix.  No `--baseline`: M1 ran it for this PR.  The criteria are the parent plan's M2 section and this plan.  Each confirmed finding gets a fix with a test.  A nit gets a fix only if the team leader agrees.  The Write tool refuses the findings file and the log to the team leader (M1).  So each round stops after its fixes and `review clean`, and the orchestrator writes the files from the team leader's report.  After the last round, the orchestrator writes `docs/reviews/harness-phase-3b-m2.md` and removes the run folder (TD-37).

## Validation and acceptance

* `cargo test -p xtask` passes, with the tests of steps 1 to 3.  Each test of steps 1 and 2 fails at `759abfa`, where `known_missed` and the two files do not exist.  The test of the real list fails with an empty `KNOWN_MISSED`.
* The acceptance of the parent plan, with the cut of the Decision log:
  * The filter of wasm32 items is cut.  The measurement shows 11 such lines in 22 rounds, and TD-33 records the gap.
  * Findings 1-41 and 1-54 of round 1 of M8 still go to a validator (steps 1 and 3).
  * The mutant of finding 2-17 of round 2 of M8 is logged as known, with TD-39 (steps 1 and 3).
* The mutants line of round 1 of M2 has `, known <k>`, and the round folder has `missed.txt` and `known.txt`.  The team leader records them in Progress.
* `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo run -p xtask -- docs-check` pass.
* The review rounds end by the stop rule, and the log is in `docs/reviews/harness-phase-3b-m2.md`.

## Idempotence and recovery

* `review mutants` writes its two files again on each run, after `clear_old_output`, so a rerun gives the same files.
* A wrong entry cannot hide a mutant that it does not match exactly.  To undo the list, empty `KNOWN_MISSED`: every line then goes to the validator, as before M2.
* The skill reads `mutants.out/missed.txt` when `missed.txt` is missing, so an older `xtask` binary still works.
* Each step is its own commit, which `git revert` undoes.  A new team leader takes over from Progress.

## Progress

* [x] 2026-10-31: the measurement of the noise (team leader), in "The measurement" above.
* [ ] Plan written and reviewed.
* [ ] Step 1: the list and its match.
* [ ] Step 2: `review mutants` writes `missed.txt` and `known.txt`.
* [ ] Step 3: the entries and their test.
* [ ] Step 4: the docs, and the diff for the orchestrator.
* [ ] Review rounds.
* [ ] M2 done: this plan moves to `plans/completed/`.

## Surprises and discoveries

* 2026-10-31, team leader: every round of the M8, M9 and catch-up logs comes after `6d0a39d`, the catch-up rounds included.  The M8 log also has the rounds "CI fix" and "speed 1" to "speed 8".
* 2026-10-31, team leader: the wasm32 variants of `mapped_floats`, `base_latency` and `main` are items of their own, with `#[cfg(target_arch = "wasm32")]`.  The rule of `.cargo/mutants.toml` that keeps them without `mutants::skip` comes from `exclude_re`, which matched by name and so hid the native variant too (`plans/completed/stream-from-storage-catchup.md`, D4).  With the attribute on one item, the native variant keeps its mutants.  M2 does not change this; see the Decision log.
* 2026-10-31, team leader: the refuted mutants of `protocol/src/lib.rs` (16) are in functions that compile natively, with no `cfg`.  Only the wasm32 tests in `protocol/tests/web.rs` reach them.

## Decision log

* 2026-10-31, team leader: M2 cuts the parser filter of wasm32 items (the parent plan's second bullet, and its first acceptance bullet).  Reason: it removes 11 of the 193 lines left (6%), in 2 of 22 rounds, and 4 of those by design.  It needs an item parser and a `cfg` evaluator on `proc-macro2`, and a change to the mutants flow, for that yield.  TD-33 records the gap.
* 2026-10-31, team leader: M2 keeps the known-missed list.  Reason: it removes 29 lines (15%), in 10 rounds, and 11 of them came back each round.  It is the mutants' side of M3's `repeat-of` match to tech debt, and it needs no parser.
* 2026-10-31, team leader: M2 does not add `mutants::skip` to the wasm32 variants of `mapped_floats`, `base_latency` and `main`.  Reason: 4 lines in 22 rounds.  The change would reverse a rule of `.cargo/mutants.toml` and TD-33, and needs a `cargo mutants --list` check of each native variant.  A later change can do it.
* 2026-10-31, team leader: the list is a Rust table (`KNOWN_MISSED`), not a text file.  The compiler checks its fields, and a test checks that each entry names an open TD and code that exists.  `cargo build -p xtask` runs before each round, so the binary has the list of `HEAD`.
* 2026-10-31, team leader: an entry matches by file, mutation text and code line, not by line number.  Line numbers move with each edit.  The mutation text alone does not tell two operator mutants of one function apart.  A wrong match must never hide a new mutant, so each part must be equal.
* 2026-10-31, team leader: a known line gets the new verdict `known`, not `pre-existing`.  Step 10 of the skill fixes each `pre-existing` finding or sends it to tech debt.  A known line is in tech debt already, so it needs no action.

## Outcomes and retrospective

To be written when M2 completes.
