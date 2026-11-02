//! `xtask attention <base> <plan>`: lists the diff lines and the agent
//! decisions that can get an inline comment at close-out (decision 0027).
//!
//! The code has two parts, so the tests can check the rules without a
//! repo.  [`changed_lines`] and [`candidates`] are pure functions of the
//! diff, the text at `HEAD` of each changed path, and the plans.  [`main`]
//! reads them with `git`.  The orchestrator picks at most 10 candidates,
//! the `hand-run` lines first.
//!
//! The rule for renames, once for each class ([`class_path`]):
//!
//! * Each class (`hand-run`, `change`, `hot-spot`) reads both paths of a
//!   rename.  It reads the new path first.  If the new path does not
//!   meet the class, it reads the old path, with the reason
//!   "renamed from <old>: <reason>".
//! * A path with no hunk can meet only `change` or `hot-spot`
//!   ([`no_hunk_paths`]).  It is never a `hand-run` candidate, since no
//!   message text changed.
//! * For `hand-run` with a hunk, the path picks the rule.  The line tests
//!   read the text at `HEAD` of the new path.
//!
//! `plans/completed/harness-phase-3b-m7.md` (step 2) gives the classes and
//! their rules.

use crate::check::{repo_root, strict_paths};
use crate::docs::glob_matches;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

pub const USAGE: &str = "usage: xtask attention <base> <plan>";

/// The exit code of a git error or a plan outside the repo.
const FAILED: i32 = 1;

/// The exit code of a usage error.
const REFUSED: i32 = 2;

/// The side of a diff line, as GitHub names it for a review comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    /// A removed line, on the old side.
    Left,
    /// An added line, on the new side.
    Right,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "LEFT",
            Side::Right => "RIGHT",
        }
    }
}

/// One changed line of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    /// The new path, or the old path of a deleted file.
    pub path: String,
    pub side: Side,
    /// The line on its side, from 1.
    pub line: usize,
    /// The new-side position of the line's hunk (the `+<n>` of `@@`).
    pub hunk_new: usize,
    /// The new-side count of the line's hunk (the `,<m>` of `+<n>,<m>`).
    /// It is 0 for a hunk that only removes lines.
    pub hunk_new_count: usize,
    /// The number of the line's hunk in the diff, from 0.
    pub hunk: usize,
    /// The line's text, without the `+` or `-`.
    pub text: String,
    /// The old path of a renamed file (`rename from`), or none.
    pub renamed_from: Option<String>,
}

/// The classes of a candidate, in their order of output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    /// A message that `stored-stems`, `worker-check` or `weights` writes
    /// by hand.
    HandRun,
    /// A change to a decision, a rule, the agent harness or CI.
    Change,
    /// A trust boundary or a correctness hot spot.
    HotSpot,
    /// An agent's judgement call, from a Decision log.
    Judgement,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::HandRun => "hand-run",
            Class::Change => "change",
            Class::HotSpot => "hot-spot",
            Class::Judgement => "judgement",
        }
    }
}

/// One line that can get an inline comment.  A judgement whose entry line
/// is not in the diff has no line and no side.  So has a file with no
/// hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub class: Class,
    pub path: String,
    pub line: Option<(usize, Side)>,
    /// Why a candidate has no line: [`NO_DIFF_LINE`] or [`NO_HUNK`].
    pub no_line: &'static str,
    pub reasons: Vec<String>,
}

/// The note of a judgement whose entry line is not in the diff.
pub const NO_DIFF_LINE: &str = "no diff line";

/// The note of a file with no hunk.
pub const NO_HUNK: &str = "no hunk";

impl Candidate {
    /// The output line: `<class> <path>:<line> <side>: <reasons>`, or
    /// `<class> <path> (<note>; list it in the PR): <reasons>`.  The note
    /// is `no diff line` or `no hunk`.
    pub fn render(&self) -> String {
        let place = match self.line {
            Some((line, side)) => format!("{}:{line} {}", self.path, side.name()),
            None => format!("{} ({}; list it in the PR)", self.path, self.no_line),
        };
        format!("{} {place}: {}", self.class.name(), self.reasons.join("; "))
    }
}

/// The reason of a `hand-run` candidate.
pub const HAND_RUN: &str = "a message that stored-stems, worker-check or weights writes by hand; CI does not run them (TD-16): ask a person for the run";

/// The reason of a path under `protocol/src/`.
pub const MESSAGES: &str = "the page-worker messages (protocol/src/**)";

/// The reason of `app/src/lib.rs`.
pub const APP_LIB: &str =
    "app/src/lib.rs holds memory and transfer code, and no strict row holds it (TD-70)";

/// The reasons of the `change` class.
pub const PROPOSED: &str = "a proposed decision: accept or reject it";
pub const DECISION: &str = "a change to a decision";
pub const RULE: &str = "a change to a rule (docs/invariants.md)";
pub const MAP: &str = "a change to the map (CLAUDE.md)";
pub const HARNESS: &str = "a change to the agent harness (.claude/)";
pub const CI: &str = "a change to CI";
pub const ALLOWLISTED: &str = "a change to an allowlisted skip (SKIP_ALLOWLIST, VERIFY-1)";

/// The reasons of an added line in the `hot-spot` class.
pub const UNSAFE: &str = "an added `unsafe`";
pub const DEPENDENCY: &str = "an added dependency (DEPS-1)";

/// The rules whose strict paths are hot spots.
pub const HOT_RULES: [&str; 7] = [
    "PROTO-2", "MEM-1", "MEM-2", "MEM-3", "MEM-4", "MEM-5", "RT-1",
];

/// Each changed line of a unified diff that `git diff -U0` gives.  It
/// skips `\ No newline at end of file` and a binary file.
pub fn changed_lines(diff: &str) -> Vec<Changed> {
    let mut out = Vec::new();
    let (mut old_path, mut path) = (String::new(), String::new());
    let (mut old_line, mut new_line, mut hunk_new, mut hunks) = (0, 0, 0, 0);
    let mut hunk_new_count = 0;
    let mut renamed_from: Option<String> = None;
    // The lines left in the current hunk, on each side.  Inside a hunk, a
    // line such as `--- x` is a removed line, not a header.
    let (mut old_left, mut new_left) = (0usize, 0usize);
    for line in diff.lines() {
        if old_left > 0 || new_left > 0 {
            let mut push = |side, line: &mut usize, text: &str| {
                out.push(Changed {
                    path: path.clone(),
                    side,
                    line: *line,
                    hunk_new,
                    hunk_new_count,
                    hunk: hunks - 1,
                    text: text.to_string(),
                    renamed_from: renamed_from.clone(),
                });
                *line += 1;
            };
            if let Some(text) = line.strip_prefix('-') {
                push(Side::Left, &mut old_line, text);
                old_left = old_left.saturating_sub(1);
                continue;
            }
            if let Some(text) = line.strip_prefix('+') {
                push(Side::Right, &mut new_line, text);
                new_left = new_left.saturating_sub(1);
                continue;
            }
            if line.starts_with('\\') {
                continue;
            }
            if line.starts_with(' ') {
                old_line += 1;
                new_line += 1;
                old_left = old_left.saturating_sub(1);
                new_left = new_left.saturating_sub(1);
                continue;
            }
            // A line of no hunk: the hunk is over.
            (old_left, new_left) = (0, 0);
        }
        if line.starts_with("diff --git ") {
            old_path.clear();
            path.clear();
            renamed_from = None;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            renamed_from = Some(from.to_string());
        } else if let Some(p) = line.strip_prefix("--- ") {
            old_path = p.strip_prefix("a/").unwrap_or(p).to_string();
        } else if let Some(p) = line.strip_prefix("+++ ") {
            path = match p {
                "/dev/null" => old_path.clone(),
                _ => p.strip_prefix("b/").unwrap_or(p).to_string(),
            };
        } else if let Some(((old_at, old_n), (new_at, new_n))) = hunk_ranges(line) {
            (old_line, old_left) = (old_at, old_n);
            (new_line, new_left, hunk_new) = (new_at, new_n, new_at);
            hunk_new_count = new_n;
            hunks += 1;
        }
    }
    out
}

