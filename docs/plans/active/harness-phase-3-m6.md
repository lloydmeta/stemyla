# Harness phase 3, M6: the commits before round 1

## Purpose

This is the milestone plan for the first bullet of M6 of `plans/active/harness-phase-3.md`, "Before round 1, in their own commits".  The parent plan's M6 section stays the criteria.  This plan does not cover the three `/review-rounds` rounds or the close-out.  The orchestrator does those after this plan is done.

After this plan:

* Decision record 0011 records the design of phase 3 and the maintainer's choices D1 to D6, with M5's result and the change to D4 of 2026-10-24.
* The guide says that phase 3 is in place.  Its recipe "Review a change" tells a person how to count approvals.
* Step 11 of the `review-rounds` skill asks the person for the approvals of the round, and of the baseline apart, and logs them as cost.  An intervention is only a message of a person, not the message that gives those counts (D4, Decision log 2026-10-24).  The skill test pins the new text.
* The fix lines of ten tech-debt entries name the agent that looks.  Each entry stays open.
* The phase 3 text of `plans/active/harness-engineering.md` names `xtask review`, the six replay cases (D1) and the three rounds (D6), and points its Gate line to D4.

No code that ships changes.

## Context and orientation

The state at the start, on 2026-10-24, at commit `71641ee` on branch `ai-agent-harness-phase-3` (CI green, tree clean):

* `docs/decisions/` holds 0001 to 0010.  `docs-check` fails a gap in the numbers, and a record with no `Status:` line.  Each record has a line in `docs/README.md`.  The form: `# 00nn: title`, `Status: ...`, then `## Context`, `## Options` (or the choices), `## Decision`, `## Consequences` (see `docs/decisions/0010-git-1-deny-rules.md`).
* The parent plan's Decisions section holds D1 to D6, which the maintainer chose on 2026-10-22.  Its Decision log line of 2026-10-24 ("D4's interventions are a person's messages ...") is the orchestrator's change to D4: approvals are cost, not interventions.  That line says: "This narrows the maintainer's D4; the summary lists it."
* M5's result (`docs/reviews/replay.md`, "Total and bar"): 8/26 = 0.31 found, 95% Wilson interval 0.17 to 0.50.  The bar (0.8, and each recorded should-fix or worse found) is not met.  By D5, M6 goes on, and the miss leads the summary.
* TD-44: the skill's Tokens cells add each agent's final context, not the tokens that it used, so D4's token ratio has no reliable measure in the log.
* `.claude/skills/review-rounds/SKILL.md`:
  * Line 26, in "Before the steps": "So `review check` of step 8 asks for approval once in each round."  It does not name the default permission mode (M4, Outcomes).
  * Step 9, line 213: `* interventions: <the number of messages of a person since review start, and the approvals of a person that you see or that the person reports, but not an approval of a late review command>`.  Line 214: `* late review commands: ...`.
  * Step 11, line 278: the rounds table header with 12 cells (`... | Interventions | Late review commands | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline found |`), and a separator line of 12 cells.
  * Step 11, line 293: "Interventions: the number of messages of a person in the round, and the approvals of a person that you see or that the person reports.  An approval of a late review command is not an intervention."
  * Step 11, line 296: "Their interventions are those of a person for those agents."
* `xtask/tests/rules.rs`, `REVIEW_SKILL_TEXT` (line 995): pins phrases of the skill, among them `"but not an approval of a late review command>"` and `"An approval of a late review command is not an intervention."`, and the 12-cell header, separator and the end of its block as one `concat!` entry.  `the_review_skill_starts_with_xtask` fails each missing phrase, one problem each.
* `docs/working-with-agents.md`:
  * Status, line 9: "Phases 1 and 2 are in place:".  Line 18: "Phase 3 adds eight read-only review agents ...".
  * Recipe "Review a change", line 87: "... Step 11 of the skill counts these late review commands in their own cell, not as interventions (TD-36). ..."
  * The changelog ends with the M4 line of 2026-10-24.
  * `rtk proxy grep -c 'Run three review rounds.  Use a new subagent' docs/working-with-agents.md` gives 0 already (M3 replaced the manual recipe).
