# Working with agents

How to get work done in this repo with an AI coding agent (Claude Code).  This guide is for anyone who directs the agent: the maintainer, a contributor, or a team that wants to use the same approach.

The agent reads `CLAUDE.md` (the map) and the docs.  This guide is for the person: what to ask for, what the agent does, and what a person must still do.  The guide changes with each phase of `plans/active/harness-engineering.md`.  The changelog at the end records each change.  `xtask docs-check` fails if the guide does not name every `xtask` command, project skill and project subagent.

## Status

Phases 1, 2 and 3 are in place:

* The map (`CLAUDE.md`) and the docs under `docs/`, with the rules by ID in `invariants.md`.
* Rules for parts of the tree in `.claude/rules/`, which load when the agent touches those files.
* `xtask docs-check`, a test that fails when the map or the docs drift from the code.
* Checks that fail a breach of a rule.  Among them: tests in `xtask/tests/rules.rs`, clippy's `disallowed-methods` and `STEMYLA_REQUIRE`.  Also the shader limits test, the UI snapshot comparison and the allocation test of the audio callback.  Most give the rule ID and the fix.  `invariants.md` names the check for each rule, or says "prose only".
* Review agents, routed by `xtask review` and run by the project skill `review-rounds` (decision 0011).  The reviews folder holds a log of the rounds of each branch.

The agent runs three review rounds with `/review-rounds` after it implements a change, because `CLAUDE.md` tells it to.  The review recipe below starts the same rounds by hand.  A person still gives evidence from real hardware, publishes and merges.  The project settings (`.claude/settings.json`) deny `xtask publish`, `gh pr merge` and a push that names `main` or `gh-pages` to the agent.  The project settings also run rustfmt on each `.rs` file in the repo that the agent edits.

Phase 3 added eight read-only review agents in `.claude/agents/`: `reviewer`, `gpu-reviewer`, `memory-reviewer`, `wire-reviewer`, `supply-chain-reviewer`, `docs-reviewer`, `validator` and `baseline` (see "Review agents" below).  `xtask review start` routes the six reviewers.  The project skill `review-rounds` (`.claude/skills/review-rounds/SKILL.md`) starts the agents in each round, and runs cargo-mutants (see "Review a change").

## Before you start

