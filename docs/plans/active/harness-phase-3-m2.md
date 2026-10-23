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
* Claude Code is 2.1.309.  macOS has `/bin/bash` 3.2.57 and `/usr/bin/jq`.  CI runs the tests on `ubuntu-latest`, with bash 5 and `jq`, so the guard runs under both.
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
* Plants on the agent files go in a detached worktree, `target/review/m2-plants/`, so the agents of the running session do not change.  Run the tests there with `cargo test --manifest-path target/review/m2-plants/xtask/Cargo.toml --test rules`.  Remove the worktree with `git worktree remove --force` after.  The worktree builds cold into its own `target/` (about 140 s and 1.4 GB); Progress records it.  Progress lists each worktree or scratch branch left.
* Writing in the agent files, the guard's messages and the docs follows `CLAUDE.md`, "Working here" (ASD-STE100).

### Step 1: the guard

* `.claude/hooks/read-only-bash.sh`, `#!/bin/bash`, for bash 3.2 (no `declare -A`, no `${x,,}`, no `mapfile`).  Tracked as mode 100755: `git add --chmod=+x`, and `chmod +x` on disk.
* It sets `LC_ALL=C`, so each character is one byte and a long command is fast.
* It reads the command with `cmd=$(jq -er '.tool_input.command | strings' 2>/dev/null) || refuse "cannot read the command"`.  So no `jq`, input that is not JSON, and a missing or non-string `command` each refuse, and no message of `jq` or bash comes before the guard's line.  There is no check for empty output: an empty command has no parts, and passes.  The guard uses no other external program, so it works with any `PATH` that holds `jq`.
* The parse, in one pass over the characters:
  * Quote state: none, single or double.  Inside single quotes every character is literal.  Outside single quotes, a backslash escapes the next character, and a backslash before a new line is removed, as bash does.
  * A backtick outside single quotes is refused, escaped or not.
  * Outside quotes, `#` at the start of a word starts a comment, to the end of the line, as bash does.  The start of a word means that no character and no quote of the word has been read yet.  So `git log # it's here` passes, `a#b` is one word, and `echo a#; git commit` and `echo ''#; git commit` hold no comment.
  * Outside quotes, `;`, `&`, `|`, `(`, `)` and a new line end a part.  An empty part is skipped.
  * Outside quotes, a space or a tab ends a word.  The guard removes the quotes and the escaping backslashes from each word, as bash does.  So `'git' commit` and `g"it" commit` give the words `git` and `commit`.
  * A quote that is not closed is refused.  A heredoc body with an apostrophe is refused this way too; the agents' text says so.
* The prefixes.  The guard strips these from the front of each part, again and again, in any order.  It compares each named prefix word after its last `/`, so `/usr/bin/env` is `env`.  It does not do so for a `NAME=value` word, so `FOO=a/b` stays a variable word.
  * `rtk`, then `proxy` after it; `timeout` and the word after it.
  * `env`, `command`, `exec`, `time`, `nohup`, `export`, `declare`, `typeset`, `readonly`, `local`, each with the words after it that start with `-`.  So `command -p git commit` shows `git commit`, and `export GIT_DIR=x` shows the word `GIT_DIR=x`.
  * The shell words `!`, `{`, `if`, `then`, `elif`, `else`, `do`, `while`, `until`, so `for f in a; do git add "$f"; done` shows `git add`.
  * A word `NAME=value` or `NAME+=value`, where `NAME` is a letter or `_`, then letters, digits or `_`.  A `NAME` that starts with `GIT_`, or that is `UPDATE_SNAPSHOTS`, is refused.
