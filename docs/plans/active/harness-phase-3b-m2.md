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

* `xtask/src/review.rs` (3,556 lines): `mutants` (near line 1308) runs `cargo mutants --in-diff <round>/mutants.diff --output <round>` under a limit.  Then it writes one line to `<round>/mutants.txt` and prints it.  `classify` and `impl Display for Line` (near lines 1038 and 1092) make the line.  Its form is `mutants: <class>: tested <n> (caught <c>, missed <m>, unviable <u>), timeouts <t>, not tested <n>`, then `; <reason>` if there is one.  `clear_old_output` removes the old `mutants.out`, `mutants.out.old` and `mutants.txt` of the round.  `prepare_mutants` reads the round's `state.json` (`since`, `head`), writes `mutants.diff`, and returns only the round folder.
* `ROUTE_ALL` (near line 112) holds `xtask/src/review.rs`, so a change to it routes all six reviewers.  `review start` warns when `xtask/src/review.rs` is newer than the binary (`SOURCE`, near line 896).
* cargo-mutants writes `<round>/mutants.out/missed.txt`: one line for each missed mutant, as `<file>:<line>:<col>: <mutation>`.  An example is `xtask/src/e2e/save.rs:503:5: replace save_once -> Result<Save> with Ok(Default::default())`.  An operator mutation ends with `in <function>`, such as `replace * with + in save_once`.  Two mutants can have the same text in one function.  At `759abfa`, `xtask/src/e2e/weights.rs:498` has `replace == with != in run` at columns 49 and 61.
* The tests of the mutants run are in `xtask/src/review.rs` (near lines 2957 to 3546).  `fake_cargo` and `round_1_of_a_rust_change` make a temporary repo and a fake `cargo` that writes `mutants.out`.  `COUNTED` and `NOTHING` pin the counts of the line.
* `.claude/skills/review-rounds/SKILL.md`, step 6: item 3 reads `<run>/$1/mutants.out/missed.txt`.  Item 4 makes a finding of each line: source `cargo-mutants`, severity `should-fix`, kind `weak-test`, rule `VERIFY-1`.  Item 5 gives `repeat-of` only to a finding that matches a `refuted` finding of an earlier round.  So a confirmed mutant that went to TD-39 comes back in each round that touches its file.  `REVIEW_SKILL_TEXT` in `xtask/tests/rules.rs` (near line 1131) pins lines of the skill.  The skill and its pins are the orchestrator's to edit.
* `docs/tech-debt.md`: TD-33 says what the mutants run cannot test.  TD-39 lists the e2e and WebDriver code that only a browser reaches, such as `save_once`, `save_playing`, `click_save` and `restore` of `xtask/src/e2e/save.rs`.  TD-66 lists `note_run` and `run` of `xtask/src/e2e/weights.rs`.
* `docs/working-with-agents.md` describes the round (the list near line 79) and the `review` tools (Tools table, near line 154).
* No new crate is needed (DEPS-1).  The code uses the standard library and `git`.

Terms:

* Missed line: one line of cargo-mutants' `missed.txt`.
* Known-missed entry: one row of the list.  It names a TD, a file, a mutation text, a code line and an offset.
* Offset: the column of a missed line minus the spaces at the start of its code line.  It stays the same when the code line moves or its indent changes.
* Known line: a missed line that an entry matches.

### The measurement (parent plan, M2, first bullet)

Source: the findings tables of `docs/reviews/iphone-crashes-2026-10-25-m8.md`, `-m9.md` and `-catchup.md`, which hold 22 rounds.  Each round's commit has `6d0a39d` as an ancestor (`git merge-base --is-ancestor`).  Each row with `cargo-mutants` in its sources is one missed line: 261 rows, in 13 of the 22 rounds.  The class of each row comes from the code at the round's commit (`git show <commit>:<file>`), and from its verdict and action.  A `Plan` agent recounted each count (plan review round 1).

