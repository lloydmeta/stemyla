# Harness phase 3b, M7: the PR body and attention comments (decision 0027)

## Purpose

This is the milestone plan for M7 of `plans/active/harness-phase-3b.md`.  The parent's M7 section stays the criteria.  M7 puts decision 0027 into the harness.

After M7:

* `.github/pull_request_template.md` gives a PR body a summary for people, "Needs your attention", "Not verified", "Verification" and a `<details>` block for agents.
* `xtask pr-body <file>` checks a body against that form.  The orchestrator runs it before each `gh pr create` and `gh pr edit --body-file`.
* `xtask attention <base> <plan>` lists the diff lines and the agent decisions that can get an inline comment at close-out.  The hand-run lines come first.
* The `orchestrate` skill has the PR body steps and the attention review, and its pins hold their key lines.

Readers: the implementer of M7, the review agents of M7's rounds, and the orchestrator.

## Context and orientation

The state at the start of M7, on 2026-11-01, at commit `0e64a34` on branch `harness-phase-3b`:

* The review base of M7 is `8b2c073`.  The orchestrator's commits `05a814b` to `0e64a34` (M6's intent review and close-out) come after it.  They change the parent plan, the `orchestrate` skill, `PLANS.md`, `CLAUDE.md` and `xtask/tests/rules.rs`, and they are not M7 work.  Round 1 sees them in its range.