* The first word.  The guard compares its text after the last `/`, so `/usr/bin/git` is `git`.
  * `gh`, `curl`, `wget`, `bash`, `sh`, `zsh`, `eval` and `xargs` are refused (label `command`).
  * `xtask` with a later word `publish` is refused (label `command`).  The deny rules see only `target/*/xtask publish` and `cargo run ... publish`, not an absolute path or `target/review/target/debug/xtask`.
  * `cargo`: it refuses a word that starts with `--config` anywhere in the part, as for `git -c`.  The subcommand is the first later word that does not start with `-` or `+`, after it skips the word that follows the exact words `--color`, `-Z` or `-C` (so `--color=never` is one option word, and skips nothing).  It refuses the subcommands `publish`, `login`, `owner`, `yank`, `install`, `uninstall`, `add`, `remove`, `rm` (an alias of `remove`), `update`, `clean`, `search`, `fix` and `logout`; `fmt` when no word is `--check`; and a word `--fix` anywhere.  Each of these gives the label `cargo subcommand`; `--config` gives `cargo option`.
  * `git`: before the subcommand, it refuses a word that starts with `-c` or `--config-env`.  It skips `-C`, `--git-dir`, `--work-tree` and `--namespace` with the next word, and skips each other word that starts with `-`.  It refuses a word that starts with `--output` anywhere in the part.  The subcommand must be one of `diff`, `log`, `show`, `status`, `blame`, `grep`, `ls-files`, `ls-tree`, `rev-parse`, `rev-list`, `merge-base`, `cat-file`, `show-ref`, `for-each-ref`, `describe`, `worktree`.  A `git` with no subcommand, such as `git --version`, passes.
