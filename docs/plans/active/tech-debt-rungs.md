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

Files that rung 1 touches: `.github/workflows/ci.yml`, `CLAUDE.md`, `README.md`, `roformer-gpu/src/lib.rs` (tests only), `engine/src/player.rs` (tests only), `engine/tests/realtime.rs`, `synth/clippy.toml` (new), `synth/src/lib.rs` (the canary), `xtask/src/check.rs`, `xtask/src/docs.rs`, `xtask/src/rules.rs`, `xtask/src/review.rs` (`KNOWN_MISSED`), `xtask/src/webdriver.rs` (tests only), `xtask/tests/check.rs`, `xtask/tests/rules.rs`, `.claude/agents/supply-chain-reviewer.md`, `.claude/rules/`, and the docs: `docs/README.md`, `docs/harness.md`, `docs/invariants.md` and `docs/tech-debt.md`.

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

1. TD-22, the nightly in CI.  Options: A, a pinned nightly in a new CI step; B, `trybuild` as a dev-dependency (a new crate, DEPS-1); C, leave TD-22 open.  Recommendation: A.  Reason: no new crate, and the pin keeps CI stable.  Answer: none on 2026-11-06; the agent applied A, with `nightly-2026-10-30` (Decision log).  On 2026-11-07, the maintainer accepted A and the pin.
2. TD-7, a new tool in CI.  Options: A, `cargo-deny` from a pinned release binary; B, from `cargo install --locked` with a pinned version; C, leave TD-7 open.  Recommendation: A, after the DEPS-1 vet.  Reason: a release binary does not build a large tree in CI.  Answer: open.
3. TD-16, a protocol version.  Options: A, a version in `Alive`, and the page refuses a mismatch; B, required fields only, with no version.  Recommendation: A.  Reason: a page and a worker of two builds can meet through a cached worker script.  Answer: open.
4. The mode.  Recommendation: not orchestrated.  Each milestone is one PR and about one session, with the maintainer present for M2's GPU runs.  Answer: 2026-11-06, the maintainer: do M1 while away, with no PR, and give the work as patches.

## Progress

* [x] 2026-11-06: triage of the open entries, and this plan.
* [x] M1: rung 1.  On 2026-11-06, the 8 steps are committed: `efe2b8d` TD-22, `979fbe4` TD-42, `913b306` TD-69, `967c181` TD-40, `01bc133` TD-23, `b40b89e` TD-41, `9358c14` TD-26 and `c1224c1` TD-39.  The code and the review rounds are done.
  * [x] 2026-11-07: CI passed on `5fe83fc` of PR 42, the nightly step included.
  * [x] 2026-11-07: `check --tier push` passed on the maintainer's Mac, as a user that is not root.  The root-only test and the GPU line ran too.
  * [x] 2026-11-08: cargo-mutants on the diff of the branch, on the maintainer's Mac: 270 mutants, 156 caught, 103 missed, 10 unviable, 1 timeout.  Of the missed mutants, 82 were in the MEM-1 canary visitor.  No test put a cfg that stays on around a canary, and many node kinds had no case.  New tests and the removal of 19 arms that cannot change the result fix them.  New cases also catch 17 mutants of `disallowed_methods`, `source_includes`, `attribute_paths`, `turns_off` and `expected_copies`.  A second run on the changed functions gave 4 missed, all equivalent (Decision log).
  * [x] Review round 1: 8 should-fix and 8 nits; fixes in `f961fdc`.
  * [x] Review round 2: 4 should-fix and 4 nits; fixes in `31adccf`.
  * [x] Review round 3: 6 should-fix, 2 of them refuted, and 4 nits; fixes in `3fde93b`.
  * [x] Review round 4: 4 should-fix and 3 nits; the should-fix fixed in `5c47302`.
  * [x] Review round 5: 2 should-fix in the canary walker, fixed in `d2fb3e7`.
  * [x] Review round 6: 3 should-fix in the canary walker, fixed in `2bd1a58`.
  * [x] Review round 7: 3 should-fix; `06b1595` replaces the walker with a `syn` visitor.
  * [x] Review round 8: 2 should-fix, fixed in `9fc0845`: more nodes, and `quote::ToTokens`.
  * [x] Review round 9: 1 should-fix, closure parameters, fixed in `a4f3ed2`.
  * [x] Review round 10: 1 should-fix, untyped patterns and field patterns, fixed in `d89fa18`.
  * [x] Review round 11: 1 pre-existing should-fix, variadic parameters, fixed in `c9bc622`.
  * [x] Review round 12: no findings.  The stop rule of decision 0024 holds.  Log: `docs/reviews/claude-tech-debt-planning-thr8px.md`.
  * [x] Review round 13, on the maintainer's Mac, of `5fe83fc`, `4ab3a1a` and `52afb99`: 2 should-fix and 1 nit; the should-fix fixed in `250851c`.  The `Range` arm stays, and `NODES` gets a type generic parameter.  Round 12's record was seeded by hand with head `c9bc622`, because rounds 1 to 12 ran in the cloud container.
  * [x] Review round 14: no findings, and no missed mutant.  The stop rule of decision 0024 holds.  The log stays in the same file.
  * [x] 2026-11-08: `check --tier push` passed on the tree of the last commit of M1, on the maintainer's Mac.
  * [x] 2026-11-08: CI passed on `5b3fbf6`, the last code commit of M1, the nightly step included (run 42405974754).
