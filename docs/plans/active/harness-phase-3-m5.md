# Harness phase 3, M5: replay old reviews

## Purpose

This is the milestone plan for M5 of `plans/active/harness-phase-3.md`.  It gives more detail than the parent plan's M5 section, which stays the criteria.  After M5:

* Six old review rounds of this repo have run again in the test harness, with the agents and the skill of this branch.  Each ran once, with no tuning (D5).
* `docs/reviews/replay.md` holds the ground truth of each case, written and committed before any run.  It also holds the result (x of n found, for each case and in total, with a 95% interval), the cost, each reviewer's verdict counts, the found findings outside the ground truth, and the threats.
* A found finding outside the ground truth that is still on `main` has a `docs/tech-debt.md` entry, or is listed for its own PR.

M5 changes no code that ships, and no agent, skill or guard text.  A miss is recorded, and M6 goes on (D5).

## Context and orientation

The state at the start of M5, on 2026-10-24, at commit `64a23ad` on branch `ai-agent-harness-phase-3`:

* The agent files `.claude/agents/*.md`, the guard `.claude/hooks/read-only-bash.sh` and `xtask/src/review.rs` are as M3 froze them at `de34663` (`git diff --stat de34663 HEAD` on them is empty).  The skill `.claude/skills/review-rounds/SKILL.md` differs from `de34663` only by M4's change to steps 9 and 11 (6 lines added, 4 removed).  No case reaches step 11, and no check reads the new field of step 9.
* The test harness, H = `/private/tmp/stemyla-harness/`, is as M3 left it.  `H/bin/make-harness.sh <repo>` builds `xtask`, and copies it, the guard and `.cargo/mutants.toml` to `H/bin/`.  It writes `H/agents.json` (the eight agents, each with the guard as a `PreToolUse` hook, `onFailure` `block`) and the plugin skill `H/stemyla/skills/review-rounds/SKILL.md`, with the `xtask` path made absolute.  M3's Progress holds its text and the case command.
* M3's clone step, with the `Cargo.lock` fix (M3 Progress, 2026-10-23): `!/Cargo.lock` goes into the clone's `.git/info/exclude`, and `git ls-files -ci --exclude-standard` must print nothing.  The maintainer's global excludes file holds `Cargo.lock`, and no replayed commit has `!/Cargo.lock` in `.gitignore`.
* M3's settled allow list: `Read`, `Write`, `Agent`, and `Bash(...)` for `/private/tmp/stemyla-harness/bin/xtask *`, `cargo *`, `git *`, `grep *`, `find *`, `sed *`, `ls *`, `cat *`, `rm -rf target/review/*`, `head *`, `tail *`, `wc *`, `jq *`, `sort *`, `diff *` and `echo *`.
* `xtask review mutants` stops at 60 minutes when `STEMYLA_MUTANTS_MINUTES` is unset, and the skill starts it in the background with a `timeout` of 65 minutes.
* The guard refuses a command whose first word is `gh`, `curl`, `wget`, `bash`, `sh`, `zsh`, `eval` or `xargs`, with the line `read-only guard: command: <text>.  <advice>` on stderr, and exit 2.  It reads the hook's JSON on stdin (`.tool_input.command`).
* The run folder of a clone on branch `replay-<case>` is `target/review/run/replay-<case>/` in the clone.  `route.txt` there names the routed reviewers, and `1/findings.md` has an `## Agents` table with one row for each agent.
* The repo has one tag, `pre-resign-iphone-crashes`, and no `replay/` tag.  The `docs/reviews/` folder does not exist yet; `docs/README.md` has the `reviews/` line (M4).
* About 96 GiB is free on the volume.  A harness clone with its build takes about 2 GiB.

The six cases, from the parent plan's table.  The criteria of a phase 2 case are the full heading of its section in `docs/plans/active/harness-phase-2.md` at the reviewed commit, read from git on 2026-10-24:

