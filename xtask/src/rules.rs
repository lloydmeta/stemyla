//! Structural checks of the tree.  Each one is a pure function over the
//! files, the docs or Cargo's output, that returns problems, and each
//! problem names its rule (`docs/invariants.md`) and the fix.
//! `xtask/tests/rules.rs` reads the real tree, the docs and Cargo's output,
//! and runs them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use anyhow::{Context, Result};
use proc_macro2::{Delimiter, Literal, Spacing, TokenStream, TokenTree};
use serde::Deserialize;

/// A breach of a rule, found in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub rule: &'static str,
    pub file: String,
    /// `None` for a problem with the whole file.
    pub line: Option<usize>,
    /// The full text: the rule, what is wrong and the fix.
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Every `.rs` file in the tree that `docs::repo_files` walks, with its text.
pub fn rust_files(root: &Path) -> Result<Vec<(String, String)>> {
    crate::docs::repo_files(root)?
        .into_iter()
        .filter(|file| file.ends_with(".rs"))
        .map(|file| {
            let text = std::fs::read_to_string(root.join(&file))
                .with_context(|| format!("reading {file}"))?;
            Ok((file, text))
        })
        .collect()
}

/// The words that start a skip message, in any case.
const SKIP_WORDS: [&str; 2] = ["skipping", "skipped"];

/// Files that hold the skip words on purpose: the helper itself, and this
/// checker's allowlist and planted inputs.
fn holds_skip_words_on_purpose(file: &str) -> bool {
    file.starts_with("test-support/") || file == "xtask/src/rules.rs"
}

/// Skip messages that cannot go through `test_support::skip`: the file, the
/// literal, and why.
const SKIP_ALLOWLIST: &[(&str, &str, &str)] = &[(
    "app/tests/web_output.rs",
    "skipped: the audio context is suspended",
    "a wasm test cannot read STEMYLA_REQUIRE; the test asserts that the browser is not Chrome, and CI runs Chrome",
)];

/// VERIFY-1: every skip message goes through `test_support::skip`, so that
/// `STEMYLA_REQUIRE` can make it a failure.  A problem is a string literal,
/// plain or raw, that starts with `skipping` or `skipped`, in any case.
/// The files are tokenised, so comments, char literals such as `'"'`, raw
/// strings and escapes cannot confuse the check.
pub fn skips_use_the_helper(files: &[(String, String)]) -> Vec<Problem> {
    files
        .iter()
        .filter(|(file, _)| !holds_skip_words_on_purpose(file))
        .flat_map(|(file, text)| skips_in(file, text))
        .collect()
}

/// VERIFY-1: each entry of the skip allowlist still matches a literal in
/// its file, so a stale entry cannot wait there to hide a new skip.
pub fn allowlisted_skips_exist(files: &[(String, String)]) -> Vec<Problem> {
    SKIP_ALLOWLIST
        .iter()
        .filter(|(path, literal, _)| {
            !files.iter().any(|(file, text)| {
                file == path
                    && TokenStream::from_str(text)
                        .is_ok_and(|tokens| string_literals(tokens).iter().any(|(_, value)| value == literal))
            })
        })
        .map(|(path, literal, _)| Problem {
            rule: "VERIFY-1",
            file: path.to_string(),
            line: None,
            message: format!(
                "VERIFY-1: the skip allowlist in xtask/src/rules.rs names \"{literal}\" in {path}, but the file has no such literal.  Fix: remove the entry, or change it to the file's skip message."
            ),
        })
        .collect()
}

fn skips_in(file: &str, text: &str) -> Vec<Problem> {
    match TokenStream::from_str(text) {
        Err(e) => vec![Problem {
            rule: "VERIFY-1",
            file: file.to_string(),
            line: None,
            message: format!(
                "VERIFY-1: {file} does not tokenise: {e}.  Fix: make the file valid Rust, or leave it out of the tree."
            ),
        }],
        Ok(tokens) => string_literals(tokens)
            .into_iter()
            .filter(|(_, value)| {
                !SKIP_ALLOWLIST
                    .iter()
                    .any(|(path, literal, _)| *path == file && literal == value)
            })
            .filter_map(|(line, value)| {
                let word = SKIP_WORDS.into_iter().find_map(|word| {
                    value
                        .get(..word.len())
                        .filter(|start| start.eq_ignore_ascii_case(word))
                })?;
                Some(Problem {
                    rule: "VERIFY-1",
                    file: file.to_string(),
                    line: Some(line),
                    message: format!(
                        "VERIFY-1: {file}:{line} prints \"{word}\" itself.  Fix: call test_support::skip(Need::..., why), so STEMYLA_REQUIRE can make it a failure.  Outside test code, add the literal to the allowlist with a reason."
                    ),
                })
            })
            .collect(),
    }
}

/// The line and the text between the quotes of every string literal, plain
/// or raw, in groups and macro calls too.  Doc comments, which proc-macro2
/// makes into `#[doc = "..."]` literals, are left out.
fn string_literals(tokens: TokenStream) -> Vec<(usize, String)> {
    tokens
        .into_iter()
        .flat_map(|tree| match tree {
            TokenTree::Group(group) => string_literals(group.stream()),
            TokenTree::Literal(literal) => string_value(&literal)
                .map(|value| (literal.span().start().line, value))
                .into_iter()
                .collect(),
            TokenTree::Ident(_) | TokenTree::Punct(_) => Vec::new(),
        })
        .collect()
}

fn string_value(literal: &Literal) -> Option<String> {
    let source = literal.span().source_text()?;
    if source.starts_with("//") || source.starts_with("/*") {
        return None;
    }
    let text = literal.to_string();
    let quoted = text
        .strip_prefix('r')
        .map_or(text.as_str(), |raw| raw.trim_matches('#'));
    quoted
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .map(str::to_string)
}

/// The packages of the web app, one WebAssembly module each.
pub const WEB_APP: [&str; 2] = ["app", "stem-worker"];

/// The wasm tree of one package of the web app, as `cargo tree --prefix
/// depth` prints it: normal edges only, and one package per line.  One run
/// per package, because one run with both merges their features, and Trunk
/// builds them apart.  `--color never`, because CI's `CARGO_TERM_COLOR:
/// always` reaches this process and would colour each `(*)`.
pub fn web_app_tree(root: &Path, package: &str) -> Result<String> {
    let output = Command::new(env!("CARGO"))
        .current_dir(root)
        .args(["tree", "--locked", "--color", "never", "-p", package])
        .args(["-e", "normal", "--target", "wasm32-unknown-unknown"])
        .args(["--prefix", "depth", "-f", "{p}"])
        .output()
        .with_context(|| format!("running cargo tree -p {package}"))?;
    anyhow::ensure!(
        output.status.success(),
        "cargo tree -p {package} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .with_context(|| format!("cargo tree -p {package} printed text that is not UTF-8"))
}

/// LAYER-1: in the normal dependency tree of the web app, `synth` brings no
/// other crate.  `trees` holds `(package, tree)` pairs from `web_app_tree`.
/// A tree that the check cannot read is a problem too, so that it cannot
/// pass without reading anything.
pub fn synth_brings_no_crate(trees: &[(&str, &str)]) -> Vec<Problem> {
    let problems: Vec<Problem> = trees
        .iter()
        .flat_map(|(package, tree)| tree_problems(package, tree))
        .collect();
    let synth_seen = trees.iter().any(|(_, tree)| {
        tree.lines()
            .filter_map(tree_line)
            .any(|line| line.name == "synth")
    });
    let unread = (!synth_seen).then(|| Problem {
        rule: "LAYER-1",
        file: manifest_of("app"),
        line: None,
        message: "LAYER-1: the web app's trees have no synth, so this check read nothing.  Fix: if the web app no longer takes synth, remove synth_brings_no_crate_into_the_web_app.  Update the docs that name it (DOC-1).".into(),
    });
    problems.into_iter().chain(unread).collect()
}

/// One line of `cargo tree --prefix depth`: the depth, then the package.
struct TreeLine<'a> {
    depth: usize,
    name: &'a str,
}

fn tree_line(line: &str) -> Option<TreeLine<'_>> {
    let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
    let depth = line[..line.len() - rest.len()].parse().ok()?;
    let name = rest.split_whitespace().next()?;
    Some(TreeLine { depth, name })
}

fn manifest_of(package: &str) -> String {
    match package {
        "stem-worker" => "worker/Cargo.toml".into(),
        _ => format!("{package}/Cargo.toml"),
    }
}

fn tree_problems(package: &str, tree: &str) -> Vec<Problem> {
    let problem = |message: String| Problem {
        rule: "LAYER-1",
        file: manifest_of(package),
        line: None,
        message,
    };
    let starts_with_package = tree
        .lines()
        .next()
        .and_then(tree_line)
        .is_some_and(|first| first.depth == 0 && first.name == package);
    let bad_start = (!starts_with_package).then(|| {
        problem(format!(
            "LAYER-1: the tree of {package} does not start with {package}, so this check cannot read it.  Fix: run cargo tree -p {package} with --prefix depth."
        ))
    });
    let without_depth = tree
        .lines()
        .filter(|line| !line.trim().is_empty() && tree_line(line).is_none())
        .map(|line| {
            problem(format!(
                "LAYER-1: the tree of {package} has a line without a depth: \"{line}\".  Fix: run cargo tree with --prefix depth."
            ))
        });
    let lines: Vec<TreeLine> = tree.lines().filter_map(tree_line).collect();
    // Each module of the web app takes wasm-bindgen directly, and only on
    // wasm32.  Natively, stem-worker's tree is its root line alone, which
    // would pass every other check.
    let is_wasm_tree = lines
        .iter()
        .any(|line| line.depth == 1 && line.name == "wasm-bindgen");
    let not_wasm = (starts_with_package && !is_wasm_tree).then(|| {
        problem(format!(
            "LAYER-1: the tree of {package} has no wasm-bindgen at depth 1, so it is not the wasm tree.  Fix: run cargo tree -p {package} with --target wasm32-unknown-unknown."
        ))
    });
    let synth_children = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.name == "synth")
        .map(|(at, synth)| {
            lines[at + 1..]
                .iter()
                .take_while(|line| line.depth > synth.depth)
                .filter(|line| line.depth == synth.depth + 1)
                .map(|line| line.name)
                .collect::<Vec<_>>()
        })
        .filter(|children| !children.is_empty())
        .map(|children| {
            problem(format!(
                "LAYER-1: synth brings {} into the web app, through {package}.  Fix: make each dependency of synth optional, behind its files feature.  Take synth without its files feature in the web app.  Tools and tests take features = [\"files\"].",
                children.join(", ")
            ))
        });
    bad_start
        .into_iter()
        .chain(not_wasm)
        .chain(without_depth)
        .chain(synth_children)
        .collect()
}

/// The kind of an edge, as `cargo metadata` gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Normal,
    Dev,
    Build,
}

/// A dependency of one workspace crate on another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub to: String,
    pub kind: Kind,
}

/// Where a stray comes from: a folder, or a `source` such as `git+...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Path(String),
    Source(String),
}

/// A crate from outside the members that can hold or reach workspace
/// code: its name, and where it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stray {
    pub name: String,
    pub origin: Origin,
}

/// A crate outside the members that takes a member: its name, its
/// `source`, and the member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutsideEdge {
    pub from: String,
    pub source: String,
    pub to: String,
}

/// A workspace crate: its package name, its manifest relative to the
/// root, its edges and the strays that it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceCrate {
    pub name: String,
    pub manifest: String,
    pub edges: Vec<Edge>,
    pub strays: Vec<Stray>,
}

/// The workspace crates, the path crates and member names in the build
/// that are not members, and the crates outside the members that take a
/// member.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    pub crates: Vec<WorkspaceCrate>,
    pub outside: Vec<Stray>,
    pub outside_takes: Vec<OutsideEdge>,
}

/// The layers of LAYER-1, lowest first.  The layers table must use these.
pub const LAYERS: [&str; 5] = ["Test support", "Pure", "Engine and GPU", "Shells", "Tools"];

/// Runs `cargo metadata --format-version 1 --all-features --locked` in
/// `root`.  Without `--no-deps`, so that `packages` also holds patched and
/// excluded path crates; with `--all-features`, so that it holds crates
/// behind a feature that is off; `--locked`, so a test never writes
/// `Cargo.lock`.
pub fn workspace_metadata(root: &Path) -> Result<String> {
    let output = Command::new(env!("CARGO"))
        .current_dir(root)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--all-features",
            "--locked",
        ])
        .output()
        .context("running cargo metadata")?;
    anyhow::ensure!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).context("cargo metadata printed text that is not UTF-8")
}

/// The parts of `cargo metadata`'s JSON that the layer checks read.
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
    workspace_members: Vec<String>,
    workspace_root: PathBuf,
    /// `null` with `--no-deps`.
    resolve: Option<Resolve>,
}

#[derive(Deserialize)]
struct MetadataPackage {
    id: String,
    name: String,
    source: Option<String>,
    manifest_path: PathBuf,
    dependencies: Vec<MetadataDependency>,
    /// The features that the package declares, by name.
    features: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct MetadataDependency {
    /// The package name, also when the dependency is renamed.
    name: String,
    source: Option<String>,
    kind: Option<String>,
    /// Present for a path dependency only.
    path: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<ResolveNode>,
}

#[derive(Deserialize)]
struct ResolveNode {
    id: String,
    deps: Vec<ResolveDep>,
    /// The features turned on in this build.
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Deserialize)]
struct ResolveDep {
    /// The ID of the package that the node takes.
    pkg: String,
}

/// What one dependency of a workspace crate is to the layer checks.
enum Found {
    Edge(Edge),
    Stray(Stray),
    Neither,
}

fn kind_of(dependency: &MetadataDependency) -> Result<Kind> {
    match dependency.kind.as_deref() {
        None => Ok(Kind::Normal),
        Some("dev") => Ok(Kind::Dev),
        Some("build") => Ok(Kind::Build),
        Some(other) => anyhow::bail!(
            "cargo metadata gave the dependency kind {other:?} for {}, which this check cannot read",
            dependency.name
        ),
    }
}

fn folder_of(package: &MetadataPackage) -> &Path {
    package.manifest_path.parent().unwrap_or(Path::new(""))
}

