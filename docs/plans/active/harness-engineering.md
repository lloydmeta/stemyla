# Harness engineering, phased

## Purpose

Make agents productive and correct in this repo without the maintainer re-explaining context or catching the same mistakes, and carry each lesson to the maintainer's identity and access management (IAM) codebases.  The target is agents working in the background up to merging their own PRs (`decisions/0002-staged-background-autonomy.md`), reached in six phases, each of which pays off on its own.

"Harness engineering" is the work of shaping everything around the model: what the agent reads, what checks its output, how work is planned, reviewed and merged ([OpenAI](https://openai.com/index/harness-engineering/), [Anthropic](https://www.anthropic.com/engineering/harness-design-long-running-apps), [Böckeler](https://martinfowler.com/articles/harness-engineering.html), [ShipWithAI](https://shipwithai.io/blog/claude-code-harness-engineering-guide/)).  Where the sources agree:

* Context is a budget: a short always-loaded map, the rest loaded on demand ([Anthropic, context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)).
* Knowledge lives in the repo, versioned; what an agent cannot read does not exist for it.
* Mechanical checks beat instructions, and their messages tell the agent how to fix the problem.
* The agent needs a way to verify its own work, and a separate, sceptical evaluator, since agents grade their own work too kindly.
* The harness grows from observed failures, and parts are removed when models no longer need them.

The evidence behind these is uneven.  Measured: context degrades with length ([Chroma](https://www.trychroma.com/research/context-rot)); context files tend to lower task success and add over 20% to cost ([arXiv 2602.11988](https://arxiv.org/abs/2602.11988v1)); a harness change alone moved one benchmark from 52.8% to 66.5% ([LangChain](https://blog.langchain.com/improving-deep-agents-with-harness-engineering)).  The large productivity figures are self-reported case studies.  Hence a bias, in every phase, towards deterministic sensors over more prose.

## Context and orientation

On 2026-10-19 the repo had a 36-line `CLAUDE.md`, `docs/ARCHITECTURE.md`, a CI workflow (fmt, clippy, tests, wasm build, browser tests in Chrome, publish) and no project Claude Code configuration.  Rules existed as prose; a survey found most unenforced and several breached (`tech-debt.md`).  The history of 12 PRs and 15 sessions gives the baseline in `harness.md`; its most frequent failures, ranked:

1. Tests that could not fail (17 of 65 review transcripts).
2. Timing-sensitive checks that pass on a PR and fail on `main`.
3. The local browser setup as a time sink (54 WebDriver connection timeouts).
4. Local setup rediscovered (chromedriver downloaded 8 times).
5. Review rounds run by hand, prompts retyped (65 review subagents).
6. Plans and memory drifting from what was merged.
7. PR descriptions broken by shell quoting (#10, #12).
8. Very long sessions producing very large PRs (#11: +5,307 lines, 14 hours).
9. The maintainer's command-rewriting hook (RTK) mangling `grep` patterns, worked around 541 times.
10. Verification claimed but not done (UI never looked at in a browser; Safari unverified).

## Plan of work

Each phase lists what it adds, its acceptance check, what it is measured by, the gate before the next phase, and the lesson for IAM.  Every phase also updates `docs/working-with-agents.md`, the guide for people who direct agents: its acceptance includes that the guide covers the phase's new ways of working and that its changelog says so.  `docs-check` enforces part of this: the guide must name every `xtask` command, project skill and project subagent.

### Phase 1: the map and the knowledge base (guides, plus one sensor)

* Adds: `CLAUDE.md` as a map of at most 100 lines, with every crate, corrected commands, a task-to-doc table and the rules by ID; `docs/README.md` (index), `invariants.md` (rules, why, enforcement, gaps), `tech-debt.md` (seeded from the survey), `harness.md` (components and baseline), `decisions/`, `plans/` (this plan), `references/` (from the maintainer's local memory, so other sessions and agents see it); path-scoped rules in `.claude/rules/` for `dsp/`, the GPU crate, memory and the wire format; `.claude/` tracked in git; `xtask docs-check` and the test `the_map_and_docs_match_the_code`; `docs/working-with-agents.md`, a guide for the people who direct agents.
* Acceptance: the docs test fails on the drift of 2026-10-19 (`synth` missing from the map, `stem-protocol` from two crate lists) and passes once fixed; each check has a unit test with a failing case; `cargo test --workspace --exclude roformer-gpu` and clippy pass.
* Measured by: what `docs-check` catches in later PRs; whether sessions still ask for or rediscover what the docs hold.
* Gate: the maintainer reviews and merges the PR.
* For IAM: give security invariants stable IDs with an "enforced by" line, so a reviewer, human or agent, can cite exactly which control a change weakens, and "prose only" is visible as a gap rather than assumed to be a control.

### Phase 2: deterministic sensors

Turn every mechanisable rule into a check whose message names the rule ID and the fix, cheapest first ([clippy configuration](https://doc.rust-lang.org/clippy/lint_configuration.html)):

* CI: run `cargo test -p roformer-gpu --lib` (TD-1); compare UI snapshots (TD-9); make browser tests a required check.
* Strict skips: `STEMYLA_REQUIRE=gpu,timestamps,fixtures` turns a skipped test into a failure (TD-8).
* A structural test in `xtask/tests/rules.rs`: crate layering over `cargo metadata` against the layers table, with the page side and the worker side apart (TD-4, M5), `synth` in the web app (`app` and `stem-worker`) over `cargo tree` (LAYER-1, M4), a scan of `dsp`'s tokens for the likely DSP-1 breaches (TD-3, M7), Python and asset bans over the tracked files (TD-5, TD-6, M8), `THIRD-PARTY.md` against `cargo tree`, and young crates pinned, over `cargo metadata` (DEPS-1, TD-7, M9), message type strings in the e2e scripts against `stem_protocol::kind` (PROTO-1, TD-16, M10), and skip messages that bypass `test_support::skip` (VERIFY-1, M3).  Each new crate is vetted, pinned if young, and listed if shipped (DEPS-1).
* Clippy `disallowed-methods` with reasons, per crate: slice `to_vec` in `app`, `worker`, `protocol`, `engine`, `roformer` and `roformer-gpu`, and JS-array `to_vec` in the first three, on both targets (MEM-1, M12); `wgpu` buffer creation outside `GpuBuffer` in `roformer-gpu`, and a private device (MEM-3, M13); justified sites carry `#[expect(clippy::disallowed_methods, reason = "...")]`, which fails when the exception goes stale.  Narrow `dsp`'s `allow(clippy::all)` (TD-2, M6).
* A naga test over every generated shader for the limits in `references/webgpu-limits.md` (TD-1).  naga 30.0.1 is already in the lock file through wgpu.
* An allocation-counting test of the audio callback, once RT-1's scope is decided (RT-1, TD-15, M11).
* Claude Code project settings in `.claude/settings.json` ([hooks](https://code.claude.com/docs/en/hooks), [permissions](https://code.claude.com/docs/en/permissions)): deny rules for `git push` and `xtask publish` (GIT-1), and `rustfmt` on edited `.rs` files after each edit.  On 2026-10-20 the maintainer dropped the other hooks (an ASSET-1 PreToolUse hook, a Stop hook, a `--body-file` hook and a SessionStart WebDriver report).  Hooks match command text and are not controls; CI and the `main` ruleset are.
* The maintainer's RTK hook is personal and stays outside the repo; its rewriting of `grep`, `sed -n` and wasm test runs is to be excluded in the maintainer's own settings (failure 9).
* Breaches the new checks expose (TD-12 to TD-14, TD-17) are fixed in their own small PRs, or carry an `#[expect]` with a reason and stay in `tech-debt.md`.
* Acceptance: every invariant that can be checked mechanically names a test or lint on its `* Enforced by:` line; each check is shown to fail on a planted breach.
* Measured by: breaches caught by checks before review; "prose only" count in `invariants.md` (8 of 21 rules prose only at the start, and most of the rest only partly enforced).
* Gate: CI green on `main` with the new checks required.
* For IAM: express authorisation and data-handling rules as layering tests and disallowed-API lints (for example "only this module may call the token signer"), with exceptions that must state a reason and expire.

### Phase 3: review agents (inferential sensors)

* Adds: project subagents in `.claude/agents/` ([sub-agents](https://code.claude.com/docs/en/sub-agents)): one general reviewer that always runs, and specialists chosen by the paths a diff touches: GPU and WGSL (`roformer-gpu/**`), memory and realtime (`engine/**`, `app/src/**`, `worker/**`), wire format (`protocol/**`, `xtask/src/e2e.rs`, `xtask/e2e/**`), supply chain and security (`Cargo.toml` files, `THIRD-PARTY.md`, `.github/**`, `.claude/**`, `xtask/src/serve.rs`), and docs (`docs/**`, `CLAUDE.md`).  A routing table in `xtask` (`xtask review-plan`) prints which reviewers a diff needs, from paths and from content (a new `to_vec` anywhere triggers the memory reviewer).
* A validator subagent tries to disprove each finding before anything is fixed, as Claude Code's `/code-review` plugin does ([code-review command](https://github.com/anthropics/claude-code/blob/main/plugins/code-review/commands/code-review.md)).
* A project skill runs the rounds: route, review in parallel, validate, fix confirmed findings with tests, repeat with fresh reviewers; it replaces the retyped prompts (failure 5).
* Every reviewer is read-only, sees the diff and the acceptance criteria but not the author's reasoning (self-preference and sycophancy: [arXiv 2404.13076](https://arxiv.org/abs/2404.13076), [arXiv 2310.13548](https://arxiv.org/abs/2310.13548)), cites a rule ID and quotes it, and must name how to break each new test ("would this test fail if the code were wrong?"), aimed at failure 1.  Reviewer files hold the method; the rules stay in `invariants.md`.  Reviewers also look for a test that returns early without `test_support::skip`, which `every_skip_can_be_required` cannot see (TD-8).
* Mutation testing of the changed functions with cargo-mutants ([mutants.rs](https://mutants.rs/)) as a computational check for tests that cannot fail.
* Acceptance: replaying the reviews of PRs #9 to #12 finds the bugs their rounds found; each reviewer's findings are logged with whether they were confirmed.
* Measured by: confirmed findings per round, false-positive rate per reviewer, rounds per PR, vacuous tests found before merge.
* Gate: two consecutive PRs where the routed review finds what manual review would, at lower effort.
* For IAM: a security reviewer is an extra sensor, never a control: it runs alongside the deterministic checks and a human, and its hit rate is measured like any other test.

### Phase 4: legibility, the evaluator, and long-running sessions

* Adds, in the app (each a small PR under MEM-1): a `window.__stemyla` state snapshot of scalars only (lanes, mute, solo, tempo, playhead, loop, status, memory figures); structured console events beside the text logs; `?fixture=demo` to start on the demo song; a command API that queues into the same inbox as a click; a tab memory total asserted in an e2e scenario (TD-11, TD-19).  The adapter, and whether it is a fallback adapter, recorded in selftest and e2e results (TD-10).
* An evaluator subagent that agrees a contract of checks with the implementing agent before work starts, maps each criterion to a command (`xtask e2e`, `?selftest`, `cargo test`), grades with hard pass or fail, and fails any GPU criterion graded on a software adapter.  It runs GPU and fixture checks with `STEMYLA_REQUIRE` set, so a skip cannot pass as evidence (TD-8).  It cannot hear audio, so audio criteria are numeric ([Anthropic, harness design](https://www.anthropic.com/engineering/harness-design-long-running-apps)).  It runs on the maintainer's Mac (real GPU), driving the page through `xtask e2e` first and Claude in Chrome for exploration ([Claude in Chrome](https://code.claude.com/docs/en/chrome)).
* Long-running sessions: a plan in `docs/plans/active/` plus the evaluator's contract; one concern per PR (failure 8); progress written at every stopping point; a handover skill before compaction; a worktree per task with its own `xtask serve` port.
* Acceptance: an evaluator run on a planted bug (a mute that does not mute) fails it; one feature delivered end to end from a plan with no correction from the maintainer.
* Measured by: corrections per task, time from plan to merged PR, evaluator verdicts overturned by the maintainer.
* Gate: three plan-driven tasks merged with the evaluator's verdict matching the maintainer's.
* For IAM: an evaluator needs the system to be observable without reading its secrets: expose state (sessions, decisions, policy versions) through a test-only interface that is compiled out of production.

### Phase 5: background agents, in stages

Each stage needs the previous one's gate.  Agents run in GitHub Actions with `anthropics/claude-code-action` ([GitHub Actions](https://code.claude.com/docs/en/github-actions)), pinned by commit SHA at 1.0.74 or later (CVE-2026-47751: earlier versions loaded `.mcp.json` from the PR head, [GHSA-8q5r-mmjf-575q](https://osv.dev/vulnerability/GHSA-8q5r-mmjf-575q)).

* 5a, report only: scheduled gardening jobs that open issues, not PRs: `docs-check` drift, plans whose status disagrees with `git log`, new releases of the pinned young crates, unused dependencies.  A token with `contents: read` and `issues: write` only, no secrets but the model key, one concern per job, and no output when there is nothing to do.
* 5b, PRs for humans to merge: the same jobs open PRs from a dedicated GitHub App with only contents and pull-request permissions (not workflows), one open PR per job on a fixed branch, labelled, size-capped, guarded by a test-count check: CI fails a PR that lowers the number of tests in `cargo test -- --list` without a stated reason (moved from phase 2 on 2026-10-20, since no deleted test has been seen while people open the PRs).  `CODEOWNERS` requires the maintainer on `.github/**`, `.claude/**`, `CLAUDE.md`, `docs/invariants.md`, tests and shaders.
* 5c, auto-merge for an allowlist: docs-only and lock-file-only changes from a known job, with CI, the routed reviewers and the validator all passing and no protected path touched.  Decision point for the maintainer: which identity approves.  The ruleset requires one approval and the maintainer is the only person.  The options: a second GitHub App approves under rules that require approval from someone other than the last pusher; the approval rule is scoped by path; or Copilot code review approves, which GitHub allows since 2026-09-01, off by default and limited by path ([GitHub changelog](https://github.blog/changelog/2026-09-01-copilot-code-review-can-now-approve-pull-requests)).  Copilot's approval is an LLM verdict, so it may count only when the deterministic checks are also required (decision 0002).  Also: signed commits required for bots, a merge queue, a strict status-check policy, and no admin bypass for agents' credentials (TD-20).
* 5d, issue to PR: a maintainer-labelled issue starts a plan-driven implementation agent, merged by a human, then by 5c's rules for the classes that earn it.
* Every stage keeps the lethal trifecta split ([Willison](https://simonwillison.net/2025/Jun/16/the-lethal-trifecta/), [Meta, agents rule of two](https://ai.meta.com/blog/practical-ai-agent-security/)): the job that reads untrusted text has no write token, and only maintainers can trigger an agent.  Untrusted text includes issues, comments and web pages, and also a PR's diff and its branch's `CLAUDE.md`, `docs/` and code: claude-code-action 1.0.74 restores only `.claude/` and `.mcp.json` from the base branch.  So a job that reviews PRs gets no write token either.
* GPU checks cannot run on GitHub's hosted runners, and a self-hosted runner on a public repo would run strangers' PR code on the maintainer's machine, which GitHub advises against for public repositories (to be re-checked in GitHub's docs at this phase); so GPU evidence stays on the maintainer's Mac, attached to the PR, and nothing that changes a kernel default auto-merges (GPU-3).
* Acceptance per stage: a planted injection in an issue or comment cannot reach a write token (tested); the jobs stay silent when there is nothing to do.
* Measured by: merge rate per job (published reference points: 46% for Anthropic's internal routines, [The Decoder](https://the-decoder.com/claude-code-now-runs-daily-maintenance-on-anthropics-software-with-a-46-percent-merge-rate/)), reverts within 30 days, rework within 7 days, cost per merged PR.  A job whose merge rate falls below a threshold pauses itself.
* For IAM: this is where the repos part ways.  Expected to carry over: the trifecta split, separate identities for author and approver, required human review on authentication and authorisation paths, signed and attributable bot commits, gated releases.  Not expected to carry over: auto-merge outside docs and dependency metadata.

### Phase 6: measure, simplify, transfer

* Review every component in `harness.md` against what it caught; remove those that catch nothing, re-test the rest when the model changes ("every component encodes an assumption about what the model can't do on its own").
* Write the transfer guide for the IAM codebase from the "For IAM" lessons and the measurements.
* Acceptance: each component has a measured reason to stay, or is gone.

## Validation and acceptance

Per phase, the acceptance lines above.  Overall: against the baseline in `harness.md`, fewer review rounds per PR, fewer vacuous tests reaching review, no CI failure on `main` after a green PR, and fewer corrections per task, with the cost per merged PR known.

## Idempotence and recovery

Every phase lands as small PRs that can be reverted on their own.  The docs check and the structural tests are plain `cargo test`; removing one removes its enforcement and nothing else.  Background jobs (phase 5) each have a kill switch: disabling the workflow.

## Progress

* [x] 2026-10-19: research (20 parallel agents: sources on harness engineering, Claude Code features, agent security, Rust enforcement, review and evaluator agents, gardening; surveys of the docs, rules, tests, history, architecture, GPU and memory code).
* [x] 2026-10-19: decisions 0001 and 0002 accepted by the maintainer.
* [x] 2026-10-19: phase 1, `xtask docs-check` written test-first; it failed on the real drift before the docs were fixed.
* [x] 2026-10-19: phase 1, `CLAUDE.md` map, `docs/` knowledge base, `.claude/rules/`, `.claude/` tracked.
* [x] 2026-10-19: phase 1, review rounds 1 and 2: their findings fixed with tests.
* [x] 2026-10-19: phase 1, `docs/working-with-agents.md`, and a `docs-check` rule that it names every tool.
* [x] 2026-10-19: phase 1, review round 3: its findings fixed with tests.
* [x] 2026-10-19: phase 1, PR #13 merged by the maintainer.
* [x] 2026-10-19: phase 2, execution plan written (`plans/active/harness-phase-2.md`), one milestone per PR.
* [ ] Phase 2.
* [ ] Phase 3.
* [ ] Phase 4.
* [ ] Phase 5a, 5b, 5c, 5d.
* [ ] Phase 6.

## Surprises and discoveries

* 2026-10-19: the docs were accurate where they spoke; every drift found was an omission (a crate, CI's real gates, a package name).  Evidence: 32 sampled claims, 4 omissions, none false.
* 2026-10-19: several rules are already breached without anything noticing: three song-sized copies (TD-12 to TD-14) and an allocating audio callback (TD-15).  Rules in prose do not hold on their own.
* 2026-10-19: roformer-gpu's tests that need no GPU, among them `workgroup_memory_within_defaults` and `sources_are_complete`, have never run in CI (TD-1).
* 2026-10-19: the maintainer's global gitignore ignores `.claude/`, so project rules would silently not be committed; the repo's `.gitignore` now re-includes it.
* 2026-10-19: claude-code-action and GitHub's Copilot coding agent cannot approve PRs, so auto-merge with one maintainer needs a second identity by design.  Copilot code review can approve since 2026-09-01 (opt-in), and the `main` ruleset already runs it on every push; turning its approval on would let an LLM verdict alone merge.

## Decision log

* 2026-10-19, maintainer: knowledge in the repo, Claude Code only (`decisions/0001-harness-in-the-repo.md`).
* 2026-10-19, maintainer: full background autonomy as the target, staged (`decisions/0002-staged-background-autonomy.md`).
* 2026-10-19, maintainer: one general reviewer always, plus specialists by changed path.
* 2026-10-19, agent: the docs check is written in Rust inside `xtask` with no new crate (BUILD-1, DEPS-1), as a test so CI runs it without a workflow change.
* 2026-10-19, agent: no YAML frontmatter on docs in phase 1 (`covers` and `verified` freshness fields are deferred to phase 5a's drift job, where a stale doc becomes an issue rather than a failing build).
* 2026-10-19, maintainer: a guide for the people who direct agents (`docs/working-with-agents.md`), written for an outsider, updated in every phase.
* 2026-10-19, maintainer: the repo's writing standard is ASD-STE100 without its American spelling; personal style preferences stay out of the repo.
* 2026-10-19, maintainer: RT-1 forbids allocation in the audio callback after warm-up (TD-15); `synth`'s file writing moves behind a feature so the app links only song generation (TD-4).
* 2026-10-19, agent: the two older plans (versioned builds, browser tests in Rust) stay out of the repo; they are implemented, their PRs (#6, #9) describe them, and they hold local paths.
* 2026-10-20, maintainer: each check is sized to its stakes and to failures seen.  Phase 2 trims M7 to a token scan, M14 to deny rules and M15 to a `rustfmt` hook, and moves the test-count guard to phase 5b.

## Outcomes and retrospective

To be written when the plan completes.