| Case | Tag | Base | Reviewed | Old round | Fix commit | Criteria |
| - | - | - | - | - | - | - |
| `pr9` | `replay/pr9` | `11273fc` | `ba1b592` | round 1 | `c01d4cd` | `commits:11273fc..ba1b592` |
| `m9` | `replay/m9` | `2af33b1` | `59816dc` | review round 1 | `4b27829` | `docs/plans/active/harness-phase-2.md#M9: young crates are pinned and the licence list is a test (TD-7, part; DEPS-1, DOC-1; maintainer decision)` |
| `m11` | `replay/m11` | `7cd8518` | `9cd4d3e` | review round 1 | `da675e4` | `docs/plans/active/harness-phase-2.md#M11: the audio callback does not allocate after warm-up (closes TD-15; RT-1, DEPS-1, DOC-1; maintainer decision, made 2026-10-19)` |
| `m2` | `replay/m2` | `4691a18` | `c40611c` | review round 2 | `280e800` | `docs/plans/active/harness-phase-2.md#M2: every shader is checked against the default limits (closes TD-1; GPU-1)` |
| `m10` | `replay/m10` | `7a99a88` | `5bd0298` | review round 2 | `d67db75` | `docs/plans/active/harness-phase-2.md#M10: the hand-written e2e messages match the protocol (TD-16, part; PROTO-1, DOC-1)` |
| `m18a` | `replay/m18a` | `32b2731` | `c4eab24` | review round 1 | `e63469e` | `docs/plans/active/harness-phase-2.md#M18: the breaches the checks found (closes TD-12, TD-13, TD-14, TD-17; MEM-1, MEM-2, MEM-5, DOC-1; maintainer decision)` |

`ba1b592` is on the local branch `tests-improvements-2026-10-18` only, and `c40611c` on `ai-agent-harness-phase-2-ms2` only.

Terms are those of the parent plan, and:

* Case run: one `claude -p` run of the skill on one case, in a new clone `H/replay-<case>-<n>/`, with its transcript `H/replay-<case>-<n>.jsonl`.  The dry run's clone is `H/replay-m11-dry-<n>/`.
* Ground-truth item: one finding of the old round whose fix commit changed code or tests for it.  Its ID is `<case>-<n>`, such as `m11-2`.
* Found finding: a finding of a case run's `findings.md` with the verdict `confirmed` or `pre-existing`.
* Unvalidated nit: a finding of severity `nit` with the verdict `unvalidated`.  The skill sends no nit to a validator (parent plan, Decision log, 2026-10-22), so a nit is never `confirmed`.  It can match a ground-truth item of severity `nit` only.
* Outside finding: a found finding that matches no ground-truth item of its case.  An unvalidated nit that matches no item is counted, not listed.
* Subagent call: a stream-json message with a non-null `parent_tool_use_id`.

## Plan of work

### How the team works

The team leader writes this plan, runs the case runs, starts each subagent and commits Progress.  An implementer (a general-purpose subagent) writes the harness scripts and the doc changes; it stops before the team leader starts the next subagent.  The ground-truth writer, the comparers, the flag checker and the cost reader are each a new general-purpose subagent, never a fork.  No agent of M5 runs `/review-rounds` or `xtask review start` in the main checkout: a log in `target/review/run/ai-agent-harness-phase-3/` would make M6's round 1 refuse to start.  At the end, `ls target/review/run/ai-agent-harness-phase-3/log.md` finds no file.

Before each commit: `cargo fmt --all --check`, each clippy command in `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `cargo run -p xtask -- docs-check` (GIT-2).  Sign each commit with `git commit -S`.  A Progress line follows each step, each case run and each review round, in a commit before the next one starts.

The agents and the skill do not change in M5 (D5).  Only the harness (its scripts and its allow list) may change, and only after the dry run.  A transcript is read only with `jq`, and stays in H as evidence of cost.

### Step 1: the tags

`git tag replay/<case> <reviewed>` for each row of the table, in this repo, with no push.  Check: `git rev-parse replay/<case>^{commit}` gives the reviewed commit, for each.

### Step 2: the ground truth

A new subagent writes the ground truth to `docs/reviews/replay.md`, and the implementer (or the subagent) adds the doc changes below.  The subagent's prompt says:

* Do not read `.claude/agents/`, `.claude/hooks/`, `.claude/skills/`, `H/agents.json` or `H/stemyla/`.  Do not read any session transcript.
* The sources, for each case: the fix commit's message (`git show -s <fix>`), its diff (`git show --stat <fix>`, then the files it needs), and, for a phase 2 case, the Progress of `docs/plans/active/harness-phase-2.md` at the fix commit (`git show <fix>:docs/plans/active/harness-phase-2.md`).  PR #9 has no plan; its source is `c01d4cd`'s message and diff.
* For each case, list each finding of the named old round (the table's "Old round" column) whose fix commit changed code or tests for it.  For `m2` and `m10`, the Progress at the fix commit lists round 1's findings too; leave those out.  Leave out a finding whose fix changed only docs, comments or wording.  Leave out a finding that the old round recorded as refuted, or as left for later with no change.
* For each item: the ID `<case>-<n>`; the severity as recorded (`blocker`, `should-fix`, `nit`), or the severity inferred and marked `(inferred)`; the place: the file, the function or test, and its line range at the reviewed commit (the fix commit's parent, in each case); one or two sentences that state the defect, not the fix; and the evidence (the fix commit, and the file of the fix).  Then the count n for the case, and the total.
* It reports, in at most 200 words: n for each case, each item it left out with the reason, and each severity it inferred.

The file:

* A title, `# Replay of six old review rounds`, and an introduction: what M5 measures, a link to the parent plan's M5 section by path, and that the ground truth was committed before any case run (the commit).
* `## Ground truth`: one subsection for each case, with the case table's row (base, reviewed, fix commit, criteria) and a table of its items: `ID | Severity | Where | Defect | Evidence`.
* The later sections (step 6) come after, in their own commits.

