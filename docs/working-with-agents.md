# Working with agents

How to get work done in this repo with an AI coding agent (Claude Code).  This guide is for anyone who directs the agent: the maintainer, a contributor, or a team that wants to use the same approach.

The agent reads `CLAUDE.md` (the map) and the docs.  This guide is for the person: what to ask for, what the agent does, and what a person must still do.  The guide changes with each phase of `plans/active/harness-engineering.md`.  The changelog at the end records each change.  `xtask docs-check` fails if the guide does not name every `xtask` command, project skill and project subagent.

## Status

Phases 1, 2, 3 and 3b are in place:

* The map (`CLAUDE.md`) and the docs under `docs/`, with the rules by ID in `invariants.md`.
* Rules for parts of the tree in `.claude/rules/`, which load when the agent touches those files.
* `xtask docs-check`, a test that fails when the map or the docs drift from the code.
* Checks that fail a breach of a rule.  Among them: tests in `xtask/tests/rules.rs`, clippy's `disallowed-methods` and `STEMYLA_REQUIRE`.  Also the shader limits test, the UI snapshot comparison and the allocation test of the audio callback.  Most give the rule ID and the fix.  `invariants.md` names the check for each rule, or says "prose only".
* Review agents, routed by `xtask review` and run by the project skill `review-rounds` (decision 0011).  The reviews folder holds a log of the rounds of each branch.
* Phase 3b: a big task runs hands-off with the project skill `orchestrate` (decision 0023).  The review rounds follow the fix rule and the stop rule of decision 0024.  `xtask check` runs the local checks by tier (decision 0026).  `xtask clean` removes what runs leave.  A PR body follows `.github/pull_request_template.md` (decision 0027).  Each agent keeps the session's model, since no cheaper model passed the measurement of decision 0025.

The agent runs review rounds with `/review-rounds` after it implements a change, because `CLAUDE.md` tells it to.  The rounds go on until the stop rule of decision 0024 holds.  The review recipe below starts the same rounds by hand.  A person still gives evidence from real hardware, publishes and merges.  The project settings (`.claude/settings.json`) deny `xtask publish`, `gh pr merge` and a push that names `main` or `gh-pages` to the agent.  The project settings also run rustfmt on each `.rs` file in the repo that the agent edits.

Phase 3 added eight read-only review agents in `.claude/agents/`: `reviewer`, `gpu-reviewer`, `memory-reviewer`, `wire-reviewer`, `supply-chain-reviewer`, `docs-reviewer`, `validator` and `baseline` (see "Review agents" below).  `xtask review start` routes the six reviewers.  The project skill `review-rounds` (`.claude/skills/review-rounds/SKILL.md`) starts the agents in each round, and runs cargo-mutants (see "Review a change").

## Before you start