* [ ] M2: rung 2.
* [ ] M3: rung 3.

## Surprises and discoveries

* 2026-11-06: `a_failed_worktree_removal_keeps_its_branch_and_run_folder` (`xtask/tests/clean.rs`) fails in a cloud container, also at `15c118d`.  The container runs as root, and a read-only folder does not stop root.  The local runs of M1 skip that test only.
* 2026-11-06: `cargo metadata` gives a target's `src_path` with the `..` of its `path`, such as `decoder/../dsp/src/lib.rs`.  So a check that compares the start of the path passes it.  A plant found this, and `targets_stay_in_their_crate` normalises the path.
* 2026-11-06: the nightly doctests and the default build filled the container's disk.  Builds then ran with `CARGO_INCREMENTAL=0`.

* 2026-11-06: TD-73 and TD-75 wait on a choice that the maintainer made on 2026-11-02 (P1 and P3 of `plans/active/harness-engineering.md`).  The work is the next harness PR's.  So this plan does not take them.

## Decision log

* 2026-11-06, agent: no answer came to question 1, so step 1 applied the recommendation, a pinned nightly.  `nightly-2026-10-30` ran the doctests in this container.
* 2026-11-06, agent: the nightly line goes in `never_run` of `xtask check`, not in tier `push`.  Reason: a machine without the nightly can then still run tier `push`; CI runs the line.
* 2026-11-06, agent: step 3 removed the TD-69 entry of `KNOWN_MISSED` in its own commit, not in the last commit of M1.  Reason: `each_known_missed_entry_is_open_and_in_the_code` fails an entry whose TD is closed, so the entry goes with the close.  No review round had started, so the route of a round did not widen.
* 2026-11-06, agent: TD-23 keeps three items, not only the registry copy of step 6: a renamed `include!`, and an `include_str!` or `include_bytes!` of another crate's file.  Reason: review rounds 1 and 2 found them, and a check of either needs name resolution or a list of allowed text files.
* 2026-11-07, agent: review round 7 replaced the token walker of the MEM-1 canary check with a `syn` visitor.  Reason: rounds 4 to 7 each found a new form that the walker missed, such as a brace in a generic argument or a `match` in a statement.  DEPS-1: `syn` 2.0.119 was already in `Cargo.lock`, through `burn-derive`, `zerocopy-derive` and other derive crates.  The lock gains only the edge from `xtask`, which does not ship.
* 2026-11-06, agent: TD-41 stays open for the hand-copied pairs.  Reason: to take the pairs from `Kernels::new` changes how the kernels are built, which needs GPU-2's checks on a real GPU.

* 2026-11-07, maintainer: accepted `syn` 2.0.119 and `quote` 1.0.47 as dependencies of `xtask` (DEPS-1).  Accepted the `nightly-2026-10-30` pin of TD-22.
* 2026-11-07, agent: the commits of M1 were applied again from patches, so each commit has a new hash.  This plan and the review log now give the new hashes.
* 2026-11-08, agent: the MEM-1 canary visitor loses 19 match arms, and each kind falls to `_ => &[]`.  Reason: the mutant of each arm survived a test of every node kind.  syn gives the attributes of an `Assign`, `Binary` or `Cast` to its left operand (`syn` 2.0.119, `stmt.rs` and `expr.rs`).  Review round 13 found that syn keeps a range statement's attributes on the `Range`, so its arm stays, with a test.  A literal, `continue`, `_`, `..`, `extern crate` or `use` holds no code.  The check does not read a macro, and only a macro makes a `Group`.  syn does not parse an or-pattern as a closure parameter.
* 2026-11-08, agent: 4 mutants of `expected_copies` stay missed, as equivalent.  A `let` that ends in `.to_vec()` always has an `=` before it, so `map_or(n - 3, ..)` and `.min(n - 3)` change nothing.  `at * 1` adds only the `=`, which is not a type name.  The timeout of `index *= 2` is a loop that never ends.

* 2026-11-06, agent: the skill items TD-47 and TD-48 go with the next harness PR, not rung 1.  Reason: a change to the `review-rounds` skill belongs with P1 and P3, which change the same skill.

## Outcomes and retrospective

* M1, 2026-11-07: the 8 steps landed, and 12 review rounds ran.  Rounds 4 to 11 each found a `#[cfg]` form that the MEM-1 canary check missed.  The `syn` visitor of round 7 stopped the parse errors.  Rounds 8 to 11 then added each `syn` node kind that holds attributes and can hold a canary.  Lesson: a check that reads Rust source starts with a parser, not a token walker.
* The mutants runs of M1 tested no mutant in this container, because the baseline fails as root.  Each fix has a hand mutation that the new test catches.  The first local run on 2026-11-08 then missed 103 of 270 mutants.  Lesson: a hand mutation shows that a test fails, but not that each branch has a test.  A test of a skip also needs its kept case, or a mutant that skips everything passes.