| Class | Lines | Verdicts | Where |
| - | - | - | - |
| 1. A file that today's `.cargo/mutants.toml` leaves out | 68 | all refuted | `app/src/save_tests.rs`, M8 round 1 |
| 2. A wasm32-only item with no `mutants::skip` | 11 | all refuted | M8 round 1: `save_type`, `storage_full_text`, `can_encode` (2), `drop_save_sources` (`app/src/lib.rs`), `StoredWav::path` (2, in `mod stored` of `app/src/export.rs`).  Catch-up round 1: the wasm32 variant of `mapped_floats` (4, `roformer-gpu/src/lib.rs`), which has no attribute by the rule of `.cargo/mutants.toml` |
| 3. A wasm32 block in a shared function | 0 | | |
| 4. A known-missed mutation (went to TD-39 or TD-66) | 29 | confirmed or pre-existing | M8 21, M9 6, catch-up 2, in 9 rounds.  11 of them repeat the function and mutation of a row of an earlier round: `save_once` 5, `save_playing` 4, `click_save` 2 |
| 5. The rest | 153 | 118 confirmed or pre-existing, 35 refuted | the refuted: `protocol/src/lib.rs` 16 (native code that only the wasm32 tests reach), constants in `xtask/src/e2e/save.rs` 10, and 9 other refuted |

What the counts show:

* Class 1 is gone already: 68 of the 114 refuted lines.  193 lines are left.
* The parser filter of the parent plan removes class 2 and class 3: 11 lines (6% of 193), in 2 of 22 rounds.  4 of them are there by the design of `.cargo/mutants.toml`.
* The known-missed list removes class 4: 29 lines (15%), in 9 rounds.  11 of them came back in a later round of the same milestone.
* The largest refuted class left is in class 5: `protocol/src/lib.rs`.  No filter by `cfg` finds it, because that code compiles natively.  M2 records it in TD-33.
* The validator gets all of a round's missed lines in one prompt.  So the noise costs part of one agent's tokens, and a log row for each line.  It does not cost an agent for each line.

## Plan of work

### How the team works

* The team leader writes this plan and starts one implementer (`general-purpose`) with its path.  The implementer does steps 1 to 4.  Each step brings its own tests, written first and seen to fail.  Each step is one or more signed commits, after the checks of GIT-2, so each commit passes `docs-check`.
* A test fails first on an assert, not on a compile error.  So the implementer first writes each new function as a stub that gives a wrong answer, in the working tree.
  * The stub of `split` marks each line known, with the debt `""`.
  * The stub of `known_problems` gives the problem `"stub"` for each entry.
  * Each test checks the whole output, so each test fails on its stub.  The implementer runs the tests on the stubs and records the failures in Surprises.
  * Then it writes the real code, and commits the tests and the code together.  No commit has a failing test (GIT-2).