* `docs/tech-debt.md`:
  * TD-36 says that the log counts the late review commands apart from the interventions, "So D4 ... does not fail by construction", and "The baseline review needs no approval".
  * TD-38 says that the interventions of step 11 count the approvals that the agent sees or that the person reports, so D4's "no more interventions" can pass when it must fail.
  * The fix lines to change: TD-3 ("Fix: review; no check is planned."), TD-7 (`cargo deny`), TD-8 (its third bullet: "Fix: phase 3's reviewers look for it."), TD-10 ("a GPU reviewer that requires a named real GPU in the PR"), TD-16 (decode failure; a version in `Alive`), TD-23 ("Fix: phase 3's reviewers look for all four."), TD-26 ("Phase 3's memory reviewer looks for a new copy of any form."), TD-27 ("Phase 3's GPU reviewer looks for GPU memory that is not destroyed."), TD-28 (its last "They do not see" bullet ends "only review sees it"), TD-29 ("Phase 3's reviewers check for it.").
* The agents that look, from the "What you check" section of each agent file:
  * `reviewer`: TD-8 (early return without `test_support::skip`), TD-29 (snapshot PNG, threshold, `#[ignore]`), TD-3 (DSP-1 forms).
  * `gpu-reviewer`: GPU-3 (TD-10), TD-27.
  * `memory-reviewer`: TD-26.
  * `wire-reviewer`: TD-16.
  * `supply-chain-reviewer`: TD-7 (`cargo info`), TD-23, GIT-1 and a change to `.claude/` (TD-28).
* `docs/invariants.md`: no `* Enforced by:` line names an agent in a code span.  VERIFY-1's line says "CI ... and review", and DEPS-1's line says "a reviewer does".  These name review in general, not an agent.
* `docs/plans/active/harness-engineering.md`, phase 3:
  * "A routing table in `xtask` (`xtask review-plan`) prints which reviewers a diff needs".
  * "Acceptance: replaying the reviews of PRs #9 to #12 finds the bugs their rounds found; ...".
  * "Measured by: confirmed findings per round, false-positive rate per reviewer, rounds per PR, vacuous tests found before merge."
  * Its Decision log has no line on D4.  This plan does not edit that plan's Progress, Surprises or Decision log (the orchestrator's instruction).  The team leader reports the missing line to the orchestrator.
* `target/review/run/ai-agent-harness-phase-3/` does not exist.  It must not exist at the end: `review start` refuses round 1 when the run folder holds a log.

Terms are those of the parent plan.  "Approval counts": the two numbers that the person gives at step 11: the approvals of the round, and the approvals of the baseline.

## Plan of work

### How the team works

The team leader writes this plan, runs the plan reviews and the review rounds, and commits Progress.  One implementer, a general-purpose subagent, does steps 1 to 4 and the review fixes.  One subagent works on the tree at a time.  No agent runs `/review-rounds`, `xtask review start` or the test harness: the orchestrator runs the skill's rounds after this plan, and its run folder must not exist yet.  Before each commit, run the checks of "Validation and acceptance", first list.  Each commit is signed (`git commit -S`), and its message ends with the attribution lines that the orchestrator gave.

### Step 1: decision record 0011

Write `docs/decisions/0011-review-agents.md`, and add its line to `docs/README.md` after the line of 0010.  The line: "* `decisions/0011-review-agents.md`: review agents chosen by `xtask review` from the diff, a validator for each serious finding, cargo-mutants in the skill, and a log of each branch's rounds (harness phase 3)."  Commit: "Harness phase 3, M6: decision 0011".