* Each refusal exits 2 and writes one line to stderr: "read-only guard: <label>: <text>.  <advice>"  `<text>` is the refused part, with each new line as a space, or the whole command, with new lines as spaces, for a refusal before the split.  `cannot read the command` has no text, so its line is exactly "read-only guard: cannot read the command.  <advice>", and its case checks the prefix "read-only guard: cannot read the command.".  `<advice>` for `backtick` and `unclosed quote` is "This is a limit of the guard's parse.  Write the command again without a backtick or an unmatched quote, for example with `sed`."  For each other label it is "This agent is read only.  Do not try another form of the command."  `<label>` is one of a fixed set, one for each refusing rule: `cannot read the command`, `backtick`, `unclosed quote`, `variable`, `command`, `cargo option`, `cargo subcommand`, `git option`, `git output`, `git subcommand`.  A pass exits 0 and writes nothing.  Any other error of its own exits 2 with a line that starts "read-only guard:".
* The agents' hook sets `onFailure: block` (step 2).  So a guard that cannot start, exits with another code or times out also blocks the call.
* `the_bash_guard_refuses_writes` in `xtask/tests/rules.rs`, written first:
  * It checks the guard's mode: `git ls-files -s .claude/hooks/read-only-bash.sh` gives `100755`, and the file on disk has an execute bit.
  * `run_hook` changes to take the stdin text (`&str`), so a case can send input that is not JSON.  `the_rustfmt_hook_formats_rust_files` passes `&input.to_string()`.
  * It runs the command `"$CLAUDE_PROJECT_DIR"/.claude/hooks/read-only-bash.sh` with `run_hook`, with the workspace root as `CLAUDE_PROJECT_DIR`, and `{"tool_name": "Bash", "tool_input": {"command": <case>}}` on stdin.  Each `git` call of the test runs with `current_dir(xtask::workspace_root())`, since `cargo test` runs an integration test in `xtask/`.
  * A refused case needs exit 2, and stderr that starts "read-only guard: <label>:" with the label that the case names.  A passed case needs exit 0 and empty stderr.  A failure message names the case, the expected label and the exit code.
  * The cases are written by hand.  Each rule has a case that, with the rule removed, gives another result: a pass, another label, or a refusal of a passed case.  So a second rule cannot hide the plant.  The skips of `git` options before the subcommand (`--no-pager`, `-C`, `--git-dir`, `--work-tree`, `--namespace`) are caught by the passed cases, since the refused `commit` stays refused without them; Progress names, for each plant, the case that caught it.  Refused, by rule (the label in brackets):
    * Backtick (`backtick`): `` echo `git commit` ``, `` echo "`git commit`" ``.
    * Split (`git subcommand`): `echo $(git commit)` (the `(`), `cargo test; git stash`, `cargo test && git stash`, `cargo test | git stash`, and a new line before `git stash`.
    * Escape outside quotes (`git subcommand`): `echo \"; git commit -m x; echo \"`.  Escape inside double quotes (`git subcommand`): `echo "\"" ; git commit -m "\""`.
    * Line join (`cargo subcommand`): `cargo pub\`, a new line, then `lish`.
    * Comment (`git subcommand`): `git log # x`, a new line, then `git commit -m x` (the comment ends at the new line); `echo a#; git commit -m x` and `echo ''#; git commit -m x` (a `#` inside a word, and after a quote, starts no comment).
    * Unclosed quote (`unclosed quote`): `grep -n "x docs`.
    * Quote removal (`git subcommand`): `'git' commit -m x`, `g"it" commit -m x`; (`cargo subcommand`) `cargo "publish"`.  Backslash removal (`git subcommand`): `\git commit -m x`; (`cargo subcommand`) `cargo pub\lish`.
    * Prefixes (the label of what follows): `rtk git commit -m x`, `rtk proxy gh pr view 1`, `env git commit -m x`, `/usr/bin/env git commit -m x`, `command git commit -m x`, `command -p git commit -m x`, `exec git commit -m x`, `time git commit -m x`, `nohup git commit -m x`, `timeout 60 gh pr view 1`, `FOO=1 git commit -m x`.
    * Shell words, one case each (`git subcommand`): `! git commit -m x`, `{ git commit -m x; }`, `if git commit -m x; then :; fi`, `if true; then git commit -m x; fi`, `if false; then :; elif git commit -m x; then :; fi`, `if false; then :; else git commit -m x; fi`, `for f in a; do git add "$f"; done`, `while git commit -m x; do :; done`, `until git commit -m x; do :; done`.
    * Variables (`variable`): `GIT_PAGER=cat git log`, `GIT_DIR=/tmp/x git log` (a `/` in the value), `UPDATE_SNAPSHOTS=1 cargo test`, `GIT_DIR+=x git log`, `export UPDATE_SNAPSHOTS=1; cargo test`, `export GIT_EXTERNAL_DIFF=x; git diff`, `declare -x GIT_DIR=x`, `typeset GIT_DIR=x`, `readonly GIT_DIR=x`, `local GIT_DIR=x`.
    * First words (`command`): `gh pr view 1`, `curl x`, `wget x`, `bash -c x`, `sh -c x`, `zsh -c x`, `eval x`, `xargs x`, `target/review/target/debug/xtask publish`.  An assignment with a `/` (`git subcommand`): `FOO=a/b git commit -m x`.  The path (`git subcommand`): `/usr/bin/git commit -m x`.
    * Cargo (`cargo subcommand`): `cargo publish`, `cargo login`, `cargo owner`, `cargo yank`, `cargo install x`, `cargo uninstall x`, `cargo add x`, `cargo remove x`, `cargo rm x`, `cargo logout`, `cargo update`, `cargo clean`, `cargo search x`, `cargo fix`, `cargo fmt`, `cargo clippy --fix`, `cargo -q publish` (an option before the subcommand), `cargo --color never publish` (a skipped value).  Cargo (`cargo option`): `cargo --config x test`.
    * Git (`git option`): `git -ccore.pager=x log`, `git -c core.pager=x log`, `git --config-env=core.pager=X log`.  Git (`git output`): `git diff --output=x`.  Git (`git subcommand`): `git commit -m x`, `git --no-pager commit -m x`, `git -C target/review/x push`, `git --git-dir x commit`, `git --work-tree x commit`, `git --namespace x commit`, `git add x`.
    * Errors (`cannot read the command`): any command with no `jq` (a `PATH` of an empty folder made by the test); input that is not JSON; a tool input with no `command`; a `command` that is a number.
  * Passed:
    * `git diff HEAD~1`, `rtk git diff`, `rtk proxy git log`, `git -C target/review/1-x diff`, `git --git-dir .git log`, `git --no-pager log`, `git --work-tree . log`, `git --namespace x log`, `git log -c` (a `-c` after the subcommand), `git --version`, `git lo\`, a new line, then `g` (the line join).
    * `git worktree add --detach target/review/1-x HEAD`, `git worktree remove --force target/review/1-x`.
    * `grep -rn "enough " docs`, `grep -rn -e curl -e wget .github`, `grep -rnE "a|curl b" xtask` (quotes), ``grep -rn '`x`' docs`` (a backtick in single quotes), `grep -rn 'a\' docs` (a backslash in single quotes), `grep -rn "a\"b" docs` (an escaped quote), `grep -n a#b docs` (no comment inside a word), `git log # it's here` (a comment).
    * `(git log)` (the `)`), `cargo test -p dsp && git diff` (empty parts), `cargo fmt --all --check`, `cargo test -p dsp --target-dir /tmp/t 2>&1`, `cargo --color never test -p dsp`, `target/debug/xtask docs-check`, `for f in a b; do grep -n x "$f"; done`, `export FOO=1`, and an empty command.
    * Time: a `grep -rn '<pattern>' docs` of 10,000 characters passes in less than 5 s.  A prototype from plan review took 0.8 s.
* Plants, each undone, each with its result in Progress: remove each rule in turn (each entry of each list, the escape outside and inside double quotes, the line join, the comment, the start-of-word condition of the comment, the `+=` form, the quote removal, the backslash removal, the basename on an assignment word, each basename, the option skip after a prefix, the cargo value skip, the unclosed-quote check, the `||` after `jq`), and see its case fail.  A plant that no case fails gets a new case.
* Tech debt, in the same commit: the new entry, "the review agents' guard matches text".  The guard stops mistakes, not an agent that tries to escape, as TD-28 says of the deny rules.  It fails closed for `git` subcommands only.  Ways past it: `git --exec-path=`, `git grep -O` (a pager program), `find -exec`, `find -delete`, `$(...)` inside double quotes, `$'...'` quoting, a command in a variable, a prefix option that takes a value (such as `env -u X git commit` or `timeout -s KILL 60 gh`), `rtk` subcommands such as `rtk run` and `rtk err`, other variables such as `CARGO_*` and `RUSTC_WRAPPER`, interpreters such as `perl -e` and `awk`, wrappers such as `sudo` and `nice`, and a function or alias.  Bash can still write files: a redirection, `sed -i`, `tee`, `rm`, `mv`, or a script that a command writes and then runs.  `git worktree add` without `--detach` makes a branch; `review check` sees it.  `git worktree remove --force` of a worktree outside `target/review/` can lose a person's work.  A heredoc body with an apostrophe is refused as an unclosed quote, but a second `'` later in the command closes it, and hides the lines between.  `env -S "git commit -m x"` and `function f { git commit -m x; }; f` pass too.  Fix: `xtask review check` sees a change to the state it records (TD-32); phase 5 runs reviewers in a sandbox with no token.
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
  * `description` is a plain YAML scalar: it starts with a capital letter, holds no `: `, no ` #` and no quote, and does not end with `:`.  So it parses.  The description says when to use the agent, narrowly, so the main agent does not choose a reviewer for other work.  For example: "Reviews one commit range of this repo in a review round, for correctness against the criteria.  Read only.  Use when a review round routes reviewer."
  * `hooks:` is the last key, since the hook block runs to the closing `---`.