Doc changes in the same commit (DOC-1): `docs/README.md`'s introduction and its `reviews/` line name `reviews/replay.md` (not a bare `replay.md`, which `docs-check` would read as `docs/replay.md`) as the record of M5's replay, beside the branch logs.

Check: `docs-check` passes.  Progress records n for each case and the total, and the commit's time (`git log -1 --format='%h %cI' -- docs/reviews/replay.md`).  The dry run's clone is made after that time.

### Step 3: the harness

The implementer:

1. Runs `H/bin/make-harness.sh /Users/lloyd/Documents/rustt/stemyla` at the current commit.  It records in Progress: exit 0; `jq -e '[.[] | .hooks.PreToolUse[].hooks[].onFailure] | length == 8 and all(. == "block")' H/agents.json` prints `true`; and `rtk proxy diff .claude/skills/review-rounds/SKILL.md H/stemyla/skills/review-rounds/SKILL.md` shows only the `xtask` path and the `--config` change.
2. Writes `H/bin/replay-clone.sh <case> <clone>` (`#!/bin/bash`, `set -euo pipefail`).  It reads the case's tag from the table above, and:
   * `git clone -q --no-local --single-branch --no-tags --branch replay/<case> /Users/lloyd/Documents/rustt/stemyla H/<clone>`, then `git checkout -q -b replay-<case>`, `git tag -d replay/<case>` (the clone writes the tag even with `--no-tags`), and `git remote remove origin`.  The clone then holds only the history up to the reviewed commit, so no agent can read the fix commit in it.
   * Adds `!/Cargo.lock` to `.git/info/exclude`, as M3's clone step does.
   * Checks, and exits 1 if one fails: `git ls-files -ci --exclude-standard` prints nothing; `git rev-parse HEAD` is the reviewed commit; `git for-each-ref` lists only `refs/heads/replay-<case>`; `git cat-file -e <fix>` fails; `git status --porcelain` prints nothing; the volume has 30 GiB free or more.
3. Writes `H/bin/replay-run.sh <case> <clone>`.  It records the start time (`date +%s`) in `H/<clone>.time`, then runs, in the clone:

   ```bash
   claude -p "/stemyla:review-rounds <base, full SHA> 1 \"<criteria>\" --review-only" \
     --setting-sources project,local --model claude-opus-5-5 --effort high \
     --agents ../agents.json --plugin-dir ../stemyla --permission-mode dontAsk \
     --allowedTools <M3's settled list> --output-format stream-json --verbose \
     < /dev/null > ../<clone>.jsonl
   ```

   It records the end time and the exit status in `H/<clone>.time`.  `STEMYLA_MUTANTS_MINUTES` is not set, so the mutants run has the default 60 minutes, as in a real round.
