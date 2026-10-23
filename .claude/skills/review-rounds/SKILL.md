---
name: review-rounds
description: Run one review round of CLAUDE.md on this branch
argument-hint: <base> <round> "<criteria>" [--review-only] [--baseline]
allowed-tools: Bash(target/debug/xtask review *)
---
!`target/debug/xtask review start $0 $1`

The lines above are the output of `xtask review start`.  This skill runs one review round of `CLAUDE.md` on this branch.  Do the steps below in order.

## Arguments

* The base is `$0`.  The `base:` line above gives its merge base with `HEAD`.
* The round is $1.
* The criteria are: $2
* The flags are in the argument string `$ARGUMENTS`.  A flag is given only if it is a word of that string:
  * `--review-only`: stop after step 9.
  * `--baseline`: also run the `baseline` agent (steps 2, 5 and 7).

## Before the steps

* Run this skill only after fmt, clippy, the tests and `cargo build -p xtask` pass.
* A refusal of `review start` (exit 3) stops the skill before this text.  Claude Code shows its lines, such as "commit first", in the session.  A `claude -p` run in text mode shows nothing.
* A `warning:` line of `review start` does not stop the round.  Copy it to the findings file (step 9).
* `allowed-tools` lets the `xtask review` commands run with no approval only in the turn in which the skill loads.  A new turn ends that permission.  A message of a person starts a new turn.  So does the notice that a background agent or command ended.
* The notices of the background reviewers and of the mutants run start new turns.  So `review check` of step 8 asks for approval once in each round.  A late `review mutants` can also ask for approval.  Approve each one once.  Do not approve with "don't ask again".  That changes the local settings file of Claude Code, and then `review check` fails the round.
* In an agent-run phase, "the person" in this skill is the orchestrator.
* If you are a subagent, do not give your final answer before the last step that you do.  Your run ends with your final answer, and a nested agent or a background command then has no reader.
* If the Write tool refuses a file of this skill to a subagent, do not write it in another way.  Give the path and the whole text of each such file in your final answer.  Also say at which step you stopped.
* In that case, stop after step 9, also without `--review-only`.  A subagent cannot do step 11 alone.  The caller writes each file at its path.  Then the caller decides the next step: step 10, step 11 or the next round.

## Names

Take each value from the lines of `review start`.  Do not make a path from the branch name, because `review start` writes `/` in a branch name as `-`.

* `<run>`: the absolute path on the `run folder:` line.
* `<target>`: the absolute path on the `target folder:` line.
* `<base>`: the full SHA on the `base:` line.
* `<since>`: the full SHA on the `since:` line.
* The branch route: the reviewer lines below the line `branch <base>..<head>:`.
* The since route: the reviewer lines below the line `since <since>..<head>:`.  The line "no changes" means that the since range is empty.
* The reasons of a reviewer: the text after its name and the colon on its line of a route, in `<run>/$1/route.txt`.  That file holds the lines of the two routes with all their reasons.  The lines above show at most 5 paths of a reviewer, then "(+n more)".

The round's folder is `<run>/$1`.  `review start` wrote `branch.diff`, `since.diff`, `route.txt` and `state.json` there.  The mutants run writes `mutants.txt` and `mutants.out/` there.  Step 9 writes the findings file `<run>/$1/findings.md`.  The log of the branch is `<run>/log.md`.

## Steps

### 1. Start the mutants run

Run this command with the Bash tool, with `run_in_background: true` and `timeout: 3900000`:

```bash
target/debug/xtask review mutants $1
```

The tool result comes at once.  The command stops itself after 60 minutes.  When it ends, it prints one line, and writes the line to `<run>/$1/mutants.txt`.  The notice that the command ended holds the line.  Do not wait for it now.  If the command refuses to run (exit 3), it prints an error line, no `mutants:` line, and no `mutants.txt`.

### 2. Start the baseline (`--baseline` only)

Start the `baseline` agent in the same message as the reviewers of step 3.  Its range is `<base>..HEAD`.  Fill in this prompt template, and give the agent only that text:

```text
Review this commit range as your instructions say.

Range: <base>..HEAD
Criteria: $2
Round: $1
Target folder: <target>
```

