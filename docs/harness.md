# The agent harness

This repo has been written by AI agents (Claude Code) from its first commit.  The harness is everything around the model that makes that work reliable: what the agent reads (guides), what checks its output (sensors), and how work is planned and reviewed.  The roadmap is `plans/active/harness-engineering.md`; the decisions behind it are `decisions/0001-harness-in-the-repo.md`, `decisions/0002-staged-background-autonomy.md` and `decisions/0003-required-skips-in-a-test-support-crate.md`.

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
| `every_kernel_fits_the_default_limits`, naga over every shader (GPU-1) | sensor, computational | a kernel over WebGPU's default limits fails only on a real adapter, on some GPUs; only the tile memory was checked before | phase 2 |
| `STEMYLA_REQUIRE` and `test_support::skip` (VERIFY-1) | sensor, computational | a test with no GPU or no fixture returned early and passed, so a run that checked nothing looked like evidence | phase 2 |
| `every_skip_can_be_required`, `the_skip_allowlist_is_current` (VERIFY-1) | sensor, computational | a new skip that prints its own message escapes `STEMYLA_REQUIRE`.  It sees only messages that start with `skipping` or `skipped`, in any case (TD-8) | phase 2 |
| `synth_brings_no_crate_into_the_web_app` (LAYER-1) | sensor, computational | `app`, in the web app (`app` and `stem-worker`), compiled `hound` for `synth`'s file writers, and the licence list credited it | phase 2 |
| the three `-p` clippy steps in CI | sensor, computational | `--workspace` builds merge features, so they hide warnings in `synth` without `files`, a native-only call to a writer in `app`, and a lost dev-dependency | phase 2 |

Planned components, by phase, are in the plan.

## Baseline

Measured on 2026-10-19 from the repo's history (PRs #1 to #12) and the 15 local Claude Code sessions that built it, before the harness existed.  Later phases are compared against it.

* Review: 65 review subagents across the sessions, with the read-only rules retyped in each prompt.  PR #11 needed 5 rounds; GitHub Copilot still found valid bugs after the agent's own review on #3 and #5.
* Vacuous tests: 17 of the 65 review transcripts report a test that could not fail (a tolerance every result passed, a check that passed by timing luck, a test that copied the code it tested).  This was the most frequent finding.
* CI: PR #11 passed CI and then failed on `main` (a timing-sensitive e2e check); PR #12 fixed it.
* Environment: 54 WebDriver "timed out while connecting" errors and chromedriver downloaded 8 times in 3 sessions.
* Size: PRs grew to +5,307 lines in 47 files (#11) from a 14-hour session that compacted twice.

## Measuring

Per PR, from the PR description and review logs: review rounds, findings confirmed per round, findings that were vacuous tests, CI failures on `main` after merge, and corrections the maintainer had to make.  Per sensor: what it caught.  A component that catches nothing for long enough is a candidate for removal; a failure that recurs gets a new component, ideally a sensor.
