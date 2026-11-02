# Harness phase 3b, M6: orchestrated mode as a skill (decision 0023)

## Purpose

This is the milestone plan for M6 of `plans/active/harness-phase-3b.md`.  It gives more detail than the parent plan's M6 section, which stays the criteria.  M6 puts decision 0023 into the harness.

After M6:

* The project skill `orchestrate` holds the generic recipe of orchestrated mode.  It describes the loop as it ran in M1 to M5 of this phase, not as the older plans described it.  A plan's "How the work runs" section holds only what is particular to that plan.
* `CLAUDE.md` and the skill's description give the same sizing rule.  So a new session that gets a big task proposes the mode in one question.
* `docs-check` fails an active plan of more than 3 milestones with no "How the work runs" section.  It also fails a copied review log whose last round does not meet the stop rule.
* `review log-check` checks a row that repeats a `reported, not fixed` row.  The `review-rounds` skill matches such a repeat from round 4, so the finding gets no validator and no new judgement.
* `review clean <round>` removes each worktree and folder under `target/review/` whose name starts with `<round>-`.

Readers: the implementer of M6, the review agents of M6's rounds, and the orchestrator.

## Context and orientation

The state at the start of M6, on 2026-11-01, at commit `7470b8d` on branch `harness-phase-3b`:

* `.claude/skills/` holds one skill, `review-rounds` (389 lines).  The parts that M6 changes:
  * "Before the steps", near line 29: a subagent whose file the Write tool refuses stops after step 9.  In M1 to M5, the team leader ran each round on to step 11, and gave the files to the orchestrator (the M4 and M5 plans, step 5 and step 6).
  * Step 3, near line 83: each reviewer is a new subagent, never a fork.  No line says that an Agent call passes no `model` or `effort`.
  * Step 4, item 4, near line 114: `repeat-of` matches a `refuted` finding of an earlier round, or a row whose Action is a TD ID or `known: <TD>`.  It does not match a `reported, not fixed` row.
  * Step 6, item 6, near line 161: the same check for the missed mutants.  Item 8 merges a reviewer's finding and a mutant's finding.
  * Step 11, near line 372: `review clean` "removes each worktree `target/review/$1-<agent>-<n>`".  After the last round, the log copy is `<branch>.md`.  The logs of this phase are `<branch>-m<n>.md`, and step 4, item 4 already reads both names.