1. Install Claude Code ([docs](https://code.claude.com/docs)) and start it at the repo root.  It loads `CLAUDE.md` itself.  It reads the project settings only from the folder where it starts.  The rustfmt hook needs `jq` and `rustfmt` on `PATH`; macOS 26 has `/usr/bin/jq`.
2. Set up commit signing with a GPG or SSH key ([GitHub docs](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits)).  The agent signs every commit (GIT-2), so a commit fails without a key.
3. To run browser tests, set up the machine as `README.md` (setting up a machine) tells you.
4. Read `CLAUDE.md` once.  It is short, and it is what the agent works from.

## How a task flows

1. Say what you want and why, in one or two sentences.  Name the result, not the steps.
2. A small task: the agent does it, runs the checks and commits.  A larger task: the agent writes a plan first.  A big task (decision 0023): the agent sends one message up front with orchestrated mode and each design or implementation choice that hinges on you, with options and a recommendation.  Then it runs the plan hands-off with the `orchestrate` skill.
3. Review the plan before the code.  One wrong line in a plan becomes many wrong lines of code.
4. The agent makes small signed commits, then runs review rounds with `/review-rounds` until the stop rule of decision 0024 holds.
5. Do what only a person can do (see below).  Push, or ask the agent to push, then merge.

## Recipes

Each recipe gives a prompt to start from, what the agent does, and what to check.

### Fix a bug

* Prompt: "Fix <the bug>.  First add a test that fails because of the bug."
* The agent: reproduces the bug with a failing test, fixes it, runs `check --tier commit`, and commits.
* Check: would the new test fail if the fix were wrong?  A test that cannot fail was the most frequent finding in this repo's history (VERIFY-1).

### Add a feature or change a design

* Prompt: "Write an execution plan for <the feature> in docs/plans/active/, as docs/plans/PLANS.md says.  Do not start the work."
* The agent: writes a plan with milestones, and one command per milestone that shows it works.
* Check: read the plan and correct it.  Then: "Do the plan, one milestone at a time."
* To continue in a new session: "Resume docs/plans/active/<plan>.md."  The agent reads the plan's progress first.

### Run a big task hands-off

* A task is big when one of these holds at the start: a plan of more than 3 milestones, a decision point in more than one milestone, work longer than one session, or you are away for the run (decision 0023).
* Before the start: a signing cache that lasts the run, `caffeinate -dims`, and `autoContinueAtUsageLimit` on.  Give the `autoMode.allow` entries that the plan's "How the work runs" section names in `~/.claude/settings.json`.  Start `claude --permission-mode auto` at the repo root.
* Prompt: "Do docs/plans/active/<plan>.md."  The agent loads the `orchestrate` skill, does the preflight, and sends you one message: the mode, the branch and PR, and each design or implementation choice that hinges on you.  Each choice has its options and a recommendation.  Answer it once.
* The agent: as orchestrator, it starts one team for each milestone, on one branch and one draft PR.  Each team writes a milestone plan, implements it and runs review rounds until the stop rule holds.  Between milestones, the orchestrator waits for CI, runs `cargo run -p xtask -- clean --yes` and updates the PR body.  It stops only for the reasons of the skill's "Stop and wait", or of the plan's section.
* To pause: ask the orchestrator to stop at a clean point, and to write the steps of the resume in the plan's Progress.  To resume, give a new session those steps.  The `orchestrate` skill has no pause or resume step of its own, and its "Start" makes the branch and the PR (TD-78).  On 2026-11-01, a run paused after a review round and resumed this way (`d376200`).
* Check: the PR's "Needs your attention", each decision that an agent made alone (the plan's Decision log), and the scratch folders that the close-out lists for `clean --yes --also`.
* Inline comments: at close-out, the orchestrator posts one review with event `COMMENT`, with at most 10 inline comments (decision 0027).  Each comment says what to decide and the default that the agent took.  The orchestrator picks them from the list of `cargo run -p xtask -- attention <base> <plan>`, the hand-run lines first.  "Needs your attention" links each item to its comment, where one exists.

### Change a GPU kernel

* Prompt: as for a feature.  The agent loads `.claude/rules/gpu.md` when it touches `roformer-gpu/` or `roformer/src/`.
* The agent: keeps the kernel inside the WebGPU limits (GPU-1) and tests it against the CPU (GPU-2).
* Check, on a machine with a real GPU: `STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release` (`gpu` alone on an adapter without `SHADER_F16_IN_F32`), then `cargo run -p xtask -- e2e selftest --weights <model> --query mm=<kernel>`.  Put the adapter name in the PR.  A pass on SwiftShader alone is not enough (GPU-3).

### Ask for evidence that a test really ran

* A test that has no GPU adapter, no `timestamp-query` or no PyTorch fixture prints "skipping" and passes.  `cargo test` hides that line.
* Prompt: "Run it with STEMYLA_REQUIRE=gpu" (or `gpu,fixtures`, `gpu,timestamps` or `gpu,f16`).  Then each such skip is a failure that names VERIFY-1.
* Check: the PR gives the command with `STEMYLA_REQUIRE`, and names the adapter.

### Change memory, buffers or audio playback

* The agent loads `.claude/rules/memory.md` when it touches `engine/`, `app/src/` or `worker/`.
* Check: does the change copy a song- or model-sized buffer (MEM-1)?  If so, the PR must say why, and a test must pin the sharing.  Read the reason of each new `#[expect(clippy::disallowed_methods)]`.

### Change a message between the page and the worker

* The agent loads `.claude/rules/protocol.md`, and changes the hand-written e2e scripts in the same commit (PROTO-1).
* Check: `cargo test -p xtask --test rules` and `cargo run -p xtask -- check --tier commit` pass.  Then CI's browser tests pass.  CI runs `e2e storage` and `e2e flac`, which write stem worker messages by hand.  `stored-stems`, `worker-check` and `weights` also write them, but CI does not run them (TD-16).  They need a GPU and the weights.  When you change the messages of one of them, ask a person to run it on a GPU with the weights.  Name the run in the PR.  Else run an `e2e` scenario by hand only when CI failed it (decision 0026).  Check the field names by hand (TD-16).

### Review a change

* Prompt: "Run `cargo build -p xtask`, then `/review-rounds main 1 "<criteria>"`, then the next rounds until the stop rule of decision 0024 holds."  The skill's first line runs `target/debug/xtask`, which fails with exit 127 if the binary is missing.
* The skill: `/review-rounds <base> <round> "<criteria>" [--review-only] [--baseline]`.  The criteria are one section of a plan (`file#heading`), or the messages of commits (`commits:<range>`).  With `--review-only`, the agent runs `review clean` after the findings file, and then stops the round.  `--baseline` also runs the frozen `baseline` review, to compare with.
* The agent, in each round:
  1. `xtask review start` refuses a tree with changes ("commit first"), and chooses the reviewers.
  2. cargo-mutants runs in the background (`xtask review mutants`).
  3. The routed reviewers run in parallel, each as a new subagent.  From round 4, they read only the fix range and the files that it touches.  The agent merges their findings.
  4. A `validator` tries to disprove each blocker and should-fix, and each missed mutant that is not known.  The agent logs each line of `known.txt` as `known`, with its TD, and no validator.
  5. `xtask review check` stops the round if the repo changed in a way that a review must not cause.
  6. The agent writes the findings file in the run folder under `target/review/run/`.  It marks each blocker and should-fix that may be real as significant or not, in the `Significant` column of the log.  In rounds 1 to 3, it fixes each found finding, with a failing test first where the finding is about behaviour.  From round 4, a round fixes blockers, significant should-fix and every confirmed finding on a strict rule.  From round 4, it logs the other findings as `reported, not fixed`.  These are each `confirmed`, `pre-existing` or `unproven` finding of none of the three kinds, and each `unvalidated` finding.  A `pre-existing` finding can go to `docs/tech-debt.md` or its own PR.  It commits "review round <n> fixes", and adds the round to the log.  Then `xtask review log-check` checks the log against the fix rule.  The rounds stop at the first round from round 3 on that meets the stop rule of step 11 of the skill.  The stop rule counts each `confirmed` finding of the three kinds, and each `pre-existing` or `unproven` one that the round fixed or sent to a TD.
  7. After the log, the agent runs `target/debug/xtask review clean <round>`.  It removes each worktree or folder under `target/review/` whose name starts with `<round>-`.  If auto mode or a person refuses it, the agent gives the command, and goes on.  A subagent whose findings file or log the Write tool refuses gives the command to its caller.  The caller runs it after it writes the file.
* A change to a file in `.claude/agents/` reaches the agents only in a new Claude Code session.  The `Scope:` line of the skill's prompt reaches a late reviewer in any session.
* Check: the refuted findings, the unproven blockers and should-fix, each `repeat-of`, each `known` finding, each `reported, not fixed` finding, and each mutants line.  The PR description lists them.
* From the PR after phase 3's, pass `--baseline` in round 1, until the Progress of `plans/active/harness-engineering.md` records the gate.
* The skill runs `xtask review` with no approval only in the turn in which it loads.  The notices of the background reviewers and of the mutants run start new turns.  So in the default permission mode, `review check` asks for approval once in each round, and a late `review mutants` can ask too.  `review log-check` and `review clean` of step 11 also ask once in each round.  They run after the findings file, so the counts of the round do not include them.  Approve each one once, not with "don't ask again".  Step 11 of the skill counts the late review commands before the findings file in their own cell (TD-36).  "Don't ask again" changes the local settings file of Claude Code, and `review check` then fails the round.
* In a mode that can ask for approval, count the tool calls that you approve in each round.  This is the default mode, or auto mode when it falls back to a prompt.  Count from `review start` until the agent writes the findings file.  In round 1 with `--baseline`, count those for the `baseline` agent and its validators apart.  Give one total if you cannot split them.  Step 11 asks for the counts and waits: give them then (TD-38).  An orchestrator that runs the skill in a `claude -p` run, in auto mode or in `dontAsk` mode does not ask.  It writes 0 with the mode.  In a `-p` run where a host or a prompt tool answers prompts, it writes the approvals that the run records, or `not counted`.  In auto mode in an interactive session, it also marks that a fallback prompt is not counted.  The log records approvals as cost.  An intervention is a message of a person in the round, but not the message with the counts.
* If `review check` names a change, the round stops.  Examine `git reflog`, the stash and the changed files, then start the round again.
* A branch has one set of rounds and one log.  So "the run folder holds a log" from round 1 means that an earlier set ran (TD-37).  In orchestrated mode, the `orchestrate` skill gives each milestone its own run, and copies its log as `<branch>-m<n>.md`.

### Clean up after runs

* Prompt: "Run `cargo run -p xtask -- clean`, and show me the list."  Then, if the list is right: "Run it with --yes."  Fetch first (`git fetch origin`), since the proofs of a merge read `origin/main`.  After a merge, run `git switch main` and `git pull --ff-only` first, since the current branch always stays.
* The agent: the list names each item, with its size and the proof that makes it safe, or the reason to keep it.  An item is a worktree, a run folder, a local branch, build output, a pid file of `xtask serve` or a scratch root.  `--yes` removes only the items with a proof.  It also stops each running `xtask serve` of this repo.  It prints the tip of each branch that it deletes, so `git branch <name> <sha>` brings it back.
* Check: read the kept items.  A branch with "no proof that it is merged" stays until you delete it.  `xtask clean` does not stop `caffeinate`, and does not remove an untracked `tests/` folder that an agent left at the repo root: do these by hand.  A worktree with changes stays.  A worktree with ignored files outside its own `target/` stays too, and so does a worktree that holds a worktree that stays.  Warning: the unattended `clean --yes` of the orchestrator also stops an `xtask serve` that a person started in this repo.  `cargo clean` stays a manual step: `xtask clean` prints the size of `target/` only.
* The maintainer's `autoMode.allow` entry (D7 of `plans/active/harness-phase-3b.md`) covers only `cargo run -p xtask -- clean --yes`.  So a run with `--also <path>`, which names a scratch root in the temporary folder, asks for approval.

### Record a decision or a gap

* Prompt: "Record this as a proposed decision in docs/decisions/."  To accept it, change its status to `accepted` yourself; agents only propose.
* Prompt: "Add this gap to docs/tech-debt.md."  The agent does this itself when it leaves a gap.

### Run browser tests on a Mac

See `references/local-browser-testing.md`: Safari first, Chrome when safaridriver fails, and the failures that also occur on `main`.

## What a person must do

* Push, or ask the agent to push a branch; it does not push unasked, and deny rules refuse its pushes that name `main` or `gh-pages` (GIT-1).  Run `cargo run -p xtask --release -- publish`; deny rules refuse it in the forms that an agent usually writes.
* Accept or reject decisions.
* Give evidence from real hardware: a real GPU (GPU-3), Safari, an iPhone for memory (MEM-4).
* Merge.  `main` needs an approval, and refuses a direct push; the admin override works on a PR only.  Deny rules refuse `gh pr merge` in the forms that an agent usually writes.
* Decide when the agent stops because a change would break a rule.

## What deny rules refuse

Deny rules in `.claude/settings.json` refuse a push that names `main` or `gh-pages`, `git push --all`, `--branches` and `--mirror`, `xtask publish` and `gh pr merge`, in the forms that an agent usually writes (GIT-1, decision 0010).  Each can publish the site.  The agent sees "Permission to use Bash with command ... has been denied", and the message does not name the rule.  When it stops there, decide if you want the command.  Merge to `main` through a PR, not a push.

The rules match the command text, so they are not a security boundary (TD-28).  They do not see a push that names no branch, such as `git push` while on `main`.  They also refuse a pipeline that starts with a push and names `main` later, such as `git push origin x | grep main`; ask the agent to run the push without the pipe.  The agent pushes other branches when asked.

## Review agents

The agents in `.claude/agents/` review a change, and validate the findings.  Each agent is read only.  It has the tools `Read` and `Bash`, and its hook runs the guard `.claude/hooks/read-only-bash.sh` before each Bash command.  The guard refuses a command that would change the repo, a remote or a published package.  Examples are `git commit`, `git push`, `git stash`, `gh`, `cargo publish`, `xtask publish`, `xtask review` and `xtask clean`, also through `cargo run` of xtask.  The agent sees "read-only guard:" and the reason.  The guard matches text, so it stops mistakes, not an agent that tries to escape (TD-34).  Claude Code runs the hook only in a folder that a person trusts, and not in `-p` mode.

Each agent file says `model: inherit` and has no `effort`, so each agent runs on the model of the session.  The Agent calls of the skills pass no `model` or `effort`.  `the_agent_models_are_the_measured_ones` fails when an agent file and the measurement of decision 0025 do not agree (TD-71).

| Agent | When it runs |
| - | - |
| `reviewer` | in each review round, on every diff, for correctness against the criteria |
| `gpu-reviewer` | when `xtask review start` routes it: a change to the GPU kernels, the network or a GPU buffer |
| `memory-reviewer` | when routed: a change to `engine/`, `app/src/`, `worker/`, `decoder/`, `protocol/`, `roformer/src/`, `roformer-gpu/src/`, `synth/src/`, `dsp/src/`, a `clippy.toml` or `.claude/rules/memory.md`.  Also one of six copy signs, such as `to_vec`, in other Rust code, except in a `tests` folder (TD-31) |
| `wire-reviewer` | when routed: a change to the messages between the page and its workers, in `protocol/`, `worker/src/web.rs`, `worker/src/web/`, `decoder/src/`, `app/src/separator.rs`, `app/src/decoder.rs`, `app/src/decoder/`, `xtask/src/e2e/`, the e2e scripts or `.claude/rules/protocol.md`.  Also `model_url` in other Rust code |
| `supply-chain-reviewer` | when routed: a change to dependencies, the build, CI, a check in `xtask` or `.claude/` |
| `docs-reviewer` | when routed: a change to the docs but `docs/plans/` and `docs/reviews/`, or to `CLAUDE.md`, `README.md` or `.claude/rules/` |
| `validator` | for each blocker and should-fix finding of a round, and for the missed mutants of a round |
| `baseline` | in round 1 with `--baseline`, on the same commit, to compare with the reviewers; it stays frozen |

## Before you merge

* CI passes.  Every job but `publish` is a required check on `main`, including `browser tests (Chrome)`.
* The PR says what was verified, how, and what was not verified.
* The PR body follows `.github/pull_request_template.md`, and `cargo run -p xtask -- pr-body <file>` passes on it (decision 0027).
* A new rule, decision or gap is recorded, and the plan's progress is current.
* A change to the layers table in `docs/ARCHITECTURE.md` changes what LAYER-1 allows.  Review it as a design change.
* A changed snapshot PNG in `app/tests/snapshots/` is a change to the UI.  So are a `kittest.toml`, a changed `threshold` or `max_failed_pixels`, an `#[ignore]` on a snapshot test, and a removed `try_snapshot`.  Look at each one before you merge.

## Tools

The agent and a person use the same `xtask` commands (`cargo run -p xtask -- <command>`; `README.md` has the details).

| Command | What it does | Needs |
| - | - | - |
| `docs-check` | checks that the map and docs match the code.  It also checks each review log that has a `Significant` column against the fix rule of decision 0024.  It checks that the last round of such a log meets the stop rule.  It fails an active plan of more than 3 milestones with no "How the work runs" or no "Decisions for the maintainer" section (decision 0023) | nothing |
| `check --tier commit`, `check --tier push` | runs the local checks of GIT-2 by tier (decision 0026).  It checks the repo of the current folder, so run it from the tree that you want to check.  Tier `commit` reads the changed paths of one commit, and picks the checks that they need.  A commit that touches a strict path runs tier `push`.  Tier `push` runs the fmt, clippy, test, `docs-check` and Trunk build lines of `CLAUDE.md`'s Commands.  It runs the GPU test line only when its paths change GPU code, or when no path changed or is known.  A commit that a strict path or a failed graph sends to tier `push` runs the GPU test line by the same rule.  A `STEMYLA_REQUIRE` in the environment replaces `gpu,f16` in the GPU test line when one of its words is `gpu`, `f16` or `timestamps`.  Another value keeps `gpu,f16`, and the printed reason says that the value was not used.  On an adapter without `SHADER_F16_IN_F32`, run `check` with `STEMYLA_REQUIRE=gpu`.  A pass of the GPU test line counts for GPU-3 only on a real GPU, named in the PR.  `check` prints each check with its time, and stops at the first failure.  `--dry-run` prints each check that it runs or skips, with the reason, and runs nothing.  No tier runs a browser test or `publish` | the `wasm32-unknown-unknown` target, for the wasm32 clippy line; Trunk, for tier `push` (`README.md`); a GPU adapter, for the GPU test line |
| `review start`, `review check`, `review mutants`, `review clean`, `review log-check` | `review start <base> <round>` chooses the reviewers of a diff, and records the state of the repo.  `review check <round>` names each change since that record.  `review mutants <round>` runs cargo-mutants on the round's `since` diff for at most 60 minutes.  It writes the missed mutants that `KNOWN_MISSED` in `xtask/src/review.rs` matches to `known.txt`, and the rest to `missed.txt`.  The mutants line counts the known lines.  If the tree is not clean at the round's head, each line goes to `missed.txt`, and the mutants line says why.  `review clean <round>` removes each worktree or folder under `target/review/` whose name starts with `<round>-`, also with changes.  `review log-check <file>` checks a run's log against the fix rule of decision 0024.  It also checks each row whose Repeat of names an earlier row of the log.  A repeat of a `reported, not fixed` row is from round 4 or later.  It has no higher severity, no new strict rule and no new `cargo-mutants` source.  It copies the verdict, the Significant value and the action of the row.  A nit keeps `unvalidated` | cargo-mutants 27.1.0, for `review mutants` (`README.md`) |
| `clean` | lists what runs leave in the repo, with sizes, and changes nothing.  The items are worktrees and run folders under `target/review/`, the worktrees of Claude Code agents, and local branches.  They also include build output and the pid files of `serve`.  `clean --yes` removes only the items that it can prove safe.  It stops each running `serve` of this repo.  `clean --also <path>` adds a scratch root in the temporary folder | `gh`, `ps` and `lsof`; it keeps an item whose fact it cannot read |
| `pr-body` | `pr-body <file>` checks a PR body against `.github/pull_request_template.md` (decision 0027).  The part for people has the headings Summary, Needs your attention, Not verified and Verification, once each and in that order.  The summary has 1 to 150 words, and Verification has 1 to 3 lines that are not blank.  A `<details><summary>For agents</summary>` block follows.  It does not count the text of an HTML comment.  It exits 1 and prints each problem, or exits 0.  It exits 2 for a usage error or a file that it cannot read | nothing |
| `attention` | `attention <base> <plan>` lists the lines that can get an inline comment at close-out (decision 0027), and changes nothing.  It reads the diff of `<base>...HEAD`, and the Decision logs of the plan and of its milestone plans.  The classes, in order: `hand-run`, a message that `stored-stems`, `worker-check` or `weights` writes by hand (TD-16); `change`, a decision, a rule, `CLAUDE.md`, `.claude/`, CI or an allowlisted skip; `hot-spot`, a strict path of PROTO-2, MEM-1 to MEM-5 or RT-1, `app/src/lib.rs`, `protocol/src/`, an added `unsafe` or an added dependency; `judgement`, an agent entry of a Decision log.  An entry with no diff line says so; list it in the PR.  A path with no hunk, such as a pure rename or a mode change, gives a candidate with no line when it meets the `change` or `hot-spot` class.  It is never a `hand-run` candidate.  List it in the PR.  Each class reads the new path of a rename, then its old path, with the reason "renamed from <old>: <reason>".  It exits 2 for a usage error or a `<base>` that starts with `-` | nothing |
| `serve` | serves the built page (`serve app/dist --port 8080`) | `trunk build` first |
| `web-test` | browser tests (wasm-bindgen-test) in Safari or Chrome | wasm-bindgen-cli, a WebDriver |
| `e2e` | drives the built page over WebDriver (`e2e smoke`, `e2e storage`, ...) | `trunk build`, a WebDriver; some scenarios need a GPU and the model |
| `publish` | builds, commits to `gh-pages` and pushes | a person only (GIT-1) |
| `gen-song` | writes the demo song's stems, mix and ground truth | ffmpeg for MP3 (optional) |
| `stretch-bench` | compares the time-stretch methods for quality and cost | nothing |
| `perf` | measures the cost of one audio callback | nothing |
| `live-smoke` | plays 2 s on the default output device | the `cpal` feature, an audio device |
| `hpss`, `sdr`, `classic` | classic drum separation, and its score against ground truth | input audio |
| `roformer` | separates a song with the model on the CPU (slow) | the model |
| `roformer-convert` | converts a PyTorch checkpoint to safetensors | the checkpoint |
| `roformer-selftest` | writes the reference files for `?selftest` | the model |

## Changelog

* 2026-10-19, phase 1: first version.  The map, the docs, rules by path and `docs-check`.  The agent runs review rounds because `CLAUDE.md` says so; the review recipe starts them by hand.
* 2026-10-19, phase 2 (M3): `STEMYLA_REQUIRE` turns a skipped test into a failure.  The GPU recipe uses it, and a new recipe says how to ask for evidence that a test ran.
* 2026-10-20, phase 2 (M5): `crates_depend_only_downwards` compares the layers table with the dependencies between crates.  `the_page_and_the_worker_meet_only_through_stem_protocol` keeps the page side and the worker side apart.  Review a change to the table as a design change.
* 2026-10-20, phase 2 (M8): `no_python_in_the_build` and `no_audio_or_weights_are_tracked` check the git index.  A tracked file over 1 MiB fails; raise `MAX_TRACKED_BYTES` only with a reason in the PR.
* 2026-10-20, phase 2 (M9): `young_crates_are_pinned` fails a crate below 1.0 without an `=` pin; pin it, or list it in `SETTLED_CRATES` with the date of its first version, two years old or more.  `third_party_lists_what_ships` fails when `THIRD-PARTY.md` differs from its commands; run the command and copy the rows.
* 2026-10-20, phase 2 (M10): `e2e_messages_use_protocol_types` fails a message type in the e2e scripts that `stem_protocol::kind` does not have.  Fix the script, or add a type that only the scripts make to `SCRIPT_ONLY_TYPES` with a reason.
* 2026-10-21, phase 2 (M11): RT-1 forbids allocation in the audio callback after warm-up.  `the_callback_does_not_allocate_after_warm_up` names the stretcher, the callback size and the phase that allocated.  Size the buffer in `Player::with_config` or `Stretcher::warm_up`, or reuse one.
* 2026-10-21, phase 2 (M12): clippy fails `slice::to_vec` in `app`, `stem-worker`, `stem-protocol`, `engine`, `roformer` and `roformer-gpu`, and `to_vec` on a `Uint8Array` or a `Float32Array` in the first three.  CI also runs clippy for `wasm32`.  Borrow the slice, share an `engine::Samples` in `app` and `engine`, or move the buffer instead of copying it.  For a needed copy, add `#[expect(clippy::disallowed_methods, reason = "MEM-1: why")]`.  A canary in each crate fails clippy if its path is off.
* 2026-10-21, phase 2 (M13): clippy fails `wgpu::Device::create_buffer`, `wgpu::util::DeviceExt::create_buffer_init`, `StagingBelt` and `DownloadBuffer::read_buffer` in `roformer-gpu` outside `GpuBuffer`.  `Gpu`'s device is private.  Make the buffer with `Gpu::buffer` or `Gpu::upload`, or read back with `Gpu::read`.
* 2026-10-21, phase 2 (M14): deny rules in `.claude/settings.json` refuse `xtask publish`, `gh pr merge` and a push that names `main` or `gh-pages` to the agent.  Decide if you want each one; merge to `main` through a PR.  `claude_settings_hold_the_deny_rules` fails if a rule goes, or if the file could make Claude Code drop it.  Start Claude Code at the repo root, where it reads the file.
* 2026-10-21, phase 2 (M14): on GitHub, `main` refuses a direct push, also from an admin; merge through a PR.  `gh-pages` refuses a force push and deletion from everyone (GIT-3).
* 2026-10-21, phase 2 (M15): a hook runs rustfmt on each `.rs` file in the repo that the agent edits.  An edit of a crate root formats its module tree too.  The agent may see a note that the file changed, and read it again before its next edit.  `the_rustfmt_hook_formats_rust_files` fails if the hook goes or stops formatting.
* 2026-10-22, phase 2 (M17): CI compares the UI snapshots.  When you change the UI on purpose, run `UPDATE_SNAPSHOTS=1 cargo test -p app --test ui`.  Look at each changed PNG.  Commit only the PNGs of the views that you changed.  On a CI failure, the `ui-snapshots` artifact holds the diff images.
* 2026-10-22, phase 2 (M17): `xtask e2e` sizes the viewport on a read taken once the page knows the new window size, and logs each try as `viewport: ...`.  A read at once after a resize could give the old size.  It failed 2 of 6 runs in PR #30, after none in the 40 runs before.
* 2026-10-22, phase 2 (M18a): the page saves a chosen song from the browser's files, so it keeps no copy of the bytes.  An OPFS write copies a view into a whole array.  The MEM-5 tests spy on `write`, so a view fails them in Chrome too.  Use the same spy in a new test of an OPFS write.
* 2026-10-22, phase 2 (M18b): the worker encodes an MP3 from blocks of the channels, and the page keeps the transferred MP3, so neither copies a song-sized buffer.  `an_mp3_is_encoded_in_blocks` bounds an encode's peak memory with a counting allocator.  The four memory breaches that phase 2 found are closed.
* 2026-10-22, phase 2: done.  19 of 21 rules name what enforces them; MEM-4 and GPU-3 stay prose only until phase 4.  The plan is in `plans/completed/`.
* 2026-10-23, phase 3 (M1): `xtask review start <base> <round>` chooses the reviewers from the diff.  It writes the diffs, the route and a state record to the run folder under `target/review/run/`.  `xtask review check <round>` names each change since that record, such as a new commit, a stash or a changed hook.  Each failure exits 3, so a skill stops on it.
* 2026-10-23, phase 3 (M1): `xtask review mutants <round>` runs cargo-mutants on the Rust files of the round's `since` diff, for at most 60 minutes.  It uses `.cargo/mutants.toml`.  It prints one line: `full`, `partial` or `none`, with the counts.  `.cargo/mutants.toml` leaves out the files that compile for `wasm32` only.  Put `#[cfg_attr(mutants, mutants::skip)]` on a new `wasm32`-only function in a shared file (TD-33).
* 2026-10-23, phase 3 (M2): eight read-only review agents in `.claude/agents/`, with a guard on their Bash commands (see "Review agents").  The guard refuses a commit, a push, a stash, `gh` and a cargo publish, with "read-only guard:" and the reason.  The recipe "Review a change" starts reviewers by hand until M3.
* 2026-10-23, phase 3 (M3): the project skill `review-rounds` runs one review round: `/review-rounds <base> <round> "<criteria>"`.  It starts the routed reviewers and a `validator` for each serious finding, runs cargo-mutants, checks the state of the repo, and writes the findings.  `CLAUDE.md` and the recipe "Review a change" use it for rounds 1, 2 and 3.  Approve `review check` once in each round, and a late `review mutants` once, not with "don't ask again".  Run `cargo build -p xtask` before round 1.  A branch has one set of rounds (TD-37).
* 2026-10-24, phase 3 (M4): `docs-check` accepts review logs in the reviews folder under `docs`.  One index line, `reviews/`, lists each log there.  A log keeps the paths and closed IDs of its time, as a plan does.  Step 11 of `review-rounds` counts the late `xtask review` commands in their own cell, not as interventions (TD-36).  The log cannot count the approvals that the agent does not see (TD-38).
* 2026-10-24, phase 3 (M6): phase 3 is in place (decision 0011).  Step 11 of `review-rounds` asks for the approvals of the round, and of the baseline apart, and logs them as cost, in their own cells.  An intervention is a message of a person.  In a mode that can ask for approval, count your approvals (TD-38).  The guard also refuses `xtask review`, and `cargo run` of xtask with `publish` or `review`.  A change to `roformer/src/`, `roformer-gpu/src/`, `synth/src/` or `dsp/src/` now routes `memory-reviewer`.  A change to `worker/src/web/` now routes `wire-reviewer`.  The tech-debt entries that review covers name the agent that looks.
* 2026-10-30, M9 of `plans/completed/stream-from-storage.md`: `STEMYLA_REQUIRE` has the word `f16`, an adapter with `SHADER_F16_IN_F32`.  It implies `gpu`.  The GPU recipe uses `gpu,f16`, so the f16 weight tests cannot skip.
* 2026-10-31, phase 3b (M1): `xtask clean` lists what runs leave in the repo, with sizes, and changes nothing.  `xtask clean --yes` removes only the items that it can prove safe, and stops each running `xtask serve` of this repo.  `xtask serve` writes a pid file in `target/xtask-serve/` for it.  `xtask review clean <round>` removes the worktrees of one round's agents, and the `review-rounds` skill runs it after the log.  The guard refuses `xtask clean` to the review agents.  `cargo clean` stays a manual step.  The recipe "Clean up after runs" gives the prompts.
* 2026-10-31, phase 3b (M2): `xtask review mutants <round>` writes the missed mutants that `KNOWN_MISSED` in `xtask/src/review.rs` matches to `known.txt`, each with its TD.  It writes the rest to `missed.txt`.  The mutants line counts the known lines, as `, known <k>`.  The `review-rounds` skill logs each known line as `known`, with its TD, and gives it no validator.
* 2026-10-31, phase 3b (M3): the review rounds follow decision 0024.  Rounds 1 to 3 fix each found finding.  From round 4, a round fixes blockers, significant should-fix and every confirmed finding on a strict rule.  It logs each other `confirmed`, `pre-existing` or `unproven` finding, and each `unvalidated` finding, as `reported, not fixed`.  The PR lists them.  The rounds stop at the first round from round 3 on with no `confirmed` finding of the three kinds, and no `pre-existing` or `unproven` one that the round fixed or sent to a TD.  From round 4, the reviewers read only the fix range and the files that it touches.  The log has a `Significant` column, which the agent fills.  `xtask review log-check` checks the log after each round, and `docs-check` checks the log in the reviews folder.  A finding that repeats one that went to tech debt gets the verdict `known`.
* 2026-11-01, phase 3b (M4): `xtask check --tier commit` runs the checks that the changed paths of one commit need, and prints each skip with its reason.  A commit that touches a strict path runs tier `push`.  `xtask check --tier push` runs the fmt, clippy, test, `docs-check` and Trunk build lines of `CLAUDE.md`'s Commands.  `--dry-run` prints the plan and runs nothing.  GIT-2 runs tier `commit` before each commit, and tier `push` before each push and each review round.  CI stays the authority, and the browser tests stay in CI (TD-70).
* 2026-11-01, phase 3b (M5): an agent file can say `model` (`inherit`, `opus`, `sonnet` or `haiku`) and `effort`.  A measurement on logged findings found no cheaper model that passes, so each agent keeps `model: inherit` (decision 0025).  `the_agent_models_are_the_measured_ones` pins the agent files.  Do not change an agent's model without a new measurement.
* 2026-11-01, phase 3b (M6): the project skill `orchestrate` runs a big task hands-off (decision 0023).  `CLAUDE.md` gives the sizing rule, and the recipe "Run a big task hands-off" gives the prompt.  `docs-check` fails an active plan of more than 3 milestones with no "How the work runs" section, and a copied log whose last round does not meet the stop rule.  From round 4, `review-rounds` matches a repeat of a `reported, not fixed` finding, and `review log-check` checks it.  `review clean <round>` removes each name under `target/review/` that starts with `<round>-`.
* 2026-11-01, phase 3b (M7): `.github/pull_request_template.md` gives the form of a PR body (decision 0027).  `xtask pr-body <file>` checks a body against it.  `xtask attention <base> <plan>` lists the lines that can get an inline comment at close-out, the hand-run lines first.
* 2026-11-02, phase 3b (M8): the close-out.  `harness.md` gives each part of phase 3b, the failure that it addresses and the numbers of the phase.  TD-73 to TD-78 record the gaps that the phase leaves.  The close-out proposes changes to phases 4 to 6, to the phase 3 gate and to the late review range.  The maintainer decides each one (`plans/active/harness-engineering.md`).  After a merge, run `git switch main` and `git pull --ff-only` before `cargo run -p xtask -- clean --yes`.
