# Harness phase 3, M4: the findings log

## Purpose

This is the milestone plan for M4 of `plans/active/harness-phase-3.md`.  It gives more detail than the parent plan's M4 section, which stays the criteria.  After M4:

* `docs-check` accepts review logs in the reviews folder under `docs`.  One index line, `reviews/`, covers each file there.  A log keeps the paths and closed IDs of its time, as a plan does.
* `no_python_in_the_build` does not look in the reviews folder for the name of Python's web server, as for plans and decisions.
* The `review-rounds` skill counts its own late `xtask review` commands, each of which asks a person for approval in the default permission mode, apart from the interventions (D4's count, Decision log 2026-10-23).  The guide's recipe and TD-36 say the same.

M5 adds the first file to the reviews folder (`replay.md`), and M6's rounds add the branch log.  M4 adds no file there.  No code that ships changes.

## Context and orientation

The state at the start of M4, on 2026-10-23, at commit `c55ddf5` on branch `ai-agent-harness-phase-3`:

* `xtask/src/docs.rs`:
  * `docs_indexed` fails each doc under `docs/` (but `docs/README.md`) whose path, from the root or from `docs/`, is not a code span of `docs/README.md`.  Its test is `every_doc_must_be_in_the_index`.
  * `names_current_paths(file)` is false for `docs/plans/` and `docs/decisions/`, so `paths_exist` skips their paths.  Its test is `paths_resolve_from_the_root_the_file_or_docs`.
  * `may_name_closed_ids(file)` is `!names_current_paths(file)`.  Its test is `closed_ids_are_named_only_in_plans_and_decisions`.
  * `repo_path` reads `reviews/` as prose, not a path (one segment, no `.md`), so the index line passes `paths_exist` while the folder is empty.  `docs/reviews/` would be read as a path and fail; no doc names it before M5.
* `xtask/src/rules.rs`: `const HISTORY: [&str; 2] = ["docs/plans/", "docs/decisions/"];`, read by `python_in_the_build`.  Its test `a_python_server_is_named` has cases for the history folders.
* `.claude/skills/review-rounds/SKILL.md`:
  * Step 9's findings form has `* interventions: <the number of messages and approvals of a person since review start>`.
  * Step 11's rounds table has the columns `Round | Agents | Tokens | Wall minutes | Interventions | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline found`, and the bullet "Interventions: the number of messages and approvals of a person in the round."
  * "Before the steps" says that `review check` of step 8 asks for approval once in each round, and that a late `review mutants` can ask too.
* `xtask/tests/rules.rs`: `REVIEW_SKILL_TEXT` pins phrases of the skill; `review_skill_problems` fails each one that is missing.
* `docs/working-with-agents.md`, recipe "Review a change": "Step 11 of the skill counts each approval as an intervention (TD-36)."
* `docs/tech-debt.md`, TD-36: "each review round has an approval, which the log counts as an intervention".  Its fix line asks the orchestrator or the maintainer to decide the count.  The orchestrator decided it on 2026-10-23 (parent plan, Decision log).
* `docs/README.md`: line 3 says `docs-check` fails a doc under `docs/` that the list lacks.  "Conventions" says that only plans and decisions are exempt from the path check and may name a closed ID.
* The highest tech-debt ID is TD-37.  M4 adds one entry (step 3).

Terms are those of the parent plan.  "Late review command": a `target/debug/xtask review` command that the skill runs after the turn in which it loaded.  `allowed-tools` has ended by then, so in the default permission mode each one asks a person for approval (TD-36).  In auto mode the classifier decides, and in `dontAsk` mode (the harness) the allow list decides; neither asks a person.  The agent sees which turn ran a command, but not whether a person approved it.

## Plan of work

### How the team works

The team leader writes this plan, runs the reviews, and commits Progress.  One implementer (a general-purpose subagent) does steps 1 to 3 and the review fixes.  The implementer never runs `/review-rounds` on this branch: the parent plan reserves the skill's rounds for M6, and a log in the run folder of this branch would make M6's round 1 refuse to start.  No harness case runs again (parent plan, M3 freeze bullet).

### Step 1: the tests, first

Write these tests, run them, and record in Progress that each fails for the stated reason:

* `xtask/src/docs.rs`:
  * `a_review_log_is_indexed_by_its_folder`: with the index span `reviews/`, `docs/reviews/x.md` gives no problem.
  * `a_review_log_is_not_indexed_without_its_folder`: without that span, `docs/reviews/x.md` gives one `index` problem.  Also, the span `plans/` does not cover `docs/plans/active/x.md` (only the reviews folder is indexed by folder).
  * `a_review_log_names_the_paths_and_ids_of_its_time`: `names_current_paths("docs/reviews/x.md")` is false, and a closed ID in `docs/reviews/x.md` gives no problem from `ids_defined` (or `id_references`, as the existing closed-ID test does).
* `xtask/src/rules.rs`: `the_python_server_may_be_named_in_a_review_log`: the server's name in `docs/reviews/x.md` gives no problem, as in a plan.
* `xtask/tests/rules.rs`: add to `REVIEW_SKILL_TEXT` the phrases `* late review commands:` and `| Interventions | Late review commands |`, and update its doc comment.

The first, third and fourth tests fail before step 2; the second passes before and after (it guards the change); the skill test fails before step 3.

### Step 2: the code

* `docs.rs`: a constant, for example `INDEXED_BY_FOLDER: &[&str] = &["reviews/"]`.  `docs_indexed` counts a doc `docs/<folder>...` as listed when the index has the span `<folder>`.  `names_current_paths` is also false for `docs/reviews/`; `may_name_closed_ids` follows.  Update the doc comments of the three functions, and the closed-ID message in `ids_defined` ("only plans and decisions name closed IDs"), to name review logs too.  The test `closed_ids_are_named_only_in_plans_and_decisions` may get a new name.
* `rules.rs`: `HISTORY` becomes `[&str; 3]` with `"docs/reviews/"`.  The doc comment of `python_in_the_build` ("outside plans and decisions") names review logs too.

### Step 3: the skill and the docs (DOC-1)

* The skill, step 9: the field `interventions` counts the messages of a person since `review start`, and the approvals of a person that the agent can see or that the person reports, but not an approval of a late review command.  Step 11's Interventions bullet uses the same words.  A new field after it: `* late review commands: <the number of target/debug/xtask review commands that the skill ran or tried to run after the turn in which it loaded>`.  The field counts commands, which the agent sees, not approvals, which it does not; it is the count of approvals only in the default permission mode.
* The skill, step 11: a column `Late review commands` after `Interventions`, in the header and the separator line.  The Interventions bullet says that an approval of a late review command is not an intervention.  A new bullet: "Late review commands: the field of the findings file.  In the default permission mode, each one asked a person for approval (TD-36)."  The baseline cells do not change: the baseline runs no `xtask review` command.
* No other text of the skill changes, "Before the steps" included (the parent plan's M3 freeze bullet: "M4 changes the text of steps 9 and 11 only").  The agent files do not change.
* The guide, recipe "Review a change": step 11 counts the late review commands in their own cell, not as interventions (TD-36).  Keep the `fa41c9f` recipe untouched (it is history in git only).  Add a changelog line at the end of the guide, as M1 to M3 did: "2026-10-24, phase 3 (M4): ..." on the reviews folder that `docs-check` accepts, and the late review commands counted apart from interventions.
* TD-36: new title "in the default permission mode, each review round has an approval that the skill forces".  The body says that the log counts the late review commands apart from the interventions (M4), so D4's gate does not fail by construction, and that this count equals the approvals only in the default permission mode.  The fix line: a project allow rule for `Bash(target/debug/xtask review *)` removes the approval; it widens a permission, so the maintainer decides.  TD-36 stays open.
* A new tech-debt entry, with the next free ID, named only in the commit that adds it, for no rule (the measure of D4): the log cannot count a person's approvals.  In the default permission mode, the reviewers' and validators' commands, and the main agent's own writes of the findings file and the log, can ask a person for approval, and the main agent does not see those approvals.  Round 1 runs more agents than the baseline, so D4's "no more interventions" can pass when it should fail.  Fix: a count from Claude Code's own record of approvals, if one becomes available, or the person's own count.
* `docs/README.md`: a line `* \`reviews/\`: ...` under "How work is done" (the log of each PR branch's review rounds, which step 11 of the skill writes; M5 adds its replay results to the line).  Line 3 and "Conventions" say that one line indexes the whole reviews folder, and that review logs, like plans and decisions, are exempt from the path check and may name a closed ID.  There, write "the reviews folder" in prose.  Never write `docs/reviews/` before M5: `paths_exist` reads it as a path, and the folder is empty.
* `docs/invariants.md`, BUILD-1's "Enforced by" line ("outside plans and decisions"): name review logs too.
* `docs/tech-debt.md`, line 3 ("Plans and decisions may still name a closed ID"): name review logs too.
* `docs/harness.md` has no text about the review log or interventions; it does not change.

Then run the checks of "Validation and acceptance", and commit, signed: "Harness phase 3, M4: the findings log".

### Step 4: review rounds 1, 2 and 3

The team leader runs three rounds with the manual recipe of `docs/working-with-agents.md` at `fa41c9f`, with new general-purpose reviewers, M4's commits as the range, and the parent plan's M4 section as the criteria.  The prompt also names this plan's Decision log, which names the parent's "approvals" field "late review commands".  It checks each finding.  The implementer fixes the real ones, each with a test, in a commit "Harness phase 3, M4: review round <n> fixes".  Progress records each round before the next one starts.  A real finding that needs skill text outside steps 9 and 11, or an agent file, is not fixed in M4: it goes to the orchestrator, for M6 (M3 freeze bullet).

## Validation and acceptance

* Step 1's tests fail before steps 2 and 3, as recorded in Progress, and pass after; `a_review_log_is_not_indexed_without_its_folder` passes before and after.
* `cargo fmt --all --check`; each clippy command in `CLAUDE.md`; `cargo test --workspace --exclude roformer-gpu`; `cargo test -p roformer-gpu --lib`; `cargo run -p xtask -- docs-check` all pass (GIT-2).
* `rtk proxy grep -ci 'late review commands' .claude/skills/review-rounds/SKILL.md` gives 3 or more.
* `rtk proxy grep -rn 'docs/reviews/' docs CLAUDE.md .claude` finds nothing outside `docs/plans/`.
* M6 tests the skill's step 11 in its rounds (parent plan).

## Idempotence and recovery

* Each step is one signed commit or part of one; to undo, revert it.
* To resume: read Progress, then `git log --oneline c55ddf5..HEAD`.
* No step writes outside the tree.  No step runs the skill or the test harness.

## Progress

* [x] 2026-10-23: plan written, at `c55ddf5`.
* [x] 2026-10-23: plan review round 1: 0 blockers, 3 significant should-fix (three "plans and decisions" texts left wrong; an edit outside steps 9 and 11; a count of approvals the agent cannot see), 2 nits.  All five fixed in the plan.
* [x] 2026-10-23: plan review round 2: 0 blockers, 2 significant should-fix (a field "skill approvals" would record approvals that auto mode and `dontAsk` mode never ask; tech-debt line 3 left wrong), 3 nits.  All fixed: the field is now "late review commands".
* [x] 2026-10-23: plan review round 3: 0 blockers, 1 significant should-fix (the interventions field counts approvals that the agent cannot see; reviewers' approvals undercount, so D4 can pass when it should fail), 3 nits.  Fixed: a new tech-debt entry, and the field counts only approvals the agent sees or the person reports.  The parent-plan wording nit goes to the orchestrator.
* [x] 2026-10-24: plan review round 4: 0 blockers, 1 significant should-fix (no guide changelog line), 6 nits.  Fixed: the changelog line, the wider debt entry, TD-36's title, the test exception, the Progress steps, and a Surprises line on IDs in a log.  The permission mode is not recorded in the log; the step 11 bullet says the count equals the approvals only in the default mode.
* [x] 2026-10-24: plan review round 5: 0 blockers, 0 significant should-fix, 6 nits; the stop rule is met.  Four nits fixed (folder-index wording, closed IDs, freeze path for review fixes, the debt entry's rule); the README's replay wording waits for M5; the permission mode stays out of the log.
* [x] 2026-10-24: step 1, the tests, failing first.  Before step 2, `cargo test -p xtask --lib` and `--test rules` gave:
  * `a_review_log_is_indexed_by_its_folder`: fails; one `index` problem, "does not list `reviews/x.md`".
  * `a_review_log_is_not_indexed_without_its_folder`: passes, as the plan says (before and after).
  * `a_review_log_names_the_paths_and_ids_of_its_time`: fails at `assertion failed: !names_current_paths("docs/reviews/x.md")`.
  * `the_python_server_may_be_named_in_a_review_log`: fails; one BUILD-1 problem, "docs/reviews/x.md:2 names Python's web server".
  * `the_review_skill_starts_with_xtask`: fails; 2 problems, the text does not hold `* late review commands:` or `| Interventions | Late review commands |`.
* [x] 2026-10-24: step 2, the code.  `INDEXED_BY_FOLDER` in `docs.rs`; `names_current_paths` is false for the reviews folder; `HISTORY` in `rules.rs` has 3 folders.  The closed-ID test is now `closed_ids_are_named_only_in_plans_decisions_and_review_logs`.  All `xtask` lib tests pass.
* [x] 2026-10-24: step 3, the skill (steps 9 and 11 only), the guide, TD-36, the new TD-38, the docs index, BUILD-1 and tech-debt line 3.  All checks pass; the M4 commit.
* [ ] Step 4: review rounds 1, 2 and 3.
  * [x] 2026-10-24: review round 1: 0 blockers, 1 should-fix (no test held `docs_indexed` to the folder prefix; a mutant passed), 1 nit (separator line not pinned).  Both fixed with tests in the round 1 fix commit.  With the mutant `.any(|folder| listed.contains(*folder))`, `a_review_log_is_not_indexed_without_its_folder` now fails ("a plan is not in it: []"); the mutant was then reverted.
  * [x] 2026-10-24: review round 2: 0 blockers, 1 should-fix (the round 1 separator pin was a substring of the findings-table separator, so it could not fail), 2 nits (step 11 bullet not pinned; guide approval line not limited to the default mode).  All fixed in the round 2 fix commit; the 11-cell mutant now fails.  `the_review_skill_starts_with_xtask` fails with 1 problem on that mutant: the text does not hold the rounds header with its separator.

## Surprises and discoveries

* 2026-10-24, plan review round 4: a review log is checked for undefined IDs like any doc (`ids_defined` exempts only closed IDs).  So M6's step 11 commit fails `docs-check` if a claim cell quotes an ID that no entry defines; M6 then rewords that cell.  Not changed in M4.

## Decision log

* 2026-10-23, team leader: only the reviews folder is indexed by its folder; plans and decisions keep one index line each.  A general folder rule would let `plans/` hide a plan from the index.
* 2026-10-23, team leader: the skill test pins the new field and column, so a later edit cannot drop D4's count without a failure.
* 2026-10-23, team leader (plan review rounds 1 and 2): the skill counts its late review commands, named for what it counts, since the agent cannot see an approval, and auto mode and `dontAsk` mode ask no person.  Not taken: a field "skill approvals", which would record approvals that never happened in M5's and M6's runs.

## Outcomes and retrospective

To be written when M4 completes.