4. Writes `H/bin/replay-check.sh <clone>`.  It finds `<run>` as the one folder `target/review/run/*/` in the clone (none, or more than one, is a Load `FAIL`, and the other checks that need it print `FAIL` too, with no exit from `set -e`), and the reviewed commit as `head` in `<run>/1/state.json`.  It reads the transcript with `jq` only, and prints one line for each check, `pass` or `FAIL` with the reason:
   * Result: the last `result` message has `is_error` false and `subtype` `success`.
   * Load: no `isReplay` message holds `local-command-stderr`; `<run>/1/state.json` exists.
   * Plugin: `system/init` has `(.plugin_errors // []) | length == 0`.
   * Mutants: the line in `<run>/1/mutants.txt` starts `mutants: full:` or `mutants: partial:`.
   * Route: each name in `<run>/1/route.txt` (each name once; the file lists the branch's route and the since route) has a row in the `## Agents` table of `<run>/1/findings.md`, and an Agent call of that type (`subagent_type`) whose `tool_use_id` has a `system` `task_notification` with `status` `completed`, or, for a foreground call, a `tool_result` that is not `async_launched`.
   * Review only: `git rev-parse HEAD` is still the reviewed commit, and `<run>/log.md` does not exist.
   * Tripwire, on subagent calls only: (a) for each Bash `command`, the guard, run with `{"tool_input":{"command":<command>}}` on stdin, exits 2 (any refusal, which covers `gh` and `curl` in any form and any part of a compound command), and the call's tool result does not hold `read-only guard:`, so the hook did not run; (b) any tool input (`command`, `file_path`, `path`, `pattern`) holds `rustt/stemyla` (the main checkout, in any form), `$HOME`, `target/review/run`, `.claude/projects`, or `stemyla-harness/` followed by anything but the case's own clone name and `/`; (c) any tool input holds `../`: the script prints the call, and the team leader resolves the path from the clone, and counts it as a hit only if it leaves the clone (H holds M3's clones, with the whole of phase 2's Progress, and `H/replay-m11-check` holds every branch).  Each hit prints the call.
   * Denials: each `dontAsk` denial, from the `system` messages with `subtype` `permission_denied` and `decision_reason_type` `mode`, or else from `permission_denials` summed over every `result` message (each holds one turn's), with its command, for the team leader to judge (the parent plan's Decision log, 2026-10-23: a denial fails the dry run only if the prescribed command did not run after it).
   * Cost, printed, no check: the count of Agent calls by type; the tokens of each subagent (`task_notification` `usage.total_tokens`, or a foreground `tool_use_result.totalTokens`; a notice with `usage` null is a background Bash, not an agent); the main agent's tokens (input, output, cache creation and cache read, summed over the `result` messages' `usage`); the session's `total_cost_usd` (the same in each `result`, for the whole session); the wall time from `H/<clone>.time` ("no time file" if it is missing); and the count of denials.
   * Each reviewer's counts: from `findings.md`, for each source, the number of `confirmed`, `pre-existing`, `refuted`, `unproven` and `unvalidated` verdicts.
5. Tests `replay-check.sh` on M3's transcripts before the dry run: on `H/control-4.jsonl` with its clone, each check gives `pass`, and the wall time prints "no time file".  A planted hit, a copy of a subagent's Bash call in a scratch transcript with the command `git status; /usr/bin/curl -s x` and a result without the guard's reason, makes the Tripwire check fail; so do a plant with `/Users/lloyd/Documents/rustt/stemyla/docs` in a `file_path`, and one with `/private/tmp/stemyla-harness/control-4/docs` in a `path`.  Progress records the outputs.

The scripts live in H, not in the repo.  Step 6 copies their text into `docs/reviews/replay.md`, since macOS clears `/private/tmp` at a restart.

### Step 4: the dry run

The team leader clones `replay-m11-dry-1` with `replay-clone.sh`, runs `replay-run.sh` as a background Bash command, waits for it, then runs `replay-check.sh`.  The dry run passes when each check passes, and no `dontAsk` denial is of a command that the agents' method prescribes and that did not run after the denial.  The dry run does not count.

