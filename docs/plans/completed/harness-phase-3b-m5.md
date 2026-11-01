# Harness phase 3b, M5: a model and effort for each agent role (decision 0025)

## Purpose

This is the milestone plan for M5 of `plans/active/harness-phase-3b.md`.  It gives more detail than the parent plan's M5 section, which stays the criteria.  M5 puts decision 0025 into the harness.

After M5:

* `agents_folder_problems` accepts `model` from the list `inherit`, `opus`, `sonnet`, `haiku`, and an optional `effort` key.
* A measurement on logged findings compares a cheaper model with the control for the validator and the docs-reviewer.  Its results table, with 95% intervals, is in this plan's Surprises and in the parent's Surprises.
* Each role that passes the pass rule gets `model` and `effort` in its file in `.claude/agents/`.  Each other role keeps `inherit`.  A test pins the model and the effort of each agent file to the results table.

Readers: the implementer of M5, the review agents of M5's rounds, and the orchestrator.

## Context and orientation

The state at the start of M5, on 2026-11-01, at commit `8c228fe` on branch `harness-phase-3b`:

* Each of the eight files in `.claude/agents/` has `model: inherit`.  `agent_problems` in `xtask/src/rules.rs` (near line 3054) requires it: the `model` check near line 3165 fails any other value.  `AGENT_KEYS` (near line 3020) is `["name", "description", "tools", "model", "hooks"]`.  Each key must be present once, and no other key is allowed.  `hooks` must be the last key.  `agents_folder_problems` (near line 3284) runs `agent_problems` on each file.
* The unit tests of the rule are in `mod agents_tests` of `xtask/src/rules.rs` (near line 6748).  One case asserts the message "has `model: opus`, not `model: inherit`" (near line 6917).  `xtask/tests/rules.rs` (near line 1059) runs `agents_folder_problems` on the real folder.
* `BASELINE_HASH` in `xtask/tests/rules.rs` freezes `.claude/agents/baseline.md`.  `baseline.md` stays as it is.
* Claude Code is 2.1.309.  Its docs ([sub-agents](https://code.claude.com/docs/en/sub-agents), [model config](https://code.claude.com/docs/en/model-config), [CLI reference](https://code.claude.com/docs/en/cli-reference)) say:
  * A subagent's `model` takes `sonnet`, `opus`, `haiku`, `fable`, a full model ID, or `inherit`.
  * A subagent's `effort` takes `low`, `medium`, `high`, `xhigh` or `max`.  It overrides the session's effort.
  * `claude -p --agents <file>` takes `model` and `effort` for each agent.
  * With no effort set anywhere, Opus 5.5, Sonnet 5.5 and Haiku 5.5 run at `medium`.
  * Without `--forward-subagent-text`, a foreground subagent's text and thinking are not in the stream.
* The maintainer's `~/.claude/settings.json` sets `"effortLevel": "xhigh"` and `"modelSettings": {"claude-opus-5-5": {"effortLevel": "high"}}` (phase 3, M3).  So the real review rounds of PR #39 ran each agent on Opus 5.5 at `high`.
* The test harness H is `/private/tmp/stemyla-harness/`.  It does not exist now.  `make-harness.sh` rebuilds it.  Its whole text is in the Progress of `plans/completed/harness-phase-3-m3.md` (2026-10-23, review round 1 fixes, item 5).  `reviews/replay.md` holds the case command, the settled allow list, `replay-clone.sh` and `replay-check.sh`.
* `KNOWN_MISSED` in `xtask/src/review.rs` holds 13 entries.  An entry matches a missed line by four parts: the file, the mutation, the code line and the offset.
* The logs of the measurement are at local refs: `refs/remotes/origin/pr/39` (`docs/reviews/iphone-crashes-2026-10-25-*.md`, eight logs), `refs/remotes/origin/pr/35` (`docs/reviews/feature-48k-audio.md`) and `refs/remotes/origin/pr/37` (`docs/reviews/feature-multi-download.md`).  Their Findings tables have 1,229 rows.
* About 723 GiB is free on the volume.
* The orchestrator committed `10e7a03` after `8c228fe`: an M4 fix that CI found (`cargo metadata` in `xtask/src/rules.rs` runs with `--color never`).  It is not M5 work.  The review base stays `8c228fe`, so round 1 reviews that fix too.

Terms:

* Role: an agent file of `.claude/agents/`.
* Arm: a model and an effort for one role.  The control arm is `model: inherit` with no `effort` key, in a session that runs `--model claude-opus-5-5 --effort high`.  So the control is the configuration of PR #39's rounds.
* Case: one logged round that the measurement replays, at the commit in the log's Commit column.
* Run: one `claude -p` session of one arm on one case, in its own clone.
* Driver prompt: the prompt of a run.  It tells the main agent to start the agents under test, each with a prompt from the skill's templates.  The main agent does not run the `review-rounds` skill.
* Item: one entry of a ground-truth list.
* Found: a logged verdict of `confirmed` (with any text after it) or `pre-existing`.

## Plan of work

### How the team works

* The team leader writes this plan.  A new `Plan` agent reviews it, at most 2 rounds, and the team leader fixes what it finds.  The team leader commits the plan and its index line, signed.
* The team leader starts one implementer (`general-purpose`) for step 2.  Later, the same implementer, or a new one with this plan's path, makes the code part of step 5 and the fixes of the review rounds.
* The team leader runs the measurement (steps 3 and 4) itself.  It reads each transcript with `jq` only.  It starts the comparers of step 4.
* The team leader and the implementer do not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.  Each change there goes to the orchestrator as a unified diff in the orchestrator's scratchpad folder `m5/`.
* Before each commit: `cargo run -p xtask -- check --tier commit` (GIT-2).  Before each review round: `cargo run -p xtask -- check --tier push` and `cargo build -p xtask`.  Each commit is signed (`git commit -S`).  Docs and commit messages are in ASD-STE100.
* A new test fails first on an assert, not on a compile error.  The implementer records each failure in Surprises.
* In H, the team writes only `bin/`, the agents files, `prompts/`, the clones, their transcripts and their time files.  It removes nothing in H.  At the end, the team gives the orchestrator the list of clone folders.  The orchestrator runs `cargo run -p xtask -- clean --yes --also <clone>` for each, never for H itself.
* No agent of M5 runs `/review-rounds` or `xtask review start` in the main checkout, but in step 6.

### Step 1: this plan

The plan and its line in `docs/README.md`, in one signed commit.

### Step 2: the agents rule accepts a model and an effort

In `xtask/src/rules.rs`:

* `AGENT_KEYS` keeps the five keys that each file must have.  A new `AGENT_OPTIONAL_KEYS` is `["effort"]`.  The unknown-key check allows both lists.  The twice check covers both lists.
* A new `AGENT_MODELS` is `["inherit", "opus", "sonnet", "haiku"]`.  The `model` check fails a value that is not in it, with the message "has `model: <value>`, which is not one of `inherit`, `opus`, `sonnet` and `haiku`".
* A new `AGENT_EFFORTS` is `["low", "medium", "high", "xhigh", "max"]`.  An `effort` key with another value, or with no value, fails, with the message "has `effort: <value>`, which is not one of ...".
* `hooks` stays the last key, so `effort` comes before `hooks`.  The hooks-last check already fails an `effort` after `hooks`.
* The fix text of `keep_the_keys` names `effort` as optional.
* `fable` and full model IDs are not in the list, since decision 0025 names four values.

Tests, in `mod agents_tests`:

* A file with `model: sonnet`, and one with `model: haiku`, has no problem.
* A file with `model: sonnet` and `effort: medium` before `hooks` has no problem.
* A file with `model: gpt` gives exactly one problem, with the new message.
* A file with `effort: extreme`, and one with `effort:` and no value, each give exactly one problem, with the new effort message.
* A file with `effort: high` twice gives exactly one problem: "has the key `effort` twice".
* A file with `effort: high` after the hook block gives exactly two problems: the hooks-last problem, and "has a hook block that is not `AGENT_HOOK_BLOCK`".  The line has no indent, so the check reads it as a key, and the hook block then holds it.
* The old case "has `model: opus`, not `model: inherit`" changes: `model: opus` now has no problem.
* Each new case asserts the exact list of messages, not a part of one.  So each case fails on the old code: there, `effort` gives an unknown-key problem, and a list with that problem is not the expected list.

The stub: first, the implementer changes only the tests, and runs them on the old code.  The old code fails each new case on an assert (for example, `model: sonnet` gives a problem where the test wants none).  The implementer records the failures in Surprises.  Then it writes the code, and commits the tests and the code together.

Docs, in the same commit (DOC-1): `docs/tech-debt.md` and `docs/harness.md` hold no text on `model: inherit` today (`grep -rn inherit docs`).  The implementer greps again, and changes each doc that states the old rule.  Plans, decisions and the logs in `docs/reviews/` record their time, so they stay as they are.  One line is different: the parent plan's line on `agents_folder_problems` says that it "requires `model: inherit` today".  Its change goes to the orchestrator as a diff.

### Step 3: the harness, the ground truth and the pass rule (checkpoint A)

The harness:

1. Write `H/bin/make-harness.sh` with the text of M3's Progress, and run it on this repo.  Check: exit 0; `jq -e '[.[] | .hooks.PreToolUse[].hooks[].onFailure] | length == 8 and all(. == "block")' H/agents.json` prints `true`; `jq -r 'keys[]' H/agents.json` gives the eight names.
2. Make one agents file for each arm with `jq`, from `H/agents.json`:
   * `H/agents-control.json`: `H/agents.json`, with no change.
   * `H/agents-validator-sonnet.json`: the `validator` gets `"model": "sonnet", "effort": "high"`.
   * `H/agents-docs-sonnet.json`: the `docs-reviewer` gets `"model": "sonnet", "effort": "high"`.
   * Only if budget is left: `H/agents-validator-haiku.json`, with `"model": "haiku", "effort": "high"`.
   * The candidate gets `effort: high` explicitly, so the agent file that a pass gives runs at the effort that the measurement used.  Without it, the effort of a Sonnet subagent would come from the person's session (`xhigh` on this Mac).
   * Each run, control and candidate, has a session with `--model claude-opus-5-5 --effort high` (the orchestrator, at checkpoint A, `ed5e4cb`).  Reason: the driver's effort must not differ between arms.  So a candidate subagent runs at `high` from its key or from the session; the measurement does not show which.  No field of the stream shows a subagent's effort, so no check reads it; the tech-debt entry of step 5 names this gap.
3. Write `H/bin/m5-clone.sh <commit> <clone>`.  It makes `H/<clone>/` with `git init`, then `git -C H/<clone> fetch --no-tags <repo> <full SHA>`, then `git -C H/<clone> checkout -b m5-<short SHA> FETCH_HEAD`.  So the clone holds the commit and its ancestors only, and no later fix.  It adds `!/Cargo.lock` to `.git/info/exclude`.  It exits 1 if a check fails:
   * `HEAD` is the commit;
   * `git rev-list --count HEAD` equals the count in this repo;
   * `git for-each-ref` lists only the branch;
   * `git ls-files -ci --exclude-standard` and `git status --porcelain` print nothing;
   * the volume has 30 GiB free or more.
4. Write `H/bin/m5-check.sh <clone>`.  It reads `H/<clone>.jsonl` with `jq` only, and prints one line for each check:
   * Result: the last `result` has `is_error` false and `subtype` `success`.
   * Prompt: the main agent's Agent calls are exactly the expected ones.  Each has the expected `subagent_type`, and its `prompt` equals the expected text in `H/prompts/<clone>.expected.json`, after the white space at each end is removed.  Each call has a tool result that is not an error.
   * Arm: each subagent `assistant` message names its `message.model`.  In a control run, each names `claude-opus-5-5`.  In a candidate run, each message of the agents under test names the arm's model (`claude-sonnet-5-5` or `claude-haiku-5-5`).  The check fails when no subagent message names a model.  The keys of the last `result`'s `modelUsage` are printed as information only, since Claude Code can make its own calls on a small model.
   * Tripwire: parts (a) to (c) of `replay-check.sh`, as `reviews/replay.md` gives them, on this clone.
   * Denials: each `dontAsk` denial, with its command, as in `replay-check.sh`.
   * `check --tier` calls: each Bash `tool_use` whose `input.command` has a simple command (split at `;`, `&&`, `||`, `|` and new lines) whose program is `cargo` or ends with `/bin/xtask`, and that holds `check --tier`.  It reads every `assistant` message, the main agent's and the subagents', with no `parent_tool_use_id` filter.  It prints the count and each command.
   * Cost: the sum of input, output, cache creation and cache read tokens over the last `result`'s `modelUsage`, for each model and in total.  Also the tokens of each subagent, from the `message.usage` of its messages, once for each message ID.  Also the wall time from `H/<clone>.time`.
   * Output: it writes the text of each Agent tool result to `H/<clone>.out/<n>.txt`.
5. Two dry runs, each on a round that holds no item: the sonnet validator arm on finding 2-1 of the PR #35 log (`3ac5db4`), and the control docs-reviewer arm on PR #35 round 3 (`e9c3aa7`, range `3ac5db4..HEAD`).  The second one runs `review start` in an old clone, and a comparer with the prompt of step 4 on its findings as "run A" only, with that round's confirmed docs-reviewer findings as a test list.  Each passes when each check of `m5-check.sh` passes.  A failed dry run allows a harness change only (a script, the allow list, a flag), and a new dry run, at most 3.  The dry run does not count.

The run command, as one background Bash command, for a clone `<clone>` and an arm file `<arm>`:

```bash
date +%s > /private/tmp/stemyla-harness/<clone>.time
cd /private/tmp/stemyla-harness/<clone>
claude -p --setting-sources project,local --model claude-opus-5-5 --effort high \
  --agents ../agents-<arm>.json --permission-mode dontAsk --forward-subagent-text \
  --allowedTools 'Read' 'Write' 'Agent' \
    'Bash(/private/tmp/stemyla-harness/bin/xtask *)' 'Bash(cargo *)' 'Bash(git *)' \
    'Bash(grep *)' 'Bash(find *)' 'Bash(sed *)' 'Bash(ls *)' 'Bash(cat *)' \
    'Bash(rm -rf target/review/*)' 'Bash(head *)' 'Bash(tail *)' 'Bash(wc *)' \
    'Bash(jq *)' 'Bash(sort *)' 'Bash(diff *)' 'Bash(echo *)' 'Bash(rustc --version)' \
  --output-format stream-json --verbose \
  < ../prompts/<clone>.txt > ../<clone>.jsonl
echo "$? $(date +%s)" >> /private/tmp/stemyla-harness/<clone>.time
```

* The prompt comes from stdin, so the command line holds no long quoted text.
* The allow list is the settled list of `reviews/replay.md`.
* `--agents` takes priority over the old agent files in the clone, so the agents under test are this branch's.

The driver prompt:

* It tells the main agent: start the agents below with the Agent tool, in one message, in the foreground, each with the given `subagent_type`.  Give each one the text between its `BEGIN` and `END` lines as its prompt, exactly.  Start no other agent.  Do not read files or run commands.  When each agent has ended, give each output, after a line `== <ID>`, as the final answer.
* Each agent's text is the template of the `review-rounds` skill, filled in: step 3's template for a reviewer, step 5's for a validator of one finding, step 6, item 7's for the validator of the missed mutants.
* A validator's finding block holds the fields of the log row: severity, kind, where, rule and claim.  The log has no evidence and no fix, so these two fields say "not recorded".
* The last paragraph of the prompt, after the agents' texts and not in their criteria, says: "The checks before the steps of the review-rounds skill passed at the base of this replay.  Do not run them."  The parent's M5 section puts this line after the flags of a skill prompt.  A driver prompt has no flags, so the line goes at its end.
* Before a reviewer's run, the team leader runs `H/bin/xtask review start <range start> 1` in the clone, before `claude -p`.  Its `target folder:` line gives the target folder.  The docs-reviewer's line of the route gives its reasons.  If the route does not name the docs-reviewer, the reasons are "none".  A validator's target folder is `<clone>/target/review/target`.
* The `Round:` line of each template holds the number of the round, such as `4` for "speed 4".  The agents name their worktrees `target/review/<round>-<agent>-<n>`.
* The criteria of a case are the plan paths at its commit.  For a PR #39 milestone, they are `docs/plans/active/stream-from-storage.md#<the milestone's heading>` and the milestone plan.  For the catch-up, they are `docs/plans/active/stream-from-storage-catchup.md`.  For PR #35, they are `docs/plans/active/48k-audio.md` (round 1) or `docs/plans/completed/48k-audio.md` (round 4).  For PR #37, they are `docs/plans/active/mix-download.md`.
* The range of a validator's finding follows the skill's step 5 and the rounds of today's step 3.  Round 1: `<base>..HEAD`.  Rounds 2 and 3: `<base>..HEAD` if `reviewer` or `baseline` is a source, else `<since>..HEAD`.  From round 4, and for the missed mutants: `<since>..HEAD`.  `<since>` is the Commit of the round before in the same log.  A round's number is the number in its label, so "speed 5" is round 5.
* A reviewer's range is `<base>..HEAD` in round 1 and `<since>..HEAD` in a later round, as the docs-reviewer had in the logged round.  The template has no `Scope:` line, also from round 4.  Reason: the logged rounds came before decision 0024, and with the line, items 4-1 and 4-2 of M8 speed 4 are outside the scope.
* In round 1, `<since>` is `<base>`, as `review start` makes it.  `<since>` of a later round is the Commit of the round before in the same log (`since_is_the_base_then_the_last_rounds_head` in `xtask/src/review.rs`).

The cases.  `<base>` is the commit before the milestone's first commit, as `plans/completed/stream-from-storage.md` ("How the work runs") gives it, or the merge base with `main` for PR #35 and #37:

| Case | Log, round | Commit | Base | Since | Source of the base |
| - | - | - | - | - | - |
| validator | PR #39 catch-up, 1 | `9611cab` | `d420932` | `d420932` | the catch-up plan names it |
| validator | PR #39 M8, 2 | `9c69f22` | `c3952ad` | `f28d22d` | the start commit of the M8 plan, the parent of its first commit `8c3fcac` |
| validator | PR #39 M8, speed 5 | `bf3334f` | (not used) | `f28ebff` | from round 4, each range starts at `<since>` |
| validator | PR #39 M8, speed 6 | `720f089` | (not used) | `bf3334f` | as above |
| validator | PR #35, 4 | `6a4dcf2` | (not used) | `e9c3aa7` | as above |
| validator | PR #37, 1 | `0732e53` | `db64d15` | `db64d15` | `git merge-base 0732e53 origin/main` |
| docs-reviewer | PR #39 M8, speed 4 | `f28ebff` | (not used) | `d88811b` | a later round |
| docs-reviewer | PR #39 M7, 1 | `db45675` | `c4f3d6a` | `c4f3d6a` | the M7 plan's start commit, the parent of `9ab6221` |
| docs-reviewer | PR #35, 1 | `9779b2c` | `62c50ab` | `62c50ab` | `git merge-base 9779b2c origin/main` |
| docs-reviewer | PR #39 M5, 2 | `6e57ad8` | (not used) | `754d8be` | a later round |
| docs-reviewer | PR #39 M9, 1 | `0789e6b` | `a6ac21b` | `a6ac21b` | the M9 plan: the base is the M8 commit before M9's code; `a6ac21b` is the parent of `298e3c3`, M9's first code commit |
| docs-reviewer | PR #39 M8, speed 1 | `2c7c29b` | `2c52968` | `2c52968` | inferred: the speed rounds were a new run of step 6, and `2c52968` is step 6's plan commit and the "before" build of the M8 plan |
| docs-reviewer | PR #39 catch-up, 1 | `9611cab` | `d420932` | `d420932` | the catch-up plan names it |

The base of M8 speed 1 is the only inferred one.  Each of its five items names a file that `2c52968..2c7c29b` changes.  Items 1-3 and 1-5 name the M8 plan by its later path in `plans/completed/`; the range changes it as `docs/plans/active/stream-from-storage-m8.md`.

The ground truth.  The team leader parses the Findings tables of the 10 logs with `awk` into one row for each finding.  Then:

* Validator items, 40, come from the six cases that hold the six refuted reviewer findings.  The cases are PR #39 catch-up round 1 (`9611cab`), M8 round 2 (`9c69f22`) and M8 round speed 5 (`bf3334f`).  The others are M8 round speed 6 (`720f089`), PR #35 round 4 (`6a4dcf2`) and PR #37 round 1 (`0732e53`).
  * Strata: each item is a blocker or should-fix with `repeat-of` none.  R holds each refuted finding of a reviewer or the baseline (6).  B holds each found finding of kind `bug` of a reviewer or the baseline (all, 5 here).  C holds the other found findings of a reviewer or the baseline (9).  Mr holds the refuted findings of `cargo-mutants` only (10), and Mf the found findings of `cargo-mutants` only (10).
  * A row whose Action says "as its same-as" is left out.  It is a baseline finding that took the verdict of a reviewer's finding, with no validator of its own (step 5 of the skill).  So PR #37's 1-b1, the defect of 1-1, is not an item.
  * A `cargo-mutants` row counts only when its claim gives the line, the column and the mutation.  The claim gives them as `<file>:<line>:<col>: <mutation>`, or as `<mutation> (<line>:<col>)` with the file in Where.  These give the line of `missed.txt`.
  * Within a stratum, the rows come round-robin over the six cases, in the order above, in the order of the log.  So the strata spread over the cases.
  * Known: no mutant item matches a `KNOWN_MISSED` entry by its four parts at its commit, so `known 0` for each run.  Each mutant item goes to the validator directly, as the parent says.  Item 2-15 of M8 round 2 has a TD-39 entry, but its code line at `9c69f22` differs from the entry's.  Row 2-17 of that round matches its entry exactly, but the round-robin takes only three of that round's four Mf rows, so it is not an item.
* Docs-reviewer items, 40: the docs-reviewer's own blockers and should-fix with the verdict `confirmed`, not `pre-existing` (the docs-reviewer is a source), in the rounds with the most of them.  The rounds, by count: M8 speed 4 (9), M7 round 1 (7), then four with 5 (PR #35 round 1, M5 round 2, M9 round 1, M8 speed 1), then the catch-up round 1 (4).  Three rounds tie at 4.  The tie goes to the round with the most items of kind `bug`, which is the catch-up round 1 (1 bug; M9 round 2 and M7 round 2 have none).  The seven rounds give exactly 40 items, 4 of them of kind `bug`.

The checkpoint A text (step 3's output) holds each item, with its log, round, finding ID, kind and logged verdict, the case table (commit, base, since, range, criteria), the pass rule, the arms, the budget plan and the commands.  The orchestrator commits the pass rule and the lists to the parent's Decision log.  Only then does any counted run start.  The dry run can come before.

### The pass rule

Common:

* The 95% interval is the Wilson score interval ([Wilson, 1927](https://doi.org/10.1080/01621459.1927.10502953)), as in `reviews/replay.md`.  Its width is upper minus lower.
* Each arm has one run on each case, to fit the budget.  So the measure does not separate run noise from the model's effect.  The results table says so.
* A role that does not pass, or that the measurement does not reach, keeps `inherit`.

Reviewer (the docs-reviewer):

* A finding of a run matches an item by the matching rule of `plans/completed/harness-phase-3-m5.md`, step 6, item 2: the same defect at the same place, in any words.  The items are blockers and should-fix, so a nit of a run matches no item.  The run's findings have no validator, so each blocker and should-fix of the run counts.
* Recall: the items that the arm's runs find, divided by n.  n is 40 when each case counts.
* A bug is an item of kind `bug`.  A bug is missed when the control found it and the candidate did not.
* The candidate passes when each of these holds:
  0. each case of the role counts for both arms, so n is 40;
  1. the width of its recall interval is 0.3 or less;
  2. its recall is at least the control's recall minus 0.15;
  3. it misses no bug.

Validator:

* Each item has a logged class: found or refuted.  A run's verdict agrees when its class is the same.  `unproven`, a missing verdict, or `known` agrees with neither class.
* Agreement: the items whose verdict agrees, divided by n, with its interval.  n is 40 when each case counts.
* The candidate passes when each of these holds:
  0. each case of the role counts for both arms, so n is 40;
  1. it gives `refuted` to no item of kind `bug` whose logged class is found, and it gives `confirmed` or `pre-existing` to each such item that the control gave one of them (decision 0025: no confirmed bug missed);
  2. it refutes at least as many items of strata R and Mr as the control;
  3. the width of its agreement interval is 0.3 or less;
  4. its agreement is at least the control's agreement minus 0.15 (decision 0025's recall line).
* Item 4 stops a candidate that refutes too much: items 1 and 2 alone would pass a validator that refutes every finding but the bugs.

The arms that the measurement drops before any run, with the reason:

* The logs hold 6, 37 and 32 confirmed blockers and should-fix of the gpu-reviewer, the wire-reviewer and the supply-chain-reviewer, in 3, 23 and 20 rounds.
* gpu-reviewer at effort `medium`: with n = 6, the interval is wider than 0.3 at each recall from 0.1 to 0.9.  It cannot meet the limit.
* wire-reviewer and supply-chain-reviewer at effort `medium`: with n = 37 or 32, the interval is wider than 0.3 at recall 0.5, but narrower at a high recall (n = 37 gives about 0.25 at 0.8).  So the width alone does not drop them.  The cost does: two arms on 23 or 20 rounds, at about 1.1M a run (`reviews/replay.md`), are about 44M to 51M tokens each, more than the whole budget.
* So these three roles keep `inherit`, and Surprises says so.
* With n = 40, the interval at recall 0.5 is 0.296 wide, and narrower at each other recall.  So the validator and the docs-reviewer arms can meet the limit.

### Step 4: the measurement (checkpoint B)

The order and the budget:

* The budget is about 45M `modelUsage` tokens, summed over the runs, the dry run included.  The comparers and the team leader's own tokens are recorded apart.
* The order: the validator (control and `sonnet`), then the docs-reviewer (control and `sonnet`), then, only if budget is left, the `haiku` validator.
* The estimates before a run: a validator run 1.3M, a docs-reviewer run 1.6M.  After a run of a role, its estimate is the mean of that role's runs so far.
* Before each run, add the tokens used, the estimate of each run in progress and the estimate of this run.  If the sum is more than 45M, the run does not start.  Then the measurement stops with what it has.
* Before the docs-reviewer arms: if the budget left is less than 14 docs-reviewer runs at the estimate, they do not start.  With fewer than 7 cases, item 0 of the pass rule fails.  The same holds for the `haiku` arm with 6 validator runs.
* Both arms of one case run at the same time, in two clones.  At most two cases run at the same time.
* After each run: `H/bin/m5-check.sh <clone>`, and the totals in the scratch file `m5/budget.txt`.

A run counts when the checks Result, Prompt and Arm pass.  An error result, a usage limit or an agent with no output cuts a run short.  A run cut short runs again in a new clone, with no change, at most 3 times.  Its tokens count in the budget.  A Tripwire hit in a candidate run makes its items count as not found, or as not agreeing.  A Tripwire hit in a control run, or a case with 3 cut-short runs of either arm, takes the case out for both arms.  Then item 0 of the pass rule fails, so the role keeps `inherit`.  Reason: a failed control would make the candidate pass more easily.

The results:

* Validator: the team leader takes each verdict from the validators' outputs (`H/<clone>.out/`), with `grep`.  It needs no comparer.
* Docs-reviewer: for each case, one new `general-purpose` comparer gets the case's items and the findings of both runs, as "run A" and "run B".  The order of the two runs changes from case to case by a fixed rule, and the comparer is not told which run is which arm.  It reads no file.  Its prompt holds the matching rule of step 6, item 2.  It marks each item `found` or `missed` for each run, with the matching finding and a quote of at most 30 words.  Each case's comparers start in one message.
* The results table: for each arm, n, the count, the share and its interval, the bugs missed or refuted, and the refutations of strata R and Mr.  Also each arm's tokens, its number of findings (docs-reviewer) and its `check --tier` calls.  And the pass result of each candidate, with the item of the pass rule that fails, if any.

Checkpoint B gives the orchestrator the results table, the tokens used, and a diff of `.claude/agents/*.md` for each role that passes.  A role that passes gets `model: sonnet` (or `haiku`) and `effort: high` before `hooks`.

### Step 5: the agent files and their pin

* The orchestrator applies the diff of `.claude/agents/`.
* The implementer writes a test in `xtask/tests/rules.rs` that pins the `model` and `effort` of each agent file to the results table (a constant table in the test).  It gives the test to the team leader as a diff, not a commit.  The team leader gives the orchestrator one diff with the agent files and the test, and the orchestrator commits them together.
* The implementer shows that the test fails without the agent change: in a `git worktree` under `target/`, it applies only the test part, and runs `cargo test -p xtask --test rules <the test>` there.  It records the failure in Surprises.
* If no role passes, no agent file changes, and the test pins `inherit` and no `effort` for each file.  Then the test passes without a change.  So the implementer plants `model: sonnet` in one agent file in a `git worktree` under `target/`, runs the test there, shows that it fails, and records it in Surprises.
* No role passed (step 4), so no agent file changes.  The test pins `inherit` and no `effort` for each file.  Nothing under `.claude/` changes, so the implementer committed the test and the tech-debt entry, signed.  The orchestrator agreed.
* `docs/tech-debt.md` gets an entry for the gaps of the measurement: one run for each arm, the reconstructed finding blocks, no check of a subagent's effort in the stream, and the roles that keep `inherit` for want of evidence.  The re-run when the model line-up changes is phase 6's (decision 0025).

### Step 6: review rounds

* Run `cargo run -p xtask -- check --tier push` and `cargo build -p xtask`.  Then run `/review-rounds 8c228fe <round> "<criteria>"` from round 1, with no `--baseline`.
* The criteria are the parent plan's M5 section and this plan.  The range also holds the orchestrator's M4 fix `10e7a03`, which round 1 reviews by the parent's M4 section.
* The rounds follow decision 0024: the fix rule of step 10 of the skill, the late range of step 3, and the stop rule of step 11.  There is no cap.
* The Write tool refuses `findings.md` and `log.md` to the team leader.  So the team leader runs each round through step 11, with `review log-check` on its scratch copy and `review clean`.  Then it gives the orchestrator the whole of both files, each with its target path.
* The team leader fills the `Significant` field and column.  A wording gap in a doc that no agent acts on is not significant (M3's reading).
* Each fix of behaviour gets a test that fails first.  A fix of `.claude/**` or `CLAUDE.md` goes to the orchestrator as a diff.

### Changes to the parent plan

The team leader gives these to the orchestrator as text:

* At checkpoint A: the pass rule and the ground-truth lists, for the parent's Decision log.
* At the close: the M5 Progress line, with the stats of the rounds; the Surprises, the results table included; Decision log lines, agent; the PR text; and the list of clone folders.

## Validation and acceptance

* `cargo test -p xtask` passes with the tests of step 2.  Each new test failed on the old code, on an assert (Surprises).
* The results table with 95% intervals is in this plan's Surprises and in the parent's Surprises.
* The agent files match the results table, and the pin of step 5 fails without the agent diff, or on the plant when no role passes.
* `cargo run -p xtask -- check --tier push` and `docs-check` pass on the last commit.

## Idempotence and recovery

* The measurement changes nothing in the repo.  Its files are in H and in the scratchpad.  A run that stops can run again in a new clone.
* The tokens of every run, counted or not, stay in `m5/budget.txt`, so a new team leader knows what is left.
* H is under `/private/tmp`, which macOS clears at a restart.  This plan holds the steps that rebuild it.  The scripts' text goes into the Surprises at checkpoint B.
* Each commit of M5 is undone by `git revert`.

## Progress

* [x] 2026-11-01: step 1, this plan.  Two `Plan` review rounds: round 1 found 1 blocker, 8 should-fix and 3 nits; round 2 found 1 blocker, 3 should-fix and 3 nits.  All fixed.
* [x] 2026-11-01: step 2, the agents rule accepts a model and an effort.  `agent_problems` accepts `model` from `AGENT_MODELS` and an optional `effort` from `AGENT_EFFORTS` before `hooks`.  Tests: `each_model_and_effort_gives_its_problems` (9 cases) in `mod agents_tests`; the case "another model" of `each_plant_is_named` is removed, since `model: opus` now has no problem.
* [x] 2026-11-01: step 3, checkpoint A.  H rebuilt at `f3a4f8f`; the ground truth and the pass rule are in the parent's Decision log (`ed5e4cb`), with the orchestrator's change: every session runs at `--effort high`.
* [x] 2026-11-01: step 4, checkpoint B.  2 dry runs and 29 counted runs, 40.85M `modelUsage` tokens.  No role passes (Surprises, the results).  The `haiku` validator stopped after 3 of 6 cases.
* [x] 2026-11-01: step 5, the agent files and their pin.  No agent file changes.  `the_agent_models_are_the_measured_ones` in `xtask/tests/rules.rs` pins `model: inherit` and no `effort` for each of the eight agents, and TD-71 records the gaps of the measurement.
  * 2026-11-01: the orchestrator removed the 35 clone folders of the measurement with `xtask clean --yes --also` (29.1 GiB).  The harness root stays.
* [x] 2026-11-01: step 6, review round 1 of `8c228fe..d077706` (reviewer, supply-chain-reviewer, docs-reviewer, 1 validator; 298,416 tokens; 10.1 wall minutes).  1 confirmed significant should-fix (1-1: the plan did not record the data of the scripts) and 5 nits, all fixed in `686cfe1`.  The parent's line on `agents_folder_problems` went to the orchestrator as a diff (`ed5f898`).  Mutants: 10 tested, 0 missed.
* [x] 2026-11-01: step 6, review round 2 (reviewer, supply-chain-reviewer, 1 validator; 165,452 tokens; 5.5 wall minutes).  1 confirmed significant should-fix (2-1: the haiku arm refuted 4 of 10 R and Mr items, not 4 of 11) and 2 nits, fixed in `b2626ff` and the orchestrator's `9d705bd`.  Mutants: 4 tested, 0 missed.
* [x] 2026-11-01: step 6, review round 3 (reviewer, supply-chain-reviewer; 109,354 tokens; 5.5 wall minutes).  2 nits, fixed in `8613d58` (the model fix text points to `AGENT_MODELS_MEASURED`; the haiku row gets its interval).  No confirmed blocker, significant should-fix or strict finding, so round 3 is the last round (decision 0024).  Mutants: 2 tested, 0 missed.

## Surprises and discoveries

* 2026-11-01: the docs of Claude Code say that a foreground subagent's text is not in the stream without `--forward-subagent-text`.  So the run command passes it.
* 2026-11-01, M5 step 2, implementer: on the old code, each of the 9 cases of `each_model_and_effort_gives_its_problems` failed on its assert.  `model: opus`, `sonnet` and `haiku` gave "not `model: inherit`".  `model: gpt` gave the old message.  Each `effort` case gave "has the key `effort`, which the check does not allow", plus its other problems.
* 2026-11-01, M5 step 2, implementer: no doc outside `docs/plans/` and `docs/decisions/` states the old `model: inherit` rule (`grep -rn inherit docs`).  The plans and decision 0025 tell the state before M5, so they stay as they are.

* 2026-11-01, step 3: the person's global excludes file ignores `CLAUDE.md`, and the replayed commits do not un-ignore it.  So the first clone failed the check of `m5-clone.sh` ("tracked files are ignored: CLAUDE.md").  `m5-clone.sh` now adds `!/CLAUDE.md` next to `!/Cargo.lock` in `.git/info/exclude`.
* 2026-11-01, step 3: `H/bin/xtask review start <range start> 1` runs in an old clone, and gives the docs-reviewer's line of the route.  It warns that the binary is older than `xtask/src/review.rs`; that is expected.
* 2026-11-01, step 4: the two dry runs passed each check: the sonnet validator on `3ac5db4` (0.14M, 36 s) and the control docs-reviewer on `e9c3aa7` (1.13M, 264 s).  The dry comparer found the one test item.
* 2026-11-01, step 4: no subagent of any run called `check --tier` (0 calls in each of the 31 transcripts, main agent and subagents).  Each run recorded `known 0`.
* 2026-11-01, step 4: each run passed Result, Prompt and Arm.  No run was cut short.  The Tripwire gave `CHECK` in 18 runs and no hit.  Each `CHECK` was a read of the cargo registry, a read of the run's own persisted tool output, or a file in `/tmp` that the same run wrote.  `dontAsk` denials: 0 to 16 a run, most of them compound commands with a variable, a heredoc or a `cd`, as in `reviews/replay.md`.
* 2026-11-01, step 4: a comparer read one file, the case's input, with the Read tool.  Step 4 said "it reads no file".  Reason: the input of a case is 8 to 23 KB, and a prompt that held it would put it in the team leader's context too.  The key of the runs A and B was in another folder.
* 2026-11-01, step 4, the results.  Each arm had one run on each case, so the measure does not separate run noise from the model's effect.  Tokens are `modelUsage` sums (input, output, cache creation and cache read).

  | Role | Arm | n | Count | Share [95% interval] | Width | Bugs | R and Mr refuted | Runs | Tokens | Result |
  | - | - | - | - | - | - | - | - | - | - | - |
  | validator | control | 40 | 36 agree | 0.900 [0.769, 0.960] | 0.191 | 5 of 5 found, 0 refuted | 13 of 16 | 6 | 6.46M | - |
  | validator | `sonnet`, `high` | 40 | 35 agree | 0.875 [0.739, 0.945] | 0.207 | 5 of 5 found, 0 refuted | 11 of 16 | 6 | 3.35M | fails item 2 (11 < 13) |
  | validator | `haiku`, `high` | 25 (3 of 6 cases) | 14 agree | 0.560 [0.371, 0.733] | 0.363 | 2 of 2 found, 0 refuted | 4 of 10 (control 8) | 3 | 6.22M | fails items 0 and 3 (width 0.363); item 2 cannot pass (at most 10 < 13) |
  | docs-reviewer | control | 40 | 17 found | 0.425 [0.285, 0.578] | 0.293 | 1 of 4 found | - | 7 | 17.30M | - |
  | docs-reviewer | `sonnet`, `high` | 40 | 8 found | 0.200 [0.105, 0.348] | 0.243 | 1 of 4 found, 0 missed | - | 7 | 6.25M | fails item 2 (0.200 < 0.275) |

  * The validator items where the two arms differ in class (control, then sonnet): R 1-6 refuted, confirmed; R 5-5 refuted, unproven; R 4-2 confirmed, refuted; Mr 5-20 refuted, confirmed; Mr 1-23 confirmed, unproven; Mf 1-16 refuted, confirmed.  The control disagrees with the log on R 4-2, Mr 5-16, Mr 1-23 and Mf 1-16; sonnet on R 1-6, R 5-5, Mr 5-16, Mr 5-20 and Mr 1-23.
  * Docs-reviewer findings, blocker and should-fix: control 23, sonnet 11; nits: control 31, sonnet 21.  By case, the control and sonnet found: `f28ebff` 4 and 2 of 9, `db45675` 5 and 1 of 7, `9779b2c` 2 and 1 of 5, and `6e57ad8` 3 and 2 of 5.  In the other cases, they found: `0789e6b` 1 and 1 of 5, `2c7c29b` 1 and 0 of 5, and `9611cab` 1 and 1 of 4.  Sonnet found no item that the control missed.
  * The `haiku` arm stopped after 3 of its 6 cases (9611cab, 9c69f22, bf3334f): its 3 runs cost 6.22M, and its mean of 2.07M a run would take the total over 45M.  On those 25 items, it refuted 4 of the 10 R and Mr items, and the control refuted 8.  Six R and Mr items were left.  So it could reach at most 10, against the control's 13.
  * Tokens of the measurement: 40.85M `modelUsage` of 45M: dry runs 1.27M, validator control 6.46M, validator sonnet 3.35M, validator haiku 6.22M, docs-reviewer control 17.30M, docs-reviewer sonnet 6.25M.  A control docs-reviewer run took 0.80M to 3.66M; a sonnet one 0.34M to 1.40M.
  * Outside the 45M (each agent's final context, from its notice): 8 comparers 0.35M (the dry one included), 2 `Plan` reviews 0.31M, the implementer of step 2 0.07M, one docs lookup 0.03M.  The team leader cannot read its own tokens; the orchestrator's notice of this agent gives them.
* 2026-11-01, step 4: so no role passes.  The validator and the docs-reviewer keep `inherit`, as do the roles that the measurement did not reach (gpu-reviewer, wire-reviewer, supply-chain-reviewer) and the roles that are not candidates (reviewer, memory-reviewer, baseline).

* 2026-11-01, step 4: the scripts of the harness, as they ran, since macOS clears `/private/tmp` at a restart.  `make-harness.sh` is M3's text, with no change.  The Tripwire and Denials parts of `m5-check.sh` are the text of `replay-check.sh` in `reviews/replay.md`, from "# Tripwire, on subagent calls only." to the Denials `tq` call; they go where the line `# (replay-check.sh: Tripwire and Denials)` is.  The arm files come from `H/agents.json` with `jq '.validator += {model: "sonnet", effort: "high"}'` (or `haiku`), and `jq '."docs-reviewer" += {model: "sonnet", effort: "high"}'`.

  `H/bin/m5-clone.sh`:

  ```bash
  #!/bin/bash
  # Makes the clone of one run of harness phase 3b, M5
  # (docs/plans/active/harness-phase-3b-m5.md, step 3): H/<clone> on the branch
  # m5-<short SHA>, with only <commit> and its ancestors.  Each git command runs
  # as `git -C`, so no command runs in the main checkout.  Exits 1 if a check
  # fails.
  # Usage: m5-clone.sh <commit> <clone>
  set -euo pipefail

  repo=/Users/lloyd/Documents/rustt/stemyla
  harness=/private/tmp/stemyla-harness
  min_free_gib=30

  if [ $# -ne 2 ]; then
    echo "usage: m5-clone.sh <commit> <clone>" >&2
    exit 1
  fi
  clone_name=$2
  case $clone_name in
    "" | */* | .*)
      echo "m5-clone: the clone must be a plain folder name: $clone_name" >&2
      exit 1
      ;;
  esac
  commit=$(git -C "$repo" rev-parse --verify "$1^{commit}")
  short=$(git -C "$repo" rev-parse --short=7 "$commit")
  branch=m5-$short
  clone=$harness/$clone_name

  if [ -e "$clone" ]; then
    echo "m5-clone: $clone exists" >&2
    exit 1
  fi

  fail() {
    echo "m5-clone: check failed: $1" >&2
    exit 1
  }

  git init -q "$clone"
  git -C "$clone" fetch -q --no-tags "$repo" "$commit"
  git -C "$clone" checkout -q -b "$branch" FETCH_HEAD
  rm -f "$clone/.git/FETCH_HEAD"
  # The person's global excludes file holds Cargo.lock and CLAUDE.md, and old
  # commits do not un-ignore them.
  printf '%s\n' '!/Cargo.lock' '!/CLAUDE.md' >> "$clone/.git/info/exclude"

  head=$(git -C "$clone" rev-parse HEAD)
  [ "$head" = "$commit" ] || fail "HEAD is $head, not $commit"

  count=$(git -C "$clone" rev-list --count HEAD)
  want=$(git -C "$repo" rev-list --count "$commit")
  [ "$count" = "$want" ] || fail "the clone has $count commits, the repo $want"

  refs=$(git -C "$clone" for-each-ref --format='%(refname)')
  [ "$refs" = "refs/heads/$branch" ] || fail "the refs are not only refs/heads/$branch: $refs"

  ignored=$(git -C "$clone" ls-files -ci --exclude-standard)
  [ -z "$ignored" ] || fail "tracked files are ignored: $ignored"

  status=$(git -C "$clone" status --porcelain)
  [ -z "$status" ] || fail "the clone is not clean: $status"

  free_kib=$(df -k "$harness" | awk 'NR == 2 { print $4 }')
  [ "$free_kib" -ge $((min_free_gib * 1024 * 1024)) ] || fail "less than $min_free_gib GiB free: $free_kib KiB"

  echo "m5-clone: $clone on $branch at $head; checks pass"
  ```

  `H/bin/m5-prompt.sh`:

  ```bash
  #!/bin/bash
  # Writes the driver prompt of one run of harness phase 3b, M5, and the
  # expected Agent calls: H/prompts/<clone>.txt and H/prompts/<clone>.expected.json.
  # Usage: m5-prompt.sh <case> <arm> <clone> [<reasons file>]
  #   <case>: a row of cases.tsv, such as v-9611cab or d-f28ebff.
  #   <arm>: control, sonnet or haiku.
  #   <reasons file>: for a d- case, the docs-reviewer's reasons of the route.
  set -euo pipefail

  data=/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/2ff96c1a-0476-4e38-8735-a8e6ad4ae153/scratchpad/m5/data
  harness=/private/tmp/stemyla-harness
  case_name=$1
  arm=$2
  clone_name=$3
  reasons_file=${4:-}
  prompts=$harness/prompts
  mkdir -p "$prompts"
  target=$harness/$clone_name/target/review/target

  row=$(awk -F'\t' -v c="$case_name" '$1 == c' "$data/cases.tsv")
  [ -n "$row" ] || { echo "m5-prompt: no case $case_name" >&2; exit 1; }
  commit=$(printf '%s' "$row" | cut -f2)
  base=$(printf '%s' "$row" | cut -f3)
  since=$(printf '%s' "$row" | cut -f4)
  round=$(printf '%s' "$row" | cut -f5)
  criteria=$(printf '%s' "$row" | cut -f6)
  short=${commit:0:7}

  # Each agent as a JSON object {id, subagent_type, prompt}, one to a line.
  agents=$(mktemp)
  case $case_name in
    v-*)
      # One validator for each reviewer item, in the order of the list.
      awk -F'\t' -v c="$short" '$5 == c && $1 !~ /^M/' "${M5_GT:-$data/validator-gt.tsv}" | while IFS=$'\t' read -r stratum log date r commit_col id reviewers severity kind rule where claim verdict repeat action mutline; do
        if [ "$round" = 1 ]; then
          range="$base..HEAD"
        elif [ "$round" -le 3 ] && printf '%s' "$reviewers" | grep -Eq '(^|, )(reviewer|baseline)(,|$)'; then
          range="$base..HEAD"
        else
          range="$since..HEAD"
        fi
        prompt=$(printf 'Validate this finding as your instructions say.\n\nID: %s\nFinding:\nseverity: %s\nkind: %s\nwhere: %s\nrule: %s\nclaim: %s\nevidence: not recorded\nfix: not recorded\n\nRange: %s\nCriteria: %s\nRound: %s\nTarget folder: %s' \
          "$id" "$severity" "$kind" "$where" "$rule" "$claim" "$range" "$criteria" "$round" "$target")
        jq -cn --arg id "$id" --arg prompt "$prompt" '{id: $id, subagent_type: "validator", prompt: $prompt}'
      done > "$agents"
      lines=$(awk -F'\t' -v c="$short" '$5 == c && $1 ~ /^M/ { print $16 }' "${M5_GT:-$data/validator-gt.tsv}")
      if [ -n "$lines" ]; then
        prompt=$(printf 'Validate these missed mutants as your instructions say.  Give one block for each line, with the line as its id.\n\nMissed mutants, from cargo-mutants:\n%s\n\nRange: %s..HEAD\nCriteria: %s\nRound: %s\nTarget folder: %s' \
          "$lines" "$since" "$criteria" "$round" "$target")
        jq -cn --arg prompt "$prompt" '{id: "mutants", subagent_type: "validator", prompt: $prompt}' >> "$agents"
      fi
      ;;
    d-*)
      [ -n "$reasons_file" ] || { echo "m5-prompt: a d- case needs the reasons file" >&2; exit 1; }
      reasons=$(cat "$reasons_file")
      if [ "$round" = 1 ]; then range="$base..HEAD"; else range="$since..HEAD"; fi
      scope=""
      prompt=$(printf 'Review this commit range as your instructions say.\n\nRange: %s%s\nCriteria: %s\nReasons of the route: %s\nRound: %s\nTarget folder: %s' \
        "$range" "$scope" "$criteria" "$reasons" "$round" "$target")
      jq -cn --arg prompt "$prompt" '{id: "docs-reviewer", subagent_type: "docs-reviewer", prompt: $prompt}' > "$agents"
      ;;
    *)
      echo "m5-prompt: unknown case kind: $case_name" >&2
      exit 1
      ;;
  esac

  jq -s --arg arm "$arm" '{arm: $arm, agents: map({id, subagent_type, prompt})}' "$agents" > "$prompts/$clone_name.expected.json"
  count=$(jq '.agents | length' "$prompts/$clone_name.expected.json")
  {
    printf 'You run one step of a replay of an old review round.  Do not use the review-rounds skill.\n\n'
    printf 'Start the %s agents below with the Agent tool, all in one message, in the foreground.  Give each the subagent_type that its BEGIN line names.  Give each agent, as its prompt, the text between its BEGIN and END lines, exactly, with no change.  Start no other agent.  Do not read files, and do not run commands.\n\n' "$count"
    printf 'When each agent has ended, give the whole output of each, in the order below, each after a line "== <ID>", as your final answer.\n'
    jq -r '.agents[] | "\nBEGIN \(.id) (subagent_type \(.subagent_type))\n\(.prompt)\nEND \(.id)"' "$prompts/$clone_name.expected.json"
    printf '\nThe checks before the steps of the review-rounds skill passed at the base of this replay.  Do not run them.\n'
  } > "$prompts/$clone_name.txt"
  rm -f "$agents"
  echo "m5-prompt: $prompts/$clone_name.txt with $count agents ($arm)"
  ```

  `H/bin/m5-check.sh`:

  ```bash
  #!/bin/bash
  # Checks one run of harness phase 3b, M5
  # (docs/plans/active/harness-phase-3b-m5.md, step 3, item 4).  It reads the
  # transcript with jq only, and prints one line for each check.  The Tripwire
  # and Denials parts are those of replay-check.sh (docs/reviews/replay.md).
  # It writes the text of each Agent tool result of the main agent to
  # H/<clone>.out/<n>.txt.  For bash 3.2 (macOS).
  # Usage: m5-check.sh <clone>
  set -euo pipefail

  harness=/private/tmp/stemyla-harness
  guard=$harness/bin/read-only-bash.sh

  if [ $# -ne 1 ]; then
    echo "usage: m5-check.sh <clone>" >&2
    exit 1
  fi
  clone_name=$1
  case $clone_name in
    "" | */* | .*)
      echo "m5-check: the clone must be a plain folder name: $clone_name" >&2
      exit 1
      ;;
  esac
  clone=$harness/$clone_name
  transcript=$harness/$clone_name.jsonl
  expected=$harness/prompts/$clone_name.expected.json
  time_file=$harness/$clone_name.time
  out=$harness/$clone_name.out

  for f in "$transcript" "$expected"; do
    if [ ! -f "$f" ]; then
      echo "m5-check: no file $f" >&2
      exit 1
    fi
  done

  tq() {
    jq -s "$@" "$transcript"
  }

  echo "m5-check: clone $clone"
  echo "m5-check: transcript $transcript"

  # Result.
  tq -r '
    [.[] | select(.type == "result")] | last
    | "Result: " + (if . == null then "FAIL: no result message"
      elif .is_error == false and .subtype == "success" then "pass"
      else "FAIL: is_error \(.is_error), subtype \(.subtype)" end)'

  # Prompt: the Agent calls of the main agent are the expected ones, in order,
  # and each has a tool result that is not an error.
  tq -r --slurpfile want "$expected" '
    ($want[0].agents) as $agents
    | [.[] | select(.type == "assistant" and .parent_tool_use_id == null) | .message.content[]?
       | select(.type == "tool_use" and .name == "Agent")] as $calls
    | [.[] | select(.type == "user") | (.message.content | if type == "array" then .[] else empty end)
       | select(type == "object" and .type == "tool_result")] as $results
    | [range(0; ([$agents, $calls] | map(length) | max)) as $i
       | ($agents[$i]) as $a | ($calls[$i]) as $c
       | if $a == null then "extra call \($i + 1): \($c.input.subagent_type)"
         elif $c == null then "no call \($i + 1) (\($a.subagent_type))"
         elif $c.input.subagent_type != $a.subagent_type then "call \($i + 1) is \($c.input.subagent_type), not \($a.subagent_type)"
         elif ($c.input.prompt | sub("^\\s+"; "") | sub("\\s+$"; "")) != ($a.prompt | sub("^\\s+"; "") | sub("\\s+$"; "")) then "call \($i + 1): the prompt differs"
         elif ([$results[] | select(.tool_use_id == $c.id and .is_error != true)] | length) == 0 then "call \($i + 1): no tool result, or an error"
         else empty end] as $problems
    | "Prompt: " + (if ($problems | length) == 0 then "pass (\($calls | length) calls)" else "FAIL: " + ($problems | join("; ")) end)'

  # Arm: the model of each subagent message.  In a control run each names
  # claude-opus-5-5; in a candidate run each names the arm's model.  The keys of
  # modelUsage are printed as information only.
  tq -r --slurpfile want "$expected" '
    ($want[0].arm) as $arm
    | ({"control": "claude-opus-5-5", "sonnet": "claude-sonnet-5-5", "haiku": "claude-haiku-5-5"}[$arm]) as $model
    | ([.[] | select(.type == "result")] | last | .modelUsage // {} | keys) as $models
    | ([.[] | select(.type == "assistant" and .parent_tool_use_id != null) | .message.model // "none"]) as $sub
    | ($sub | unique) as $names
    | (if ($sub | length) == 0 then ["no subagent message"]
       elif ($sub | map(select(. == "none")) | length) == ($sub | length) then ["no subagent message names a model"]
       else [$names[] | select(. != "none" and (startswith($model) | not)) | "a subagent message names \(.)"] end) as $problems
    | "Arm: " + (if ($problems | length) == 0 then "pass (\($arm): \($sub | length) subagent messages, models \($names | join(", ")))"
        else "FAIL: " + ($problems | join("; ")) end)
      + "; modelUsage keys: \($models | join(", "))"'

  # (replay-check.sh: Tripwire and Denials)
  # check --tier calls: each Bash call, of the main agent and of the subagents,
  # with a simple command whose program is cargo or .../bin/xtask and that
  # holds `check --tier`.
  tq -r '
    [.[] | select(.type == "assistant") | . as $m | .message.content[]?
     | select(.type == "tool_use" and .name == "Bash" and (.input.command | type) == "string")
     | select([.input.command | splits("(;|&&|\\|\\||\\||\\n)")
               | sub("^\\s+"; "")
               | select(test("^(cargo|\\S*/bin/xtask)\\s") and test("check --tier"))] | length > 0)
     | "  \(if $m.parent_tool_use_id == null then "main" else "subagent \($m.parent_tool_use_id)" end): \(.input.command | gsub("\\s+"; " ") | .[0:300])"] as $calls
    | "check --tier calls: \($calls | length)", $calls[]'

  # Cost.
  echo "Cost:"
  tq -r '
    ([.[] | select(.type == "result")] | last) as $result
    | ($result.modelUsage // {}) as $usage
    | ($usage | to_entries | map({model: .key, sum: ((.value.inputTokens // 0) + (.value.outputTokens // 0) + (.value.cacheCreationInputTokens // 0) + (.value.cacheReadInputTokens // 0))})) as $per
    | "  modelUsage total: \($per | map(.sum) | add // 0)",
      ($per[] | "  modelUsage \(.model): \(.sum)"),
      "  total_cost_usd: \($result.total_cost_usd // "none")",
      ([.[] | select(.type == "assistant" and .parent_tool_use_id != null)]
       | group_by(.parent_tool_use_id)[]
       | (unique_by(.message.id) | map(.message.usage | (.input_tokens // 0) + (.output_tokens // 0) + (.cache_creation_input_tokens // 0) + (.cache_read_input_tokens // 0)) | add) as $sum
       | "  subagent \(.[0].parent_tool_use_id): \($sum) (\([.[].message.model // empty] | unique | join(",")))")'
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

  # Output: the text of each Agent tool result of the main agent.
  mkdir -p "$out"
  n=0
  for id in $(tq -r '.[] | select(.type == "assistant" and .parent_tool_use_id == null) | .message.content[]? | select(.type == "tool_use" and .name == "Agent") | .id'); do
    n=$((n + 1))
    tq -r --arg id "$id" '
      [.[] | select(.type == "user") | (.message.content | if type == "array" then .[] else empty end)
       | select(type == "object" and .type == "tool_result" and .tool_use_id == $id)
       | .content | if type == "array" then map(select(.type == "text") | .text) | join("\n") else tostring end]
      | last // ""' > "$out/$n.txt"
  done
  echo "Output: $n files in $out"
  ```

  The team leader's `make.sh` (scratchpad):

  ```bash
  #!/bin/bash
  # Writes the comparer prompt of one docs-reviewer case.
  # Usage: make.sh <short commit> <rows tsv> <run A out file> [<run B out file>]
  set -euo pipefail
  commit=$1; rows=$2; a=$3; b=${4:-}
  strip() { sed '1{/^\[Subagent hand-back\]/d;}; s/^  //' "$1" | sed '/^agentId: /,$d'; }
  echo "Compare review findings with a list of known defects.  Use only the text below.  Do not read files, and do not run commands."
  echo
  if [ -n "$b" ]; then echo "The items are defects that an old review round found and confirmed.  Run A and run B are two new reviews of the same commit range, in no set order."; else echo "The items are defects that an old review round found and confirmed.  Run A is a new review of the same commit range."; fi
  echo
  echo "The rule: an item is found by a run when a finding of that run, of severity blocker or should-fix, states the same defect at the same place, in any words.  The same place is the same file, and the same function, test or section; the line ranges of the item tell whether a finding's line is in it.  For a weak-test item, either the function under test or the test is its place.  A finding that names only the area, not the defect, does not match.  A nit does not match.  One finding can match more than one item."
  echo
  if [ -n "$b" ]; then echo "Give one block for each item:"; echo; echo "item: <the item ID>"; echo "run A: found, the matching finding's where, and a quote of its claim of at most 30 words; or missed"; echo "run B: the same"; echo "reason: one sentence"; else echo "Give one block for each item:"; echo; echo "item: <the item ID>"; echo "run A: found, the matching finding's where, and a quote of its claim of at most 30 words; or missed"; echo "reason: one sentence"; fi
  echo
  echo "Then give, for each run, the count of its blocker and should-fix findings, and the count of its nits."
  echo
  echo "Items:"
  awk -F'\t' -v c="$commit" '$1 == c { printf "\n- %s: severity %s; kind %s; where %s; claim: %s\n", $2, $3, $4, $5, $6 }' "$rows"
  echo
  echo "Run A findings:"
  echo
  strip "$a"
  if [ -n "$b" ]; then echo; echo "Run B findings:"; echo; strip "$b"; fi
  ```

  The team leader's `verdicts.sh` (scratchpad):

  ```bash
  #!/bin/bash
  # Prints "<commit>\t<item id>\t<verdict>" for each item of one validator run.
  # Usage: verdicts.sh <clone>
  set -euo pipefail
  H=/private/tmp/stemyla-harness
  S=/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/2ff96c1a-0476-4e38-8735-a8e6ad4ae153/scratchpad/m5
  clone=$1
  commit=${clone##*-}
  n=0
  for id in $(jq -r '.agents[].id' "$H/prompts/$clone.expected.json"); do
    n=$((n + 1))
    f=$H/$clone.out/$n.txt
    if [ "$id" != mutants ]; then
      v=$(grep -m1 -E '^[[:space:]*]*verdict[*:]+' "$f" | sed -E 's/^[^v]*verdict[*: ]+//; s/[^a-z-].*//')
      printf '%s\t%s\t%s\n' "$commit" "$id" "${v:-none}"
    else
      awk -F'\t' -v c="$commit" '$5 == c && $1 ~ /^M/ { print $6 "\t" $16 }' "$S/data/validator-gt.tsv" | while IFS=$'\t' read -r item line; do
        v=$(awk -v l="$line" '
          { t = $0; sub(/^[ \t*`-]*/, "", t) }
          t ~ /^id[*:]/ { x = t; sub(/^id[*: ]+`?/, "", x); sub(/`? *$/, "", x); cur = (x == l) }
          cur && t ~ /^verdict[*:]/ { x = t; sub(/^verdict[*: ]+/, "", x); sub(/[^a-z-].*/, "", x); print x; exit }' "$f")
        printf '%s\t%s\t%s\n' "$commit" "$item" "${v:-none}"
      done
    fi
  done
  ```

* 2026-11-01, step 4: the data of the scripts.  The scripts of the entry above read data files that it does not record.  This entry records them.

  The scripts name the folder `m5/` of the session scratchpad of the run, here called `S`: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/2ff96c1a-0476-4e38-8735-a8e6ad4ae153/scratchpad/m5`.  `m5-prompt.sh` reads `$S/data`, and `verdicts.sh` sets `S`.  A re-run sets `S` to its own folder.  The script text above stays as it is, because it is the record of what ran.  The three awk programs are in `S`, and the data files are in `$S/data`.

  `parse.awk`:

  ```awk
  # Parse the Findings table of a review log into TSV.  -v lg=<name>
  /^## / { inf = ($0 == "## Findings"); next }
  inf && /^\| 20/ {
    line = $0
    gsub(/\\\|/, "\001", line)
    n = split(line, c, /\|/)
    out = lg
    for (i = 2; i < n; i++) { v = c[i]; gsub(/^ +| +$/, "", v); gsub(/\001/, "\\|", v); out = out "\t" v }
    print out
  }
  ```

  `mutline.awk`:

  ```awk
  # Append column 15: the cargo-mutants line of a mutant row, or "-".
  BEGIN { FS = OFS = "\t" }
  {
    line = "-"
    if ($6 == "cargo-mutants") {
      c = $11
      if (match(c, /[A-Za-z0-9_\/.-]+\.rs:[0-9]+:[0-9]+: .*/)) line = substr(c, RSTART)
      else if (match(c, /mutant: .* \([0-9]+:[0-9]+\)$/)) {
        m = substr(c, RSTART + 8); lc = m; sub(/.* \(/, "", lc); sub(/\)$/, "", lc); sub(/ \([0-9]+:[0-9]+\)$/, "", m)
        f = $10; sub(/:.*/, "", f); line = f ":" lc ": " m
      }
    }
    print $0, line
  }
  ```

  `select.awk`:

  ```awk
  # Validator ground truth: round-robin over the six commits, in this order.
  BEGIN { FS = "\t"; OFS = "\t"; split("9611cab 9c69f22 bf3334f 720f089 6a4dcf2 0732e53", order, " ")
    q["R"] = 99; q["B"] = 99; q["C"] = 9; q["Mr"] = 10; q["Mf"] = 10 }
  {
    c = $4; if (!(c in pos)) { for (i in order) if (order[i] == c) pos[c] = i }
    if (!(c in pos) || pos[c] == "") next
    if ($13 != "none" || $14 ~ /same-as/ || !($7 == "blocker" || $7 == "should-fix")) next
    found = ($12 ~ /^confirmed/ || $12 == "pre-existing"); ref = ($12 == "refuted")
    mut = ($6 == "cargo-mutants")
    if (mut) { if ($15 == "-") next; s = found ? "Mf" : (ref ? "Mr" : "") }
    else if (ref) s = "R"; else if (found) s = ($8 == "bug") ? "B" : "C"; else s = ""
    if (s == "") next
    n[s, c]++; row[s, c, n[s, c]] = $0
  }
  END {
    split("R B C Mr Mf", ss, " ")
    for (k = 1; k <= 5; k++) { s = ss[k]; taken = 0
      for (pass = 1; pass <= 40 && taken < q[s]; pass++)
        for (i = 1; i <= 6 && taken < q[s]; i++) { c = order[i]; if (n[s, c] >= pass) { print s, row[s, c, pass]; taken++ } }
    }
  }
  ```

  These commands build the data from the logs at the PR refs.  Run them in the repo, and give each awk program and each TSV file with its path in `S`.

  ```bash
  : > rows.tsv
  for f in catchup m3 m4 m5 m6 m7 m8 m9; do git show refs/remotes/origin/pr/39:docs/reviews/iphone-crashes-2026-10-25-$f.md | awk -v lg=pr39-$f -f parse.awk >> rows.tsv; done
  git show refs/remotes/origin/pr/35:docs/reviews/feature-48k-audio.md | awk -v lg=pr35 -f parse.awk >> rows.tsv
  git show refs/remotes/origin/pr/37:docs/reviews/feature-multi-download.md | awk -v lg=pr37 -f parse.awk >> rows.tsv
  awk -f mutline.awk rows.tsv > rows15.tsv
  awk -f select.awk rows15.tsv > validator-gt.tsv
  ```

  * Row 597 of `rows.tsv` (PR #39 M6, finding 1-28) has `||` inside a code span.  `parse.awk` splits it at each `|`, so the row has 16 cells, not 14.  The team leader joined cells 11 to 13 back into the claim by hand, before `mutline.awk` ran.  The claim is ``missed mutant `&&` to `||` in the smoke check``.
  * The columns of `rows.tsv` are log, date, round, commit, finding, reviewers, severity, kind, rule, where, claim, verdict, repeat of and action.
  * `rows15.tsv` adds a 15th column: the cargo-mutants line, or `-`.
  * `validator-gt.tsv` puts the stratum in front of each row, so its columns move one place to the right.
  * The docs-reviewer items are the rows of `rows15.tsv` that agree with each of these conditions:
    * the commit is one of the seven cases;
    * the reviewers hold `docs-reviewer`;
    * the severity is blocker or should-fix;
    * the verdict starts with `confirmed`.
  * `compare/make.sh` reads the docs-reviewer items from `$S/compare/docs-items.tsv`, as `<short commit>\t<finding>\t<severity>\t<kind>\t<where>\t<claim>`.  This command, in `$S/data`, gives that file exactly (40 rows):

    ```bash
    for c in f28ebff db45675 9779b2c 6e57ad8 0789e6b 2c7c29b 9611cab; do awk -F'\t' -v c=$c 'BEGIN{OFS="\t"} $4 == c && $6 ~ /(^|, )docs-reviewer(,|$)/ && ($7 == "blocker" || $7 == "should-fix") && $12 ~ /^confirmed/ { print $4, $5, $7, $8, $10, $11 }' rows15.tsv; done > ../compare/docs-items.tsv
    ```

  * The A/B rule of the comparers: the docs-reviewer cases run in this order: `f28ebff`, `db45675`, `9779b2c`, `6e57ad8`, `0789e6b`, `2c7c29b`, `9611cab`.  In an odd place of that order, run A is the control.  In an even place, run A is the sonnet run.

  `$S/data/cases.tsv` has these rows, with a tab between the columns.  The first row of the file is the header.  The column "Dry run" is not in the file; it marks the two rows of the dry runs.

  | case | commit | base | since | round | criteria | Dry run |
  | - | - | - | - | - | - | - |
  | `v-9611cab` | `9611cab215f0d839ece5bb2e9fb007e3c8dbec2a` | `d420932c62699271dbd262b579dae797129eb056` | `d420932c62699271dbd262b579dae797129eb056` | 1 | docs/plans/active/stream-from-storage-catchup.md | no |
  | `v-9c69f22` | `9c69f221c5060f0a385e652be2128ae1d09dc1df` | `c3952ad06d6ce39a09364c3c878be13938c42895` | `f28d22decb91b6452f7f526714924baac1eeb9c1` | 2 | docs/plans/active/stream-from-storage.md#M8: saving reads from storage and writes to storage, and docs/plans/active/stream-from-storage-m8.md | no |
  | `v-bf3334f` | `bf3334f413f425fcfa428c528ec36ad00bef8d21` | `2c5296854f882afaee7e58210c91af1027675829` | `f28ebff9acc4dc4606717e25cab2b84d78ff0a04` | 5 | docs/plans/active/stream-from-storage.md#M8: saving reads from storage and writes to storage, and docs/plans/active/stream-from-storage-m8.md | no |
  | `v-720f089` | `720f08912e5512e51bb990aea5c4beba2c5f6669` | `2c5296854f882afaee7e58210c91af1027675829` | `bf3334f413f425fcfa428c528ec36ad00bef8d21` | 6 | docs/plans/active/stream-from-storage.md#M8: saving reads from storage and writes to storage, and docs/plans/active/stream-from-storage-m8.md | no |
  | `v-6a4dcf2` | `6a4dcf216ece680bb5ae882ce1aebb6f8faca12a` | `62c50aba3513063524f11ac40434c1c9d89e28b7` | `e9c3aa76c84266c81d3e6770e6bb09d06744a601` | 4 | docs/plans/completed/48k-audio.md | no |
  | `v-0732e53` | `0732e5351b131521587e7f35509f750c59b09462` | `db64d158ec8957421dd872f00a2b06187b727e68` | `db64d158ec8957421dd872f00a2b06187b727e68` | 1 | docs/plans/active/mix-download.md | no |
  | `d-f28ebff` | `f28ebff9acc4dc4606717e25cab2b84d78ff0a04` | `2c5296854f882afaee7e58210c91af1027675829` | `d88811bccb03824d0e530d29be06f1472b4b0709` | 4 | docs/plans/active/stream-from-storage.md#M8: saving reads from storage and writes to storage, and docs/plans/active/stream-from-storage-m8.md | no |
  | `d-db45675` | `db456759ca31e7844e53089d213c22de1d4d263d` | `c4f3d6a3aa93140485792ef42560cb6e4e9b7af7` | `c4f3d6a3aa93140485792ef42560cb6e4e9b7af7` | 1 | docs/plans/active/stream-from-storage.md#M7: lanes, spectrograms and classic drums from storage, and docs/plans/active/stream-from-storage-m7.md | no |
  | `d-9779b2c` | `9779b2c04d37542538d09b635afdfa1d784bf638` | `62c50aba3513063524f11ac40434c1c9d89e28b7` | `62c50aba3513063524f11ac40434c1c9d89e28b7` | 1 | docs/plans/active/48k-audio.md | no |
  | `d-6e57ad8` | `6e57ad8eb1f099386aef7ffda9fdea2b357c2d2a` | `83dfdde275c00b3cd1caaee690872b8b5d2a515c` | `754d8be769abceccb9d973da564dbdc41f204431` | 2 | docs/plans/active/stream-from-storage.md#M5: a song decodes from storage into storage, and docs/plans/active/stream-from-storage-m5.md | no |
  | `d-0789e6b` | `0789e6b85e3ba6ed8ec7ea2f01a54ae69b4aea36` | `a6ac21b760abff8e1b48d783a387e6892bdc5221` | `a6ac21b760abff8e1b48d783a387e6892bdc5221` | 1 | docs/plans/active/stream-from-storage.md#M9: f16 weights on the GPU, and docs/plans/active/stream-from-storage-m9.md | no |
  | `d-2c7c29b` | `2c7c29be5ccdb5ee79b71315a501545f6b75eb3f` | `2c5296854f882afaee7e58210c91af1027675829` | `2c5296854f882afaee7e58210c91af1027675829` | 1 | docs/plans/active/stream-from-storage.md#M8: saving reads from storage and writes to storage, and docs/plans/active/stream-from-storage-m8.md | no |
  | `d-9611cab` | `9611cab215f0d839ece5bb2e9fb007e3c8dbec2a` | `d420932c62699271dbd262b579dae797129eb056` | `d420932c62699271dbd262b579dae797129eb056` | 1 | docs/plans/active/stream-from-storage-catchup.md | no |
  | `v-3ac5db4` | `3ac5db4d4415961f8ef0c19cb8f31f8748b0f7fb` | `62c50aba3513063524f11ac40434c1c9d89e28b7` | `9779b2c04d37542538d09b635afdfa1d784bf638` | 2 | docs/plans/active/48k-audio.md | yes |
  | `d-e9c3aa7` | `e9c3aa76c84266c81d3e6770e6bb09d06744a601` | `62c50aba3513063524f11ac40434c1c9d89e28b7` | `3ac5db4d4415961f8ef0c19cb8f31f8748b0f7fb` | 3 | docs/plans/active/48k-audio.md | yes |

* 2026-11-01, M5 step 5, implementer: in a worktree at `819066d` with the new test, the plant `model: sonnet` in `docs-reviewer.md` failed `the_agent_models_are_the_measured_ones` on its assert.  The message was: "`.claude/agents/docs-reviewer.md` has the model lines `["sonnet"]`, not one `model: inherit`".  The first run in the main checkout also failed, on `.gitkeep`, which has no row.  So the test now skips `IGNORED_IN_AGENTS`, as `agents_folder_problems` does.
* 2026-11-01, M5 review round 1, implementer: on the old code, `one_of_names_one_value_and_two_values` failed on its first assert.  `one_of(&["x"])` gave `` " and `x`" ``, not `` "`x`" ``.

## Decision log

* 2026-11-01, agent: a run starts only the agents under test, with a driver prompt, not the whole `review-rounds` skill.  Reason: a whole round starts each routed reviewer, the validators and the mutants run, about 6M to 17M tokens a case (`reviews/replay.md`).  The parent's budget counts only the role under test (a docs-reviewer about 1.5M, a validator 0.06M to 0.37M).
* 2026-11-01, agent: the note that the checks before the steps passed goes at the end of the driver prompt, since a driver prompt has no flags.
* 2026-11-01, agent: the validator's ground truth comes from the six cases of the six refuted reviewer findings.  Reason: fewer clones and runs, and each case gives items of each stratum.
* 2026-11-01, agent: the candidate arms set `effort: high`.  Reason: the control ran at `high`, and a Sonnet subagent with no effort key takes the person's session effort.
* 2026-11-01, agent (plan review round 1): a candidate run's session had `--effort xhigh` (replaced at checkpoint A, below).  The reviewer's template has no `Scope:` line, as in the logged rounds.  A failed control takes its case out for both arms.  The effort arms of the wire-reviewer and the supply-chain-reviewer are dropped for their cost, not for their width.
* 2026-11-01, agent (plan review round 2): each pass rule needs n = 40, with no case taken out, since the width item alone can pass a smaller n.  The validator must also confirm each found bug that the control confirmed.  A baseline row with the verdict of its same-as finding is not an item, so stratum B holds 5 and C 9.
* 2026-11-01, agent: the validator's pass rule also has decision 0025's recall line, as agreement within 0.15 of the control.  Reason: without it, a validator that refutes all but the bugs passes.
* 2026-11-01, orchestrator, at checkpoint A (`ed5e4cb`): every run, control and candidate, has `--model claude-opus-5-5 --effort high`, so the driver's effort does not differ between arms.  The candidate's agent entry keeps `effort: high`.  The comparers' tokens and the team leader's stay outside the 45M.
* 2026-11-01, agent, step 4: the `haiku` validator stopped after 3 of its 6 cases.  Reason: its mean of 2.07M a run would take the total over 45M, and item 2 of the pass rule could no longer pass (at most 10 refutations against the control's 13).  The rule of step 4 reads "the mean of that role's runs".  The validator role's mean of 1.07M would have let all 6 start.  So the team leader used the arm's own mean.
* 2026-11-01, agent, step 4: the docs-reviewer arms ran to the end, though after 4 cases sonnet was 8 items behind.  Reason: a pass was still possible by the rule until the last case.

## Outcomes and retrospective

Done on 2026-11-01.

* `agent_problems` accepts `model` from `inherit`, `opus`, `sonnet` and `haiku`, and an optional `effort` before `hooks` (`f3a4f8f`).  `the_agent_models_are_the_measured_ones` pins each agent file to the results (`d077706`).
* The measurement used 40.85M of the 45M `modelUsage` tokens.  No candidate passed its pass rule:
  * the `sonnet` validator refuted 11 of 16 refuted items, against the control's 13;
  * the `sonnet` docs-reviewer found 8 of 40 items, against the control's 17, more than 0.15 below;
  * the `haiku` validator stopped after 3 of 6 cases, and could not pass item 2.
* So each agent keeps `model: inherit`.  The gpu-reviewer, wire-reviewer and supply-chain-reviewer were not measured, for want of items or budget.  TD-71 records the gaps.
* The cheaper arms used about half the tokens: `sonnet` validator 3.35M against 6.46M, `sonnet` docs-reviewer 6.25M against 17.30M.  The loss in recall was larger than the saving allows.
* 3 review rounds, 9 round agents, about 0.57M final-context tokens, 21.1 wall minutes, 0 interventions.

What to do differently:

* Keep the data of a measurement with its scripts from the start (round 1, 1-1).  A scratch folder under `/private/tmp` is gone at a restart.
* Check each count of a results table against the ground truth before the checkpoint (round 2, 2-1).
* A driver prompt that starts only the agents under test cost 0.14M to 4.0M a run, and made the budget fit.  A whole skill run would not have fit.
