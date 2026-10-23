//! `xtask docs-check`: keeps the agent map (`CLAUDE.md`), `docs/` and the
//! path-scoped rules in `.claude/rules/` true to the code.
//!
//! The reader of a problem is usually an agent, so every problem names its
//! check, what is wrong and how to fix it.  `docs/README.md` lists the
//! conventions these checks hold the docs to.

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

/// Longest the always-loaded map may grow: everything else is read on demand.
pub const MAP_LINE_BUDGET: usize = 100;

/// Folders never walked, relative to the root: git, build output, and the
/// gitignored audio, model and page folders that exist on some machines only.
/// The same names deeper in the tree are walked.
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    "target",
    "third_party",
    "audio",
    "stems",
    "pages",
    "app/dist",
    ".claude/worktrees",
];

/// Folders inside a crate, which docs name relative to that crate
/// (`src/shaders/` on the line about `roformer-gpu/`).
const CRATE_RELATIVE: &[&str] = &["src/", "tests/", "benches/", "e2e/"];

/// Paths the docs may name that a build creates, so they are absent in a
/// fresh checkout: these folders and everything in them.
const GENERATED: &[&str] = &["app/dist", "target", "third_party", "audio", "stems"];

/// Characters that make a code span prose, a template or a Rust path rather
/// than a file path: `xtask e2e`, `v/<id>/`, `engine::Samples`, `a@v1`.
const NOT_IN_PATHS: &str = "<>*?{}=()$:,;'\"|\\@";

const DECISION_STATUSES: &[&str] = &["proposed", "accepted", "superseded", "rejected"];

/// The living sections every execution plan keeps current (see `docs/plans/PLANS.md`).
pub const PLAN_SECTIONS: &[&str] = &[
    "## Progress",
    "## Surprises and discoveries",
    "## Decision log",
    "## Outcomes and retrospective",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub check: &'static str,
    pub file: String,
    pub message: String,
}

impl Problem {
    fn new(check: &'static str, file: &str, message: String) -> Problem {
        Problem {
            check,
            file: file.to_string(),
            message,
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[docs/{}] {}: {}", self.check, self.file, self.message)
    }
}

/// A workspace member: its folder and its package name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub dir: String,
    pub package: String,
}

/// Runs every check over the repository at `root`.
pub fn check(root: &Path) -> Result<Vec<Problem>> {
    let files = repo_files(root)?;
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel)).with_context(|| format!("reading {rel}"))
    };
    let markdown: Vec<(String, String)> = files
        .iter()
        .filter(|f| is_agent_doc(f))
        .map(|f| read(f).map(|text| (f.clone(), text)))
        .collect::<Result<_>>()?;
    let doc = |rel: &str| {
        markdown
            .iter()
            .find(|(f, _)| f == rel)
            .map(|(_, text)| text.as_str())
    };

    let members = workspace_members(&read("Cargo.toml")?)
        .into_iter()
        .map(|dir| {
            let manifest = read(&format!("{dir}/Cargo.toml"))?;
            let package = package_name(&manifest)
                .with_context(|| format!("{dir}/Cargo.toml has no [package] name"))?;
            Ok(Member { dir, package })
        })
        .collect::<Result<Vec<_>>>()?;
    let tree = Tree::new(&files);
    let member_dirs: Vec<String> = members.iter().map(|m| m.dir.clone()).collect();
    let top_level: BTreeSet<String> = files
        .iter()
        .filter_map(|f| f.split_once('/').map(|(first, _)| first.to_string()))
        .collect();
    let test_fns = test_fn_names(
        &files
            .iter()
            .filter(|f| f.ends_with(".rs"))
            .map(|f| read(f))
            .collect::<Result<Vec<_>>>()?,
    );
    let ids = defined_ids(
        &markdown
            .iter()
            .filter(|(f, _)| ID_FILES.contains(&f.as_str()))
            .cloned()
            .collect::<Vec<_>>(),
    );

    let mut problems = Vec::new();
    problems.extend(map_within_budget(doc("CLAUDE.md").unwrap_or_default()));
    problems.extend(crates_documented(
        &members,
        &[
            ("CLAUDE.md", doc("CLAUDE.md"), "## Layout", "`<dir>/`"),
            (
                "README.md",
                read("README.md").ok().as_deref(),
                "## Layout",
                "`<dir>/`",
            ),
            (
                "docs/ARCHITECTURE.md",
                doc("docs/ARCHITECTURE.md"),
                "## Crates",
                "`<package>`",
            ),
        ],
    ));
    problems.extend(ids.problems.clone());
    for (file, text) in &markdown {
        problems.extend(paths_exist(&tree, file, text, &top_level, &member_dirs));
        problems.extend(id_references(file, text, &ids));
    }
    if let Some(invariants) = doc("docs/invariants.md") {
        problems.extend(enforcing_tests_exist(invariants, &test_fns));
        problems.extend(map_lists_every_rule(
            doc("CLAUDE.md").unwrap_or_default(),
            invariants,
        ));
    }
    let tools: BTreeSet<String> = xtask_commands(&read("xtask/src/main.rs")?)
        .into_iter()
        .chain(agent_capabilities(&files))
        .collect();
    match doc(GUIDE) {
        Some(guide) => problems.extend(guide_covers(guide, &tools)),
        None => problems.push(Problem::new(
            "guide",
            GUIDE,
            "is missing.  Fix: restore the guide for people who direct agents here.".into(),
        )),
    }
    problems.extend(docs_indexed(&markdown, doc("docs/README.md")));
    problems.extend(rules_match_files(&markdown, &files));
    problems.extend(decisions_well_formed(&markdown));
    problems.extend(plans_well_formed(&markdown));
    Ok(problems)
}

/// The files that hold stable IDs (`### MEM-1: ...`): rules and debts.
const ID_FILES: &[&str] = &["docs/invariants.md", "docs/tech-debt.md"];

fn is_agent_doc(file: &str) -> bool {
    file == "CLAUDE.md"
        || (file.starts_with("docs/") && file.ends_with(".md"))
        || (file.starts_with(".claude/rules/") && file.ends_with(".md"))
        || (file.starts_with(".claude/agents/") && file.ends_with(".md"))
        || (file.starts_with(".claude/skills/") && file.ends_with(".md"))
}

/// Every file under `root`, as a `/`-separated path relative to it, sorted.
pub fn repo_files(root: &Path) -> Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
            let path = entry?.path();
            let rel = path
                .strip_prefix(root)
                .expect("walked from root")
                .to_string_lossy()
                .replace('\\', "/");
            if SKIPPED_DIRS.contains(&rel.as_str()) {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, out)?;
            } else {
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

/// The folders listed in the root manifest's `members = [...]` (not
/// `default-members`), with `#` comments ignored.
pub fn workspace_members(cargo_toml: &str) -> Vec<String> {
    let lines: Vec<&str> = cargo_toml
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default())
        .collect();
    let Some(start) = lines.iter().position(|l| {
        l.split_once('=')
            .is_some_and(|(key, _)| key.trim() == "members")
    }) else {
        return Vec::new();
    };
    let list = lines[start..].join("\n");
    let (Some(open), Some(close)) = (list.find('['), list.find(']')) else {
        return Vec::new();
    };
    list[open + 1..close]
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The `name` in a manifest's `[package]` table.
pub fn package_name(manifest: &str) -> Option<String> {
    let package = &manifest[manifest.find("[package]")?..];
    package
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with('['))
        .find_map(|l| {
            let (key, value) = l.split_once('=')?;
            (key.trim() == "name").then(|| value.trim().trim_matches('"').to_string())
        })
}

/// Inline code spans outside fenced code blocks, as CommonMark reads them: a
/// span opens with a run of backticks and closes at the next run of the same
/// length, within one paragraph; `` \` `` outside a span is a literal backtick.
pub fn inline_code(markdown: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut paragraph: Vec<&str> = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let marker = fence_marker(trimmed);
        let closes = match (fence, marker) {
            (Some((c, n)), Some((m, k))) => {
                c == m && k >= n && trimmed.trim_start_matches(c).trim().is_empty()
            }
            _ => false,
        };
        let block_start =
            fence.is_none() && (marker.is_some() || trimmed.is_empty() || starts_a_block(trimmed));
        if closes || block_start || fence.is_some() {
            spans.extend(spans_in(&paragraph.join("\n")));
            paragraph.clear();
        }
        if closes {
            fence = None;
        } else if fence.is_none() && marker.is_some() {
            fence = marker;
        } else if fence.is_none() && !trimmed.is_empty() {
            paragraph.push(line);
            if trimmed.starts_with('#') {
                spans.extend(spans_in(&paragraph.join("\n")));
                paragraph.clear();
            }
        }
    }
    spans.extend(spans_in(&paragraph.join("\n")));
    spans
}