/// The path of each file section of `diff` that has no hunk, with the old
/// path of a rename: a pure rename, a mode change, a new empty file or a
/// binary file.
pub fn no_hunk_paths(diff: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    // The current file section: its path, its old path, and if it has a
    // hunk.  A line of a hunk starts with `+`, `-`, ` ` or `\\`, so it is
    // not a header.
    let mut section: Option<(String, Option<String>, bool)> = None;
    for line in diff.lines() {
        if let Some(paths) = line.strip_prefix("diff --git ") {
            if let Some((path, from, false)) = section.take() {
                out.push((path, from));
            }
            let path = paths.rsplit_once(" b/").map_or(paths, |(_, b)| b);
            section = Some((path.to_string(), None, false));
        } else if let Some((path, from, has_hunk)) = section.as_mut() {
            if let Some(to) = line.strip_prefix("rename to ") {
                *path = to.to_string();
            } else if let Some(old) = line.strip_prefix("rename from ") {
                *from = Some(old.to_string());
            } else if line.starts_with("@@ ") {
                *has_hunk = true;
            }
        }
    }
    if let Some((path, from, false)) = section {
        out.push((path, from));
    }
    out
}

/// The old and new (start, count) of a line `@@ -a[,b] +c[,d] @@`.  A
/// missing count is 1.
fn hunk_ranges(line: &str) -> Option<((usize, usize), (usize, usize))> {
    fn range(text: &str) -> Option<(usize, usize)> {
        let (start, count) = text.split_once(',').unwrap_or((text, "1"));
        Some((start.parse().ok()?, count.parse().ok()?))
    }
    let mut words = line.strip_prefix("@@ ")?.split(' ');
    let old = range(words.next()?.strip_prefix('-')?)?;
    let new = range(words.next()?.strip_prefix('+')?)?;
    Some((old, new))
}

/// Each candidate of `diff`, merged and sorted.  `files` holds the text at
/// `HEAD` of each changed path.  `plans` holds the plan and its milestone
/// plans, as (path, text).
pub fn candidates(
    diff: &str,
    files: &[(String, String)],
    plans: &[(String, String)],
) -> Vec<Candidate> {
    let lines = changed_lines(diff);
    let text_of = |path: &str| {
        files
            .iter()
            .find(|(p, _)| p == path)
            .map_or("", |(_, text)| text.as_str())
    };
    let hunks: Vec<&[Changed]> = lines.chunk_by(|a, b| a.hunk == b.hunk).collect();
    let mut found = Vec::new();
    for hunk in &hunks {
        let text = text_of(&hunk[0].path);
        let hit = class_path(&hunk[0], |path| hand_run_line(path, hunk, text));
        if let Some((first, from)) = hit {
            found.push(candidate(
                Class::HandRun,
                first,
                renamed(vec![HAND_RUN.to_string()], from),
            ));
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for changed in &lines {
        if seen.contains(&changed.path.as_str()) {
            continue;
        }
        seen.push(&changed.path);
        let text = text_of(&changed.path);
        if let Some((reason, from)) = class_path(changed, |path| change_reason(path, text)) {
            found.push(candidate(
                Class::Change,
                changed,
                renamed(vec![reason.to_string()], from),
            ));
        }
    }
    for hunk in &hunks {
        let allowlist = block(text_of(&hunk[0].path), "const SKIP_ALLOWLIST", "];");
        let hit = class_path(&hunk[0], |path| {
            let rules = path == "xtask/src/rules.rs";
            hunk.iter().find(|c| rules && inside(allowlist, c))
        });
        if let Some((first, from)) = hit {
            found.push(candidate(
                Class::Change,
                first,
                renamed(vec![ALLOWLISTED.to_string()], from),
            ));
        }
    }
    for hunk in &hunks {
        let hit = class_path(&hunk[0], |path| {
            Some(hot_reasons(path)).filter(|r| !r.is_empty())
        });
        if let Some((reasons, from)) = hit {
            found.push(candidate(Class::HotSpot, &hunk[0], renamed(reasons, from)));
        }
    }
    for changed in lines.iter().filter(|c| c.side == Side::Right) {
        let text = text_of(&changed.path);
        let hit = class_path(changed, |path| {
            (path.ends_with(".rs") && has_unsafe(&changed.text)).then_some(UNSAFE)
        });
        if let Some((reason, from)) = hit {
            found.push(candidate(
                Class::HotSpot,
                changed,
                renamed(vec![reason.to_string()], from),
            ));
        }
        let hit = class_path(changed, |path| {
            let manifest = path == "Cargo.toml" || path.ends_with("/Cargo.toml");
            (manifest && is_dependency(text, changed.line, &changed.text)).then_some(DEPENDENCY)
        });
        if let Some((reason, from)) = hit {
            found.push(candidate(
                Class::HotSpot,
                changed,
                renamed(vec![reason.to_string()], from),
            ));
        }
    }
    for (path, text) in plans {
        for (n, author, said) in agent_entries(text) {
            let in_diff = lines
                .iter()
                .any(|c| c.path == *path && c.side == Side::Right && c.line == n);
            found.push(Candidate {
                class: Class::Judgement,
                path: path.clone(),
                line: in_diff.then_some((n, Side::Right)),
                no_line: NO_DIFF_LINE,
                reasons: vec![format!(
                    "{author}: {}",
                    said.chars().take(80).collect::<String>()
                )],
            });
        }
    }
    // A path with no hunk changes no message text, so it is never a
    // `hand-run` candidate.
    for (path, from) in no_hunk_paths(diff) {
        let text = text_of(&path);
        // A file with no hunk, as a changed line with no line, so that
        // `class_path` reads it as it reads the others.
        let file = Changed {
            path: path.clone(),
            side: Side::Right,
            line: 0,
            hunk_new: 0,
            hunk_new_count: 0,
            hunk: 0,
            text: String::new(),
            renamed_from: from,
        };
        let mut classes = Vec::new();
        if let Some((reason, from)) = class_path(&file, |p| change_reason(p, text)) {
            for reason in renamed(vec![reason.to_string()], from) {
                classes.push((Class::Change, reason));
            }
        }
        let hot = class_path(&file, |p| Some(hot_reasons(p)).filter(|r| !r.is_empty()));
        if let Some((reasons, from)) = hot {
            for reason in renamed(reasons, from) {
                classes.push((Class::HotSpot, reason));
            }
        }
        if let Some((class, _)) = classes.first() {
            found.push(Candidate {
                class: *class,
                path,
                line: None,
                no_line: NO_HUNK,
                reasons: classes.into_iter().map(|(_, reason)| reason).collect(),
            });
        }
    }
    merge_and_sort(found)
}

/// The class path of `changed`: what `meets` gives for its new path, or
/// else for the old path of a rename.  The second value is the old path
/// when the old path gave the answer.  Each class reads a path only
/// through this function, so no class can forget the old path.
fn class_path<T>(
    changed: &Changed,
    meets: impl Fn(&str) -> Option<T>,
) -> Option<(T, Option<&str>)> {
    if let Some(found) = meets(&changed.path) {
        return Some((found, None));
    }
    let old = changed.renamed_from.as_deref()?;
    meets(old).map(|found| (found, Some(old)))
}

/// `reasons`, each as `renamed from <old>: <reason>` when the old path
/// `from` gave them.
fn renamed(reasons: Vec<String>, from: Option<&str>) -> Vec<String> {
    match from {
        Some(old) => reasons
            .into_iter()
            .map(|reason| format!("renamed from {old}: {reason}"))
            .collect(),
        None => reasons,
    }
}

/// A candidate at the line of `changed`.
fn candidate(class: Class, changed: &Changed, reasons: Vec<String>) -> Candidate {
    Candidate {
        class,
        path: changed.path.clone(),
        line: Some((changed.line, changed.side)),
        no_line: NO_DIFF_LINE,
        reasons,
    }
}

/// The lines at `HEAD` that a changed line is between, as (low, high).
/// An added line is at its own line, so low and high are that line.  A
/// removed line of a hunk with new lines is at the hunk's new-side
/// position.  A hunk that only removes lines (`+<n>,0`) is the gap between
/// the lines `n` and `n + 1`.
fn span(changed: &Changed) -> (usize, usize) {
    match changed.side {
        Side::Right => (changed.line, changed.line),
        Side::Left if changed.hunk_new_count == 0 => (changed.hunk_new, changed.hunk_new + 1),
        Side::Left => (changed.hunk_new, changed.hunk_new),
    }
}

/// The first line of `hunk` that changes a message that `stored-stems`,
/// `worker-check` or `weights` writes by hand.  `path` picks the rule, and
/// `text` is the file at `HEAD` of the new path.
fn hand_run_line<'a>(path: &str, hunk: &'a [Changed], text: &str) -> Option<&'a Changed> {
    match path {
        "protocol/src/lib.rs" | "xtask/e2e/worker.js" => hunk.first(),
        "xtask/src/e2e/weights.rs" => {
            let tests = text
                .lines()
                .position(|line| line.trim() == "#[cfg(test)]")
                .map_or(usize::MAX, |i| i + 1);
            hunk.iter().find(|c| span(c).0 < tests)
        }
        "xtask/src/e2e.rs" => {
            let items = hand_run_items(text);
            hunk.iter()
                .find(|c| items.iter().any(|item| inside(Some(*item), c)))
        }
        _ => None,
    }
}