/// The workspace, from `cargo metadata`'s JSON.  Edges are found by
/// `path`, compared as whole paths, so a renamed dependency counts under its
/// package name, and a registry crate with a member's name is no edge.
pub fn workspace(metadata: &str) -> Result<Workspace> {
    let metadata: Metadata =
        serde_json::from_str(metadata).context("reading cargo metadata's JSON")?;
    let is_member = |id: &str| metadata.workspace_members.iter().any(|member| member == id);
    let members: Vec<&MetadataPackage> = metadata
        .packages
        .iter()
        .filter(|package| is_member(&package.id))
        .collect();
    let member_names: BTreeSet<&str> = members
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    let resolve = metadata.resolve.as_ref().context(
        "cargo metadata gave no resolve, so it ran with --no-deps; the checks need the whole build",
    )?;
    for package in &members {
        let node = resolve
            .nodes
            .iter()
            .find(|node| node.id == package.id)
            .with_context(|| {
                format!("cargo metadata's resolve has no node for {}", package.name)
            })?;
        let off: Vec<&str> = package
            .features
            .keys()
            .filter(|feature| !node.features.contains(feature))
            .map(String::as_str)
            .collect();
        anyhow::ensure!(
            off.is_empty(),
            "cargo metadata ran without --all-features: {} has {} off, so a dependency behind a feature can hide",
            package.name,
            off.join(", ")
        );
    }
    let crates = members
        .iter()
        .map(|package| workspace_crate(package, &members, &member_names, &metadata.workspace_root))
        .collect::<Result<Vec<_>>>()?;
    let outside = metadata
        .packages
        .iter()
        .filter(|package| !is_member(&package.id))
        .filter_map(|package| match &package.source {
            None => Some(Stray {
                name: package.name.clone(),
                origin: Origin::Path(folder_of(package).display().to_string()),
            }),
            Some(source) if member_names.contains(package.name.as_str()) => Some(Stray {
                name: package.name.clone(),
                origin: Origin::Source(source.clone()),
            }),
            Some(_) => None,
        })
        .collect();
    // A path crate that takes a member is in `outside` already.
    let outside_takes = resolve
        .nodes
        .iter()
        .filter(|node| !is_member(&node.id))
        .filter_map(|node| {
            let package = metadata
                .packages
                .iter()
                .find(|package| package.id == node.id)?;
            let source = package.source.as_ref()?;
            Some(
                node.deps
                    .iter()
                    .filter_map(|dep| members.iter().find(|member| member.id == dep.pkg))
                    .map(|member| OutsideEdge {
                        from: package.name.clone(),
                        source: source.clone(),
                        to: member.name.clone(),
                    }),
            )
        })
        .flatten()
        .collect();
    Ok(Workspace {
        crates,
        outside,
        outside_takes,
    })
}

fn workspace_crate(
    package: &MetadataPackage,
    members: &[&MetadataPackage],
    member_names: &BTreeSet<&str>,
    root: &Path,
) -> Result<WorkspaceCrate> {
    let manifest = package.manifest_path.strip_prefix(root).with_context(|| {
        format!(
            "the manifest {} is outside the workspace root {}",
            package.manifest_path.display(),
            root.display()
        )
    })?;
    let found = package
        .dependencies
        .iter()
        .map(|dependency| {
            let kind = kind_of(dependency)?;
            Ok(match &dependency.path {
                Some(path) => match members.iter().find(|member| folder_of(member) == path) {
                    Some(member) if member.id == package.id => Found::Neither,
                    Some(member) => Found::Edge(Edge {
                        to: member.name.clone(),
                        kind,
                    }),
                    None => Found::Stray(Stray {
                        name: dependency.name.clone(),
                        origin: Origin::Path(path.display().to_string()),
                    }),
                },
                None if member_names.contains(dependency.name.as_str()) => Found::Stray(Stray {
                    name: dependency.name.clone(),
                    origin: Origin::Source(dependency.source.clone().unwrap_or_default()),
                }),
                None => Found::Neither,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(WorkspaceCrate {
        name: package.name.clone(),
        manifest: manifest.to_string_lossy().replace('\\', "/"),
        edges: found
            .iter()
            .filter_map(|found| match found {
                Found::Edge(edge) => Some(edge.clone()),
                _ => None,
            })
            .collect(),
        strays: found
            .into_iter()
            .filter_map(|found| match found {
                Found::Stray(stray) => Some(stray),
                _ => None,
            })
            .collect(),
    })
}

const ARCHITECTURE: &str = "docs/ARCHITECTURE.md";
const TEST_SUPPORT: &str = "test-support";
const PROTOCOL: &str = "stem-protocol";
const DSP: &str = "dsp";
const TABLE_HEADER: [&str; 3] = ["Layer", "Crate", "Depends on"];

fn layer1(file: &str, message: String) -> Problem {
    Problem {
        rule: "LAYER-1",
        file: file.to_string(),
        line: None,
        message,
    }
}

/// One row of the layers table that the check can read.
struct Row<'a> {
    layer: &'a str,
    name: &'a str,
    deps: Vec<&'a str>,
}

enum Table<'a> {
    Missing,
    /// The first line of the header or separator that is wrong.
    BadStart(&'a str),
    /// A header with no line after it.
    NoSeparator,
    /// Each row, or the line of a row that cannot be read.
    Rows(Vec<std::result::Result<Row<'a>, &'a str>>),
}

/// The table that follows `### Layers` before any other heading.
fn layers_table(architecture: &str) -> Table<'_> {
    let lines: Vec<&str> = architecture.lines().collect();
    let Some(heading) = lines
        .iter()
        .position(|line| line.trim_end() == "### Layers")
    else {
        return Table::Missing;
    };
    let after = &lines[heading + 1..];
    let Some(start) = after
        .iter()
        .position(|line| line.starts_with('|') || line.starts_with('#'))
        .filter(|&at| after[at].starts_with('|'))
    else {
        return Table::Missing;
    };
    let table: Vec<&str> = after[start..]
        .iter()
        .take_while(|line| line.starts_with('|'))
        .copied()
        .collect();
    let header = table[0];
    if cells(header).as_deref() != Some(&TABLE_HEADER[..]) {
        return Table::BadStart(header);
    }
    let is_separator = |line: &str| {
        cells(line).is_some_and(|cells| {
            cells.len() == 3
                && cells
                    .iter()
                    .all(|cell| !cell.is_empty() && cell.chars().all(|c| c == '-'))
        })
    };
    match table.get(1) {
        Some(separator) if is_separator(separator) => {}
        Some(separator) => return Table::BadStart(separator),
        None => return Table::NoSeparator,
    }
    Table::Rows(
        table[2..]
            .iter()
            .map(|line| row(line).ok_or(*line))
            .collect(),
    )
}

/// The trimmed cells of a line that starts and ends with `|`.
fn cells(line: &str) -> Option<Vec<&str>> {
    let inner = line.trim_end().strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(str::trim).collect())
}

fn code_span(cell: &str) -> Option<&str> {
    let inner = cell.strip_prefix('`')?.strip_suffix('`')?;
    (!inner.is_empty() && !inner.contains('`')).then_some(inner)
}

fn row(line: &str) -> Option<Row<'_>> {
    let cells = cells(line)?;
    let [layer, name, deps] = cells.as_slice() else {
        return None;
    };
    let deps: Vec<&str> = if *deps == "no workspace crate" {
        Vec::new()
    } else {
        deps.split(", ").map(code_span).collect::<Option<_>>()?
    };
    let once = deps.iter().collect::<BTreeSet<_>>().len() == deps.len();
    (!layer.is_empty() && once).then_some(())?;
    Some(Row {
        layer,
        name: code_span(name)?,
        deps,
    })
}

/// LAYER-1: the edges match the layers table, the table goes only
/// downwards, and no stray hides a dependency.  The table and the edges
/// must match both ways, so the check cannot pass while it reads nothing.
pub fn crates_follow_the_layers(workspace: &Workspace, architecture: &str) -> Vec<Problem> {
    if workspace.crates.is_empty() {
        return vec![layer1(
            "Cargo.toml",
            "LAYER-1: cargo metadata gave no workspace crate, so this check read nothing.  Fix: run cargo metadata --format-version 1 --all-features --locked at the workspace root.".into(),
        )];
    }
    let table = match layers_table(architecture) {
        Table::Missing => vec![layer1(
            ARCHITECTURE,
            "LAYER-1: docs/ARCHITECTURE.md has no table under ### Layers, so this check read nothing.  Fix: restore the table, with the columns Layer, Crate and Depends on.".into(),
        )],
        Table::BadStart(line) => vec![layer1(
            ARCHITECTURE,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md starts with \"{line}\", not \"| Layer | Crate | Depends on |\" and \"| - | - | - |\".  Fix: use those three columns, one row for each crate."
            ),
        )],
        Table::NoSeparator => vec![layer1(
            ARCHITECTURE,
            "LAYER-1: the layers table in docs/ARCHITECTURE.md has no separator line after its header.  Fix: add \"| - | - | - |\" after \"| Layer | Crate | Depends on |\", then one row for each crate.".into(),
        )],
        Table::Rows(rows) => table_problems(&workspace.crates, &rows),
    };
    table
        .into_iter()
        .chain(test_support_problems(&workspace.crates))
        .chain(stray_problems(workspace))
        .collect()
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Normal => "normal",
        Kind::Dev => "dev",
        Kind::Build => "build",
    }
}

fn table_problems(
    crates: &[WorkspaceCrate],
    lines: &[std::result::Result<Row<'_>, &str>],
) -> Vec<Problem> {
    let by_name: BTreeMap<&str, &WorkspaceCrate> = crates
        .iter()
        .map(|krate| (krate.name.as_str(), krate))
        .collect();
    // Borrowed, so the `move` closures below take a reference.
    let members = &by_name;
    let unreadable = lines.iter().filter_map(|line| line.as_ref().err()).map(|line| {
        layer1(
            ARCHITECTURE,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has a row that this check cannot read: \"{line}\".  Fix: give the layer and one crate in a code span.  Then give its dependencies once each, as code spans joined by \", \", or \"no workspace crate\"."
            ),
        )
    });
    let readable: Vec<&Row> = lines.iter().filter_map(|line| line.as_ref().ok()).collect();
    let not_members = readable
        .iter()
        .filter(|row| !members.contains_key(row.name))
        .map(|row| {
            layer1(
                ARCHITECTURE,
                format!(
                    "LAYER-1: the layers table in docs/ARCHITECTURE.md has a row for {}, which is not a workspace crate.  Fix: remove the row, or use the package name from its Cargo.toml.",
                    row.name
                ),
            )
        });
    let member_rows: Vec<&Row> = readable
        .iter()
        .filter(|row| members.contains_key(row.name))
        .copied()
        .collect();
    // The first row of each crate counts; the rest are left out.
    let rows: Vec<&Row> = member_rows
        .iter()
        .enumerate()
        .filter(|(at, row)| {
            !member_rows[..*at]
                .iter()
                .any(|earlier| earlier.name == row.name)
        })
        .map(|(_, row)| *row)
        .collect();
    let repeated = rows.iter().filter_map(|row| {
        let count = member_rows.iter().filter(|other| other.name == row.name).count();
        (count > 1).then(|| {
            layer1(
                ARCHITECTURE,
                format!(
                    "LAYER-1: the layers table in docs/ARCHITECTURE.md has {count} rows for {}.  Fix: keep one row for each crate.",
                    row.name
                ),
            )
        })
    });
    let layers: Vec<&str> = rows
        .iter()
        .enumerate()
        .filter(|(at, row)| !rows[..*at].iter().any(|earlier| earlier.layer == row.layer))
        .map(|(_, row)| row.layer)
        .collect();
    let wrong_layers = (layers != LAYERS).then(|| {
        layer1(
            ARCHITECTURE,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has the layers \"{}\", not those in LAYERS in xtask/src/rules.rs.  Fix: use the five layers in LAYERS: {}.  To change the layers, change the table, LAYERS and LAYER-1 in one commit (DOC-1).",
                layers.join(", "),
                LAYERS.join(", ")
            ),
        )
    });
    let apart = layers
        .iter()
        .filter(|layer| {
            let first = rows.iter().position(|row| row.layer == **layer);
            let last = rows.iter().rposition(|row| row.layer == **layer);
            first
                .zip(last)
                .is_some_and(|(first, last)| rows[first..=last].iter().any(|row| row.layer != **layer))
        })
        .map(|layer| {
            layer1(
                ARCHITECTURE,
                format!(
                    "LAYER-1: the layers table in docs/ARCHITECTURE.md has rows for {layer} that are not together.  Fix: put each layer's rows together, lowest layer first."
                ),
            )
        });
    let row_of = &|name: &str| rows.iter().find(|row| row.name == name).copied();
    let rank_of = &|row: &Row| layers.iter().position(|layer| *layer == row.layer);
    let without_rows = crates
        .iter()
        .filter(|krate| row_of(&krate.name).is_none())
        .map(|krate| {
            layer1(
                ARCHITECTURE,
                format!(
                    "LAYER-1: the workspace crate {} has no row in the layers table in docs/ARCHITECTURE.md.  Fix: add a row with its layer and the workspace crates that it depends on.",
                    krate.name
                ),
            )
        });
    let listed = rows.iter().flat_map(|row| {
        let krate = members[row.name];
        row.deps.iter().flat_map(move |dep| {
            if *dep == TEST_SUPPORT {
                return vec![layer1(
                    ARCHITECTURE,
                    format!(
                        "LAYER-1: the layers table in docs/ARCHITECTURE.md lists test-support for {}.  Fix: remove it.  The table leaves out test-support, which any crate but dsp may take as a dev-dependency.",
                        row.name
                    ),
                )];
            }
            if !members.contains_key(dep) {
                return vec![layer1(
                    ARCHITECTURE,
                    format!(
                        "LAYER-1: the layers table in docs/ARCHITECTURE.md lists {dep} for {name}, but {dep} is not a workspace crate.  Fix: correct the name, or remove it from {name}'s row.",
                        name = row.name
                    ),
                )];
            }
            let Some(dep_row) = row_of(dep) else {
                return Vec::new();
            };
            let stale = (!krate.edges.iter().any(|edge| edge.to == *dep)).then(|| {
                layer1(
                    ARCHITECTURE,
                    format!(
                        "LAYER-1: the layers table in docs/ARCHITECTURE.md lists {dep} for {name}, but {manifest} has no dependency on it.  Fix: remove {dep} from {name}'s row, so the table stays true.",
                        name = row.name,
                        manifest = krate.manifest
                    ),
                )
            });
            let not_lower = (rank_of(dep_row) >= rank_of(row)).then(|| {
                layer1(
                    ARCHITECTURE,
                    format!(
                        "LAYER-1: the layers table in docs/ARCHITECTURE.md lets {} ({}) depend on {dep} ({}), which is not in a lower layer.  Fix: remove the dependency, or move a crate to another layer of the table, so that each crate depends only on lower layers.",
                        row.name, row.layer, dep_row.layer
                    ),
                )
            });
            stale.into_iter().chain(not_lower).collect()
        })
    });
    // test-support's own edges, and edges to it, have rules of their own.
    let unlisted = rows
        .iter()
        .filter(|row| row.name != TEST_SUPPORT)
        .flat_map(|row| {
            let krate = members[row.name];
            let kinds: BTreeMap<&str, BTreeSet<Kind>> = krate
                .edges
                .iter()
                .filter(|edge| edge.to != TEST_SUPPORT && row_of(&edge.to).is_some())
                .fold(BTreeMap::new(), |mut kinds, edge| {
                    kinds.entry(edge.to.as_str()).or_default().insert(edge.kind);
                    kinds
                });
            kinds
                .into_iter()
                .filter(|(to, _)| !row.deps.contains(to))
                .map(|(to, kinds)| {
                    let kinds: Vec<&str> = kinds.into_iter().map(kind_name).collect();
                    layer1(
                        &krate.manifest,
                        format!(
                            "LAYER-1: {name} depends on {to} ({kinds}), but the layers table in docs/ARCHITECTURE.md does not list it.  Fix: remove the dependency from {manifest}.  If the design changes, add {to} to {name}'s row in the same commit.",
                            name = krate.name,
                            kinds = kinds.join(", "),
                            manifest = krate.manifest
                        ),
                    )
                })
                .collect::<Vec<_>>()
        });
    unreadable
        .chain(not_members)
        .chain(repeated)
        .chain(wrong_layers)
        .chain(apart)
        .chain(without_rows)
        .chain(listed)
        .chain(unlisted)
        .collect()
}

