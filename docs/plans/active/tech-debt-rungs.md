# Tech debt, in rungs of impact and effort

## Purpose

`docs/tech-debt.md` holds 63 open entries.  Many of them wait for a harness phase, a person or an upstream crate.  Some of them are small test or check gaps that an agent can close with native tests only.  This plan sorts the entries by impact and effort, and closes them in rungs.  Each rung is one PR.  Rung 1 closes the gaps of high impact and low effort.  Rung 2 closes the gaps of high impact and medium effort, which need a browser or a GPU.  Rung 3 is one PR for the wire format (TD-16), since it changes PROTO-1's surface.

Impact: an entry that weakens the enforcement of a strict rule (MEM-*, PROTO-*, GPU-*, LAYER-1, DEPS-1, RT-1), or that can give a wrong result to a user, has high impact.  An entry that concerns only the cost or the measure of the harness has low impact.

Effort: low means native tests or a CI line, with no browser, GPU or model.  Medium means a browser test, a GPU run on the maintainer's Mac, a new tool, or a change of behaviour.  High means a redesign or a new subsystem.

## Context and orientation

The triage of each open entry, on 2026-11-06, at `15c118d`:

| Group | Entries | Why not in a rung |
| - | - | - |
| Rung 1 | TD-22, TD-23, TD-26, TD-39 (one bullet), TD-40, TD-41, TD-42, TD-69 | |
| Rung 2 | TD-7, TD-51, TD-53, TD-57, TD-64, TD-66 | |
| Rung 3 | TD-16 | |
| The next harness PR | TD-47, TD-48, TD-73, TD-75 | The maintainer accepted P1 and P3 of `plans/active/harness-engineering.md` on 2026-11-02, and "that work is the next PR's".  A change to the `review-rounds` skill belongs there. |
| Harness phase 4 | TD-10, TD-11, TD-19, TD-78 | Phase 4 owns them (`plans/active/harness-engineering.md`). |
| Harness phase 5 | TD-20, TD-21, TD-28, TD-31, TD-34, TD-35, TD-46, TD-49, TD-50 | Phase 5 owns them: a sandbox, the base branch's `xtask` and the agents' identity. |
| Harness phase 6 | TD-44, TD-71 | Phase 6 owns them. |
| A person's step | TD-8, TD-36, TD-37, TD-45, TD-54, TD-60, TD-63, TD-72 | A run on an iPhone or the Mac, fixtures from PyTorch, or a choice for the maintainer. |
| Later, high effort | TD-18, TD-39 (the fake driver), TD-55, TD-58, TD-62 | Each one is a design change; each one gets its own plan. |
| Upstream or not possible | TD-52, TD-59, TD-65 | They wait for `flacenc`, `rusty_mp3`, or a way to plant an internal GPU error. |
| No fix planned | TD-3, TD-25, TD-27, TD-29, TD-32, TD-33, TD-38, TD-43, TD-67, TD-68, TD-70, TD-74, TD-76, TD-77 | Each entry says that review or a person covers it. |

Files that rung 1 touches: `.github/workflows/ci.yml`, `roformer-gpu/src/lib.rs` (tests only), `engine/tests/realtime.rs`, `engine/src/player.rs` and `engine/src/stretch/mod.rs` (tests only), `xtask/src/rules.rs`, `xtask/tests/rules.rs`, `xtask/tests/docs.rs`, `xtask/src/review.rs` (`KNOWN_MISSED`), `xtask/src/webdriver.rs` (tests only), `synth/clippy.toml` (new), `docs/tech-debt.md`.

A change to `xtask/src/review.rs` routes all six reviewers (TD-33).  So the removal of TD-69's `KNOWN_MISSED` entry goes in the last commit of M1.

## Plan of work

### M1: rung 1, high impact and low effort (one PR)

Each step is one commit.  Each step adds a test that fails on the gap first, then passes.