/// The lines, from 1, of each top-level item of `e2e.rs` that writes or
/// reads a message of `stored-stems` or `worker-check` by hand.  An item
/// starts at column 0 with `fn`, `pub fn`, `const` or `static`.  Its name
/// starts with `stored_stems` or `worker_check` in any case, or is
/// `stem_protocol_file`.  A `fn` ends at the next line that is exactly
/// `}`.  A `const` or `static` ends at its first line that ends with `;`.
fn hand_run_items(text: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut items = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let (rest, is_fn) = if let Some(rest) = line
            .strip_prefix("fn ")
            .or_else(|| line.strip_prefix("pub fn "))
        {
            (rest, true)
        } else if let Some(rest) = line
            .strip_prefix("const ")
            .or_else(|| line.strip_prefix("static "))
        {
            (rest, false)
        } else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<String>()
            .to_lowercase();
        if !(name.starts_with("stored_stems")
            || name.starts_with("worker_check")
            || name == "stem_protocol_file")
        {
            continue;
        }
        let end = (i..lines.len()).find(|j| {
            if is_fn {
                *j > i && lines[*j] == "}"
            } else {
                lines[*j].trim_end().ends_with(';')
            }
        });
        items.push((i + 1, end.unwrap_or(lines.len().saturating_sub(1)) + 1));
    }
    items
}

/// The lines, from 1, from the first line of `text` that holds `start` to
/// the next line that is exactly `end`, or to the end of `text`.
fn block(text: &str, start: &str, end: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.iter().position(|line| line.contains(start))?;
    let last = (first + 1..lines.len())
        .find(|i| lines[*i] == end)
        .unwrap_or(lines.len().saturating_sub(1));
    Some((first + 1, last + 1))
}

/// True when `changed` is inside `block`: its span starts at or after the
/// first line, and ends at or before the last line.
fn inside(block: Option<(usize, usize)>, changed: &Changed) -> bool {
    let (low, high) = span(changed);
    block.is_some_and(|(first, last)| first <= low && high <= last)
}

/// The reason of a changed file in the `change` class, or none.  `text`
/// is the file at `HEAD`.
fn change_reason(path: &str, text: &str) -> Option<&'static str> {
    if glob_matches("docs/decisions/*.md", path) {
        return Some(if text.contains("Status: proposed") {
            PROPOSED
        } else {
            DECISION
        });
    }
    match path {
        "docs/invariants.md" => Some(RULE),
        "CLAUDE.md" => Some(MAP),
        _ if glob_matches(".claude/**", path) => Some(HARNESS),
        _ if glob_matches(".github/{workflows,actions}/**", path) => Some(CI),
        _ => None,
    }
}

/// The reasons of a changed path in the `hot-spot` class: each hot rule
/// whose strict paths hold it, `app/src/lib.rs`, and the page-worker
/// messages.
fn hot_reasons(path: &str) -> Vec<String> {
    let mut reasons: Vec<String> = HOT_RULES
        .iter()
        .filter(|rule| {
            strict_paths(rule)
                .iter()
                .any(|glob| glob_matches(glob, path))
        })
        .map(|rule| format!("a strict path of {rule}"))
        .collect();
    if path == "app/src/lib.rs" {
        reasons.push(APP_LIB.to_string());
    }
    if glob_matches("protocol/src/**", path) {
        reasons.push(MESSAGES.to_string());
    }
    reasons
}

/// True when `line` holds `unsafe` after the line start, a space, `(` or
/// `{`, and before a space or `{`, and `line` is not a `//` line.  An
/// attribute `#[unsafe(...)]` is a hit too.
fn has_unsafe(line: &str) -> bool {
    if line.trim_start().starts_with("//") {
        return false;
    }
    line.match_indices("unsafe").any(|(i, word)| {
        let before = line[..i].chars().next_back();
        let after = line[i + word.len()..].chars().next();
        let word =
            matches!(before, None | Some(' ' | '(' | '{')) && matches!(after, Some(' ' | '{'));
        let attribute = before == Some('[') && after == Some('(');
        word || attribute
    })
}

/// True when the added line `added`, at `line` of a `Cargo.toml` whose
/// text at `HEAD` is `text`, is in a dependency table and is not blank or
/// a comment.
fn is_dependency(text: &str, line: usize, added: &str) -> bool {
    let added = added.trim();
    if added.is_empty() || added.starts_with('#') {
        return false;
    }
    let header = text
        .lines()
        .take(line)
        .map(str::trim)
        .filter(|l| l.starts_with('['))
        .last();
    header.is_some_and(|h| h.ends_with("dependencies]") || h.contains("dependencies."))
}

/// Each agent entry of the `## Decision log` section of a plan: its line
/// (from 1), its author and its text.
fn agent_entries(plan: &str) -> Vec<(usize, &str, &str)> {
    let mut entries = Vec::new();
    let mut in_log = false;
    for (i, line) in plan.lines().enumerate() {
        if line.starts_with("## ") {
            in_log = line.trim_end() == "## Decision log";
            continue;
        }
        if !in_log {
            continue;
        }
        let Some((author, said)) = entry(line) else {
            continue;
        };
        if author == "agent" || author.starts_with("agent (") || author.starts_with("agent,") {
            entries.push((i + 1, author, said));
        }
    }
    entries
}

/// The author and the text of an entry line `* <YYYY-MM-DD>, <author>: <text>`.
fn entry(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("* ")?;
    let date = rest.get(..10)?;
    let is_date = date.char_indices().all(|(i, c)| match i {
        4 | 7 => c == '-',
        _ => c.is_ascii_digit(),
    });
    if !is_date {
        return None;
    }
    let (author, said) = rest[10..].strip_prefix(", ")?.split_once(':')?;
    Some((author, said.trim()))
}

/// Merges the candidates with the same path, line and side, with the
/// first class and each reason once.  Sorts them by class, then path, then
/// line, with no line last in its class.
fn merge_and_sort(found: Vec<Candidate>) -> Vec<Candidate> {
    let mut merged: Vec<Candidate> = Vec::new();
    for c in found {
        let same = merged
            .iter_mut()
            .find(|m| c.line.is_some() && m.path == c.path && m.line == c.line);
        match same {
            Some(m) => {
                m.class = m.class.min(c.class);
                for reason in c.reasons {
                    if !m.reasons.contains(&reason) {
                        m.reasons.push(reason);
                    }
                }
            }
            None => merged.push(c),
        }
    }
    merged.sort_by(|a, b| {
        (a.class, a.line.is_none(), &a.path, a.line).cmp(&(
            b.class,
            b.line.is_none(),
            &b.path,
            b.line,
        ))
    });
    merged
}

/// Runs `xtask attention <base> <plan>` in the repo of the current folder.
/// It gives 0 after the list, 1 for a git error or a plan outside the
/// repo, and 2 for a usage error.
pub fn main(args: &[String]) -> i32 {
    // A `<base>` that starts with `-` would be read as an option of git.
    let [base, plan] = args else {
        eprintln!("{USAGE}");
        return REFUSED;
    };
    if base.starts_with('-') {
        eprintln!("{USAGE}");
        return REFUSED;
    }
    match gather(base, plan) {
        Ok((diff, files, plans)) => {
            let found = candidates(&diff, &files, &plans);
            for candidate in &found {
                println!("{}", candidate.render());
            }
            println!(
                "attention: {} candidates; pick at most 10, the hand-run lines first (decision 0027)",
                found.len()
            );
            0
        }
        Err(e) => {
            eprintln!("{e:#}");
            FAILED
        }
    }
}

/// Files as (path, text).
type Texts = Vec<(String, String)>;