/// `test-support` is a dev-dependency only, takes no workspace crate, and
/// `dsp` does not take it.  These rules need no table.
fn test_support_problems(crates: &[WorkspaceCrate]) -> Vec<Problem> {
    crates
        .iter()
        .flat_map(|krate| {
            let kinds: BTreeSet<Kind> = krate
                .edges
                .iter()
                .filter(|edge| edge.to == TEST_SUPPORT)
                .map(|edge| edge.kind)
                .collect();
            let manifest = &krate.manifest;
            match krate.name.as_str() {
                DSP => (!kinds.is_empty())
                    .then(|| {
                        layer1(
                            manifest,
                            format!(
                                "LAYER-1: dsp takes test-support, but dsp must have no dependencies (DSP-1).  Fix: remove it from {manifest}."
                            ),
                        )
                    })
                    .into_iter()
                    .collect::<Vec<_>>(),
                TEST_SUPPORT => krate
                    .edges
                    .iter()
                    .map(|edge| edge.to.as_str())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|to| {
                        layer1(
                            manifest,
                            format!(
                                "LAYER-1: test-support takes {to}, but test-support must take no workspace crate.  The crates that take it must reach nothing through it.  Fix: remove {to} from {manifest}."
                            ),
                        )
                    })
                    .collect(),
                name => {
                    let normal = kinds.contains(&Kind::Normal).then(|| {
                        layer1(
                            manifest,
                            format!(
                                "LAYER-1: {name} takes test-support as a normal dependency, so it would ship.  Fix: move it to [dev-dependencies] in {manifest}."
                            ),
                        )
                    });
                    let build = kinds.contains(&Kind::Build).then(|| {
                        layer1(
                            manifest,
                            format!(
                                "LAYER-1: {name} takes test-support as a build dependency, but it is a dev-dependency only.  Fix: move it to [dev-dependencies] in {manifest}."
                            ),
                        )
                    });
                    normal.into_iter().chain(build).collect()
                }
            }
        })
        .collect()
}

/// A stray hides what it depends on from the check, so each is a problem.
fn stray_problems(workspace: &Workspace) -> Vec<Problem> {
    let of_crates = workspace.crates.iter().flat_map(|krate| {
        krate.strays.iter().map(|stray| {
            let (from, name) = (&krate.name, &stray.name);
            let message = match &stray.origin {
                Origin::Path(path) => format!(
                    "LAYER-1: {from} takes {name} from {path}, which is not a workspace crate, so this check cannot see what {name} depends on.  Fix: make {name} a workspace crate with a row in the layers table, or remove it."
                ),
                Origin::Source(source) => format!(
                    "LAYER-1: {from} takes {name} from {source}, not from the workspace.  Fix: take {name} by path, or remove it."
                ),
            };
            layer1(&krate.manifest, message)
        })
    });
    let outside = workspace.outside.iter().map(|stray| {
        let name = &stray.name;
        let message = match &stray.origin {
            Origin::Path(path) => format!(
                "LAYER-1: the build takes {name} from {path}, a path crate that is not a workspace crate, so this check cannot see what {name} depends on.  Fix: remove the [patch] or exclude that brings it in.  Or make it a workspace crate with a row in the layers table."
            ),
            Origin::Source(source) => format!(
                "LAYER-1: the build takes {name} from {source}, not from the workspace.  Fix: remove the dependency that brings it in."
            ),
        };
        layer1("Cargo.toml", message)
    });
    let takes = workspace.outside_takes.iter().map(|edge| {
        layer1(
            "Cargo.toml",
            format!(
                "LAYER-1: {from}, from {source}, takes the workspace crate {to}, so this check cannot see the dependency.  Fix: remove the [patch] that points {to} at the workspace, or remove {from}.",
                from = edge.from,
                source = edge.source,
                to = edge.to
            ),
        )
    });
    of_crates.chain(outside).chain(takes).collect()
}

/// LAYER-1: the page side and the worker side meet only through
/// `stem-protocol`.  Each side is its shell and every workspace crate that
/// it reaches through normal edges, for any target.
pub fn shells_meet_only_through_the_protocol(crates: &[WorkspaceCrate]) -> Vec<Problem> {
    let missing: Vec<Problem> = WEB_APP
        .iter()
        .filter(|shell| !crates.iter().any(|krate| krate.name == **shell))
        .map(|shell| {
            layer1(
                "Cargo.toml",
                format!(
                    "LAYER-1: the workspace has no crate {shell}, so this check cannot compare the page side and the worker side.  Fix: if the web app changed, change WEB_APP in xtask/src/rules.rs and LAYER-1's text (DOC-1)."
                ),
            )
        })
        .collect();
    if !missing.is_empty() {
        return missing;
    }
    let [page, worker] = WEB_APP.map(|shell| reached(crates, shell));
    let shared: Vec<&str> = page
        .intersection(&worker)
        .filter(|name| **name != PROTOCOL)
        .copied()
        .collect();
    if shared.is_empty() {
        return Vec::new();
    }
    vec![layer1(
        "Cargo.toml",
        format!(
            "LAYER-1: {} and {} both depend on {}, directly or through other workspace crates.  The page side and the worker side meet only through stem-protocol.  Fix: keep each crate on one side, and send what the other side needs as a message in stem-protocol.",
            WEB_APP[0],
            WEB_APP[1],
            shared.join(", ")
        ),
    )]
}

/// `shell` and every workspace crate that it reaches through normal edges.
fn reached<'a>(crates: &'a [WorkspaceCrate], shell: &'a str) -> BTreeSet<&'a str> {
    let mut seen = BTreeSet::from([shell]);
    let mut frontier = vec![shell];
    while let Some(name) = frontier.pop() {
        let next = crates
            .iter()
            .filter(|krate| krate.name == name)
            .flat_map(|krate| &krate.edges)
            .filter(|edge| edge.kind == Kind::Normal)
            .map(|edge| edge.to.as_str());
        for to in next {
            if seen.insert(to) {
                frontier.push(to);
            }
        }
    }
    seen
}

/// The lints that DSP-1's forms trip, each relaxed by one `#![expect]` in
/// `dsp/src/lib.rs` with a reason that starts with `DSP-1`.
pub const DSP1_LINTS: [&str; 4] = [
    "assign_op_pattern",
    "len_zero",
    "manual_clamp",
    "manual_swap",
];

/// The text of `dsp/Cargo.toml`.  Other text, such as a dependency or a
/// `[lints]` override, can break DSP-1.
const DSP_MANIFEST: &str = "[package]
name = \"dsp\"
version = \"0.1.0\"
edition.workspace = true

[dependencies]

[lints]
workspace = true
";

/// Every file under `dsp/`, as a path from the root and its text, except
/// the `.DS_Store` files that Finder makes.
pub fn dsp_files(root: &Path) -> Result<Vec<(String, String)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if path.file_name().is_none_or(|name| name != ".DS_Store") {
                let rel = path
                    .strip_prefix(root)
                    .expect("walked from root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(&path).with_context(|| format!("reading {rel}"))?;
                out.push((rel, String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, &root.join(DSP), &mut out)?;
    out.sort();
    Ok(out)
}

/// DSP-1: `dsp` keeps its beginner style.  A scan of the tokens finds the
/// likely breaches: methods other than `len`, `push` and `capacity`,
/// closures, compound assignments, `for` over a list, lint attributes, and
/// files or manifest text that can relax the lints.  TD-3 names what the
/// scan cannot see.
pub fn dsp_keeps_beginner_style(files: &[(String, String)]) -> Vec<Problem> {
    files
        .iter()
        .flat_map(|(file, text)| dsp_file_problems(file, text))
        .collect()
}

/// The only methods that `dsp` calls.
const DSP_METHODS: [&str; 3] = ["len", "push", "capacity"];

/// Names that a tidy-up reaches through `::`, as in `std::mem::swap`.
const DSP_PATH_NAMES: [&str; 6] = ["iter", "into_iter", "swap", "min", "max", "clamp"];

/// Attributes that can relax a lint or bring in other code.
const DSP_ATTRIBUTES: [&str; 7] = [
    "allow", "warn", "deny", "forbid", "expect", "cfg_attr", "path",
];

const DSP_LIB: &str = "dsp/src/lib.rs";

const DSP_STYLE_FIX: &str = "write the loop with while and indexing, len() > 0, an if for each bound, or a swap through a temporary";

/// The kinds of breach that the scan finds, one problem for each kind on a
/// line at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DspBreach {
    Method,
    Path,
    Closure,
    CompoundAssignment,
    ForOverAList,
    Attribute,
    Include,
}

fn dsp_file_problems(file: &str, text: &str) -> Vec<Problem> {
    let whole_file = |message: String| {
        vec![Problem {
            rule: "DSP-1",
            file: file.to_string(),
            line: None,
            message,
        }]
    };
    if file == "dsp/Cargo.toml" {
        if text.replace("\r\n", "\n") == DSP_MANIFEST {
            return Vec::new();
        }
        return whole_file(format!(
            "DSP-1: {file} differs from DSP_MANIFEST in xtask/src/rules.rs, so it can add a crate or relax a lint.  Fix: restore the text."
        ));
    }
    if !(file.starts_with("dsp/src/") && file.ends_with(".rs")) {
        return whole_file(format!(
            "DSP-1: {file} is not dsp/Cargo.toml or a .rs file in dsp/src/, so it can change how dsp builds or is linted.  Fix: remove it."
        ));
    }
    let tokens = match TokenStream::from_str(text) {
        Ok(tokens) => tokens,
        Err(e) => {
            return whole_file(format!(
                "DSP-1: {file} does not tokenise: {e}.  Fix: make the file valid Rust."
            ));
        }
    };
    let mut breaches = BTreeMap::new();
    scan_dsp_tokens(file, tokens, DspLevel::Top, &mut breaches);
    breaches
        .into_iter()
        .map(|((line, _), what)| Problem {
            rule: "DSP-1",
            file: file.to_string(),
            line: Some(line),
            message: format!("DSP-1: {file}:{line} {what}"),
        })
        .collect()
}

/// Where a stream of tokens is in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DspLevel {
    Top,
    Group,
    /// The braces of an import, as in `use std::mem::{replace, swap}`.
    ImportList,
}

/// Finds the breaches in `tokens` and in every group in them.  For each
/// line and kind, it keeps the first breach and its message.
fn scan_dsp_tokens(
    file: &str,
    tokens: TokenStream,
    level: DspLevel,
    breaches: &mut BTreeMap<(usize, DspBreach), String>,
) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for (k, tree) in trees.iter().enumerate() {
        if let TokenTree::Group(group) = tree {
            let after_path = k >= 2
                && punct_spacing(trees.get(k - 1), ':').is_some()
                && joint(trees.get(k - 2), ':');
            let inner = if group.delimiter() == Delimiter::Brace && after_path {
                DspLevel::ImportList
            } else {
                DspLevel::Group
            };
            scan_dsp_tokens(file, group.stream(), inner, breaches);
        }
        if let Some((kind, what)) = dsp_breach_at(file, &trees, k, level) {
            breaches
                .entry((tree.span().start().line, kind))
                .or_insert(what);
        }
    }
}

fn punct_spacing(tree: Option<&TokenTree>, char: char) -> Option<Spacing> {
    match tree {
        Some(TokenTree::Punct(punct)) if punct.as_char() == char => Some(punct.spacing()),
        _ => None,
    }
}

fn joint(tree: Option<&TokenTree>, char: char) -> bool {
    punct_spacing(tree, char) == Some(Spacing::Joint)
}

fn ident_name(tree: Option<&TokenTree>) -> Option<String> {
    match tree {
        Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
        _ => None,
    }
}