* `xtask/src/review.rs`: `of_round` (near line 1840) matches `<round>-<agent>-` only for an agent of `round_agents()`.  `clean` (near line 1853) removes the registered worktrees and the other folders under `target/review/` that `of_round` matches.  The test `review_clean_removes_the_worktrees_of_one_round_only` (`xtask/tests/review.rs`, near line 126) keeps `1-stranger-1`.
* Reviewers made worktrees outside that pattern: `1-correctness-1` and `3-gpu-1` in M2, and `5-gpu-1`, `5-memory-1`, `5-reviewer-1`, `5-harness-1`, `6-reviewer-2` and `6-harness-1` in M3 (the parent's Surprises).
* `xtask/src/review_log.rs`: `fix_rule_problems` checks each row of a log's findings table against the fix rule.  It reads the columns `Round`, `Finding`, `Reviewers`, `Severity`, `Significant`, `Rule`, `Verdict`, `Repeat of` and `Action`.  It does not compare two rows.  It does not read the Rounds table.  `review log-check <file>` runs it (`log_check`, `xtask/src/review.rs`, near line 1920).
* `xtask/src/docs.rs`: `check` runs each check of `docs-check`.  `plans_well_formed` (near line 1201) checks the folder and the living sections of each plan.  `review_logs_follow_the_fix_rule` (near line 1233) runs `fix_rule_problems` on each file in `docs/reviews/`.  Three logs have the `Significant` column: `harness-phase-3b-m3.md` (last round 6), `-m4.md` (7) and `-m5.md` (3).
* `docs/plans/active/` holds `harness-engineering.md` (phases, no `M<n>` heading) and `harness-phase-3b.md` (`### M1` to `### M8`, and `## How the work runs`).  Four completed plans have more than 3 `M<n>` headings and no such section.
* `docs/plans/PLANS.md` lists the sections that every plan has.  It has no optional section.
* `CLAUDE.md` has 91 lines; `MAP_LINE_BUDGET` is 100.  Its "Plans:" line has no sizing rule.
* `docs/working-with-agents.md`: `guide_covers` fails if the guide does not name each skill in a code span.  So the guide must name `orchestrate` in the commit that adds the skill.
* `xtask/tests/rules.rs`: `REVIEW_SKILL_TEXT` and `review_skill_problems` pin the review skill.  No test pins an orchestrate skill.
* `docs/tech-debt.md`, TD-37: "No doc tells the agent what to do for a branch with more than one cycle of implementation and review."  The orchestrate skill tells it, so TD-37's text changes with the skill (DOC-1).
* Claude Code is 2.1.309.  Its settings docs ([settings](https://code.claude.com/docs/en/settings)) say: `--model` sets the `model` key and `--effort` sets `effortLevel` and `modelSettings` for one session.  `ANTHROPIC_MODEL` overrides `model`.  The keys can be in `~/.claude/settings.json`, `.claude/settings.json` and `.claude/settings.local.json`.
* `target/review/run/` is empty, so M6's rounds start a new run with base `7470b8d`.
* No new crate is needed (DEPS-1).

Terms:

* Reported row: a row of a run's log whose Action is `reported, not fixed`.
* Reported repeat: a finding from round 4 that is the same finding as a reported row, and that step 4, item 4 matches to it.
* Severity order: `blocker` is higher than `should-fix`, which is higher than `nit`.
* A row is on a strict rule as `Row::is_strict` reads it: a token of the Rule cell is an ID of `strict_rules()`, or the Reviewers name `cargo-mutants`.  2c uses the whole of `is_strict`.  R3 of 2d compares the Rule cell tokens and the `cargo-mutants` source apart.
* Milestone heading: a heading line (one to six `#` and a space) whose text starts with `M`, one or more digits, and then a `:`, a space or the end of the line.
* Checkpoint: a report of the team leader to the orchestrator that gives files and diffs, and then waits for the orchestrator's message.

## Plan of work

### How the team works

* The team leader writes this plan.  A new `Plan` agent reviews it, at most 2 rounds, and the team leader fixes what it finds.  The team leader commits the plan and its index line, signed.
* The team leader starts one implementer (`general-purpose`) with this plan's path.  The implementer does step 2.  The same implementer, or a new one with this plan's path, makes the fixes of the review rounds.
* The team leader writes the texts of step 3 itself.  It gives them to the orchestrator as files and unified diffs in the orchestrator's scratchpad folder `m6/`.
* The team leader and the implementer do not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.
* Before each commit: `cargo run -p xtask -- check --tier commit` (GIT-2).  Before each review round: `cargo run -p xtask -- check --tier push` and `cargo build -p xtask`.  Each commit is signed (`git commit -S`).  Docs and commit messages are in ASD-STE100.
* A new test fails first on an assert, not on a compile error.  So the implementer first writes each new function as a stub that gives a wrong answer.  It runs the new tests on the stubs, and records the failures in Surprises.  No commit holds a failing test.
* Each new test holds at least one fail case with its pass cases.  So a stub that gives no problem fails each test.  A test of pass cases only would pass on that stub, so there is none.
* No Agent call passes `model` or `effort` (decision 0025, the parent's M6 section).

### Step 1: this plan

The team leader writes this plan and its line in `docs/README.md`.  A new `Plan` agent reviews it against the parent's M6 section, the parent's Surprises and Decision log, and decisions 0023 and 0024.  At most 2 rounds.  The team leader fixes each real finding, then commits.

### Step 2: the code and docs outside `.claude/` (implementer)

#### 2a: `review clean` matches each name of the round

* `of_round(name, round)` is true when `name` starts with `<round>-`.  It does not read the agent names, so `round_agents` goes.
* `10-reviewer-1` is not of round 1, since `1-` is not its start.  `run/` and `target/` start with no digit, so they stay.
* The doc comments of `of_round` and `clean` say the new rule.
* Test: `review_clean_removes_the_worktrees_of_one_round_only` also plants the names of M2 and M3 in round 1 form: `1-correctness-1`, `1-gpu-1`, `1-memory-1`, `1-harness-1` and `1-reviewer-2`.  The old `1-stranger-1` moves to the removed list.  The names that the test already plants (`1-reviewer-1`, `1-docs-reviewer-1`, `1-validator-1-3-1`, `1-baseline-1`) stay in the planted set and in the removed list.  Each is removed.  `10-reviewer-1`, `2-reviewer-1`, `run/main/1`, `target/debug` and `.claude/worktrees/1-reviewer-1` stay.  The stub is the old `of_round`, so the test fails on its list of removed names.
* `docs/working-with-agents.md`: item 7 of the recipe "Review a change" and the tools table row of `review clean` say "each worktree or folder under `target/review/` whose name starts with `<round>-`".

#### 2b: an active plan of more than 3 milestones has "How the work runs"

* A new pure function in `xtask/src/docs.rs`, `plan_runs_problems(file, text) -> Vec<Problem>`, called from `plans_well_formed` for each file in `docs/plans/active/`.
* It counts the distinct numbers of the milestone headings.  With more than 3, it wants a heading line `## How the work runs` or `### How the work runs`, which can have more text after it, such as ` (M3 to M10)`.
* Message: "has <k> milestones (`M<n>` headings) and no `How the work runs` section.  Fix: add it at level 2 or 3, as docs/plans/PLANS.md says (decision 0023)."
* It does not read `docs/plans/completed/`, `docs/plans/PLANS.md` or a file outside `docs/plans/`.
* Tests, each with a fail case, on a stub that gives no problem:
  * an active plan with `### M1` to `### M4` and no section gives one problem;
  * the same plan with `## How the work runs` passes, and with `### How the work runs (M3 to M10)` passes;
  * the section at level 4 (`#### How the work runs`) gives the problem;
  * a completed plan with 4 milestones and no section passes.  It is in the same test as the active plan that fails, and both go through `plans_well_formed`;
  * an active plan with `### M1` to `### M3` passes, and so does a plan that names `M4` only in a bullet or in the text of a heading that does not start with it (`### Step 4: M4's text`);
  * the same number twice (`### M2: a` and `### M2: b`) counts once.
* `docs/plans/PLANS.md` gets a section `## Optional sections` after `## Sections`:
  * "How the work runs": how a big task runs in orchestrated mode (decision 0023).  The `orchestrate` skill holds the generic recipe.  The section holds only what is particular to the plan.
  * Its sub-headings, in this order, one level below the section: Mode; Branch and PR; Maintainer steps; Preflight; Decision points; Review rule; Kick-off prompt; Extra stop conditions.  The text gives what each holds, in one line.
  * "A big task's plan must have it.  A task is big by any of the four signals of decision 0023.  `docs-check` checks only one signal: it fails a plan in `active/` of more than 3 milestones without the section."
  * The text names the skill as `orchestrate`, in a code span.  Reason: `guide_covers` and the other docs read a skill by its name.
* `docs/README.md`, "## Conventions": the plans line says that an active plan of more than 3 milestones has "How the work runs".
* The working-with-agents.md tools table row of `docs-check` names the two new checks of 2b and 2c.

#### 2c: a copied log's last round meets the stop rule

* A new pure function in `xtask/src/docs.rs`, `last_round_problems(text) -> Vec<String>`.  `review_logs_follow_the_fix_rule` runs it on each file in `docs/reviews/` that has the `Significant` column, after `fix_rule_problems`.  `review log-check` does not run it, since a run's log is not at its last round after each round.
* `xtask/src/review_log.rs` makes the parse of a table and of a row public in the crate (`pub(crate)`), so that both checks use one parse.  `fix_rule_problems` does not change for 2c.
* The last round is the Round cell of the last row of the first table under `## Rounds`.  No such row, or a cell that is not a whole number, gives a problem.
* The highest Round of the Findings table is not above the last round.  Else: "the Findings table has rows of round <f>, but the Rounds table ends at round <r>.  Fix: add the row of each round to the Rounds table."
* The last round is 3 or more.  Else: "the last round is <r>, but the rounds stop at round 3 at the earliest.  Fix: run the next round (decision 0024)."
* Each Findings row of that round meets the stop rule of step 11 of the `review-rounds` skill:
  * a row with the verdict `confirmed` that is a blocker, has `yes`, or names a strict rule gives a problem, whatever its action;
  * a row with the verdict `pre-existing` or `unproven` of those three kinds gives a problem when its action is a fix or a TD ID;
  * a `known`, `refuted` or `unvalidated` row gives no problem, and nor does a `no change:` action.
* Message: "<ID>: round <r> is the last round of the log, but this finding keeps the rounds open.  Fix: run the next round (step 11 of the review-rounds skill)."
* A round with no Findings rows meets the rule.
* Tests, each with a fail case, on a stub that gives no problem:
  * last round 2, with no rows, gives one problem;
  * last round 4 with a confirmed nit, a confirmed should-fix with `no`, and a `known` mutant passes;
  * last round 4 with each of: a confirmed blocker; a confirmed should-fix with `yes`; a confirmed should-fix on MEM-1 with `no`; a `pre-existing` should-fix with `yes` and a fix commit; an `unproven` should-fix with `yes` and a TD ID.  Each gives one problem with its ID;
  * a `pre-existing` should-fix with `yes` and `no change: its own PR` in the last round passes;
  * a row of round 3 that keeps the rounds open, in a log whose last round is 4, passes;
  * a log with no Rounds row gives one problem;
  * a log whose Rounds table ends at round 4, with a Findings row of round 5, gives one problem;
  * a log with no `Significant` column gives no problem.
* The fixture `fix_rule_log` in `mod tests` of `xtask/src/docs.rs` has no Rounds table.  So the tests `only_a_review_log_is_checked_against_the_fix_rule` and `a_review_log_that_follows_the_fix_rule_gives_no_problem` would get the new problems.  The fixture gets a Rounds table that ends at round 4, and its action keeps the test's meaning.
* The implementer runs `docs-check` on the three logs of this phase, and records in Surprises that each passes.

#### 2d: `review log-check` checks a reported repeat

The skill change of step 3 gives a reported repeat the action `reported, not fixed`.  A blocker or should-fix copies the reported row's verdict and Significant value.  A nit keeps `unvalidated` and `-`, as decision 0024 says ("A nit stays `unvalidated`").  The new problems are in `fix_rule_problems`, so `review log-check` and `docs-check` both give them:

* R0, a value problem: Repeat of is `none`, a TD ID, or a finding ID (`<round>-<n>` or `<round>-b<n>`).  Else: "<ID>: Repeat of is `<x>`, not `none`, a TD or a finding ID.  Fix: write one of them."  The three logs of this phase have only `none` there.
* R1 to R4 check each row whose Repeat of is a finding ID.  If R1 fails, R2 to R4 do not run.
* A reported row is a row whose Action is exactly `reported, not fixed`.

* R1: the ID is the Finding of a row of this log in an earlier round.  Else: "<ID>: Repeat of names `<x>`, which is no row of an earlier round of this log.  Fix: name a row of this log, or write `none`.  A row of another log can have the same ID."
* R2: if the named row's verdict is `refuted`, this row's verdict is `refuted`.
* R3: if the named row's Action is `reported, not fixed`, all of these are true, and each false one gives its own problem:
  * this row's round is 4 or more;
  * the named row's Repeat of is `none`;
  * this row's severity is the same as the named row's, or lower;
  * each strict rule that this row names, the named row names too;
  * if this row's Reviewers name `cargo-mutants`, the named row's Reviewers name it too;
  * its Action is `reported, not fixed`;
  * a blocker or should-fix has the named row's Verdict and Significant;
  * a nit has the Verdict `unvalidated`.  The old check already wants `-` in its Significant.
* Each R3 message names the cause, and the fix: "match it as a new finding: write `none` in Repeat of, and validate it" (for a round, severity, rule or source), or "copy the cell of the named row" (for a verdict, Significant or action).
* R4: the named row's verdict is `refuted`, or its Action is `reported, not fixed`.  Else: "<ID>: Repeat of names `<x>`, which is neither `refuted` nor `reported, not fixed`.  Fix: match it as a new finding: write `none` in Repeat of, and validate it."  Reason: a row that names a fixed row would skip its validator.
* Tests in `mod tests` of `xtask/src/review_log.rs`, one log for each case of the parent's M6 section.  Each pass case passes `fix_rule_problems` with no problem.  Each fail case gives exactly its problem.  The stub gives no problem for each repeat:
  1. a repeat of the same severity (should-fix, confirmed, `no`) passes; a lower severity (a nit of a reported should-fix, `unvalidated`, `-`) passes, and with the copied `confirmed` it gives the verdict problem; a nit of a reported nit (`unvalidated`, `-`) passes;
  2. the named rows of these cases are `unproven` should-fix rows with `no`, so the old rule (which must fix only a `confirmed` row) gives no problem.  A blocker repeat that copies the row gives the severity problem.  A repeat that names MEM-1, of a row with rule `none`, gives the rule problem.  A repeat with `cargo-mutants`, of a row without it, gives the source problem.  The same three as new findings, with `none` in Repeat of and a fix commit, pass;
  3. a repeat of a missed mutant: a mutant row of round 5 that repeats an `unproven` mutant row of round 4 (`no`, reported) passes;
  4. a finding in rounds 1 to 3: a row of round 2 that copies a reported nit of round 1 gives R3's round problem.  A round 1 to 3 nit can be `reported, not fixed`, since rounds 1 to 3 fix a nit only if the team leader agrees.  A row of round 4 that names a row of round 4, or of a later round, gives R1;
  5. a row of round 6 whose Repeat of is `5-2`, in a log with no row `5-2`, gives R1.  This is the case of a row of an earlier milestone's log with the same ID: the skill reads reported rows only from this run's log.
  * Also: a repeat of a `refuted` row with the verdict `refuted` passes, and with `confirmed` gives R2.  A row that names a fixed `confirmed` row gives R4.  A Repeat of `5-2 (M3)` gives R0.
* `review log-check` passes on each log of the tests above that has no problem.  A test in `xtask/tests/review.rs` runs the real command on one pass log and one fail log.
* The implementer runs `review log-check` and `docs-check` on the three logs of this phase.  If an old row fails R1 or R2, the implementer records it in Surprises, and the team leader decides.  The logs in `docs/reviews/` are not edited.

#### 2e: the order of the work

Each of 2a to 2d is its own commit, with its tests and its doc lines.  Before 2d, the implementer checks the Repeat of cells of the three logs: it lists each value that is not `none` or a TD, in Surprises.

### Step 3: the `.claude/` texts, for the orchestrator (team leader)

The team leader writes these in the orchestrator's scratchpad folder `m6/`, after step 2 is committed:

* `SKILL.md`: the whole file `.claude/skills/orchestrate/SKILL.md`.
* `claude.diff`: one unified diff of `.claude/skills/review-rounds/SKILL.md`, `CLAUDE.md`, `xtask/tests/rules.rs`, `docs/working-with-agents.md` and `docs/tech-debt.md`.  The skill file is a new file, so it is not in the diff.  The orchestrator copies it, then applies the diff, in one commit.
* `README.txt`: the commands to land both, the checks that the team leader ran, and their results.

The orchestrate skill:

* Frontmatter: `name: orchestrate`, a `description` that names the four signals of decision 0023, and `argument-hint: <plan>`.  No other key.  The description holds no `: `, since a plain YAML value cannot hold it.  Reason: `disable-model-invocation` would stop the model from loading it, and `context: fork` would run it in a fork.
* Sections: when to use it (the sizing rule, the one question); roles; who edits what; the rules for every agent; preflight; the start; the loop for each milestone (the plan, the implementer, the checkpoints, the review rounds, the log copy, the close of the milestone, the intent review, the run folder); between milestones (CI, `xtask clean --yes`, the PR body); the counts; stop and wait; close-out; the report format; the brief of a team leader.
* The team agents stay `general-purpose` with briefs.  No agent file gets write tools, so `agents_folder_problems` stays as it is.
* A GPU hand run that the protocol rule asks for is not a stop.  It goes in the PR under "Needs your attention".
* The loop follows the parent's Surprises and Decision log of M1 to M5, as the orchestrator's brief of M6 lists them.  The team leader checks each item there, and lists in README.txt each item that has no line in the parent plan.  For such an item, it gives a Surprises or Decision log line for the parent.
* The stop rule: the skill points to step 11 of the `review-rounds` skill, and does not say it again.  The intent review uses the same stop rule.
* It names M7 as the milestone that adds the steps of the PR body.  Until then, the skill only says to update the body with `gh pr edit --body-file`, from a file outside the tree.

The `review-rounds` diff:

* "Before the steps": a new bullet: "Do not pass `model` or `effort` to the Agent tool.  The agent file decides them (decision 0025)."
* "Before the steps": the bullet on a refused file says what the team leaders did in M1 to M5.  A subagent writes the text of each refused file, does the fixes and the commit of step 10, and does step 11 up to the log.  It leaves the approvals fields as "not asked", as step 9 wrote them.  It runs `review log-check` on a copy of the log in a scratch folder, if it can write one.  Then it gives the whole text of each file and the commands, and stops.  With `--review-only`, it stops after step 9.
* Step 4, item 4: the match of a reported repeat, as 2d checks it.  It reads reported rows only from `<run>/log.md`.  The match runs only from round 4, and it can match a reported row of any earlier round, a reported nit of rounds 1 to 3 included.  If the matched row has an ID in its Repeat of, the finding names that ID.  A blocker or should-fix copies the row's verdict and Significant value; a nit keeps `unvalidated` and `-`.  A finding that has one of the three differences (a higher severity, a new strict rule, a new `cargo-mutants` source) is a new finding: no `repeat-of`, and steps 5 and 6 validate it.  If it matches more than one reported row, it uses the row of the latest round.
* Step 6, item 6: the same match for a missed mutant.  It matches only a reported row whose Reviewers name `cargo-mutants`.  Its old sentence "A finding whose `repeat-of` is an ID gets the verdict `refuted`" names a `refuted` finding instead.
* Step 6, item 8: when a merge adds `cargo-mutants` to a reported repeat whose row does not name it, the merged finding is a new finding.  It loses its `repeat-of` and its copied values, and takes the verdict of the mutant's validator.
* Step 10: "Do not fix a reported repeat.  Its action is `reported, not fixed`."
* Step 11, the bullet for a subagent that is not the orchestrator: it no longer stops before the log when the Write tool refused a file.  It gives the questions, and goes on to the log, as "Before the steps" says.
* Step 11: `review clean` removes each worktree or folder under `target/review/` whose name starts with `$1-`.  After the last round, in a run of the `orchestrate` skill, the copy is `<branch>-m<n>.md` for milestone `<n>`.

`CLAUDE.md`, the "Plans:" line: one more sentence, so the file keeps 91 lines.  "A big task runs in orchestrated mode, with the `orchestrate` skill (decision 0023)."  Then the four signals, and "Propose the mode in one question, then run hands-off."

`docs/working-with-agents.md`: the step "A larger task" of "How a task flows" names the mode.  The line "A branch has one set of rounds and one log" of the recipe "Review a change" says that the orchestrate skill gives each milestone a run.  A new recipe "Run a big task hands-off" gives the kick-off prompt, the one question, and what the person checks.  A changelog line for M6.

`docs/tech-debt.md`, TD-37: the orchestrate skill gives one run for each milestone, and a copy for each milestone, as `<branch>-m<n>.md`.  A branch outside orchestrated mode still has the gap.

`xtask/tests/rules.rs`:

* `ORCHESTRATE_SKILL_TEXT` and `orchestrate_skill_problems`, as `review_skill_problems` does for the review skill.  The test `the_orchestrate_skill_keeps_its_key_lines` pins at least: the frontmatter keys; no fork; at most 15 subagents; `git commit -S`; the stop rule's reference to step 11 of the `review-rounds` skill; `cargo run -p xtask -- clean --yes`; no `model` or `effort` in an Agent call; `rustup target list --installed` and `trunk --version`; `check --tier commit` before each commit and `check --tier push` before each review round and each push; the session's model and effort, with `--model`, `--effort`, `model` and `effortLevel`; the stop rule's two "not a stop" cases (a refused cleanup command, a hand run on a GPU); and the manual removal of the run folder after the intent review (TD-37).
* `SIZING_SIGNALS`: one short phrase for each signal of decision 0023.  `the_sizing_rule_says_the_same_in_each_place` fails if `CLAUDE.md`'s "Plans:" line or the skill's `description` line lacks one.
* `REVIEW_SKILL_TEXT` gets the new lines of the review skill: the `model` and `effort` line, the reported repeat of step 4, item 4, its line in step 10, and the new `review clean` sentence.  Each old pin that quotes a changed line changes with it, and `the_review_skill_starts_with_xtask` passes on the new skill.

The checks of step 3, in a scratch worktree of `HEAD` under `target/` (`git worktree add`), removed after:

* `git apply --check claude.diff` passes on `HEAD`.
* With only the `xtask/tests/rules.rs` part, `cargo test -p xtask --test rules` fails on an assert of each new pin.  The team leader records each failure in README.txt.
* With the skill file and the whole diff, `cargo run -p xtask -- check --tier push` passes, and `CLAUDE.md` has at most 100 lines (`MAP_LINE_BUDGET`).

### Step 4: the checkpoint before round 1

The team leader gives the orchestrator the paths of step 3, the parent plan text of step 3 (the lines for items with no source), and a short report.  Then it waits.  The orchestrator lands the skill and the diff in one signed commit.

### Step 5: the dry run (acceptance)

After the orchestrator lands step 3, the team leader runs one `claude -p` session in a scratch clone:

* Clone `HEAD` to `/private/tmp/stemyla-harness/m6-dry-<n>` (`git clone --quiet --no-hardlinks <repo> <dir>`), with `<n>` from 1.
* The input is a plan of 4 milestones for a small change, with no "How the work runs" section, given on stdin.  The plan file stays in the scratchpad folder `m6/`.
* The command, at most 10 turns and 15 minutes: `perl -e 'alarm 900; exec @ARGV' claude -p "do this plan" --max-turns 10 --permission-mode dontAsk --allowedTools Read Glob Grep Skill --output-format stream-json --verbose < <plan> > <m6>/dry-<n>.jsonl`, in the clone.  This Mac has no `timeout`, and a foreground Bash call stops at 10 minutes, so the team leader runs it in the background.  The stream file goes in the scratchpad folder `m6/`, not in the clone.
* Pass: the first assistant text proposes orchestrated mode and asks one question, before any work.  The stream has no `tool_use` of `Agent`, `Write`, `Edit` or `Bash`, and the run ends after that reply.  A tool that the allow list refuses counts as a try, and fails the pass.
* The team leader reads the stream with `jq` only.  It records the reply's first lines, the tool calls by name, and the session's model in Surprises.
* If auto mode refuses the command, the team leader records it, gives the command to the orchestrator, and does not work around it.
* The clone stays until close-out.  The orchestrator removes it with `cargo run -p xtask -- clean --yes --also <dir>`.

### Step 6: review rounds

* Run `cargo run -p xtask -- check --tier push` and `cargo build -p xtask`.  Then run `/review-rounds 7470b8d <round> "<criteria>"` from round 1, with no `--baseline`.
* The criteria are the parent plan's M6 section at the commit before round 1, and this plan.
* The rounds follow decision 0024: the fix rule of step 10 of the skill, the late range of step 3 and the stop rule of step 11.  There is no cap.
* The team leader fills the `Significant` field and column, by M3's reading: a wording gap in a doc that no agent acts on is not significant.
* The Write tool refuses `findings.md` and `log.md` to the team leader (M1 to M5).  So the team leader runs each round to step 11: the fixes, the commit, and `review log-check` on a copy of the log in `m6/`.  Then it gives the orchestrator the files, the commands and a diff for each fix of `.claude/**` or `CLAUDE.md`, and waits.  This is one checkpoint for each round.
* Each fix of behaviour gets a test that fails first.
* A mutant that the team sends to tech debt gets a `KNOWN_MISSED` entry in M6's last commit.

### Step 7: close-out

After the last round, the team leader moves this plan to `plans/completed/` with its retrospective, and fixes each line that names the old path.  It keeps the run folder for the intent review.  It gives the orchestrator a close-out file in `m6/` with the text below.

### Changes to the parent plan

The team leader gives these to the orchestrator as text:

* At step 4: a Surprises or Decision log line for each item of the orchestrator's brief that has no line in the parent plan.
* At step 7: the M6 Progress line, with the stats of the rounds; the Surprises, the dry run's reply included; Decision log lines, agent; the PR text; and the scratch folders for `xtask clean --yes --also`.

## Validation and acceptance

* `cargo test -p xtask` passes with the tests of step 2.  Each new test failed on its stub, on an assert (Surprises).
* The parent's acceptance:
  * `the_orchestrate_skill_keeps_its_key_lines` and the new pins of the review skill fail without the skill text (step 3), and pass with it.
  * A `docs-check` test fails an active plan of 4 milestones with no "How the work runs" section, and passes a completed one (2b).
  * The dry run's first reply proposes orchestrated mode (step 5, Surprises).
* `review log-check` passes on each pass log of 2d, and on the log after each round.
* `docs-check` passes on the three logs of this phase with the last-round check (2c).
* With the orchestrator's commit of step 3, `cargo run -p xtask -- check --tier push` passes on `HEAD`, and `CLAUDE.md` has at most 100 lines.

## Idempotence and recovery

* Each of 2a to 2d is its own commit, which `git revert` undoes.  The orchestrator's commit of step 3 is one more.
* `review clean` acts only on `target/review/`, and on names that start with `<round>-`.  `run/` is never one.
* The new checks read files and change none.  A rerun gives the same problems.
* The dry run writes only in its clone and its stream file in `m6/`.  A rerun uses a new `<n>`.
* A new team leader takes over from Progress.

## Progress

* [x] 2026-11-01: step 1, this plan, reviewed by a `Plan` agent (2 rounds), and committed.
  * 2026-11-01: review round 2 found no blocker, 6 should-fix (the verdict of a nit repeat, the fixtures of 2d, tests of pass cases only, the dry run's `timeout`, a repeat of a repeat, the PLANS.md rule) and 4 nits.  All fixed.  The review stops at its cap of 2 rounds.
  * 2026-11-01: review round 1 found 1 blocker (a nit cannot copy `no`), 4 should-fix (two docs.rs fixtures, the rounds 1 to 3 case, three old skill lines, a repeat of a fixed row) and 4 nits.  All fixed in this plan.
* [x] 2026-11-01: step 2, `review clean`, the plan check, the last-round check, the repeat check (2a to 2d).
  * 2026-11-01: 2d in `2ac0525`.  `review log-check` and `docs-check` check R0 to R4.  Tests: `a_reported_repeat_of_the_same_or_a_lower_severity_copies_the_row`, `a_repeat_with_a_higher_severity_a_new_strict_rule_or_a_new_source_is_a_new_finding`, `a_missed_mutant_repeats_a_reported_mutant`, `a_reported_repeat_is_from_round_4_and_of_an_earlier_round`, `a_repeat_names_a_row_of_this_log`, `a_repeat_of_a_refuted_row_is_refuted`, `a_repeat_names_a_refuted_or_a_reported_row`, `repeat_of_is_none_a_td_or_a_finding_id`, `review_log_check_checks_a_reported_repeat`.
  * 2026-11-01: 2c in `b9daf78`.  `docs-check` checks that a copied log's last round meets the stop rule.  Tests: `a_log_ends_at_round_3_or_later`, `a_last_round_with_a_finding_of_the_three_kinds_keeps_the_rounds_open`, `only_the_last_round_keeps_the_rounds_open`, `a_log_has_a_rounds_row_for_each_round_of_its_findings`, `a_log_with_no_significant_column_has_no_last_round`.
  * 2026-11-01: 2b in `eb7eb3b`.  `docs-check` fails an active plan of more than 3 milestones with no "How the work runs".  Tests: `an_active_plan_of_more_than_3_milestones_needs_how_the_work_runs`, `a_completed_plan_needs_no_how_the_work_runs`, `only_a_heading_that_starts_with_m_n_is_a_milestone`, `a_milestone_number_counts_once`.
  * 2026-11-01: 2a in `71b261a`.  `review clean` removes each name that starts with `<round>-`.  Test: `review_clean_removes_the_worktrees_of_one_round_only`.
* [x] 2026-11-01: step 3, the skill, the review skill diff, `CLAUDE.md`, the guide, TD-37 and the pins, in the orchestrator's scratchpad folder `m6/` (`SKILL.md`, `claude.diff`, `README.txt`, `parent.md`).  `git apply --check` passes on `177cf31`.  Tier `push` passes in a scratch worktree with the whole bundle; `CLAUDE.md` keeps 91 lines.
* [x] 2026-11-01: step 4, the checkpoint before round 1.  The orchestrator landed the bundle in `7fa2b7a`, with three changes of its own to the skill (Roles, "A checkpoint" item 1, "Between milestones" item 4), and the parent's lines in `03e28e6`.
* [ ] Step 5: the dry run.  Done in part, on 2026-11-01 (`/private/tmp/stemyla-harness/m6-dry-1`, a clone of `03e28e6`): the session loaded the `orchestrate` skill and proposed orchestrated mode in its only reply (7 turns, $0.40), so the parent's acceptance holds.  This plan's line "asks one question" is not verified: the allow list refused the Bash that the preflight needs before the question (review round 1, 1-6).  Next: a second run with the read-only preflight commands allowed.
  * 2026-11-01: second run (`/private/tmp/stemyla-harness/m6-dry-2`, a clone of `d376200`), with the read-only preflight commands allowed.  It proposed orchestrated mode again, and asked no question (9 turns, $0.46).  Its first Bash call was a compound read (`ls`; `wc`; `cat`), which the allow list did not match.  From that refusal it decided that all Bash was refused, and it ran no preflight check.  A third run with Bash allowed in a clone with no remote was refused by auto mode ("Create Unsafe Agents").  So "asks one question" stays not verified.  The orchestrator has the command for a person to run.
* [ ] Step 6: review rounds.
  * 2026-11-01: review round 1 of `7470b8d..9b129b1` (6 reviewers and 10 validators; 666,928 tokens; 20.6 wall minutes; 0 interventions).  13 confirmed should-fix (1-1 to 1-9, 1-14) and 4 nits.  1-2, 1-6 and 1-7 are significant.  The test and doc fixes are in "review round 1 fixes"; the skill, review-rounds and `CLAUDE.md` fixes are a diff for the orchestrator (`m6/r1-claude.diff`).  The mutants run was stopped after 2 of 146 mutants for the pause; both missed lines are fixed (1-4, 1-14).
  * Next step for a resume: the orchestrator writes `m6/r1-findings.txt` as `target/review/run/harness-phase-3b/1/findings.md` and `m6/r1-log.txt` as `target/review/run/harness-phase-3b/log.md`, runs `review log-check` and `review clean 1`, and lands `m6/r1-claude.diff`.  Then the dry run again with the read-only preflight commands allowed (step 5), tier `push`, `cargo build -p xtask`, and `/review-rounds 7470b8d 2 "docs/plans/active/harness-phase-3b.md#M6: orchestrated mode as a skill (decision 0023), and docs/plans/active/harness-phase-3b-m6.md"`.
* [ ] Step 7: close-out.

## Surprises and discoveries

* 2026-11-01, 2a: on the stub (the old `of_round`), `review_clean_removes_the_worktrees_of_one_round_only` failed on its list of removed names.  The old code removed `1-reviewer-2` too, since `reviewer` is an agent of `round_agents`.  It kept the other new names and `1-stranger-1`.
* 2026-11-01, 2b: on the stub (no problem), each new test failed on its first fail case, `left: []`, with the one `has 4 milestones` problem on the right: `an_active_plan_of_more_than_3_milestones_needs_how_the_work_runs`, `a_completed_plan_needs_no_how_the_work_runs`, `only_a_heading_that_starts_with_m_n_is_a_milestone` and `a_milestone_number_counts_once`.
* 2026-11-01, 2c: on the stub (no problem), each new test failed on an assert, with `left: []` or `left: 0`: `a_log_ends_at_round_3_or_later` (the round 2 problem), `a_last_round_with_a_finding_of_the_three_kinds_keeps_the_rounds_open` (the five IDs), `only_the_last_round_keeps_the_rounds_open` (`3-1`), `a_log_has_a_rounds_row_for_each_round_of_its_findings` (no Rounds row) and `a_log_with_no_significant_column_has_no_last_round` (its fail case, round 2).
* 2026-11-01, 2c: `cargo run -p xtask -- docs-check` passes with the last-round check ("docs match the code").  So `harness-phase-3b-m3.md`, `-m4.md` and `-m5.md` each meet it.  `review log-check` passes on each of the three.
* 2026-11-01, before 2d: each Repeat of cell of `harness-phase-3b-m3.md` (61 rows), `-m4.md` (56) and `-m5.md` (11) is `none`.  No cell is a TD or a finding ID.  So no old row is a repeat that R0 to R4 check.
* 2026-11-01, 2d: on the stubs (no R0 to R4 problem), each new test failed on its fail-case assert, `left: []`: `a_reported_repeat_of_the_same_or_a_lower_severity_copies_the_row`, `a_repeat_with_a_higher_severity_a_new_strict_rule_or_a_new_source_is_a_new_finding`, `a_missed_mutant_repeats_a_reported_mutant`, `a_reported_repeat_is_from_round_4_and_of_an_earlier_round`, `a_repeat_names_a_row_of_this_log`, `a_repeat_of_a_refuted_row_is_refuted`, `a_repeat_names_a_refuted_or_a_reported_row` and `repeat_of_is_none_a_td_or_a_finding_id`.  `review_log_check_checks_a_reported_repeat` (`xtask/tests/review.rs`) failed on the exit code of the fail log: 0, not 3.
* 2026-11-01, 2d: with R0, the old test `a_known_row_names_its_td_in_the_action_and_in_repeat_of` failed on its row `4-4`, whose Repeat of is `TD-x`.  R0 is a value problem, so the `known` problem did not come.  The row now has `TD-39` in Repeat of and `known: TD-x` in the action, so it keeps its `known` problem.  `repeat_of_is_none_a_td_or_a_finding_id` holds `TD-x` in Repeat of.
* 2026-11-01, 2d: after 2d, `review log-check` passes on `harness-phase-3b-m3.md`, `-m4.md` and `-m5.md` ("the log follows the fix rule", exit 0), and `docs-check` passes ("docs match the code").  No old row fails R0 to R4.
* 2026-11-01, step 3: with only the `xtask/tests/rules.rs` part of the diff, `cargo test -p xtask --test rules` gave 30 passed and 3 failed, each on an assert.  `the_orchestrate_skill_keeps_its_key_lines` gave 23 problems (the keys, the 4 signals, 18 lines), since the missing file reads as empty.  `the_sizing_rule_says_the_same_in_each_place` named the 4 signals and the skill.  `the_review_skill_starts_with_xtask` gave 6 problems: the 4 new lines, and the 2 new lines of step 11's subagent bullet, which replace 2 old pins.  With the whole bundle, tier `push` gave "11 passed in 139.5 s".
* 2026-11-01, step 3: `docs-check` reads a code span of the skill that ends in `.md` or `.json` as a path, so `findings.md`, `log.md`, `.claude/settings.local.json` and `plans/active/harness-phase-3b.md` failed.  The skill writes `<run>/<round>/findings.md` and `<run>/log.md`, names the local settings file in words, and names M7 with no path.
* 2026-11-01, step 3: each item of the orchestrator's brief was checked in the parent plan.  Five have no line there, or only a part of one: the checkpoint of each round and the orchestrator's own paths, the intent review from round 4 and the removal of the run folder, the CI cases, the 30 minutes of a stalled subagent, and the permission rule with the install of a missing tool.  `parent.md` gives a Decision log line for each.
* 2026-11-01, step 5: the dry run's session ran on `claude-opus-5-5[1m]`, with the plan of 4 milestones on stdin and the prompt "do this plan".  Its tool calls: Bash (1, refused by `dontAsk`: `cat`, `ls` and `grep` to read the guide and the plans), Read (1), Glob (1), Grep (1), then Skill (`orchestrate`).  No Agent, Write or Edit call.  The first and only reply starts: "I haven't started the plan.  The `orchestrate` skill has me check the repo before anything else, and this session can't run shell commands at all."  Then: "The plan has 4 milestones, so `CLAUDE.md` counts it as a big task (decision 0023).  That means the `orchestrate` skill runs it, after I've checked the setup and you've answered one question about how to run it."  It lists the preflight checks that need Bash, the way to start the run in auto mode, and the points of its one question: the missing "How the work runs" section, the plan's open choices, the branch and the draft PR.
* 2026-11-01, review round 1: each new assert of the fixes fails on its mutant: `round(row) < 5` and `< 3` (1-4); the space, the leading zero and level 1 of `plan_runs_problems` (1-5); `named && fixed` to `fixed` (1-9); `m == c || k >= n` (1-14).  Each gave 569 passed and 1 failed.  `the_orchestrate_pins_fail_on_a_changed_skill` checks 1-1 to 1-3 on the real skill.
* 2026-11-01, step 5, second run: the session ran on `claude-opus-5-5[1m]`.  Its tool calls: Bash (1, refused: `ls docs ...; wc -l app/src/lib.rs; cat docs/working-with-agents.md | head -80`), Grep (2), Glob (2), Read (3).  No Skill call, but it read `.claude/skills/orchestrate/SKILL.md`.  No Agent, Write or Edit call.  Its reply starts: "I haven't started the plan.  The orchestrate preflight fails on its first check: this session is in `dontAsk` mode and refuses every Bash command."  Then: "It has 4 milestones, and CLAUDE.md (decision 0023) counts more than 3 as a big task."  It names the effort as the one value that it cannot read, as the skill says.

## Decision log

* 2026-11-01, agent (team leader): the match of a reported repeat reads only this run's log, `<run>/log.md`.  Reason: an earlier milestone's log can hold a row with the same ID, and its code can have changed since.  So a finding in a new milestone is judged again.
* 2026-11-01, agent (team leader): a blocker or should-fix that is a reported repeat copies the verdict and the Significant value of its row.  A nit keeps `unvalidated` and `-`.  Each keeps its own severity, kind, where and claim.  Reason: the repeat gets no validator and no new judgement, `review log-check` can compare the cells, and decision 0024 keeps each nit `unvalidated`.
* 2026-11-01, agent (team leader): the review skill's bullet on a refused file follows what the team leaders did in M1 to M5: they ran on to step 11 and gave the files.  Reason: the orchestrate skill describes that loop, and the two skills must not contradict.
* 2026-11-01, agent (team leader): the guide lines that name the `orchestrate` skill land with the skill, in the orchestrator's commit.  Reason: M1's retrospective ("land the `.claude/` part of a change in the same commit as the docs that describe it").
* 2026-11-01, agent (implementer): `plan_runs_problems` reads no line in a fenced code block as a heading.  Reason: a shell comment such as `# M4 ...` in a fence is not a milestone heading in Markdown.  The test `only_a_heading_that_starts_with_m_n_is_a_milestone` holds that case.
* 2026-11-01, agent (implementer): for 2c, `review_log.rs` gives `table`, `findings`, `Row`, `Row::is_named` and `action_kind` to the crate.  `Row` now owns its cells, so `findings` can return the rows.  `fix_rule_problems` uses the same parse, and its tests do not change.
* 2026-11-01, agent (implementer): the plan gives no fix for three messages of 2d.  R3's item "the named row's Repeat of is `none`" says "Fix: write `<y>` in Repeat of", as the skill names the ID of the matched row's Repeat of.  R2 says "Fix: write `refuted`, or match it as a new finding".  The nit verdict says "Fix: write `unvalidated`", since "copy the cell" is wrong for a nit.  The test `a_reported_repeat_of_the_same_or_a_lower_severity_copies_the_row` holds the first (`6-1`).

* 2026-11-01, agent (team leader): the dry run passes the parent's acceptance: its first reply proposes orchestrated mode.  It does not pass this plan's own pass line: its one Bash call (a read, before it loaded the skill) was refused, and it asked no question, since the skill puts the preflight first.  Review round 1 (1-6) found that step 5 was marked done anyway.  So step 5 stays open, and a second run allows the read-only preflight commands.

* 2026-11-01, agent (team leader): the second dry run failed this plan's pass line, but the cause is not in the skill.  The session took one refused compound command as a refusal of all Bash, and so it ran no preflight check.  The skill text does not change.  A `dontAsk` run with an allow list cannot reach the question, and auto mode refused a run with Bash allowed.  So the evidence for "asks one question" is left to a person's run, or to the next big task (the parent's "Validation and acceptance").

## Outcomes and retrospective

To be written when the milestone completes.