* The implementer does not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.  Step 4 gives the team leader one diff file for the orchestrator.  The diff holds the skill, its pins in `xtask/tests/rules.rs`, and each line of the docs that describes what the skill does.  So those docs land in the same commit as the skill (DOC-1; M1's retrospective and its finding 1-11).
* Tests run on temporary repos and fixture text only, never on this repo's `target/review/`.  No test needs `refs/remotes/origin/pr/39`, since CI does not have it.  The fixture lines come from the logs above.  The code lines come from `git show` of the round's commit, copied into the test.

### Step 1: the list and its match

Put the list and its match in `xtask/src/review.rs`, in a section with a heading comment.  Then a change to the list routes all six reviewers (`ROUTE_ALL`), and `review start` warns about an older binary (`SOURCE`).

* `pub struct Known { pub debt: &'static str, pub file: &'static str, pub mutation: &'static str, pub code: &'static str, pub offset: usize }`.
  * `debt` is a TD ID, and `file` is a path from the repo root.
  * `mutation` is the text of a missed line after `<file>:<line>:<col>: `.
  * `code` is the line of the file at `<line>`, with the spaces at its ends removed.
  * `offset` is `<col>` minus the number of spaces and tabs at the start of that line.
* `pub const KNOWN_MISSED: &[Known]`: empty in step 1.  Step 3 fills it.
* `pub fn split<'a>(missed: &'a str, list: &[Known], line_at: impl Fn(&str, usize) -> Option<String>) -> Split<'a>`.  `line_at(file, n)` gives line `n` of the file, untrimmed.  `Split` has `to_validate: Vec<&'a str>` and `known: Vec<(&'a str, &'static str)>`: each known line with its TD.  Each list keeps the order of `missed`.  `split` ignores empty lines.
* A line is known when all of these hold:
  * it parses as `<file>:<line>:<col>: <mutation>`;
  * an entry has the same `file` and the same `mutation`;
  * `line_at(file, line)`, trimmed, equals the entry's `code`;
  * `<col>` minus the indent of that line equals the entry's `offset`.
* Else the line goes to the validator.  This holds also for a line that does not parse, and for a file that `line_at` cannot read.  So an old entry sends its mutant to the validator.
* An entry hides only a mutant with the same file, text, code line and offset.  Step 3's `known_problems` refuses a code line that the file has more than once.  So a copy of a known line cannot be hidden.
* The doc comment says why each part is there.  Line numbers move.  The text alone does not tell two operator mutants of one function apart.  The offset tells two mutants on one line apart.

Tests of step 1, in `xtask/src/review.rs`, with a fixture list (not `KNOWN_MISSED`) and a fake `line_at`:

* The line of finding 2-17 of round 2 of M8 is known, with its TD.  The line: `xtask/src/e2e/save.rs:503:5: replace save_once -> Result<Save> with Ok(Default::default())`.  Its code at `9c69f22` is `    restore(p, seconds)?;`, so its offset is 1.
* The lines of findings 1-41 and 1-54 of round 1 of M8 go to the validator.  They are `xtask/src/e2e/save.rs:427:39: replace * with + in save_once`, and the same with `* with /`.  They are in the same function as 2-17, and `2fdecda` fixed them with tests.  Their code at `f28d22d` is `    let item_y = o.item_y + o.item_dy * row as f64;`.
* The same mutation and file with another code line goes to the validator.  So does the same mutation in another file.
* Two lines `replace == with != in run` are on one code line, at two columns.  An entry with the offset of one makes only that one known.
* A line whose file `line_at` cannot read goes to the validator.  So does a line with no `<line>:<col>: ` part.
* The output keeps the order of the input, and has no empty line.

### Step 2: `review mutants` writes `missed.txt` and `known.txt`

* `prepare_mutants` returns the round folder and the round's state, so `mutants` has `head`.
* After cargo-mutants ends, `mutants` reads `<round>/mutants.out/missed.txt`, if it exists.  It calls `split` with `KNOWN_MISSED`.
* `line_at` reads the file at the round's `head` with `git show <head>:<file>`.  cargo-mutants copies the working tree.  `review start` found that tree clean at `head`, and the skill commits no fix before step 10.  If the two differ, a line only goes to the validator.
* It writes `<round>/missed.txt` (the lines to validate) and `<round>/known.txt` (each line, a tab, and its TD).  Each line ends with a newline.  It writes both files, empty if need be, also when cargo-mutants wrote no `missed.txt`.  It does not change anything in `mutants.out/`.
* `clear_old_output` also removes an old `missed.txt` and `known.txt` of the round.
* The mutants line gets `, known <k>` after `not tested <n>`, and before the reason.  `<k>` is the number of known lines.  The line is the same in `mutants.txt` and on stdout.  It is `known 0` when the run made no `missed.txt`, also for the class `none`.
* `mutants` uses `KNOWN_MISSED`.  A seam, such as a `mutants_with(…, list)` that `mutants` calls, lets the run test give its own list.
* Tests of step 2, in `xtask/src/review.rs`:
  * The fake cargo writes a `mutants.out/missed.txt` with two lines, one on the fixture list.  The test repo's committed file holds the entry's code at that line.  After the run, `missed.txt` holds the other line.  `known.txt` holds the known line and its TD.  The line ends `, known 1`.
  * A run whose fake writes no `missed.txt` gives two empty files and `known 0`.
  * The test of `clear_old_output` checks that an old `missed.txt` and `known.txt` go.
  * Each test that pins the line (`COUNTED`, `NOTHING` and the others) gets `, known <k>`.  These are the "log format test" of the parent plan.  The rounds table of the log keeps its columns, since the line is one cell.

### Step 3: the entries, and the test that keeps them true

* Fill `KNOWN_MISSED` from the 29 rows of class 4.  For each row, find the mutant at `759abfa` with `cargo mutants --list --line-col=true -f <file>`.  Take its mutation text, its code line and its offset.  Each entry has the TD of its row's action.
* Each entry is one mutant.  A row that repeats an earlier row gives no new entry.
* A row of M9 has no column, such as 1-33 (`replace == with != in run`).  The code at the row's commit must show which mutant it is.  If its line has two such operators, add no entry for it.  Then the validator sees each of them.
* Leave out a row whose mutant `--list` no longer shows.
* Leave out a mutant outside a function, such as the five rows at `2c7c29b:xtask/src/e2e/save.rs:156`.  They mutate the constant `WAITS_S`.
* A commit in parentheses after a TD, as in "TD-39 (1fcd8bd review round 2 fixes)", is the commit that wrote the TD.  Keep such a row.  Leave out a row only if its fix commit added a test for that mutant.
* Record each entry and each row left out in Surprises.
* `fn known_problems(list: &[Known], tech_debt: &str, read: impl Fn(&str) -> Option<String>) -> Vec<String>` gives one problem for each of these:
  * a `debt` that is not an open TD: no `### <debt>: ` heading in `tech_debt`, or a bullet `* <debt>:` under its `## Closed`;
  * a `file` that `read` cannot read;
  * a `mutation` whose function name (see below) is not a Rust identifier;
  * a file with no `fn <name>` line for that function;
  * a `code` line that is in the body of no function of that name;
  * a `code` line that the file has more than once, trimmed;
  * two entries that are the same.
* A `fn <name>` line has `fn`, a space, the name, then `(` or `<`.  `pub`, `pub(…)` or `async` can come before `fn`.  So `fn restore` does not match `fn restore_check(`.
* The body of a function runs from its `fn` line to the first later line that is `}` at the `fn` line's indent.  A function that ends on its `fn` line has that one line.  Any function of the name counts.
* The function of a mutation is the text after the last ` in `, if there is one.  Else it is the text after `replace ` up to ` -> ` or ` with `.  Take the last part after `::`, without a `<…>` part.  `replace StemApp::can_encode -> bool with true` gives `can_encode`.  `replace == with != in run` gives `run`.
* Tests of `known_problems`, with fixture entries and text: one for each problem above, and none for a good entry.  One test has `restore(p, seconds)?;` in two functions, and an entry that names neither.  One test has `fn restore_check(` and no `fn restore(`.
* `each_known_missed_entry_is_open_and_in_the_code`: `known_problems(KNOWN_MISSED, <docs/tech-debt.md>, <a reader of this repo's files>)` is empty.  This test passes with an empty list, so it is not a test that fails first.  The tests of `known_problems` are.
* A test with the real list, the line of 2-17 and its code: the line is known, with TD-39.  The lines of 1-41 and 1-54 go to the validator.  This is the acceptance of the parent plan, with the real list.  It fails with an empty `KNOWN_MISSED`.

### Step 4: the docs, and the diff for the orchestrator (DOC-1)

The implementer commits only text that says what `xtask` does:

* `docs/tech-debt.md`, TD-33:
  * A bullet: `xtask review mutants` writes the lines that `KNOWN_MISSED` (`xtask/src/review.rs`) matches to `known.txt`, and the rest to `missed.txt`.
  * A bullet: native code that only the wasm32 tests reach shows as missed, such as the JS readers of `protocol/src/lib.rs`.  The validator refutes it (16 lines in the measurement).
  * The first bullet says "A new `wasm32`-only function without the attribute shows as missed".  After it, add the count of the measurement: 11 lines in 22 rounds.
* TD-39 and TD-66: one sentence each.  The mutants that PR #39's rounds sent to the TD, and that `--list` still shows, are entries of `KNOWN_MISSED`.
* `docs/working-with-agents.md`, the Tools table, the `review mutants` cell: it writes `missed.txt` and `known.txt`, and the mutants line counts the known lines.  The changelog line for M2 says the same, and no more.
* `docs/harness.md`: if its row for `xtask review mutants` names what the run gives the validator, the diff below changes it.

The implementer gives the team leader one diff file, not a commit, with:

* `.claude/skills/review-rounds/SKILL.md`:
  * Line 44 ("The mutants run writes `mutants.txt` and `mutants.out/` there.") also names `missed.txt` and `known.txt`.
  * Step 6, item 3 reads `<run>/$1/missed.txt` and `<run>/$1/known.txt`.  It goes to item 8 only when both are empty or missing.  If neither file exists, the run is from an older `xtask`: read `mutants.out/missed.txt` as `missed.txt`.
  * Step 6, item 4 makes findings of the lines of `missed.txt` only.
  * A new item after item 4: each line of `known.txt` becomes a finding with the next ID.  Its fields are those of item 4.  Its verdict is `known`, and its `repeat-of` is the TD after the tab.  It gets no validator.  Its action in the log is "known: <TD>".
  * The new item moves the numbers of items 5 to 8.  Each "go to item <n>" in step 6 follows them.
  * Step 7, List B: a found finding is also one with the verdict `known`.
  * Step 9: the verdict list of a finding adds `known`.  The `repeat-of` field takes an ID or a TD.  Step 10: "Do not fix a `known` finding.  Its gap is in its TD."
  * Step 11's PR text: each `repeat-of` of a finding that is not `known`, and then each `known` finding with its TD, once.
* `xtask/tests/rules.rs`: the new lines of `REVIEW_SKILL_TEXT`, such as "Read `<run>/$1/missed.txt`" and "Its verdict is `known`", with the doc comment.
* `docs/working-with-agents.md`, "Review a change", item 4: the validator gets each missed mutant that is not known.  The agent logs a known one as `known`, with its TD.  The "Check:" line names each `known` finding.
* `docs/tech-debt.md`, TD-33: a sentence that the skill logs each line of `known.txt` as `known`, with no validator.
* `docs/harness.md`, if its row names what the validator gets.

The implementer checks the diff in a scratch worktree at the last commit of step 4.  `git apply --check` passes.  `cargo test -p xtask --test rules` passes with the diff, and fails with only the pins.  The team leader puts the diff in the scratchpad and gives it to the orchestrator at the checkpoint, before round 1.  So round 1 runs with the new skill, and its mutants run is the first live use.

### Step 5: review rounds

Run `cargo build -p xtask`, then `/review-rounds 759abfa <round> "<criteria>"` for rounds 1, 2 and 3.  Run more rounds until a round has no blocker and no significant should-fix.  No `--baseline`: M1 ran it for this PR.  The criteria are the parent plan's M2 section and this plan.  Each confirmed finding gets a fix with a test.  A nit gets a fix only if the team leader agrees.  The Write tool refuses the findings file and the log to the team leader (M1).  So each round stops after its fixes and `review clean`, and the orchestrator writes the files from the team leader's report.  After the last round, the orchestrator writes `docs/reviews/harness-phase-3b-m2.md` and removes the run folder (TD-37).

### Changes to the parent plan

The team leader gives these to the orchestrator, which edits the parent plan.  The edits land before round 1, with the skill diff, since round 1's criteria include the parent's M2 section.

* The M2 section: the parser filter and its first acceptance bullet are cut, with the reason of the Decision log below.
* The M2 section: the test of the mutants line (`COUNTED` and the others) changes in step 2.  The skill's pins change in the orchestrator's commit of the skill diff.  The parent says "in the same commit"; the Decision log has the reason.
* The M2 section: the list is the table `KNOWN_MISSED` in `xtask/src/review.rs`, and each entry also has a code line and an offset.
* The parent's Surprises: the counts of the measurement.  The parent's Decision log: the cut.
* M3: `repeat-of` can now hold a TD ID, for a `known` finding.  M3's match of reviewer findings to tech debt can use the same form.

## Validation and acceptance

* `cargo test -p xtask` passes, with the tests of steps 1 to 3.  Each test of steps 1 to 3 fails on its stub, on an assert, before the real code (Surprises records it).  The test of the real list fails with an empty `KNOWN_MISSED`.  The tests of step 2 fail on the code of `759abfa`, which writes neither file.
* The acceptance of the parent plan, with the cut of the Decision log:
  * The filter of wasm32 items is cut.  The measurement shows 11 such lines in 22 rounds, and TD-33 records the gap.
  * Findings 1-41 and 1-54 of round 1 of M8 still go to a validator (steps 1 and 3).
  * The mutant of finding 2-17 of round 2 of M8 is logged as known, with TD-39 (steps 1 and 3).
* The mutants line of round 1 of M2 has `, known <k>`, and the round folder has `missed.txt` and `known.txt`.  The team leader records them in Progress.  `<k>` is likely 0, since M2's diff changes no file of the list.
* `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo run -p xtask -- docs-check` pass.
* The review rounds end by the stop rule, and the log is in `docs/reviews/harness-phase-3b-m2.md`.

## Idempotence and recovery

* `review mutants` writes its two files again on each run, after `clear_old_output`, so a rerun gives the same files.
* A wrong entry cannot hide a mutant that it does not match exactly.  To undo the list, empty `KNOWN_MISSED`.  Every line then goes to the validator, as before M2.
* The skill reads `mutants.out/missed.txt` when neither new file exists, so an older `xtask` binary still works.
* Each step is its own commit, which `git revert` undoes.  A new team leader takes over from Progress.

## Progress

* [x] 2026-10-31: the measurement of the noise (team leader), in "The measurement" above.
* [x] 2026-10-31: plan written (`0b84126`).
* [x] 2026-10-31: plan review round 1 (one `Plan` agent): no blocker, 9 should-fix and 9 nits.  The agent recounted the measurement and found one count wrong (9 rounds, not 10).  All findings are fixed in this plan.
* [x] 2026-10-31: plan review round 2 (a new `Plan` agent), at `77ba7b8`: 1 blocker, 6 should-fix (2 significant) and 5 nits.  The blocker: a commit with failing tests breaks GIT-2.  All findings are fixed in this plan.  The review stops at its cap of 2 rounds.
* [x] 2026-10-31: step 1, `Known`, an empty `KNOWN_MISSED` and `split`, with 7 tests (implementer).
* [x] 2026-10-31: step 2, `review mutants` writes `missed.txt` and `known.txt`, and the mutants line ends `, known <k>` (implementer).
* [ ] Step 3: the entries and their test.
* [ ] Step 4: the docs, and the diff for the orchestrator.
* [ ] Review rounds.
* [ ] M2 done: this plan moves to `plans/completed/`.

## Surprises and discoveries

* 2026-10-31, team leader: the counts of the measurement, in "The measurement" above.  The rounds after `6d0a39d` have 261 missed lines.  68 are in a file that `.cargo/mutants.toml` now leaves out.  11 are in wasm32-only items, 0 in wasm32 blocks, 29 known-missed and 153 others.
* 2026-10-31, team leader: every round of the M8, M9 and catch-up logs comes after `6d0a39d`, the catch-up rounds included.  The M8 log also has the rounds "CI fix" and "speed 1" to "speed 8".
* 2026-10-31, team leader: the wasm32 variants of `mapped_floats`, `base_latency` and `main` are items of their own, with `#[cfg(target_arch = "wasm32")]`.  `.cargo/mutants.toml` keeps them without `mutants::skip`.  That rule comes from `exclude_re`, which matched by name and so hid the native variant too (`plans/completed/stream-from-storage-catchup.md`, D4).  With the attribute on one item, the native variant keeps its mutants.  M2 does not change this (Decision log).
* 2026-10-31, team leader: the refuted mutants of `protocol/src/lib.rs` (16) are in functions that compile natively, with no `cfg`.  Only the wasm32 tests in `protocol/tests/web.rs` reach them.
* 2026-10-31, team leader: at `759abfa`, `note_run` of `xtask/src/e2e/weights.rs` has three `replace || with && in note_run` mutants (lines 280 to 282).  At `0789e6b`, TD-66 named two (lines 254 and 256).  Step 3 maps each row to the code of today.
* 2026-10-31, implementer: step 1's tests failed first on the stub of `split`, on `assert_eq`.  All 6 tests of the plan failed: the stub gave each line as known, with the debt `""`.
* 2026-10-31, implementer: step 1 adds a seventh test, `split_reads_no_file_for_a_line_that_no_entry_names`.  `split` calls `line_at` only for a line that an entry names, so step 2 runs no `git show` for most lines.  The test also failed on the stub.
* 2026-10-31, implementer: step 2's tests failed first on the stubs, on `assert_eq`.  The stubs were a `Line` that printed no `known`, a `mutants_with` that wrote no file, and the old `clear_old_output`.  21 tests failed: the 17 tests that pin the line of `classify`, the two run tests, the clear test and the split test.  With `known` in the line, three tests still failed: no `missed.txt` (`None`, not `Some("")`) in the two run tests, and "nor the missed lines of the last run" in the clear test.  `mutants_splits_the_missed_lines_by_the_list` failed with `known 0`, not `known 1`.
* 2026-10-31, implementer: the fake `cargo` copies `missed.txt` from its own folder to `mutants.out`, if it is there.  So a test gives the missed lines without an environment variable, which tests that run at the same time would share.

## Decision log

* 2026-10-31, team leader: M2 cuts the parser filter of wasm32 items (the parent plan's second bullet, and its first acceptance bullet).  Reason: it removes 11 of the 193 lines left (6%), in 2 of 22 rounds, and 4 of those by design.  It needs an item parser and a `cfg` evaluator on `proc-macro2`, and a change to the mutants flow, for that yield.  TD-33 records the gap.
* 2026-10-31, team leader: M2 keeps the known-missed list.  Reason: it removes 29 lines (15%), in 9 rounds, and 11 of them came back in a later round.  It is the mutants' side of M3's `repeat-of` match to tech debt, and it needs no parser.
* 2026-10-31, team leader: M2 does not add `mutants::skip` to the wasm32 variants of `mapped_floats`, `base_latency` and `main`.  Reason: 4 lines in 22 rounds.  The change would reverse a rule of `.cargo/mutants.toml` and TD-33.  It also needs a `cargo mutants --list` check of each native variant.  A later change can do it.
* 2026-10-31, team leader: the list is a Rust table (`KNOWN_MISSED`) in `xtask/src/review.rs`, not a text file or a new module.  The compiler checks its fields.  A change to it routes all six reviewers, and `review start` warns when the binary is older.  `cargo build -p xtask` runs before each round, so the binary has the list of `HEAD`.
* 2026-10-31, team leader: an entry matches by file, mutation text, code line and offset, not by line number.  Line numbers move with each edit.  The text alone does not tell two operator mutants of one function apart.  The code line alone does not tell two on one line apart.  A wrong match must never hide another mutant, so each part must be equal.
* 2026-10-31, team leader: a known line gets the new verdict `known`, not `pre-existing`.  Step 10 of the skill fixes each `pre-existing` finding or sends it to tech debt.  A known line is in tech debt already, so it needs no action.
* 2026-10-31, team leader: plan review round 1 found that the test of the real list passes with an empty list.  So the tests that fail first are those of `known_problems`, with fixture entries.
* 2026-10-31, team leader: the stubs stay in the working tree, and no commit holds a failing test (GIT-2).  Plan review round 2 found the first form, a committed stub, a blocker.  Each stub gives a wrong answer, so each test fails on it.
* 2026-10-31, team leader: the test of the mutants line changes in step 2, and the skill's pins change in the orchestrator's commit.  The parent plan asks for one commit.  Reason: the skill is the orchestrator's to edit.  The pins do not pin the mutants line, so each commit passes its tests.
* 2026-10-31, team leader: the plan review stopped at its cap of 2 rounds.  Round 2's findings are fixed in this plan, with no third review.

## Outcomes and retrospective

To be written when M2 completes.
