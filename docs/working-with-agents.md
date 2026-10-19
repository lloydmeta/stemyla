# Working with agents

How to get work done in this repo with an AI coding agent (Claude Code).  This guide is for anyone who directs the agent: the maintainer, a contributor, or a team that wants to use the same approach.

The agent reads `CLAUDE.md` (the map) and the docs.  This guide is for the person: what to ask for, what the agent does, and what a person must still do.  The guide changes with each phase of `plans/active/harness-engineering.md`.  The changelog at the end records each change.  `xtask docs-check` fails if the guide does not name every `xtask` command, project skill and project subagent.

## Status

Phase 1 is in place:

* The map (`CLAUDE.md`) and the docs under `docs/`, with the rules by ID in `invariants.md`.
* Rules for parts of the tree in `.claude/rules/`, which load when the agent touches those files.
* `xtask docs-check`, a test that fails when the map or the docs drift from the code.

A person still starts review rounds, gives evidence from real hardware, pushes and merges.  No hooks, project skills or project subagents exist yet (phases 2 and 3).

## Before you start

1. Install Claude Code ([docs](https://code.claude.com/docs)) and start it at the repo root.  It loads `CLAUDE.md` itself.
2. To run browser tests, set up the machine as `README.md` (setting up a machine) tells you.
3. Read `CLAUDE.md` once.  It is short, and it is what the agent works from.

## How a task flows

1. Say what you want and why, in one or two sentences.  Name the result, not the steps.
2. A small task: the agent does it, runs the checks and commits.  A larger task: the agent writes a plan first.
3. Review the plan before the code.  One wrong line in a plan becomes many wrong lines of code.
4. The agent makes small signed commits, then runs three review rounds.
5. Do what only a person can do (see below), then push and merge.

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

* Prompt: as for a feature.  The agent loads `.claude/rules/gpu.md` when it touches `roformer-gpu/`.
* The agent: keeps the kernel inside the WebGPU limits (GPU-1) and tests it against the CPU (GPU-2).
* Check, on a machine with a real GPU: `cargo test -p roformer-gpu --release`, then `cargo run -p xtask -- e2e selftest --weights <model> --query mm=<kernel>`.  Put the adapter name in the PR.  A pass on SwiftShader alone is not enough (GPU-3).

### Change memory, buffers or audio playback

* The agent loads `.claude/rules/memory.md` when it touches `engine/`, `app/src/` or `worker/`.
* Check: does the change copy a song- or model-sized buffer (MEM-1)?  If so, the PR must say why, and a test must pin the sharing.

### Change a message between the page and the worker

* The agent loads `.claude/rules/protocol.md`, and changes the hand-written e2e scripts in the same commit (PROTO-1).
* Check: `cargo run -p xtask -- web-test` and `cargo run -p xtask -- e2e storage` pass.

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

* Push, and run `cargo run -p xtask --release -- publish`.  The agent does neither unless asked (GIT-1).
* Accept or reject decisions.
* Give evidence from real hardware: a real GPU (GPU-3), Safari, an iPhone for memory (MEM-4).
* Merge.  `main` needs an approval.
* Decide when the agent stops because a change would break a rule.

## Before you merge

* CI passes, including `browser tests (Chrome)`, which is not yet a required check (TD-20).
* The PR says what was verified, how, and what was not verified.
* A new rule, decision or gap is recorded, and the plan's progress is current.

## Tools

The agent and a person use the same `xtask` commands (`cargo run -p xtask -- <command>`; `README.md` has the details).

| Command | What it does | Needs |
| - | - | - |
| `docs-check` | checks that the map and docs match the code | nothing |
| `serve` | serves the built page (`serve app/dist --port 8080`) | `trunk build` first |
| `web-test` | browser tests (wasm-bindgen-test) in Safari or Chrome | wasm-bindgen-cli, a WebDriver |
| `e2e` | drives the built page over WebDriver (`e2e smoke`, `e2e storage`, ...) | `trunk build`, a WebDriver; some scenarios need a GPU and the model |
| `publish` | builds, commits to `gh-pages` and pushes | only when asked (GIT-1) |
| `gen-song` | writes the demo song's stems, mix and ground truth | ffmpeg for MP3 (optional) |
| `stretch-bench` | compares the time-stretch methods for quality and cost | nothing |
| `perf` | measures the cost of one audio callback | nothing |
| `live-smoke` | plays 2 s on the default output device | the `cpal` feature, an audio device |
| `hpss`, `sdr`, `classic` | classic drum separation, and its score against ground truth | input audio |
| `roformer` | separates a song with the model on the CPU (slow) | the model |
| `roformer-convert` | converts a PyTorch checkpoint to safetensors | the checkpoint |
| `roformer-selftest` | writes the reference files for `?selftest` | the model |

## Changelog

* 2026-10-19, phase 1: first version.  The map, the docs, rules by path and `docs-check`.  Review rounds start from a prompt.
