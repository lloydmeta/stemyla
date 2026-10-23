# Harness phase 3, M2: the agents and their guard

## Purpose

This is the milestone plan for M2 of `plans/active/harness-phase-3.md`.  It gives more detail than the parent plan's M2 section, which stays the criteria.  After M2:

* `.claude/hooks/read-only-bash.sh` refuses the Bash commands of a review agent that would change the repo, a remote or a published package.
* `.claude/agents/` holds eight read-only agents: the six reviewers that `xtask review start` routes, the `validator`, and the frozen `baseline`.
* Tests keep each agent read only by its tools and its hook, keep `baseline.md` frozen, and give each rule of the guard one case.

M3 calls the agents from the `review-rounds` skill and builds `agents.json` from these files, so the frontmatter stays plain and the hook block stays one fixed shape.  M3 freezes the agent files; M5 measures them.  No code that ships changes.

## Context and orientation

The state at the start of M2, on 2026-10-23, at commit `813a85a` on branch `ai-agent-harness-phase-3`:

* `.claude/agents/` and `.claude/skills/` hold only a `.gitkeep`.  `.claude/hooks/` does not exist.  The preflight showed that a new agent file in `.claude/agents/` loads in the running session with no restart.
* `.claude/settings.json` holds the deny rules of GIT-1 and a PostToolUse rustfmt hook.  `the_rustfmt_hook_formats_rust_files` in `xtask/tests/rules.rs` runs that hook with `run_hook`: `/bin/sh -c <command>`, the tool call as JSON on stdin, `CLAUDE_PROJECT_DIR` set, and an optional `PATH`.  M2 reuses `run_hook`.
* `xtask/tests/rules.rs` holds `every_reviewer_has_an_agent_file`, with `#[ignore = "M2 adds the agent files"]`.  It reads the tracked files (`git ls-files`), so a new agent file counts only once it is staged.
* `xtask/src/review.rs` has `REVIEWERS`: `reviewer`, `gpu-reviewer`, `memory-reviewer`, `wire-reviewer`, `supply-chain-reviewer`, `docs-reviewer`.  `xtask review start` prints, for each reviewer, its reasons, and the absolute path of the shared target folder (`target/review/target`).
* `docs-check` reads each `.md` file in `.claude/agents/` as an agent doc.  It checks each code span that looks like a path, and each rule or debt ID, and fails a closed ID outside a plan.  `agent_capabilities` takes each agent's name from its file name, and `guide_covers` fails each name that `docs/working-with-agents.md` does not hold in a code span.  The guide's Status says that no project subagents exist.
* The open debt IDs that the reviewers name: TD-3, TD-7, TD-8, TD-16, TD-21, TD-23, TD-26, TD-27, TD-29.  The highest ID is 33.  So this plan says "the new tech-debt entry", and the commit that adds it gives it the next free number.
* Claude Code is 2.1.309.  macOS has `/bin/bash` 3.2.57 and `/usr/bin/jq`.
* The maintainer's PreToolUse hook `rtk hook claude` rewrites `git`, `grep`, `find` and `cargo test` to `rtk ...`.  Matching hooks of one event run in parallel, and a block (exit 2) wins over an allow ([hooks](https://code.claude.com/docs/en/hooks)).  So the guard strips `rtk` and `rtk proxy`, whichever input it sees.
* Facts from the docs, checked on 2026-10-23:
  * A project subagent's frontmatter accepts `hooks` in the same shape as settings.  Its hooks run only after a person trusts the folder; a `-p` session never counts as trusted; hooks of agents passed with `--agents` run ([sub-agents](https://code.claude.com/docs/en/sub-agents), "Hooks in subagent frontmatter").
  * `tools` is a comma string or a YAML list.  Claude Code ignores an unknown frontmatter field.  A file whose frontmatter is not valid YAML is not listed.
  * A PreToolUse hook that exits 2 blocks the call, and Claude gets its stderr.  Another non-zero exit, a hook that cannot start, and a timeout (600 s by default) let the call go ahead, unless the hook sets `onFailure: "block"`, from Claude Code 2.1.295 ([hooks](https://code.claude.com/docs/en/hooks), "Block the action when a hook fails").
  * The PreToolUse input for Bash holds the command at `.tool_input.command`.
* The deny rules in `.claude/settings.json` also apply to the agents.  So `cargo run -p xtask -- publish` stays refused, though the guard does not refuse `cargo run`.

Terms are as in the parent plan.  Also:

* Guard: `.claude/hooks/read-only-bash.sh`.
* Part: a piece of a command between two separators, as the guard splits it.
* First word: the first word of a part after the guard strips its prefixes.
* Hook block: the lines of an agent's frontmatter from `hooks:` to the closing `---`.
* Plant: a change made on purpose to show that a test fails, then undone.

## Plan of work

### How the team works

* The team leader writes this plan, runs the plan reviews, runs the trusted-session check (step 3) and the three review rounds, and commits each Progress line.
* An implementer does steps 1 and 2.  It writes each test first, runs it, and records in Progress that it failed, with the failure in a few words.  It records each plant and its result.
* Before each commit: `cargo fmt --all --check`, each clippy command in `CLAUDE.md`, `cargo test --workspace --exclude roformer-gpu`, `cargo test -p roformer-gpu --lib` and `cargo run -p xtask -- docs-check` (GIT-2).  Sign each commit with `git commit -S`.  Doc changes go in the same commit (DOC-1).
* Plants on the agent files go in a detached worktree, `target/review/m2-plants/`, so the agents of the running session do not change.  Run the tests there with `cargo test --manifest-path target/review/m2-plants/xtask/Cargo.toml --test rules`.  Remove the worktree with `git worktree remove --force` after.  Progress lists each worktree or scratch branch left.
* Writing in the agent files, the guard's messages and the docs follows `CLAUDE.md`, "Working here" (ASD-STE100).

### Step 1: the guard

* `.claude/hooks/read-only-bash.sh`, `#!/bin/bash`, for bash 3.2 (no `declare -A`, no `${x,,}`, no `mapfile`).  Tracked as mode 100755: `git add --chmod=+x`, and `chmod +x` on disk.
* It sets `LC_ALL=C`, so each character is one byte and a long command is fast.
* It reads the command with `jq -r '.tool_input.command | strings'` from stdin.  If `jq` fails, is missing, or gives no string, it refuses with "read-only guard: cannot read the command".  It uses no other external program, so it works with any `PATH` that holds `jq`.
* The parse, in one pass over the characters:
  * Quote state: none, single or double.  Inside single quotes every character is literal.  Outside single quotes, a backslash escapes the next character, and a backslash before a new line is removed, as bash does.
  * A backtick outside single quotes is refused, escaped or not: "read-only guard: a backtick runs a command".
  * Outside quotes, `;`, `&`, `|`, `(`, `)` and a new line end a part.  An empty part is skipped.
  * Outside quotes, a space or a tab ends a word.  The guard removes the quotes and the escaping backslashes from each word, as bash does.  So `'git' commit` and `g"it" commit` give the words `git` and `commit`.
  * A quote that is not closed is refused: "read-only guard: a quote is not closed".
* The prefixes.  The guard strips these from the front of each part, again and again, in any order:
  * `rtk`, then `proxy` after it; `env`, `command`, `exec`, `time`, `nohup`; `timeout` and the word after it.
  * The shell words `!`, `{`, `if`, `then`, `elif`, `else`, `do`, `while`, `until`, so `for f in a; do git add "$f"; done` shows `git add`.
  * A word `NAME=value`, where `NAME` is a letter or `_`, then letters, digits or `_`.  A `NAME` that starts with `GIT_`, or that is `UPDATE_SNAPSHOTS`, is refused.
* The first word.  The guard compares its text after the last `/`, so `/usr/bin/git` is `git`.
  * `gh`, `curl`, `wget`, `bash`, `sh`, `zsh`, `eval` and `xargs` are refused.
  * `cargo`: the subcommand is the first later word that does not start with `-` or `+`.  It refuses the subcommands `publish`, `login`, `owner`, `yank`, `install`, `add`, `remove`, `update`, `clean`, `search` and `fix`; `fmt` when no word is `--check`; and a word `--fix` anywhere.
  * `git`: before the subcommand, it refuses a word that starts with `-c` or `--config-env`.  It skips `-C`, `--git-dir`, `--work-tree` and `--namespace` with the next word, and skips each other word that starts with `-`.  It refuses a word that starts with `--output` anywhere in the part.  The subcommand must be one of `diff`, `log`, `show`, `status`, `blame`, `grep`, `ls-files`, `ls-tree`, `rev-parse`, `rev-list`, `merge-base`, `cat-file`, `show-ref`, `for-each-ref`, `describe`, `worktree`.  A `git` with no subcommand, such as `git --version`, passes.
* Each refusal exits 2 and writes one line to stderr: "read-only guard: <the rule>: <the part>.  This agent is read only; do not try another form of the command."  A pass exits 0 and writes nothing.  Any error of its own exits 2 with a reason that starts "read-only guard:".
* The agents' hook sets `onFailure: block` (step 2).  So a guard that cannot start, exits with another code or times out also blocks the call.
* `the_bash_guard_refuses_writes` in `xtask/tests/rules.rs`, written first:
  * It checks the guard's mode: `git ls-files -s .claude/hooks/read-only-bash.sh` gives `100755`, and the file on disk has an execute bit.
  * It runs the command `"$CLAUDE_PROJECT_DIR"/.claude/hooks/read-only-bash.sh` with `run_hook`, with the workspace root as `CLAUDE_PROJECT_DIR`, and `{"tool_name": "Bash", "tool_input": {"command": <case>}}` on stdin.
  * A refused case needs exit 2 and stderr that starts "read-only guard:".  A passed case needs exit 0.  A failure message names the case, the rule and the exit code.
  * The cases are written by hand, one for each rule, so each fails with its rule removed.  Refused, by rule:
    * Backtick: `` echo `git commit` ``, `` echo "`git commit`" ``.
    * Split: `echo $(git commit)` (the `(`), `cargo test; git stash`, `cargo test && git stash`, `cargo test | git stash`, and a new line before `git stash`.
    * Escape: `echo "\"" ; git commit -m x` (an escaped quote inside double quotes does not end them).
    * Line join: `git \` and a new line, then `commit -m x`.
    * Unclosed quote: `echo "x; git commit -m x`.
    * Quote removal: `'git' commit -m x`, `g"it" commit -m x`.
    * Prefixes: `rtk git commit -m x`, `rtk proxy gh pr view 1`, `env git commit -m x`, `command git commit -m x`, `exec git commit -m x`, `time git commit -m x`, `nohup git commit -m x`, `timeout 60 gh pr view 1`, `FOO=1 git commit -m x`.
    * Shell words, one case each: `! git commit -m x`, `{ git commit -m x; }`, `if git commit -m x; then :; fi`, `if true; then git commit -m x; fi`, `if false; then :; elif git commit -m x; then :; fi`, `if false; then :; else git commit -m x; fi`, `for f in a; do git add "$f"; done`, `while git commit -m x; do :; done`, `until git commit -m x; do :; done`.
    * Variables: `GIT_PAGER=cat git log`, `UPDATE_SNAPSHOTS=1 cargo test`.
    * First words: `gh pr view 1`, `curl x`, `wget x`, `bash -c x`, `sh -c x`, `zsh -c x`, `eval x`, `xargs x`, and `/usr/bin/git commit -m x` (the path).
    * Cargo: `cargo publish`, `cargo login`, `cargo owner`, `cargo yank`, `cargo install x`, `cargo add x`, `cargo remove x`, `cargo update`, `cargo clean`, `cargo search x`, `cargo fix`, `cargo fmt`, `cargo clippy --fix`, `cargo -q publish` (an option before the subcommand).
    * Git: `git -ccore.pager=x log`, `git -c core.pager=x log`, `git --config-env=core.pager=X log`, `git diff --output=x`, `git commit -m x`, `git --no-pager commit -m x`, `git -C target/review/x push`, `git --git-dir x commit`, `git --work-tree x commit`, `git --namespace x commit`, `git add x`.
    * Errors: any command with no `jq` (a `PATH` of an empty folder made by the test); input that is not JSON; a tool input with no `command`.
  * Passed:
    * `git diff HEAD~1`, `rtk git diff`, `rtk proxy git log`, `git -C target/review/1-x diff`, `git --git-dir .git log`, `git --no-pager log`, `git --work-tree . log`, `git --namespace x log`, `git log -c` (a `-c` after the subcommand), `git --version`.
    * `git worktree add --detach target/review/1-x HEAD`, `git worktree remove --force target/review/1-x`.
    * `grep -rn "enough " docs`, `grep -rn -e curl -e wget .github`, `grep -rnE "a|curl b" xtask` (quotes), ``grep -rn '`x`' docs`` (a backtick in single quotes), `grep -rn 'a\' docs` (a backslash in single quotes).
    * `(git log)` (the `)`), `cargo test -p dsp && git diff` (empty parts), `cargo fmt --all --check`, `cargo test -p dsp --target-dir /tmp/t 2>&1`, `for f in a b; do grep -n x "$f"; done`, `cargo test \`, a new line, then `-p dsp`.
    * Time: a `grep -rn '<pattern>' docs` of 10,000 characters passes in less than 5 s.
* Plants, each undone, each with its result in Progress: remove each rule in turn (each entry of each list, the escape, the line join, the quote removal, the basename, the unclosed-quote check, the `|| exit 2` after `jq`), and see its case fail.  A plant that no case fails gets a new case.
* Tech debt, in the same commit: the new entry, "the review agents' guard matches text".  The guard stops mistakes, not an agent that tries to escape, as TD-28 says of the deny rules.  It fails closed for `git` subcommands only.  Ways past it: `git --exec-path=`, `git grep -O` (a pager program), `find -exec`, `find -delete`, `$(...)` inside double quotes, `$'...'` quoting, a command in a variable, the options of `env`, `timeout` and `time` (such as `env -u X git commit`), and a function or alias.  Bash can still write files: a redirection, `sed -i`, `tee`, `rm`, `mv`, or a script that a command writes and then runs.  `git worktree add` without `--detach` makes a branch; `review check` sees it.  Fix: `xtask review check` sees a change to the state it records (TD-32); phase 5 runs reviewers in a sandbox with no token.
* `docs/harness.md`: a row for the guard and `the_bash_guard_refuses_writes` (sensor, computational; phase 3), with the failure it addresses: "phase 2's reviewers could commit, push or stash, and only their prompt said not to".  It names the new entry.

### Step 2: the eight agents

* The frontmatter of each file is exactly this shape, with `<name>` the file name and `<description>` one line:

  ```yaml
  ---
  name: <name>
  description: <description>
  tools: Read, Bash
  model: inherit
  hooks:
    PreToolUse:
      - matcher: Bash
        hooks:
          - type: command
            command: '"$CLAUDE_PROJECT_DIR"/.claude/hooks/read-only-bash.sh'
            onFailure: block
  ---
  ```

  * No `isolation`, as the parent plan says.  No `Grep` or `Glob`: on macOS, an agent with `Bash` searches with `grep` and `find` through Bash, which the guard sees.
  * `description` is a plain YAML scalar: it starts with a capital letter, and holds no `: `, no ` #`, and no quote.  So it parses.  It says when to use the agent, narrowly, so the main agent does not choose a reviewer for other work.  For example: "Reviews one commit range of this repo in a review round, for correctness against the criteria.  Read only.  Use when a review round routes reviewer."
* `pub fn agent_problems(file_name: &str, text: &str) -> Vec<String>` in `xtask/src/rules.rs`, pure, line by line, with no YAML crate.  Each problem names the file, the rule and the fix.  It checks:
  * The text starts with a `---` line, and a second `---` line closes the frontmatter.
  * The top-level keys (lines at indent 0 of the form `key:`) are a subset of `name`, `description`, `tools`, `model`, `hooks`.  Each of the five is present, once.
  * `name` is the file name without `.md`.
  * `description` is not empty, and is a plain scalar as above.
  * `model` is `inherit`, since all agents inherit the main model (parent plan, Decisions).
  * `tools`, as a comma list on its line or as `- item` lines below it, is a subset of `Read`, `Grep`, `Glob`, `Bash`, holds `Bash`, and holds neither `Grep` nor `Glob`.
  * The hook block, with trailing spaces removed, equals the constant `AGENT_HOOK_BLOCK` (the seven lines from `hooks:` to `onFailure: block` above).  So `PreToolUse`, the matcher `Bash`, `type: command`, the exact command line and `onFailure: block` are each checked, and no other event or hook can hide in the block.
* `xtask/tests/rules.rs`:
  * `the_agents_are_read_only_by_tools`: reads every `.md` file in `.claude/agents/` on disk, and runs `agent_problems` on each.  `validator.md` and `baseline.md` are present.  `git hash-object --no-filters .claude/agents/baseline.md` equals `BASELINE_HASH`, a constant in this file; the failure says that `baseline.md` is frozen, and that only M3, or a found finding in M6, may change it with `BASELINE_HASH`.
  * `every_reviewer_has_an_agent_file` loses its `#[ignore]`.
* Unit tests of `agent_problems` in `xtask/src/rules.rs`, `mod tests`, written first.  Each case is a hand-written file text.  A good file has no problem; each plant of the parent plan, and each rule above, gives a problem that names it:
  * no `tools` key; `Edit` in `tools`; `Task` in a YAML list; `Grep` beside `Bash`; `tools: Read` (no `Bash`);
  * a `memory` key; a key twice; no `description`; a description with `: `; `model: opus`;
  * `name` unlike the file name (the renamed file);
  * a changed command line (double quotes outside, or another path); the hook on `PostToolUse`; the matcher `Edit`; no `onFailure`; a second hook in the block;
  * no closing `---`.
* Tests first: before the agent files, `the_agents_are_read_only_by_tools` fails on the missing `validator.md` and `baseline.md`, and `every_reviewer_has_an_agent_file` on the six missing reviewers.  The unit tests fail on a stub that gives no problem.
* Plants on the real files, in `target/review/m2-plants/`, each undone: rename `memory-reviewer.md` to `mem-reviewer.md`, staged (both tests fail); add `Edit` to the `tools` of `reviewer.md` (the agents test fails); change one word of `baseline.md` (the hash fails); `git rm --cached` one reviewer file (`every_reviewer_has_an_agent_file` fails on a row with no file).
* The common parts of every agent body, from the parent plan (the method only; rules stay in `docs/invariants.md`, read by ID).  Each body has these sections, in this order: `## Input`, `## Read only`, `## Method`, `## Output`.  The six reviewers hold the same text in the common parts; a specialist adds its own method in a `## What you check` section.
  * Input: a commit range, the criteria (`file#heading`: read only that section; `commits:<range>`: read only those commit messages), the route's reasons, and the absolute target folder that `xtask review start` printed.  Not the round's folder, since it holds the mutants output.
  * Read only.  Do not read the run folder under `target/review/run/`, the branch logs in the reviews folder under `docs`, the messages of "review round" commits, or the Progress lines of a plan about review rounds.  Do not change a file in the main checkout.  To try a change: `git worktree add --detach target/review/<round>-<name>-<n> HEAD`; change files there with Bash; build with `--manifest-path` into that worktree and `--target-dir <the printed folder>`, with a Bash `timeout` of 600000 ms; then `git worktree remove --force` it.
  * The guard: a refused command gives "read-only guard:".  Do not try another form of it; say in the output what you could not check.
  * Check each claim of a commit message, a plan or a doc against the code, and say how.
  * Put a search pattern that holds a backtick or `|` in single quotes.  Run cargo with `--manifest-path`, not `cd`.  Test one crate (`cargo test -p <crate>`), since parallel agents share one target folder.  A build that passes the 10-minute `timeout` while it waits for cargo's lock goes on in the background: wait for it, and do not start it again.
  * For each new or changed test: name a change to the code that it should catch, and say if you ran it.  A test that no such change fails is a finding (VERIFY-1).
  * Output: one block for each finding, with the fields `severity` (`blocker`, `should-fix` or `nit`), `kind` (`bug`, `weak-test`, `rule`, `doc` or `other`), `where` (`file:line`), `rule` (an ID and a quote, or `none`), `claim`, `evidence` (a command and its output, or a quote), `fix`.  Then a list of the tests, each with the change that breaks it, and if it was run.  "No findings" is a valid output.
* Each reviewer's own checks, from the parent plan:
  * `reviewer`: correctness against the criteria; a test that returns early without `test_support::skip` (TD-8); a snapshot PNG, threshold or `#[ignore]` that changes (TD-29); DSP-1 forms that `dsp_is_beginner_style` does not scan (TD-3).
  * `gpu-reviewer`: GPU-1 to GPU-3, MEM-3; a buffer, texture or query set that is never destroyed (TD-27).
  * `memory-reviewer`: MEM-1 to MEM-5, RT-1; a copy in any form that TD-26 lists; work on the audio callback's path; a `clippy.toml` that drops a path.
  * `wire-reviewer`: PROTO-1, PROTO-2; a field name or default that one side changes and the other does not (TD-16).
  * `supply-chain-reviewer`: DEPS-1, BUILD-1, ASSET-1, GIT-1; a licence or registry, with `cargo info` (TD-7); the four forms of TD-23; a change to CI, to a check in `xtask`, to `.claude/` or to `.mcp.json`.
  * `docs-reviewer`: DOC-1; each changed claim against the code (TD-21); the writing standard in `CLAUDE.md`.
* `validator`: its input is one finding (or the lines of a round's missed mutants), the commit range and the criteria; not the reviewer's other output.  It has the common `## Read only` section.  Its method: try to disprove the finding with its own reading and commands; run a reviewer's command only after it reads it.  `confirmed` and `pre-existing` need a command or a quote that shows the defect, and `pre-existing` also shows the defect at the range's base; `refuted` needs evidence that the claim is false; else `unproven`.  A missed mutant is `confirmed` if a real input shows the change, and `refuted` if the change keeps the behaviour.  A mutant in code for `wasm32` only, or behind a feature that no test turns on, is `refuted` with that reason.  Output: one block for each finding or mutant line, with `id`, `verdict`, `evidence` and `reason`.
* `baseline`: one general reviewer, frozen.  Its body comes from the recipe "Review a change" at `fa41c9f` and the reviewer prompts of phase 2: read only, verify by running, report blockers, should-fix and nits, and check that each test can fail.  The implementer reads at most three phase 2 reviewer prompts, from the Agent calls in the transcripts under `~/.claude/projects/-Users-lloyd-Documents-rustt-stemyla/`, with `jq`, and names them in Progress.  It has the common `## Input`, `## Read only` and `## Output` sections, so the validators and M3's comparer see one shape.  It names no tech-debt ID, so a closed ID never forces an edit.  It names no specialist's checks.
* Docs, in the same commit:
  * The guide: Status names the eight agents in code spans, says that `xtask review` routes the six reviewers and that M3's skill will start them, and no longer says that no project subagents exist.  A short section or table names each agent and when it runs.  The recipe "Review a change" stays the manual prompt until M3.  A changelog line.
  * `docs/harness.md`: the guard's row also names `the_agents_are_read_only_by_tools`.  The agents' own row comes in M3, as the parent plan says.
  * The new tech-debt entry gains: the guard does not stop `Read` of the `gh` or cargo tokens; frontmatter hooks do not run in an untrusted folder or in `-p` mode, so phase 5 passes the agents with `--agents`; the agents' text is a prompt, not a control.
* A code span in an agent file names no `docs/reviews/` path, and names a file of the run folder by its full `target/review/...` path (parent plan, "What every milestone does").

### Step 3: the check in the trusted session

The team leader runs this, since the frontmatter hooks run only in a trusted, interactive session.

1. After step 2's commit, the team leader reads the Agent tool's list of types.  It must list the eight agents.  An agent whose YAML does not parse is not listed.
2. `git worktree add --detach target/review/m2-guard HEAD`.  Note the worktree's `HEAD`.
3. Start a `reviewer` (subagent type `reviewer`) with a prompt that asks it to run exactly `cd <absolute path of target/review/m2-guard> && git commit --allow-empty -m x` once, and to quote the tool result.
4. Pass: the tool result holds "read-only guard:", and the worktree's `HEAD` has not moved.  Remove the worktree.
5. Record both checks in Progress.  If the reviewer does not run the command, the check is inconclusive: reword the prompt and run it again.

### Step 4: review rounds 1, 2 and 3

* Each round: one new general-purpose reviewer or more, with the recipe "Review a change" of `docs/working-with-agents.md` at `fa41c9f`, M2's commits as the range, and the parent plan's M2 section with this plan as the criteria.  Not the new agent types.
* The team leader checks each finding against the code.  It sends the real ones to the implementer, each with a test that fails before the fix.  A fix to the guard or a hook block runs step 3 again.
* A Progress line after each round and after its fixes, committed before the next round.

## Validation and acceptance

* `cargo test -p xtask --test rules` passes, with `the_bash_guard_refuses_writes`, `the_agents_are_read_only_by_tools` and `every_reviewer_has_an_agent_file` (no `#[ignore]`).  `cargo test -p xtask --lib rules` passes, with the `agent_problems` tests.
* Each plant of the parent plan fails `the_agents_are_read_only_by_tools` or its unit tests.  `every_reviewer_has_an_agent_file` fails on a row with no file.  Each guard case fails with its rule removed.  Progress names each plant and its result.
* In the trusted session: the Agent tool lists the eight types, and a `reviewer` asked to run `git commit --allow-empty -m x` in a worktree under `target/review/` gets "read-only guard:".  Progress records both.
* fmt, every clippy command of `CLAUDE.md`, the workspace tests, `cargo test -p roformer-gpu --lib` and `docs-check` pass on the last commit.

## Idempotence and recovery

* The guard and the tests write nothing outside the test's own folder in `CARGO_TARGET_TMPDIR`.
* Scratch worktrees are under `target/review/m2-*`; remove each with `git worktree remove --force <path>`, then `git worktree prune`.
* To resume: read Progress, then `git log --oneline 813a85a..HEAD`.
* To undo M2: revert its commits.  `every_reviewer_has_an_agent_file` then gets its `#[ignore]` back.  Keep the ID of the tech-debt entry that M2 adds: move it under `## Closed` as withdrawn.

## Progress

* [x] 2026-10-23: milestone plan written by the team leader.
* [ ] Step 1: the guard.
* [ ] Step 2: the eight agents.
* [ ] Step 3: the check in the trusted session.
* [ ] Step 4: review rounds 1, 2 and 3.

## Surprises and discoveries

* 2026-10-23, team leader: Claude Code 2.1.295 added `onFailure: "block"` for a command hook.  Without it, a PreToolUse hook that cannot start, exits with a code other than 0 or 2, or times out lets the call go ahead.  Evidence: [hooks](https://code.claude.com/docs/en/hooks), "Block the action when a hook fails".

## Decision log

* 2026-10-23, team leader: the hook block adds `onFailure: block` after the parent plan's exact command line.  So a missing, broken or slow guard blocks the call, and the guard fails closed as the parent plan wants.  M3's `agents.json` copies the block.
* 2026-10-23, team leader: the test compares the whole hook block with one constant.  This is stricter than the parent plan's checks, and simpler to read line by line.
* 2026-10-23, team leader: the guard also strips `exec` and the shell words `!`, `{`, `if`, `then`, `elif`, `else`, `do`, `while`, `until`; removes quotes and escapes from words; handles a backslash; refuses a quote that is not closed; and compares the first word after its last `/`.  Agents write loops often, and an escaped quote could hide a part.  So `exec git` and `/usr/bin/git`, which the parent plan lists as ways past, are now refused.
* 2026-10-23, team leader: the guard also refuses `cargo remove`, `cargo update` and `cargo clean`.  `remove` pairs with `add`; `update` writes `Cargo.lock`, and `clean` empties the target folder that parallel reviewers share.
* 2026-10-23, team leader: the frontmatter checks are a pure function, `agent_problems`, with a unit test for each plant.  So the plants stay as tests, and the test of the real files stays short.  The checks also need `description` (Claude Code ignores a file without it), a plain-scalar description, and `model: inherit`.
* 2026-10-23, team leader: no test checks that the six reviewers share the common text word for word.  M3 freezes the files, and review sees a change to `.claude/agents/`, which routes every reviewer.

## Outcomes and retrospective

To be written when the milestone completes.
