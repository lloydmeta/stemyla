# Docs index

What each doc is for and when to read it.  `CLAUDE.md` at the root is the map loaded into every session; everything here is read on demand.  `cargo run -p xtask -- docs-check` fails when a doc under `docs/` is missing from this list.

## Always relevant

* `invariants.md`: every rule by ID (MEM-1, GPU-1, ...), why it holds, what enforces it and its known gaps.  Read before any change.
* `tech-debt.md`: known gaps in enforcement and known breaches of the rules, by ID (`TD-n`).  Read before relying on a rule being checked, and add to it when leaving a gap.

## How it works

* `ARCHITECTURE.md`: the pipeline from a song to six stems, the crates and their layers, the model, memory on phones, the GPU kernels, browser storage, the tests, and a vocabulary of terms.

## How work is done

* `working-with-agents.md`: for the person who directs the agent: recipes for common tasks, what a person must still do, and the tools.  Read it first when you start to work here.
* `harness.md`: the parts of the agent harness (map, rules, checks, reviews), the failure each one addresses, and the baseline it is measured against.
* `plans/PLANS.md`: how to write and keep an execution plan.  Plans in progress are in `plans/active/`, finished ones in `plans/completed/`.
* `plans/active/harness-engineering.md`: the phased adoption of harness engineering in this repo.
* `plans/active/harness-phase-2.md`: phase 2 of that plan: each rule a machine can check becomes a test, a lint or a hook, one PR per milestone.
* `decisions/0001-harness-in-the-repo.md`: agent knowledge is versioned in the repo, with `CLAUDE.md` as a map; the harness is Claude Code only.
* `decisions/0002-staged-background-autonomy.md`: agents are to work in the background, up to merging their own PRs, reached in stages with gates.
* `decisions/0003-required-skips-in-a-test-support-crate.md`: `STEMYLA_REQUIRE` turns a skipped test into a failure, through one helper in the `test-support` crate.
* `decisions/0004-synth-file-writers-behind-a-feature.md`: `synth`'s file writers are behind its `files` feature, so the web app (`app` and `stem-worker`) takes only song generation.
* `decisions/0005-how-strictly-the-crate-checks-read-layer-1.md`: the layers table lists each dependency between workspace crates, and the page side and the worker side share only `stem-protocol` (LAYER-1).

## References

Distilled facts from outside the repo, each with its source and the date it was checked.

* `references/browser-quirks.md`: Safari, Web Audio and WebGPU behaviour the code works around.
* `references/webgpu-limits.md`: the WebGPU default limits every kernel must fit (GPU-1).
* `references/local-browser-testing.md`: running `xtask web-test` and `xtask e2e` on the maintainer's Mac: Safari, Chrome, model files and known local failures.

## Conventions

These are what `docs-check` holds the docs to:

* Paths in code spans (`engine/src/song.rs`) exist, relative to the repo root, to the doc's own folder or, inside `docs/`, to `docs/`.  Plans and decisions are exempt: plans name files still to be made, and decisions are records of their time.  Name files, functions and types rather than linking to lines that move; `file.rs:12` is fine for evidence, and the check ignores the line.
* IDs are defined once, as a `### ID: title` heading in `invariants.md` or `tech-debt.md`, and never reused; every `###` heading there is an ID.  A mention of an ID whose prefix is in use (MEM, TD, ...) must be defined; an unknown prefix cannot be told from text such as `SHA-256`, so it is not checked.  A closed ID is a `* ID: ...` bullet under `## Closed`.  Only plans and decisions may name it, and it cannot be defined again.
* The tests an invariant names on its `* Enforced by:` line exist.
* Every workspace crate is listed in the layout of `CLAUDE.md` and `README.md` and under the crates of `ARCHITECTURE.md`.
* Decisions are numbered `0001-...` with no gaps and carry a `Status:` line.
* Plans live in `plans/active/` or `plans/completed/` and keep the living sections listed in `plans/PLANS.md`.
* Every rule in `.claude/rules/` has `paths:` frontmatter whose globs match files.
* `CLAUDE.md` stays within 100 lines, and its rules section lists exactly the current rules in `invariants.md`.
* `working-with-agents.md` names every `xtask` command and every project skill, subagent and command in `.claude/`.