* `pub fn agent_problems(file_name: &str, text: &str) -> Vec<Problem>` in `xtask/src/rules.rs`, pure, line by line, with no YAML crate, in the style of the other checks there.  Each problem names the file, the rule and the fix.  It checks:
  * The text starts with a `---` line, and a second `---` line closes the frontmatter.
  * The top-level keys (lines at indent 0 of the form `key:`) are a subset of `name`, `description`, `tools`, `model`, `hooks`.  Each of the five is present, once.
  * `name` is the file name without `.md`.
  * `description` is not empty, and is a plain scalar as above.
  * `hooks` is the last key; the problem message says so.
  * `model` is `inherit`, since all agents inherit the main model (parent plan, Decisions).
  * `tools`, as a comma list on its line or as `- item` lines below it, is a subset of `Read`, `Grep`, `Glob`, `Bash`, holds `Bash`, and holds neither `Grep` nor `Glob`.
  * The hook block, with trailing spaces removed, equals the constant `AGENT_HOOK_BLOCK` (the seven lines from `hooks:` to `onFailure: block` above).  So `PreToolUse`, the matcher `Bash`, `type: command`, the exact command line and `onFailure: block` are each checked, and no other event or hook can hide in the block.
* `xtask/tests/rules.rs`:
  * `the_agents_are_read_only_by_tools`: reads every `.md` file in `.claude/agents/` on disk, and runs `agent_problems` on each.  `validator.md` and `baseline.md` are present.  `git hash-object --no-filters .claude/agents/baseline.md`, run in the workspace root, equals `BASELINE_HASH`, a constant in this file.  The failure says that `baseline.md` is frozen from the end of M3, and that a change before then, or for a found finding in M6, updates `BASELINE_HASH` in the same commit.
  * `every_reviewer_has_an_agent_file` loses its `#[ignore]`.
