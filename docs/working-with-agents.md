# Working with agents

How to get work done in this repo with an AI coding agent (Claude Code).  This guide is for anyone who directs the agent: the maintainer, a contributor, or a team that wants to use the same approach.

The agent reads `CLAUDE.md` (the map) and the docs.  This guide is for the person: what to ask for, what the agent does, and what a person must still do.  The guide changes with each phase of `plans/active/harness-engineering.md`.  The changelog at the end records each change.  `xtask docs-check` fails if the guide does not name every `xtask` command, project skill and project subagent.

## Status

Phase 1 is in place:

* The map (`CLAUDE.md`) and the docs under `docs/`, with the rules by ID in `invariants.md`.
* Rules for parts of the tree in `.claude/rules/`, which load when the agent touches those files.
* `xtask docs-check`, a test that fails when the map or the docs drift from the code.

The agent runs three review rounds after it implements a change, because `CLAUDE.md` tells it to; the review recipe below is the same thing started by hand.  A person still gives evidence from real hardware, publishes and merges.  The project settings (`.claude/settings.json`) deny `xtask publish`, `gh pr merge` and a push that names `main` or `gh-pages` to the agent.  The project settings also run rustfmt on each `.rs` file in the repo that the agent edits.  No project skills or project subagents exist yet (phase 3).

## Before you start

1. Install Claude Code ([docs](https://code.claude.com/docs)) and start it at the repo root.  It loads `CLAUDE.md` itself.  It reads the project settings only from the folder where it starts.  The rustfmt hook needs `jq` and `rustfmt` on `PATH`; macOS 26 has `/usr/bin/jq`.
2. Set up commit signing with a GPG or SSH key ([GitHub docs](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits)).  The agent signs every commit (GIT-2), so a commit fails without a key.
3. To run browser tests, set up the machine as `README.md` (setting up a machine) tells you.
4. Read `CLAUDE.md` once.  It is short, and it is what the agent works from.

## How a task flows

1. Say what you want and why, in one or two sentences.  Name the result, not the steps.
2. A small task: the agent does it, runs the checks and commits.  A larger task: the agent writes a plan first.
3. Review the plan before the code.  One wrong line in a plan becomes many wrong lines of code.
4. The agent makes small signed commits, then runs three review rounds.
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
* Check, on a machine with a real GPU: `STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release`, then `cargo run -p xtask -- e2e selftest --weights <model> --query mm=<kernel>`.  Put the adapter name in the PR.  A pass on SwiftShader alone is not enough (GPU-3).

### Ask for evidence that a test really ran

* A test that has no GPU adapter, no `timestamp-query` or no PyTorch fixture prints "skipping" and passes.  `cargo test` hides that line.
* Prompt: "Run it with STEMYLA_REQUIRE=gpu" (or `gpu,fixtures`, or `gpu,timestamps`).  Then each such skip is a failure that names VERIFY-1.
* Check: the PR gives the command with `STEMYLA_REQUIRE`, and names the adapter.

### Change memory, buffers or audio playback

* The agent loads `.claude/rules/memory.md` when it touches `engine/`, `app/src/` or `worker/`.
* Check: does the change copy a song- or model-sized buffer (MEM-1)?  If so, the PR must say why, and a test must pin the sharing.  Read the reason of each new `#[expect(clippy::disallowed_methods)]`.

### Change a message between the page and the worker

* The agent loads `.claude/rules/protocol.md`, and changes the hand-written e2e scripts in the same commit (PROTO-1).
* Check: `cargo run -p xtask -- web-test`, `cargo run -p xtask -- e2e storage` and `cargo test -p xtask --test rules` pass.

### Review a change

* Prompt: "Run three review rounds.  Use a new subagent with fresh context for each round.  Verify each finding before you fix it, and fix real findings with tests."
* The agent: starts one reviewer per round, checks what each reviewer reports, and fixes what is real.
* Check: read the findings the agent rejected as well as the ones it fixed.

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

## Before you merge

* CI passes.  Every job but `publish` is a required check on `main`, including `browser tests (Chrome)`.
* The PR says what was verified, how, and what was not verified.
* A new rule, decision or gap is recorded, and the plan's progress is current.
* A change to the layers table in `docs/ARCHITECTURE.md` changes what LAYER-1 allows.  Review it as a design change.

## Tools

The agent and a person use the same `xtask` commands (`cargo run -p xtask -- <command>`; `README.md` has the details).

| Command | What it does | Needs |
| - | - | - |
| `docs-check` | checks that the map and docs match the code | nothing |
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