* Title: `# 0011: review agents, routed by xtask review and run by a skill`.
* `Status: accepted (2026-10-22, by the maintainer)`.
* Context: before phase 3, the agent wrote each reviewer's prompt by hand (`plans/active/harness-engineering.md`, failure 5), and one general reviewer read every diff.  Phase 2's reviewers found a test that could pass on wrong code in 14 of 18 PRs.  Phase 3 (`plans/active/harness-phase-3.md`) is one PR.  The maintainer chose D1 to D6 on 2026-10-22, before the work started.
* Options: what was not taken for each part, from the parent plan.  These are a table in the PR description (D2), a CI job for cargo-mutants (D3), and every specialist in every round (D6).  Also PRs #9 to #12 rebuilt from transcripts (D1), a flat 5 times or no ceiling (D4), and tuning on the measured cases (D5).
* The Decision section says that the maintainer's acceptance covers D1 to D6 and the plan that the maintainer started, not each later choice of an agent.
* Decision, one bullet for each part:
  * Routing and round guards in `xtask review`: `start` refuses a tree with changes, routes six reviewers by paths and content signs, and records the state; `check` stops a round on a change to that state; `mutants` runs cargo-mutants under a 60-minute limit.
  * A `validator` for each blocker and should-fix, and for the missed mutants.  Nits are logged unvalidated.
  * Read-only agents: tools `Read` and `Bash`, and the guard `.claude/hooks/read-only-bash.sh` as a `PreToolUse` hook with `onFailure: block`.  No `isolation`, since a worktree branches from the default branch.
  * The log (D2): the reviews folder under `docs`, one file for each branch, written after round 3.
  * cargo-mutants (D3): in the skill only, not in CI.
  * The rounds (D6): three rounds, as `CLAUDE.md` says.
  * The replay set (D1) and its miss (D5), with M5's result as a fact: 8 of 26 found (0.31; 95% interval 0.17 to 0.50), below the bar of 0.8.  By D5 the result is recorded in `reviews/replay.md`, and the gate decides after the merge.
  * The gate's effort (D4): the ceiling on tokens, wall time and interventions.  On 2026-10-24 the orchestrator narrowed D4: an approval of a tool call is cost, logged apart, not an intervention.  This narrowing is not part of the maintainer's acceptance of 2026-10-22, and the summary to the maintainer lists it.  The token figure of the log is each agent's final context, not the tokens that it used, so D4's token ratio has no reliable measure in the log (TD-44).  The maintainer decides how to measure it at the gate.
* Consequences:
  * A reviewer is a sensor, never a control.  No `* Enforced by:` line in `invariants.md` names an agent.  The tech-debt entries that review covers name the agent that looks.
  * Known gaps: TD-31 to TD-38, TD-44, TD-46 and TD-47.  Name each by ID only, with a few words.
  * Phase 5 passes the agents with `--agents`, since frontmatter hooks do not run in `-p` mode or in an untrusted folder, and routes with the base branch's `xtask` (TD-31).
  * For IAM: a security reviewer is an extra sensor, and its hit rate is measured like any test (the parent plan of phases).
* Every ID that the record names must be defined in `invariants.md` or `tech-debt.md`.  Check with `docs-check`.

### Step 2: the skill, its test, the guide, TD-36 and TD-38

One commit: "Harness phase 3, M6: approvals as cost in the review log".