1. TD-22 (MEM-3): add a CI step `cargo +nightly-<date> test --locked -p roformer-gpu --doc`, with a pinned nightly.  Nightly rustdoc checks the error code of a `compile_fail,E0XXX` doctest; stable does not.  Acceptance: change `E0505` to `E0308` in `Rec`'s doctest, and the step fails; restore it, and the step passes.  Close TD-22.
2. TD-42 (PROTO-1): add each missing case to the tests of `e2e_messages_use_protocol_types` in `xtask/src/rules.rs`.  The cases are a `SCRIPT_ONLY_TYPES` entry used only in `xtask/src/e2e.rs`, `["type"] <=` in Rust, a macro other than `json!`, a misspelt type first in `includes`, and a `/*` comment over several lines for `js_tokens`.  Acceptance: each new case fails on the mutant that the entry names.  Close TD-42.
3. TD-69 (VERIFY-1): add a test that runs `docs::check` on a fixture repo in a temporary folder, with one known problem for each check.  The test wants each problem.  Remove the TD-69 entry of `KNOWN_MISSED` in the last commit of M1.  Acceptance: the mutant `check -> Ok(vec![])` fails the new test.  Close TD-69.
4. TD-40 (RT-1): add a breach case for each untested branch of `app_stretcher_problems`.  Check the output of `WsolaStretcher::can_pull` for a push over 256 frames.  Bound the warm-up push.  Test the playhead before a loop wrap in `Player::drop_oldest_anchor_when_full`.  Acceptance: a wrong comparison in `drop_oldest_anchor_when_full` fails a test.  Close TD-40.
5. TD-41 (GPU-1): add the three missing cases: the count of read-only storage buffers, the y-axis limit, and an invocation breach that only z causes.  Change the doc comment of `every_layout_matches_its_shaders` to say what it checks.  Acceptance: each case fails on a mutant of its comparison.  If the pairs can come from the code without a device, take them from there and close TD-41.  Else narrow TD-41 to that bullet.
6. TD-23 (LAYER-1): add three checks to `xtask/tests/rules.rs`.  The first fails a `#[path]` or an `include!` whose path leaves its crate.  The second fails a workspace target whose `src_path` is outside its package folder.  The third fails a tracked symbolic link (`git ls-files -s`, mode `120000`).  `include_str!` of text in `xtask`'s tests stays allowed, since `xtask` reads it as text, not code.  Acceptance: a breach case for each check fails.  Narrow TD-23 to the registry copy.
7. TD-26 (MEM-1): add `synth/clippy.toml` with the MEM-1 paths.  Add a test that fails when a shipped crate has no `clippy.toml` with those paths.  `dsp`, `xtask` and `test-support` are exempt, with the reasons of TD-26.  Acceptance: a crate without the file fails the test.  Narrow TD-26.
8. TD-39 (one bullet): give `sends_json_and_reads_a_sized_reply` a read timeout under 30 s, so a client that reads to the end of the stream fails.  Narrow TD-39.

Acceptance of M1: `cargo run -p xtask -- check --tier push` passes; CI passes, the nightly step included; `docs/tech-debt.md` lists TD-22, TD-40, TD-42 and TD-69 under `## Closed`, and TD-41 if step 5 closes it.

### M2: rung 2, high impact and medium effort (one PR)

These steps need a browser, a GPU run on the maintainer's Mac, or a new tool.

