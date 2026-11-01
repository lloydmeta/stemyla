# Harness phase 3b, M3: the fix rule and the stop rule (decision 0024)

## Purpose

This is the milestone plan for M3 of `plans/active/harness-phase-3b.md`.  It gives more detail than the parent plan's M3 section, which stays the criteria.  M3 puts decision 0024 into the harness.

After M3:

* The `review-rounds` skill, `CLAUDE.md`, `docs/working-with-agents.md` and decision 0011 say the same fix rule and stop rule.  A test pins one sentence of the rule in each place.
* Rounds 1 to 3 fix each found finding.  From round 4, a round fixes only blockers, significant should-fix and confirmed findings on a strict rule.  It logs the rest as `reported, not fixed`.
* The rounds stop at the first round from round 3 on with none of those three.  From round 4, the reviewers read only the fix range and the files that it touches.
* The findings log has a `Significant` column, which the team leader fills.  `docs-check` checks each log in `docs/reviews/` that has the column against the fix rule.  `xtask review log-check` checks the log of a run after each round.
* A finding that repeats an earlier finding that went to tech debt gets the verdict `known`, not `refuted`, and no validator.

Readers: the implementer of M3, the review agents of M3's rounds, and the orchestrator.

## Context and orientation

The state at the start of M3, on 2026-10-31, at commit `666d83d` on branch `harness-phase-3b`:

* `.claude/skills/review-rounds/SKILL.md` (330 lines) runs one round.  The parts that M3 changes:
  * Step 3 starts `reviewer` on `<base>..HEAD` in each round, and the other reviewers of the since route on `<since>..HEAD`.  `reviewer` is in each since route that is not empty, with the reason "every diff" (`route`, `xtask/src/review.rs`, near line 225).
  * Step 4, item 4 reads `<run>/<r>/findings.md` of each earlier round.  A finding that is the same as a `refuted` one gets `repeat-of` and the verdict `refuted`.  Step 6, item 6 does the same check for the missed mutants.  `REVIEW_SKILL_TEXT` pins "Do the `repeat-of` check of step 4".
  * Step 6, item 8 merges a reviewer's finding and a mutant's finding.  The merged finding keeps the reviewer's ID, and the step says nothing about its rule.  So a merged row can have the rule `none`, as 1-4 of `docs/reviews/iphone-crashes-2026-10-25-catchup.md` has.
  * Step 9 gives the template of `findings.md`.  It has no `significant` field.
  * Step 10 fixes each found finding in each round.
  * Step 11 gives the log's two tables.  The findings table has 13 columns and no `Significant` column.  The Action bullet says "the fix commit and its subject; or the reason for no change, such as "refuted", or the tech-debt ID".  "After round 3" copies the log to `docs/reviews/` and gives the PR text, which names each `known` mutant once.
* `CLAUDE.md` (88 lines; the map limit is 100), the line "Review:" says to run `/review-rounds` "for rounds 1, 2 and 3".
* `docs/working-with-agents.md` says "three review rounds" near lines 17 and 33.  The recipe "Review a change" (near line 76) gives the prompt "then rounds 2 and 3", and item 6 says to fix each found finding.
* `docs/README.md`, "## Conventions" (near line 83), lists what `docs-check` holds the docs to.
* Decision 0011 is accepted.  Its D6 bullet says "three rounds, as `CLAUDE.md` says", and its D2 bullet says the log is written "after round 3".  Decision 0024 amends D6.  The parent plan's M3 moves the time of the log.  Only a person changes an accepted decision's rule text, so M3 proposes only a pointer line (step 4).
* Decision 0024 holds the sentence "From round 4, a round fixes blockers, significant should-fix and every confirmed finding on a strict rule."  Its strict rules are "PROTO-1, PROTO-2, MEM-1 to MEM-5, RT-1, GPU-1 to GPU-3, VERIFY-1, GIT-1, DEPS-1, ASSET-1": 15 IDs, each a heading of `docs/invariants.md`.
* `docs/tech-debt.md`: TD-33 says what the mutants run cannot test, and how `KNOWN_MISSED` works.  TD-37 says "`CLAUDE.md` tells the agent to run review rounds 1, 2 and 3", and names the skill's step "After round 3".
* `xtask/src/docs.rs`: `check` (near line 83) runs each check of `docs-check` on the markdown files.  `is_agent_doc` includes `docs/reviews/*.md`.  No check reads a log's findings table.  `Problem::new(check, file, message)` makes one problem.
* `xtask/src/review.rs`: `run_with` dispatches `start`, `check`, `mutants` and `clean`.  Each failure exits 3 (`REFUSED`).  `USAGE` names the forms.
* `xtask/tests/rules.rs`: `REVIEW_SKILL_TEXT` (near line 1137) pins lines of the skill, and `the_review_skill_starts_with_xtask` fails a missing one.  The rounds table header is pinned; the findings table header is not.
* `xtask clean` (M1) keeps a run folder until its `log.md` equals the copy in `docs/reviews/`.  So a copy after the last round, not after round 3, needs no change to `xtask clean`.
* The logs in `docs/reviews/` write a `|` in a cell as `\|`, as in `replace && with \|\| in note_run`.  Their Action cells are free text.  The common forms are `<sha> review round <n> fixes`, `refuted`, `TD-<n>`, `TD-<n>; <sha> ...` and `merged into <ID>; <sha> ...`.  The skill gives `known: <TD>` since M2, but no log has it yet.
* In M1, the intent review's fix rounds were a new run.  Their rows went into the log as rounds 5 to 7, and they fixed nits (5-2, 5-6, 6-3, 7-3 to 7-8).
* `target/review/run/` is empty, so M3's rounds start a new run with base `666d83d`.
* No new crate is needed (DEPS-1).