1. The test first.  In `REVIEW_SKILL_TEXT`:
   * Remove `"but not an approval of a late review command>"` and `"An approval of a late review command is not an intervention."`.
   * Add these 10 entries, each a phrase of the new skill text in item 2.  Write each as a Rust string; the backticks stay.
     1. `` So in the default permission mode, `review check` of step 8 asks for approval once in each round. ``
     2. `but not the message that gives the approval counts of step 11`
     3. `* approvals:`
     4. `* baseline approvals:`
     5. `Do not ask yourself.`
     6. `Then write each count as 0 with the mode`
     7. `If you asked, wait for the answer.  Do not guess the counts.`
     8. `but not the message that gives the approval counts.  An approval is not an intervention.`
     9. `* Approvals: the count that the person gave for the round, as cost.`
     10. `Baseline approvals: the count that the person gave for the baseline`
   * Replace the `concat!` entry with the 14-cell header, the 14-cell separator and the end of the block (item 2).  The entries are now 11 new ones.
   * Update the doc comment of `REVIEW_SKILL_TEXT`: the approvals are cost in their own fields and cells, asked for at step 11, and an intervention is a message of a person.
   * Run `cargo test -p xtask --test rules the_review_skill_starts_with_xtask` before the skill changes.  It must fail with 11 problems, one for each new entry.  Record the count and the lines in Progress.
2. The skill, four places only.  The agent files and the other steps do not change.
   * "Before the steps", line 26: replace "So `review check` of step 8 asks for approval once in each round." with "So in the default permission mode, `review check` of step 8 asks for approval once in each round."
   * Step 9: replace line 213 with the first line below.  Line 214 (`* late review commands:`) stays.  Add the two approvals lines after line 214:
     * `* interventions: <the number of messages of a person since review start, but not the message that gives the approval counts of step 11.  An approval is not an intervention>`
     * `* approvals: <not asked.  Step 11 writes the count that the person gives for the round>`
     * `* baseline approvals: <not asked.  Step 11 writes the count that the person gives for the baseline.  Without --baseline, write "-">`
   * Step 11: before "The log `<run>/log.md` has two tables.", add exactly this text:

     ````markdown
     First, get the approval counts of the round.  They are cost, not interventions.

     * If you are the orchestrator of an agent-run phase, you are "the person" (see "Before the steps").  Do not ask yourself.  The round can run in auto mode or `dontAsk` mode with no person present.  Then write each count as 0 with the mode, for example `0 (auto mode)`.  Without `--baseline`, keep "-" in the `baseline approvals` field.  Auto mode can fall back to a prompt, but with no person present that prompt stops the session.  Else ask the maintainer the questions below.
     * If you are not that orchestrator, ask the person:
       1. "How many tool calls did you approve for the round's agents and for me, from `review start` until the findings file?  Do not count the `baseline` agent and its validators."
       2. With `--baseline`: "How many tool calls did you approve for the `baseline` agent and its validators in that time?"
       3. Also say: "Give one total if you cannot split them.  In a mode that asks for no approval, each count is 0."
     * If you asked, wait for the answer.  Do not guess the counts.
     * Write the first count in the `approvals` field of `<run>/$1/findings.md`, and the second in its `baseline approvals` field, in place of "not asked".  With one total, the `approvals` field holds the total, which then includes the baseline's, and the `baseline approvals` field holds "in approvals".
     ````
   * Step 11, the rounds table: the header becomes `| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |`, and the separator has 14 cells.
   * Step 11, the bullets:
     * Interventions: "Interventions: the number of messages of a person in the round, but not the message that gives the approval counts.  An approval is not an intervention."
     * After the "Late review commands" bullet, a new bullet: "* Approvals: the count that the person gave for the round, as cost.  It is the field of the findings file."
     * The baseline bullet: "Their interventions are those of a person for those agents." becomes "Their interventions are the messages of a person for those agents.  Baseline approvals: the count that the person gave for the baseline, or "in approvals" if the person gave one total."
   * Run the test again: it passes.  Then save a copy of the changed skill in the scratchpad.  For each new entry, change its phrase in the skill, run the test, and see it fail with that one problem.  Then copy the saved skill back.  For the header entry, also try a 13-cell separator.  Do not use `git stash`, which changes the stash that `review check` records.  At the end, `git diff` shows only the planned change.  Record the result in Progress.
