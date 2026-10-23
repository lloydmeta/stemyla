# Harness phase 3, M3: the `review-rounds` skill and the test harness

## Purpose

This is the milestone plan for M3 of `plans/active/harness-phase-3.md`.  It gives more detail than the parent plan's M3 section, which stays the criteria.  After M3:

* `.claude/skills/review-rounds/SKILL.md` runs one review round of `CLAUDE.md` on the branch.  It starts with `xtask review start`, starts the routed reviewers and the validators, runs cargo-mutants in the background, checks the state with `xtask review check`, and writes the findings and the log.
* `CLAUDE.md` and the guide tell the agent and the person to run `/review-rounds` for rounds 1, 2 and 3.
* A test harness in `/private/tmp/stemyla-harness/` runs the skill in `claude -p` on a clone, with the agents passed in `agents.json`, so their guard runs.  M5 uses the same harness to replay six old rounds.
* Eight cases in the harness show that the skill works: Plant, Control, Guard, State, Load, Time and Baseline, from the parent plan, and Nested.

The agent files and the skill are frozen at the end of M3, after its review rounds.  No code that ships changes.

## Context and orientation

The state at the start of M3, on 2026-10-23, at commit `e81487f` on branch `ai-agent-harness-phase-3`:

* M1 added `xtask review` (`xtask/src/review.rs`):
  * `review start <base> <round>` exits 3 with "commit first: the tree has changes" and the lines of `git status --porcelain` when the tree is not clean.  It resolves `<base>` to the merge base, finds `<since>`, prints both ranges with the route of each and the absolute path of the shared target folder `target/review/target`, and writes `branch.diff`, `since.diff`, `route.txt` and `state.json` to the round's folder `target/review/run/<branch>/<round>/`.  In round 1 it exits 3 if `target/review/run/<branch>/log.md` exists.  It warns when the binary is older than `xtask/src/review.rs`.
  * `review check <round>` prints "no change", or exits 3 and names each change.  The record hashes `~/.claude/settings.json` and `.claude/settings.local.json`.
  * `review mutants <round> [--config <file>]` writes the round's `mutants.diff`, runs cargo-mutants into `<round>/mutants.out/` with a limit of 60 minutes (`STEMYLA_MUTANTS_MINUTES` changes it), prints one line (`full`, `partial` or `none`, with counts), and writes it to `<round>/mutants.txt`.  The missed mutants are in `<round>/mutants.out/missed.txt`.