Terms:

* Found finding: a finding with the verdict `confirmed` or `pre-existing` (the skill, step 7).
* Strict rule: a rule of decision 0024's list.  A row is on a strict rule when its Rule cell names one of them.  A row with `cargo-mutants` in its Reviewers is on VERIFY-1.
* Significant: the team leader's mark on a blocker or should-fix, by decision 0024's test.  The test: a rule or decision fails in a case a user can reach, or later work goes wrong.  A contradiction or a misleading measure is significant too.
* Fix range: `<since>..HEAD` of a round, the commits since the previous round started.
* Late round: a round from round 4 on.
* Part of an Action cell: the text between two `;`, trimmed.  A cell with no `;` has one part.
* Word: a run of letters and digits.  So `m3-r4.diff` holds the words `m3`, `r4` and `diff`.
* Not-fixed part: a part that starts with `reported, not fixed`, `refuted`, `known: `, `no change:` or a TD ID (`TD-` and digits).
* Merge part: a part that starts with `merged into `.
* Fix part: another part that holds a commit SHA or the word `diff`.  A commit SHA is a word of 7 to 40 hex characters with at least one digit.
* Fix action: an Action cell with a fix part.
* Not-fixed action: an Action cell with no fix part, at least one not-fixed part, and no other part but merge parts.
* So `TD-39; abc1234 review round 4 fixes` is a fix action.  `no change: the since diff does not touch it` is a not-fixed action.

## Plan of work

### How the team works

* The team leader writes this plan.  A new `Plan` agent reviews it, at most 2 rounds, and the team leader fixes what it finds.  The team leader commits the plan and its index line, signed, before step 1.
* The team leader starts one implementer (`general-purpose`) with this plan's path.  The implementer does steps 1 to 4.
* Each step is one or more signed commits (`git commit -S`).  Before each commit, run these (GIT-2):
  * `cargo fmt --all --check`;
  * `cargo clippy --workspace --all-targets -- -D warnings`;
  * `cargo test -p xtask`;
  * `cargo run -p xtask -- docs-check`.
* M3 changes only `xtask` and docs, so the other clippy lines of `CLAUDE.md` do not apply.
* A new test fails first on an assert, not on a compile error.  So the implementer first writes each new function as a stub that gives a wrong answer, in the working tree:
  * `strict_rules` gives an empty list.  `the_strict_rules_are_those_of_decision_0024` (step 1) fails on it.
  * `fix_rule_problems` gives one problem, `stub`, for each text.
  * `review_logs_follow_the_fix_rule` gives one problem, `stub`, for each file.
  * `log_check` prints nothing and exits 0.
* Each test checks the whole output, so each test fails on its stub.  The implementer runs the new tests on the stubs, and records the failures in Surprises.
* Then it writes the real code, and commits the tests and the code together.  No commit holds a failing test (GIT-2).
* The implementer does not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.  Step 4 gives the team leader one diff file for the orchestrator.
* Tests use fixture text and temporary folders only.  No test reads this repo's `target/review/`.
* Docs and commit messages are in ASD-STE100, as `CLAUDE.md` says.

### Step 1: the check of a log's findings table

Add `xtask/src/review_log.rs`, and `pub mod review_log` in `xtask/src/lib.rs`.  It holds pure functions only, with a doc comment that names decision 0024.

* `pub const STRICT_RULES_TEXT: &str = "PROTO-1, PROTO-2, MEM-1 to MEM-5, RT-1, GPU-1 to GPU-3, VERIFY-1, GIT-1, DEPS-1, ASSET-1";`.  This is the text of decision 0024.
* `pub fn strict_rules() -> Vec<String>`: the IDs of `STRICT_RULES_TEXT`, in order.  `X-a to X-b` gives `X-a`, `X-a+1` and so on to `X-b`.  So it gives 15 IDs.
* `pub fn fix_rule_problems(text: &str) -> Vec<String>`: the problems of one log.  It reads the first table under `## Findings`.
  * If that table has no `Significant` column, it gives no problem.  So the logs before M3 pass.
  * It skips the separator line below the header.
  * A cell ends at a `|` that has no `\` before it.  Each cell is trimmed.
  * The header must have the columns `Round`, `Finding`, `Reviewers`, `Severity`, `Significant`, `Rule`, `Verdict`, `Repeat of` and `Action`.  The check finds each by its name, not by its place.  A missing column gives one problem that names it, and no row problems.
* The problems of a row, each with the finding's ID and a fix, in the form `<ID>: <what is wrong>.  Fix: <the change>.`:
  1. The row has a different number of cells than the header.  Then the check gives this one problem for the row.
  2. Round is not a whole number of 1 or more.
  3. Severity is not `blocker`, `should-fix` or `nit`.
  4. Verdict is not `confirmed`, `pre-existing`, `refuted`, `unproven`, `unvalidated` or `known`.
  5. Significant is not `yes`, `no` or `-`.
  6. A blocker or should-fix with the verdict `confirmed`, `pre-existing` or `unproven` has `-` in Significant.  Or another row has `yes` or `no` there.  A `known` finding gets no value (parent plan, M3).
  7. A `known` row has no Action `known: <TD>`, or its Repeat of is not that TD.
  8. The Action is neither a fix action nor a not-fixed action, such as `own PR` or `merged into 1-1` alone.
  9. A `refuted` or `known` row has a fix action.
  10. A row that the round must fix has a not-fixed action other than a TD ID.  A TD ID passes, since a team can send a confirmed mutant to tech debt (TD-39).  A `no change:` part does not pass a `confirmed` row.  A `pre-existing` row that the round must handle has a not-fixed action with no TD ID and no `no change:` part.
  11. In a late round, a row that the round may not fix has a fix action.  A `refuted` or `known` row gets problem 9 only.
* What a round must fix:
  * In rounds 1 to 3: each `confirmed` row.
  * In a late round: each `confirmed` row that is a blocker, has `yes`, or is on a strict rule.
* What a round must handle: a fix, a TD or its own PR (`no change:`).
  * In rounds 1 to 3: each `pre-existing` row, nits included.  A baseline nit can get `pre-existing` by `same-as`.
  * In a late round: each `pre-existing` row that is a blocker, has `yes`, or is on a strict rule.
* What a late round may fix: each row with the verdict `confirmed`, `pre-existing` or `unproven` that is a blocker, has `yes`, or is on a strict rule.  An `unvalidated` nit is never fixed in a late round, also on VERIFY-1.  A `confirmed` nit on a strict rule, such as a baseline nit with `same-as`, must be fixed.
* A Rule cell is on a strict rule when a token of it is an ID of `strict_rules()`.  The cell splits into tokens at each character that is not a letter, a digit or `-`.  `VERIFY-1`, `MEM-1, MEM-5`, `MEM-1/MEM-2`, `RT-1's rule`, `VERIFY-1; D9 a quote` and `GPU-2 (a quote)` are on a strict rule.  `DOC-1`, `TD-33` and `none` are not.
* In `xtask/tests/rules.rs`, add `the_strict_rules_are_those_of_decision_0024`, with the stub of `strict_rules`.  It checks three things:
  * `strict_rules()` has 15 IDs;
  * decision 0024's file holds `STRICT_RULES_TEXT`;
  * each ID has a heading in `docs/invariants.md` (`xtask::docs::defined_ids`).