### 3. Start the reviewers

The six reviewers are `reviewer`, `gpu-reviewer`, `memory-reviewer`, `wire-reviewer`, `supply-chain-reviewer` and `docs-reviewer`.  Choose the reviewers and their ranges:

* Round 1: start each reviewer of the branch route.  The range of each is `<base>..HEAD`.  Its reasons come from the branch route.
* A round above 1: start `reviewer` on `<base>..HEAD`, with its reasons from the branch route.  Also start each other reviewer of the since route on `<since>..HEAD`, with its reasons from the since route.  If the since range is empty, start `reviewer` only.

Start each reviewer, and the baseline, in one message.  Use the Agent tool, with `subagent_type` set to the name of the reviewer.  Each one is a new subagent.  Never start a fork, because a fork gets your context.

Fill in this prompt template for each reviewer, and give the reviewer only that text.  Do not add your reasoning, or anything from an earlier round:

```text
Review this commit range as your instructions say.

Range: <the range of the reviewer>
Criteria: $2
Reasons of the route: <the reasons of the reviewer>
Round: $1
Target folder: <target>
```

### 4. Collect and merge the findings

Wait for each reviewer, and for the baseline with `--baseline`.  A background agent gives its output in the notice that it ended.  A foreground agent gives its output in its tool result.  For each agent, record the total tokens and the duration of its `<usage>`.

Each finding of a reviewer is one block with the fields `severity`, `kind`, `where`, `rule`, `claim`, `evidence` and `fix`.  "No findings" gives no block.  Merge the blocks:

1. Two blocks are the same finding when they have the same `where` and the same claim.  The same `where` is the same file, at the same line or in the same function.  The same claim is the same defect, in other words.
2. Merge each set of blocks that are the same finding into one finding.  Its sources are each reviewer that reported it.  Its severity is the highest severity of the blocks: `blocker`, then `should-fix`, then `nit`.
3. Give each finding the ID `$1-<n>`, with `<n>` from 1.
4. Read `<run>/<r>/findings.md` for each round `<r>` below $1.  Find each finding there with the verdict `refuted`.  If a finding of this round is the same finding as one of them, give it `repeat-of:` and the ID of that finding.  Give it the verdict `refuted`.  It gets no validator.
5. Keep the baseline's findings in a separate list.  Merge them only with each other.  Give each the ID `$1-b<n>`, with `<n>` from 1.
6. If a baseline finding is the same finding as a reviewer's finding, give it `same-as:` and the ID of the reviewer's finding.

### 5. Validate the findings

Start one `validator` for each of these:

* each `blocker` and `should-fix` of the reviewers, with no `repeat-of`;
* each `blocker` and `should-fix` of the baseline, but not one with `same-as` a finding that has a validator.

A baseline finding with `same-as` a finding that has a validator gets the verdict of that finding, from its validator.  Each other `nit` gets the verdict `unvalidated`, with no validator.

Start the validators in one message, at most 15 at once.  If there are more than 15, start them in batches of 15.  Start the next batch when each validator of the last batch has ended.  Use the Agent tool, with `subagent_type` set to `validator`.

The range of a finding is `<base>..HEAD` if a source of the finding reviewed that range, or the finding is the baseline's.  Else it is `<since>..HEAD`.  Fill in this prompt template for each finding, and give the validator only that text:

```text
Validate this finding as your instructions say.

ID: <the ID of the finding>
Finding:
<the block of the finding: severity, kind, where, rule, claim, evidence and fix, as a source reported it>

Range: <the range of the finding>
Criteria: $2
Round: $1
Target folder: <target>
```

Each validator gives one block with the fields `id`, `verdict`, `evidence` and `reason`.  Record the verdict of each finding, and its evidence in a few words.  Record the tokens and the duration of each validator from its `<usage>`.

Wait for each validator before step 6.  Step 6 takes its wait for the mutants run off the wall minutes, so that wait must not overlap a validator.

### 6. Validate the missed mutants