3. The guide, recipe "Review a change", line 87: replace "Step 11 of the skill counts these late review commands in their own cell, not as interventions (TD-36)." with "Step 11 of the skill counts these late review commands in their own cell (TD-36)."  After that bullet, add:
   > * In the default permission mode, count the tool calls that you approve in each round.  Count from `review start` until the agent writes the findings file.  In round 1 with `--baseline`, count those for the `baseline` agent and its validators apart.  Give one total if you cannot split them.  Step 11 asks for the counts and waits: give them then (TD-38).  The log records approvals as cost.  An intervention is a message of a person in the round, but not the message with the counts.
4. TD-36: replace the last sentence of its first paragraph, "The baseline review needs no approval.", with "The baseline review runs no `xtask review` command."  Its second paragraph becomes: "Since M4, the log counts these late review commands in their own field and cell.  Since M6, an approval is not an intervention.  Step 11 asks the person for the approvals of each round, and the log records them as cost (TD-38).  So the approvals that the skill forces do not fail D4 of the parent plan (`plans/active/harness-phase-3.md`, Decisions).  The count of late review commands equals their approvals only in the default permission mode."  The title and the fix line do not change.
5. TD-38: the title does not change.  The body becomes: "This entry concerns no rule.  It concerns the measure of D4 of the parent plan (`plans/active/harness-phase-3.md`, Decisions).  In the default permission mode, a command of a reviewer, a validator or the baseline can ask a person for approval.  The main agent's writes of the findings file and the log can ask too.  The main agent does not see those approvals.  So step 11 of the `review-rounds` skill asks the person for the approvals of the round and of the baseline.  The log records them as cost, not as interventions.  The counts rest on the person.  A person can miscount, or cannot tell the baseline's approvals from the round's, and then gives one total.  The maintainer reads the counts at the gate."  The fix line: "count the approvals from Claude Code's own record of approvals, if one becomes available.  Until then, the person gives the counts."
6. The guide, Status: "Phases 1 and 2 are in place:" becomes "Phases 1, 2 and 3 are in place:".  Add a last bullet to that list: "* Review agents, routed by `xtask review` and run by the project skill `review-rounds`, with a log of each branch's rounds in the reviews folder (decision 0011)."  Line 18 "Phase 3 adds" becomes "Phase 3 added".
7. The guide, changelog, a last line: "2026-10-24, phase 3 (M6): phase 3 is in place (decision 0011).  Step 11 of `review-rounds` asks for the approvals of the round, and of the baseline apart, and logs them as cost, in their own cells.  An intervention is a message of a person.  In the default permission mode, count your approvals (TD-38).  The tech-debt entries that review covers name the agent that looks."

Items 6 and 7 go in this commit, since step 1's record exists by then.

### Step 3: the tech-debt fix lines

One commit: "Harness phase 3, M6: tech debt names the agent that looks".  Each entry stays open, and keeps its title and body.  Only the fix text changes:

* TD-3: "Fix: review, by the `reviewer` agent, which looks for DSP-1 forms that the scan does not read.  No check is planned.  ..." (keep the reason that follows).
* TD-7: keep the `cargo deny` fix.  Add: "Until then, the `supply-chain-reviewer` agent checks the licence and the registry of each new crate with `cargo info`."
* TD-8, third bullet: "Fix: the `reviewer` agent looks for it."
* TD-10: "Fix: log `isFallbackAdapter` and the adapter name in selftest and e2e results.  Until then, the `gpu-reviewer` agent checks GPU-3 on each diff that routes it.  It reads the diff and the criteria, not the PR description."
* TD-16: keep the fix.  Add: "Until then, the `wire-reviewer` agent looks for a field name or a default that one side changes and the other side does not, on each diff that routes it (TD-31)."
* TD-23: "Fix: the `supply-chain-reviewer` agent looks for all four." (keep the later checks that follow).
* TD-26: "Fix: none planned.  The `memory-reviewer` agent looks for a new copy of any form, on each diff that routes it (TD-31)."
* TD-27: "Fix: none planned.  The `gpu-reviewer` agent looks for GPU memory that is not destroyed, on each diff that routes it (TD-31)."
* TD-28: the last "They do not see" bullet ends "and only review, by the `supply-chain-reviewer` agent, sees it."
* TD-29: "Fix: review; a changed PNG or threshold is a UI change, and the `reviewer` agent looks at it.  No check is planned."
* Do not edit `docs/invariants.md`.  Check: `` rtk proxy grep -n '^\* Enforced by:' docs/invariants.md | rtk proxy grep -c -E '`(reviewer|[a-z-]+-reviewer|validator|baseline)`' `` gives 0.