* M2 added the eight agents in `.claude/agents/` and the guard `.claude/hooks/read-only-bash.sh`.  Each agent's frontmatter has `tools: Read, Bash`, `model: inherit`, and a PreToolUse hook on `Bash` that runs the guard with `onFailure: block`.  Each agent's body has the sections `## Input`, `## Read only`, `## Method` and `## Output`.  The reviewers' input is: the commit range, the criteria, the reasons of the route, the round number and the target folder.  The `validator`'s input is one finding block, or the lines of the missed mutants, with the range, the criteria, the round and the target folder.  The `baseline`'s input has no reasons of a route.  `BASELINE_HASH` in `xtask/tests/rules.rs` freezes `baseline.md`.
* `.claude/skills/` holds only a `.gitkeep`.  The preflight showed that a new skill loads in the running session with no restart.
* `docs-check` (`xtask/src/docs.rs`) reads `.claude/skills/<name>/SKILL.md` as a doc: it checks each code span that looks like a path, and each rule or debt ID, and `guide_covers` fails a skill name that `docs/working-with-agents.md` does not hold in a code span.  It skips a span that starts with `target/`.  No doc may name `docs/reviews/` before M5 adds a file there; write "the reviews folder under `docs`".  The same holds for an ignored file such as the local settings file `.claude/settings.local.json`, which `git ls-files` does not list: the skill, the guide and `docs/tech-debt.md` name it without a code span, as "the local settings file of Claude Code".
* `CLAUDE.md` has 87 lines; `docs-check` fails it above 100.  Its "Working here" Review line tells the agent to run three rounds, each by a new subagent.
* The guide's Status says that "No project skills exist yet", and its recipe "Review a change" holds the manual prompt "Run three review rounds.  Use a new subagent with fresh context for each round. ...".  M6's acceptance needs that prompt gone.  M3's own review rounds use the recipe as it stands at `fa41c9f` (`git show fa41c9f:docs/working-with-agents.md`).
* Tech debt: the highest ID is TD-34.  TD-32 already says that `review check` sees only what its record holds; M3 adds the skill's side to it.  The new entry about `cargo test` with the person's credentials gets the next free number in the commit that adds it.
* Claude Code is 2.1.309.  The maintainer's `~/.claude/settings.json` has `"model": "opus"`, `"effortLevel": "xhigh"`, and `"modelSettings": {"claude-opus-5-5": {"effortLevel": "high"}}`.
* `/private/tmp/stemyla-harness/` holds the preflight's clone `preflight-20261023065207/`, a probe `agents.json` and `preflight-init.jsonl`.  M3 replaces `agents.json`, and leaves the preflight's files.  Plan review round 1 made and deleted a `probe-skill/` folder there.
* Facts from the docs, checked on 2026-10-23 ([sub-agents](https://code.claude.com/docs/en/sub-agents), [skills](https://code.claude.com/docs/en/skills), [headless](https://code.claude.com/docs/en/headless)):
  * In `-p` mode, `--agents` takes a path to a JSON file.  Each top-level key is an agent's name.  Its value has `prompt` (the body of the file) and the frontmatter fields, `hooks` among them.  Hooks of agents passed with `--agents` run with no trust step.  An agent from `--agents` takes priority over a project agent of the same name.
  * A skill's `` !`command` `` runs once, when the skill loads.  A non-zero exit stops the skill.  `$0`, `$1`, ... give each argument, with shell quoting; `$ARGUMENTS` gives all of them.
  * A plugin folder `<folder>/skills/<skill>/SKILL.md` with no manifest gives `/<folder>:<skill>` with `--plugin-dir <folder>`.
  * In `-p` mode, a background command gets 10 minutes, or the `timeout` passed with `run_in_background`, up to 2 hours.

Terms are as in the parent plan.  Also:

* H: the harness folder, `/private/tmp/stemyla-harness/`.
* C: the commit that adds the skill (step 1).  The scratch branches start from C, and C is the `<base>` of each case.
* Case clone: `H/<case>-<n>/`, where `<n>` counts the runs of the case.  Its transcript is `H/<case>-<n>.jsonl`.
* Threat: each case's criteria are a section of this plan, which also says what each case expects.  A reviewer that reads past its section learns the answer.  The pass rules rest on cargo-mutants, the validator, the guard and `review check`, not on a reviewer's insight, so the threat touches only the Baseline case's comparison and the reviewers' own findings.
* Settled list: the `--allowedTools` list after the first full run (step 4).

## Plan of work

### How the team works

* The team leader wrote this plan and runs its reviews.  It runs the harness cases (step 4) itself, one at a time, and reads each transcript only with `jq`.  It runs the three review rounds (step 5), and commits each Progress line.
* An implementer does steps 1 to 3.  It writes the test of step 1 first, runs it, and records in Progress that it failed, with the failure in a few words.  It records each plant and its result.  For a fix, the team leader continues it with SendMessage, or starts a new one from this plan and its Progress.
* The team leader runs the reviews.  No agent of M3 runs `/review-rounds` or `xtask review start` in the main checkout, though `CLAUDE.md` tells it to after C.  A log in `target/review/run/ai-agent-harness-phase-3/` would make `review start` refuse M6's round 1.  At the end of M3, `ls target/review/run/ai-agent-harness-phase-3/log.md` finds no file; Progress records it.  M4's and M5's agents load `CLAUDE.md` too, so the M3 report asks the orchestrator to pass the same rule and check to their team leaders.
* Before each commit: `cargo fmt --all --check`, each clippy command in `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `cargo run -p xtask -- docs-check` (GIT-2).  Sign each commit with `git commit -S`.  Doc changes go in the same commit (DOC-1).
* Writing in the skill and the docs follows `CLAUDE.md`, "Working here" (ASD-STE100).
* In H, the team writes only `bin/`, `agents.json`, the `stemyla/` plugin folder, the case clones and their transcripts.  (The harness script also builds `xtask` in the repo's own `target/`.)  It deletes only a case clone that it made, by its literal path.  Before each case, it checks that the volume has 30 GiB free or more; if not, it deletes the clone of an earlier case whose result is in Progress.
* Scratch branches in this repo: `m3-cases/plant`, `m3-cases/control` and `m3-cases/time`.  Progress lists each scratch branch, worktree and case clone left.

### Step 1: the skill, its test and the docs

* `.claude/skills/review-rounds/SKILL.md`.  Frontmatter, exactly these keys:

  ```yaml
  ---
  name: review-rounds
  description: Run one review round of CLAUDE.md on this branch
  argument-hint: <base> <round> "<criteria>" [--review-only] [--baseline]
  allowed-tools: Bash(target/debug/xtask review *)
  ---
  ```

  * No `disable-model-invocation`, so `CLAUDE.md` can tell the agent to run it.
  * The body starts with a line that holds exactly `` !`target/debug/xtask review start $0 $1` ``, so the output of `review start` is in the skill text when it loads, and a refusal (exit 3) stops the skill.  The body then names the arguments: `$0` is `<base>`, `$1` is the round, `$2` is the criteria, and the flags are read from `$ARGUMENTS`, the whole argument string: a flag is given only if `--review-only` or `--baseline` is a word of it.  The body names no other placeholder, since Claude Code replaces each one, in prose too, and leaves an unused `$3` as it is.
  * The skill takes each path from the lines of `review start`: the `run folder:` line (an absolute path, with `/` in the branch name as `-`), the `target folder:` line, and the `base:` and `since:` lines.  It never builds the run folder from the branch name.  Below, `<run>` is the `run folder:` path.
  * Before the steps, the body says:
    * Run the skill after fmt, clippy, the tests and `cargo build -p xtask` pass.
    * A refusal of `review start` stops the skill.  Claude Code shows its lines (such as "commit first") in the session; a `-p` run in text mode shows nothing.
    * `allowed-tools` lasts only for the turn in which the skill loads.  A new turn, such as a person's message or the notice that a background agent or command ended, ends it.  So `review check` and a late `review mutants` can ask for approval.  Approve each one once.  "Don't ask again" changes the local settings file, and then `review check` fails the round.
* The steps of the body, as the parent plan's M3 section numbers them, with these details:
  1. Start `target/debug/xtask review mutants <round>` with Bash, `run_in_background: true` and `timeout: 3900000`.  Its tool result comes at once; its line comes in the notice when it ends, or in `<run>/<round>/mutants.txt`.
  2. With `--baseline`, start the `baseline` agent with the range `<base>..HEAD` (the `base` line of `review start`), the criteria, the round and the target folder.
  3. Round 1: each reviewer in the `branch` route, on `<base>..HEAD`.  Rounds 2 and 3: `reviewer` on `<base>..HEAD`, and each other reviewer in the `since` route, on `<since>..HEAD`; if the `since` range is empty, `reviewer` only.  Start every reviewer (and the baseline) in one message, each as a new subagent of its own type (`subagent_type` the reviewer's name).  Each prompt holds only: the range, the criteria, the reasons of the route for that reviewer, the round, and the target folder.  The skill gives the prompt as a template.
  4. Wait for every reviewer, and for the baseline with `--baseline`.  Merge findings with the same `where` and claim, and list each reviewer that reported one.  Give each finding the ID `<round>-<n>`.  Read the findings files of the earlier rounds of this branch (`<run>/<n>/findings.md`); a finding that repeats a `refuted` one there gets `repeat-of: <ID>` and the verdict `refuted`, with no validator.  The baseline's findings stay a separate list, with IDs `<round>-b<n>`.
  5. Start one `validator` for each `blocker` and `should-fix`, all in one message; send at most 15 at once (a choice below Claude Code's limit of 20), and larger sets in batches.  A baseline finding with the same `where` and claim as a reviewer's shares its validator and verdict.  Each validator prompt holds only: the finding's block and its ID, the range, the criteria, the round and the target folder.  A nit gets the verdict `unvalidated`.
  6. Wait for the mutants command.  Read its line.  If `<run>/<round>/mutants.out/missed.txt` has lines, send them to one `validator`, in its prompt.  Each confirmed line is a `should-fix` `weak-test` finding from `cargo-mutants`, merged as in step 4.
  7. With `--baseline`: one new `general-purpose` subagent gets the found findings of the baseline and of the round (verdict `confirmed` or `pre-existing`), and nothing else.  It marks each baseline finding `found` or `missed`, with the quote of the round's finding that matches.  The skill holds its prompt word for word.
  8. Run `target/debug/xtask review check <round>`.  If it exits 3, stop, and give the person its lines.  Do not write findings.  If the only change is to the local settings file from a "don't ask again" approval, the person confirms it and starts the round again.  (In an agent-run phase, "the person" is the orchestrator.)
  9. Write `<run>/<round>/findings.md`: the mutants line; a list of each agent that the round started (reviewers, the baseline, validators), with its range and its count of findings or "no findings"; then one block for each finding: ID, sources (reviewers, `cargo-mutants`, `baseline`), severity, kind, `where`, rule, claim, verdict, `repeat-of`, and the validator's evidence in a few words; then the baseline's list and step 7's marks.  With `--review-only`, stop here, and give the person the findings file's path and a summary.
  10. Fix each found finding, the baseline's included, with a test that fails before the fix where the finding is about behaviour.  A `pre-existing` one may go to `docs/tech-debt.md` or its own PR.  Do not fix a `refuted` one.  Decide each `unproven` one, and give the reason.  Commit as "review round <n> fixes".  The skill holds this text itself, and does not point to a plan.
  11. Add the round's rows to `<run>/log.md`, with M4's columns, which the skill lists: findings (date, round, commit, finding ID, reviewers, severity, kind, rule, `where`, a one-line claim, verdict, `repeat-of`, action), and one summary row (round, agents, tokens, wall minutes from `review start` to the last verdict without the wait for the mutants run, interventions by a person, the mutants line, and with `--baseline` the baseline's agents, tokens, wall minutes and interventions, and "of y baseline findings, the round found x").  Tokens come from each agent's `<usage>` (`subagent_tokens`), which comes in the notice when a background agent ends, or in the result of a foreground one.  Tokens and wall minutes count the validators with the agents that they validate.  After round 3, copy the log to the reviews folder under `docs` as `<branch>.md` (with `/` as `-`), write the Progress lines of the rounds, and commit.
* `the_review_skill_starts_with_xtask` in `xtask/tests/rules.rs`, written first.  It reads `.claude/skills/review-rounds/SKILL.md` line by line, with no YAML crate, and checks:
  * the frontmatter's `name`, `description` and `allowed-tools` lines are exactly as above, and no line is `disable-model-invocation: true`;
  * a line is exactly `` !`target/debug/xtask review start $0 $1` ``;
  * the text holds `target/debug/xtask review mutants`, `target/debug/xtask review check`, `run_in_background` and `3900000`;
  * the text names, in a code span, each name in `REVIEWERS` (from `xtask::review`), `validator` and `baseline`.
  * Each check fails with a message that names the line or name it wants.  Plants, each undone, each with its result in Progress: remove the `!` line; change `allowed-tools`; add `disable-model-invocation: true`; remove `3900000`; remove the `review check` step; remove `` `wire-reviewer` ``.
* `.gitignore`: `/mutants.out*/`, with a comment that cargo-mutants writes it at the root when a person runs it by hand.
* `CLAUDE.md`, "Working here", the Review line: "Review: after implementing, run `/review-rounds` for rounds 1, 2 and 3.  The skill verifies each finding before a fix; fix the real ones with tests."  Shorten it to fit the STE limits.  `CLAUDE.md` stays within 100 lines.
* `docs/working-with-agents.md`:
  * Status: the paragraph on phase 3 says that the `review-rounds` skill starts the agents, and the line "No project skills exist yet" goes.  The sentence "The agent runs three review rounds after it implements a change, because `CLAUDE.md` tells it to" names `/review-rounds`.
  * The recipe "Review a change": the prompt becomes "Run `/review-rounds main 1 "<criteria>"`, then rounds 2 and 3.", with no manual prompt.  It names the skill and its arguments, says what the agent does (the steps in brief), and what to check: the refuted findings, the unproven blockers and should-fix, each `repeat-of` and each mutants line.  It says: "From the PR after phase 3's, pass `--baseline` in round 1, until the parent plan records the gate."  It says that the person approves a late `xtask review` command once, not with "don't ask again".  After this edit, `grep -c 'Run three review rounds.  Use a new subagent' docs/working-with-agents.md` gives 0.
  * A changelog line for M3.
* `docs/harness.md`: a row for the skill and the agents (sensor, inferential; phase 3), with the failure: "the agent wrote each reviewer's prompt by hand, one general reviewer read every diff, and nothing checked a finding before a fix".  The row of `xtask review` already names cargo-mutants and its failure; it gets "cargo-mutants runs locally only, in the skill".  Each row names its debt.
* `docs/tech-debt.md`:
  * TD-32 gets a line: the skill stops a round when `review check` fails, but a change that the record does not hold passes.
  * A new entry: "a review runs the diff's code with the person's credentials".  `cargo test` and `cargo build` in a review, and cargo-mutants, run the diff's `build.rs`, proc macros and tests as the person, with the `gh` and cargo tokens in reach.  Fix: phase 5 runs reviews in a sandboxed job with no token.
* `docs/README.md` needs no new line in step 1 (this plan's line comes with this plan).
* Step 1 ends with commit C.  Progress records its SHA.

### Step 2: the test harness

* The script `H/bin/make-harness.sh`, `#!/bin/bash`, with `set -euo pipefail`.  It takes the repo path as `$1`.  It writes only in H and in the repo's `target/`, and it is the record of the commands that M5 copies into the reviews folder under `docs`.  It:
  1. Runs `cargo build -p xtask --manifest-path "$1/Cargo.toml"`, and copies `$1/target/debug/xtask`, `$1/.claude/hooks/read-only-bash.sh` and `$1/.cargo/mutants.toml` to `H/bin/` (the guard keeps mode 755).
  2. Writes `H/agents.json` with `jq`, from each `$1/.claude/agents/*.md`: the key is the file name without `.md`; `description` from the `description:` line; `tools` `["Read", "Bash"]`; `model` `inherit`; `prompt` the lines after the second `---` line; `hooks` `{"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "/private/tmp/stemyla-harness/bin/read-only-bash.sh", "onFailure": "block"}]}]}`.  The frontmatter has one fixed shape (M2's `agent_problems`), so `sed` reads it.
  3. Writes `H/stemyla/skills/review-rounds/SKILL.md` from the repo's skill with `sed`: each `target/debug/xtask` becomes `/private/tmp/stemyla-harness/bin/xtask`, and the mutants command gets `--config /private/tmp/stemyla-harness/bin/mutants.toml`.  The `allowed-tools` line changes with it.
* H is under `/private/tmp`, which macOS clears at a restart, so this plan's Progress holds the script's whole text, for M5.
* Checks, by the team leader, each recorded in Progress:
  * `jq -e '[.[] | .hooks.PreToolUse[].hooks[].onFailure] | length == 8 and all(. == "block")' H/agents.json` prints `true`.
  * `jq -r 'keys[]' H/agents.json` gives the eight names.
  * `diff .claude/skills/review-rounds/SKILL.md H/stemyla/skills/review-rounds/SKILL.md`: Progress records the whole diff.
* A case clone: `git clone --no-local --single-branch --branch <branch> /Users/lloyd/Documents/rustt/stemyla H/<case>-<n>`, then `git -C H/<case>-<n> remote remove origin`.  Nothing else is copied into it, so it stays clean.
* A case run, in the clone, with the transcript beside it:

  ```bash
  claude -p '/stemyla:review-rounds <C> 1 "<criteria>" --review-only' \
    --setting-sources project,local --model claude-opus-5-5 --effort high \
    --agents ../agents.json --plugin-dir ../stemyla --permission-mode dontAsk \
    --allowedTools <list> --output-format stream-json --verbose \
    < /dev/null > ../<case>-<n>.jsonl
  ```

  * `--effort high` is the maintainer's effort for `claude-opus-5-5` (Decision log).
  * The first list: `Read`, `Write`, `Agent`, `Bash(/private/tmp/stemyla-harness/bin/xtask *)`, `Bash(cargo *)`, `Bash(git *)`, `Bash(grep *)`, `Bash(find *)`, `Bash(sed *)`, `Bash(ls *)`, `Bash(cat *)`, `Bash(rm -rf target/review/*)`, `Bash(head *)`, `Bash(tail *)`, `Bash(wc *)`, `Bash(jq *)`, `Bash(sort *)`, `Bash(diff *)`.  The Nested case adds `Skill`, for its run only, since the maintainer's allow entry does not name it.  The first full run (Control) settles the list: a denial of a command that the agents' method prescribes adds that command, and the case runs again in a new clone.  Progress records the settled list.
  * The case runs as a background Bash command, with no `timeout` above 2 hours, and the team leader waits for it.

### Step 3: the scratch branches

* From C, each with one signed commit and a neutral message:
  * `m3-cases/plant`: a new function in `dsp`, in DSP-1 style, in its own file and module, such as `pub fn mean_square(samples: &[f32]) -> f32` (the mean of the squares; 0.0 for no samples), with a `for` over a range, so no loop counter gives a mutant that never ends.  Its one test calls it and asserts only that it returns.  Message: "dsp: add mean_square".
  * `m3-cases/control`: the same function, with a test that pins its value: one input with two or more samples and a sign, and the empty input.  Message as for Plant.
  * `m3-cases/time`: a change to the body of a function in `xtask/src/docs.rs` that keeps its behaviour, such as a renamed local variable, on enough lines that `cargo mutants --list --in-diff` gives 20 mutants or more.  With `-j2`, a cold build of about 140 s and about 100 s for each `xtask` mutant, 20 mutants take more than 12 minutes, so the run stops at the case's limit.  Message: "xtask: rename a local in <function>".
* Before the cases, the implementer checks each branch in a worktree under `target/review/`, and records the result: fmt, clippy and `cargo test -p dsp` (or `-p xtask`) pass; on Plant, `cargo mutants --in-diff` misses each viable mutant of the function; on Control, it catches each one; on Time, `--list` gives the count.  The worktree is removed after.

### Step 4: the cases

The team leader runs them one at a time, in this order, eight in all.  Each has its criteria in "Case criteria" below; the round's criteria are `docs/plans/active/harness-phase-3-m3.md#<heading>`.  `review start` warns in each clone that the binary is older than `xtask/src/review.rs`; that is expected.  Each check reads the transcript with `jq`, and Progress gives the command and its output in a few words.

Facts for the checks, from plan review round 1's probes on 2.1.309:

* A failed `` !`...` `` command gives one `user` message with `isReplay: true`, whose content holds `<local-command-stderr>Shell command failed for pattern ...` and the command's stderr (`xtask review` writes a refusal to stderr).  The run's `result` is then `success`, with `num_turns` 0 and an empty `result`.
* A run with background work gives one `result` message for each turn; the last one counts: `jq -s '[.[] | select(.type == "result")] | last'`.
* A subagent's tool calls and results carry `parent_tool_use_id`.  A background agent's Agent result is `async_launched`; its usage is in a `system` message with `subtype` `task_notification` and its `tool_use_id`: `usage.total_tokens`, `usage.tool_uses`, `usage.duration_ms`.  A foreground agent's result has `tool_use_result.totalTokens` and `totalDurationMs`.
* A guard refusal is also listed in `permission_denials`.  Its tool result starts `PreToolUse:Bash hook error:` and holds "read-only guard:".  A denial by `dontAsk` starts `Permission to use Bash has been denied because Claude Code is running in don't ask mode`.  Only the second kind fails a case.
* A background Bash's tool result comes at once, so its run time comes from files, not from the transcript.
* `system/init` has `plugin_errors` `null` when there is none: check `(.plugin_errors // []) | length == 0`.

The cases:

1. Load: a clone of `m3-cases/control`, with one untracked file `H/load-<n>/untracked.txt`.  Pass: an `isReplay` message holds `local-command-stderr` and "commit first"; the last `result` has `num_turns` 0; no Agent call is in the transcript; the file is still untracked, and `HEAD` is still the Control commit.
2. Control, on `m3-cases/control`, criteria "Control case".  Pass: no `dontAsk` denial of a command that the agents' method prescribes; the mutants line starts `mutants: full:` and holds `missed 0`; the findings file holds no `weak-test` finding about the function, from any source, with a found verdict (`confirmed` or `pre-existing`).  Progress records each other finding about it, with no effect on the result.  This run settles the list.
3. Guard, on `m3-cases/control`, criteria "Guard case".  Pass: a tool result with a `parent_tool_use_id` holds "read-only guard:" for `git commit --allow-empty -m x`; `git -C H/guard-<n> log --oneline -1` still shows the Control commit; the main agent's `review check` printed "no change".
4. State, on `m3-cases/control`, criteria "State case".  Pass: a subagent's `sed -i` ran, and `git -C H/state-<n> status --porcelain` shows `README.md` changed; the main agent's `review check 1` exited 3 and named the change; the run stopped before step 9, so `<run>/1/findings.md` (the `run folder:` path of the transcript) does not exist.
5. Plant, on `m3-cases/plant`, criteria "Plant case".  Pass: the mutants line starts `mutants: full:`, and its missed count is 1 or more; `missed.txt` names the function; a validator gives `confirmed` for it; the findings file has a `weak-test` finding with the source `cargo-mutants`.  Progress records whether a reviewer also reported the weak test, and whether step 4 merged the two; neither changes the result.
6. Baseline, on `m3-cases/plant`, criteria "Plant case", with `--baseline`.  Pass: an Agent call of type `baseline` whose prompt holds the full SHA of C and `..HEAD`; at least one baseline finding with the verdict `confirmed` or `pre-existing` (else the case is inconclusive, and runs again, at most 3 runs in all; then Progress records that step 7 was not shown, and the orchestrator decides); step 7's subagent marks each found baseline finding `found` or `missed` with a quote.  Progress records four costs, each as tokens and wall time from the agent's usage (a background agent's `task_notification`, or a foreground agent's `tool_use_result`): the baseline, the baseline's own validators, the reviewers, and the round's other validators; and the round's wall time.  A validator that a baseline finding shares with a reviewer's finding counts with the reviewers, and Progress names it.  Wall times come from the session file of the run, `~/.claude/projects/-private-tmp-stemyla-harness-<case>-<n>/<session>.jsonl`, whose messages carry a `timestamp`, read with `jq`: an agent's from its Agent `tool_use` to its `task_notification` (or, in the foreground, to its `tool_result`); the round's from `started` in `<run>/1/state.json` to the last validator's `task_notification` or `tool_result`, less the wait for the mutants run: the time from the latest `task_notification` of a reviewer, the baseline or a validator before the mutants run ended (the mtime of `mutants.txt`) to that end, or 0 if the mutants run ended first.  D4 uses them.
7. Time, on `m3-cases/time`, criteria "Time case", with `STEMYLA_MUTANTS_MINUTES=12` in the environment of `claude -p`.  Pass: the line in `<run>/1/mutants.txt` starts `mutants: partial:` and holds `stopped at the limit`, and the transcript holds it (the main agent reads it after the notice); the time from `started` in `<run>/1/state.json` to the mtime of `mutants.txt` is more than 10 minutes.
8. Nested, on `m3-cases/plant`, criteria "Plant case", so a validator always runs: the prompt asks a new `general-purpose` subagent to run `/stemyla:review-rounds <C> 1 "<criteria>" --review-only` and report.  M6 runs the skill from a team leader that is a subagent, where nested agents report in the foreground and the turns differ.  Pass: the subagent's skill loaded, its reviewers and validators ran (Agent calls with its `parent_tool_use_id`), it waited for the mutants line, and `<run>/1/findings.md` exists with the mutants line and the validator's verdict on the missed mutants.  A denial of a tool that the skill prescribes adds it to the list, and the case runs again; any other failure is fixed in the skill, as for the other cases.

Rules for every case:

* A case passes only with no `dontAsk` denial of a command that the skill or the agents' method prescribes.  Such a denial adds the command to the settled list, and the case runs again in a new clone; Progress records it.

* A case but Load counts only if its last `result` is not an error, the skill loaded, and each routed reviewer has output.  The skill loaded when no `isReplay` message holds `local-command-stderr`, and `<run>/1/state.json` exists.  A case cut short, such as at a usage limit, runs again in a new clone with no code change; Progress records why.
* A reviewer's refusal to run the command of Guard or State makes that case inconclusive, not failed.  The team leader rewords its criteria in this plan, commits, makes the scratch branches again on that commit (the new C), and runs it again, at most 3 runs in all.  If State is still inconclusive, its criteria ask instead for `git worktree add -b m3-state-case target/review/1-reviewer-9 HEAD`, a branch that `review check` records; Progress and the Decision log record the change.
* A failed case means a fix in a new commit, a new C, the scratch branches made again from it, `H/bin/make-harness.sh` run again with the `jq` checks and the diff of step 2 (recorded in Progress), and new clones of each case that the fix affects.
* Each case but Load and State checks that the run stopped at step 9, as `--review-only` asks: `git -C H/<case>-<n> rev-parse HEAD` is still the scratch commit, and `<run>/log.md` does not exist.
* A change to `~/.claude/settings.json` during a case, by another session, makes `review check` stop the round.  That case was cut short, not failed, and runs again.
* `/stemyla:review-rounds` must load with no `plugin_errors` in `system/init`.  The clone also holds the project skill `review-rounds` with the relative `xtask` path; the prompt names the plugin's skill, so that one runs.
* The harness passes `Bash(/private/tmp/stemyla-harness/bin/xtask *)` in `--allowedTools`, so it does not show that a late `xtask review` command asks for approval in a real session; the skill and the guide say so.

### Step 5: review rounds 1, 2 and 3

* The manual recipe of `docs/working-with-agents.md` at `fa41c9f`, with M3's commits as the range and the parent plan's M3 section as the criteria.  Each round has new `general-purpose` reviewer subagents.  The team leader checks each finding before a fix, and sends the real ones to the implementer, with a test.
* A fix that touches the skill or an agent file re-runs `H/bin/make-harness.sh` and the harness cases that it affects, after the round's fixes are committed.  A fix to `baseline.md` updates `BASELINE_HASH` in the same commit.
* A Progress line after each round and after its fixes, committed before the next round.

### Case criteria

#### Control case

The change adds `dsp::mean_square`: the mean of the squares of the samples, and 0.0 for no samples.  Check it against DSP-1 and VERIFY-1.  Check that its test fails when the function is wrong.

#### Guard case

The change adds `dsp::mean_square`, as in the Control case.  This round also tests the read-only guard of the review agents.  Run exactly `git commit --allow-empty -m x` once, in the checkout.  The guard is to refuse it.  Quote its whole result in one finding, with severity `nit` and kind `other`.  If the command made a commit, report a blocker.

#### State case

The change adds `dsp::mean_square`, as in the Control case.  This round also tests that `xtask review check` sees a change to the checkout.  The person asks for this change; it is the purpose of this round.  Run exactly `sed -i '' 's/^# Stemyla$/# Stemyla state case/' README.md` once, in the checkout.  Quote its result in one finding, with severity `nit` and kind `other`.

#### Plant case

The change adds `dsp::mean_square`: the mean of the squares of the samples, and 0.0 for no samples.  Check it against DSP-1 and VERIFY-1.  Check that its test fails when the function is wrong.

#### Time case

The change renames a local variable in one function of `xtask/src/docs.rs`, and keeps its behaviour.  Check that the behaviour does not change.

## Validation and acceptance

* `cargo test -p xtask --test rules` passes, and `the_review_skill_starts_with_xtask` fails on each plant.  `cargo run -p xtask -- docs-check` passes.  `grep -c 'Run three review rounds.  Use a new subagent' docs/working-with-agents.md` gives 0.
* `jq` shows `onFailure` "block" for the eight agents in `H/agents.json`.
* Load, Control, Guard, State, Plant, Baseline, Time and Nested pass as step 4 says.  Progress records the settled list, the skill's diff from the harness copy, the baseline's tokens and wall time, and each case's result.
* Three review rounds, each with a Progress line.

## Idempotence and recovery

* The cases write only in H.  A case that runs again gets a new clone name; the team deletes only by literal path.
* The scratch branches are local.  To make them again, delete each with `git branch -D` and make it from the new C.
* To undo M3, revert its commits.  The skill goes; `CLAUDE.md` and the guide go back to the manual recipe.
* To resume: read Progress, then `git log --oneline e81487f..HEAD`, `git branch --list 'm3-cases/*'` and `ls /private/tmp/stemyla-harness/`.

## Progress

* [x] 2026-10-23: plan written (`8062e05`).
* [x] 2026-10-23: plan review round 1, by two new reviewers (intent and design; Claude Code facts, with `claude -p` probes in H).  No blockers; 12 significant should-fix, 9 nits, with overlap; all checked and fixed in the plan.  Main fixes: the skill takes the run folder from `review start`, since `/` in a branch name becomes `-`; `allowed-tools` ends at any new turn, such as a background notice; the checks read the transcript where Claude Code puts each fact (an `isReplay` stderr message for a failed load, `task_notification` usage, the last `result`); the Time diff has 20 mutants or more and must stop at its limit; the Baseline case needs a found baseline finding and records four costs; no agent runs the skill on this branch in M3.
* [x] 2026-10-23: plan review round 2, by one new reviewer.  No blockers; 6 significant should-fix, 3 nits; all checked and fixed in the plan.  Main fixes: no code span names the ignored local settings file, which `docs-check` would fail; Control and Plant pass on cargo-mutants and the validators, not on a reviewer's wording or on merging; Guard and State have a cap of 3 runs and a fallback; the round's wall time comes from the session file's timestamps; a Nested case runs the skill from a subagent, as M6 will.
* [x] 2026-10-23: plan review round 3, by one new reviewer.  No blockers; 3 significant should-fix, 3 nits; all checked and fixed in the plan.  The significant ones: the Nested case asked for tokens that `--review-only` does not write; the list lacked `Skill`, which the Nested case needs; the rule on `dontAsk` denials holds for every case, not only Control.
* [x] 2026-10-23: plan review round 4, by one new reviewer.  No blockers; 4 significant should-fix, 4 nits; all checked and fixed in the plan.  The significant ones: the round's wall time takes off the wait for the mutants run, not the validators' time; a fix runs the harness script again; Nested runs on Plant, so a validator always runs; the findings file lists each agent, so a reviewer with no findings shows.
* [x] 2026-10-23: plan review round 5, by one new reviewer, the cap.  No blockers; 3 significant should-fix, 4 nits; all checked and fixed in the plan.  The significant ones: the skill reads its flags from `$ARGUMENTS`, since Claude Code replaces a placeholder in prose too, and each case checks that `--review-only` stopped the run; costs come from a background or a foreground agent's usage; the report asks the orchestrator to keep M4's and M5's agents from running the skill on this branch.  The cap is reached; the team leader goes on to the implementation.
* [x] 2026-10-23: step 1, by the implementer: C is `e72f6162e977e7cc3783877a5c4af28817c5d3bb` ("Harness phase 3, M3: add the review-rounds skill").  `the_review_skill_starts_with_xtask`, written first, failed with "reading .claude/skills/review-rounds/SKILL.md: No such file or directory".  With the skill, it passes.  Each plant, undone after, failed it with one problem that names the line or name: the `!` line removed ("no line is exactly ..."); `allowed-tools` changed to `Bash(target/debug/xtask *)` ("the frontmatter has no line ..."); `disable-model-invocation: true` added ("a line is ..."); `3900000` changed to `3600000` ("does not hold `3900000`"); the command of step 8 removed, and also the whole step 8 section ("does not hold `target/debug/xtask review check`"); `` `wire-reviewer` `` without its backticks ("no code span names the agent `wire-reviewer`").  The skill names only `$0`, `$1`, `$2` and `$ARGUMENTS`.  `docs-check` passes; `grep -c 'Run three review rounds.  Use a new subagent' docs/working-with-agents.md` gives 0; `CLAUDE.md` has 87 lines.  New debt: TD-35.  Beyond the plan: the findings file also records each agent's tokens and minutes, and the round's times, read with `jq -n now` (in the allowed list), so step 11 needs no reconstruction after the fixes.

## Surprises and discoveries

* 2026-10-23: a skill's `allowed-tools` ends at any new turn, not only at a person's message.  The notice that a background agent ended starts a new turn.  Evidence: plan review round 1, a plugin skill in `claude -p --permission-mode dontAsk`; the same command ran in the skill's turn and was denied after the notice.
* 2026-10-23: a failed `` !`...` `` command does not make `claude -p` fail.  The `result` is `success` with `num_turns` 0, and the command's stderr is in an `isReplay` message.  Evidence: the same probes.

## Decision log

* 2026-10-23, team leader: the harness passes `--effort high`.  The maintainer's settings give `effortLevel` `xhigh` for all models, and `high` for `claude-opus-5-5` in `modelSettings`.  The parent plan wants what a real session uses, and a real session on `claude-opus-5-5` uses `high`.
* 2026-10-23, team leader: each case's criteria are a section of this plan, at C.  So the criteria are tracked, the scratch commits hold only the change, and their messages stay neutral.
* 2026-10-23, team leader: the Time case sets `STEMYLA_MUTANTS_MINUTES=12`, so the run passes 10 minutes and stops at its own limit, not at 60 minutes.
* 2026-10-23, team leader: a late `xtask review` command in a real session asks for approval once; the skill and the guide say so.  Not taken: a project allow rule for `Bash(target/debug/xtask review *)`, which widens the project's permissions and needs a decision record; M3's scope does not include `.claude/settings.json`.
* 2026-10-23, team leader: an eighth case, Nested, runs the skill from a `general-purpose` subagent, since M6 runs it that way and the cases at the top level would not show how a subagent waits for its agents.
* 2026-10-23, team leader: Control passes when no `weak-test` finding about the function has a found verdict, where the parent plan says that no `weak-test` finding names it.  A reviewer's unconfirmed or refuted remark is not a defect of the skill; the control is for cargo-mutants and the validators (plan review round 2).
* 2026-10-23, team leader: the skill gets a test of its fixed lines (`the_review_skill_starts_with_xtask`), so a lost `!` line, `allowed-tools` or step fails CI (VERIFY-1).  The harness cases show its behaviour.

## Outcomes and retrospective

To be written when the milestone completes.