1. If the notice that the mutants command ended has come, the wait for it is 0 seconds.  Else, run `jq -n now`, and record its output as the start of the wait.  Then wait for the notice.  When it comes, run `jq -n now` again, and record the end of the wait.
2. Read the mutants line in the notice.  If the notice does not show it, read `<run>/$1/mutants.txt`.  Read that file only after the notice has come, because the run writes the file when it ends.  The line starts with `mutants: full:`, `mutants: partial:` or `mutants: none:`.  If the notice shows no `mutants:` line and the file does not exist, the command refused to run.  Then record its error line as the mutants line, and go to item 8.
3. Read `<run>/$1/mutants.out/missed.txt`.  If the file does not exist or has no lines, go to item 8.
4. Each line of `missed.txt` becomes a finding with the next ID `$1-<n>`.  Its source is `cargo-mutants`, its severity `should-fix` and its kind `weak-test`.  Its `where` is the file and line at the start of the line.  Its rule is `VERIFY-1`, and its claim is "no test fails with this mutant", with the line.
5. Do the `repeat-of` check of step 4, item 4, for each of these findings.  A finding with `repeat-of` gets the verdict `refuted`.  Leave its line out of the validator's prompt.  If no line is left, go to item 7.
6. Start one `validator` with this prompt template, filled in.  The prompt holds each line that has no `repeat-of`.  The range is `<since>..HEAD`, because the mutants run tests the since diff:

   ```text
   Validate these missed mutants as your instructions say.  Give one block for each line, with the line as its id.

   Missed mutants, from cargo-mutants:
   <each line of missed.txt with no repeat-of>

   Range: <since>..HEAD
   Criteria: $2
   Round: $1
   Target folder: <target>
   ```

   The verdict of each of these findings is the verdict of the validator for its line.
7. If a finding of a reviewer and a finding of item 4 are the same finding, as step 4 says, merge the two.  The merged finding keeps the reviewer's ID, and gets `cargo-mutants` in its sources.  If its two verdicts differ, it keeps `confirmed` or `pre-existing` over the others.
8. Run `jq -n now`, and record the output as the end of the round.  Run `jq .started <run>/$1/state.json` for the start of the round.

### 7. Compare with the baseline (`--baseline` only)

A finding is found when its verdict is `confirmed` or `pre-existing`.  If the baseline has no found finding, skip this step.  Record "of 0 baseline findings, the round found 0".

Else start one new subagent with `subagent_type` set to `general-purpose`.  Fill in this prompt template, and give the subagent only that text.  List A holds each found finding of the baseline.  List B holds each found finding of the reviewers and of cargo-mutants.  Give each finding as its ID, its `where` and its claim:

```text
Compare two lists of review findings.  Use only the two lists below.  Do not read files, and do not run commands.

List A holds the found findings of a baseline review.  List B holds the found findings of a review round on the same commit.

For each finding in list A, find a finding in list B that reports the same defect at the same place.  A finding in list B can match more than one finding in list A.

Give one block for each finding in list A:

id: the ID of the finding in list A
mark: found or missed
match: the ID of the matching finding in list B, and a quote of its claim; or none
reason: one sentence

List A:
<the found findings of the baseline: ID, where and claim of each>

List B:
<the found findings of the round: ID, where and claim of each>
```

Record the mark of each baseline finding, with the match.  Count x, the baseline findings marked `found`, and y, all found baseline findings.  Record "of y baseline findings, the round found x", with the numbers.

### 8. Check the state of the repo

Run this command:

```bash
target/debug/xtask review check $1
```

* If it prints "no change", go to step 9.
* If it exits 3, stop.  Do not write findings.  Give the person each line of its output.  Its lines name each change since `review start`.
* If the only change is to the local settings file of Claude Code, from an approval with "don't ask again", the person confirms that change.  Then the person starts the round again.

### 9. Write the findings

Write `<run>/$1/findings.md` with the Write tool, in the form below.  If the Write tool refuses it, do as "Before the steps" says.

