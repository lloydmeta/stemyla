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

/// Folders never walked: build output, git, and the gitignored audio, model
/// and page folders that exist on some machines and not on others.
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    "target",
    "third_party",
    "audio",
    "stems",
    "pages",
    "dist",
    "node_modules",
    ".claude/worktrees",
];

/// Paths the docs may name that a build creates, so they are absent in a
/// fresh checkout.
const GENERATED_PREFIXES: &[&str] = &["app/dist", "target/", "third_party/", "audio/", "stems/"];

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
    let top_level: BTreeSet<String> = files
        .iter()
        .filter_map(|f| f.split_once('/').map(|(first, _)| first.to_string()))
        .collect();
    let rust_fns = rust_fn_names(
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
        problems.extend(paths_exist(root, file, text, &top_level));
        problems.extend(ids_defined(file, text, &ids.defined));
    }
    if let Some(invariants) = doc("docs/invariants.md") {
        problems.extend(enforcing_tests_exist(invariants, &rust_fns));
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
            if SKIPPED_DIRS
                .iter()
                .any(|s| rel == *s || rel.ends_with(&format!("/{s}")))
            {
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

/// The folders listed in the root manifest's `members = [...]`.
pub fn workspace_members(cargo_toml: &str) -> Vec<String> {
    let Some(start) = cargo_toml.find("members") else {
        return Vec::new();
    };
    let rest = &cargo_toml[start..];
    let (Some(open), Some(close)) = (rest.find('['), rest.find(']')) else {
        return Vec::new();
    };
    rest[open + 1..close]
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

/// Inline code spans (`like this`) outside fenced code blocks.
pub fn inline_code(markdown: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut fenced = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut parts = line.split('`');
        parts.next();
        while let (Some(code), Some(_)) = (parts.next(), parts.next()) {
            spans.push(code.to_string());
        }
    }
    spans
}

/// The text from a `## Heading` line up to the next heading of that level.
pub fn section<'a>(markdown: &'a str, heading: &str) -> Option<&'a str> {
    let start = markdown
        .match_indices(heading)
        .find(|(i, _)| *i == 0 || markdown.as_bytes()[i - 1] == b'\n')
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
    let prose = path.is_empty()
        || path.starts_with("http")
        || path.starts_with(['~', '/', '-', '@', '.'].as_slice())
            && !path.starts_with(".claude/")
            && !path.starts_with(".github/")
        || path.contains(|c: char| c.is_whitespace() || "<>*?{}=()$:,;'\"|\\".contains(c));
    if prose || GENERATED_PREFIXES.iter().any(|g| path.starts_with(g)) {
        return None;
    }
    let first = path.split('/').next().unwrap_or_default();
    let in_tree = path.contains('/') && top_level.contains(first);
    (path.ends_with(".md") || in_tree).then(|| path.to_string())
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

/// Plans name files they are about to create, and decisions are records of
/// their time, so only their IDs and structure are checked, not their paths.
fn names_current_paths(file: &str) -> bool {
    !(file.starts_with("docs/plans/") || file.starts_with("docs/decisions/"))
}

/// The places a path in `file` may be relative to: the repo root, the file's
/// own folder, and inside `docs/`, the `docs/` folder.
fn path_bases(file: &str) -> Vec<String> {
    let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut bases = vec![String::new(), format!("{dir}/")];
    if file.starts_with("docs/") {
        bases.push("docs/".to_string());
    }
    bases
}

fn paths_exist(root: &Path, file: &str, text: &str, top_level: &BTreeSet<String>) -> Vec<Problem> {
    if !names_current_paths(file) {
        return Vec::new();
    }
    let bases = path_bases(file);
    inline_code(text)
        .iter()
        .filter_map(|span| repo_path(span, top_level))
        .filter(|p| !bases.iter().any(|b| root.join(format!("{b}{p}")).exists()))
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

/// IDs defined by `### PREFIX-N: title` headings, and the problems with them.
#[derive(Debug, Clone, Default)]
pub struct Ids {
    pub defined: BTreeSet<String>,
    pub problems: Vec<Problem>,
}

pub fn defined_ids(files: &[(String, String)]) -> Ids {
    let mut ids = Ids::default();
    for (file, text) in files {
        for line in text.lines() {
            let Some(id) = line
                .strip_prefix("### ")
                .and_then(|h| h.split_once(':'))
                .map(|(id, _)| id)
                .filter(|id| is_id(id))
            else {
                continue;
            };
            if !ids.defined.insert(id.to_string()) {
                ids.problems.push(Problem::new(
                    "ids",
                    file,
                    format!("`{id}` is defined twice.  Fix: give the newer entry the next free number; IDs are never reused."),
                ));
            }
        }
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
/// exists) must be defined, so a reference never points at nothing.
fn ids_defined(file: &str, text: &str, defined: &BTreeSet<String>) -> Vec<Problem> {
    let prefixes: BTreeSet<&str> = defined
        .iter()
        .filter_map(|id| id.split('-').next())
        .collect();
    let tokens: BTreeSet<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|t| is_id(t))
        .filter(|t| prefixes.contains(t.split('-').next().unwrap_or_default()))
        .collect();
    tokens
        .into_iter()
        .filter(|t| !defined.contains(*t))
        .map(|t| {
            Problem::new(
                "ids",
                file,
                format!("`{t}` is not defined in docs/invariants.md or docs/tech-debt.md.  Fix: define it there as `### {t}: ...`, or correct the reference."),
            )
        })
        .collect()
}

/// Names of every `fn` in the given Rust sources.
pub fn rust_fn_names(sources: &[String]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for source in sources {
        for (i, _) in source.match_indices("fn ") {
            let boundary = i == 0 || !source.as_bytes()[i - 1].is_ascii_alphanumeric();
            let name: String = source[i + 3..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if boundary && !name.is_empty() {
                names.insert(name);
            }
        }
    }
    names
}

/// The tests an invariant names on its `* Enforced by:` line exist, so a
/// rule never claims a check that was renamed or deleted.
pub fn enforcing_tests_exist(invariants: &str, rust_fns: &BTreeSet<String>) -> Vec<Problem> {
    invariants
        .lines()
        .filter(|l| l.trim_start().starts_with("* Enforced by:"))
        .flat_map(inline_code)
        .filter(|span| {
            span.contains('_') && span.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
        .filter(|name| !rust_fns.contains(name))
        .map(|name| {
            Problem::new(
                "tests",
                "docs/invariants.md",
                format!("names test `{name}`, but no `fn {name}` exists.  Fix: name the test that now enforces the rule, or move the rule's gap to docs/tech-debt.md."),
            )
        })
        .collect()
}

/// Every doc under `docs/` is listed in `docs/README.md`, the index agents
/// read to find the rest.
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
        .filter(|f| !listed.contains(*f) && !listed.contains(&f["docs/".len()..]))
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

/// The `paths:` globs in a rule's frontmatter.
pub fn rule_globs(rule: &str) -> Option<Vec<String>> {
    let body = rule.strip_prefix("---\n")?;
    let front = &body[..body.find("\n---")?];
    let list = &front[front.find("paths:")? + "paths:".len()..];
    Some(
        list.lines()
            .filter_map(|l| l.trim().strip_prefix("- "))
            .map(|g| g.trim().trim_matches('"').to_string())
            .collect(),
    )
}

/// `*` matches within one path segment and `**` any number of segments.
pub fn glob_matches(glob: &str, path: &str) -> bool {
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
                    && (head.len()..=part.len()).any(|i| segment(tail, &part[i..]))
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
        for glob in globs
            .iter()
            .filter(|g| !files.iter().any(|f| glob_matches(g, f)))
        {
            problems.push(Problem::new(
                "rules",
                file,
                format!("glob `{glob}` matches no file, so the rule never loads.  Fix: point it at the files it is about."),
            ));
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
        assert_eq!(repo_path("src/shaders/", &t), None, "relative to a crate");
        assert_eq!(repo_path("https://example.com/a.md", &t), None);
    }

    #[test]
    fn paths_resolve_from_the_root_the_file_or_docs() {
        assert_eq!(path_bases("CLAUDE.md"), ["", "/"]);
        assert_eq!(
            path_bases("docs/references/a.md"),
            ["", "docs/references/", "docs/"]
        );
        assert!(names_current_paths("docs/tech-debt.md"));
        assert!(!names_current_paths("docs/plans/active/a.md"));
        assert!(!names_current_paths("docs/decisions/0001-a.md"));
    }

    #[test]
    fn a_missing_path_is_reported_and_a_present_one_is_not() {
        let root = std::env::temp_dir().join(format!("docs-check-{}", std::process::id()));
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/invariants.md"), "").unwrap();
        let t = top(&["docs", "engine"]);
        let text = "see `invariants.md`, `docs/invariants.md`, `engine/gone.rs` and `gone.md`";
        let problems = paths_exist(&root, "docs/tech-debt.md", text, &t);
        let named: Vec<&str> = problems
            .iter()
            .map(|p| p.message.split('`').nth(1).unwrap())
            .collect();
        assert_eq!(named, ["engine/gone.rs", "gone.md"]);
        assert!(paths_exist(&root, "docs/plans/active/a.md", text, &t).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
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
        let problems = ids_defined(
            "CLAUDE.md",
            "see MEM-2, MEM-9 and TD-4; not SHA-256 or UTF-8",
            &ids.defined,
        );
        let named: Vec<&str> = problems
            .iter()
            .map(|p| p.message.split('`').nth(1).unwrap())
            .collect();
        assert_eq!(named, ["MEM-9", "TD-4"]);
    }

    #[test]
    fn a_renamed_enforcing_test_is_reported() {
        let fns = rust_fn_names(&[
            "fn a_buffer_is_shared() {}\npub async fn another_one(x: u8) {}\nfn_like_not"
                .to_string(),
        ]);
        assert!(fns.contains("a_buffer_is_shared") && fns.contains("another_one"));
        let invariants = "### MEM-1: x\n\n* Enforced by: `a_buffer_is_shared`, `gone_test`, CI `test` job, `engine/src/song.rs`.\n* Gaps: `not_checked_here`\n";
        let problems = enforcing_tests_exist(invariants, &fns);
        assert_eq!(problems.len(), 1, "{problems:#?}");
        assert!(problems[0].message.contains("`gone_test`"));
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
    }

    #[test]
    fn a_rule_without_paths_or_with_a_dead_glob_is_reported() {
        assert_eq!(
            rule_globs("---\npaths:\n  - \"dsp/**\"\n  - engine/src/*.rs\n---\nbody\n"),
            Some(vec!["dsp/**".to_string(), "engine/src/*.rs".to_string()])
        );
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
        ];
        let files_with_problems: Vec<String> = rules_match_files(&markdown, &files)
            .into_iter()
            .map(|p| p.file)
            .collect();
        assert_eq!(
            files_with_problems,
            [".claude/rules/old.md", ".claude/rules/bare.md"]
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
}