* `.github/` has `workflows/`, `actions/` and `dependabot.yml`.  It has no PR template.  `xtask/src/review.rs` routes `.github/**` to the supply-chain-reviewer.
* `xtask/src/main.rs` sends each subcommand to a module of `xtask/src/lib.rs`.  `check`, `clean` and `review` have a `main(args) -> i32`.
* `xtask/src/check.rs`: `STRICT_PATHS` holds the globs of each strict rule, `strict_paths(rule)` gives one row, and `crate::docs::glob_matches(glob, path)` matches a glob.  `app/src/lib.rs` is in no row (TD-70).
* `.claude/rules/protocol.md`: `stored-stems`, `worker-check` and `weights` write stem worker messages by hand, and CI does not run them (TD-16).  Their messages are in `xtask/src/e2e.rs`, in `xtask/src/e2e/weights.rs`, and in `xtask/e2e/worker.js` (`e2e.ask`).  In `e2e.rs`, they are in the top-level items whose names start with `stored_stems` or `worker_check`, and in `fn stem_protocol_file`.  The messages are defined in `protocol/src/lib.rs`.  The decode messages (`protocol/src/decode.rs`) are run by CI (`e2e storage`, `e2e flac`).
* Decision log entries of this phase have the forms `* <date>, agent:`, `agent (orchestrator):`, `agent (team leader):` and `agent, M5, before any run:`.  The older milestone plans also use `team leader:` and `implementer, step 2:`.  The maintainer's entries are `maintainer:` and `maintainer, D1:`.  The parent's M7 section takes an author that starts with `agent`, then `:`, ` (` or `,`.
* `docs/working-with-agents.md`: `guide_covers` fails if the guide does not name each `xtask` subcommand in a code span.  So the guide names `pr-body` and `attention` in the commit that adds them.
* `xtask/tests/rules.rs`: `ORCHESTRATE_SKILL_TEXT` pins the orchestrate skill, and `the_orchestrate_pins_fail_on_a_changed_skill` changes one pinned line at a time.
* The orchestrate skill's "Start" says "M7 of harness phase 3b adds the form of the body".
* The maintainer's PR body draft is `target/harness-phase-3b-resume/pr-body.md`.  The team does not edit it.
* The orchestrator posts the `COMMENT` review at the phase's close-out.  M7 does not post anything on GitHub, and it does not do the parent's "first step" probe (the orchestrator's brief).
* No new crate is needed (DEPS-1).

Terms:

* Candidate: one line that `xtask attention` gives: a class, a path, a line and a side of the diff, and the reasons.
* Hand-run line: a diff line that changes a message that `stored-stems`, `worker-check` or `weights` writes.
* Agent entry: a Decision log entry that an agent wrote.

## Plan of work

### How the team works

* The team leader writes this plan.  A new `Plan` agent reviews it, at most 2 rounds.  The team leader commits it, signed.
* One implementer (`general-purpose`) does step 1 and step 2.  The team leader writes step 3 for the orchestrator.
* The team does not edit `.claude/**`, `CLAUDE.md` or `docs/plans/active/harness-phase-3b.md`.
* Before each commit: `cargo run -p xtask -- check --tier commit`.  Before each review round: `cargo run -p xtask -- check --tier push` and `cargo build -p xtask`.  Each commit is signed.  Write in ASD-STE100.
* Each new function starts as a stub that gives a wrong answer.  Each new test fails first on the stub, on an assert.  Each test has a fail case.  The implementer records the failures in Surprises.  No commit holds a failing test.

### Step 1: the template and `xtask pr-body` (one commit)

`.github/pull_request_template.md`: the four headings and the `<details>` block, each with one HTML comment that says what goes there.

* `## Summary`: for people, what changed and why, at most 150 words.
* `## Needs your attention`: only the items for the maintainer to decide or to do.  A tool install and a hand run are items to do.  An agent decision that changes a rule, a permission, a tool or an accepted decision is an item to decide (`PLANS.md`).  A decision item says what to decide and the default that the agent took.  An item links to its inline comment where one exists.  Else "Nothing".
* `## Not verified`: what no automated check covers, such as Safari, a real GPU or an iPhone.  Else "Nothing".
* `## Verification`: 2 or 3 lines.
* `<details><summary>For agents</summary>` to `</details>`: the milestones; the commands and results; the review rounds with their mutants lines; each `reported, not fixed` finding; each `known` finding once, with its TD; every agent decision; the rule and debt IDs; the plan and decision paths.

`xtask/src/pr_body.rs`: `pub fn problems(body: &str) -> Vec<String>` and `pub fn main(args: &[String]) -> i32`.

* It removes each HTML comment (`<!--` to `-->`, also over lines) first, as GitHub does not show one.  It reads `\r\n` as `\n`.
* The part for people is the text before the first line that is `<details><summary>For agents</summary>` (trimmed).  No such line gives a problem.  No `</details>` line after it gives a problem.
* Each of the four headings is one line in the part for people, exactly once, in the order above.  Each missing heading gives its own problem.  A heading twice gives a problem.  Headings out of order give one problem.  Other headings are allowed.
* The summary is the text from `## Summary` to the next heading of level 1 or 2 (`# ` or `## `, then text), or the end of the part.  A `###` line and `#39` stay in the summary.  Its words are the tokens between whitespace.  0 words gives "the summary is empty".  More than 150 gives "the summary has <n> words, over the limit of 150".
* The Verification section has 1 to 3 lines that are not blank.  Decision 0027 asks for 2 or 3; the check takes 1, so the maintainer's draft, with one full line, passes.
* Each message names the rule (decision 0027) and a fix.
* `main`: one argument, the file.  It prints each problem and exits 1, or prints one pass line and exits 0.  A wrong argument count or a file that it cannot read exits 2 with the usage or the error.
* The stub of `problems` gives `["stub"]` for each body.  Each fail case asserts its exact problem text.
* Tests in `mod tests`: a good body passes; a summary of 150 words passes and of 151 fails; each missing heading fails; a heading twice fails; two headings out of order fail; a heading that is only in `<details>` is missing; no `<details>` line fails; no `</details>` fails; the words of an HTML comment do not count; 4 Verification lines fail; a body with `\r\n` passes.  The template (`include_str!`) gives exactly two problems: "the summary is empty" and "the Verification section is empty".
* `xtask/tests/pr_body.rs` runs the binary: a good body in a temp file exits 0; the template exits 1 and prints the two problems; no argument exits 2.
* The implementer runs `xtask pr-body` on `target/harness-phase-3b-resume/pr-body.md`, and records the result in Surprises.
* `main.rs` and its usage line get `pr-body`.  `docs/working-with-agents.md`: a tools table row for `pr-body`, a line in "Before you merge" (the body follows the template, and `pr-body` passes), and a changelog line.

### Step 2: `xtask attention` (one commit)

`xtask/src/attention.rs`.  Pure functions:

* `pub fn changed_lines(diff: &str) -> Vec<Changed>`: parses a unified diff with `-U0`.  Each changed line has the path, the side (`RIGHT` for an added line, `LEFT` for a removed line), its line on that side, the new-side position of its hunk (`hunk_new`), the new-side count of its hunk (`hunk_new_count`), its hunk number, its text and the old path of a rename (`rename from`).  The path is the `+++ b/` path, or the `--- a/` path of a deleted file.  It skips `\ No newline at end of file` and a binary file.
* `pub fn candidates(diff: &str, files: &[(String, String)], plans: &[(String, String)]) -> Vec<Candidate>`.  `files` holds the text at `HEAD` of each changed path.  `plans` holds the plan and its milestone plans.
* A candidate's line is the first changed line of its hunk, file or entry that meets its class.

The classes, in their order of output:

1. `hand-run`, for each hunk:
   * of `protocol/src/lib.rs` or `xtask/e2e/worker.js`;
   * of `xtask/src/e2e/weights.rs` above its `#[cfg(test)]` line;
   * of a top-level item of `xtask/src/e2e.rs` that writes or reads a hand-run message, in the text at `HEAD`.  An item starts at column 0 with `fn`, `pub fn`, `const` or `static`.  Its name starts with `stored_stems` or `worker_check`, in upper or lower case, or it is `fn stem_protocol_file`.  So `STORED_STEMS_SESSION`, `stored_stems_clip` and `stored_stems_checks` count.  `fn open_on_the_worker_check`, a decode-worker log check, does not.  A `fn` ends at the next line that is exactly `}` (column 0, not trimmed).  A `const` or `static` ends at its first line that ends with `;`.  A removed line of a hunk with new lines uses its hunk's new-side position.  A removed line of a hunk that only removes lines (`+<n>,0`) is the gap between the lines `n` and `n + 1`.  It is above the tests when `n` is less than the `#[cfg(test)]` line.  It is in an item `[first, last]` when `first <= n` and `n + 1 <= last`.  So a removal just above `#[cfg(test)]` is a hand-run line, and a removal just before a `fn` line or just after its `}` is not.  A field line with no `type` key is a message line too (TD-16: a renamed field fails nothing).
   * Reason: "a message that stored-stems, worker-check or weights writes by hand; CI does not run them (TD-16): ask a person for the run".
2. `change`:
   * for each file: `docs/decisions/*.md` (a proposed decision, when its text holds `Status: proposed`; else a change to a decision); `docs/invariants.md`; `CLAUDE.md`; `.claude/**`; `.github/workflows/**` and `.github/actions/**` (CI);
   * for each hunk of `xtask/src/rules.rs` with a changed line in `SKIP_ALLOWLIST` (from its `const` line to the next line that is `];`): an allowlisted skip.
3. `hot-spot`:
   * for each hunk: a path of the rows PROTO-2, MEM-1 to MEM-5 and RT-1 of `check::STRICT_PATHS`, by `strict_paths`; `app/src/lib.rs` (TD-70); `protocol/src/**` (the page-worker messages);
   * for each added line of a `.rs` file, outside a `//` line, with `unsafe` that has the line start, a space, `(` or `{` before it, and a space or `{` after it, or that has `[` before it and `(` after it (`#[unsafe(no_mangle)]`).  So `unsafe_code` and `--enable-unsafe-webgpu` are not hits;
   * for each added line in a dependency table of a `Cargo.toml`: a section whose header ends in `dependencies]` or holds `dependencies.`, by the text at `HEAD`.  A blank line or a comment is not a dependency.
4. `judgement`, for each agent entry in the `## Decision log` section of a file of `plans`:
   * An entry is a line `* <YYYY-MM-DD>, <author>: ...` (the author is the text up to the first `:`).  It is an agent entry when the author is `agent`, or starts with `agent (` or `agent,` (the parent's M7 section).
   * Its line is the entry line, if that line is an added line of the diff.  Else it has no line, and the output says "no diff line; list it in the PR".
   * Reason: the author and the first 80 characters of the text.

The rule for renames, once for all classes (`class_path`):

* Each class (`hand-run`, `change`, `hot-spot`) reads both paths of a rename.  It reads the new path first.  If the new path meets no class, it reads the old path, with the reason "renamed from <old>: <reason>".
* A path with no hunk can meet only `change` or `hot-spot`.  It is never a `hand-run` candidate, since no message text changed.
* For `hand-run` with a hunk, the path picks the rule (`protocol/src/lib.rs`, `worker.js`, `weights.rs`, `e2e.rs`).  The line tests read the text at `HEAD` of the new path.
* The test `each_class_reads_both_paths_of_a_rename` holds one row for each case.  Each row is a diff section and its exact candidates.

Then the candidates are merged and sorted:

* Candidates with the same path, line and side become one, with the first class and each reason once.
* The sort is by class, then path, then line.  A candidate with no line comes last in its class.

`pub fn main(args: &[String]) -> i32`, for `xtask attention <base> <plan>`:

* It runs `git diff --no-color --no-ext-diff -M -U0 <base>...HEAD` at the repo root.  It reads each changed path at `HEAD` with `git show`.  A path that is not at `HEAD` (a deleted file) gets an empty text, not an error.  The plans are `<plan>` and each `docs/plans/{active,completed}/<stem>-m<n>.md` at `HEAD`.
* It prints one line for each candidate: `<class> <path>:<line> <side>: <reasons>`, or `<class> <path> (<note>; list it in the PR): <reasons>`.  The note is `no diff line` or `no hunk`.  The last line gives the count and "pick at most 10, the hand-run lines first (decision 0027)".  It exits 0.  A wrong argument count or a `<base>` that starts with `-` exits 2.  A git error or a plan outside the repo exits 1.  A path with no hunk gives a candidate with no line when it meets the `change` or `hot-spot` class.  Examples are a pure rename, a mode change, a new empty file and a binary file.

Tests in `mod tests`, each on a stub that gives no candidate.  Each negative case is in a diff with a positive case, and each test asserts the exact list of candidates, so it fails on the stub:

* each class gives its line: `protocol/src/lib.rs`; a type line in `fn stored_stems`, after an indented `}` line, with `fn smoke` before it in the file; a line in `fn stored_stems_checks`, `fn stored_stems_clip`, `const STORED_STEMS_SESSION`, a `pub fn` and a `static` item; a field-only line in `fn worker_check` and in `weights.rs`; `xtask/e2e/worker.js`; a proposed and an accepted decision; `docs/invariants.md`, `CLAUDE.md`, `.claude/skills/x/SKILL.md`, `.github/workflows/ci.yml`; a line in `SKIP_ALLOWLIST`; `app/src/separator.rs`, `engine/src/song.rs`, `engine/src/player.rs`, `app/src/lib.rs`; an added `unsafe {`; an added dependency; a path with no hunk of each class;
* each glob of the PROTO-2, MEM-1 to MEM-5 and RT-1 rows of `STRICT_PATHS`, with `**` and `*` filled in, gives a `hot-spot`.  So a new row path is covered;
* none for: a docs-only diff (`docs/working-with-agents.md`, `docs/README.md`); a change in `fn smoke` and in another function of `e2e.rs`; a line after the `;` of a multi-line `const`; a no-hunk `docs/README.md`; a line of `weights.rs` below `#[cfg(test)]`; a `--enable-unsafe-webgpu` line; a line outside `SKIP_ALLOWLIST` in `rules.rs`; `// unsafe` and `#![forbid(unsafe_code)]`; a `[package]` version in `Cargo.toml`;
* the Decision log: each form `agent:`, `agent (orchestrator):`, `agent (team leader):`, `agent (implementer):` and `agent, M5, before any run:` gives a `judgement`; `maintainer:`, `maintainer, D1:`, `team leader:`, `agents:` and `agentic:` give none; an entry line in `## Progress` gives none; an agent entry that is not in the diff has no line;
* order: a diff with a `.claude/` file, `app/src/lib.rs`, an agent entry with a line, an agent entry with no line and a `protocol/src/lib.rs` hunk, more than 10 candidates in all.  The output is exactly: the hand-run line first, then `change`, then `hot-spot`, then `judgement`, with the no-line entry last;
* merge: one hunk of `worker/src/web.rs` gives one candidate with the reasons of PROTO-2, MEM-2 and MEM-5.  One hunk of `protocol/src/lib.rs` gives one `hand-run` candidate that also holds the PROTO-2 reason;
* the parse: a removed-only hunk gives `LEFT` and the old line; a context line moves both sides (`a_context_line_moves_both_sides`); a rename gives the new path and its old path; a rename out of a class path gives its candidate, by its old path; a deleted file gives the old path; `\ No newline at end of file` gives no line.

`xtask/tests/attention.rs` runs the binary in a temp repo of two commits.  The second commit changes `protocol/src/lib.rs` and `docs/README.md`, deletes a file, adds a row to `[workspace.dependencies]` of the root `Cargo.toml`, adds a proposed decision, and adds an agent entry to a plan.  A milestone plan holds an agent entry that is not in the diff.  The output is exactly: the `hand-run` line, the `change` line of the decision, the `hot-spot` line of the dependency, the `judgement` line, and the no-line `judgement` entry, with no line of `docs/README.md`.  It exits 0.  The plan as `./<plan>`, `././<plan>`, an absolute path, an absolute path through a symlink, and a path from `docs/` (also with `../`) give the same output.  A base that git does not know exits 1.  No argument, one argument, or a `<base>` that starts with `-` exits 2.

The implementer runs `xtask attention origin/main docs/plans/active/harness-phase-3b.md` on this branch, and records the count of each class in Surprises.

`main.rs` and its usage line get `attention`.  `docs/working-with-agents.md`: a tools table row for `attention`, the recipe "Run a big task hands-off" says what the inline comments are, and a changelog line.

### Step 3: the texts for the orchestrator (team leader)

In the orchestrator's scratchpad folder `m7/`:

* `claude.diff`: `.claude/skills/orchestrate/SKILL.md` and `xtask/tests/rules.rs`, in one diff.  `ORCHESTRATE_SKILL_TEXT` pins these lines, and `the_orchestrate_pins_fail_on_a_changed_skill` changes some of them:
  * the body follows the template, and `xtask pr-body` runs before each `gh pr create` and `gh pr edit --body-file`, also between milestones;
  * "Needs your attention" holds only the items to decide or to do; the `<details>` block lists every agent decision;
  * at close-out, after the last fix and the last push, `xtask attention` lists the candidates;
  * one review with event `COMMENT`, through the GitHub MCP server's pending review or `gh api`, with at most 10 inline comments, the hand-run lines first;
  * each comment says what to decide and the default that the agent took;
  * plain PR comments that link to the lines, if GitHub refuses the review.
* `pr-body.md`: the maintainer's draft in the full form: 2 or 3 Verification lines, "Needs your attention" with only the items to decide or to do, and in `<details>` the moved agent decisions and the paths of the Decision logs of the plan and of each milestone plan.  `xtask pr-body` passes on it.
* `parent.md`: the text for the parent plan, if any.

## Validation and acceptance

* `cargo test -p xtask` passes, with the tests of step 1 and step 2.  Each failed first on its stub (Surprises).
* `cargo run -p xtask -- pr-body target/harness-phase-3b-resume/pr-body.md` exits 0, and so does it on `m7/pr-body.md`.  The template exits 1, with only those two problems.
* `cargo run -p xtask -- attention origin/main docs/plans/active/harness-phase-3b.md` lists candidates of each class that this branch has.
* `cargo run -p xtask -- docs-check` and `cargo run -p xtask -- check --tier push` pass.
* After the orchestrator applies `claude.diff`, `the_orchestrate_skill_keeps_its_key_lines` and `the_orchestrate_pins_fail_on_a_changed_skill` pass.

## Idempotence and recovery

Each step is one commit that `git revert` undoes.  `pr-body` and `attention` change nothing.  A new implementer resumes from this plan and its Progress.

## Progress

* [x] 2026-11-01: this plan.
* [x] 2026-11-01: Step 1: the template and `xtask pr-body`.
* [x] 2026-11-01: Step 2: `xtask attention`.
* [x] 2026-11-02: Step 3: the texts for the orchestrator (`m7/claude.diff`, `m7/pr-body.md`).  The orchestrate pins pass with the diff applied, and `xtask pr-body` passes on both bodies.  Tier `push` passed before review round 1 (11 checks, 172 s).
* [ ] Review rounds.

## Surprises and discoveries

* 2026-11-01, plan review round 1 (one `Plan` agent): 8 should-fix and 2 nits, all fixed in this plan: the hand-run lines, the body of a function, stubs that pass, a deleted file, the agent decisions of the older plans, the PR body's own form, the skill's key lines, the merge and order tests, the `unsafe` match and the start commit.
* 2026-11-01, agent (implementer), step 1: on the stub `["stub"]`, all 17 tests of `pr_body::tests` failed on an assert.  In `xtask/tests/pr_body.rs`, the good body and the template failed on an assert; the usage test passed, as it does not call `problems`.
* 2026-11-01, agent (implementer), step 1: `xtask pr-body target/harness-phase-3b-resume/pr-body.md` passes (exit 0).  The template exits 1 with two problems: the summary is empty, and the Verification section is empty.
* 2026-11-01, agent (implementer), step 2: on the stubs, all 15 tests of `attention::tests` failed on an assert.  Two fixtures in my tests were wrong, and I corrected them before the code was written.
* 2026-11-01, agent (implementer), step 2: `xtask attention origin/main docs/plans/active/harness-phase-3b.md` at 33c9ca6 gives 98 candidates: `hand-run` 0, `change` 18, `hot-spot` 2 (an `unsafe` in a test string of `review.rs`, and `set_var` in `tests/clean.rs`), `judgement` 78 (55 of the parent, 9 of M5, 9 of M6, 5 of M7).  Each agent entry is a new line of this branch, so none has "no diff line".  M1 to M4 write `team leader:`, so they give none.
* 2026-11-02, agent (team leader): at `419c38b`, the same command gives 103 candidates: `change` 18, `hot-spot` 6, `judgement` 79.  The extra hot spots are the `unsafe` matches in the new tests of `attention.rs`, which hold `unsafe` in test strings.
* 2026-11-02, agent (implementer), review round 1: on the old code, the new tests failed on an assert: the `stored_stems_checks`, `#[unsafe(...)]` and removal-after-a-body cases, the `### More` summary, `./<plan>`, and a `<base>` of `--output=x` (exit 1, not 2).  The mutant tests (1-1, 1-3, 1-8, 1-9, 1-18, 1-19, 1-23, 1-25, 1-26) passed on the old code, as they pin its behaviour.
* 2026-11-02, agent (implementer), review round 2: on the old code, these new cases failed on an assert.  A removal just above `#[cfg(test)]` gave no line.  A removal just before `fn stored_stems(` gave a line.  A line of `fn stem_protocol_file` gave no line.  A plan path through a symlink exited 1 with "not in the repo".  The `..` tests of `repo_path` passed on the old code.
* 2026-11-02, agent (implementer), review round 3: on the old code, two new tests failed on an assert.  A line of `const STORED_STEMS_SESSION` and of `fn stored_stems_clip` gave no line.  A path with no hunk gave no candidate.  The test of a removal at the line of a not-in-the-diff entry passed on the old code.
* 2026-11-02, agent (implementer), review round 4: on the old code, three new tests failed on an assert.  A rename out of a class path gave no candidate.  A path with no hunk gave a `hand-run` candidate.  `fn open_on_the_worker_check` gave a `hand-run` line.  The `git mv`, last-section, ` b/` and one-line `const` tests passed on the old code.
* 2026-11-02, agent (implementer), review round 5: on the old code, 3 of the 36 rows of `each_class_reads_both_paths_of_a_rename` failed on an assert.  Each was a rename with a hunk out of a `hand-run` path: `protocol/src/lib.rs`, `xtask/e2e/worker.js` and `weights.rs` (5-2).  The `change` rows with a hunk passed on the old code (5-1).

## Decision log

* 2026-11-01, agent (team leader): the team does not post the probe review of the parent's "first step".  The orchestrator posts the real `COMMENT` review at the phase's close-out, and uses plain PR comments if GitHub refuses it (the orchestrator's brief).
* 2026-11-01, agent (team leader): `xtask attention` takes only the authors that start with `agent`, as the parent's M7 section says (315f610).  So this plan and its implementer write `agent (team leader):` and `agent (implementer):`.  The `team leader:` entries of the older milestone plans are not read, and those plans are complete.
* 2026-11-01, agent (team leader): a hand-run line is any changed line in the body of `fn stored_stems(` or `fn worker_check(`, or in `weights.rs` above its tests, not only a line with a `type` key.  Reason: the plan review found field lines with no `type` key, and TD-16 says that a renamed field fails nothing.
* 2026-11-01, agent (team leader): `xtask pr-body` checks that the Verification section has 1 to 3 lines, besides the parent's three checks.  Decision 0027 asks for 2 or 3, but the maintainer's draft has one full line, and the orchestrator's brief says that the check passes on a real body.  The check does not check the links of "Needs your attention", since a comment exists only after close-out.
* 2026-11-01, agent (implementer): the template gives two problems, not one: the summary is empty, and the Verification section is empty.  Reason: the check takes 1 to 3 Verification lines, so 0 is a problem, and the template has only a comment there.  The tests assert both problems.
* 2026-11-01, agent (implementer): a judgement with no diff line prints `(no diff line; list it in the PR)`.  Reason: the plan gives both `(no diff line)` and "no diff line; list it in the PR", and this text holds both.
* 2026-11-02, agent (implementer): `xtask attention` makes `<plan>` relative to the repo root: it joins it to the current folder, removes `.` and `..` parts, and removes the root.  So `./<plan>`, an absolute path and a path from a subfolder give the same output.  `git` resolves `HEAD:./<path>` from the current folder too, but the plan path must match the diff path.
* 2026-11-02, agent (implementer): `repo_path` makes a plan path canonical when it exists, and removes `..` parts by text when it does not.  I removed its `.` arm, since `components` drops each `.` part, and a comment says so.  Each changed line has one span (low, high).  A removal of a removal-only hunk spans the gap.  `inside` and the `#[cfg(test)]` check of `weights.rs` use that span.
* 2026-11-02, agent (implementer): the hand-run rule of `e2e.rs` is a goal, not a list of names.  It takes each top-level item whose name starts with `stored_stems` or `worker_check`, and `fn stem_protocol_file`.  Reason: each review round found one more helper (`stored_stems_checks`, `stem_protocol_file`, `STORED_STEMS_SESSION`).  `pub fn` and `static` items have tests too.
* 2026-11-02, agent (implementer): a path with no hunk gives one candidate, with the first class that it meets and the reasons of each class.  It is never a `hand-run` candidate, since it changes no message text (review round 4).  A rename is classed by its new path, and by its old path when the new path meets no class.  Its note is `no hunk`, so the list tells it from a judgement with `no diff line`.
* 2026-11-02, agent (implementer): the rename rule is one goal for all classes, and one helper, `class_path`, applies it.  Each class reads a path only through `class_path`: the new path, else the old path.  Reason: rounds 3, 4 and 5 each found one more rename or no-hunk case.  A table test holds a row for each class and each kind of change.

## Outcomes and retrospective

To be filled in at the end.