If it fails, only the harness may change: a script, or the allow list (one command that the agents' method prescribes).  The team leader records the change, makes a new clone, and runs again, at most 3 dry runs.  If a harness change is not enough, the miss follows D5: record it, and run the six cases as they are.

### Step 5: the six cases

The team leader runs the cases one at a time, in the table's order: `pr9`, `m9`, `m11`, `m2`, `m10`, `m18a`.  Each case gets a new clone `replay-<case>-1`, `replay-run.sh` in the background, and `replay-check.sh replay-<case>-<n>` after.

* A case counts only if the Result, Load and Route checks pass.  Otherwise (an error result, a usage limit, the skill not loading, a routed reviewer with no output) it runs again in a new clone `replay-<case>-<n+1>`, with no change, and Progress records why.  A cut-short run is not tuning.  After 3 cut-short runs of one case, the team leader stops the case: its items count as missed, and the report names it.
* A tripwire hit fails the case: its items count as missed in the total, and `replay.md` gives its own x/n apart.  A Mutants check that fails (the line is `none`, such as a baseline that fails on an old tree) does not stop the case; `replay.md` records it as a threat.
* Progress records, for each case: the clone, the check lines, the mutants line, the route, and the cost.

### Step 6: compare, flag, and record

1. The team leader extracts, for each counted case, the found findings and the unvalidated nits from `<run>/1/findings.md` (each `### <ID>` block whose verdict is `confirmed`, `pre-existing`, or `unvalidated` with severity `nit`), with `awk` or `sed`, into the scratchpad, in two lists.
2. A new comparer subagent for each case (in one message; six at most) gets the two lists only: the case's ground-truth table, and its found findings with its unvalidated nits, marked as such.  It reads no file.  Its prompt: mark each ground-truth item `found` (by a reviewer, by `cargo-mutants`, or both, from the finding's sources) or `missed`.  An item is found when a found finding states the same defect at the same place (the same file, and the same function or test; the item's line range tells whether a finding's line is in it), in any words.  A ground-truth item of severity `nit` may also be found by an unvalidated nit; the result marks it `found (unvalidated)`.  A finding that names only the area, not the defect, does not match.  A missed mutant matches an item that is a weak test of the same function.  Give the matching finding's ID and a quote of at most 30 words.  Then list each found finding that matches no item (the outside findings), with its ID and severity.
3. At the same time, a new flag subagent reads `.claude/agents/*.md`, `.claude/skills/review-rounds/SKILL.md` and the ground-truth section of `docs/reviews/replay.md`.  It flags any text in the agent files or the skill that names a ground-truth item: its defect, or its place together with its kind of defect.  It quotes each flag, with the file and line.  A general method, such as "check that each test fails on wrong code", is not a flag.
4. At the same time, a new cost subagent finds the old round's reviewer tokens for each case, in the old session transcripts under `~/.claude/projects/-Users-lloyd-Documents-rustt-stemyla/`, read with `jq` and `grep` only.  It finds the subagent whose prompt names the case's range (base and reviewed commits), and sums its usage.  It reports "not found" where no transcript is left, with what it searched.
5. A new checker subagent gets the outside findings of all cases, with their blocks, and checks each against `main` (`git show main:<file>`): `still on main`, `fixed on main` (with the commit, if found), or `gone` (the code no longer exists).  It reports each in one line.
6. The implementer writes the rest of `docs/reviews/replay.md`, in one commit with the Progress line:
   * `## Harness`: the commands (the text of `replay-clone.sh`, `replay-run.sh` and `replay-check.sh`), the allow list, the model and effort, and the commit whose agents and skill ran.
   * `## Result`: for each case, x/n, and the table of its items with `found` or `missed`, by whom, the matching finding's ID and the quote; the total x/n and its 95% Wilson score interval ([Wilson, 1927](https://doi.org/10.1080/01621459.1927.10502953)), to two decimals; the total again without the `found (unvalidated)` items; the bar (0.8 or more, and each `should-fix` or worse found, where only a severity as recorded counts, not an inferred one) and whether it is met.
   * `## Cost`: for each case: agents by type, tokens (subagents and main), `total_cost_usd`, wall time, `permission_denials`, the mutants line; and the old round's reviewer tokens.
   * `## Reviewers`: for each reviewer, the counts of `confirmed`, `pre-existing` and `refuted`, summed over the cases.
   * `## Outside findings`: each one, with its case, ID, severity, source, one-line claim and the checker's state.  The maintainer classes a sample of them as real or false after the PR is ready.
   * `## Flags`: the flag subagent's result.
   * `## Threats`: the parent plan's five, and each one that M5 found (a failed Mutants check, a re-run, a denial).
7. Each outside finding that is still on `main`, with a severity of `should-fix` or worse, gets a `docs/tech-debt.md` entry in the same commit, unless the team leader lists it for its own PR.  Findings of one gap share one entry.  A nit still on `main` is listed in `replay.md` only.

### Step 7: review rounds 1, 2 and 3

