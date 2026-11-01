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

Docs, in the same commit (DOC-1): `docs/tech-debt.md` and `docs/harness.md` hold no text on `model: inherit` today (`grep -rn inherit docs`).  The implementer greps again, and changes each doc that states the old rule.  Plans in `docs/plans/completed/` and logs in `docs/reviews/` stay as they are.

### Step 3: the harness, the ground truth and the pass rule (checkpoint A)

The harness:

1. Write `H/bin/make-harness.sh` with the text of M3's Progress, and run it on this repo.  Check: exit 0; `jq -e '[.[] | .hooks.PreToolUse[].hooks[].onFailure] | length == 8 and all(. == "block")' H/agents.json` prints `true`; `jq -r 'keys[]' H/agents.json` gives the eight names.
2. Make one agents file for each arm with `jq`, from `H/agents.json`:
   * `H/agents-control.json`: `H/agents.json`, with no change.
   * `H/agents-validator-sonnet.json`: the `validator` gets `"model": "sonnet", "effort": "high"`.
   * `H/agents-docs-sonnet.json`: the `docs-reviewer` gets `"model": "sonnet", "effort": "high"`.
   * Only if budget is left: `H/agents-validator-haiku.json`, with `"model": "haiku", "effort": "high"`.
   * The candidate gets `effort: high` explicitly, so the agent file that a pass gives runs at the effort that the measurement used.  Without it, the effort of a Sonnet subagent would come from the person's session (`xhigh` on this Mac).
   * A candidate run's session has `--effort xhigh`, as the person's session on this Mac, and a control run's session has `--effort high`.  If the `effort` key works, a candidate subagent runs at `high`.  If it does not, the subagent runs at `xhigh`, as the agent file would on this Mac.  So the measured arm is the arm that the agent file gives on this Mac in both cases.  No field of the stream shows a subagent's effort, so no check reads it; the tech-debt entry of step 5 names this gap.  The main agent of a run only starts the agents, so its effort does not change the result.
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
claude -p --setting-sources project,local --model claude-opus-5-5 --effort <high or xhigh> \
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
* The criteria of a case are the plan paths at its commit: for a PR #39 milestone, `docs/plans/active/stream-from-storage.md#<the milestone's heading>` and the milestone plan; for the catch-up, `docs/plans/active/stream-from-storage-catchup.md`; for PR #35, `docs/plans/active/48k-audio.md` (round 1) or `docs/plans/completed/48k-audio.md` (round 4); for PR #37, `docs/plans/active/mix-download.md`.
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

* Validator items, 40, from the six cases that hold the six refuted reviewer findings: PR #39 catch-up round 1 (`9611cab`), M8 round 2 (`9c69f22`), M8 round speed 5 (`bf3334f`), M8 round speed 6 (`720f089`), PR #35 round 4 (`6a4dcf2`) and PR #37 round 1 (`0732e53`).
  * Strata, each a blocker or should-fix with `repeat-of` none: R, each refuted finding of a reviewer or the baseline (6); B, each found finding of kind `bug` of a reviewer or the baseline (all, 5 here); C, the other found findings of a reviewer or the baseline (9); Mr, refuted findings of `cargo-mutants` only (10); Mf, found findings of `cargo-mutants` only (10).
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
* Before each run: if the tokens used, plus the estimate of each run in progress, plus the estimate of this run, are more than 45M, the run does not start, and the measurement stops with what it has.
* Before the docs-reviewer arms: if the budget left is less than 14 docs-reviewer runs at the estimate, they do not start.  With fewer than 7 cases, item 0 of the pass rule fails.  The same holds for the `haiku` arm with 6 validator runs.
* Both arms of one case run at the same time, in two clones.  At most two cases run at the same time.
* After each run: `H/bin/m5-check.sh <clone>`, and the totals in the scratch file `m5/budget.txt`.

A run counts when the checks Result, Prompt and Arm pass.  A run cut short (an error result, a usage limit, an agent with no output) runs again in a new clone, with no change, at most 3 times; its tokens count in the budget.  A Tripwire hit in a candidate run makes its items count as not found, or as not agreeing.  A Tripwire hit in a control run, or a case with 3 cut-short runs of either arm, takes the case out for both arms.  Then item 0 of the pass rule fails, so the role keeps `inherit`.  Reason: a failed control would make the candidate pass more easily.

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
* [ ] Step 3: the harness, the ground truth and the pass rule (checkpoint A).
* [ ] Step 4: the measurement (checkpoint B).
* [ ] Step 5: the agent files and their pin.
* [ ] Step 6: review rounds.

## Surprises and discoveries

* 2026-11-01: the docs of Claude Code say that a foreground subagent's text is not in the stream without `--forward-subagent-text`.  So the run command passes it.
* 2026-11-01, M5 step 2, implementer: on the old code, each of the 9 cases of `each_model_and_effort_gives_its_problems` failed on its assert.  `model: opus`, `sonnet` and `haiku` gave "not `model: inherit`".  `model: gpt` gave the old message.  Each `effort` case gave "has the key `effort`, which the check does not allow", plus its other problems.
* 2026-11-01, M5 step 2, implementer: no doc outside `docs/plans/` and `docs/decisions/` states the old `model: inherit` rule (`grep -rn inherit docs`).  The plans and decision 0025 tell the state before M5, so they stay as they are.

## Decision log

* 2026-11-01, agent: a run starts only the agents under test, with a driver prompt, not the whole `review-rounds` skill.  Reason: a whole round starts each routed reviewer, the validators and the mutants run, about 6M to 17M tokens a case (`reviews/replay.md`).  The parent's budget counts only the role under test (a docs-reviewer about 1.5M, a validator 0.06M to 0.37M).
* 2026-11-01, agent: the note that the checks before the steps passed goes at the end of the driver prompt, since a driver prompt has no flags.
* 2026-11-01, agent: the validator's ground truth comes from the six cases of the six refuted reviewer findings.  Reason: fewer clones and runs, and each case gives items of each stratum.
* 2026-11-01, agent: the candidate arms set `effort: high`.  Reason: the control ran at `high`, and a Sonnet subagent with no effort key takes the person's session effort.
* 2026-11-01, agent (plan review round 1): a candidate run's session has `--effort xhigh`, as the person's session on this Mac, so the measured arm is the arm that the agent file gives here.  The reviewer's template has no `Scope:` line, as in the logged rounds.  A failed control takes its case out for both arms.  The effort arms of the wire-reviewer and the supply-chain-reviewer are dropped for their cost, not for their width.
* 2026-11-01, agent (plan review round 2): each pass rule needs n = 40, with no case taken out, since the width item alone can pass a smaller n.  The validator must also confirm each found bug that the control confirmed.  A baseline row with the verdict of its same-as finding is not an item, so stratum B holds 5 and C 9.
* 2026-11-01, agent: the validator's pass rule also has decision 0025's recall line, as agreement within 0.15 of the control.  Reason: without it, a validator that refutes all but the bugs passes.

## Outcomes and retrospective

To be written when the plan completes.
