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
| `xtask review start`, `check` and `mutants`, with `.cargo/mutants.toml` | sensor, computational | one general reviewer read every diff; a reviewer could change the repo, and no one saw it; reviewers planted mutants by hand.  The route is a short denylist (TD-31), the check sees only its record (TD-32), and cargo-mutants tests only part of a diff (TD-33).  cargo-mutants runs locally only, in the skill.  `no_tracked_file_is_ignored` keeps each tracked file, such as `Cargo.lock`, in the copy of cargo-mutants | phase 3 |
| `.claude/hooks/read-only-bash.sh`, a PreToolUse guard on the review agents' Bash calls, kept by `the_bash_guard_refuses_writes` and `the_bash_guard_explains_each_refusal`; `the_agents_are_read_only_by_tools` keeps each agent's tools and hook | sensor, computational | phase 2's reviewers could commit, push or stash, and only their prompt said not to.  It matches text, and Bash can still write files (TD-34) | phase 3 |
| the `review-rounds` skill (`.claude/skills/review-rounds/SKILL.md`), kept by `the_review_skill_starts_with_xtask`, and the review agents in `.claude/agents/` | sensor, inferential | the agent wrote each reviewer's prompt by hand, one general reviewer read every diff, and nothing checked a finding before a fix.  A round stops only on a change that `review check` records (TD-32), and a review runs the diff's code with the person's credentials (TD-35) | phase 3 |
| `xtask clean`, and the pid file that `xtask serve` writes | tool, computational | runs left 53 GB of replay clones and 4.3 GB of reviewer worktrees.  `target` grew to about 91 GB, and the disk filled once.  Runs also left 40 to 45 squash-merged branches, agent worktrees and a running `xtask serve`.  Nothing in the repo removed them, and auto mode refused the agent's `rm -rf` and `git branch -D`.  `clean --yes` removes only an item with a proof.  It sees a process only by its open files (TD-67), and a stopped `serve` leaves its pid file (TD-68) | phase 3b |
| `xtask review clean <round>`, which the `review-rounds` skill runs after the log | tool, computational | reviewers make their worktrees with `git worktree add`, so the worktree sweep of Claude Code never removes them (4.3 GB in `target/review`).  Agents also named them outside the form `<round>-<agent>-<n>` (M2, M3), so it removes each name that starts with `<round>-` | phase 3b |
| the known-missed list, `KNOWN_MISSED` in `xtask/src/review.rs`: `review mutants` writes its matches to `known.txt`, and the skill gives them no validator.  `known_problems` keeps each entry open and in the code | sensor, computational | a confirmed mutant that went to TD-39 or TD-66 came back in each round and got a validator again, since `repeat-of` matched only refuted findings.  Yield: `known 0` in each of the 31 rounds of M2 to M7 that ran `review mutants`, 28 of them in M3 to M7.  The 31 replay runs of M5 also gave `known 0`.  No diff of a round touched a function of its 13 entries, in 5 files.  On the rounds of PR #39 after `6d0a39d`, it would match at most 23 of 29 lines (M2).  An entry can go stale (TD-33) | phase 3b |
| the fix rule and the stop rule of decision 0024: the `Significant` column of the log, `xtask review log-check` after each round, the log checks of `docs-check`, and `the_fix_rule_says_the_same_in_each_place` | sensor, computational | rounds 4 and later of the 14 older logs fixed 124 findings: 71 nits and 13 bugs.  The rule that rounds go past round 3 lived only in a completed plan, and nothing checked a log against a rule.  A `reported, not fixed` finding came back and got a validator again (M3, M4), so `log-check` also checks a repeat of it.  The team leader judges significance, and the check reads the Action cell as text.  The check comes from the branch's own code (TD-31) | phase 3b |
| from round 4, the `Scope:` line of the skill's prompt: a reviewer reads the fix range and the files that it touches (D4 of `plans/active/harness-phase-3b.md`) | guide | a late round reviewed the whole branch again at full cost, and 7 of 10 late bugs were in fix churn.  A late round of M3 to M7 used 0.06M to 0.62M final-context tokens.  A bug in a file that the fix range does not touch is out of scope (TD-73) | phase 3b |
| `xtask check --tier commit` and `--tier push` (decision 0026), with `STRICT_PATHS` in `xtask/src/check.rs`.  `the_push_tier_runs_the_lines_of_claude_md` and `git_2_names_the_tiers` keep `CLAUDE.md` and GIT-2 in step with it | sensor, computational | GIT-2 ran every local check before each commit.  A PR #39 commit of comments and paths ran them for 16 minutes, against 7.5 minutes in CI.  No CI failure of that period was in a check that runs locally.  Warm, tier `commit` takes 11 s on a docs change and 68 s on an `xtask` change, and tier `push` about 2 minutes.  Tier `commit` can pass where the full checks fail (TD-70) | phase 3b |
| `agent_problems` takes `model` from `inherit`, `opus`, `sonnet` and `haiku`, and an optional `effort`.  `the_agent_models_are_the_measured_ones` pins each agent file to the measurement of decision 0025 | sensor, computational | each agent ran on the session's model, because the check required `model: inherit`, and nothing tied a change of model to evidence.  M5 found no candidate that passes, so the pin keeps each file on `inherit`.  The measurement had one run of each arm (TD-71) | phase 3b |
| the `orchestrate` skill (`.claude/skills/orchestrate/SKILL.md`) and the sizing line of `CLAUDE.md`, kept by `the_orchestrate_skill_keeps_its_key_lines` and `the_orchestrate_pins_fail_on_a_changed_skill` | guide | harness phase 2 ran as about 17 sessions and 18 PRs, with at least 54 maintainer messages.  The hands-off mode lived as prose that a person copied into each plan.  PR #39's planning session started the work inline, until the maintainer stepped in three times.  No run shows the up-front message yet (TD-72) | phase 3b |
| the plan and log rules of `docs-check`: an active plan of more than 3 milestones has the sections "How the work runs" and "Decisions for the maintainer", and the last round of a copied log meets the stop rule | sensor, computational | PR #39's plan had no section for the mode.  The maintainer's decisions came during the run, not up front (the correction of 2026-11-01 in `plans/active/harness-phase-3b.md`).  A log could end at a round that did not meet the stop rule.  It does not check completed plans | phase 3b |
| `xtask pr-body <file>` and `.github/pull_request_template.md` (decision 0027) | sensor, computational | PR bodies grew to 15.5k characters with no headings, and the "Not verified" section was gone.  It checks the headings, a summary of 1 to 150 words, 1 to 3 Verification lines and the `<details>` block.  It does not check what "Needs your attention" holds | phase 3b |
| `xtask attention <base> <plan>` (decision 0027) | sensor, computational | no PR had an inline comment, and a person read the whole log to find what to decide.  It lists the candidate lines for at most 10 comments, the hand-run lines first, and the orchestrator posts one `COMMENT` review.  Its classes go by path and by item name, so a message helper outside the named items is missed | phase 3b |

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

