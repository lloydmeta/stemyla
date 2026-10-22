# The agent harness

This repo has been written by AI agents (Claude Code) from its first commit.  The harness is everything around the model that makes that work reliable: what the agent reads (guides), what checks its output (sensors), and how work is planned and reviewed.  The roadmap is `plans/active/harness-engineering.md`; the decisions behind it are the records in `decisions/`.

Two principles shape it:

* Every component encodes an assumption about what the model cannot do on its own ([Anthropic, harness design for long-running apps](https://www.anthropic.com/engineering/harness-design-long-running-apps)).  So each one below names the failure it addresses, and is removed when it stops earning its place.
* Prefer computational sensors (tests, lints, structural checks: fast and deterministic) to inferential ones (LLM review), and pair each guide with a sensor ([Böckeler, harness engineering](https://martinfowler.com/articles/harness-engineering.html)).  Context files on their own tend to lower task success and add over 20% to cost ([Evaluating AGENTS.md, arXiv 2602.11988](https://arxiv.org/abs/2602.11988v1)), so the map stays short and rules move into checks.

## Components

| Component | Kind | Failure it addresses | Added |
| - | - | - | - |
| `CLAUDE.md` as a map of at most 100 lines | guide | context re-explained each session; one long file goes stale and is skimmed | phase 1 |
| `invariants.md`, rules with stable IDs | guide | rules lost in prose; reviewers cannot cite what a change breaks | phase 1 |
| `.claude/rules/`, rules scoped to paths | guide | crate-specific rules cost context in every session | phase 1 |
| `plans/` with living execution plans | guide | plans outside the repo invisible to other sessions and agents; status left stale | phase 1 |
| `decisions/` | guide | settled choices re-argued, or silently reversed | phase 1 |
| `tech-debt.md` | guide | known gaps rediscovered, or mistaken for enforced rules | phase 1 |
| `references/` | guide | local setup and browser quirks rediscovered (chromedriver downloaded 8 times) | phase 1 |
| `working-with-agents.md`, kept complete by `docs-check` | guide, for people | how to drive agents lived in one person's head and in old transcripts | phase 1 |
| `xtask docs-check` (`the_map_and_docs_match_the_code`) | sensor, computational | docs drift from code (on 2026-10-19: `synth` missing from the map, `stem-protocol` from two crate lists) | phase 1 |
| CI runs `cargo test -p roformer-gpu --lib` and `--doc` | sensor, computational | `--exclude roformer-gpu` also dropped the crate's tests that need no GPU, and its doctests, so they never ran in CI | phase 2 |
| `browser tests (Chrome)` is a required check in the `main` ruleset | sensor, on GitHub | a PR could merge while the browser tests failed | phase 2 |
| `every_kernel_fits_the_default_limits`, naga over every shader (GPU-1) | sensor, computational | a kernel over WebGPU's default limits fails only on a real adapter, on some GPUs; only the tile memory was checked before | phase 2 |
| `STEMYLA_REQUIRE` and `test_support::skip` (VERIFY-1) | sensor, computational | a test with no GPU or no fixture returned early and passed, so a run that checked nothing looked like evidence | phase 2 |
| `every_skip_can_be_required`, `the_skip_allowlist_is_current` (VERIFY-1) | sensor, computational | a new skip that prints its own message escapes `STEMYLA_REQUIRE`.  It sees only messages that start with `skipping` or `skipped`, in any case (TD-8) | phase 2 |
| `synth_brings_no_crate_into_the_web_app` (LAYER-1) | sensor, computational | `app`, in the web app (`app` and `stem-worker`), compiled `hound` for `synth`'s file writers, and the licence list credited it | phase 2 |
| the three `-p` clippy steps in CI | sensor, computational | `--workspace` builds merge features, so they hide warnings in `synth` without `files`, a native-only call to a writer in `app`, and a lost dev-dependency | phase 2 |
| the `wasm32` clippy step in CI | sensor, computational | the web code compiles only for `wasm32`, so no lint ever ran on it in CI, where two of the song-sized copies were (closed in M18) | phase 2 |
| clippy's `disallowed-methods` for `to_vec`, with a canary for each path (MEM-1) | sensor, computational | three song-sized copies went unnoticed (closed in M18), and a wrong path in `clippy.toml` passes clippy.  It sees `to_vec` only (TD-26) | phase 2 |
| the spy on `write` in the MEM-5 tests of the page and the worker (MEM-5) | sensor, computational | the tests of whole writes read back the bytes, which only Safari can fail, so CI could not see a view reach `write` | phase 2 |
| clippy's `disallowed-methods` for `wgpu` buffer creation, with a canary, and a private device (MEM-3) | sensor, computational | `GpuBuffer` was a convention: a buffer made on `Gpu`'s public device, or by a wgpu helper, is never destroyed on the web.  It sees `roformer-gpu` only (TD-27) | phase 2 |
| `crates_depend_only_downwards`, `the_page_and_the_worker_meet_only_through_stem_protocol` (LAYER-1) | sensor, computational | nothing compared the layers table with the manifests, or kept the page side and the worker side apart; a crate outside the workspace could hide a dependency | phase 2 |
| the `#![expect]`s in `dsp/src/lib.rs`, one for each lint | sensor, computational | `#![allow(clippy::all)]` in `dsp` switched off every clippy lint, so a correctness lint passed there, and a `disallowed-methods` list could not reach it | phase 2 |
| `dsp_is_beginner_style` (DSP-1) | sensor, computational | clippy has no lint against iterators, closures or `swap`, and one more `+=` passed the `expect`s.  It scans for the likely breaches only (TD-3) | phase 2 |
| `no_python_in_the_build` (BUILD-1), `no_audio_or_weights_are_tracked` (ASSET-1), over the git index | sensor, computational | nothing failed a Python file or a Python step in CI, and `git add -f` could put a song or a model file past `.gitignore`.  `.gitignore` also ignored source folders named `audio` or `stems` at any depth | phase 2 |
| `young_crates_are_pinned`, `third_party_lists_what_ships` (DEPS-1), over `cargo metadata` and `cargo tree` | sensor, computational | `wsola` and `timestretch` came in as caret ranges, and nothing compared `THIRD-PARTY.md` with what ships | phase 2 |
| `e2e_messages_use_protocol_types` (PROTO-1) | sensor, computational | the e2e scripts write message types by hand, and CI runs only some scenarios, so a renamed type failed late or nowhere.  It reads the likely shapes only, and no field names (TD-16) | phase 2 |
| `the_callback_does_not_allocate_after_warm_up`, a counting allocator over the audio callback (RT-1) | sensor, computational | wsola's `pull`, wsola's growing input, chipmunk's `double_speed` and a short loop's anchors allocated in the callback, and nothing counted.  It runs natively only (TD-25) | phase 2 |
| deny rules in `.claude/settings.json` for a push to `main` or `gh-pages`, `xtask publish` and `gh pr merge`, kept by `claude_settings_hold_the_deny_rules` (GIT-1) | sensor, computational | nothing stopped an agent from pushing to `main` or `gh-pages`, publishing the site or merging a PR; auto mode pushes with no prompt.  They refuse the call before it runs, and match command text only (TD-28) | phase 2 |
| GitHub rulesets: `main` lets the admin role bypass for pull requests only, and `gh-pages history` blocks force pushes and deletion for everyone (GIT-1, GIT-3) | sensor, on GitHub | the admin role, which the agent shares, could push to `main` directly, and nothing stopped a rewrite of `gh-pages`.  Nothing in the repo checks the rulesets (TD-20) | phase 2 |
| a PostToolUse hook in `.claude/settings.json` runs rustfmt on each `.rs` file in the repo that `Edit` or `Write` changes, kept by `the_rustfmt_hook_formats_rust_files` (GIT-2) | sensor, computational | in 5 of 31 sessions, `cargo fmt --all --check` found a diff before a commit, and the agent ran fmt and the check again.  It sees `Edit` and `Write` only; the `fmt` job is the control | phase 2 |
| UI snapshots compared in CI's `test` job, and `no_build_file_updates_snapshots` (VERIFY-1) | sensor, computational | `UPDATE_SNAPSHOTS=1` in CI rewrote every snapshot that failed, so CI never compared the UI; the comment said the PNGs could not match, and they did.  A change of 1 colour level passes (TD-29) | phase 2 |

Planned components, by phase, are in the plan.

## Baseline

Measured on 2026-10-19 from the repo's history (PRs #1 to #12) and the 15 local Claude Code sessions that built it, before the harness existed.  Later phases are compared against it.

* Review: 65 review subagents across the sessions, with the read-only rules retyped in each prompt.  PR #11 needed 5 rounds; GitHub Copilot still found valid bugs after the agent's own review on #3 and #5.
* Vacuous tests: 17 of the 65 review transcripts report a test that could not fail (a tolerance every result passed, a check that passed by timing luck, a test that copied the code it tested).  This was the most frequent finding.
* CI: PR #11 passed CI and then failed on `main` (a timing-sensitive e2e check); PR #12 fixed it.
* Environment: 54 WebDriver "timed out while connecting" errors and chromedriver downloaded 8 times in 3 sessions.
* Size: PRs grew to +5,307 lines in 47 files (#11) from a 14-hour session that compacted twice.

## Phase 2, measured

Measured on 2026-10-22 over phase 2's 18 PRs (#14 to #32, without the Dependabot bump #24), against the baseline above.

* Rules: "prose only" fell from 8 to 2 of the 21 rules in `invariants.md`.
* CI: the 18 runs on `main`, one for each PR, passed.  No run failed on `main` after a green PR.
* Size: without the plans, the median PR adds 418 lines (#30 adds 402, #17 adds 434), and the largest (#18) adds 2,260.
* Review: about 26 of 58 implementation review rounds (a recount gave up to 33) found a test that could pass on wrong code (about 35 findings, in 14 of 18 PRs).  The baseline had 17 of 65 review transcripts.  Phase 2's reviewers ran mutants on purpose, so the counts are not like for like.
* Checks: a new check found one real breach first; the surveys had found the others.

The rest is in `plans/completed/harness-phase-2.md`.

## Measuring

Per PR, from the PR description and review logs: review rounds, findings confirmed per round, findings that were vacuous tests, CI failures on `main` after merge, and corrections the maintainer had to make.  Per sensor: what it caught.  A component that catches nothing for long enough is a candidate for removal; a failure that recurs gets a new component, ideally a sensor.