```markdown
# Review round $1

* mutants: <the mutants line, as the command printed it>
* base: <base>; since: <since>; criteria: <the criteria>
* flags: <the flags, or none>
* warnings: <each warning: line of review start, or none>
* times: started <started>, wait <seconds>, end <end>; wall minutes <(end - started - wait) / 60, to 1 decimal>
* interventions: <the number of messages and approvals of a person since review start>

## Agents

| Agent | Range | Findings | Tokens | Minutes |
| - | - | - | - | - |
| reviewer | <base>..HEAD | <IDs of its findings, or no findings> | <tokens> | <minutes> |
| validator | <range> | verdicts for <IDs> | <tokens> | <minutes> |

## Findings

### <ID>

* sources: <each reviewer, cargo-mutants>
* severity: <blocker, should-fix or nit>
* kind: <bug, weak-test, rule, doc or other>
* where: <file:line>
* rule: <ID and quote, or none>
* claim: <the claim>
* verdict: <confirmed, pre-existing, refuted, unproven or unvalidated>
* repeat-of: <ID, or none>
* evidence: <the evidence of the validator, in a few words>

## Baseline findings

### $1-b<n>

* (the same fields, with sources: baseline, and same-as: <ID, or none>)
* mark: <found or missed, with the match and its quote; or not compared>

## Baseline comparison

of <y> baseline findings, the round found <x>
```

* List each agent that the round started: the reviewers, the baseline, and each validator.  A validator's Findings cell names the IDs that it judged.
* Without `--baseline`, leave out the two baseline sections.
* With `--review-only`, stop here.  Give the person the path of the findings file and a summary: the mutants line, and the count of findings for each verdict.

### 10. Fix the findings

* Fix each found finding, the baseline's included.  A found finding has the verdict `confirmed` or `pre-existing`.
* Where the finding is about behaviour, first write a test that fails before the fix.
* A `pre-existing` finding can go to `docs/tech-debt.md`, or to its own PR, in place of a fix.
* Do not fix a `refuted` finding.
* Decide each `unproven` finding, and record the reason.
* A nit has the verdict `unvalidated`.  Fix it only if you agree with it.
* Before the commit, run fmt, clippy, the tests and `docs-check` (GIT-2).
* Commit the fixes, signed, with the subject "review round $1 fixes".

### 11. Add the round to the log

The log `<run>/log.md` has two tables.  If the Write tool refuses it, do not write it in another way.  Give the path and the whole text of the file in your final answer, and stop.  If the file does not exist, write it in this form first:

```markdown
# Review log

## Findings

| Date | Round | Commit | Finding | Reviewers | Severity | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |
| - | - | - | - | - | - | - | - | - | - | - | - | - |

## Rounds

| Round | Agents | Tokens | Wall minutes | Interventions | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline found |
| - | - | - | - | - | - | - | - | - | - | - |
```

Add one row to the findings table for each finding of the round, the baseline's included:

* Date: today, as `YYYY-MM-DD`.  Commit: the short SHA of `HEAD` at `review start`.
* Reviewers: the sources of the finding.  Rule: the rule ID, or none.  Claim: one line.
* Action: the fix commit and its subject; or the reason for no change, such as "refuted", or the tech-debt ID.

Add one row to the rounds table:

* Agents: the number of agents that the round started, but the baseline and its own validators.  The validators count with the agents that they validate.  Step 7's comparer counts neither with the round nor with the baseline.
* Tokens: the sum of the tokens of those agents.
* Wall minutes: the wall minutes of the findings file.  They run from `review start` to the last verdict, without the wait for the mutants run.
* Interventions: the number of messages and approvals of a person in the round.
* Mutants: the mutants line.
* With `--baseline`: the baseline agents are the baseline and the validators that step 5 started for its findings.  Their tokens are the sum of their tokens.  Their wall minutes are the duration of the baseline, plus the longest duration of those validators.  Their interventions are those of a person for those agents.  Baseline found is "of y baseline findings, the round found x".  Without `--baseline`, write "-" in each baseline cell.

After round 3:

* Copy `<run>/log.md` to the reviews folder under `docs`, as `<branch>.md`.  `<branch>` is the last part of the `<run>` path.
* Write a Progress line for each round in the plan of the work.
* Run the checks, and commit, signed.
* Give the person the text for the PR description: each `refuted` finding, each `unproven` blocker or should-fix, each `repeat-of`, and the mutants line of each round.