/// The breach that starts at `trees[k]`, if any, and its message.
fn dsp_breach_at(
    file: &str,
    trees: &[TokenTree],
    k: usize,
    level: DspLevel,
) -> Option<(DspBreach, String)> {
    let at = |offset: isize| k.checked_add_signed(offset).and_then(|j| trees.get(j));
    let tree = &trees[k];
    if let Some(raw) = ident_name(Some(tree)) {
        let name = raw.strip_prefix("r#").unwrap_or(&raw);
        let after_dot = punct_spacing(at(-1), '.').is_some() && !joint(at(-2), '.');
        let first_in_import =
            level == DspLevel::ImportList && (k == 0 || punct_spacing(at(-1), ',').is_some());
        let after_path =
            first_in_import || (punct_spacing(at(-1), ':').is_some() && joint(at(-2), ':'));
        if after_dot && !DSP_METHODS.contains(&name) {
            return Some((
                DspBreach::Method,
                format!(
                    "calls the method `{name}`; dsp calls only len, push and capacity.  Fix: {DSP_STYLE_FIX}."
                ),
            ));
        }
        if after_path && DSP_PATH_NAMES.contains(&name) {
            return Some((
                DspBreach::Path,
                format!("calls `{name}` through `::`.  Fix: {DSP_STYLE_FIX}."),
            ));
        }
        if raw == "move" {
            return Some((
                DspBreach::Closure,
                "has a `move` closure.  Fix: write the loop out with while, or with for over a range.".into(),
            ));
        }
        if raw == "for" {
            return dsp_for_breach(&trees[k + 1..]);
        }
        if name == "include" && punct_spacing(at(1), '!').is_some() {
            return Some((
                DspBreach::Include,
                "brings in another file with include!.  Fix: put the code in a module in dsp/src/."
                    .into(),
            ));
        }
        return None;
    }
    if let Some(spacing) = punct_spacing(Some(tree), '|') {
        let first_of_pair = spacing == Spacing::Joint
            && (punct_spacing(at(1), '|').is_some() || punct_spacing(at(1), '=').is_some());
        if !first_of_pair && !joint(at(-1), '|') {
            return Some((
                DspBreach::Closure,
                "has a closure, a lone `|`.  Fix: write the loop out with while, or with for over a range.".into(),
            ));
        }
    }
    if punct_spacing(Some(tree), '=').is_some() {
        let operator = ['+', '-', '*', '/', '%', '^', '&', '|']
            .into_iter()
            .find(|&op| joint(at(-1), op))
            .map(String::from)
            .or_else(|| {
                ['<', '>']
                    .into_iter()
                    .find(|&op| joint(at(-1), op) && joint(at(-2), op))
                    .map(|op| format!("{op}{op}"))
            });
        if let Some(op) = operator {
            return Some((
                DspBreach::CompoundAssignment,
                format!("uses the compound assignment `{op}=`.  Fix: write x = x {op} y."),
            ));
        }
    }
    if punct_spacing(Some(tree), '#').is_some() {
        let inner = punct_spacing(at(1), '!').is_some();
        let body = if inner { at(2) } else { at(1) };
        if let Some(TokenTree::Group(group)) = body
            && group.delimiter() == Delimiter::Bracket
        {
            let raw = ident_name(group.stream().into_iter().next().as_ref())?;
            let name = raw.strip_prefix("r#").unwrap_or(&raw);
            let listed = inner
                && level == DspLevel::Top
                && file == DSP_LIB
                && is_dsp1_expect(group.stream());
            if DSP_ATTRIBUTES.contains(&name) && !listed {
                return Some((
                    DspBreach::Attribute,
                    format!(
                        "has the attribute `{name}`, which can relax a lint or bring in other code.  Fix: remove it.  To keep a form that DSP-1 needs, add #![expect(clippy::<lint>, reason = \"DSP-1: ...\")] to {DSP_LIB}, and the lint to DSP1_LINTS."
                    ),
                ));
            }
        }
    }
    None
}

/// A `for` breach unless the loop's expression, before its `{` block, has
/// `..` outside any group.  A `for` with no `in`, as in `impl A for B`, is
/// not a loop.
fn dsp_for_breach(rest: &[TokenTree]) -> Option<(DspBreach, String)> {
    let header: Vec<&TokenTree> = rest
        .iter()
        .take_while(|tree| !matches!(tree, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace))
        .collect();
    let is_loop = header
        .iter()
        .any(|tree| ident_name(Some(tree)).as_deref() == Some("in"));
    let over_a_range = header
        .windows(2)
        .any(|pair| joint(Some(pair[0]), '.') && punct_spacing(Some(pair[1]), '.').is_some());
    (is_loop && !over_a_range).then(|| {
        (
            DspBreach::ForOverAList,
            "has a `for` loop that is not over a range.  Fix: loop over a range of indices, as in for i in 0..samples.len().".into(),
        )
    })
}