/// The fence character and length of a line that opens or closes a fenced
/// code block: three or more backticks or tildes.
fn fence_marker(trimmed: &str) -> Option<(char, usize)> {
    let c = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = trimmed.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

/// A line that starts a new block, so no code span runs into it from the
/// line before: a list item, a heading, a table row or a quote.
fn starts_a_block(trimmed: &str) -> bool {
    let numbered = trimmed
        .split_once(". ")
        .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    numbered
        || ["* ", "- ", "+ ", "#", "|", ">"]
            .iter()
            .any(|m| trimmed.starts_with(m))
}

/// The code spans in one paragraph.
fn spans_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let run_at = |i: usize| chars[i..].iter().take_while(|c| **c == '`').count();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && matches!(chars.get(i + 1), Some('`' | '\\')) {
            i += 2;
            continue;
        }
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let open = run_at(i);
        let start = i + open;
        let mut j = start;
        let close = loop {
            if j >= chars.len() {
                break None;
            }
            let run = if chars[j] == '`' { run_at(j) } else { 0 };
            if run == open {
                break Some(j);
            }
            j += run.max(1);
        };
        match close {
            Some(end) => {
                let code: String = chars[start..end]
                    .iter()
                    .collect::<String>()
                    .replace('\n', " ");
                let stripped = match code.strip_prefix(' ').and_then(|c| c.strip_suffix(' ')) {
                    Some(inner) if !code.trim().is_empty() => inner.to_string(),
                    _ => code,
                };
                spans.push(stripped);
                i = end + open;
            }
            None => i = start,
        }
    }
    spans
}

/// The text from a `## Heading` line up to the next heading of that level.
pub fn section<'a>(markdown: &'a str, heading: &str) -> Option<&'a str> {
    let start = markdown
        .match_indices(heading)
        .find(|(i, _)| {
            let bytes = markdown.as_bytes();
            let line_start = *i == 0 || bytes[i - 1] == b'\n';
            let line_end = matches!(bytes.get(i + heading.len()), None | Some(b'\n' | b'\r'));
            line_start && line_end
        })
        .map(|(i, _)| i + heading.len())?;
    let level = &heading[..=heading.find(' ')?];
    let rest = &markdown[start..];
    let end = rest
        .match_indices(&format!("\n{level}"))
        .next()
        .map_or(rest.len(), |(i, _)| i);
    Some(&rest[..end])
}

/// A code span read as a path in this repository, if it looks like one.
/// Prose such as `xtask e2e`, Rust paths (`engine::Samples`), templates
/// (`v/<id>/`) and folders a build creates are not paths to check.
pub fn repo_path(span: &str, top_level: &BTreeSet<String>) -> Option<String> {
    let without_line = strip_line_suffix(span);
    let path = without_line
        .split('#')
        .next()
        .unwrap_or_default()
        .trim_end_matches('/');
    let path = path.strip_prefix("./").unwrap_or(path);
    let prose = path.is_empty()
        || path.starts_with("http")
        || path.starts_with(['~', '/', '-'].as_slice())
        || path.contains(|c: char| c.is_whitespace() || NOT_IN_PATHS.contains(c));
    let generated = GENERATED
        .iter()
        .any(|g| path == *g || path.starts_with(&format!("{g}/")));
    if prose || generated {
        return None;
    }
    let first = path.split('/').next().unwrap_or_default();
    let nested = path.contains('/');
    // `.flac` or `.gitkeep` alone is an extension or a file kind; a dotted
    // folder (`.claude/rules`) is a path and is checked, typos included.
    if first.starts_with('.') && first != ".." && !nested {
        return None;
    }
    // `docs.rs/wgpu` or `0.5/1.0` is a site or a number, unless a
    // top-level folder has that name.
    if !first.starts_with('.') && first.contains('.') && nested && !top_level.contains(first) {
        return None;
    }
    let file_name = path.rsplit('/').next().unwrap_or_default();
    let in_tree = nested && top_level.contains(first);
    let names_a_file = nested && file_name.contains('.');
    let crate_relative = CRATE_RELATIVE.iter().any(|c| path.starts_with(c));
    (path.ends_with(".md") || in_tree || names_a_file || crate_relative && nested)
        .then(|| path.to_string())
}

/// `file.rs:12` or `file.rs:12-20` without its line numbers.
fn strip_line_suffix(span: &str) -> &str {
    match span.rsplit_once(':') {
        Some((head, tail))
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit() || c == '-') =>
        {
            head
        }
        _ => span,
    }
}

/// Plans name files they are about to create, and decisions and review logs
/// are records of their time, so only their IDs and structure are checked,
/// not their paths.
fn names_current_paths(file: &str) -> bool {
    !(file.starts_with("docs/plans/")
        || file.starts_with("docs/decisions/")
        || file.starts_with("docs/reviews/"))
}

/// The places a path in `file` may be relative to: the repo root, the file's
/// own folder, and inside `docs/`, the `docs/` folder.
fn path_bases(file: &str) -> Vec<String> {
    let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut bases = vec![String::new()];
    if !dir.is_empty() {
        bases.push(format!("{dir}/"));
    }
    if file.starts_with("docs/") {
        bases.push("docs/".to_string());
    }
    bases
}

/// The files in the repo and the folders that hold them, as `/` paths.  A
/// path is looked up here, not on disk, so case matters on every system.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub files: BTreeSet<String>,
    pub dirs: BTreeSet<String>,
}

impl Tree {
    pub fn new(files: &[String]) -> Tree {
        let dirs = files
            .iter()
            .flat_map(|f| f.match_indices('/').map(move |(i, _)| f[..i].to_string()))
            .collect();
        Tree {
            files: files.iter().cloned().collect(),
            dirs,
        }
    }

    fn contains(&self, path: &str) -> bool {
        self.files.contains(path) || self.dirs.contains(path)
    }
}

/// `a/./b/../c` as `a/c`; `None` when `..` climbs out of the repo.
fn normalise(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn paths_exist(
    tree: &Tree,
    file: &str,
    text: &str,
    top_level: &BTreeSet<String>,
    member_dirs: &[String],
) -> Vec<Problem> {
    if !names_current_paths(file) {
        return Vec::new();
    }
    let bases = path_bases(file);
    let crate_bases: Vec<String> = member_dirs.iter().map(|d| format!("{d}/")).collect();
    inline_code(text)
        .iter()
        .filter_map(|span| repo_path(span, top_level))
        .filter(|p| {
            // `src/lib.rs` is relative to a crate: any member's will do.
            let in_a_crate = CRATE_RELATIVE.iter().any(|c| p.starts_with(c));
            let tried = if in_a_crate { &crate_bases } else { &bases };
            !tried
                .iter()
                .filter_map(|b| normalise(&format!("{b}{p}")))
                .any(|candidate| tree.contains(&candidate))
        })
        .map(|p| {
            Problem::new(
                "paths",
                file,
                format!(
                    "`{p}` does not exist.  Fix: correct the path, or remove the mention if the file is gone."
                ),
            )
        })
        .collect()
}

/// The rules the map lists, one `* ID: ...` bullet each under `## Rules`,
/// are exactly the invariants in `docs/invariants.md` that are not retired.
pub fn map_lists_every_rule(claude: &str, invariants: &str) -> Vec<Problem> {
    let listed: BTreeSet<&str> = section(claude, "## Rules")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("* ")?.split_once(':'))
        .map(|(id, _)| id)
        .filter(|id| is_id(id))
        .collect();
    let rules: BTreeSet<&str> = invariants
        .lines()
        .filter_map(|l| l.strip_prefix("### ")?.split_once(':'))
        .filter(|(id, title)| is_id(id) && !title.trim_start().starts_with("Retired"))
        .map(|(id, _)| id)
        .collect();
    let missing = rules.difference(&listed).map(|id| {
        Problem::new(
            "rules",
            "CLAUDE.md",
            format!("rule `{id}` from docs/invariants.md is missing from `## Rules`.  Fix: add a one-line `* {id}: ...` bullet."),
        )
    });
    let extra = listed.difference(&rules).map(|id| {
        Problem::new(
            "rules",
            "CLAUDE.md",
            format!("`{id}` is listed under `## Rules` but is not a current rule in docs/invariants.md.  Fix: define it there, or remove the bullet."),
        )
    });
    missing.chain(extra).collect()
}