/// The diff of `<base>...HEAD`, the text at `HEAD` of each changed path,
/// and the plans: `plan` and each `docs/plans/{active,completed}/<stem>-m<n>.md`.
fn gather(base: &str, plan: &str) -> Result<(String, Texts, Texts)> {
    let dir = std::env::current_dir().context("reading the current folder")?;
    let root = repo_root(&dir)?;
    let plan = &repo_path(&root, &dir, plan)?;
    let diff = git(
        &root,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "-M",
            "-U0",
            &format!("{base}...HEAD"),
        ],
    )?;
    let mut files: Texts = Vec::new();
    let paths = changed_lines(&diff)
        .into_iter()
        .map(|changed| changed.path)
        .chain(no_hunk_paths(&diff).into_iter().map(|(path, _)| path));
    for path in paths {
        if files.iter().any(|(p, _)| *p == path) {
            continue;
        }
        // A deleted file has no text at HEAD, so it gets an empty text.
        let text = git(&root, &["show", &format!("HEAD:{path}")]).unwrap_or_default();
        files.push((path, text));
    }
    let mut plans = vec![(
        plan.to_string(),
        git(&root, &["show", &format!("HEAD:{plan}")])?,
    )];
    let listing = git(
        &root,
        &[
            "ls-tree",
            "-r",
            "--name-only",
            "HEAD",
            "docs/plans/active",
            "docs/plans/completed",
        ],
    )?;
    for path in milestone_plans(plan, &listing) {
        let text = git(&root, &["show", &format!("HEAD:{path}")])?;
        plans.push((path, text));
    }
    Ok((diff, files, plans))
}