1. Install Claude Code ([docs](https://code.claude.com/docs)) and start it at the repo root.  It loads `CLAUDE.md` itself.  It reads the project settings only from the folder where it starts.  The rustfmt hook needs `jq` and `rustfmt` on `PATH`; macOS 26 has `/usr/bin/jq`.
2. Set up commit signing with a GPG or SSH key ([GitHub docs](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits)).  The agent signs every commit (GIT-2), so a commit fails without a key.
3. To run browser tests, set up the machine as `README.md` (setting up a machine) tells you.
4. Read `CLAUDE.md` once.  It is short, and it is what the agent works from.

## How a task flows

1. Say what you want and why, in one or two sentences.  Name the result, not the steps.
2. A small task: the agent does it, runs the checks and commits.  A larger task: the agent writes a plan first.
3. Review the plan before the code.  One wrong line in a plan becomes many wrong lines of code.
4. The agent makes small signed commits, then runs three review rounds with `/review-rounds`.
5. Do what only a person can do (see below).  Push, or ask the agent to push, then merge.

## Recipes

Each recipe gives a prompt to start from, what the agent does, and what to check.

### Fix a bug

* Prompt: "Fix <the bug>.  First add a test that fails because of the bug."
* The agent: reproduces the bug with a failing test, fixes it, runs fmt, clippy, the tests and `docs-check`, and commits.
* Check: would the new test fail if the fix were wrong?  A test that cannot fail was the most frequent finding in this repo's history (VERIFY-1).

### Add a feature or change a design

* Prompt: "Write an execution plan for <the feature> in docs/plans/active/, as docs/plans/PLANS.md says.  Do not start the work."
* The agent: writes a plan with milestones, and one command per milestone that shows it works.
* Check: read the plan and correct it.  Then: "Do the plan, one milestone at a time."
* To continue in a new session: "Resume docs/plans/active/<plan>.md."  The agent reads the plan's progress first.

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
* Check: `cargo run -p xtask -- web-test`, `cargo run -p xtask -- e2e storage` and `cargo test -p xtask --test rules` pass.

### Review a change

* Prompt: "Run `cargo build -p xtask`, then `/review-rounds main 1 "<criteria>"`, then rounds 2 and 3."  The skill's first line runs `target/debug/xtask`, which fails with exit 127 if the binary is missing.
* The skill: `/review-rounds <base> <round> "<criteria>" [--review-only] [--baseline]`.  The criteria are one section of a plan (`file#heading`), or the messages of commits (`commits:<range>`).  `--review-only` stops the round after the findings file.  `--baseline` also runs the frozen `baseline` review, to compare with.
* The agent, in each round:
  1. `xtask review start` refuses a tree with changes ("commit first"), and chooses the reviewers.
  2. cargo-mutants runs in the background (`xtask review mutants`).
  3. The routed reviewers run in parallel, each as a new subagent.  The agent merges their findings.
  4. A `validator` tries to disprove each blocker and should-fix, and the missed mutants.
  5. `xtask review check` stops the round if the repo changed in a way that a review must not cause.
  6. The agent writes the findings file in the run folder under `target/review/run/`.  It fixes each found finding, with a failing test first where the finding is about behaviour.  A `pre-existing` finding can go to `docs/tech-debt.md` or its own PR.  It commits "review round <n> fixes", and adds the round to the log.
* Check: the refuted findings, the unproven blockers and should-fix, each `repeat-of`, and each mutants line.  The PR description lists them.
* From the PR after phase 3's, pass `--baseline` in round 1, until the Progress of `plans/active/harness-engineering.md` records the gate.
* The skill runs `xtask review` with no approval only in the turn in which it loads.  The notices of the background reviewers and of the mutants run start new turns.  So in the default permission mode, `review check` asks for approval once in each round, and a late `review mutants` can ask too.  Approve each one once, not with "don't ask again".  Step 11 of the skill counts these late review commands in their own cell (TD-36).  "Don't ask again" changes the local settings file of Claude Code, and `review check` then fails the round.
* In a mode that can ask for approval, count the tool calls that you approve in each round.  This is the default mode, or auto mode when it falls back to a prompt.  Count from `review start` until the agent writes the findings file.  In round 1 with `--baseline`, count those for the `baseline` agent and its validators apart.  Give one total if you cannot split them.  Step 11 asks for the counts and waits: give them then (TD-38).  An orchestrator that runs the skill in a `claude -p` run, in auto mode or in `dontAsk` mode does not ask.  It writes 0 with the mode.  In a `-p` run where a host or a prompt tool answers prompts, it writes the approvals that the run records, or `not counted`.  In auto mode in an interactive session, it also marks that a fallback prompt is not counted.  The log records approvals as cost.  An intervention is a message of a person in the round, but not the message with the counts.
* If `review check` names a change, the round stops.  Examine `git reflog`, the stash and the changed files, then start the round again.
* A branch has one set of rounds and one log.  So "the run folder holds a log" from round 1 means that an earlier set ran (TD-37).

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

The agents in `.claude/agents/` review a change, and validate the findings.  Each agent is read only.  It has the tools `Read` and `Bash`, and its hook runs the guard `.claude/hooks/read-only-bash.sh` before each Bash command.  The guard refuses a command that would change the repo, a remote or a published package.  Examples are `git commit`, `git push`, `git stash`, `gh`, `cargo publish`, `xtask publish` and `xtask review`, also through `cargo run` of xtask.  The agent sees "read-only guard:" and the reason.  The guard matches text, so it stops mistakes, not an agent that tries to escape (TD-34).  Claude Code runs the hook only in a folder that a person trusts, and not in `-p` mode.

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
* A new rule, decision or gap is recorded, and the plan's progress is current.
* A change to the layers table in `docs/ARCHITECTURE.md` changes what LAYER-1 allows.  Review it as a design change.
* A changed snapshot PNG in `app/tests/snapshots/` is a change to the UI.  So are a `kittest.toml`, a changed `threshold` or `max_failed_pixels`, an `#[ignore]` on a snapshot test, and a removed `try_snapshot`.  Look at each one before you merge.

## Tools

The agent and a person use the same `xtask` commands (`cargo run -p xtask -- <command>`; `README.md` has the details).

| Command | What it does | Needs |
| - | - | - |
| `docs-check` | checks that the map and docs match the code | nothing |
| `review start`, `review check`, `review mutants` | `review start <base> <round>` chooses the reviewers of a diff, and records the state of the repo.  `review check <round>` names each change since that record.  `review mutants <round>` runs cargo-mutants on the round's `since` diff for at most 60 minutes | cargo-mutants 27.1.0, for `review mutants` (`README.md`) |
| `clean` | lists what runs leave in the repo, with sizes, and changes nothing: worktrees and run folders under `target/review/`, the worktrees of Claude Code agents, merged local branches, build output and the pid files of `serve`.  `clean --yes` removes only the items that it can prove safe, and stops a `serve` that a run left.  `clean --also <path>` adds a scratch root in the temporary folder | `gh`, `ps` and `lsof`; it keeps an item whose fact it cannot read |
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