## Phase 3b, measured

Measured on 2026-11-02 over PR #41 (M1 to M8 of `plans/active/harness-phase-3b.md`), against PR #39.  Token figures from the review logs are final contexts, not tokens used (TD-44).

* Hands-off: the orchestrator ran M1 to M8 on one branch and one PR.  The 40 review rounds of M1 to M7 had 0 interventions.  The maintainer sent 11 messages in the run.  3 of them changed the work: the pause for travel, the correction of the up-front questions, and the new end at 08:00 EST.
* Late rounds: in M3 to M7, 15 rounds after round 3 (3.0 for each milestone) fixed 27 findings and logged 48 as `reported, not fixed`.  3 of the 27 were bugs (M7 4-1, 4-7 and 5-2), each in a file that the fix range touched.  In 7 logs of PR #39 (all but M8), 17 rounds after round 3 (2.4 for each log) fixed about 58 findings, by an approximate count of the Action cells.  So the fixes after each late round fell, from 3.4 to 1.8, but the rounds after round 3 did not.
* Cost: the 40 rounds of M1 to M7 used about 15.68M final-context tokens and 500 wall minutes.  The rounds after round 3 of M3 to M7 used 4.07M of their 11.77M (35%).  No one measured the share of the weekly usage limit, since the orchestrator cannot see it.  Rounds 1 to 3 of one milestone used 0.57M to 1.89M, against 1.6M to 2.9M in PR #39.  This PR changes the harness and PR #39 the app, so these are not like for like.  M5's measurement used 40.85M `modelUsage` tokens.
* Models: no cheaper model passed (M5), so the cost of each agent role did not fall.
* Checks: warm, tier `commit` took 11 s to 68 s, and tier `push` about 2 minutes.  A PR #39 commit ran the full set for 16 minutes.
* Known-missed list: `known 0` in each round that ran `review mutants`.
* Cleanup: M5 removed 35 clone folders (29.1 GiB) with `clean --yes --also`.  The orchestrator ran `clean --yes` between milestones.  The PR body of #41 lists the steps left for the maintainer after the merge.
* CI: one browser job failed by wall-clock timing, and its rerun passed (M2).  One real failure (`8c228fe`: colour codes in a `cargo metadata` error) got a fix and a test (`10e7a03`).

The rest is in `plans/active/harness-phase-3b.md`.

## Measuring

Per PR, from the PR description and review logs: review rounds, findings confirmed per round, findings that were vacuous tests, CI failures on `main` after merge, and corrections the maintainer had to make.  Per sensor: what it caught.  A component that catches nothing for long enough is a candidate for removal; a failure that recurs gets a new component, ideally a sensor.