* Tests of step 1, in `xtask/src/review_log.rs`, with fixture logs:
  * `strict_rules()` gives the 15 IDs in order.
  * A log with no `Significant` column and bad rows gives no problem.
  * A good log gives no problem.  Its rows:
    * round 2: a fixed should-fix, and a fixed nit;
    * round 4: a fixed blocker, a fixed should-fix with `yes`, and a fixed should-fix with `no` on VERIFY-1;
    * round 4: a fixed should-fix with `no`, the rule `none` and `cargo-mutants` in its Reviewers;
    * round 4: a nit and a should-fix with `no` on DOC-1, both `reported, not fixed`;
    * round 4: a `confirmed` should-fix on VERIFY-1 with the action `TD-39`;
    * round 4: a fixed `unproven` blocker, a fixed `pre-existing` should-fix with `yes`, and a fixed `confirmed` nit on VERIFY-1;
    * round 4: rules `MEM-1, MEM-5` and `GPU-2 (a quote)` on fixed should-fix with `no`;
    * round 4: `TD-39; abc1234 review round 4 fixes` on a fixed should-fix on VERIFY-1, and `no change: the since diff does not touch it` on an `unproven` should-fix with `no`;
    * a merged mutant row with `merged into 4-1; abc1234 review round 4 fixes`;
    * a `known` row on VERIFY-1 with `known: TD-39`, a `refuted` row, a fix in a diff (`the orchestrator's commit of m3-r4.diff`), and a claim with `\|`.
  * Round 4: each of these gives problem 11:
    * a fixed nit, also on VERIFY-1;
    * a fixed should-fix with `no` and the rule `DOC-1`;
    * a nit with the action `TD-39; abc1234 review round 4 fixes`;
    * a fixed `pre-existing` should-fix with `no` and the rule `none`.
  * Problem 10: a `confirmed` nit on VERIFY-1 in round 4 with `reported, not fixed`.
  * Problem 10: a `confirmed` should-fix on VERIFY-1 with `reported, not fixed` in round 4.  Also a `confirmed` should-fix in round 2 with `reported, not fixed`.
  * Problem 7: a `known` row with the action `refuted`, or with `known: TD-39` and Repeat of `TD-66`.
  * Problem 9: a `refuted` row with `abc1234 review round 1 fixes`, and one with `refuted; abc1234 review round 1 fixes`.
  * Problem 8: the action `own PR`.
  * Problem 5: Significant `maybe`.  Problem 6: a nit with `yes`, and a `confirmed`, a `pre-existing` and an `unproven` should-fix with `-`.
  * A row with one cell too few gives only problem 1.  The round `speed 1` gives problem 2.  The severity `major` and the verdict `confirmd` give problems 3 and 4.
  * A header with `Significant` and no `Action` column gives one problem that names the missing column.

### Step 2: `docs-check` runs the check

* In `xtask/src/docs.rs`, add `fn review_logs_follow_the_fix_rule(markdown: &[(String, String)]) -> Vec<Problem>`.  It runs `fix_rule_problems` on each file under `docs/reviews/`.  Each problem gets the check name `reviews` and the file.  `check` calls it.
* Tests, in `xtask/src/docs.rs`:
  * A fixture list with two files gives exactly one problem.  The first is a log under `docs/reviews/` with the column and a fixed nit in round 4.  The second has the same text in `docs/plans/active/a.md`.  The problem has the check `reviews` and the first file.
  * A list with only a good log under `docs/reviews/` gives no problem.
* `docs-check` on this repo still passes, since no log has the column yet.
* `docs/README.md`, "## Conventions": a bullet that a review log with a `Significant` column follows the fix rule of decision 0024.
* `docs/working-with-agents.md`, the Tools row of `docs-check`: it also checks each review log that has a `Significant` column against the fix rule of decision 0024.

