# 0010: how Claude Code's deny rules keep GIT-1

Status: accepted (2026-10-21, by the maintainer)

## Context

Harness phase 2, M14, adds project permission rules in `.claude/settings.json` for GIT-1.  With no rule, Claude Code's auto mode pushes to a branch of the repo with no prompt.  The agent runs with the maintainer's git credentials, and the admin role bypasses the `main` ruleset (TD-20).  So an agent can push to `main`, and CI's `publish` job then publishes the site.  `xtask publish` and `gh pr merge` also publish the site.

Claude Code matches a rule against the command text, after a PreToolUse hook rewrites it.  The maintainer's personal RTK hook rewrites `git push` to `rtk git push`.  It skips its rewrite when a deny rule matches, but not when an ask rule matches.

## Options

* (a) Deny `git push`, `xtask publish` and `gh pr merge`, and their `rtk` and `rtk proxy` forms.  The agent never pushes; a person pushes, with `! git push` in the session or in a terminal.
* (b) Deny `xtask publish` and `gh pr merge`, and ask for `git push`.  Claude Code prompts for each push, also in auto mode, and the agent pushes when the person accepts.
* (c) Take push rights away from the agent's credentials, or sandbox the network.  These do not depend on command text.  But the agent has only the maintainer's credentials, and the sandbox blocks a host, not a command.

## Decision

(a), chosen by the maintainer on 2026-10-21, as the agent recommended.  (c) is deferred to phase 5, where agents get their own identity.

## Consequences

* GIT-1 becomes "an agent never pushes, publishes or merges".  A person pushes for each PR, and for each round of fixes after it opens.
* `.claude/settings.json` holds the deny rules.  A test in `xtask` fails when a rule is missing, or when the file has a key that could make Claude Code drop it.
* The rules match command text only, so they are not a security boundary.  The control is the `main` ruleset and the agents' own identity (phase 5).
* claude-code-action restores `.claude/` from the base branch, so the rules bind the bots of phase 5 too.  Phases 5b and 5c revisit GIT-1 and this decision before a bot pushes or merges.