/// `expect(clippy::<lint>, reason = "DSP-1...")`, with one lint from
/// DSP1_LINTS.
fn is_dsp1_expect(attribute: TokenStream) -> bool {
    let trees: Vec<TokenTree> = attribute.into_iter().collect();
    let [TokenTree::Ident(expect), TokenTree::Group(arguments)] = trees.as_slice() else {
        return false;
    };
    let arguments: Vec<TokenTree> = arguments.stream().into_iter().collect();
    let [
        clippy,
        colon,
        second_colon,
        lint,
        comma,
        reason,
        equals,
        TokenTree::Literal(text),
        rest @ ..,
    ] = arguments.as_slice()
    else {
        return false;
    };
    *expect == "expect"
        && ident_name(Some(clippy)).as_deref() == Some("clippy")
        && joint(Some(colon), ':')
        && punct_spacing(Some(second_colon), ':').is_some()
        && ident_name(Some(lint)).is_some_and(|lint| DSP1_LINTS.contains(&lint.as_str()))
        && punct_spacing(Some(comma), ',').is_some()
        && ident_name(Some(reason)).as_deref() == Some("reason")
        && punct_spacing(Some(equals), '=').is_some()
        && string_value(text).is_some_and(|reason| reason.starts_with("DSP-1"))
        && rest
            .iter()
            .all(|tree| punct_spacing(Some(tree), ',').is_some())
        && rest.len() <= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_problem(file: &str, package: &str, children: &str) -> Problem {
        Problem {
            rule: "LAYER-1",
            file: file.into(),
            line: None,
            message: format!(
                "LAYER-1: synth brings {children} into the web app, through {package}.  Fix: make each dependency of synth optional, behind its files feature.  Take synth without its files feature in the web app.  Tools and tests take features = [\"files\"]."
            ),
        }
    }

    const NO_SYNTH: &str = "LAYER-1: the web app's trees have no synth, so this check read nothing.  Fix: if the web app no longer takes synth, remove synth_brings_no_crate_into_the_web_app.  Update the docs that name it (DOC-1).";

    #[test]
    fn synth_with_children_is_a_problem() {
        let tree = "\
0app v0.1.0 (/x/app)
1anyhow v1.0.104
1synth v0.1.0 (/x/synth)
2anyhow v1.0.104
2hound v3.5.1
2serde v1.0.229
3serde_core v1.0.229
3serde_derive v1.0.229 (proc-macro)
4proc-macro2 v1.0.107 (*)
4quote v1.0.47 (*)
4syn v3.0.7 (*)
2serde_json v1.0.152
3itoa v1.0.18
3memchr v2.8.3
3serde_core v1.0.229
3zmij v1.0.23
1wasm-bindgen v0.2.129 (*)
1web-sys v0.3.106 (*)
";
        let problems = synth_brings_no_crate(&[("app", tree)]);
        assert_eq!(
            problems,
            [Problem {
                rule: "LAYER-1",
                file: "app/Cargo.toml".into(),
                line: None,
                message: "LAYER-1: synth brings anyhow, hound, serde, serde_json into the web app, through app.  Fix: make each dependency of synth optional, behind its files feature.  Take synth without its files feature in the web app.  Tools and tests take features = [\"files\"].".into(),
            }]
        );
    }

    #[test]
    fn synth_without_children_is_no_problem() {
        let app = "0app v0.1.0 (/x/app)\n1wasm-bindgen v0.2.129\n1synth v0.1.0 (/x/synth)\n1web-sys v0.3.106\n";
        let worker = "0stem-worker v0.1.0 (/x/worker)\n1wasm-bindgen v0.2.129 (*)\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", app), ("stem-worker", worker)]),
            []
        );
    }

    #[test]
    fn a_tree_without_wasm_bindgen_is_a_problem() {
        let app = "0app v0.1.0 (/x/app)\n1wasm-bindgen v0.2.129\n1synth v0.1.0 (/x/synth)\n";
        let worker = "0stem-worker v0.1.0 (/x/worker)\n2wasm-bindgen v0.2.129\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", app), ("stem-worker", worker)]),
            [Problem {
                rule: "LAYER-1",
                file: "worker/Cargo.toml".into(),
                line: None,
                message: "LAYER-1: the tree of stem-worker has no wasm-bindgen at depth 1, so it is not the wasm tree.  Fix: run cargo tree -p stem-worker with --target wasm32-unknown-unknown.".into(),
            }]
        );
    }

    #[test]
    fn a_line_at_synths_depth_is_not_its_child() {
        let tree =
            "0app v0.1.0\n1wasm-bindgen v0.2.129\n1synth v0.1.0\n1engine v0.1.0\n2hound v3.5.1\n";
        assert_eq!(synth_brings_no_crate(&[("app", tree)]), []);
    }

    #[test]
    fn a_shallower_line_ends_synths_children() {
        let tree = "0app v0.1.0\n1wasm-bindgen v0.2.129\n1engine v0.1.0\n2synth v0.1.0\n1dsp v0.1.0\n3hound v3.5.1\n";
        assert_eq!(synth_brings_no_crate(&[("app", tree)]), []);
    }

    #[test]
    fn a_deeper_synth_is_read() {
        let tree =
            "0app v0.1.0\n1wasm-bindgen v0.2.129\n9engine v0.1.0\n10synth v0.1.0\n11hound v3.5.1\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", tree)]),
            [synth_problem("app/Cargo.toml", "app", "hound")]
        );
    }

    #[test]
    fn a_repeated_synth_gives_one_problem() {
        let tree = "\
0app v0.1.0
1wasm-bindgen v0.2.129
1synth v0.1.0
2hound v3.5.1
1engine v0.1.0
2synth v0.1.0 (*)
";
        assert_eq!(
            synth_brings_no_crate(&[("app", tree)]),
            [synth_problem("app/Cargo.toml", "app", "hound")]
        );
    }

    #[test]
    fn a_tree_without_synth_is_a_problem() {
        let app = "0app v0.1.0\n1wasm-bindgen v0.2.129\n1engine v0.1.0\n";
        let worker = "0stem-worker v0.1.0\n1wasm-bindgen v0.2.129\n1synthx v1.0.0\n2hound v3.5.1\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", app), ("stem-worker", worker)]),
            [Problem {
                rule: "LAYER-1",
                file: "app/Cargo.toml".into(),
                line: None,
                message: NO_SYNTH.into(),
            }]
        );
    }

    #[test]
    fn synth_under_stem_worker_is_named() {
        let app = "0app v0.1.0 (/x/app)\n1wasm-bindgen v0.2.129\n1synth v0.1.0 (/x/synth)\n1web-sys v0.3.106\n";
        let worker = "\
0stem-worker v0.1.0 (/x/worker)
1wasm-bindgen v0.2.129 (*)
1synth v0.1.0 (/x/synth)
2anyhow v1.0.104
2hound v3.5.1
2serde v1.0.229 (*)
2serde_json v1.0.152 (*)
";
        assert_eq!(
            synth_brings_no_crate(&[("app", app), ("stem-worker", worker)]),
            [synth_problem(
                "worker/Cargo.toml",
                "stem-worker",
                "anyhow, hound, serde, serde_json"
            )]
        );
    }

    #[test]
    fn a_line_without_a_depth_is_a_problem() {
        let tree =
            "0stem-worker v0.1.0\n1wasm-bindgen v0.2.129\n1synth v0.1.0\n\n  \nsynth v0.1.0\n";
        assert_eq!(
            synth_brings_no_crate(&[("stem-worker", tree)]),
            [Problem {
                rule: "LAYER-1",
                file: "worker/Cargo.toml".into(),
                line: None,
                message: "LAYER-1: the tree of stem-worker has a line without a depth: \"synth v0.1.0\".  Fix: run cargo tree with --prefix depth.".into(),
            }]
        );
    }

    #[test]
    fn a_first_line_below_depth_0_is_a_problem() {
        let tree = "1app v0.1.0\n0app v0.1.0\n1wasm-bindgen v0.2.129\n1synth v0.1.0\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", tree)]),
            [Problem {
                rule: "LAYER-1",
                file: "app/Cargo.toml".into(),
                line: None,
                message: "LAYER-1: the tree of app does not start with app, so this check cannot read it.  Fix: run cargo tree -p app with --prefix depth.".into(),
            }]
        );
    }

    #[test]
    fn a_tree_that_does_not_start_with_its_package_is_a_problem() {
        let worker = "0app v0.1.0\n1synth v0.1.0\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", ""), ("stem-worker", worker)]),
            [
                Problem {
                    rule: "LAYER-1",
                    file: "app/Cargo.toml".into(),
                    line: None,
                    message: "LAYER-1: the tree of app does not start with app, so this check cannot read it.  Fix: run cargo tree -p app with --prefix depth.".into(),
                },
                Problem {
                    rule: "LAYER-1",
                    file: "worker/Cargo.toml".into(),
                    line: None,
                    message: "LAYER-1: the tree of stem-worker does not start with stem-worker, so this check cannot read it.  Fix: run cargo tree -p stem-worker with --prefix depth.".into(),
                },
            ]
        );
    }

    fn check(file: &str, text: &str) -> Vec<Problem> {
        skips_use_the_helper(&[(file.to_string(), text.to_string())])
    }

    fn lines(problems: &[Problem]) -> Vec<Option<usize>> {
        problems.iter().map(|p| p.line).collect()
    }

    #[test]
    fn a_skip_message_in_a_test_is_a_problem() {
        let problems = check(
            "roformer/tests/x.rs",
            "fn gpu() {\n    eprintln!(\"skipping: no adapter\");\n}\n",
        );
        assert_eq!(
            problems,
            [Problem {
                rule: "VERIFY-1",
                file: "roformer/tests/x.rs".into(),
                line: Some(2),
                message: "VERIFY-1: roformer/tests/x.rs:2 prints \"skipping\" itself.  Fix: call test_support::skip(Need::..., why), so STEMYLA_REQUIRE can make it a failure.  Outside test code, add the literal to the allowlist with a reason.".into(),
            }]
        );
        assert_eq!(problems[0].to_string(), problems[0].message);
    }

    #[test]
    fn a_skipped_literal_in_src_is_a_problem() {
        let problems = check("engine/src/a.rs", "const A: &str = \"skipped: x\";\n");
        assert_eq!(lines(&problems), [Some(1)]);
        assert!(
            problems[0]
                .message
                .starts_with("VERIFY-1: engine/src/a.rs:1 prints \"skipped\" itself."),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_skip_word_in_any_case_is_a_problem() {
        let text = "fn f() {\n    eprintln!(\"Skipping: x\");\n    eprintln!(\"SKIPPED: x\");\n}\n";
        let problems = check("a/tests/b.rs", text);
        assert_eq!(lines(&problems), [Some(2), Some(3)]);
        assert!(
            problems[0].message.contains("prints \"Skipping\" itself."),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_literal_on_the_line_after_a_macro_call_has_that_line() {
        let text = "fn f() {\n    eprintln!(\n        \"skipping: {}\",\n        1\n    );\n}\n";
        assert_eq!(lines(&check("a/tests/b.rs", text)), [Some(3)]);
    }

    #[test]
    fn a_skip_literal_after_a_quote_char_is_found() {
        let text = r#"fn f() { let q = '"'; eprintln!("skipping: x"); }"#;
        assert_eq!(lines(&check("a/tests/b.rs", text)), [Some(1)]);
    }

    #[test]
    fn a_raw_skip_literal_is_found() {
        let text = "fn f() {\n    let a = r\"skipping\";\n    let b = r##\"skipped \"x\"\"##;\n}\n";
        assert_eq!(lines(&check("a/tests/b.rs", text)), [Some(2), Some(3)]);
    }

    #[test]
    fn literals_that_only_look_like_skips_are_not_problems() {
        let text = r##"fn f() {
    let a = r#"a "b"#;
    let b = "\\";
    let c = "selftest: skipping the full chunk";
    let d = "\"skipping\"";
    let e = b"skipping";
}
"##;
        assert_eq!(check("a/tests/b.rs", text), []);
    }

    /// Each doc comment has no space after its marker, so it is a problem
    /// unless doc comments are left out.
    #[test]
    fn skip_words_in_comments_are_not_problems() {
        let text = "//!skipping x\n// skipping\n///skipping: x\n/**skipped */\nfn f() {\n    /*skipped*/\n}\n";
        assert_eq!(check("a/tests/b.rs", text), []);
    }

    #[test]
    fn the_allowlisted_skip_is_not_a_problem_in_its_own_file_only() {
        let text = "fn f() { console_log!(\"skipped: the audio context is suspended\"); }";
        assert_eq!(check("app/tests/web_output.rs", text), []);
        assert_eq!(lines(&check("app/tests/web.rs", text)), [Some(1)]);
    }

    #[test]
    fn an_allowlist_entry_whose_literal_is_gone_is_a_problem() {
        let (file, literal, _) = SKIP_ALLOWLIST[0];
        let current = format!("fn f() {{ console_log!(\"{literal}\"); }}");
        assert_eq!(allowlisted_skips_exist(&[(file.to_string(), current)]), []);
        let reworded = "fn f() { console_log!(\"skipped: suspended\"); }".to_string();
        let problems = allowlisted_skips_exist(&[(file.to_string(), reworded)]);
        assert_eq!(
            problems,
            [Problem {
                rule: "VERIFY-1",
                file: file.into(),
                line: None,
                message: format!(
                    "VERIFY-1: the skip allowlist in xtask/src/rules.rs names \"{literal}\" in {file}, but the file has no such literal.  Fix: remove the entry, or change it to the file's skip message."
                ),
            }]
        );
        assert_eq!(
            lines(&allowlisted_skips_exist(&[])),
            [None],
            "a file that is not in the tree"
        );
    }

    #[test]
    fn each_allowlist_entry_has_a_reason() {
        for (file, literal, reason) in SKIP_ALLOWLIST {
            assert!(!reason.trim().is_empty(), "{file}: {literal}");
        }
    }

    #[test]
    fn the_helper_and_this_checker_are_left_out() {
        let text = "fn f() { eprintln!(\"skipping: x\"); }";
        assert_eq!(check("test-support/src/lib.rs", text), []);
        assert_eq!(check("test-support/tests/skip.rs", text), []);
        assert_eq!(check("xtask/src/rules.rs", text), []);
        assert_eq!(lines(&check("xtask/src/docs.rs", text)), [Some(1)]);
    }

    #[test]
    fn a_file_that_does_not_tokenise_is_a_problem() {
        let problems = check("a/src/b.rs", "fn f() { let s = \"unterminated; }\n");
        assert_eq!(lines(&problems), [None]);
        assert!(
            problems[0]
                .message
                .starts_with("VERIFY-1: a/src/b.rs does not tokenise: "),
            "{}",
            problems[0]
        );
        assert!(
            problems[0]
                .message
                .ends_with(".  Fix: make the file valid Rust, or leave it out of the tree."),
            "{}",
            problems[0]
        );
    }

    // LAYER-1, crate layering (M5).

    use serde_json::{Value, json};

    const ARCH: &str = "docs/ARCHITECTURE.md";

    const LAYERS_TABLE: &str = "\
| Layer | Crate | Depends on |
| - | - | - |
| Test support | `test-support` | no workspace crate |
| Pure | `dsp` | no workspace crate |
| Pure | `synth` | no workspace crate |
| Pure | `roformer` | no workspace crate |
| Pure | `stem-protocol` | no workspace crate |
| Engine and GPU | `engine` | `dsp` |
| Engine and GPU | `roformer-gpu` | `roformer` |
| Shells | `app` | `dsp`, `engine`, `stem-protocol`, `synth` |
| Shells | `stem-worker` | `roformer`, `roformer-gpu`, `stem-protocol` |
| Tools | `xtask` | `dsp`, `engine`, `roformer`, `synth` |
";

    const TEST_SUPPORT_ROW: &str = "| Test support | `test-support` | no workspace crate |\n";
    const DSP_ROW: &str = "| Pure | `dsp` | no workspace crate |\n";
    const SYNTH_ROW: &str = "| Pure | `synth` | no workspace crate |\n";
    const ROFORMER_ROW: &str = "| Pure | `roformer` | no workspace crate |\n";
    const PROTOCOL_ROW: &str = "| Pure | `stem-protocol` | no workspace crate |\n";
    const ENGINE_ROW: &str = "| Engine and GPU | `engine` | `dsp` |\n";
    const GPU_ROW: &str = "| Engine and GPU | `roformer-gpu` | `roformer` |\n";
    const APP_ROW: &str = "| Shells | `app` | `dsp`, `engine`, `stem-protocol`, `synth` |\n";
    const XTASK_ROW: &str = "| Tools | `xtask` | `dsp`, `engine`, `roformer`, `synth` |\n";

    /// A doc shaped like `docs/ARCHITECTURE.md`, with `table` under
    /// `### Layers` and a later table that the check must not read.
    fn architecture(table: &str) -> String {
        format!(
            "# How it works\n\n## Crates\n\n### Layers\n\nCrates depend only downwards.\n\n{table}\nAny crate but `dsp` may take `test-support`.\n\n## The model\n\n| Part | Parameters |\n| - | - |\n| band split | 1 M |\n"
        )
    }

    fn edit(table: &str, from: &str, to: &str) -> String {
        assert!(table.contains(from), "the fixture has no {from:?}");
        table.replacen(from, to, 1)
    }

    fn krate(name: &str, manifest: &str, edges: &[(&str, Kind)]) -> WorkspaceCrate {
        WorkspaceCrate {
            name: name.into(),
            manifest: manifest.into(),
            edges: edges
                .iter()
                .map(|(to, kind)| Edge {
                    to: (*to).into(),
                    kind: *kind,
                })
                .collect(),
            strays: Vec::new(),
        }
    }

    /// The workspace crates and edges of 2026-10-20.
    fn today() -> Workspace {
        use Kind::{Dev, Normal};
        Workspace {
            crates: vec![
                krate("dsp", "dsp/Cargo.toml", &[]),
                krate("engine", "engine/Cargo.toml", &[("dsp", Normal)]),
                krate(
                    "app",
                    "app/Cargo.toml",
                    &[
                        ("dsp", Normal),
                        ("engine", Normal),
                        ("synth", Normal),
                        ("stem-protocol", Normal),
                        ("synth", Dev),
                    ],
                ),
                krate("synth", "synth/Cargo.toml", &[]),
                krate("stem-protocol", "protocol/Cargo.toml", &[]),
                krate(
                    "xtask",
                    "xtask/Cargo.toml",
                    &[
                        ("dsp", Normal),
                        ("engine", Normal),
                        ("synth", Normal),
                        ("roformer", Normal),
                    ],
                ),
                krate("roformer", "roformer/Cargo.toml", &[("test-support", Dev)]),
                krate("test-support", "test-support/Cargo.toml", &[]),
                krate(
                    "roformer-gpu",
                    "roformer-gpu/Cargo.toml",
                    &[("roformer", Normal), ("test-support", Dev)],
                ),
                krate(
                    "stem-worker",
                    "worker/Cargo.toml",
                    &[
                        ("roformer", Normal),
                        ("roformer-gpu", Normal),
                        ("stem-protocol", Normal),
                    ],
                ),
            ],
            ..Workspace::default()
        }
    }

    fn with_edges(workspace: Workspace, added: &[(&str, &str, Kind)]) -> Workspace {
        Workspace {
            crates: workspace
                .crates
                .into_iter()
                .map(|c| {
                    let edges = c
                        .edges
                        .iter()
                        .cloned()
                        .chain(added.iter().filter(|(from, _, _)| *from == c.name).map(
                            |(_, to, kind)| Edge {
                                to: (*to).into(),
                                kind: *kind,
                            },
                        ))
                        .collect();
                    WorkspaceCrate { edges, ..c }
                })
                .collect(),
            ..workspace
        }
    }

    fn without_edge(workspace: Workspace, from: &str, to: &str) -> Workspace {
        Workspace {
            crates: workspace
                .crates
                .into_iter()
                .map(|c| {
                    let edges = c
                        .edges
                        .iter()
                        .filter(|e| !(c.name == from && e.to == to))
                        .cloned()
                        .collect();
                    WorkspaceCrate { edges, ..c }
                })
                .collect(),
            ..workspace
        }
    }

    fn without_crate(workspace: Workspace, name: &str) -> Workspace {
        Workspace {
            crates: workspace
                .crates
                .into_iter()
                .filter(|c| c.name != name)
                .collect(),
            ..workspace
        }
    }

    fn layer1(file: &str, message: impl Into<String>) -> Problem {
        Problem {
            rule: "LAYER-1",
            file: file.into(),
            line: None,
            message: message.into(),
        }
    }

    fn by_file_and_message(problems: Vec<Problem>) -> Vec<Problem> {
        let mut sorted = problems;
        sorted.sort_by(|a, b| (&a.file, &a.message).cmp(&(&b.file, &b.message)));
        sorted
    }

    #[track_caller]
    fn assert_problems(actual: Vec<Problem>, expected: Vec<Problem>) {
        assert_eq!(by_file_and_message(actual), by_file_and_message(expected));
    }

    fn layers(workspace: &Workspace, table: &str) -> Vec<Problem> {
        crates_follow_the_layers(workspace, &architecture(table))
    }

    fn unlisted(from: &str, to: &str, kinds: &str, manifest: &str) -> Problem {
        layer1(
            manifest,
            format!(
                "LAYER-1: {from} depends on {to} ({kinds}), but the layers table in docs/ARCHITECTURE.md does not list it.  Fix: remove the dependency from {manifest}.  If the design changes, add {to} to {from}'s row in the same commit."
            ),
        )
    }

    fn stale(from: &str, to: &str, manifest: &str) -> Problem {
        layer1(
            ARCH,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md lists {to} for {from}, but {manifest} has no dependency on it.  Fix: remove {to} from {from}'s row, so the table stays true."
            ),
        )
    }

    fn not_lower(from: &str, from_layer: &str, to: &str, to_layer: &str) -> Problem {
        layer1(
            ARCH,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md lets {from} ({from_layer}) depend on {to} ({to_layer}), which is not in a lower layer.  Fix: remove the dependency, or move a crate to another layer of the table, so that each crate depends only on lower layers."
            ),
        )
    }

    fn wrong_layers(layers: &str) -> Problem {
        layer1(
            ARCH,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has the layers \"{layers}\", not those in LAYERS in xtask/src/rules.rs.  Fix: use the five layers in LAYERS: Test support, Pure, Engine and GPU, Shells, Tools.  To change the layers, change the table, LAYERS and LAYER-1 in one commit (DOC-1)."
            ),
        )
    }

    fn apart(layer: &str) -> Problem {
        layer1(
            ARCH,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has rows for {layer} that are not together.  Fix: put each layer's rows together, lowest layer first."
            ),
        )
    }

    fn no_row(name: &str) -> Problem {
        layer1(
            ARCH,
            format!(
                "LAYER-1: the workspace crate {name} has no row in the layers table in docs/ARCHITECTURE.md.  Fix: add a row with its layer and the workspace crates that it depends on."
            ),
        )
    }

    fn unreadable(line: &str) -> Problem {
        layer1(
            ARCH,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has a row that this check cannot read: \"{line}\".  Fix: give the layer and one crate in a code span.  Then give its dependencies once each, as code spans joined by \", \", or \"no workspace crate\"."
            ),
        )
    }

    fn bad_start(line: &str) -> Problem {
        layer1(
            ARCH,
            format!(
                "LAYER-1: the layers table in docs/ARCHITECTURE.md starts with \"{line}\", not \"| Layer | Crate | Depends on |\" and \"| - | - | - |\".  Fix: use those three columns, one row for each crate."
            ),
        )
    }

    const NO_TABLE: &str = "LAYER-1: docs/ARCHITECTURE.md has no table under ### Layers, so this check read nothing.  Fix: restore the table, with the columns Layer, Crate and Depends on.";

    fn path_stray(from: &str, name: &str, path: &str, manifest: &str) -> Problem {
        layer1(
            manifest,
            format!(
                "LAYER-1: {from} takes {name} from {path}, which is not a workspace crate, so this check cannot see what {name} depends on.  Fix: make {name} a workspace crate with a row in the layers table, or remove it."
            ),
        )
    }

    fn shared(names: &str) -> Problem {
        layer1(
            "Cargo.toml",
            format!(
                "LAYER-1: app and stem-worker both depend on {names}, directly or through other workspace crates.  The page side and the worker side meet only through stem-protocol.  Fix: keep each crate on one side, and send what the other side needs as a message in stem-protocol."
            ),
        )
    }

    #[test]
    fn the_layers_of_today_give_no_problem() {
        assert_problems(layers(&today(), LAYERS_TABLE), vec![]);
        let trailing_spaces = architecture(LAYERS_TABLE).replace("### Layers\n", "### Layers  \n");
        assert_problems(crates_follow_the_layers(&today(), &trailing_spaces), vec![]);
    }

    #[test]
    fn an_edge_that_the_table_does_not_list_is_a_problem() {
        let workspace = with_edges(
            today(),
            &[
                ("engine", "app", Kind::Normal),
                ("stem-worker", "dsp", Kind::Normal),
            ],
        );
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![
                unlisted("engine", "app", "normal", "engine/Cargo.toml"),
                unlisted("stem-worker", "dsp", "normal", "worker/Cargo.toml"),
            ],
        );
    }

    #[test]
    fn a_dev_edge_counts() {
        let workspace = with_edges(today(), &[("engine", "app", Kind::Dev)]);
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![unlisted("engine", "app", "dev", "engine/Cargo.toml")],
        );
    }

    #[test]
    fn an_edge_from_a_crate_with_no_workspace_crate_is_a_problem() {
        let workspace = with_edges(today(), &[("synth", "dsp", Kind::Normal)]);
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![unlisted("synth", "dsp", "normal", "synth/Cargo.toml")],
        );
    }

    #[test]
    fn a_dev_edge_keeps_a_listed_crate() {
        let table = edit(
            LAYERS_TABLE,
            GPU_ROW,
            "| Engine and GPU | `roformer-gpu` | `roformer`, `dsp` |\n",
        );
        for kind in [Kind::Dev, Kind::Build] {
            let workspace = with_edges(today(), &[("roformer-gpu", "dsp", kind)]);
            assert_problems(layers(&workspace, &table), vec![]);
        }
    }

    #[test]
    fn kinds_are_named_once_in_their_order() {
        let workspace = with_edges(
            today(),
            &[
                ("engine", "app", Kind::Build),
                ("engine", "app", Kind::Dev),
                ("engine", "app", Kind::Normal),
                ("engine", "app", Kind::Normal),
            ],
        );
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![unlisted(
                "engine",
                "app",
                "normal, dev, build",
                "engine/Cargo.toml",
            )],
        );
    }

    #[test]
    fn a_listed_crate_without_an_edge_is_a_problem() {
        let workspace = without_edge(
            without_edge(today(), "app", "dsp"),
            "stem-worker",
            "roformer",
        );
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![
                stale("app", "dsp", "app/Cargo.toml"),
                stale("stem-worker", "roformer", "worker/Cargo.toml"),
            ],
        );
    }

    #[test]
    fn a_row_that_lists_a_higher_layer_is_a_problem() {
        let table = edit(
            LAYERS_TABLE,
            ENGINE_ROW,
            "| Engine and GPU | `engine` | `dsp`, `app` |\n",
        );
        let workspace = with_edges(today(), &[("engine", "app", Kind::Normal)]);
        assert_problems(
            layers(&workspace, &table),
            vec![not_lower("engine", "Engine and GPU", "app", "Shells")],
        );
    }

    #[test]
    fn a_row_that_lists_its_own_layer_is_a_problem() {
        let table = edit(LAYERS_TABLE, SYNTH_ROW, "| Pure | `synth` | `dsp` |\n");
        let workspace = with_edges(today(), &[("synth", "dsp", Kind::Normal)]);
        assert_problems(
            layers(&workspace, &table),
            vec![not_lower("synth", "Pure", "dsp", "Pure")],
        );
    }

    #[test]
    fn the_layers_are_fixed() {
        let base = edit(
            &edit(LAYERS_TABLE, DSP_ROW, ""),
            TEST_SUPPORT_ROW,
            &format!("{TEST_SUPPORT_ROW}| Base | `dsp` | no workspace crate |\n"),
        );
        let base = edit(&base, SYNTH_ROW, "| Pure | `synth` | `dsp` |\n");
        let workspace = with_edges(today(), &[("synth", "dsp", Kind::Normal)]);
        assert_problems(
            layers(&workspace, &base),
            vec![wrong_layers(
                "Test support, Base, Pure, Engine and GPU, Shells, Tools",
            )],
        );

        let moved = edit(
            &edit(LAYERS_TABLE, TEST_SUPPORT_ROW, ""),
            PROTOCOL_ROW,
            &format!("{PROTOCOL_ROW}{TEST_SUPPORT_ROW}"),
        );
        assert_problems(
            layers(&today(), &moved),
            vec![wrong_layers(
                "Pure, Test support, Engine and GPU, Shells, Tools",
            )],
        );

        let missing = edit(
            LAYERS_TABLE,
            TEST_SUPPORT_ROW,
            "| Pure | `test-support` | no workspace crate |\n",
        );
        assert_problems(
            layers(&today(), &missing),
            vec![wrong_layers("Pure, Engine and GPU, Shells, Tools")],
        );
    }

    #[test]
    fn a_layer_whose_rows_are_apart_is_a_problem() {
        let table = edit(
            &edit(&edit(LAYERS_TABLE, ROFORMER_ROW, ""), PROTOCOL_ROW, ""),
            XTASK_ROW,
            &format!("{XTASK_ROW}{ROFORMER_ROW}{PROTOCOL_ROW}"),
        );
        assert_problems(layers(&today(), &table), vec![apart("Pure")]);
    }

    #[test]
    fn a_row_left_out_is_left_out_of_the_layer_checks() {
        let not_a_member = edit(
            LAYERS_TABLE,
            XTASK_ROW,
            &format!("{XTASK_ROW}| Base | `foo` | no workspace crate |\n"),
        );
        assert_problems(
            layers(&today(), &not_a_member),
            vec![layer1(
                ARCH,
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has a row for foo, which is not a workspace crate.  Fix: remove the row, or use the package name from its Cargo.toml.",
            )],
        );
        let repeated = edit(
            LAYERS_TABLE,
            XTASK_ROW,
            &format!("{XTASK_ROW}| Engine and GPU | `engine` | `dsp` |\n"),
        );
        assert_problems(
            layers(&today(), &repeated),
            vec![layer1(
                ARCH,
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has 2 rows for engine.  Fix: keep one row for each crate.",
            )],
        );
    }

    #[test]
    fn a_crate_without_a_row_is_a_problem() {
        let table = edit(
            &edit(LAYERS_TABLE, ENGINE_ROW, ""),
            XTASK_ROW,
            "| Tools | `xtask` | `dsp`, `roformer`, `synth` |\n",
        );
        assert_problems(layers(&today(), &table), vec![no_row("engine")]);
    }

    #[test]
    fn a_row_for_a_crate_that_is_not_a_member_is_a_problem() {
        let table = edit(
            LAYERS_TABLE,
            PROTOCOL_ROW,
            &format!("{PROTOCOL_ROW}| Pure | `foo` | `dsp` |\n"),
        );
        assert_problems(
            layers(&today(), &table),
            vec![layer1(
                ARCH,
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has a row for foo, which is not a workspace crate.  Fix: remove the row, or use the package name from its Cargo.toml.",
            )],
        );
    }

    #[test]
    fn a_repeated_row_is_a_problem() {
        let table = edit(
            LAYERS_TABLE,
            ENGINE_ROW,
            &format!("{ENGINE_ROW}| Engine and GPU | `engine` | `app` |\n"),
        );
        assert_problems(
            layers(&today(), &table),
            vec![layer1(
                ARCH,
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has 2 rows for engine.  Fix: keep one row for each crate.",
            )],
        );
    }

    #[test]
    fn a_listed_crate_that_is_not_a_member_is_a_problem() {
        let table = edit(
            LAYERS_TABLE,
            APP_ROW,
            "| Shells | `app` | `dsp`, `engine`, `stem-protocol`, `synth`, `foo` |\n",
        );
        assert_problems(
            layers(&today(), &table),
            vec![layer1(
                ARCH,
                "LAYER-1: the layers table in docs/ARCHITECTURE.md lists foo for app, but foo is not a workspace crate.  Fix: correct the name, or remove it from app's row.",
            )],
        );
    }

    #[test]
    fn test_support_as_a_dev_dependency_needs_no_row() {
        let workspace = with_edges(
            today(),
            &[
                ("app", "test-support", Kind::Dev),
                ("stem-protocol", "test-support", Kind::Dev),
                ("xtask", "test-support", Kind::Dev),
            ],
        );
        assert_problems(layers(&workspace, LAYERS_TABLE), vec![]);
    }

    #[test]
    fn test_support_as_a_normal_or_build_dependency_is_a_problem() {
        let workspace = with_edges(
            today(),
            &[
                ("engine", "test-support", Kind::Normal),
                ("synth", "test-support", Kind::Build),
            ],
        );
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![
                layer1(
                    "engine/Cargo.toml",
                    "LAYER-1: engine takes test-support as a normal dependency, so it would ship.  Fix: move it to [dev-dependencies] in engine/Cargo.toml.",
                ),
                layer1(
                    "synth/Cargo.toml",
                    "LAYER-1: synth takes test-support as a build dependency, but it is a dev-dependency only.  Fix: move it to [dev-dependencies] in synth/Cargo.toml.",
                ),
            ],
        );
    }

    #[test]
    fn test_support_between_dev_edges_is_still_a_problem() {
        let workspace = with_edges(
            today(),
            &[
                ("engine", "test-support", Kind::Dev),
                ("engine", "test-support", Kind::Normal),
                ("engine", "test-support", Kind::Dev),
                ("engine", "test-support", Kind::Normal),
            ],
        );
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![layer1(
                "engine/Cargo.toml",
                "LAYER-1: engine takes test-support as a normal dependency, so it would ship.  Fix: move it to [dev-dependencies] in engine/Cargo.toml.",
            )],
        );
    }

    #[test]
    fn dsp_may_not_take_test_support() {
        let dsp_problem = || {
            vec![layer1(
                "dsp/Cargo.toml",
                "LAYER-1: dsp takes test-support, but dsp must have no dependencies (DSP-1).  Fix: remove it from dsp/Cargo.toml.",
            )]
        };
        let workspace = with_edges(
            today(),
            &[
                ("dsp", "test-support", Kind::Dev),
                ("dsp", "test-support", Kind::Normal),
            ],
        );
        assert_problems(layers(&workspace, LAYERS_TABLE), dsp_problem());
        let build_only = with_edges(today(), &[("dsp", "test-support", Kind::Build)]);
        assert_problems(layers(&build_only, LAYERS_TABLE), dsp_problem());
    }

    #[test]
    fn test_support_takes_no_workspace_crate() {
        let workspace = with_edges(
            today(),
            &[
                ("test-support", "dsp", Kind::Dev),
                ("test-support", "dsp", Kind::Normal),
            ],
        );
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![layer1(
                "test-support/Cargo.toml",
                "LAYER-1: test-support takes dsp, but test-support must take no workspace crate.  The crates that take it must reach nothing through it.  Fix: remove dsp from test-support/Cargo.toml.",
            )],
        );
    }

    #[test]
    fn a_row_that_lists_test_support_is_a_problem() {
        let table = edit(
            LAYERS_TABLE,
            ENGINE_ROW,
            "| Engine and GPU | `engine` | `dsp`, `test-support` |\n",
        );
        assert_problems(
            layers(&today(), &table),
            vec![layer1(
                ARCH,
                "LAYER-1: the layers table in docs/ARCHITECTURE.md lists test-support for engine.  Fix: remove it.  The table leaves out test-support, which any crate but dsp may take as a dev-dependency.",
            )],
        );
    }

    #[test]
    fn strays_are_problems() {
        let base = today();
        let workspace = Workspace {
            crates: base
                .crates
                .into_iter()
                .map(|c| {
                    let strays = if c.name == "app" {
                        vec![
                            Stray {
                                name: "shim".into(),
                                origin: Origin::Path("/x/shim".into()),
                            },
                            Stray {
                                name: "roformer".into(),
                                origin: Origin::Source("git+https://example.com/x".into()),
                            },
                        ]
                    } else {
                        c.strays.clone()
                    };
                    WorkspaceCrate { strays, ..c }
                })
                .collect(),
            outside: vec![
                Stray {
                    name: "shim".into(),
                    origin: Origin::Path("/x/shim".into()),
                },
                Stray {
                    name: "roformer".into(),
                    origin: Origin::Source("git+https://example.com/x".into()),
                },
            ],
            ..base
        };
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![
                path_stray("app", "shim", "/x/shim", "app/Cargo.toml"),
                layer1(
                    "app/Cargo.toml",
                    "LAYER-1: app takes roformer from git+https://example.com/x, not from the workspace.  Fix: take roformer by path, or remove it.",
                ),
                layer1(
                    "Cargo.toml",
                    "LAYER-1: the build takes shim from /x/shim, a path crate that is not a workspace crate, so this check cannot see what shim depends on.  Fix: remove the [patch] or exclude that brings it in.  Or make it a workspace crate with a row in the layers table.",
                ),
                layer1(
                    "Cargo.toml",
                    "LAYER-1: the build takes roformer from git+https://example.com/x, not from the workspace.  Fix: remove the dependency that brings it in.",
                ),
            ],
        );
    }

    #[test]
    fn an_outside_package_that_takes_a_member_is_a_problem() {
        let workspace = Workspace {
            outside_takes: vec![OutsideEdge {
                from: "foo".into(),
                source: "git+file:///x/gitfoo".into(),
                to: "roformer".into(),
            }],
            ..today()
        };
        assert_problems(
            layers(&workspace, LAYERS_TABLE),
            vec![layer1(
                "Cargo.toml",
                "LAYER-1: foo, from git+file:///x/gitfoo, takes the workspace crate roformer, so this check cannot see the dependency.  Fix: remove the [patch] that points roformer at the workspace, or remove foo.",
            )],
        );
    }

    #[test]
    fn a_missing_table_is_a_problem() {
        let no_heading = architecture(LAYERS_TABLE).replace("### Layers", "### Tiers");
        assert_problems(
            crates_follow_the_layers(&today(), &no_heading),
            vec![layer1(ARCH, NO_TABLE)],
        );

        for later in ["## X", "### X"] {
            let table_in_a_later_section = format!(
                "# How it works\n\n### Layers\n\nCrates depend only downwards.\n\n{later}\n\n{LAYERS_TABLE}"
            );
            assert_problems(
                crates_follow_the_layers(&today(), &table_in_a_later_section),
                vec![layer1(ARCH, NO_TABLE)],
            );
        }

        let base = today();
        let with_a_stray = Workspace {
            crates: base
                .crates
                .into_iter()
                .map(|c| {
                    let strays = if c.name == "app" {
                        vec![Stray {
                            name: "shim".into(),
                            origin: Origin::Path("/x/shim".into()),
                        }]
                    } else {
                        c.strays.clone()
                    };
                    WorkspaceCrate { strays, ..c }
                })
                .collect(),
            ..base
        };
        assert_problems(
            crates_follow_the_layers(&with_a_stray, &no_heading),
            vec![
                layer1(ARCH, NO_TABLE),
                path_stray("app", "shim", "/x/shim", "app/Cargo.toml"),
            ],
        );
    }

    #[test]
    fn the_old_table_is_a_problem() {
        let old = "\
| Layer | Crates | Depend on |
| - | - | - |
| Test support | `test-support` | no workspace crate |
| Pure | `dsp`, `synth`, `roformer`, `stem-protocol` | no workspace crate |
| Engine and GPU | `engine`, `roformer-gpu` | `dsp`; `roformer` |
";
        assert_problems(
            layers(&today(), old),
            vec![bad_start("| Layer | Crates | Depend on |")],
        );
    }

    #[test]
    fn a_bad_separator_is_a_problem() {
        let table = edit(LAYERS_TABLE, "| - | - | - |\n", "");
        assert_problems(
            layers(&today(), &table),
            vec![bad_start(
                "| Test support | `test-support` | no workspace crate |",
            )],
        );
        for separator in ["| - | - |", "| - |  | - |"] {
            let table = edit(LAYERS_TABLE, "| - | - | - |\n", &format!("{separator}\n"));
            assert_problems(layers(&today(), &table), vec![bad_start(separator)]);
        }
    }

    #[test]
    fn a_header_alone_is_a_problem() {
        assert_problems(
            layers(&today(), "| Layer | Crate | Depends on |\n"),
            vec![layer1(
                ARCH,
                "LAYER-1: the layers table in docs/ARCHITECTURE.md has no separator line after its header.  Fix: add \"| - | - | - |\" after \"| Layer | Crate | Depends on |\", then one row for each crate.",
            )],
        );
    }

    #[test]
    fn a_row_that_cannot_be_read_is_a_problem() {
        let cases = [
            "| Pure | `dsp` | `dsp` (dev only) |",
            "| Pure | dsp | no workspace crate |",
            "| Pure | `dsp` | dsp |",
            "| Pure | `dsp` (pure) | no workspace crate |",
            "| Pure | `dsp`, `synth` | no workspace crate |",
            "| Pure | `dsp` |",
            "|  | `dsp` | no workspace crate |",
            "| Pure | `dsp` | no workspace crate",
            "| Pure | `dsp` |  |",
            "| Pure | `dsp` | `dsp`,`engine` |",
            "| Pure | `dsp` | `dsp`, `dsp` |",
            "| Pure | `` | no workspace crate |",
        ];
        for case in cases {
            let table = edit(LAYERS_TABLE, XTASK_ROW, &format!("{XTASK_ROW}{case}\n"));
            assert_problems(layers(&today(), &table), vec![unreadable(case)]);
        }
    }

    #[test]
    fn an_empty_workspace_is_a_problem() {
        assert_problems(
            layers(
                &Workspace::default(),
                "| Layer | Crate | Depends on |\n| - | - | - |\n",
            ),
            vec![layer1(
                "Cargo.toml",
                "LAYER-1: cargo metadata gave no workspace crate, so this check read nothing.  Fix: run cargo metadata --format-version 1 --all-features --locked at the workspace root.",
            )],
        );
    }

    fn shells(workspace: &Workspace) -> Vec<Problem> {
        shells_meet_only_through_the_protocol(&workspace.crates)
    }

    #[test]
    fn the_shells_of_today_meet_only_through_stem_protocol() {
        assert_problems(shells(&today()), vec![]);
    }

    #[test]
    fn a_crate_on_both_sides_is_a_problem() {
        let workspace = with_edges(today(), &[("app", "roformer-gpu", Kind::Normal)]);
        assert_problems(shells(&workspace), vec![shared("roformer, roformer-gpu")]);
    }

    #[test]
    fn a_crate_reached_through_another_is_found() {
        let workspace = with_edges(today(), &[("engine", "roformer", Kind::Normal)]);
        assert_problems(shells(&workspace), vec![shared("roformer")]);
    }

    #[test]
    fn the_worker_side_is_followed_to_every_depth() {
        let workspace = with_edges(
            without_edge(today(), "stem-worker", "roformer"),
            &[("roformer", "dsp", Kind::Normal)],
        );
        assert_problems(shells(&workspace), vec![shared("dsp")]);
    }

    #[test]
    fn a_crate_reached_through_stem_protocol_is_found() {
        let workspace = with_edges(today(), &[("stem-protocol", "test-support", Kind::Normal)]);
        assert_problems(shells(&workspace), vec![shared("test-support")]);
    }

    #[test]
    fn a_shell_is_in_its_own_set() {
        let workspace = with_edges(today(), &[("stem-worker", "app", Kind::Normal)]);
        assert_problems(shells(&workspace), vec![shared("app, dsp, engine, synth")]);
        let workspace = with_edges(today(), &[("app", "stem-worker", Kind::Normal)]);
        assert_problems(
            shells(&workspace),
            vec![shared("roformer, roformer-gpu, stem-worker")],
        );
    }

    #[test]
    fn a_dev_or_build_edge_does_not_join_the_sides() {
        let workspace = with_edges(
            today(),
            &[
                ("app", "roformer", Kind::Dev),
                ("engine", "roformer", Kind::Dev),
                ("app", "roformer", Kind::Build),
                ("engine", "roformer", Kind::Build),
            ],
        );
        assert_problems(shells(&workspace), vec![]);
    }

    #[test]
    fn a_cycle_ends() {
        let workspace = with_edges(today(), &[("dsp", "engine", Kind::Normal)]);
        assert_problems(shells(&workspace), vec![]);
    }

    #[test]
    fn a_missing_shell_is_a_problem() {
        for shell in ["stem-worker", "app"] {
            assert_problems(
                shells(&without_crate(today(), shell)),
                vec![layer1(
                    "Cargo.toml",
                    format!(
                        "LAYER-1: the workspace has no crate {shell}, so this check cannot compare the page side and the worker side.  Fix: if the web app changed, change WEB_APP in xtask/src/rules.rs and LAYER-1's text (DOC-1)."
                    ),
                )],
            );
        }
    }

    // The parser of `cargo metadata`'s JSON.  Paths are under `/x`, so no
    // home folder is committed.

    fn id(name: &str, folder: &str) -> String {
        format!("path+file://{folder}#{name}@0.1.0")
    }

    fn package(name: &str, folder: &str, source: Option<&str>, dependencies: Vec<Value>) -> Value {
        json!({
            "id": source.map_or_else(|| id(name, folder), |s| format!("{s}#{name}@0.1.0")),
            "name": name,
            "source": source,
            "manifest_path": format!("{folder}/Cargo.toml"),
            "dependencies": dependencies,
            "features": {},
        })
    }

    fn path_dep(name: &str, path: &str, kind: Option<&str>) -> Value {
        json!({ "name": name, "source": null, "rename": null, "kind": kind, "target": null, "path": path })
    }

    fn source_dep(name: &str, source: &str) -> Value {
        json!({ "name": name, "source": source, "rename": null, "kind": null, "target": null })
    }

    /// The JSON, with a resolve node for each member (no features, no
    /// dependencies) and the `other_nodes`.
    fn metadata(packages: Vec<Value>, members: &[String], other_nodes: Vec<Value>) -> String {
        let nodes: Vec<Value> = members
            .iter()
            .map(|member| json!({ "id": member, "deps": [], "features": [] }))
            .chain(other_nodes)
            .collect();
        json!({
            "packages": packages,
            "workspace_members": members,
            "workspace_root": "/x",
            "resolve": { "nodes": nodes },
        })
        .to_string()
    }

    #[test]
    fn path_dependencies_on_members_are_edges() {
        let synth_for_tests = json!({
            "name": "synth", "source": null, "rename": null, "kind": "dev",
            "target": "cfg(not(target_arch = \"wasm32\"))", "path": "/x/synth",
        });
        let renamed_engine = json!({
            "name": "engine", "source": null, "rename": "audio", "kind": null,
            "target": null, "path": "/x/engine",
        });
        let members = [
            ("roformer", "/x/roformer"),
            ("roformer-gpu", "/x/roformer-gpu"),
            ("dsp", "/x/dsp"),
            ("engine", "/x/engine"),
            ("synth", "/x/synth"),
            ("stem-protocol", "/x/protocol"),
            ("app", "/x/app"),
            ("stem-worker", "/x/worker"),
        ];
        let deps = |name: &str| -> Vec<Value> {
            match name {
                "engine" => vec![path_dep("engine", "/x/engine", Some("dev"))],
                "app" => vec![
                    path_dep("dsp", "/x/dsp", None),
                    renamed_engine.clone(),
                    path_dep("synth", "/x/synth", None),
                    synth_for_tests.clone(),
                    path_dep("stem-protocol", "/x/protocol", Some("build")),
                    source_dep(
                        "anyhow",
                        "registry+https://github.com/rust-lang/crates.io-index",
                    ),
                ],
                "stem-worker" => vec![path_dep("roformer-gpu", "/x/roformer-gpu", None)],
                _ => vec![],
            }
        };
        let json = metadata(
            members
                .iter()
                .map(|(name, folder)| package(name, folder, None, deps(name)))
                .collect(),
            &members.map(|(name, folder)| id(name, folder)),
            vec![],
        );
        use Kind::{Build, Dev, Normal};
        let expected = Workspace {
            crates: vec![
                krate("roformer", "roformer/Cargo.toml", &[]),
                krate("roformer-gpu", "roformer-gpu/Cargo.toml", &[]),
                krate("dsp", "dsp/Cargo.toml", &[]),
                krate("engine", "engine/Cargo.toml", &[]),
                krate("synth", "synth/Cargo.toml", &[]),
                krate("stem-protocol", "protocol/Cargo.toml", &[]),
                krate(
                    "app",
                    "app/Cargo.toml",
                    &[
                        ("dsp", Normal),
                        ("engine", Normal),
                        ("synth", Normal),
                        ("synth", Dev),
                        ("stem-protocol", Build),
                    ],
                ),
                krate(
                    "stem-worker",
                    "worker/Cargo.toml",
                    &[("roformer-gpu", Normal)],
                ),
            ],
            ..Workspace::default()
        };
        assert_eq!(workspace(&json).expect("the JSON parses"), expected);
    }

    #[test]
    fn a_path_that_is_not_a_member_is_a_stray() {
        let json = metadata(
            vec![
                package(
                    "app",
                    "/x/app",
                    None,
                    vec![
                        path_dep("shim", "/x/shim", None),
                        path_dep("nested", "/x/app/nested", None),
                        path_dep("roformer", "/x/vendor/roformer", None),
                    ],
                ),
                package("roformer", "/x/roformer", None, vec![]),
            ],
            &[id("app", "/x/app"), id("roformer", "/x/roformer")],
            vec![],
        );
        let path = |name: &str, folder: &str| Stray {
            name: name.into(),
            origin: Origin::Path(folder.into()),
        };
        let expected = Workspace {
            crates: vec![
                WorkspaceCrate {
                    strays: vec![
                        path("shim", "/x/shim"),
                        path("nested", "/x/app/nested"),
                        path("roformer", "/x/vendor/roformer"),
                    ],
                    ..krate("app", "app/Cargo.toml", &[])
                },
                krate("roformer", "roformer/Cargo.toml", &[]),
            ],
            ..Workspace::default()
        };
        assert_eq!(workspace(&json).expect("the JSON parses"), expected);
    }

    #[test]
    fn a_members_name_from_elsewhere_is_a_stray() {
        let json = metadata(
            vec![
                package(
                    "dsp",
                    "/x/dsp",
                    None,
                    vec![
                        source_dep(
                            "engine",
                            "registry+https://github.com/rust-lang/crates.io-index",
                        ),
                        source_dep("roformer", "git+https://example.com/x"),
                    ],
                ),
                package("engine", "/x/engine", None, vec![]),
                package("roformer", "/x/roformer", None, vec![]),
            ],
            &[
                id("dsp", "/x/dsp"),
                id("engine", "/x/engine"),
                id("roformer", "/x/roformer"),
            ],
            vec![],
        );
        let expected = Workspace {
            crates: vec![
                WorkspaceCrate {
                    strays: vec![
                        Stray {
                            name: "engine".into(),
                            origin: Origin::Source(
                                "registry+https://github.com/rust-lang/crates.io-index".into(),
                            ),
                        },
                        Stray {
                            name: "roformer".into(),
                            origin: Origin::Source("git+https://example.com/x".into()),
                        },
                    ],
                    ..krate("dsp", "dsp/Cargo.toml", &[])
                },
                krate("engine", "engine/Cargo.toml", &[]),
                krate("roformer", "roformer/Cargo.toml", &[]),
            ],
            ..Workspace::default()
        };
        assert_eq!(workspace(&json).expect("the JSON parses"), expected);
    }

    #[test]
    fn a_path_package_that_is_not_a_member_is_outside() {
        let registry = "registry+https://github.com/rust-lang/crates.io-index";
        let json = metadata(
            vec![
                package("app", "/x/app", None, vec![source_dep("shim", registry)]),
                package("roformer", "/x/roformer", None, vec![]),
                package(
                    "shim",
                    "/x/shim",
                    None,
                    vec![path_dep("roformer", "/x/roformer", None)],
                ),
                package(
                    "roformer",
                    "/git/roformer",
                    Some("git+https://example.com/x"),
                    vec![],
                ),
                package("roformer", "/x/vendor/roformer", None, vec![]),
            ],
            &[id("app", "/x/app"), id("roformer", "/x/roformer")],
            vec![json!({
                "id": id("shim", "/x/shim"),
                "deps": [ { "name": "roformer", "pkg": id("roformer", "/x/roformer") } ],
            })],
        );
        let expected = Workspace {
            crates: vec![
                krate("app", "app/Cargo.toml", &[]),
                krate("roformer", "roformer/Cargo.toml", &[]),
            ],
            outside: vec![
                Stray {
                    name: "shim".into(),
                    origin: Origin::Path("/x/shim".into()),
                },
                Stray {
                    name: "roformer".into(),
                    origin: Origin::Source("git+https://example.com/x".into()),
                },
                Stray {
                    name: "roformer".into(),
                    origin: Origin::Path("/x/vendor/roformer".into()),
                },
            ],
            outside_takes: vec![],
        };
        assert_eq!(workspace(&json).expect("the JSON parses"), expected);
    }

    #[test]
    fn a_package_that_is_not_a_member_is_left_out() {
        let json = metadata(
            vec![
                package("dsp", "/x/dsp", None, vec![]),
                package(
                    "anyhow",
                    "/registry/anyhow",
                    Some("registry+https://github.com/rust-lang/crates.io-index"),
                    vec![],
                ),
            ],
            &[id("dsp", "/x/dsp")],
            vec![],
        );
        let expected = Workspace {
            crates: vec![krate("dsp", "dsp/Cargo.toml", &[])],
            ..Workspace::default()
        };
        assert_eq!(workspace(&json).expect("the JSON parses"), expected);
    }

    #[test]
    fn an_outside_package_that_takes_a_member_is_found() {
        let git_foo = "git+file:///x/gitfoo";
        let git_roformer = "git+https://example.com/x";
        let json = metadata(
            vec![
                package("roformer", "/x/roformer", None, vec![]),
                package("roformer-gpu", "/x/roformer-gpu", None, vec![]),
                package("app", "/x/app", None, vec![source_dep("foo", git_foo)]),
                package("foo", "/git/foo", Some(git_foo), vec![]),
                package("roformer", "/git/roformer", Some(git_roformer), vec![]),
            ],
            &[
                id("roformer", "/x/roformer"),
                id("roformer-gpu", "/x/roformer-gpu"),
                id("app", "/x/app"),
            ],
            vec![json!({
                "id": format!("{git_foo}#foo@0.1.0"),
                "deps": [
                    { "name": "roformer_gpu", "pkg": id("roformer-gpu", "/x/roformer-gpu") },
                    { "name": "roformer", "pkg": format!("{git_roformer}#roformer@0.1.0") },
                ],
            })],
        );
        let expected = Workspace {
            crates: vec![
                krate("roformer", "roformer/Cargo.toml", &[]),
                krate("roformer-gpu", "roformer-gpu/Cargo.toml", &[]),
                krate("app", "app/Cargo.toml", &[]),
            ],
            outside: vec![Stray {
                name: "roformer".into(),
                origin: Origin::Source(git_roformer.into()),
            }],
            outside_takes: vec![OutsideEdge {
                from: "foo".into(),
                source: git_foo.into(),
                to: "roformer-gpu".into(),
            }],
        };
        assert_eq!(workspace(&json).expect("the JSON parses"), expected);
    }

    #[test]
    fn a_metadata_without_resolve_is_an_error() {
        let json = json!({
            "packages": [package("dsp", "/x/dsp", None, vec![])],
            "workspace_members": [id("dsp", "/x/dsp")],
            "workspace_root": "/x",
            "resolve": null,
        })
        .to_string();
        let error = workspace(&json).expect_err("a run with --no-deps is an error");
        assert!(error.to_string().contains("--no-deps"), "{error:#}");
    }

    #[test]
    fn a_metadata_without_all_features_is_an_error() {
        let xtask = json!({
            "id": id("xtask", "/x/xtask"),
            "name": "xtask",
            "source": null,
            "manifest_path": "/x/xtask/Cargo.toml",
            "dependencies": [],
            "features": { "cpal": ["engine/cpal"] },
        });
        let json = metadata(vec![xtask], &[id("xtask", "/x/xtask")], vec![]);
        let error = workspace(&json).expect_err("a run without --all-features is an error");
        assert!(
            error.to_string().contains("--all-features") && error.to_string().contains("cpal"),
            "{error:#}"
        );
    }

    #[test]
    fn a_member_without_a_resolve_node_is_an_error() {
        let json = json!({
            "packages": [package("dsp", "/x/dsp", None, vec![])],
            "workspace_members": [id("dsp", "/x/dsp")],
            "workspace_root": "/x",
            "resolve": { "nodes": [] },
        })
        .to_string();
        let error = workspace(&json).expect_err("a member without a node is an error");
        assert!(error.to_string().contains("no node for dsp"), "{error:#}");
    }

    #[test]
    fn an_unknown_kind_is_an_error() {
        let json = metadata(
            vec![package(
                "app",
                "/x/app",
                None,
                vec![path_dep("dsp", "/x/dsp", Some("other"))],
            )],
            &[id("app", "/x/app")],
            vec![],
        );
        let error = workspace(&json).expect_err("an unknown kind is an error");
        assert!(error.to_string().contains("other"), "{error:#}");
    }

    #[test]
    fn a_manifest_outside_the_root_is_an_error() {
        for folder in ["/y/app", "/xy/app"] {
            let json = metadata(
                vec![package("app", folder, None, vec![])],
                &[id("app", folder)],
                vec![],
            );
            let error = workspace(&json).expect_err("a manifest outside the root is an error");
            assert!(error.to_string().contains(folder), "{error:#}");
        }
    }

    fn dsp_problems(file: &str, text: &str) -> Vec<Problem> {
        dsp_keeps_beginner_style(&[(file.into(), text.into())])
    }

    /// `statement` on line 2, in the body of a function.
    fn in_body(statement: &str) -> String {
        format!("fn f() {{\n    {statement}\n}}\n")
    }

    /// `attribute` on line 2, at the top of a file.
    fn at_top(attribute: &str) -> String {
        format!("//! A sound helper.\n{attribute}\nfn f() {{}}\n")
    }

    #[test]
    fn beginner_code_passes() {
        let lib = r#"//! Tiny sound helpers.
#![expect(
    clippy::assign_op_pattern,
    reason = "DSP-1: `x = x + y` shows the operation in full"
)]
#![expect(clippy::len_zero, reason = "DSP-1: `len() > 0` needs only `len`")]
#![expect(clippy::manual_clamp, reason = "DSP-1: an `if` for each bound")]
#![expect(clippy::manual_swap, reason = "DSP-1 forbids `swap`")]