### Step 4: the plan of phases

One commit: "Harness phase 3, M6: the plan of phases".  In `docs/plans/active/harness-engineering.md`, phase 3 only, and not its Progress, Surprises or Decision log:

* "(`xtask review-plan`)" becomes "(`xtask review`)".
* Acceptance: "replaying the reviews of PRs #9 to #12 finds the bugs their rounds found" becomes "replaying six old review rounds finds the bugs those rounds found: PR #9, and phase 2's M2, M9, M10, M11 and M18a (D1 of `plans/active/harness-phase-3.md`, which gives why PRs #10 to #12 are left out)".  The rest of the line stays.
* Measured by: add a last sentence: "Rounds per PR stay three until a later phase changes the rule (D6 of `plans/active/harness-phase-3.md`)."
* Gate: after "at lower effort", add "(D4 of `plans/active/harness-phase-3.md` bounds the effort, and counts approvals as cost)".  This is phase 3 text, not the Decision log.

### Step 5: review rounds 1, 2 and 3

The team leader runs at most three rounds of review of `71641ee..HEAD`, each with a new `reviewer` subagent (read only, with the guard), and a new `validator` subagent for each blocker and should-fix.  The criteria: the parent plan's M6 section, first bullet, and this plan.  The prompt names the range and the criteria only.  The implementer fixes each real finding, with a test where the finding is about behaviour, in a commit "Harness phase 3, M6: review round <n> fixes".  Progress records each round before the next one starts.  Stop at the first round with no blocker and no significant should-fix.

## Validation and acceptance

Before each commit:

* `cargo run -q -p xtask -- docs-check`
* `cargo fmt --all --check`
* `cargo clippy -q --workspace --all-targets -- -D warnings`
* `cargo test --workspace --exclude roformer-gpu`
* `cargo test -p roformer-gpu --lib`

At the end:

* Step 2's test failed before the skill change, with 11 problems, as recorded in Progress.  It passes after.  Each new entry fails it when its phrase goes.
* `cargo run -q -p xtask -- docs-check` passes.
* `rtk proxy grep -c 'Run three review rounds.  Use a new subagent' docs/working-with-agents.md` gives 0.
* `` rtk proxy grep -n '^\* Enforced by:' docs/invariants.md | rtk proxy grep -c -E '`(reviewer|[a-z-]+-reviewer|validator|baseline)`' `` gives 0.
* `rtk proxy grep -c 'review-plan' docs/plans/active/harness-engineering.md` gives 0.
* `ls target/review/run/ai-agent-harness-phase-3/` fails: the folder does not exist.
* The rest of M6's acceptance (the log's rows, the PR text) comes from the orchestrator's rounds.

## Idempotence and recovery

* Each step is one signed commit; to undo, revert it.
* To resume: read Progress, then `git log --oneline 71641ee..HEAD`.
* No step writes outside the tree, runs the skill, or makes the run folder.

## Progress