/// The guide for people who direct agents.
pub const GUIDE: &str = "docs/working-with-agents.md";

/// The subcommands `xtask` dispatches on: each `"name" =>` arm in `main.rs`.
pub fn xtask_commands(main_rs: &str) -> BTreeSet<String> {
    // An arm's pattern may wrap over lines (`"a" | "b"` then `| "c" =>`), and
    // may carry a guard (`"a" if ok =>`).
    let mut patterns = Vec::new();
    let mut pattern = String::new();
    for line in main_rs.lines().map(str::trim_start) {
        if !(line.starts_with('"') || (line.starts_with('|') && !pattern.is_empty())) {
            pattern.clear();
            continue;
        }
        pattern.push_str(line);
        pattern.push(' ');
        if let Some((arms, _)) = pattern.split_once("=>") {
            let unguarded = arms.split(" if ").next().unwrap_or_default();
            patterns.push(unguarded.to_string());
            pattern.clear();
        }
    }
    patterns
        .iter()
        .flat_map(|arms| arms.split('|'))
        .map(|arm| arm.trim().trim_matches('"').to_string())
        .filter(|arm| {
            !arm.is_empty()
                && arm
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
        .collect()
}

/// The project's skills (`.claude/skills/<name>/SKILL.md`) and subagents
/// (`.claude/agents/<name>.md`).
pub fn agent_capabilities(files: &[String]) -> BTreeSet<String> {
    files
        .iter()
        .filter_map(|f| {
            let skill = f
                .strip_prefix(".claude/skills/")
                .and_then(|s| s.strip_suffix("/SKILL.md"));
            let flat = |folder: &str| {
                f.strip_prefix(folder)
                    .and_then(|a| a.strip_suffix(".md"))
                    .filter(|a| !a.contains('/'))
            };
            skill
                .or(flat(".claude/agents/"))
                .or(flat(".claude/commands/"))
                .map(str::to_string)
        })
        .collect()
}

/// The guide names every `xtask` command, skill and subagent in a code span,
/// so a new way of working is never left undocumented.
pub fn guide_covers(guide: &str, names: &BTreeSet<String>) -> Vec<Problem> {
    let words: BTreeSet<String> = inline_code(guide)
        .iter()
        .flat_map(|span| {
            span.split_whitespace()
                .map(|w| w.trim_start_matches('/').to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    names
        .iter()
        .filter(|name| !words.contains(*name))
        .map(|name| {
            Problem::new(
                "guide",
                GUIDE,
                format!("does not mention `{name}`.  Fix: say when a person or the agent uses it, in a recipe or the tools table, and add a changelog line."),
            )
        })
        .collect()
}

fn map_within_budget(claude: &str) -> Vec<Problem> {
    let lines = claude.lines().count();
    if lines <= MAP_LINE_BUDGET {
        return Vec::new();
    }
    vec![Problem::new(
        "budget",
        "CLAUDE.md",
        format!(
            "{lines} lines, over the budget of {MAP_LINE_BUDGET}.  Fix: CLAUDE.md is the map loaded into every session; move detail into a doc under docs/ or a path-scoped rule in .claude/rules/ and link to it."
        ),
    )]
}

/// Every workspace member appears in each doc's crate list, written as the
/// doc's form (`<dir>/` or `<package>`); a package name always counts.
pub fn crates_documented(
    members: &[Member],
    docs: &[(&str, Option<&str>, &str, &str)],
) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (file, text, heading, form) in docs {
        let Some(list) = text.and_then(|t| section(t, heading)) else {
            problems.push(Problem::new(
                "crates",
                file,
                format!("has no `{heading}` section.  Fix: list every workspace crate under `{heading}`."),
            ));
            continue;
        };
        let spans: BTreeSet<String> = inline_code(list).into_iter().collect();
        for m in members {
            let named = spans.contains(&format!("{}/", m.dir))
                || spans.contains(&m.dir)
                || spans.contains(&m.package);
            if !named {
                let entry = form
                    .replace("<dir>", &m.dir)
                    .replace("<package>", &m.package);
                problems.push(Problem::new(
                    "crates",
                    file,
                    format!(
                        "workspace crate `{}` ({}/) is missing from `{heading}`.  Fix: add a {entry} entry saying what the crate is for.",
                        m.package, m.dir
                    ),
                ));
            }
        }
    }
    problems
}

/// IDs defined by `### PREFIX-N: title` headings, IDs closed by a
/// `* PREFIX-N: ...` bullet under `## Closed`, and the problems with them.
#[derive(Debug, Clone, Default)]
pub struct Ids {
    pub defined: BTreeSet<String>,
    pub closed: BTreeSet<String>,
    pub problems: Vec<Problem>,
}

const CLOSED: &str = "## Closed";

/// The lines of `text` in its `## Closed` section.
fn closed_section(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .skip_while(|l| *l != CLOSED)
        .skip(1)
        .take_while(|l| !l.starts_with("## "))
}

/// `text` without the `* ID:` that opens each bullet of its `## Closed`
/// section, which names a closed ID on purpose.  The rest of each bullet
/// is kept, so the IDs it names are checked.
fn without_closed_ids(text: &str) -> String {
    let before = text.lines().take_while(|l| *l != CLOSED);
    let closed = closed_section(text).map(|l| {
        l.strip_prefix("* ")
            .and_then(|bullet| bullet.split_once(':'))
            .map_or(l, |(_, rest)| rest)
    });
    let after = text
        .lines()
        .skip_while(|l| *l != CLOSED)
        .skip(1)
        .skip_while(|l| !l.starts_with("## "));
    before
        .chain(closed)
        .chain(after)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn defined_ids(files: &[(String, String)]) -> Ids {
    let mut ids = Ids::default();
    let mut defined_in = Vec::new();
    for (file, text) in files {
        for heading in text.lines().filter_map(|l| l.strip_prefix("### ")) {
            let Some(id) = heading
                .split_once(':')
                .map(|(id, _)| id)
                .filter(|id| is_id(id))
            else {
                ids.problems.push(Problem::new(
                    "ids",
                    file,
                    format!("heading `### {heading}` is not `### ID: title`, so it defines nothing.  Fix: an ID is two to six capitals, a dash and a number, followed by a colon (`### MEM-6: ...`)."),
                ));
                continue;
            };
            if !ids.defined.insert(id.to_string()) {
                ids.problems.push(Problem::new(
                    "ids",
                    file,
                    format!("`{id}` is defined twice.  Fix: give the newer entry the next free number; IDs are never reused."),
                ));
            }
            defined_in.push((file, id));
        }
        for line in closed_section(text).filter(|l| !l.trim().is_empty()) {
            if line.starts_with("### ") {
                ids.problems.push(Problem::new(
                    "ids",
                    file,
                    format!("`{line}` is under `{CLOSED}`, which holds only closed IDs.  Fix: put open entries above `{CLOSED}`."),
                ));
                continue;
            }
            let Some(id) = line
                .strip_prefix("* ")
                .and_then(|l| l.split_once(':'))
                .map(|(id, _)| id)
                .filter(|id| is_id(id))
            else {
                ids.problems.push(Problem::new(
                    "ids",
                    file,
                    format!("`{line}` under `{CLOSED}` is not `* ID: what closed it`, so it closes nothing.  Fix: write it as `* TD-1: closed by ...`."),
                ));
                continue;
            };
            if !ids.closed.insert(id.to_string()) {
                ids.problems.push(Problem::new(
                    "ids",
                    file,
                    format!("`{id}` is closed twice.  Fix: keep one bullet under `{CLOSED}`."),
                ));
            }
        }
    }
    for (file, id) in defined_in
        .into_iter()
        .filter(|(_, id)| ids.closed.contains(*id))
    {
        ids.problems.push(Problem::new(
            "ids",
            file,
            format!("`{id}` is closed but defined again.  Fix: give the new entry the next free number; IDs are never reused."),
        ));
    }
    ids
}

/// `MEM-1`: two to six capitals, a dash, digits.
fn is_id(token: &str) -> bool {
    let Some((prefix, number)) = token.split_once('-') else {
        return false;
    };
    (2..=6).contains(&prefix.len())
        && prefix.chars().all(|c| c.is_ascii_uppercase())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// Every ID-shaped token whose prefix is one in use (`MEM-7` when `MEM-1`
/// exists) must be defined, so a reference never points at nothing.  A
/// closed ID may appear only where [`may_name_closed_ids`] allows it.
fn ids_defined(file: &str, text: &str, ids: &Ids) -> Vec<Problem> {
    let prefixes: BTreeSet<&str> = ids
        .defined
        .iter()
        .chain(&ids.closed)
        .filter_map(|id| id.split('-').next())
        .collect();
    let tokens: BTreeSet<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|t| is_id(t))
        .filter(|t| prefixes.contains(t.split('-').next().unwrap_or_default()))
        .collect();
    tokens
        .into_iter()
        .filter(|t| !ids.defined.contains(*t))
        .filter(|t| !(ids.closed.contains(*t) && may_name_closed_ids(file)))
        .map(|t| {
            let message = if ids.closed.contains(t) {
                format!("`{t}` is closed (a bullet under `{CLOSED}`), so nothing here may rely on it.  Fix: remove the reference, or name what closed it; only plans, decisions and review logs name closed IDs.")
            } else {
                format!("`{t}` is not defined in docs/invariants.md or docs/tech-debt.md.  Fix: define it there as `### {t}: ...`, or correct the reference.")
            };
            Problem::new("ids", file, message)
        })
        .collect()
}

/// The ID problems of one doc.  Only the bullets of the ID files'
/// `## Closed` sections may open with a closed ID, so only that ID is left
/// out; a `## Closed` heading in another doc is checked like any other
/// section.
fn id_references(file: &str, text: &str, ids: &Ids) -> Vec<Problem> {
    if ID_FILES.contains(&file) {
        ids_defined(file, &without_closed_ids(text), ids)
    } else {
        ids_defined(file, text, ids)
    }
}

/// Plans, decisions and review logs are records of their time, so they may
/// name an ID that has since closed.
fn may_name_closed_ids(file: &str) -> bool {
    !names_current_paths(file)
}

/// Attributes that mark a test function.
const TEST_ATTRIBUTES: &[&str] = &["#[test]", "#[wasm_bindgen_test", "#[tokio::test"];

/// The test functions in the sources: those `cargo test` runs, and those
/// marked `#[ignore]`, which it does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tests {
    pub run: BTreeSet<String>,
    pub ignored: BTreeSet<String>,
}

/// The test functions in the given Rust sources: each `fn` whose attributes
/// include a test attribute.
pub fn test_fn_names(sources: &[String]) -> Tests {
    let mut tests = Tests::default();
    for source in sources {
        let (mut is_test, mut is_ignored) = (false, false);
        for line in source.lines().map(str::trim_start) {
            if TEST_ATTRIBUTES.iter().any(|a| line.starts_with(a)) {
                is_test = true;
                continue;
            }
            if line.starts_with("#[ignore") {
                is_ignored = true;
                continue;
            }
            if line.starts_with("#[") || line.starts_with("//") || line.is_empty() {
                continue;
            }
            if let Some(name) = fn_name(line).filter(|_| is_test) {
                if is_ignored {
                    tests.ignored.insert(name);
                } else {
                    tests.run.insert(name);
                }
            }
            (is_test, is_ignored) = (false, false);
        }
    }
    tests
}

/// The name in a line that declares a function (`pub async fn name(`).
fn fn_name(line: &str) -> Option<String> {
    let (at, _) = line
        .match_indices("fn ")
        .find(|(i, _)| *i == 0 || line.as_bytes()[i - 1] == b' ')?;
    let name: String = line[at + 3..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// The tests an invariant names on its `* Enforced by:` line exist, so a
/// rule never claims a check that was renamed or deleted.
pub fn enforcing_tests_exist(invariants: &str, tests: &Tests) -> Vec<Problem> {
    invariants
        .lines()
        .filter(|l| l.trim_start().starts_with("* Enforced by:"))
        .flat_map(inline_code)
        .filter(|span| {
            span.contains('_') && span.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
        .filter(|name| !tests.run.contains(name))
        .map(|name| {
            let why = if tests.ignored.contains(&name) {
                format!("names test `{name}`, which is `#[ignore]`d, so `cargo test` never runs it.  Fix: name a test that runs, or record the gap in docs/tech-debt.md.")
            } else {
                format!("names test `{name}`, but no test function `{name}` exists.  Fix: name the test that now enforces the rule, or move the rule's gap to docs/tech-debt.md.")
            };
            Problem::new("tests", "docs/invariants.md", why)
        })
        .collect()
}

/// The folders under `docs/` that one index span (`reviews/`) lists as a
/// whole.  A review log is added each round, so one line covers them all.
/// Plans and decisions keep one line each, so a plan cannot hide.
const INDEXED_BY_FOLDER: &[&str] = &["reviews/"];

/// Every doc under `docs/` is listed in `docs/README.md`, the index agents
/// read to find the rest.  A doc in a folder of [`INDEXED_BY_FOLDER`] is
/// listed when the index has that folder's span.
fn docs_indexed(markdown: &[(String, String)], index: Option<&str>) -> Vec<Problem> {
    let Some(index) = index else {
        return vec![Problem::new(
            "index",
            "docs/README.md",
            "is missing.  Fix: add docs/README.md listing every doc and when to read it.".into(),
        )];
    };
    let listed: BTreeSet<String> = inline_code(index).into_iter().collect();
    markdown
        .iter()
        .map(|(f, _)| f)
        .filter(|f| f.starts_with("docs/") && *f != "docs/README.md")
        .filter(|f| {
            let in_docs = &f["docs/".len()..];
            let folder_listed = INDEXED_BY_FOLDER
                .iter()
                .any(|folder| in_docs.starts_with(folder) && listed.contains(*folder));
            !listed.contains(*f) && !listed.contains(in_docs) && !folder_listed
        })
        .map(|f| {
            Problem::new(
                "index",
                "docs/README.md",
                format!(
                    "does not list `{}`.  Fix: add it with one line on when to read it.",
                    &f["docs/".len()..]
                ),
            )
        })
        .collect()
}

/// One glob from a rule's `paths:`, and whether YAML can read it as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleGlob {
    pub pattern: String,
    pub valid_yaml: bool,
}

/// Characters that cannot start a plain (unquoted) YAML scalar.
const YAML_INDICATORS: &[char] = &['*', '&', '!', '{', '[', '%', '@', '`', '|', '>'];

/// `"x"` or `'x'` without its quotes, and whether it was quoted.
fn unquote(s: &str) -> (&str, bool) {
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
            return (inner, true);
        }
    }
    (s, false)
}

/// `s` split at commas outside braces, so `a,{b,c}` is two parts.
fn split_outside_braces(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// A YAML value without a trailing ` # comment`, which an unquoted value
/// may carry; a quoted value ends at its closing quote.
fn strip_comment(raw: &str) -> &str {
    let raw = raw.trim();
    for q in ['"', '\''] {
        if let Some(end) = raw.strip_prefix(q).and_then(|r| r.find(q)) {
            return &raw[..end + 2];
        }
    }
    raw.split(" #").next().unwrap_or_default()
}

fn rule_glob(raw: &str, quoted_outside: bool) -> RuleGlob {
    let (pattern, quoted) = unquote(strip_comment(raw));
    RuleGlob {
        pattern: pattern.trim().to_string(),
        valid_yaml: quoted || quoted_outside || !pattern.starts_with(YAML_INDICATORS),
    }
}

/// The `paths:` globs in a rule's frontmatter, in any form Claude Code
/// reads: a YAML list, a `[a, b]` flow list, or a comma-separated string.
pub fn rule_globs(rule: &str) -> Option<Vec<RuleGlob>> {
    let rule = rule.replace("\r\n", "\n");
    let body = rule.strip_prefix("---\n")?;
    let front = &body[..body.find("\n---")?];
    let mut lines = front.lines().skip_while(|l| !l.starts_with("paths:"));
    let inline = lines.next()?.strip_prefix("paths:")?.trim();
    if inline.is_empty() {
        return Some(
            lines
                .take_while(|l| l.starts_with([' ', '\t', '-']))
                .filter_map(|l| l.trim().strip_prefix("- "))
                .map(|g| rule_glob(g, false))
                .collect(),
        );
    }
    if let Some(flow) = inline.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Some(
            split_outside_braces(flow)
                .into_iter()
                .map(|g| rule_glob(g, false))
                .collect(),
        );
    }
    let (string, quoted) = unquote(strip_comment(inline));
    let valid = quoted || !string.starts_with(YAML_INDICATORS);
    Some(
        split_outside_braces(string)
            .into_iter()
            .map(|g| RuleGlob {
                pattern: g.trim().to_string(),
                valid_yaml: valid,
            })
            .collect(),
    )
}

/// `a.{rs,toml}` as `a.rs` and `a.toml`.
fn expand_braces(glob: &str) -> Vec<String> {
    let Some(open) = glob.find('{') else {
        return vec![glob.to_string()];
    };
    let Some(close) = glob[open..].find('}').map(|i| open + i) else {
        return vec![glob.to_string()];
    };
    glob[open + 1..close]
        .split(',')
        .flat_map(|alt| expand_braces(&format!("{}{alt}{}", &glob[..open], &glob[close + 1..])))
        .collect()
}

/// `*` matches within one path segment, `**` any number of segments, and
/// `{a,b}` either alternative.
pub fn glob_matches(glob: &str, path: &str) -> bool {
    expand_braces(glob)
        .iter()
        .any(|g| glob_matches_one(g, path))
}

fn glob_matches_one(glob: &str, path: &str) -> bool {
    fn segments(globs: &[&str], parts: &[&str]) -> bool {
        match (globs.first(), parts.first()) {
            (None, None) => true,
            (Some(&"**"), _) => {
                segments(&globs[1..], parts) || (!parts.is_empty() && segments(globs, &parts[1..]))
            }
            (Some(g), Some(p)) => segment(g, p) && segments(&globs[1..], &parts[1..]),
            _ => false,
        }
    }
    fn segment(glob: &str, part: &str) -> bool {
        match glob.split_once('*') {
            None => glob == part,
            Some((head, tail)) => {
                part.starts_with(head)
                    && part
                        .char_indices()
                        .map(|(i, _)| i)
                        .chain([part.len()])
                        .filter(|i| *i >= head.len())
                        .any(|i| segment(tail, &part[i..]))
            }
        }
    }
    let globs: Vec<&str> = glob.split('/').collect();
    let parts: Vec<&str> = path.split('/').collect();
    segments(&globs, &parts)
}

fn rules_match_files(markdown: &[(String, String)], files: &[String]) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (file, text) in markdown
        .iter()
        .filter(|(f, _)| f.starts_with(".claude/rules/"))
    {
        let Some(globs) = rule_globs(text).filter(|g| !g.is_empty()) else {
            problems.push(Problem::new(
                "rules",
                file,
                "has no `paths:` frontmatter, so it would load in every session.  Fix: start the file with `---`, a `paths:` list of globs, and `---`; put rules for every session in CLAUDE.md.".into(),
            ));
            continue;
        };
        for glob in &globs {
            let pattern = &glob.pattern;
            if !glob.valid_yaml {
                problems.push(Problem::new(
                    "rules",
                    file,
                    format!("glob `{pattern}` starts with a character YAML reserves, so the frontmatter does not parse and Claude Code loads the rule in every session.  Fix: quote it (`- \"{pattern}\"`)."),
                ));
            } else if !files.iter().any(|f| glob_matches(pattern, f)) {
                problems.push(Problem::new(
                    "rules",
                    file,
                    format!("glob `{pattern}` matches no file, so the rule never loads.  Fix: point it at the files it is about."),
                ));
            }
        }
    }
    problems
}

fn decisions_well_formed(markdown: &[(String, String)]) -> Vec<Problem> {
    let decisions: Vec<&(String, String)> = markdown
        .iter()
        .filter(|(f, _)| f.starts_with("docs/decisions/"))
        .collect();
    let mut problems = Vec::new();
    for (n, (file, text)) in decisions.iter().enumerate() {
        let expected = format!("docs/decisions/{:04}-", n + 1);
        if !file.starts_with(&expected) {
            problems.push(Problem::new(
                "decisions",
                file,
                format!("is out of sequence: expected a name starting `{expected}`.  Fix: number decisions 0001, 0002, ... with no gaps; never renumber an accepted one."),
            ));
        }
        let status = text
            .lines()
            .find_map(|l| l.strip_prefix("Status: "))
            .map(|s| s.split_whitespace().next().unwrap_or_default());
        if !status.is_some_and(|s| DECISION_STATUSES.contains(&s)) {
            problems.push(Problem::new(
                "decisions",
                file,
                format!("needs a `Status: <{}>` line.  Fix: agents write `proposed`; only a person accepts.", DECISION_STATUSES.join("|")),
            ));
        }
    }
    problems
}

fn plans_well_formed(markdown: &[(String, String)]) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (file, text) in markdown
        .iter()
        .filter(|(f, _)| f.starts_with("docs/plans/"))
    {
        if file == "docs/plans/PLANS.md" {
            continue;
        }
        if !(file.starts_with("docs/plans/active/") || file.starts_with("docs/plans/completed/")) {
            problems.push(Problem::new(
                "plans",
                file,
                "is outside docs/plans/active/ and docs/plans/completed/.  Fix: move it into one of them.".into(),
            ));
        }
        for heading in PLAN_SECTIONS
            .iter()
            .filter(|h| !text.lines().any(|l| l.trim_end() == **h))
        {
            problems.push(Problem::new(
                "plans",
                file,
                format!("lacks the living section `{heading}`.  Fix: add it (see docs/plans/PLANS.md) and keep it current at every stopping point."),
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn members_are_read_from_a_one_line_or_multi_line_list() {
        assert_eq!(
            workspace_members("[workspace]\nmembers = [\"dsp\", \"engine\"]\n"),
            ["dsp", "engine"]
        );
        assert_eq!(
            workspace_members("members = [\n  \"a\",\n  \"b\",\n]\n[workspace.package]\n"),
            ["a", "b"]
        );
        assert_eq!(
            workspace_members(
                "[workspace]\ndefault-members = [\"app\"]\nmembers = [\"dsp\", \"app\"]\n"
            ),
            ["dsp", "app"]
        );
        assert_eq!(
            workspace_members("members = [\n  # the [pure] crates\n  \"dsp\",\n]\n"),
            ["dsp"]
        );
    }

    #[test]
    fn the_package_name_comes_from_the_package_table_only() {
        let manifest = "[package]\nname = \"stem-worker\"\n\n[dependencies]\nname = \"x\"\n";
        assert_eq!(package_name(manifest).as_deref(), Some("stem-worker"));
        assert_eq!(package_name("[dependencies]\nname = \"x\"\n"), None);
    }

    #[test]
    fn code_spans_inside_fences_are_ignored() {
        let md = "see `a/b.rs` and `c`\n```bash\n`not/me`\n```\nthen `d/`";
        assert_eq!(inline_code(md), ["a/b.rs", "c", "d/"]);
    }

    #[test]
    fn a_section_ends_at_the_next_heading_of_its_level() {
        let md = "# T\n## Layout\n* `dsp/`\n### Sub\n* `x/`\n## Commands\n* `y/`\n";
        let layout = section(md, "## Layout").unwrap();
        assert!(layout.contains("`dsp/`") && layout.contains("`x/`"));
        assert!(!layout.contains("`y/`"));
        assert_eq!(section(md, "## Missing"), None);
        let prefixed = "## Layout of the UI\n* `ui/`\n## Layout\n* `dsp/`\n";
        assert_eq!(section(prefixed, "## Layout"), Some("\n* `dsp/`\n"));
    }

    #[test]
    fn only_spans_that_name_repo_files_are_paths() {
        let t = top(&["app", "engine", "docs", ".claude"]);
        assert_eq!(
            repo_path("engine/src/song.rs:13-49", &t).as_deref(),
            Some("engine/src/song.rs")
        );
        assert_eq!(
            repo_path("docs/ARCHITECTURE.md#crates", &t).as_deref(),
            Some("docs/ARCHITECTURE.md")
        );
        assert_eq!(
            repo_path("plans/PLANS.md", &t).as_deref(),
            Some("plans/PLANS.md")
        );
        assert_eq!(
            repo_path(".claude/rules/", &t).as_deref(),
            Some(".claude/rules")
        );
        assert_eq!(repo_path("app/dist", &t), None, "a build creates it");
        assert_eq!(repo_path("engine::Samples", &t), None);
        assert_eq!(repo_path("xtask e2e", &t), None);
        assert_eq!(repo_path("v/<id>/", &t), None);
        assert_eq!(repo_path("sessions/<id>/", &t), None);
        assert_eq!(
            repo_path("src/shaders/", &t).as_deref(),
            Some("src/shaders"),
            "relative to a crate, so checked against every crate"
        );
        assert_eq!(repo_path("src/lib.rs", &t).as_deref(), Some("src/lib.rs"));
        assert_eq!(
            repo_path(".cluade/rules/gpu.md", &t).as_deref(),
            Some(".cluade/rules/gpu.md"),
            "a typo in a dotted folder is still checked"
        );
        assert_eq!(repo_path(".md", &t), None);
        assert_eq!(
            repo_path("enigne/src/song.rs", &t).as_deref(),
            Some("enigne/src/song.rs"),
            "a typo in the first folder is still checked"
        );
        assert_eq!(repo_path("https://example.com/a.md", &t), None);
    }

    #[test]
    fn paths_resolve_from_the_root_the_file_or_docs() {
        assert_eq!(path_bases("CLAUDE.md"), [""]);
        assert_eq!(
            path_bases("docs/references/a.md"),
            ["", "docs/references/", "docs/"]
        );
        assert!(names_current_paths("docs/tech-debt.md"));
        assert!(!names_current_paths("docs/plans/active/a.md"));
        assert!(!names_current_paths("docs/decisions/0001-a.md"));
    }

    fn missing(tree: &Tree, file: &str, text: &str) -> Vec<String> {
        let t = top(&["docs", "engine", ".claude"]);
        paths_exist(
            tree,
            file,
            text,
            &t,
            &["engine".to_string(), "app".to_string()],
        )
        .iter()
        .map(|p| p.message.split('`').nth(1).unwrap().to_string())
        .collect()
    }

    #[test]
    fn a_missing_path_is_reported_and_a_present_one_is_not() {
        let tree = Tree::new(&[
            "docs/invariants.md".to_string(),
            "docs/ARCHITECTURE.md".to_string(),
            "engine/src/song.rs".to_string(),
        ]);
        let text = "see `invariants.md`, `docs/invariants.md`, `engine/src`, `engine/gone.rs` and `gone.md`";
        assert_eq!(
            missing(&tree, "docs/tech-debt.md", text),
            ["engine/gone.rs", "gone.md"]
        );
        assert!(missing(&tree, "docs/plans/active/a.md", text).is_empty());
    }

    #[test]
    fn a_crate_relative_path_must_exist_in_some_crate() {
        let tree = Tree::new(&[
            "engine/src/song.rs".to_string(),
            "app/src/lib.rs".to_string(),
        ]);
        assert_eq!(
            missing(
                &tree,
                "CLAUDE.md",
                "`src/lib.rs`, `src/song.rs`, `src/gone.rs`"
            ),
            ["src/gone.rs"]
        );
    }

    #[test]
    fn spans_do_not_run_across_blocks() {
        let gone = "engine/gone.rs".to_string();
        assert!(inline_code("* a ` stray\n* see `engine/gone.rs` here\n").contains(&gone));
        assert!(inline_code("# A ` heading\nsee `engine/gone.rs`\n").contains(&gone));
        assert!(inline_code("| a ` | b |\n| `engine/gone.rs` | c |\n").contains(&gone));
        assert!(inline_code("1. a ` x\n2. `engine/gone.rs`\n").contains(&gone));
        assert_eq!(
            inline_code("an escaped \\\\`engine/gone.rs`"),
            ["engine/gone.rs"]
        );
    }

    #[test]
    fn fences_open_and_close_as_commonmark_says() {
        let md = "````md\n```\n`inside`\n```\n````\n`after`\n~~~\n`tilde`\n~~~\n`last`\n";
        assert_eq!(inline_code(md), ["after", "last"]);
    }

    #[test]
    fn paths_are_matched_by_exact_case_and_resolved() {
        let tree = Tree::new(&[
            "docs/ARCHITECTURE.md".to_string(),
            "engine/src/song.rs".to_string(),
        ]);
        let text = "`docs/architecture.md`, `./engine/gone.rs`, `../ARCHITECTURE.md`, `../../../x.md`, `./engine/src/song.rs`";
        assert_eq!(
            missing(&tree, "docs/references/a.md", text),
            ["docs/architecture.md", "engine/gone.rs", "../../../x.md"]
        );
    }

    #[test]
    fn sites_versions_and_actions_are_not_paths() {
        let t = top(&["app", "docs", ".claude"]);
        for span in [
            "actions/checkout@v4.2.0",
            "docs.rs/wgpu/latest/wgpu/struct.Limits.html",
            "0.5/1.0",
            ".flac",
            ".gitkeep",
        ] {
            assert_eq!(repo_path(span, &t), None, "{span}");
        }
        assert_eq!(
            repo_path("app/distortion.rs", &t).as_deref(),
            Some("app/distortion.rs")
        );
        assert_eq!(repo_path("app/dist/index.html", &t), None);
    }

    #[test]
    fn code_spans_follow_commonmark() {
        assert_eq!(
            inline_code("| `` ` `` | `engine/gone.rs` |"),
            ["`", "engine/gone.rs"]
        );
        assert_eq!(inline_code("``engine/gone.rs``"), ["engine/gone.rs"]);
        assert_eq!(
            inline_code("a \\` b and `engine/gone.rs`"),
            ["engine/gone.rs"]
        );
        assert_eq!(
            inline_code("a span `that\nwraps` and `next`"),
            ["that wraps", "next"]
        );
        assert_eq!(
            inline_code("an `unclosed span\n\nthen `x`"),
            ["x"],
            "a span never crosses a paragraph"
        );
    }

    #[test]
    fn a_missing_crate_is_reported_in_each_doc_that_lists_crates() {
        let members = [
            Member {
                dir: "dsp".into(),
                package: "dsp".into(),
            },
            Member {
                dir: "worker".into(),
                package: "stem-worker".into(),
            },
            Member {
                dir: "synth".into(),
                package: "synth".into(),
            },
        ];
        let claude = "## Layout\n* `dsp/`: x\n* `worker/`: y\n## Commands\n* `synth/`\n";
        let architecture = "## Crates\n* `dsp`\n* `stem-worker`\n* `synth`\n";
        let problems = crates_documented(
            &members,
            &[
                ("CLAUDE.md", Some(claude), "## Layout", "`<dir>/`"),
                (
                    "docs/ARCHITECTURE.md",
                    Some(architecture),
                    "## Crates",
                    "`<package>`",
                ),
                ("README.md", None, "## Layout", "`<dir>/`"),
            ],
        );
        let files: Vec<&str> = problems.iter().map(|p| p.file.as_str()).collect();
        assert_eq!(files, ["CLAUDE.md", "README.md"], "{problems:#?}");
        assert!(
            problems[0].message.contains("`synth`"),
            "a mention outside the section does not count"
        );
    }

    #[test]
    fn ids_are_unique_and_every_reference_is_defined() {
        let files = vec![
            (
                "docs/invariants.md".to_string(),
                "### MEM-1: a\n### MEM-2: b\n### MEM-1: again\n".to_string(),
            ),
            ("docs/tech-debt.md".to_string(), "### TD-1: c\n".to_string()),
        ];
        let ids = defined_ids(&files);
        assert_eq!(ids.defined.len(), 3);
        assert_eq!(ids.problems.len(), 1, "MEM-1 twice");
        let malformed = defined_ids(&[(
            "docs/invariants.md".to_string(),
            "### SEC-1 - no colon\n### LEGIBLE-1: seven letters\n### OK-1: fine\n".to_string(),
        )]);
        assert_eq!(malformed.defined.len(), 1);
        assert_eq!(malformed.problems.len(), 2, "{:#?}", malformed.problems);
        let problems = ids_defined(
            "CLAUDE.md",
            "see MEM-2, MEM-9 and TD-4; not SHA-256 or UTF-8",
            &ids,
        );
        let named: Vec<&str> = problems
            .iter()
            .map(|p| p.message.split('`').nth(1).unwrap())
            .collect();
        assert_eq!(named, ["MEM-9", "TD-4"]);
    }

    #[test]
    fn closed_ids_are_named_only_in_plans_decisions_and_review_logs() {
        let tech_debt = "## Gaps\n\n### TD-2: open\n\nsee TD-1\n\n## Closed\n\n* TD-1: closed by `a_test`.\n\n## After\n\nTD-1 again\n";
        let ids = defined_ids(&[("docs/tech-debt.md".to_string(), tech_debt.to_string())]);
        assert!(ids.problems.is_empty(), "{:#?}", ids.problems);
        assert_eq!(ids.closed, BTreeSet::from(["TD-1".to_string()]));
        assert_eq!(
            without_closed_ids(tech_debt),
            "## Gaps\n\n### TD-2: open\n\nsee TD-1\n\n\n closed by `a_test`.\n\n## After\n\nTD-1 again"
        );

        let text = "closes TD-1, keeps TD-2, not TD-3";
        let in_plan = ids_defined("docs/plans/active/a.md", text, &ids);
        assert_eq!(in_plan.len(), 1, "{in_plan:#?}");
        assert!(in_plan[0].message.starts_with("`TD-3` is not defined"));
        let in_decision = ids_defined("docs/decisions/0001-a.md", text, &ids);
        assert_eq!(in_decision.len(), 1, "{in_decision:#?}");
        let in_doc = ids_defined("docs/invariants.md", text, &ids);
        assert_eq!(in_doc.len(), 2, "{in_doc:#?}");
        assert!(
            in_doc[0].message.starts_with("`TD-1` is closed"),
            "{}",
            in_doc[0].message
        );
        assert!(in_doc[0].message.contains("Fix: "));
        for text in [
            "see TD-1\n\n## Closed\n",
            "## Closed\n\n## After\n\nTD-1 again",
        ] {
            let outside_closed = ids_defined("docs/tech-debt.md", &without_closed_ids(text), &ids);
            assert_eq!(outside_closed.len(), 1, "{text:?}: {outside_closed:#?}");
        }
        let inside_closed = ids_defined(
            "docs/tech-debt.md",
            &without_closed_ids("## Closed\n\n* TD-1: x\n"),
            &ids,
        );
        assert!(inside_closed.is_empty(), "{inside_closed:#?}");
    }

    /// Only the opening ID of each bullet in the ID files' `## Closed`
    /// sections is left out of the check.
    #[test]
    fn a_closed_section_elsewhere_is_checked() {
        let ids = defined_ids(&[(
            "docs/tech-debt.md".to_string(),
            "### TD-2: open\n\n## Closed\n\n* TD-1: closed.\n".to_string(),
        )]);
        let text = "## Closed\n\n* TD-1: x, and TD-99\n";
        let elsewhere = id_references("docs/README.md", text, &ids);
        let named: Vec<&str> = elsewhere
            .iter()
            .map(|p| p.message.split('`').nth(1).unwrap())
            .collect();
        assert_eq!(named, ["TD-1", "TD-99"]);
        let in_tech_debt = id_references("docs/tech-debt.md", text, &ids);
        let named: Vec<&str> = in_tech_debt
            .iter()
            .map(|p| p.message.split('`').nth(1).unwrap())
            .collect();
        assert_eq!(named, ["TD-99"], "the bullet's text is checked");
    }

    /// Every ID prefix is still checked when all its IDs are closed.
    #[test]
    fn a_closed_prefix_is_still_checked() {
        let ids = defined_ids(&[(
            "docs/tech-debt.md".to_string(),
            "## Closed\n\n* TD-1: closed.\n".to_string(),
        )]);
        let problems = ids_defined("docs/plans/active/a.md", "TD-1 and TD-9", &ids);
        assert_eq!(problems.len(), 1, "{problems:#?}");
        assert!(problems[0].message.starts_with("`TD-9` is not defined"));
    }

    #[test]
    fn a_closed_id_is_never_reused_or_malformed() {
        let ids = defined_ids(&[(
            "docs/tech-debt.md".to_string(),
            "### TD-1: back\n\n## Closed\n\n* TD-1: closed.\n* TD-1: again.\n* closed TD-4\n"
                .to_string(),
        )]);
        let messages: Vec<&str> = ids.problems.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(messages.len(), 3, "{messages:#?}");
        let reused = defined_ids(&[
            (
                "docs/invariants.md".to_string(),
                "### TD-1: back\n".to_string(),
            ),
            (
                "docs/tech-debt.md".to_string(),
                "## Closed\n\n* TD-1: closed.\n".to_string(),
            ),
        ]);
        assert_eq!(reused.problems.len(), 1, "{:#?}", reused.problems);
        assert_eq!(reused.problems[0].file, "docs/invariants.md");
        assert!(messages[0].starts_with("`TD-1` is closed twice"));
        assert!(
            messages[1]
                .starts_with("`* closed TD-4` under `## Closed` is not `* ID: what closed it`")
        );
        assert!(messages[2].starts_with("`TD-1` is closed but defined again"));
    }

    /// A new entry added at the end of the file lands under `## Closed`;
    /// the message says where it goes.
    #[test]
    fn an_open_entry_under_closed_is_reported() {
        let ids = defined_ids(&[(
            "docs/tech-debt.md".to_string(),
            "## Closed\n\n* TD-1: closed.\n\n### TD-2: new gap\n".to_string(),
        )]);
        let messages: Vec<&str> = ids.problems.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "`### TD-2: new gap` is under `## Closed`, which holds only closed IDs.  Fix: put open entries above `## Closed`."
            ]
        );
    }

    #[test]
    fn the_guide_names_every_command_skill_and_subagent() {
        let main_rs = "match cmd.as_str() {\n    \"gen-song\" => {\n    \"hpss\" | \"sdr\" => x,\n    Err(e) => y,\n    other => bail!(),\n}\n";
        let commands = xtask_commands(main_rs);
        let named: Vec<&str> = commands.iter().map(String::as_str).collect();
        assert_eq!(named, ["gen-song", "hpss", "sdr"]);
        let wrapped = "        \"hpss\" | \"sdr\" | \"classic\" | \"roformer\" | \"roformer-convert\" | \"perf\"\n        | \"gen-song\" => run(),\n        \"live\" if ok => go(),\n";
        let wrapped_commands: Vec<String> = xtask_commands(wrapped).into_iter().collect();
        assert_eq!(
            wrapped_commands,
            [
                "classic",
                "gen-song",
                "hpss",
                "live",
                "perf",
                "roformer",
                "roformer-convert",
                "sdr"
            ]
        );
        let files = [
            ".claude/skills/review/SKILL.md".to_string(),
            ".claude/skills/review/reference.md".to_string(),
            ".claude/agents/gpu-reviewer.md".to_string(),
            ".claude/rules/dsp.md".to_string(),
            ".claude/commands/ship.md".to_string(),
        ];
        let tools: BTreeSet<String> = commands
            .into_iter()
            .chain(agent_capabilities(&files))
            .collect();
        let guide =
            "Run `cargo run -p xtask -- gen-song`, then `/review`.\n\n| `hpss`, `sdr` | x |\n";
        let problems = guide_covers(guide, &tools);
        let missing: Vec<&str> = problems
            .iter()
            .map(|p| p.message.split('`').nth(1).unwrap())
            .collect();
        assert_eq!(missing, ["gpu-reviewer", "ship"]);
    }

    #[test]
    fn the_map_lists_exactly_the_current_rules() {
        let invariants = "### MEM-1: a\n### MEM-2: b\n### GPU-9: Retired: no longer needed\n";
        let claude = "## Rules\n\n* MEM-1: a\n* DSP-1: c\n\n## Working here\n* MEM-2: not in the rules section\n";
        let problems = map_lists_every_rule(claude, invariants);
        let named: Vec<&str> = problems
            .iter()
            .map(|p| p.message.split('`').nth(1).unwrap())
            .collect();
        assert_eq!(named, ["MEM-2", "DSP-1"]);
        assert!(map_lists_every_rule("## Rules\n* MEM-1: a\n* MEM-2: b\n", invariants).is_empty());
    }

    #[test]
    fn only_test_functions_count_as_tests() {
        let source = "#[test]\nfn a_buffer_is_shared() {}\n\n#[wasm_bindgen_test]\n#[ignore]\npub async fn another_one(x: u8) {}\n\nfn a_helper() {}\n#[test]\n// why\nfn after_a_comment() {}\nconst S: &str = \"fn in_a_string() {}\";\n";
        let tests = test_fn_names(&[source.to_string()]);
        let run: Vec<&str> = tests.run.iter().map(String::as_str).collect();
        let ignored: Vec<&str> = tests.ignored.iter().map(String::as_str).collect();
        assert_eq!(run, ["a_buffer_is_shared", "after_a_comment"]);
        assert_eq!(ignored, ["another_one"]);
    }

    #[test]
    fn a_renamed_or_ignored_enforcing_test_is_reported() {
        let tests = test_fn_names(&[
            "#[test]\nfn a_buffer_is_shared() {}\n#[test]\n#[ignore]\nfn a_slow_check() {}\n"
                .to_string(),
        ]);
        let invariants = "### MEM-1: x\n\n* Enforced by: `a_buffer_is_shared`, `gone_test`, `a_slow_check`, CI `test` job, `engine/src/song.rs`.\n* Gaps: `not_checked_here`\n";
        let problems = enforcing_tests_exist(invariants, &tests);
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(
            problems[0]
                .message
                .contains("`gone_test`, but no test function")
        );
        assert!(
            problems[1]
                .message
                .contains("`a_slow_check`, which is `#[ignore]`d")
        );
    }

    #[test]
    fn globs_match_by_segment() {
        assert!(glob_matches("dsp/**", "dsp/src/lib.rs"));
        assert!(glob_matches(
            "roformer-gpu/src/shaders/*.wgsl",
            "roformer-gpu/src/shaders/matmul.wgsl"
        ));
        assert!(glob_matches("**/Cargo.toml", "Cargo.toml"));
        assert!(glob_matches("**/Cargo.toml", "app/Cargo.toml"));
        assert!(!glob_matches("dsp/*.rs", "dsp/src/lib.rs"));
        assert!(!glob_matches("engine/**", "engineer/a.rs"));
        assert!(!glob_matches(
            "roformer-gpu/src/shaders/*.wgsl",
            "roformer-gpu/src/shaders/mod.rs"
        ));
    }

    #[test]
    fn globs_match_names_that_are_not_ascii() {
        assert!(glob_matches("**/*.md", "docs/é.md"));
        assert!(glob_matches("audio/*ncé.mp3", "audio/Beyoncé.mp3"));
        assert!(!glob_matches("*.rs", "é.md"));
    }

    fn patterns(rule: &str) -> Option<Vec<(String, bool)>> {
        rule_globs(rule).map(|g| g.into_iter().map(|g| (g.pattern, g.valid_yaml)).collect())
    }

    #[test]
    fn rule_paths_are_read_in_every_form_claude_code_accepts() {
        let both = Some(vec![
            ("dsp/**".to_string(), true),
            ("engine/src/*.rs".to_string(), true),
        ]);
        assert_eq!(
            patterns("---\npaths:\n  - \"dsp/**\"\n  - engine/src/*.rs\n---\nbody\n"),
            both
        );
        assert_eq!(
            patterns("---\npaths:\n  - 'dsp/**'\n  - engine/src/*.rs\n---\n"),
            both
        );
        assert_eq!(
            patterns("---\r\npaths:\r\n  - \"dsp/**\"\r\n  - engine/src/*.rs\r\n---\r\n"),
            both
        );
        assert_eq!(patterns("---\npaths: dsp/**, engine/src/*.rs\n---\n"), both);
        assert_eq!(
            patterns("---\npaths: [\"dsp/**\", engine/src/*.rs]\n---\n"),
            both
        );
        assert_eq!(
            patterns("---\npaths:\n  - **/Cargo.toml\n---\n"),
            Some(vec![("**/Cargo.toml".to_string(), false)]),
            "YAML reads a leading * as an alias"
        );
        assert_eq!(
            patterns("---\npaths: \"**/Cargo.toml\"\n---\n"),
            Some(vec![("**/Cargo.toml".to_string(), true)])
        );
        assert_eq!(
            patterns("---\npaths: [\"src/*.{rs,toml}\", app/**]\n---\n"),
            Some(vec![
                ("src/*.{rs,toml}".to_string(), true),
                ("app/**".to_string(), true)
            ])
        );
        assert_eq!(
            patterns("---\npaths:\n  - \"dsp/**\" # the dsp crate\n  - app/** # the app\n---\n"),
            Some(vec![
                ("dsp/**".to_string(), true),
                ("app/**".to_string(), true)
            ])
        );
        assert!(glob_matches("engine/src/*.{rs,toml}", "engine/src/a.toml"));
        assert!(!glob_matches("engine/src/*.{rs,toml}", "engine/src/a.md"));
    }

    #[test]
    fn a_rule_without_paths_or_with_a_dead_glob_is_reported() {
        let files = vec!["dsp/src/lib.rs".to_string()];
        let markdown = vec![
            (
                ".claude/rules/dsp.md".to_string(),
                "---\npaths:\n  - \"dsp/**\"\n---\n".to_string(),
            ),
            (
                ".claude/rules/old.md".to_string(),
                "---\npaths:\n  - \"gone/**\"\n---\n".to_string(),
            ),
            (
                ".claude/rules/bare.md".to_string(),
                "no frontmatter\n".to_string(),
            ),
            (
                ".claude/rules/alias.md".to_string(),
                "---\npaths:\n  - **/lib.rs\n---\n".to_string(),
            ),
        ];
        let files_with_problems: Vec<String> = rules_match_files(&markdown, &files)
            .into_iter()
            .map(|p| p.file)
            .collect();
        assert_eq!(
            files_with_problems,
            [
                ".claude/rules/old.md",
                ".claude/rules/bare.md",
                ".claude/rules/alias.md"
            ]
        );
    }

    #[test]
    fn decisions_are_numbered_in_order_and_carry_a_status() {
        let markdown = vec![
            (
                "docs/decisions/0001-a.md".to_string(),
                "# A\n\nStatus: accepted (2026-10-19)\n".to_string(),
            ),
            (
                "docs/decisions/0003-c.md".to_string(),
                "# C\n\nStatus: maybe\n".to_string(),
            ),
        ];
        let problems = decisions_well_formed(&markdown);
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(
            problems
                .iter()
                .all(|p| p.file == "docs/decisions/0003-c.md")
        );
    }

    #[test]
    fn a_plan_lists_every_living_section_and_sits_in_active_or_completed() {
        let full = PLAN_SECTIONS.join("\n\n");
        let markdown = vec![
            ("docs/plans/active/a.md".to_string(), full.clone()),
            ("docs/plans/b.md".to_string(), full),
            (
                "docs/plans/completed/c.md".to_string(),
                "## Progress\n".to_string(),
            ),
            ("docs/plans/PLANS.md".to_string(), "rules".to_string()),
        ];
        let problems = plans_well_formed(&markdown);
        let files: Vec<&str> = problems.iter().map(|p| p.file.as_str()).collect();
        assert_eq!(
            files,
            [
                "docs/plans/b.md",
                "docs/plans/completed/c.md",
                "docs/plans/completed/c.md",
                "docs/plans/completed/c.md"
            ]
        );
    }

    #[test]
    fn the_map_has_a_line_budget() {
        assert!(map_within_budget(&"x\n".repeat(MAP_LINE_BUDGET)).is_empty());
        assert_eq!(
            map_within_budget(&"x\n".repeat(MAP_LINE_BUDGET + 1)).len(),
            1
        );
    }

    #[test]
    fn every_doc_must_be_in_the_index() {
        let markdown = vec![
            ("docs/ARCHITECTURE.md".to_string(), String::new()),
            ("docs/plans/PLANS.md".to_string(), String::new()),
        ];
        let problems = docs_indexed(&markdown, Some("* `ARCHITECTURE.md`: how it fits"));
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("`plans/PLANS.md`"));
        assert_eq!(docs_indexed(&markdown, None).len(), 1);
    }

    #[test]
    fn a_review_log_is_indexed_by_its_folder() {
        let markdown = vec![("docs/reviews/x.md".to_string(), String::new())];
        let problems = docs_indexed(&markdown, Some("* `reviews/`: the review logs"));
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn a_review_log_is_not_indexed_without_its_folder() {
        let markdown = vec![("docs/reviews/x.md".to_string(), String::new())];
        let problems = docs_indexed(&markdown, Some("* `plans/`: the plans"));
        assert_eq!(problems.len(), 1, "{problems:#?}");
        assert_eq!(problems[0].check, "index");
        assert!(problems[0].message.contains("`reviews/x.md`"));

        let plan = vec![("docs/plans/active/x.md".to_string(), String::new())];
        let problems = docs_indexed(&plan, Some("* `plans/`: the plans"));
        assert_eq!(problems.len(), 1, "only the reviews folder: {problems:#?}");
        assert!(problems[0].message.contains("`plans/active/x.md`"));
    }

    #[test]
    fn a_review_log_names_the_paths_and_ids_of_its_time() {
        assert!(!names_current_paths("docs/reviews/x.md"));
        let ids = defined_ids(&[(
            "docs/tech-debt.md".to_string(),
            "### TD-2: open\n\n## Closed\n\n* TD-1: closed.\n".to_string(),
        )]);
        let problems = id_references("docs/reviews/x.md", "closes TD-1, keeps TD-2", &ids);
        assert!(problems.is_empty(), "{problems:#?}");
    }
}