/// `plan`, which is relative to `dir` or absolute, as a `/`-separated
/// path relative to `root`.  A path that exists is made canonical first,
/// since `root` from git has no symlink.  A path that does not exist loses
/// its `..` parts.
pub fn repo_path(root: &Path, dir: &Path, plan: &str) -> Result<String> {
    let joined = dir.join(plan);
    let path = joined.canonicalize().unwrap_or_else(|_| {
        let mut path = std::path::PathBuf::new();
        // `components` drops each `.` after the first part, and `dir` is
        // absolute.
        for part in joined.components() {
            match part {
                std::path::Component::ParentDir => {
                    path.pop();
                }
                other => path.push(other),
            }
        }
        path
    });
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{plan} is not in the repo {}", root.display()))?;
    let parts: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

/// The paths of `listing` that are `docs/plans/{active,completed}/<stem>-m<n>.md`,
/// where `<stem>` is the file stem of `plan`.
pub fn milestone_plans(plan: &str, listing: &str) -> Vec<String> {
    let stem = Path::new(plan)
        .file_stem()
        .map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let prefix = format!("{stem}-m");
    listing
        .lines()
        .filter(|path| {
            let name = path
                .strip_prefix("docs/plans/active/")
                .or_else(|| path.strip_prefix("docs/plans/completed/"));
            let number = name
                .and_then(|n| n.strip_prefix(&prefix))
                .and_then(|n| n.strip_suffix(".md"));
            number.is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
        .collect()
}

/// Runs git in `dir`, and gives its output.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A diff of `path` with one hunk: `old` lines removed at `old_at`,
    /// and `new` lines added at `new_at`.
    fn hunk(path: &str, old_at: usize, old: &[&str], new_at: usize, new: &[&str]) -> String {
        let mut out = format!(
            "diff --git a/{path} b/{path}\nindex 1111111..2222222 100644\n--- a/{path}\n+++ b/{path}\n@@ -{old_at},{} +{new_at},{} @@\n",
            old.len(),
            new.len()
        );
        for line in old {
            out.push_str(&format!("-{line}\n"));
        }
        for line in new {
            out.push_str(&format!("+{line}\n"));
        }
        out
    }

    /// A diff that adds `text` at `line` of `path`.
    fn added(path: &str, line: usize, text: &str) -> String {
        hunk(path, line - 1, &[], line, &[text])
    }

    fn texts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(path, text)| (path.to_string(), text.to_string()))
            .collect()
    }

    /// The rendered candidates of `diff`.
    fn rendered(diff: &str, files: &[(&str, &str)], plans: &[(&str, &str)]) -> Vec<String> {
        candidates(diff, &texts(files), &texts(plans))
            .iter()
            .map(Candidate::render)
            .collect()
    }

    fn strict(rule: &str) -> String {
        format!("a strict path of {rule}")
    }

    #[test]
    fn a_protocol_or_worker_js_hunk_is_one_hand_run_line() {
        let diff = [
            hunk("protocol/src/lib.rs", 7, &["old"], 7, &["new", "more"]),
            added("xtask/e2e/worker.js", 3, "let x = 1;"),
        ]
        .concat();
        assert_eq!(
            rendered(&diff, &[], &[]),
            vec![
                format!(
                    "hand-run protocol/src/lib.rs:7 LEFT: {HAND_RUN}; {}; {}; {MESSAGES}",
                    strict("PROTO-2"),
                    strict("MEM-2")
                ),
                format!("hand-run xtask/e2e/worker.js:3 RIGHT: {HAND_RUN}"),
            ]
        );
    }

    const E2E: &str = "fn smoke(o: &Path) {\n    let m = json!({ \"type\": \"x\" });\n}\n\nfn stored_stems(o: &Opts) -> Result<()> {\n    if x {\n        y();\n    }\n    let m = json!({ \"type\": \"init\" });\n}\n\nfn worker_check(o: &Opts) -> Result<()> {\n    let m = json!({\n        \"type\": \"init\",\n        \"weights\": w,\n    });\n}\n\nfn after() {\n    let m = json!({ \"type\": \"x\" });\n}\n\nfn stored_stems_checks(r: &Value) {\n    let ok = r[\"type\"] == \"done\";\n}\n\nfn stem_protocol_file(name: &str) -> String {\n    format!(\"{name}.wav\")\n}\n\nconst STORED_STEMS_SESSION: &str =\n    \"sessions/1\";\n\nfn stored_stems_clip(out: &Path) -> Result<()> {\n    let a = 1;\n}\n\nconst OTHER: &str = \"x\";\n\npub fn worker_check_reply() -> Value {\n    json!({})\n}\n\nstatic WORKER_CHECK_ARGS: &[&str] = &[\"a\"];\n\nfn open_on_the_worker_check(log: &str) -> bool {\n    log.contains(\"open\")\n}\n\nconst STORED_STEMS_X: &str = \"x\";\nfn smoke_two() {\n    let a = 1;\n}\n";

    const WEIGHTS: &str = "pub fn run() {\n    let m = json!({\n        \"type\": \"init\",\n        \"settings\": s,\n    });\n}\n\n#[cfg(test)]\nmod tests {\n    fn t() { let m = json!({ \"type\": \"x\" }); }\n}\n";

    #[test]
    fn a_line_of_stored_stems_worker_check_or_weights_is_a_hand_run_line() {
        let files = [
            ("xtask/src/e2e.rs", E2E),
            ("xtask/src/e2e/weights.rs", WEIGHTS),
        ];
        let diff = [
            added(
                "xtask/src/e2e.rs",
                2,
                "    let m = json!({ \"type\": \"y\" });",
            ),
            added(
                "xtask/src/e2e.rs",
                9,
                "    let m = json!({ \"type\": \"init\" });",
            ),
            hunk(
                "xtask/src/e2e.rs",
                15,
                &["        \"weights\": old,"],
                14,
                &[],
            ),
            added(
                "xtask/src/e2e.rs",
                20,
                "    let m = json!({ \"type\": \"y\" });",
            ),
            added("xtask/src/e2e.rs", 4, "// before stored_stems"),
            added(
                "xtask/src/e2e.rs",
                24,
                "    let ok = r[\"type\"] == \"ok\";",
            ),
            added("xtask/src/e2e.rs", 28, "    format!(\"{name}.flac\")"),
            added("xtask/src/e2e.rs", 32, "    \"sessions/2\";"),
            added("xtask/src/e2e.rs", 33, "// after the const"),
            added("xtask/src/e2e.rs", 35, "    let a = 2;"),
            added("xtask/src/e2e.rs", 38, "const OTHER: &str = \"y\";"),
            added("xtask/src/e2e.rs", 41, "    json!({ \"a\": 1 })"),
            added("xtask/src/e2e.rs", 47, "    log.contains(\"opened\")"),
            added("xtask/src/e2e.rs", 52, "    let a = 2;"),
            added(
                "xtask/src/e2e.rs",
                44,
                "static WORKER_CHECK_ARGS: &[&str] = &[\"b\"];",
            ),
            added("xtask/src/e2e/weights.rs", 4, "        \"settings\": t,"),
            added("xtask/src/e2e/weights.rs", 7, "// before the tests"),
            added("xtask/src/e2e/weights.rs", 8, "#[cfg(test)]"),
            added(
                "xtask/src/e2e/weights.rs",
                10,
                "    fn t() { let m = json!({ \"type\": \"y\" }); }",
            ),
        ]
        .concat();
        assert_eq!(
            rendered(&diff, &files, &[]),
            vec![
                format!("hand-run xtask/src/e2e.rs:9 RIGHT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e.rs:15 LEFT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e.rs:24 RIGHT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e.rs:28 RIGHT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e.rs:32 RIGHT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e.rs:35 RIGHT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e.rs:41 RIGHT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e.rs:44 RIGHT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e/weights.rs:4 RIGHT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e/weights.rs:7 RIGHT: {HAND_RUN}"),
            ]
        );
    }

    #[test]
    fn a_removal_just_after_a_body_is_not_in_it() {
        let files = [("xtask/src/e2e.rs", E2E)];
        let diff = [
            hunk("xtask/src/e2e.rs", 11, &["    let late = 1;"], 10, &[]),
            hunk("xtask/src/e2e.rs", 10, &["    let last = 1;"], 9, &[]),
        ]
        .concat();
        assert_eq!(
            rendered(&diff, &files, &[]),
            vec![format!("hand-run xtask/src/e2e.rs:10 LEFT: {HAND_RUN}")]
        );
    }

    #[test]
    fn a_removal_is_the_gap_between_two_new_lines() {
        let files = [
            ("xtask/src/e2e.rs", E2E),
            ("xtask/src/e2e/weights.rs", WEIGHTS),
        ];
        let diff = [
            hunk("xtask/src/e2e/weights.rs", 8, &["    let x = 1;"], 7, &[]),
            hunk("xtask/src/e2e.rs", 5, &["// gone"], 4, &[]),
            hunk("xtask/src/e2e.rs", 4, &["// old"], 4, &["// new"]),
            hunk("xtask/src/e2e.rs", 10, &["}"], 10, &["} "]),
        ]
        .concat();
        assert_eq!(
            rendered(&diff, &files, &[]),
            vec![
                format!("hand-run xtask/src/e2e.rs:10 LEFT: {HAND_RUN}"),
                format!("hand-run xtask/src/e2e/weights.rs:8 LEFT: {HAND_RUN}"),
            ]
        );
    }

    #[test]
    fn repo_path_removes_dot_dot_and_refuses_a_path_above_the_root() {
        let root = Path::new("/no/such/repo");
        assert_eq!(repo_path(root, root, "a/../b").unwrap(), "b");
        assert_eq!(
            repo_path(root, &root.join("docs"), "../docs/./p.md").unwrap(),
            "docs/p.md"
        );
        let e = repo_path(root, root, "../x.md").unwrap_err();
        assert_eq!(
            format!("{e:#}"),
            "../x.md is not in the repo /no/such/repo: prefix not found"
        );
    }

    #[test]
    fn a_hunk_gives_its_first_line_in_a_body() {
        let files = [("xtask/src/e2e.rs", E2E)];
        let diff = hunk(
            "xtask/src/e2e.rs",
            2,
            &[
                "    let m = json!({ \"type\": \"x\" });",
                "}",
                "",
                "fn stored_stems(o: &Opts) -> Result<()> {",
            ],
            2,
            &[
                "    let m = 2;",
                "}",
                "",
                "fn stored_stems(o: Opts) -> Result<()> {",
            ],
        );
        assert_eq!(
            rendered(&diff, &files, &[]),
            vec![format!("hand-run xtask/src/e2e.rs:5 RIGHT: {HAND_RUN}")]
        );
    }

    #[test]
    fn a_decision_a_rule_the_map_the_harness_and_ci_are_change_lines() {
        let diff = [
            hunk(
                "docs/decisions/0028-new.md",
                0,
                &[],
                1,
                &["# New", "", "Status: proposed"],
            ),
            added("docs/decisions/0001-old.md", 5, "More."),
            added("docs/invariants.md", 9, "* A rule."),
            added("CLAUDE.md", 2, "A line."),
            hunk(".claude/skills/x/SKILL.md", 3, &["old"], 3, &["new"]),
            added(".claude/skills/x/SKILL.md", 9, "later"),
            added(".github/workflows/ci.yml", 4, "  run: x"),
            added(".github/actions/test/action.yml", 4, "  run: x"),
            added(".github/dependabot.yml", 4, "  x"),
            added("docs/working-with-agents.md", 3, "unsafe { x }"),
            added("docs/README.md", 3, "* `x.md`: x."),
        ]
        .concat();
        let files = [
            ("docs/decisions/0028-new.md", "# New\n\nStatus: proposed\n"),
            (
                "docs/decisions/0001-old.md",
                "# Old\n\nStatus: accepted (2026-10-19, by the maintainer)\n",
            ),
        ];
        assert_eq!(
            rendered(&diff, &files, &[]),
            vec![
                format!("change .claude/skills/x/SKILL.md:3 LEFT: {HARNESS}"),
                format!("change .github/actions/test/action.yml:4 RIGHT: {CI}"),
                format!("change .github/workflows/ci.yml:4 RIGHT: {CI}"),
                format!("change CLAUDE.md:2 RIGHT: {MAP}"),
                format!("change docs/decisions/0001-old.md:5 RIGHT: {DECISION}"),
                format!("change docs/decisions/0028-new.md:1 RIGHT: {PROPOSED}"),
                format!("change docs/invariants.md:9 RIGHT: {RULE}"),
            ]
        );
    }

    const RULES: &str = "const A: &[&str] = &[\n    \"a\",\n];\n\nconst SKIP_ALLOWLIST: &[(&str, &str, &str)] = &[\n    (\"f\", \"skipped: x\", \"why\"),\n];\n\nconst B: &[&str] = &[\n    \"b\",\n];\n";

    #[test]
    fn a_line_in_skip_allowlist_is_a_change_line_and_a_line_outside_is_none() {
        let files = [("xtask/src/rules.rs", RULES)];
        let diff = [
            added("xtask/src/rules.rs", 2, "    \"c\","),
            hunk(
                "xtask/src/rules.rs",
                7,
                &["    (\"h\", \"skipped\", \"why\"),"],
                6,
                &[],
            ),
            hunk(
                "xtask/src/rules.rs",
                9,
                &[],
                9,
                &["    \"c\",", "    \"d\","],
            ),
            hunk(
                "xtask/src/rules.rs",
                5,
                &[],
                6,
                &["    (\"g\", \"skipped: y\", \"why\"),", ""],
            ),
        ]
        .concat();
        assert_eq!(
            rendered(&diff, &files, &[]),
            vec![
                format!("change xtask/src/rules.rs:6 RIGHT: {ALLOWLISTED}"),
                format!("change xtask/src/rules.rs:7 LEFT: {ALLOWLISTED}"),
            ]
        );
    }

    #[test]
    fn a_block_at_the_first_line_of_a_file_is_read() {
        let files = [(
            "xtask/src/rules.rs",
            "const SKIP_ALLOWLIST: &[(&str, &str, &str)] = &[\n    (\"f\", \"skipped: x\", \"why\"),\n];\n\nconst B: &[&str] = &[];\n",
        )];
        let diff = [
            added(
                "xtask/src/rules.rs",
                2,
                "    (\"g\", \"skipped: y\", \"why\"),",
            ),
            added("xtask/src/rules.rs", 5, "const B: &[&str] = &[];"),
        ]
        .concat();
        assert_eq!(
            rendered(&diff, &files, &[]),
            vec![format!("change xtask/src/rules.rs:2 RIGHT: {ALLOWLISTED}")]
        );
    }

    #[test]
    fn hot_paths_unsafe_and_a_dependency_are_hot_spot_lines() {
        let cargo = "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nfoo = \"1\"\n\n# a comment\n\n[target.'cfg(unix)'.dev-dependencies]\nbar = \"1\"\n\n[dependencies.baz]\nversion = \"1\"\n";
        let diff = [
            added("app/src/separator.rs", 3, "x"),
            added("engine/src/song.rs", 3, "x"),
            added("engine/src/player.rs", 3, "x"),
            added("app/src/lib.rs", 3, "x"),
            added("dsp/src/lib.rs", 1, "#![forbid(unsafe_code)]"),
            added("dsp/src/lib.rs", 2, "    // unsafe { f() }"),
            added("dsp/src/lib.rs", 3, "    let unsafely = 1;"),
            added("dsp/src/lib.rs", 4, "    \"--enable-unsafe-webgpu\","),
            added("dsp/src/lib.rs", 5, "    unsafe { f() }"),
            added("dsp/src/lib.rs", 6, "pub unsafe fn g() {}"),
            added("dsp/src/lib.rs", 7, "    let x = (unsafe{ f() });"),
            added("dsp/src/lib.rs", 8, "#[unsafe(no_mangle)]"),
            added("dsp/src/lib.rs", 9, "#[unsafe(export_name = \"g\")]"),
            added("dsp/src/lib.rs", 10, "    let a = [unsafe_b];"),
            added("roformer-gpu/src/net.rs", 3, "x"),
            added("roformer/src/weights.rs", 3, "x"),
            hunk("Cargo.toml", 4, &[], 5, &["serde = \"1\""]),
            added("dsp/src/web.js", 7, "unsafe { f() }"),
            hunk(
                "engine/Cargo.toml",
                2,
                &["version = \"0.0.1\""],
                3,
                &["version = \"0.1.0\""],
            ),
            hunk("engine/Cargo.toml", 5, &[], 6, &["foo = \"1\""]),
            hunk("engine/Cargo.toml", 7, &[], 8, &["# a comment"]),
            hunk("engine/Cargo.toml", 8, &[], 9, &[""]),
            hunk("engine/Cargo.toml", 10, &[], 11, &["bar = \"1\""]),
            hunk("engine/Cargo.toml", 13, &[], 14, &["version = \"1\""]),
        ]
        .concat();
        assert_eq!(
            rendered(
                &diff,
                &[
                    ("engine/Cargo.toml", cargo),
                    (
                        "Cargo.toml",
                        "[workspace]\nmembers = []\n\n[workspace.dependencies]\nserde = \"1\"\n"
                    )
                ],
                &[]
            ),
            vec![
                format!("hot-spot Cargo.toml:5 RIGHT: {DEPENDENCY}"),
                format!("hot-spot app/src/lib.rs:3 RIGHT: {APP_LIB}"),
                format!(
                    "hot-spot app/src/separator.rs:3 RIGHT: {}; {}",
                    strict("PROTO-2"),
                    strict("MEM-2")
                ),
                format!("hot-spot dsp/src/lib.rs:5 RIGHT: {UNSAFE}"),
                format!("hot-spot dsp/src/lib.rs:6 RIGHT: {UNSAFE}"),
                format!("hot-spot dsp/src/lib.rs:7 RIGHT: {UNSAFE}"),
                format!("hot-spot dsp/src/lib.rs:8 RIGHT: {UNSAFE}"),
                format!("hot-spot dsp/src/lib.rs:9 RIGHT: {UNSAFE}"),
                format!("hot-spot engine/Cargo.toml:6 RIGHT: {DEPENDENCY}"),
                format!("hot-spot engine/Cargo.toml:11 RIGHT: {DEPENDENCY}"),
                format!("hot-spot engine/Cargo.toml:14 RIGHT: {DEPENDENCY}"),
                format!("hot-spot engine/src/player.rs:3 RIGHT: {}", strict("RT-1")),
                format!("hot-spot engine/src/song.rs:3 RIGHT: {}", strict("MEM-1")),
                format!(
                    "hot-spot roformer-gpu/src/net.rs:3 RIGHT: {}",
                    strict("MEM-3")
                ),
                format!(
                    "hot-spot roformer/src/weights.rs:3 RIGHT: {}",
                    strict("MEM-4")
                ),
            ]
        );
    }

    #[test]
    fn the_hot_rules_are_the_fixed_list() {
        assert_eq!(
            HOT_RULES,
            [
                "PROTO-2", "MEM-1", "MEM-2", "MEM-3", "MEM-4", "MEM-5", "RT-1"
            ]
        );
    }

    #[test]
    fn each_glob_of_the_hot_rules_gives_a_hot_spot() {
        let mut count = 0;
        for rule in HOT_RULES {
            for glob in strict_paths(rule) {
                let path = glob.replace("**", "a/b").replace('*', "x");
                let found = candidates(&added(&path, 1, "x"), &[], &[]);
                assert_eq!(found.len(), 1, "{rule} {path}: {found:?}");
                assert!(
                    found[0].class <= Class::HotSpot && found[0].reasons.contains(&strict(rule)),
                    "{rule} {path}: {found:?}"
                );
                count += 1;
            }
        }
        assert!(count >= 30, "{count}");
    }

    const PLAN: &str = "# Plan\n\n## Progress\n\n* 2026-11-01, agent: in Progress.\n\n## Decision log\n\n* 2026-11-01, agent: one.\n* 2026-11-01, agent (orchestrator): two.\n* 2026-11-01, agent (team leader): three.\n* 2026-11-01, agent (implementer): four.\n* 2026-11-01, agent, M5, before any run: five.\n* 2026-11-01, maintainer: no.\n* 2026-11-01, maintainer, D1: no.\n* 2026-11-01, team leader: no.\n* 2026-11-01, agents: no.\n* 2026-11-01, agentic: no.\n* agent: no date.\n* 2026-1-01, agent: a short date.\n* 2026-11-0x, agent: not a date.\n\n## Outcomes and retrospective\n\n* 2026-11-01, agent: after the log.\n";

    #[test]
    fn each_agent_form_in_the_decision_log_is_a_judgement() {
        let path = "docs/plans/active/p.md";
        let all: Vec<&str> = PLAN.lines().collect();
        let diff = hunk(path, 0, &[], 1, &all);
        assert_eq!(
            rendered(&diff, &[(path, PLAN)], &[(path, PLAN)]),
            vec![
                format!("judgement {path}:9 RIGHT: agent: one."),
                format!("judgement {path}:10 RIGHT: agent (orchestrator): two."),
                format!("judgement {path}:11 RIGHT: agent (team leader): three."),
                format!("judgement {path}:12 RIGHT: agent (implementer): four."),
                format!("judgement {path}:13 RIGHT: agent, M5, before any run: five."),
            ]
        );
    }

    #[test]
    fn an_agent_entry_that_is_not_in_the_diff_has_no_line() {
        let path = "docs/plans/active/p-m1.md";
        let long = "x".repeat(100);
        let plan = format!(
            "## Decision log\n\n* 2026-11-01, agent: {long}\n* 2026-11-02, agent: in the diff.\n* 2026-11-03, agent (implementer): not in the diff.\n"
        );
        let diff = [
            added(path, 4, "* 2026-11-02, agent: in the diff."),
            hunk(path, 5, &["* 2026-11-03, agent: gone."], 4, &[]),
            added(
                "docs/plans/active/q.md",
                3,
                "* 2026-11-01, agent: another plan.",
            ),
        ]
        .concat();
        assert_eq!(
            rendered(&diff, &[], &[(path, &plan)]),
            vec![
                format!("judgement {path}:4 RIGHT: agent: in the diff."),
                format!(
                    "judgement {path} (no diff line; list it in the PR): agent: {}",
                    "x".repeat(80)
                ),
                format!(
                    "judgement {path} (no diff line; list it in the PR): agent (implementer): not in the diff."
                ),
            ]
        );
    }

    #[test]
    fn the_output_is_hand_run_change_hot_spot_then_judgement() {
        let path = "docs/plans/active/p.md";
        let milestone = "docs/plans/active/p-m1.md";
        let old = "## Decision log\n\n* 2026-10-31, agent (team leader): old.\n";
        let all: Vec<&str> = PLAN.lines().collect();
        let mut diff = String::new();
        for n in 0..6 {
            diff.push_str(&added(&format!(".claude/skills/s{n}/SKILL.md"), 1, "x"));
        }
        diff.push_str(&added("app/src/lib.rs", 3, "x"));
        diff.push_str(&hunk(path, 0, &[], 1, &all));
        diff.push_str(&added("protocol/src/lib.rs", 40, "pub struct X;"));
        let found = rendered(&diff, &[], &[(path, PLAN), (milestone, old)]);
        let mut want = vec![format!(
            "hand-run protocol/src/lib.rs:40 RIGHT: {HAND_RUN}; {}; {}; {MESSAGES}",
            strict("PROTO-2"),
            strict("MEM-2")
        )];
        for n in 0..6 {
            want.push(format!(
                "change .claude/skills/s{n}/SKILL.md:1 RIGHT: {HARNESS}"
            ));
        }
        want.push(format!("hot-spot app/src/lib.rs:3 RIGHT: {APP_LIB}"));
        want.extend([
            format!("judgement {path}:9 RIGHT: agent: one."),
            format!("judgement {path}:10 RIGHT: agent (orchestrator): two."),
            format!("judgement {path}:11 RIGHT: agent (team leader): three."),
            format!("judgement {path}:12 RIGHT: agent (implementer): four."),
            format!("judgement {path}:13 RIGHT: agent, M5, before any run: five."),
            format!(
                "judgement {milestone} (no diff line; list it in the PR): agent (team leader): old."
            ),
        ]);
        assert_eq!(found, want);
    }

    #[test]
    fn one_hunk_of_worker_web_gives_one_candidate_with_each_rule() {
        assert_eq!(
            rendered(
                &hunk("worker/src/web.rs", 10, &["a"], 10, &["b", "c"]),
                &[],
                &[]
            ),
            vec![format!(
                "hot-spot worker/src/web.rs:10 LEFT: {}; {}; {}",
                strict("PROTO-2"),
                strict("MEM-2"),
                strict("MEM-5")
            )]
        );
    }

    #[test]
    fn two_hunks_of_one_file_give_two_candidates_in_line_order() {
        let diff = [
            hunk("engine/src/song.rs", 30, &[], 31, &["b"]),
            hunk("engine/src/song.rs", 4, &[], 5, &["a"]),
        ]
        .concat();
        assert_eq!(
            rendered(&diff, &[], &[]),
            vec![
                format!("hot-spot engine/src/song.rs:5 RIGHT: {}", strict("MEM-1")),
                format!("hot-spot engine/src/song.rs:31 RIGHT: {}", strict("MEM-1")),
            ]
        );
    }

    /// A changed line.  `at` is the new-side position and count of its
    /// hunk.
    fn changed(
        path: &str,
        side: Side,
        line: usize,
        at: (usize, usize),
        hunk: usize,
        text: &str,
    ) -> Changed {
        Changed {
            path: path.to_string(),
            side,
            line,
            hunk_new: at.0,
            hunk_new_count: at.1,
            hunk,
            text: text.to_string(),
            renamed_from: None,
        }
    }

    #[test]
    fn the_parse_gives_each_side_line_hunk_and_text() {
        let diff = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -3,2 +2,0 @@\n-gone\n--- also gone\n@@ -9 +8,3 @@ fn f() {\n-old\n+new\n \n++++ new too\n\\ No newline at end of file\n";
        assert_eq!(
            changed_lines(diff),
            vec![
                changed("a.rs", Side::Left, 3, (2, 0), 0, "gone"),
                changed("a.rs", Side::Left, 4, (2, 0), 0, "-- also gone"),
                changed("a.rs", Side::Left, 9, (8, 3), 1, "old"),
                changed("a.rs", Side::Right, 8, (8, 3), 1, "new"),
                changed("a.rs", Side::Right, 10, (8, 3), 1, "+++ new too"),
            ]
        );
    }

    #[test]
    fn a_context_line_moves_both_sides() {
        let diff = "--- a/a.rs\n+++ b/a.rs\n@@ -10,3 +10,2 @@\n ctx\n-removed\n ctx\n@@ -20,2 +19,3 @@\n ctx\n+added\n ctx\n";
        let found: Vec<(Side, usize)> = changed_lines(diff)
            .iter()
            .map(|c| (c.side, c.line))
            .collect();
        assert_eq!(found, vec![(Side::Left, 11), (Side::Right, 20)]);
    }

    #[test]
    fn a_path_with_no_hunk_gives_a_candidate_with_no_line() {
        let mode = |path: &str| {
            format!("diff --git a/{path} b/{path}\nold mode 100644\nnew mode 100755\n")
        };
        let diff = [
            "diff --git a/docs/decisions/0028-a.md b/docs/decisions/0028-b.md\nsimilarity index 100%\nrename from docs/decisions/0028-a.md\nrename to docs/decisions/0028-b.md\n".to_string(),
            mode(".claude/hooks/h.sh"),
            "diff --git a/.claude/empty.md b/.claude/empty.md\nnew file mode 100644\nindex 0000000..e69de29\n".to_string(),
            "diff --git a/.github/workflows/x.bin b/.github/workflows/x.bin\nindex 1..2 100644\nBinary files a/.github/workflows/x.bin and b/.github/workflows/x.bin differ\n".to_string(),
            mode("docs/README.md"),
            mode("protocol/src/lib.rs"),
            mode("xtask/src/e2e.rs"),
            added("CLAUDE.md", 2, "A line."),
            mode("engine/src/song.rs"),
        ]
        .concat();
        let files = [("docs/decisions/0028-b.md", "# B\n\nStatus: proposed\n")];
        let none = "(no hunk; list it in the PR)";
        assert_eq!(
            rendered(&diff, &files, &[]),
            vec![
                format!("change CLAUDE.md:2 RIGHT: {MAP}"),
                format!("change .claude/empty.md {none}: {HARNESS}"),
                format!("change .claude/hooks/h.sh {none}: {HARNESS}"),
                format!("change .github/workflows/x.bin {none}: {CI}"),
                format!("change docs/decisions/0028-b.md {none}: {PROPOSED}"),
                format!("hot-spot engine/src/song.rs {none}: {}", strict("MEM-1")),
                format!(
                    "hot-spot protocol/src/lib.rs {none}: {}; {}; {MESSAGES}",
                    strict("PROTO-2"),
                    strict("MEM-2")
                ),
            ]
        );
        assert_eq!(
            no_hunk_paths(&diff),
            [
                ("docs/decisions/0028-b.md", Some("docs/decisions/0028-a.md")),
                (".claude/hooks/h.sh", None),
                (".claude/empty.md", None),
                (".github/workflows/x.bin", None),
                ("docs/README.md", None),
                ("protocol/src/lib.rs", None),
                ("xtask/src/e2e.rs", None),
                ("engine/src/song.rs", None),
            ]
            .map(|(path, from)| (path.to_string(), from.map(str::to_string)))
        );
        let spaced = "diff --git a/a/p b/q b/a/p b/r\nsimilarity index 100%\nrename from a/p b/q\nrename to a/p b/r\n";
        assert_eq!(
            no_hunk_paths(spaced),
            vec![("a/p b/r".to_string(), Some("a/p b/q".to_string()))]
        );
    }

    #[test]
    fn a_rename_out_of_a_class_path_is_classed_by_its_old_path() {
        let rename = |from: &str, to: &str| {
            format!(
                "diff --git a/{from} b/{to}\nsimilarity index 100%\nrename from {from}\nrename to {to}\n"
            )
        };
        let diff = [
            rename("engine/clippy.toml", "engine/clippy.toml.off"),
            rename(".claude/skills/x/SKILL.md", "old/SKILL.md"),
            "diff --git a/engine/src/song.rs b/engine/src/tune.rs\nsimilarity index 90%\nrename from engine/src/song.rs\nrename to engine/src/tune.rs\nindex 1..2 100644\n--- a/engine/src/song.rs\n+++ b/engine/src/tune.rs\n@@ -3 +3 @@\n-a\n+b\n".to_string(),
            rename("docs/a.md", "docs/b.md"),
        ]
        .concat();
        let none = "(no hunk; list it in the PR)";
        assert_eq!(
            rendered(&diff, &[], &[]),
            vec![
                format!(
                    "change old/SKILL.md {none}: renamed from .claude/skills/x/SKILL.md: {HARNESS}"
                ),
                format!(
                    "hot-spot engine/src/tune.rs:3 LEFT: renamed from engine/src/song.rs: {}",
                    strict("MEM-1")
                ),
                format!(
                    "hot-spot engine/clippy.toml.off {none}: renamed from engine/clippy.toml: {}",
                    strict("MEM-1")
                ),
            ]
        );
    }

    /// A rename with one hunk at line 3, from `from` to `to`.
    fn renamed(from: &str, to: &str) -> String {
        format!(
            "diff --git a/{from} b/{to}\nsimilarity index 90%\nrename from {from}\nrename to {to}\nindex 1..2 100644\n--- a/{from}\n+++ b/{to}\n@@ -3 +3 @@\n-a\n+b\n"
        )
    }

    /// A rename with no hunk, from `from` to `to`.
    fn pure_rename(from: &str, to: &str) -> String {
        format!(
            "diff --git a/{from} b/{to}\nsimilarity index 100%\nrename from {from}\nrename to {to}\n"
        )
    }

    /// A mode change of `path`, with no hunk.
    fn mode_change(path: &str) -> String {
        format!("diff --git a/{path} b/{path}\nold mode 100644\nnew mode 100755\n")
    }

    /// Each class reads the new path of a rename, then its old path.  A
    /// path with no hunk is never `hand-run`.  Each row is one diff section
    /// and its exact candidates.
    #[test]
    fn each_class_reads_both_paths_of_a_rename() {
        // The old and the new path of a rename have different texts, so a
        // line test that reads the old path's text fails a row.
        let files = [
            ("xtask/src/e2e/weights.rs", WEIGHTS),
            ("x/w.rs", "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n"),
            ("x/w2.rs", WEIGHTS),
            (
                "xtask/src/e2e.rs",
                "fn smoke() {\n    let a = 1;\n    let m = 1;\n}\n",
            ),
            (
                "x/e.rs",
                "fn stored_stems(o: &Opts) {\n    let a = 1;\n    let m = 1;\n}\n",
            ),
            (
                "x/rules.rs",
                "const SKIP_ALLOWLIST: &[(&str, &str, &str)] = &[\n    (\"a\", \"b\", \"c\"),\n    (\"d\", \"e\", \"f\"),\n];\n",
            ),
            (
                "engine/Cargo.toml.off",
                "[package]\nname = \"x\"\n\n[dependencies]\nfoo = \"1\"\n",
            ),
        ];
        let none = "(no hunk; list it in the PR)";
        let proto = format!("{}; {}; {MESSAGES}", strict("PROTO-2"), strict("MEM-2"));
        let proto_from = |old: &str| {
            format!(
                "renamed from {old}: {}; renamed from {old}: {}; renamed from {old}: {MESSAGES}",
                strict("PROTO-2"),
                strict("MEM-2")
            )
        };
        let mem1 = strict("MEM-1");
        let mut rows: Vec<(String, Vec<String>)> = vec![
            (
                renamed("x/a.rs", "protocol/src/lib.rs"),
                vec![format!(
                    "hand-run protocol/src/lib.rs:3 LEFT: {HAND_RUN}; {proto}"
                )],
            ),
            (
                renamed("protocol/src/lib.rs", "x/a.rs"),
                vec![format!(
                    "hand-run x/a.rs:3 LEFT: renamed from protocol/src/lib.rs: {HAND_RUN}; {}",
                    proto_from("protocol/src/lib.rs")
                )],
            ),
            (
                pure_rename("protocol/src/lib.rs", "x/a.rs"),
                vec![format!(
                    "hot-spot x/a.rs {none}: {}",
                    proto_from("protocol/src/lib.rs")
                )],
            ),
            (
                mode_change("protocol/src/lib.rs"),
                vec![format!("hot-spot protocol/src/lib.rs {none}: {proto}")],
            ),
            (
                renamed("x/w.js", "xtask/e2e/worker.js"),
                vec![format!("hand-run xtask/e2e/worker.js:3 LEFT: {HAND_RUN}")],
            ),
            (
                renamed("xtask/e2e/worker.js", "x/w.js"),
                vec![format!(
                    "hand-run x/w.js:3 LEFT: renamed from xtask/e2e/worker.js: {HAND_RUN}"
                )],
            ),
            (pure_rename("xtask/e2e/worker.js", "x/w.js"), vec![]),
            (mode_change("xtask/e2e/worker.js"), vec![]),
            (
                renamed("x/w.rs", "xtask/src/e2e/weights.rs"),
                vec![format!(
                    "hand-run xtask/src/e2e/weights.rs:3 LEFT: {HAND_RUN}"
                )],
            ),
            (
                renamed("xtask/src/e2e/weights.rs", "x/w2.rs"),
                vec![format!(
                    "hand-run x/w2.rs:3 LEFT: renamed from xtask/src/e2e/weights.rs: {HAND_RUN}"
                )],
            ),
            (pure_rename("xtask/src/e2e/weights.rs", "x/w.rs"), vec![]),
            (mode_change("xtask/src/e2e/weights.rs"), vec![]),
        ];
        for (path, other, reason, class) in [
            (
                ".claude/rules/x.md",
                "x/r.md",
                HARNESS.to_string(),
                "change",
            ),
            ("docs/invariants.md", "x/i.md", RULE.to_string(), "change"),
            (
                ".github/workflows/ci.yml",
                "x/ci.yml",
                CI.to_string(),
                "change",
            ),
            ("engine/clippy.toml", "x/c.toml", mem1.clone(), "hot-spot"),
            ("engine/src/song.rs", "x/s.rs", mem1.clone(), "hot-spot"),
        ] {
            rows.extend([
                (
                    renamed(other, path),
                    vec![format!("{class} {path}:3 LEFT: {reason}")],
                ),
                (
                    renamed(path, other),
                    vec![format!(
                        "{class} {other}:3 LEFT: renamed from {path}: {reason}"
                    )],
                ),
                (
                    pure_rename(path, other),
                    vec![format!(
                        "{class} {other} {none}: renamed from {path}: {reason}"
                    )],
                ),
                (
                    mode_change(path),
                    vec![format!("{class} {path} {none}: {reason}")],
                ),
            ]);
        }
        rows.extend([
            (
                renamed(".claude/rules/a.md", ".claude/rules/b.md"),
                vec![format!("change .claude/rules/b.md:3 LEFT: {HARNESS}")],
            ),
            (
                pure_rename("engine/src/song.rs", "engine/src/player.rs"),
                vec![format!(
                    "hot-spot engine/src/player.rs {none}: {}",
                    strict("RT-1")
                )],
            ),
            (renamed("x/a.md", "x/b.md"), vec![]),
            (pure_rename("x/a.md", "x/b.md"), vec![]),
            (
                renamed("xtask/src/e2e.rs", "x/e.rs"),
                vec![format!(
                    "hand-run x/e.rs:3 LEFT: renamed from xtask/src/e2e.rs: {HAND_RUN}"
                )],
            ),
            (
                renamed("xtask/src/rules.rs", "x/rules.rs"),
                vec![format!(
                    "change x/rules.rs:3 LEFT: renamed from xtask/src/rules.rs: {ALLOWLISTED}"
                )],
            ),
            (
                "diff --git a/x/u.rs b/x/u.txt\nsimilarity index 90%\nrename from x/u.rs\nrename to x/u.txt\nindex 1..2 100644\n--- a/x/u.rs\n+++ b/x/u.txt\n@@ -3 +3 @@\n-a\n+    unsafe { f() }\n".to_string(),
                vec![format!("hot-spot x/u.txt:3 RIGHT: renamed from x/u.rs: {UNSAFE}")],
            ),
            (
                "diff --git a/engine/Cargo.toml b/engine/Cargo.toml.off\nsimilarity index 90%\nrename from engine/Cargo.toml\nrename to engine/Cargo.toml.off\nindex 1..2 100644\n--- a/engine/Cargo.toml\n+++ b/engine/Cargo.toml.off\n@@ -5 +5 @@\n-bar = \"1\"\n+foo = \"1\"\n".to_string(),
                vec![format!(
                    "hot-spot engine/Cargo.toml.off:5 RIGHT: renamed from engine/Cargo.toml: {DEPENDENCY}"
                )],
            ),
        ]);
        let mut failed = Vec::new();
        for (diff, want) in &rows {
            let got = rendered(diff, &files, &[]);
            if got != *want {
                failed.push(format!("{diff}\n  got  {got:?}\n  want {want:?}"));
            }
        }
        assert!(
            failed.is_empty(),
            "{} rows failed:\n{}",
            failed.len(),
            failed.join("\n")
        );
        assert_eq!(rows.len(), 40);
    }

    #[test]
    fn a_rename_gives_the_new_path_and_its_old_path_and_a_deleted_file_the_old_path() {
        let diff = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\nindex 1..2 100644\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\nindex 1..0\n--- a/gone.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-x\ndiff --git a/new.md b/new.md\nnew file mode 100644\nindex 0..1\n--- /dev/null\n+++ b/new.md\n@@ -0,0 +1 @@\n+y\ndiff --git a/i.png b/i.png\nindex 1..2 100644\nBinary files a/i.png and b/i.png differ\n";
        assert_eq!(
            changed_lines(diff),
            vec![
                Changed {
                    renamed_from: Some("old.rs".to_string()),
                    ..changed("new.rs", Side::Left, 1, (1, 1), 0, "a")
                },
                Changed {
                    renamed_from: Some("old.rs".to_string()),
                    ..changed("new.rs", Side::Right, 1, (1, 1), 0, "b")
                },
                changed("gone.rs", Side::Left, 1, (0, 0), 1, "x"),
                changed("new.md", Side::Right, 1, (1, 1), 2, "y"),
            ]
        );
    }

    #[test]
    fn the_milestone_plans_of_a_plan_are_found() {
        let listing = "docs/plans/active/p.md\ndocs/plans/active/p-m7.md\ndocs/plans/completed/p-m1.md\ndocs/plans/completed/p-m12.md\ndocs/plans/completed/p-mx.md\ndocs/plans/completed/p-m.md\ndocs/plans/completed/q-m1.md\ndocs/plans/completed/p-m1.txt\ndocs/plans/other/p-m2.md\n";
        assert_eq!(
            milestone_plans("docs/plans/active/p.md", listing),
            vec![
                "docs/plans/active/p-m7.md",
                "docs/plans/completed/p-m1.md",
                "docs/plans/completed/p-m12.md",
            ]
        );
    }
}