* [x] 2026-10-24: plan written, at `71641ee`.
* [x] 2026-10-24: plan review round 1: 1 blocker (the invariants check matched DEPS-1's "a reviewer does"), 3 significant should-fix (TD-36's sentence, step 9's line order, step 11 would wait in the orchestrator's own rounds), 1 should-fix (decision 0011's citation and Options), 4 nits.  All fixed in the plan.
* [x] 2026-10-24: plan review round 2: 0 blockers, 1 significant should-fix (step 11's question counted the baseline in the round's count), 2 should-fix (the count of new test entries; where the skill text ends), 3 nits.  All fixed: step 11 asks two counts, its exact text is in one fenced block with the orchestrator's case first, and 11 entries are pinned.
* [x] 2026-10-24: plan review round 3: 0 blockers, 1 significant should-fix (step 11's "do not ask" contradicted its "ask the maintainer"), 2 should-fix (auto mode can fall back to a prompt; the Gate line of the plan of phases still says "at lower effort"), 2 nits.  All fixed in the plan.
* [x] 2026-10-24: plan review round 4: 0 blockers, 0 significant should-fix, 1 should-fix (decision 0011 left out TD-46 and TD-47), 6 nits; the stop rule is met.  All fixed in the plan: step 11's wait has a condition and its 0 case names where the mode goes; the Purpose bullet names the four changes to the plan of phases; the index line of 0011 is given.
* [x] 2026-10-24: the plan and its line in `docs/README.md` committed.
* [x] 2026-10-24: step 1: decision 0011, with its line in `docs/README.md`.  `docs-check` passes; it reads each TD ID of the record as defined.
* [x] 2026-10-24: step 2: the skill, its test, the guide, TD-36 and TD-38.  Failing first, with only `xtask/tests/rules.rs` changed: `the_review_skill_starts_with_xtask` panicked with "11 review skill problem(s)", one for each new entry.  For example: "the text does not hold `Do not ask yourself.`", and "the text does not hold `* approvals:`".  After the skill change, it passes.  12 mutants of the changed skill, each from a saved copy: a change to the phrase of each of the 11 new entries, and a 13-cell separator.  Each failed the test with "1 review skill problem(s)", which named that entry.  The skill was then copied back, and `git diff` shows only the planned change.
* [x] 2026-10-24: step 3: the fix lines of TD-3, TD-7, TD-8, TD-10, TD-16, TD-23, TD-26, TD-27, TD-28 and TD-29 name the agent that looks.  Each entry stays open, with its title and body.  `invariants.md` does not change, and the check of its `* Enforced by:` lines for an agent gives 0.
* [ ] Step 4: the plan of phases.
* [ ] Step 5: review rounds.

## Surprises and discoveries

* 2026-10-24: the M6 section asks that the plan of phases' Decision log record D4's change of the gate.  It has no such line; only `plans/active/harness-phase-3.md` has it (Decision log, 2026-10-24).  The orchestrator told the team not to edit that Decision log, so the team leader reports it.

## Decision log

* 2026-10-24, team leader: the late review commands field and cell stay.  The agent sees those commands, and the person's approval count covers all approvals.  The two measures differ, and the M6 section does not remove the field.
* 2026-10-24, team leader: the person counts approvals from `review start` until the findings file, the period of the round's wall minutes.  The approvals of step 10's fixes are not part of the comparison with the baseline.
* 2026-10-24, team leader: the approvals fields are "not asked" until step 11, so a `--review-only` run leaves them so.
* 2026-10-24, team leader (plan review round 1): in an agent-run phase the orchestrator is "the person" (the skill, line 27), so an orchestrator that runs the skill does not ask.  In a mode that asks a person for no approval, it writes 0 and names the mode.  M6's rounds run so, with no person present.
* 2026-10-24, team leader (plan review round 2): step 11 asks two counts, the round's without the baseline's, so the Approvals cell leaves out the baseline as the Agents cell does.  With one total, the Approvals cell includes the baseline, and the Baseline approvals cell says so.
* 2026-10-24, team leader (plan review round 3): step 4 also adds a pointer to D4 on the Gate line of the plan of phases.  It is phase 3 text, which the orchestrator did not forbid, and the line was false after D4.  The missing Decision log line is still reported.  The orchestrator gets this fourth change in the report.

## Outcomes and retrospective

To be written when the plan completes.