mod mix;

pub use mix::mix;
"#;
        let mix = r#"/// Adds the stems.
pub fn mix(stems: &[&[f32]], volumes: &[f32]) -> Vec<(f32, f32)> {
    let mut result = Vec::new();
    let mut x = 0;
    x = x + 1;
    x = -1;
    let n = stems.len();
    if x <= 1 && x >= -1 || x == 0 && x != 2 {
        result.push((volumes[0], stems[0][x as usize]));
    }
    for i in 0..n {}
    for i in 0..=n {}
    for i in 0..stems.len() {}
    let mut i = 0;
    while i < n {
        i = i + 1;
    }
    let zeros = vec![0.0; n];
    assert!(zeros.len() > 0, "{}", result.capacity());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "mix needs at least one stem")]
    fn no_stems() {
        mix(&[], &[]);
    }
}
"#;
        let files = [
            ("dsp/Cargo.toml".into(), DSP_MANIFEST.into()),
            ("dsp/src/lib.rs".into(), lib.into()),
            ("dsp/src/mix.rs".into(), mix.into()),
        ];
        assert_eq!(dsp_keeps_beginner_style(&files), []);
    }

    #[test]
    fn each_breach_is_named() {
        let module = "dsp/src/mix.rs";
        let lib = "dsp/src/lib.rs";
        let compound: Vec<(&str, String, Vec<&str>)> =
            ["+", "-", "*", "/", "%", "^", "&", "|", "<<", ">>"]
                .into_iter()
                .map(|op| (module, in_body(&format!("a {op}= b;")), vec!["compound"]))
                .collect();
        let cases: Vec<(&str, String, Vec<&str>)> = [
            (module, in_body("let a = b.iter();"), vec!["`iter`"]),
            (module, in_body("let a = b.to_vec();"), vec!["`to_vec`"]),
            (module, in_body("b.sort_by(f);"), vec!["`sort_by`"]),
            (module, in_body("let a = b.min(c);"), vec!["`min`"]),
            (module, in_body("let a = b.r#iter();"), vec!["`iter`"]),
            (module, in_body("b.swap(c, d);"), vec!["`swap`"]),
            (
                module,
                in_body("std::mem::swap(&mut a, &mut b);"),
                vec!["`swap`"],
            ),
            (module, in_body("let a = f32::max(b, c);"), vec!["`max`"]),
            (module, in_body("let a = b.is_empty();"), vec!["`is_empty`"]),
            (module, in_body("let a = b.clamp(c, d);"), vec!["`clamp`"]),
            (module, in_body("let g = |x| x;"), vec!["closure"]),
            (module, in_body("let g = move || 0;"), vec!["closure"]),
            (module, in_body("for s in samples {}"), vec!["`for`"]),
            (module, in_body("for s in &samples[1..] {}"), vec!["`for`"]),
            (
                module,
                in_body("for i in (0..n).rev() {}"),
                vec!["`rev`", "`for`"],
            ),
            (
                module,
                in_body("assert!(a.iter().count() > 0);"),
                vec!["`iter`"],
            ),
            (lib, at_top("#![allow(clippy::all)]"), vec!["`allow`"]),
            (
                lib,
                at_top(r#"#![expect(clippy::all, reason = "x")]"#),
                vec!["`expect`"],
            ),
            (
                lib,
                at_top(r#"#![expect(clippy::eq_op, reason = "DSP-1")]"#),
                vec!["`expect`"],
            ),
            (
                lib,
                at_top(r#"#![expect(clippy::len_zero, reason = "x")]"#),
                vec!["`expect`"],
            ),
            (
                lib,
                in_body(r#"#![expect(clippy::len_zero, reason = "DSP-1: x")]"#),
                vec!["`expect`"],
            ),
            (
                module,
                at_top(r#"#![expect(clippy::len_zero, reason = "DSP-1: x")]"#),
                vec!["`expect`"],
            ),
            (
                module,
                in_body("#[allow(clippy::eq_op)]\n    let a = 1;"),
                vec!["`allow`"],
            ),
            (module, in_body("#![allow(clippy::eq_op)]"), vec!["`allow`"]),
            (
                lib,
                at_top("#![cfg_attr(clippy, allow(clippy::all))]"),
                vec!["`cfg_attr`"],
            ),
            (module, at_top("#[path = \"x.rs\"]"), vec!["`path`"]),
            (module, in_body("include!(\"x.rs\");"), vec!["include!"]),
            (module, in_body("r#include!(\"x.rs\");"), vec!["include!"]),
            (lib, at_top("#![r#allow(clippy::all)]"), vec!["`allow`"]),
            (
                module,
                at_top("use std::mem::{replace, swap};"),
                vec!["`swap`"],
            ),
            (module, at_top("use std::{cmp::{max}};"), vec!["`max`"]),
        ]
        .into_iter()
        .chain(compound)
        .collect();
        for (file, text, named) in cases {
            let problems = dsp_problems(file, &text);
            assert_eq!(problems.len(), named.len(), "{text}\n{problems:#?}");
            for (problem, name) in problems.iter().zip(&named) {
                assert_eq!(problem.rule, "DSP-1", "{text}");
                assert_eq!(problem.file, file, "{text}");
                assert_eq!(problem.line, Some(2), "{text}\n{problem}");
                assert!(
                    problem.message.starts_with(&format!("DSP-1: {file}:2 ")),
                    "{problem}"
                );
                assert!(problem.message.contains(name), "{name} in {problem}");
                assert!(problem.message.contains("Fix: "), "{problem}");
            }
        }
    }

    #[test]
    fn the_manifest_and_files_are_fixed() {
        let manifest = "dsp/Cargo.toml";
        assert_eq!(
            dsp_problems(manifest, &DSP_MANIFEST.replace('\n', "\r\n")),
            []
        );
        let cases = [
            (
                manifest,
                DSP_MANIFEST.replace("[lints]", "[dev-dependencies]\nrand = \"0.9\"\n\n[lints]"),
            ),
            (
                manifest,
                format!("{DSP_MANIFEST}\n[lints.clippy]\nall = \"allow\"\n"),
            ),
            ("dsp/build.rs", "fn main() {}\n".into()),
            (
                "dsp/clippy.toml",
                "too-many-lines-threshold = 1000\n".into(),
            ),
            ("dsp/src/notes.txt", "a note\n".into()),
            ("dsp/src/mix.rs", "fn f() { \"\n".into()),
        ];
        for (file, text) in cases {
            let problems = dsp_problems(file, &text);
            assert_eq!(problems.len(), 1, "{file}\n{text}\n{problems:#?}");
            assert_eq!(problems[0].rule, "DSP-1");
            assert_eq!(problems[0].file, file);
            assert_eq!(problems[0].line, None);
            assert!(
                problems[0].message.starts_with(&format!("DSP-1: {file} ")),
                "{}",
                problems[0]
            );
            assert!(problems[0].message.contains("Fix: "), "{}", problems[0]);
        }
    }
}