Each round is a new general-purpose reviewer subagent, never a repo reviewer agent type, with the manual recipe of `docs/working-with-agents.md` at `fa41c9f` (`git show fa41c9f:docs/working-with-agents.md`, recipe "Review a change").  The range is M5's commits, from `64a23ad` to `HEAD`.  The criteria are the parent plan's M5 section, and this plan.  The team leader checks each finding before a fix, and has the implementer fix the real ones, with a check where a check can show it.  A fix changes no agent, skill or guard text, and runs no case again unless the finding shows a fault of the harness that voids a case.

## Validation and acceptance

* `git tag -l 'replay/*'` lists six tags, each at its reviewed commit.
* The first commit of `docs/reviews/replay.md` (`git log --reverse --format='%h %cI' -- docs/reviews/replay.md | head -1`) is older than the dry run's clone (`stat -f %Sm` of `H/replay-m11-dry-1/.git`, recorded in Progress).
* The dry run passes step 4, or its miss is recorded.
* Each of the six cases has one counted run, with `replay-check.sh`'s output in Progress.
* `docs/reviews/replay.md` holds each section of step 6.  The parent plan's Progress gets M5's line from the orchestrator.
* `cargo run -p xtask -- docs-check` passes, and so do the commands of "How the team works".
* `ls target/review/run/ai-agent-harness-phase-3/log.md` finds no file.

## Idempotence and recovery

* To resume: read Progress, then `git log --oneline 64a23ad..HEAD`, then `ls H`.  A case with a transcript but no Progress line runs again in a new clone, since its result is not recorded.
* The clones and transcripts stay in H as evidence.  A clone is deleted only by its literal path.  The tags stay (parent plan); `git tag -d replay/<case>` removes one.
* A harness script that is lost (a restart clears `/private/tmp`) is written again from `replay.md` (after step 6) or from this plan.

## Progress

* [x] 2026-10-24: plan written.
* [x] 2026-10-24: plan review round 1, by two new reviewers (intent; feasibility, with git and `jq` probes).  No blockers; 8 significant should-fix (with overlap), 2 other should-fix, 9 nits; all checked and fixed.  The significant ones: the clone keeps its tag, so the ref check failed every case (now `git tag -d`); `permission_denials` holds one turn only (now summed, or read from `permission_denied` messages); a background Agent call always has an `async_launched` result (now a completed `task_notification`); the tripwire missed other forms of `gh` and `curl` (now any guard refusal without the guard's reason) and the sibling clones in H, which hold phase 2's Progress and the fix commits; a nit is never validated, so a ground-truth nit could not be found (now an unvalidated nit may match it, reported apart); the comparer needs line ranges; the ground truth names its old round.

## Surprises and discoveries

* 2026-10-24: `git clone --single-branch --no-tags --branch <tag>` still writes that tag in the clone.  Evidence: plan review round 1, a probe clone, where `git for-each-ref` listed `refs/tags/replay/m11`.
* 2026-10-24: each stream-json `result` holds the `permission_denials` of its own turn only.  Evidence: plan review round 1, `jq` on `H/plant-3.jsonl`: five results with 0, 0, 1, 2 and 0 denials.

## Decision log

* 2026-10-24, team leader: each clone holds only the history up to the reviewed commit (`--single-branch --no-tags`), and `replay-clone.sh` checks that the fix commit is absent.  M3's check clone of M11 held every branch, so an agent could read the old fix commit with `git log --all`.
* 2026-10-24, team leader: the criteria of a phase 2 case are the full heading text, so the reviewer finds the section in the plan at the reviewed commit.
* 2026-10-24, team leader: a tripwire hit counts the case's items as missed, so a leak cannot raise the total.
* 2026-10-24, team leader: the flag check also reads the skill, since the same agent wrote it and its text goes into the reviewers' prompts.
* 2026-10-24, team leader: an outside nit still on `main` is listed in `replay.md` only; a `should-fix` or worse gets a tech-debt entry or its own PR.
* 2026-10-24, team leader: a ground-truth nit may be found by an unvalidated nit, marked `found (unvalidated)`, and `replay.md` gives the total with and without them.  The parent plan counts only `confirmed` and `pre-existing` as found, but the skill validates no nit, so without this a ground-truth nit is a miss by design.
* 2026-10-24, team leader: the tripwire counts any guard refusal whose result lacks the guard's reason, a superset of the parent's `gh` and `curl` test, and any path into another clone in H.  M3's clones stay; deleting them would lose M3's evidence.

## Outcomes and retrospective

To be written when M5 completes.