1. TD-7 (DEPS-1): vet `cargo-deny`, then add a `deny.toml` with a licence allowlist and a registry allowlist of crates.io only.  Add a CI step that runs it.  Acceptance: a test crate with a GPL licence, and one from a git source, each fail the step.  Close TD-7, or narrow it to the vendored copy.
2. TD-51 (VERIFY-1): run `engine`'s tests on `wasm32` in `xtask web-test`.  Acceptance: a `usize` product in `Filter::apply` fails on `wasm32`.  Close TD-51.
3. TD-66 (VERIFY-1): move the checks of `note_run` and `run` in `xtask/src/e2e/weights.rs` into pure functions, with tests.  Call `gpu` and `load_format` from `worker/src/web/tests.rs`.  Run `cargo mutants` on M9's files on a quiet machine.  Remove the TD-66 entries of `KNOWN_MISSED` in the last commit.  Close TD-66.
4. TD-64: check only the tiles of the stored choice; a first run still checks all four.  GPU-2: check it against the CPU and in a browser.  Acceptance: a test that a stored choice of two tiles compiles two f16 variants.  Close TD-64.
5. TD-57: the player gives its own loop with `Hint::Reading`, and `wanted_blocks` follows it while a seek holds.  Acceptance: a native test that sets a loop behind the head counts no underrun at the old loop's start.  Close TD-57.
6. TD-53 (VERIFY-1): add browser tests that deliver an MP3 and a FLAC reply of the memory path to `App`, with a spy on the link.  Add a test of `store::usage`.  Narrow TD-53 to what is left.

Acceptance of M2: `check --tier push`, `xtask web-test` and CI pass.  The PR names the GPU run of step 4 and the browser.

### M3: rung 3, the wire format (one PR)

1. TD-16 (PROTO-1): the stem worker's decoders refuse a missing required field, as the decode worker's do.  `Alive` carries a protocol version, and the page refuses a worker of another version.  Change the hand-written messages of `xtask e2e` in the same commit (PROTO-1).  Run `e2e stored-stems`, `worker-check` and `weights` on the Mac's GPU, and name the runs in the PR.  Acceptance: a reply with no required field gives an error, not `""`, `0` or `false`.  Narrow TD-16 to what the scan cannot read.

Acceptance of M3: `check --tier push`, `xtask web-test`, CI's `browser-tests` job and the three GPU scenarios pass.

## Validation and acceptance

After each milestone, `cargo run -p xtask -- docs-check` passes, and `docs/tech-debt.md` lists each closed ID under `## Closed`, with what closed it.  After M3, 12 or more entries are closed or narrowed, with no new breach of a rule.

## Idempotence and recovery

Each step is one commit with its own test, so a revert of one commit undoes one step.  If a step's test shows a real breach, stop that step.  Record the breach as a new TD, and go on with the next step.

## Decisions for the maintainer

1. TD-22, the nightly in CI.  Options: A, a pinned nightly in a new CI step; B, `trybuild` as a dev-dependency (a new crate, DEPS-1); C, leave TD-22 open.  Recommendation: A.  Reason: no new crate, and the pin keeps CI stable.  Answer: open.
2. TD-7, a new tool in CI.  Options: A, `cargo-deny` from a pinned release binary; B, from `cargo install --locked` with a pinned version; C, leave TD-7 open.  Recommendation: A, after the DEPS-1 vet.  Reason: a release binary does not build a large tree in CI.  Answer: open.
3. TD-16, a protocol version.  Options: A, a version in `Alive`, and the page refuses a mismatch; B, required fields only, with no version.  Recommendation: A.  Reason: a page and a worker of two builds can meet through a cached worker script.  Answer: open.
4. The mode.  Recommendation: not orchestrated.  Each milestone is one PR and about one session, with the maintainer present for M2's GPU runs.  Answer: open.

## Progress

* [x] 2026-11-06: triage of the open entries, and this plan.
* [ ] M1: rung 1.
* [ ] M2: rung 2.
* [ ] M3: rung 3.

## Surprises and discoveries

* 2026-11-06: TD-73 and TD-75 wait on a choice that the maintainer made on 2026-11-02 (P1 and P3 of `plans/active/harness-engineering.md`).  The work is the next harness PR's.  So this plan does not take them.

## Decision log

* 2026-11-06, agent: the skill items TD-47 and TD-48 go with the next harness PR, not rung 1.  Reason: a change to the `review-rounds` skill belongs with P1 and P3, which change the same skill.

## Outcomes and retrospective

Not started.