* Unit tests of `agent_problems` in `xtask/src/rules.rs`, `mod tests`, written first.  Each case is a hand-written file text.  A good file has no problem; each plant of the parent plan, and each rule above, gives a problem that names it:
  * no `tools` key; `Edit` in `tools`; `Task` in a YAML list; `Grep` beside `Bash`; `tools: Read` (no `Bash`);
  * a `memory` key; a key twice; no `description`; a description with `: `; a description that ends with `:`; `model: opus`; a key after `hooks`;
  * `name` unlike the file name (the renamed file);
  * a changed command line (double quotes outside, or another path); the hook on `PostToolUse`; the matcher `Edit`; no `onFailure`; a second hook in the block;
  * no closing `---`.
* Tests first: before the agent files, `the_agents_are_read_only_by_tools` fails on the missing `validator.md` and `baseline.md`, and `every_reviewer_has_an_agent_file` on the six missing reviewers.  The unit tests fail on a stub that gives no problem.
* Plants on the real files, in `target/review/m2-plants/`, each undone: rename `memory-reviewer.md` to `mem-reviewer.md`, staged (both tests fail); add `Edit` to the `tools` of `reviewer.md` (the agents test fails); change one word of `baseline.md` (the hash fails); `git rm --cached` one reviewer file (`every_reviewer_has_an_agent_file` fails on a row with no file).
* The common parts of every agent body, from the parent plan (the method only; rules stay in `docs/invariants.md`, read by ID).  Each body has these sections, in this order: `## Input`, `## Read only`, `## Method`, `## What you check` (the six reviewers only), `## Output`.  The six reviewers hold the same text in the common parts.  Each reviewer, `reviewer` too, puts its own checks (below) in `## What you check`.
  * `## Input`: a commit range, the criteria (`file#heading`: read only that section; `commits:<range>`: read only those commit messages), the route's reasons, and the absolute target folder that `xtask review start` printed.  Not the round's folder, since it holds the mutants output.
  * `## Read only`, the same text in all eight files:
    * Do not read the run folder under `target/review/run/`, the branch logs in the reviews folder under `docs`, the messages of "review round" commits, or the Progress lines of a plan about review rounds.
    * Do not change a file in the main checkout.  To try a change: `git worktree add --detach target/review/<round>-<name>-<n> HEAD`; change files there with Bash; build with `--manifest-path` into that worktree and `--target-dir <the target folder of the input>`, with a Bash `timeout` of 600000 ms; then `git worktree remove --force` it.
    * The guard: a refused command gives "read-only guard:".  For a refusal as read only, do not try another form of it; say in the output what you could not check.  For `backtick` or `unclosed quote`, the parse is the limit: write the command again.  Code with a backtick or a `'` (a lifetime, a doc comment), as in a heredoc, trips it.  To write such code in a worktree, use `sed -i` with the text in single quotes, and write a `'` as `'\''`.
    * Put a search pattern that holds a backtick or `|` in single quotes.  Run cargo with `--manifest-path`, not `cd`.  Test one crate (`cargo test -p <crate>`), since parallel agents share one target folder.  A build that passes the 10-minute `timeout` while it waits for cargo's lock goes on in the background: wait for it, and do not start it again.
  * `## Method`, for the six reviewers and `baseline`:
    * Check each claim of a commit message, a plan or a doc against the code, and say how.
    * For each new or changed test: name a change to the code that it should catch, and say if you ran it.  A test that no such change fails is a finding (VERIFY-1).
  * `## Output`: one block for each finding, with the fields `severity` (`blocker`, `should-fix` or `nit`), `kind` (`bug`, `weak-test`, `rule`, `doc` or `other`), `where` (`file:line`), `rule` (an ID and a quote, or `none`), `claim`, `evidence` (a command and its output, or a quote), `fix`.  Then a list of the tests, each with the change that breaks it, and if it was run.  "No findings" is a valid output.