### Step 3: `xtask review log-check <file>`

The check of step 2 sees a log only after the last round, when the log is copied.  So a wrong fix would show only at close-out.  This command checks the log of a run after each round.

* In `xtask/src/review.rs`, `run_with` gets `log-check <file>`, and `USAGE` names it.
* `log_check(path)` reads the file and runs `fix_rule_problems`.
  * It prints each problem, and exits 3.
  * If the file has no `Significant` column, it prints that, and exits 3.  The skill writes the column, so a log of a new run without it is wrong.
  * If it cannot read the file, it prints the error, and exits 3.
  * Else it prints `log-check: the log follows the fix rule`, and exits 0.
* Tests, on files in a temporary folder: a good log exits 0 with that line.  A log with a fixed nit in round 4 exits 3 and prints the problem.  A log with no column exits 3.  A missing file exits 3.
* `docs/working-with-agents.md`, the Tools row of the `review` commands: `review log-check <file>` checks a run's log against the fix rule of decision 0024.

### Step 4: the diff for the orchestrator (DOC-1)

The implementer writes one diff file, not a commit, and gives its path to the team leader.  It holds every text that says the rule, so the rule lands in one commit (decision 0024, Consequences; M1's retrospective).

`.claude/skills/review-rounds/SKILL.md`:

* Step 3: rounds 2 and 3 stay as they are.  The late range, in short sentences:
  * From round 4, start each reviewer of the since route on `<since>..HEAD`, `reviewer` included.
  * Its reasons come from the since route.
  * It reads the fix range and the files that it touches (decision 0024).
  * If the since range is empty, start no reviewer.  The round has no findings, so it is the last round.
* Step 4, item 4: the `repeat-of` check also matches tech debt.
  * Also read `<run>/log.md`, `docs/reviews/<branch>.md` and each `docs/reviews/<branch>-m<n>.md`.  `<branch>` is the last part of `<run>`.
  * Find each row whose Action is a TD ID or `known: <TD>`.  Skip a TD that `docs/tech-debt.md` lists under `## Closed`.
  * If a finding of this round is the same finding as such a row, give it `repeat-of:` and that TD.
  * Give it the verdict `known`.  It gets no validator.
  * A `repeat-of` of a `refuted` finding still gives `refuted`.
* Step 6, item 6 keeps the pin "Do the `repeat-of` check of step 4".  A finding whose `repeat-of` is an ID gets `refuted`.  A finding whose `repeat-of` is a TD gets `known`.  Neither gets a validator.
* Step 6, item 8: the merged finding's rule also names VERIFY-1.
* Step 9: each finding gets the field `* significant: <yes or no; or ->`.
  * You fill it, not a reviewer.
  * A blocker or should-fix with the verdict `confirmed`, `pre-existing` or `unproven` gets `yes` or `no`, by decision 0024's test.
  * Each other finding, `known` included, gets `-`.
* Step 10: the fix rule.
  * Rounds 1 to 3: fix each found finding, as today.  Fix a nit only if you agree with it.
  * The pinned sentence, from decision 0024: "From round 4, a round fixes blockers, significant should-fix and every confirmed finding on a strict rule."
  * From round 4, log each other finding as `reported, not fixed`, nits included.  Do not fix it.
  * The strict rules are `STRICT_RULES_TEXT`, word for word.
  * A mutant that the team sends to tech debt gets a `KNOWN_MISSED` entry in the milestone's last commit, not in a round's fixes.  Reason: a change to `xtask/src/review.rs` routes all six reviewers (`ROUTE_ALL`).
* Step 11:
  * The findings table gets the column `Significant` after `Severity`, with the value of the findings file.  The template header and its separator line get 14 cells.
  * The Action bullet gives the forms:
    * the fix commit and its subject, or the diff that holds the fix;
    * `reported, not fixed`; `refuted`; `known: <TD>`; a TD ID;
    * `no change:` and the reason, such as its own PR;
    * `merged into <ID>; ` and the action of that finding.
  * A `no change:` reason names no commit SHA.
  * After the log row, run `target/debug/xtask review log-check <run>/log.md`.  If it names a problem, correct the row or the fix before the next round.
  * Where the Write tool refuses the log (lines 30 and 286), the caller writes the log, then runs `review log-check`, then `review clean`.  The pins hold both commands.
  * The stop rule, in short sentences.  The round is the last round when all of these are true:
    * its number is 3 or more;
    * it has no `confirmed` blocker, no `confirmed` should-fix with `yes`, and no `confirmed` finding on a strict rule;
    * it fixed, or sent to a TD, no `pre-existing` or `unproven` blocker, should-fix with `yes`, or finding on a strict rule.
  * A `known` finding does not count.  A `confirmed` finding that went to a TD in this round counts too, so the next round runs.  There is no cap.
  * "After round 3" becomes "After the last round".  Else, the next round follows.
  * The PR text lists each `reported, not fixed` finding.  It lists each `known` finding once with its TD, not only each `known` mutant.

`CLAUDE.md`, the line "Review:", as one line:

* Run `cargo build -p xtask`, then `/review-rounds` from round 1 until the stop rule of decision 0024 holds.
* The skill verifies each blocker and should-fix before a fix.
* Rounds 1 to 3 fix each confirmed finding, with tests.  The pinned sentence follows.
* "Fix the real ones with tests" goes, since it contradicts the late rule.
* The line count stays at 88.

`docs/working-with-agents.md`:

* Near lines 17 and 33: review rounds until the stop rule, not three rounds.
* The recipe "Review a change": the prompt runs rounds until the stop rule.  Item 6's "It fixes each found finding" names rounds 1 to 3.  It gets the pinned sentence, the `Significant` column and `review log-check`.  The "Check:" line names the `reported, not fixed` findings in the PR text.
* A changelog line for M3.

`docs/decisions/0011-review-agents.md`: a proposal for the orchestrator.

* A new line after the Status line: "Amended on 2026-10-31: decision 0024 amends the rounds (D6), and `plans/active/harness-phase-3b.md` (M3) moves the log to after the last round (D2)."  Then the pinned sentence.
* It leaves the text of D2 and D6 as it is.
* If the orchestrator refuses it, the orchestrator removes this hunk and the 0011 entry of the test below.  Then the parent plan's acceptance of "four places" is not met.  The team leader records that in this plan's Decision log, and gives a line for the parent's Progress.

`docs/tech-debt.md`:

* TD-33: the sentence on `KNOWN_MISSED` entries in the milestone's last commit, with its reason.
* TD-37: `CLAUDE.md` tells the agent to run rounds until the stop rule.  The step is "After the last round".

`xtask/tests/rules.rs`:

* `REVIEW_SKILL_TEXT` gets the new lines.  The doc comment of `REVIEW_SKILL_TEXT` names them:
  * the pinned sentence and the stop rule;
  * the late range of step 3, and its empty case;
  * the `known` verdict of a TD `repeat-of`, and the logs that step 4 reads;
  * the VERIFY-1 rule of a merged finding;
  * the `significant` field, and the `KNOWN_MISSED` timing;
  * `target/debug/xtask review log-check <run>/log.md`, also for the caller where the Write tool refuses the log;
  * "After the last round", and the `reported, not fixed` PR line.
* `the_fix_rule_says_the_same_in_each_place`: the pinned sentence is in `CLAUDE.md`, the skill, `docs/working-with-agents.md`, decision 0011 and decision 0024.  Each missing place gives its own line in the failure.
* `the_review_skill_names_the_strict_rules`: the skill holds `STRICT_RULES_TEXT`.
* `the_review_skill_log_passes_the_fix_rule_check`: the skill's findings table template, its header and separator line, has the `Significant` column.  `fix_rule_problems` on that template under `## Findings` gives no problem.  So the template and the check use the same column names.

The implementer checks the diff in a scratch worktree at the last commit of step 3:

* `git apply --check` passes.
* With only the `xtask/tests/rules.rs` part, `cargo test -p xtask --test rules` fails on an assert of each new pin.
* With the whole diff, `cargo test -p xtask`, `cargo run -p xtask -- docs-check` and `cargo fmt --all --check` pass.
* The implementer records the failures in Surprises.

The team leader puts the diff in the scratchpad, checks it again on `HEAD`, and gives it to the orchestrator at the checkpoint before round 1.  So round 1 runs with the new skill and the new log column.

### Step 5: review rounds

* Run `cargo build -p xtask`, then `/review-rounds 666d83d <round> "<criteria>"` from round 1, with no `--baseline`.  M1 ran the baseline for this PR.
* The criteria are the parent plan's M3 section at the commit before round 1, and this plan.
* The rounds follow the new rule: the fix rule of step 10, the late range of step 3, and the stop rule of step 11.
* The team leader fills the `Significant` field and column.  It writes the log text to `m3/log.md` in its scratchpad, and runs `target/debug/xtask review log-check` on that file.  Then it gives the files to the orchestrator.
* Each fix of behaviour gets a test that fails first.
* The Write tool refuses `findings.md` and `log.md` to the team leader (M1, M2).  So the team leader runs each round to step 11: the fixes, the commit and `review clean <round>`.  Then it gives the orchestrator the files as text, and waits.  The orchestrator asked for this order; `review clean` removes only worktrees, so the order changes no count.
* A fix of `.claude/**` or `CLAUDE.md` goes to the orchestrator as a diff.
* After the last round, the orchestrator writes `docs/reviews/harness-phase-3b-m3.md`.  `docs-check` then runs the check of step 1 on it.

### Changes to the parent plan

The team leader gives these to the orchestrator as text.

* The M3 Progress line, with the stats of the rounds.
* A Decision log line, agent: the action forms of the log check, and why a TD ID passes problem 10.
* A Decision log line, agent: a later set of rounds on a milestone, such as after an intent review, goes on with the same run.  The orchestrator runs the intent review before `clean --yes`, so the run folder stays.  Then it runs `/review-rounds <base> <N+1>`, so those rounds follow the late rule.  `review start` refuses round `N+1` without `<run>/<N>/state.json`.  If the run folder is gone, the gap goes to TD-37.
* A Decision log line, agent: decision 0011 gets a pointer line, not a change to D2 or D6, if the orchestrator accepts the hunk.
* A Decision log line, agent: `xtask review log-check`, which the parent's M3 section does not name.  Reason: `docs-check` sees a log only after the last round.

## Validation and acceptance

* `cargo test -p xtask` passes, with the tests of steps 1 to 3.  Each test failed on its stub, on an assert (Surprises).
* The parent plan's acceptance:
  * `fix_rule_problems` is a pure check of a log's findings table.  It applies only to logs with the `Significant` column, so the logs before M3 pass.
  * Its tests accept the new column.  They reject a fix of a nit from round 4, and of a should-fix with `no` that is not on a strict rule.
  * `docs-check` runs it on each file in `docs/reviews/` with the column.
  * The pinned skill lines change.  `the_fix_rule_says_the_same_in_each_place` pins one sentence in each of the four places, and in decision 0024, if the orchestrator accepts the 0011 hunk.
* With the orchestrator's commit of the diff, `cargo test -p xtask --test rules` passes.
* `review log-check` passes on the log after each round.  `docs/reviews/harness-phase-3b-m3.md` has the `Significant` column, and `docs-check` passes on it.
* `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo run -p xtask -- docs-check` pass.

## Idempotence and recovery

* The check reads text only, so a rerun gives the same problems.
* A log with no `Significant` column is never checked by `docs-check`.  To undo the check, remove the call in `check`.
* Each step is its own commit, which `git revert` undoes.  The orchestrator's commit of the diff is one commit, so the rule changes in all places at once, or in none.
* A new team leader takes over from Progress.

## Progress

* [x] 2026-10-31: plan written (team leader).
* [x] 2026-10-31: plan review round 1 (one `Plan` agent): no blocker, 11 should-fix (7 significant) and 7 nits.  All are fixed in this plan, but the order of `review clean` (nit 17, Decision log).
* [x] 2026-10-31: plan review round 2 (a new `Plan` agent): no blocker, 7 should-fix (6 significant) and 5 nits.  All are fixed in this plan.  The review stops at its cap of 2 rounds.
* [x] 2026-10-31: step 1: `review_log.rs`, `strict_rules` and `fix_rule_problems`, with their tests (implementer, `9f22a7e`).
* [x] 2026-10-31: step 2: `docs-check` runs the check; the Conventions bullet and the Tools row (implementer, `7c7b2c6`).
* [x] 2026-10-31: step 3: `xtask review log-check <file>` (implementer, `7846ccb`).
* [x] 2026-10-31: step 4, the implementer's part: the diff `m3/m3-skill.diff` in the team leader's scratchpad, checked on `7846ccb` (implementer).  The team leader checks it again on `HEAD`.
* [x] 2026-10-31: checkpoint: the orchestrator lands the diff before round 1, with the 0011 hunk (`c1e02e3`).
* [x] 2026-11-01: review round 1 fixes (implementer): 1-1, 1-2, 1-3, 1-4, 1-6, 1-8, 1-10 to 1-13, 1-15, 1-18, 1-19 (TD-69) and 1-20 to 1-23.  The diff `m3/m3-r1-skill.diff` holds 1-5, 1-7, 1-9, 1-14, 1-16 and 1-17.
* [x] 2026-11-01: review round 2 fixes (implementer): 2-1 to 2-4, 2-7, 2-8, 2-12 to 2-14 and 2-16.  The diff `m3/m3-r2-skill.diff` holds 2-5, 2-6, 2-9 to 2-11 and 2-15.
* [x] 2026-11-01: review round 3 fixes (implementer): 3-2 (the guide line), 3-5, 3-7 and 3-9.  The diff `m3/m3-r3-skill.diff` holds 3-1, 3-2 (the skill line), 3-3, 3-4, 3-6 and 3-8.
* [x] 2026-11-01: review round 4 fixes (4-1), as the late rule allows (implementer).  The diff `m3/m3-r4-skill.diff` holds the skill and guide lines.  4-2 to 4-5 are `reported, not fixed`.
* [ ] A `KNOWN_MISSED` entry for TD-69 (finding 1-19) in M3's last commit.
* [ ] Step 5: review rounds to the stop rule.
* [ ] M3 done: the log in `docs/reviews/harness-phase-3b-m3.md`, and this plan in `plans/completed/`.

## Surprises and discoveries

* 2026-10-31, team leader: the strict list of decision 0024 has 15 IDs, not 16 as the first draft said (plan review round 1).
* 2026-10-31, implementer, step 1: each new test failed on its stub, on an assert:
  * `the_strict_rules_are_those_of_decision_0024`: "strict_rules() gives 0 IDs, not 15".
  * `the_strict_rules_are_the_15_ids_in_order`: `left: []`.
  * `a_good_log_gives_no_problem`: the assert of `has_significant_column` failed (its stub gives `false`).
  * The other 9 tests of `review_log.rs`: `left: ["stub"]`, against the expected problems or `[]`.
* 2026-10-31, implementer, step 2: each new test failed on its stub, on an assert:
  * `only_a_review_log_is_checked_against_the_fix_rule`: two `stub` problems, one for each file, not the one problem of the log.
  * `a_review_log_that_follows_the_fix_rule_gives_no_problem`: one `stub` problem, not `[]`.
* 2026-10-31, implementer, step 3: each new test failed on its stub, on an assert.  The stub gives `(0, [])`:
  * `log_check_passes_a_log_that_follows_the_fix_rule`: `(0, [])`, not the line `log-check: the log follows the fix rule`.
  * `log_check_fails_a_fixed_nit_in_round_4` and `log_check_fails_a_log_with_no_significant_column`: `(0, [])`, not exit 3 and the problem.
  * `log_check_fails_a_missing_file`: exit 0, not 3.
* 2026-10-31, implementer, step 4: the diff changes the skill, `CLAUDE.md`, `docs/working-with-agents.md`, decision 0011, `docs/tech-debt.md` and `xtask/tests/rules.rs`.  The checks of the diff, at `7846ccb`, in a scratch worktree with its own target folder:
  * `git apply --check` passes at the repo root.
  * With only the `xtask/tests/rules.rs` part, `cargo test -p xtask --test rules` fails 4 tests, each on an assert.  The other 23 pass.
  * `the_review_skill_starts_with_xtask`: 34 problems, one for each new pin of `REVIEW_SKILL_TEXT`.  No old pin fails.
  * `the_fix_rule_says_the_same_in_each_place`: 4 problems: `CLAUDE.md`, the skill, `docs/working-with-agents.md` and decision 0011.
  * `the_review_skill_names_the_strict_rules`: the skill does not hold `STRICT_RULES_TEXT`.
  * `the_review_skill_log_passes_the_fix_rule_check`: the template has no `Significant` column.
  * With the whole diff, `cargo fmt --all --check`, `cargo test -p xtask` and `cargo run -p xtask -- docs-check` pass.  `CLAUDE.md` stays at 88 lines.
* 2026-11-01, implementer, review round 1 fixes: each new or changed test fails on the code at `9de7009` or on the named mutant, on an assert:
  * `a_round_must_fix_each_finding_that_the_rule_names`: at `9de7009`, `MEM-1/MEM-2`, `RT-1's rule`, `PROTO-1/PROTO-2` and `MEM-1,MEM-5` give no problem (1-2).  With `round >= 3`, 3-1 gives no problem (1-1).  With a split on whitespace and no trim, 4-7 (`VERIFY-1; D9 a quote`) gives no problem (1-3).  With the trim, 4-3 to 4-6 give no problem (1-2).
  * `a_good_log_gives_no_problem`: with `round >= 3`, 3-1 and 3-2 give problem 11 (1-1).  With a split on whitespace alone, 4-20 gives problem 11 (1-3).
  * `a_round_fixes_a_pre_existing_finding_that_the_rule_names_or_gives_the_reason`: at `9de7009`, no problem (1-15).
  * `a_refuted_finding_has_no_fix`: at `9de7009`, 4-1 also gives problem 11 (1-13).
  * `an_action_names_a_fix_or_a_reason_for_no_fix`: each of the four mutants of `is_sha` makes one of `own PR (defaced)`, `own PR #41` and `see round 4` a fix (1-8, 1-10, 1-21, 1-23).
  * `a_known_row_names_its_td_in_the_action_and_in_repeat_of`: with `is_td` as `true`, or with its `&&` as `||`, `known: TD-x` or `known: none` gives no problem (1-11, 1-20, 1-22).
  * `a_row_with_bad_cells_gives_only_those_problems`: with `r >= 0`, the round `0` gives no problem (1-1).
  * `review_log_rs_routes_all_six`: without the `ROUTE_ALL` entry, the route is `["reviewer"]` (1-6).
  * `the_review_skill_names_the_strict_rules` and `the_strict_rules_are_those_of_decision_0024`: with `, LAYER-1, DOC-1` after `ASSET-1` in the skill or in decision 0024, each fails on its line (1-4).  The old tests passed this change.
  * `a_good_log_gives_no_problem` row 2-3 (1-12) passes at `9de7009` too.  It keeps the `no change:` case of 1-15.
* 2026-11-01, implementer: the checks of `m3/m3-r1-skill.diff`, at `363bb83`, in a scratch worktree with its own target folder:
  * `git apply --check` passes at the repo root.
  * With only the `xtask/tests/rules.rs` part, `the_review_skill_starts_with_xtask` fails on its assert, with 6 problems, one for each new pin.  The other 26 rules tests pass.
  * With the whole diff, `cargo fmt --all --check`, `cargo test -p xtask` and `cargo run -p xtask -- docs-check` pass.  `CLAUDE.md` stays at 88 lines.
* 2026-11-01, implementer, review round 2 fixes: each new or changed test fails on the code at `0e1037d` or on the named mutant, on an assert:
  * `a_round_fixes_a_pre_existing_finding_that_the_rule_names_or_gives_the_reason`: at `0e1037d`, the round 2 `pre-existing` nit 2-2 gives no problem (2-1).
  * `a_refuted_finding_has_no_fix`: with `let answered = verdict == "refuted";`, the `known` row 4-2 also gives problem 11 (2-2).
  * `a_round_must_fix_each_finding_that_the_rule_names`: with `{ td: false, no_change: false }`, 2-2 and 4-8 (`no change: later`) give no problem (2-4).
  * `an_action_names_a_fix_or_a_reason_for_no_fix`: without the `all(...)` clause, `reported, not fixed; see round 4` gives problem 10, not problem 8, since 1-6 is a round 1 `pre-existing` row (2-7).  Without the merge case of that clause, `merged into 1-1; reported, not fixed` gives problem 8.  At `0e1037d`, `no change needed` gives no problem (2-13).
  * `a_row_with_bad_cells_gives_only_those_problems`: with `row {n}` in place of `row {n + 1}`, the problem starts `row 4:` (2-14).
  * `a_good_log_gives_no_problem`: when only the first part of the action is read for `known: `, 4-23 (`merged into 4-1; known: TD-39`) gives problem 7 (2-14).
  * `start_warns_when_the_branch_changes_review_log_rs_or_the_binary_is_older`: without the `review_log.rs` entry of `WATCHED_SOURCES`, it gets no warning, as at `0e1037d` (2-8).  It and `the_log_check_source_warns_on_its_own` fail when either warning names `xtask/src/review.rs` or `the route` in place of the source.
* 2026-11-01, implementer: the checks of `m3/m3-r2-skill.diff`, at `4b3af3a`, in a scratch worktree with its own target folder:
  * `git apply --check` passes at the repo root.
  * With only the `xtask/tests/rules.rs` part, two tests fail on their asserts.  `the_review_skill_starts_with_xtask` gives 6 problems, one for each new or changed pin.  `each_reviewer_reads_the_late_scope` gives one problem for each of the six reviewers.  The other 26 rules tests pass.
  * With the whole diff, `cargo fmt --all --check`, `cargo test -p xtask` and `cargo run -p xtask -- docs-check` pass.  The agent tests, such as `every_reviewer_has_an_agent_file` and `the_agents_are_read_only_by_tools`, pass.  `CLAUDE.md` stays at 88 lines.
* 2026-11-01, implementer, review round 3 fixes: each new or changed test fails on the code at `00035e1`, on an assert (3-7):
  * `a_changed_review_rs_warns`: a base with no blob and a head with one, or the other way round, give no warning, not the `adds` or `removes` line.
  * `start_warns_when_the_branch_adds_review_log_rs`: a branch that adds `xtask/src/review_log.rs` gets no warning.
* 2026-11-01, implementer: the checks of `m3/m3-r3-skill.diff`, at `64e6f4c`, in a scratch worktree with its own target folder:
  * `git apply --check` passes at the repo root.
  * With only the `xtask/tests/rules.rs` part, two tests fail on their asserts.  `the_review_skill_starts_with_xtask` gives 2 problems, one for each new or changed pin.  `each_reviewer_reads_the_late_scope` gives one problem for each of the six reviewers.  The other 26 rules tests pass.
  * With the whole diff, `cargo fmt --all --check`, `cargo test -p xtask` and `cargo run -p xtask -- docs-check` pass.  `CLAUDE.md` stays at 88 lines.

## Decision log

* 2026-10-31, team leader: an action is a fix when a part of it, between `;`, holds a commit SHA or the word `diff`, and does not start with a not-fixed form.  Reason: the Action cells of the logs are free text.  A prefix alone passes `TD-39; abc1234 review round 4 fixes` (plan review round 1).  A SHA anywhere fails `no change: ... (9cf2fd0)` (plan review round 2).
* 2026-10-31, team leader: a TD ID passes problem 10 on a finding that the round must fix.  Reason: a team can send a confirmed mutant to tech debt, as for TD-39 and TD-66.  The PR lists it, and the stop rule still counts it.
* 2026-10-31, team leader: a row with `cargo-mutants` in its Reviewers is on VERIFY-1, and the skill's merge step names VERIFY-1 too.  Reason: a merged row can keep the reviewer's rule `none` (1-4 of M1's log).
* 2026-10-31, team leader: the stop rule counts the `confirmed` findings that the round must fix, as decision 0024's sentence does.  It also counts a `pre-existing` or `unproven` finding of the three kinds that the round fixed, or sent to a TD (finding 4-1 of review round 4).  Reason: no later reviewer would read that fix (plan review round 2).
* 2026-10-31, team leader: a `confirmed` nit on a strict rule, such as a baseline nit with `same-as`, is on a strict rule.  So a late round must fix it.  An `unvalidated` nit is never fixed in a late round.
* 2026-10-31, team leader: decision 0011 gets only a pointer line, as a proposal in the diff.  Reason: it is accepted, and only a person changes its rule text.
* 2026-10-31, team leader: the check does not check the stop rule.  Reason: a later set of rounds, such as after an intent review, can follow a stop.
* 2026-10-31, team leader: `xtask review log-check` checks a run's log after each round.  Reason: `docs-check` sees the log only after the last round, when a wrong fix is in the history (plan review round 1).
* 2026-10-31, team leader: step 4 of the skill also reads the logs of the branch's earlier milestones in `docs/reviews/`.  Reason: each milestone starts a new run, so its own log does not hold the TDs of M1 and M2 (plan review round 1).
* 2026-10-31, team leader: the team leader runs `review clean` before the orchestrator writes the files, as the orchestrator asked.  Reason: `review clean` removes only the round's worktrees, so the order changes no count.
* 2026-10-31, implementer: a row with a bad value (problems 2 to 5) gives only those problems, not problems 6 to 11.  Reason: problems 6 to 11 read those values, so they would add only noise.
* 2026-10-31, implementer: `review_log.rs` also has `pub fn has_significant_column`.  Reason: `log_check` (step 3) must tell a log with no column from a good log.  Its stub gives `false`.
* 2026-10-31, implementer: problem 7 looks for a part `known: <TD>` of the Action cell, not the whole cell.  The TD must equal Repeat of.
* 2026-10-31, implementer: step 10 of the skill also says that a late round fixes a `pre-existing` or `unproven` finding only of the three kinds.  Reason: the log check (problem 11) and the stop rule use this rule.
* 2026-10-31, implementer: step 9 of the skill gives decision 0024's test of significance.  Reason: the skill then needs no other file to fill the field.
* 2026-10-31, implementer: each step ticks the step before it with its SHA, since a commit cannot hold its own SHA.  The last commit ticks steps 3 and 4.
* 2026-10-31, orchestrator (recorded by the implementer): the orchestrator accepted the 0011 hunk in `c1e02e3`, and added "by the agent" to its line.
* 2026-11-01, implementer: problem 10 also applies to a `pre-existing` row that the round must handle (finding 1-15).  A fix, a TD ID or `no change:` passes it.  Reason: step 10 of the skill lets such a finding go to tech debt or to its own PR, not to `reported, not fixed`.  In rounds 1 to 3, this includes a `pre-existing` nit (finding 2-1 of review round 2).  Reason: step 10 fixes each found finding in rounds 1 to 3, and a baseline nit can get `pre-existing` by `same-as`.
* 2026-11-01, implementer: a Rule cell splits at each character that is not a letter, a digit or `-` (finding 1-2).  Reason: real cells hold `MEM-1/MEM-2`, `PROTO-1;` and `RT-1's`.
* 2026-11-01, implementer: problem 11 skips a `refuted` or `known` row (finding 1-13).  Reason: problem 9 already names its fix, and the two fixes contradict.

## Outcomes and retrospective

To be written when M3 completes.