* Each reviewer's own checks, from the parent plan:
  * `reviewer`: correctness against the criteria; a test that returns early without `test_support::skip` (TD-8); a snapshot PNG, threshold or `#[ignore]` that changes (TD-29); DSP-1 forms that `dsp_is_beginner_style` does not scan (TD-3).
  * `gpu-reviewer`: GPU-1 to GPU-3, MEM-3; a buffer, texture or query set that is never destroyed (TD-27).
  * `memory-reviewer`: MEM-1 to MEM-5, RT-1; a copy in any form that TD-26 lists; work on the audio callback's path; a `clippy.toml` that drops a path.
  * `wire-reviewer`: PROTO-1, PROTO-2; a field name or default that one side changes and the other does not (TD-16).
  * `supply-chain-reviewer`: DEPS-1, BUILD-1, ASSET-1, GIT-1; a licence or registry, with `cargo info` (TD-7); the four forms of TD-23; a change to CI, to a check in `xtask`, to `.claude/` or to `.mcp.json`.
  * `docs-reviewer`: DOC-1; each changed claim against the code (TD-21); the writing standard in `CLAUDE.md`.
* `validator`: its `## Input` is one finding (or the lines of a round's missed mutants), the commit range, the criteria and the absolute target folder; not the reviewer's other output.  M3's skill passes the target folder that `review start` printed.  It has the common `## Read only` section.  Its method: try to disprove the finding with its own reading and commands; run a reviewer's command only after it reads it.  `confirmed` and `pre-existing` need a command or a quote that shows the defect, and `pre-existing` also shows the defect at the range's base (`git show <base>:<file>`, or a worktree at `<base>` in place of `HEAD`); `refuted` needs evidence that the claim is false; else `unproven`.  A missed mutant is `confirmed` if a real input shows the change, and `refuted` if the change keeps the behaviour.  A mutant in code for `wasm32` only, or behind a feature that no test turns on, is `refuted` with that reason.  Output: one block for each finding or mutant line, with `id`, `verdict`, `evidence` and `reason`.
* `baseline`: one general reviewer, frozen.  Its body comes from the recipe "Review a change" at `fa41c9f` and the reviewer prompts of phase 2: read only, verify by running, report blockers, should-fix and nits, and check that each test can fail.  The implementer reads at most three phase 2 reviewer prompts, from the Agent calls in the transcripts under `~/.claude/projects/-Users-lloyd-Documents-rustt-stemyla/`, with `jq`, and names them in Progress.  It takes round 1 prompts, and not those of M5's six cases (PR #9, M9, M11, M2, M10 and M18a), since a later round's prompt can name an earlier round's findings, which are M5's ground truth.  It has the common `## Input`, `## Read only`, `## Method` and `## Output` sections, so the validators and M3's comparer see one shape.  It names no tech-debt ID, so a closed ID never forces an edit.  It names no specialist's checks.
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
5. A tool result with a refusal by auto mode or by a permission rule, and no "read-only guard:", fails the check: the guard did not stop the call, since hooks run before the permission check.  The team leader reports it, and does not try the action another way.  Only a reviewer that did not call Bash makes the check inconclusive: reword the prompt and run it again.
6. The `onFailure` probe.  Write the untracked file `.claude/agents/preflight-probe.md`, a name that the maintainer's auto-mode allow entry permits.  It has the frontmatter of step 2 with `name: preflight-probe`, but its command names a missing script, `.claude/hooks/missing.sh`.  Start a `preflight-probe` agent, and ask it to run `ls` once and quote the result.  Pass: the call is blocked, and the result holds `blocking because onFailure is "block"`.  Then delete the file; `docs-check` fails while it exists.  If the call runs, record the gap in the new tech-debt entry, and change the Decision log line on `onFailure`.
7. Record each check in Progress.

### Step 4: review rounds 1, 2 and 3

* Each round: one new general-purpose reviewer or more, with the recipe "Review a change" of `docs/working-with-agents.md` at `fa41c9f`, M2's commits as the range, and the parent plan's M2 section with this plan as the criteria.  Not the new agent types.
* The team leader checks each finding against the code.  It sends the real ones to the implementer, each with a test that fails before the fix.  A fix to the guard or a hook block runs step 3 again.
* A Progress line after each round and after its fixes, committed before the next round.

## Validation and acceptance

* `cargo test -p xtask --test rules` passes, with `the_bash_guard_refuses_writes`, `the_agents_are_read_only_by_tools` and `every_reviewer_has_an_agent_file` (no `#[ignore]`).  `cargo test -p xtask --lib rules` passes, with the `agent_problems` tests.
* Each plant of the parent plan fails `the_agents_are_read_only_by_tools` or its unit tests.  `every_reviewer_has_an_agent_file` fails on a row with no file.  Each guard case fails with its rule removed.  Progress names each plant and its result.
* In the trusted session: the Agent tool lists the eight types, and a `reviewer` asked to run `git commit --allow-empty -m x` in a worktree under `target/review/` gets "read-only guard:".  A probe agent whose hook names a missing script is blocked by `onFailure: block`.  Progress records each.
* fmt, every clippy command of `CLAUDE.md`, the workspace tests, `cargo test -p roformer-gpu --lib` and `docs-check` pass on the last commit.

## Idempotence and recovery

* The guard and the tests write nothing outside the test's own folder in `CARGO_TARGET_TMPDIR`.
* Scratch worktrees are under `target/review/m2-*`; remove each with `git worktree remove --force <path>`, then `git worktree prune`.
* To resume: read Progress, then `git log --oneline 813a85a..HEAD`.
* To undo M2: revert its commits.  `every_reviewer_has_an_agent_file` then gets its `#[ignore]` back.  Keep the ID of the tech-debt entry that M2 adds: move it under `## Closed` as withdrawn.

## Progress

* [x] 2026-10-23: milestone plan written by the team leader (`854f9a7`).
* [x] 2026-10-23: plan review round 1, by two new reviewers (feasibility, with a bash 3.2 prototype of the guard; completeness).  No blockers; 7 should-fix, 12 nits, with overlap; all checked and fixed.  The significant ones: two guard cases passed with their rule removed, so each refusal now names a label that its case asserts; `jq` needs `-e` and no stderr; nothing checked `onFailure: block`, so step 3 adds a probe agent; a refusal by auto mode fails step 3; the validator gets the target folder.  Also: comments, `export`, `cargo --config` and `--color`, the basename of prefixes; more ways past in the debt entry; `hooks` is the last key; the baseline reads round 1 prompts outside M5's cases.
* [x] 2026-10-23: plan review round 2, by one new reviewer, with its own bash 3.2 prototype of the guard.  No blockers; 1 significant should-fix, 3 other should-fix, 9 nits; all checked and fixed.  The significant one: no case showed that a comment starts only at the start of a word, and a wrong guard passed `echo a#; git commit`.  Also: `NAME+=value`; the advice of a `backtick` or `unclosed quote` refusal lets the agent write the command again; `run_hook` takes text, so a case can send input that is not JSON.
* [x] 2026-10-23: plan review round 3, by one new reviewer, with its own bash 3.2 prototype of the guard.  No blockers; 2 significant should-fix, 2 other should-fix, 4 nits; all checked and fixed.  The significant ones: a basename on a `NAME=value` word, and a backslash kept in a word, each passed every case and let a write through; each now has cases and a plant.  Also: `cargo rm`, `uninstall` and `logout`; `xtask publish` from a path that the deny rules miss; the exact line of `cannot read the command`; more ways past in the debt entry; CI runs the guard under bash 5.
* [ ] Step 1: the guard.
* [ ] Step 2: the eight agents.
* [ ] Step 3: the check in the trusted session.
* [ ] Step 4: review rounds 1, 2 and 3.

## Surprises and discoveries

* 2026-10-23, team leader: Claude Code 2.1.295 added `onFailure: "block"` for a command hook.  Without it, a PreToolUse hook that cannot start, exits with a code other than 0 or 2, or times out lets the call go ahead.  Evidence: [hooks](https://code.claude.com/docs/en/hooks), "Block the action when a hook fails".

## Decision log

* 2026-10-23, team leader: the hook block adds `onFailure: block` after the parent plan's exact command line.  So a missing, broken or slow guard blocks the call, and the guard fails closed as the parent plan wants.  Step 3 checks this with a probe agent.  M3's `agents.json` copies the block, with the guard's absolute path.
* 2026-10-23, team leader: the test compares the whole hook block with one constant.  This is stricter than the parent plan's checks, and simpler to read line by line.
* 2026-10-23, team leader: the guard also strips `exec` and the shell words `!`, `{`, `if`, `then`, `elif`, `else`, `do`, `while`, `until`; removes quotes and escapes from words; handles a backslash; refuses a quote that is not closed; and compares the first word after its last `/`.  Agents write loops often, and an escaped quote could hide a part.  So `exec git` and `/usr/bin/git`, which the parent plan lists as ways past, are now refused.
* 2026-10-23, team leader (plan review round 1): the guard also treats a comment as bash does, applies the basename to prefixes, skips the options after a prefix, strips `export`, `declare`, `typeset`, `readonly` and `local` so their variables are checked, refuses `cargo --config`, and skips the value of `cargo --color`, `-Z` and `-C`.  Each refusal names a label, and each case asserts it, so one rule cannot hide the plant of another.
* 2026-10-23, team leader: M5's tripwire "splits and strips each Bash command as the guard does".  The guard's parse is now larger than the parent plan's.  M5 uses the guard's rules as M2 leaves them; the simplest way is to run the guard on each command.  A tripwire with the guard's parse has the guard's blind spots.
* 2026-10-23, team leader: the guard also refuses `cargo remove` (and its alias `rm`), `uninstall`, `logout`, `update` and `clean`, and `xtask publish`.  `remove` pairs with `add`, `uninstall` could remove cargo-mutants, and `logout` pairs with `login`; the deny rules miss `xtask publish` from an absolute path; `update` writes `Cargo.lock`, and `clean` empties the target folder that parallel reviewers share.
* 2026-10-23, team leader: the frontmatter checks are a pure function, `agent_problems`, with a unit test for each plant.  So the plants stay as tests, and the test of the real files stays short.  The checks also need `description` (Claude Code ignores a file without it), a plain-scalar description, and `model: inherit`.
* 2026-10-23, team leader: no test checks that the six reviewers share the common text word for word.  M3 freezes the files, and review sees a change to `.claude/agents/`, which routes every reviewer.

## Outcomes and retrospective

To be written when the milestone completes.
