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
const SKIP_ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "app/tests/web_output.rs",
        "skipped: the audio context is suspended",
        "a wasm test cannot read STEMYLA_REQUIRE; the test asserts that the browser is not Chrome, and CI runs Chrome",
    ),
    (
        "app/src/streamtest.rs",
        "skipped: the audio context is suspended",
        "a wasm test cannot read STEMYLA_REQUIRE; the test asserts the audio clock in Chrome, and CI runs Chrome",
    ),
];

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
pub const WEB_APP: [&str; 3] = ["app", "decode-worker", "stem-worker"];

/// The shells of the page side: the page and its decode worker.
pub const PAGE_SIDE: [&str; 2] = ["app", "decode-worker"];

/// The shell of the worker side.
pub const WORKER_SIDE: &str = "stem-worker";

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
        "decode-worker" => "decoder/Cargo.toml".into(),
        "stem-protocol" => "protocol/Cargo.toml".into(),
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
    /// The version requirement, normalised by Cargo (`0.1` is `^0.1`).
    /// `None` only in the layer tests' fixtures.
    #[serde(default)]
    req: Option<String>,
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
/// `stem-protocol`.  Each shell of the page side, and the worker side's
/// shell, is the shell and every workspace crate that it reaches through
/// normal edges, for any target.  Two crates of the page side may share
/// crates.
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
    let worker = reached(crates, WORKER_SIDE);
    PAGE_SIDE
        .iter()
        .filter_map(|page| {
            let shared: Vec<&str> = reached(crates, page)
                .intersection(&worker)
                .filter(|name| **name != PROTOCOL)
                .copied()
                .collect();
            (!shared.is_empty()).then(|| {
                layer1(
                    "Cargo.toml",
                    format!(
                        "LAYER-1: {page} and {WORKER_SIDE} both depend on {}, directly or through other workspace crates.  The page side and the worker side meet only through stem-protocol.  Fix: keep each crate on one side, and send what the other side needs as a message in stem-protocol.",
                        shared.join(", ")
                    ),
                )
            })
        })
        .collect()
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
/// files or manifest text that can relax the lints.  A name after its own
/// module, as in `pub use max::max`, is `dsp`'s own function; with a prefix,
/// as in `self::max::max`, it still fails.  TD-3 names what the scan cannot
/// see.
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
        let after_colons = punct_spacing(at(-1), ':').is_some() && joint(at(-2), ':');
        let own_module = after_colons
            && ident_name(at(-3))
                .is_some_and(|segment| segment.strip_prefix("r#").unwrap_or(&segment) == name);
        let after_path = first_in_import || (after_colons && !own_module);
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

/// A file in the git index, with the size of its staged content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedFile {
    pub path: String,
    pub size: u64,
}

/// Every file in the git index of `root`, with its staged size.  The index
/// is what the next commit holds, so a file staged with `git add -f` is
/// seen before the commit.  It needs git 2.42 or later, for `%(objectsize)`.
pub fn tracked_files(root: &Path) -> Result<Vec<TrackedFile>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--format=%(objectsize) %(path)"])
        .output()
        .context("running git ls-files")?;
    if !output.status.success() {
        anyhow::bail!(
            "git ls-files failed: {}.  The check needs git 2.42 or later, in a git checkout.",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text =
        String::from_utf8(output.stdout).context("git ls-files gave a path that is not UTF-8")?;
    parse_ls_files(&text)
}

/// Reads `git ls-files -z --format='%(objectsize) %(path)'`: entries that
/// end in NUL, each a size, a space and a path.
pub fn parse_ls_files(output: &str) -> Result<Vec<TrackedFile>> {
    output
        .split_terminator('\0')
        .map(|entry| {
            let (size, path) = entry
                .split_once(' ')
                .with_context(|| format!("git ls-files gave {entry:?}, with no space"))?;
            let size = size.parse().with_context(|| {
                format!("git ls-files gave the size {size:?} for {path}; the checks read files, not submodules")
            })?;
            Ok(TrackedFile {
                path: path.to_string(),
                size,
            })
        })
        .collect()
}

/// Each tracked file that reads as UTF-8, from the working tree, with its
/// text.  It skips a file that the working tree has deleted, since a
/// deletion adds nothing, a file that is not UTF-8, such as a PNG, and a
/// tracked link to a folder.
pub fn tracked_texts(root: &Path, tracked: &[TrackedFile]) -> Result<Vec<(String, String)>> {
    tracked
        .iter()
        .filter_map(|file| match std::fs::read(root.join(&file.path)) {
            Ok(bytes) => String::from_utf8(bytes)
                .ok()
                .map(|text| Ok((file.path.clone(), text))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) if e.kind() == std::io::ErrorKind::IsADirectory => None,
            Err(e) => Some(Err(
                anyhow::Error::new(e).context(format!("reading {}", file.path))
            )),
        })
        .collect()
}

/// The files that build, serve or test the project outside Cargo's own
/// Rust: CI and its actions, Trunk's configuration, build scripts, and
/// Cargo's configuration in any `.cargo/` folder.
pub fn is_build_file(path: &str) -> bool {
    let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
    let in_cargo_folder = folder == ".cargo" || folder.ends_with("/.cargo");
    path.starts_with(".github/")
        || name == "Trunk.toml"
        || name == "build.rs"
        || (in_cargo_folder && (name == "config.toml" || name == "config"))
}

/// The one Python file that BUILD-1 allows.
const EXPORT_TOOL: &str = "tools/roformer_export.py";

/// Extensions and file names of Python code and of its package tools,
/// compared in lower case.
const PYTHON_EXTENSIONS: [&str; 3] = ["py", "pyc", "ipynb"];
const PYTHON_FILE_NAMES: [&str; 4] = ["requirements.txt", "pyproject.toml", "pipfile", "uv.lock"];

/// Words that run Python or install its packages, in lower case.
const PYTHON_WORDS: [&str; 7] = ["python", "python3", "pip", "pip3", "pipx", "uv", "uvx"];

/// The names of Python's built-in web server, matched with case, as Python
/// matches module names.  Each is joined by `concat!`, so this file never
/// holds the name and needs no exception.
const PYTHON_SERVERS: [&str; 2] = [concat!("http", ".server"), concat!("SimpleHTTP", "Server")];

/// Folders of history, which may quote a breach.
const HISTORY: [&str; 3] = ["docs/plans/", "docs/decisions/", "docs/reviews/"];

/// The text after the last `.` of the file name, in lower case.
fn extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
}

/// BUILD-1: Python appears only in `tools/roformer_export.py`.  It fails a
/// tracked Python file, a Python word in a build file, and the name of
/// Python's web server in a tracked text file outside plans, decisions and
/// review logs.
/// `texts` holds the tracked files that read as UTF-8 (`tracked_texts`).
pub fn python_in_the_build(tracked: &[TrackedFile], texts: &[(String, String)]) -> Vec<Problem> {
    let python_files = tracked
        .iter()
        .filter(|file| file.path != EXPORT_TOOL && is_python_file(&file.path))
        .map(|file| Problem {
            rule: "BUILD-1",
            file: file.path.clone(),
            line: None,
            message: format!(
                "BUILD-1: {} is a Python file.  Fix: write it in Rust, as an xtask command; Python appears only in {EXPORT_TOOL}.",
                file.path
            ),
        });
    let in_texts = texts.iter().flat_map(|(file, text)| {
        let build_file = is_build_file(file);
        let history = HISTORY.iter().any(|folder| file.starts_with(folder));
        text.lines().enumerate().flat_map(move |(index, line)| {
            let lower = line.to_lowercase();
            let word = build_file
                .then(|| {
                    lower
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .find(|word| PYTHON_WORDS.contains(word))
                        .map(|word| format!("runs Python (\"{word}\") in a build file.  Fix: use Cargo, Trunk or an xtask command."))
                })
                .flatten();
            let server = (!history && PYTHON_SERVERS.iter().any(|server| line.contains(server)))
                .then(|| "names Python's web server.  Fix: serve with cargo run -p xtask -- serve.".to_string());
            [word, server].into_iter().flatten().map(move |what| Problem {
                rule: "BUILD-1",
                file: file.clone(),
                line: Some(index + 1),
                message: format!("BUILD-1: {file}:{} {what}", index + 1),
            })
        })
    });
    python_files.chain(in_texts).collect()
}

fn is_python_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    PYTHON_FILE_NAMES.contains(&name.as_str())
        || extension(path).is_some_and(|extension| PYTHON_EXTENSIONS.contains(&extension.as_str()))
}

/// Extensions of audio and of model weights, in lower case.
const AUDIO_EXTENSIONS: [&str; 9] = [
    "mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "aif", "aiff",
];
const WEIGHT_EXTENSIONS: [&str; 9] = [
    "safetensors",
    "ckpt",
    "pt",
    "pth",
    "bin",
    "onnx",
    "npy",
    "npz",
    "gguf",
];

/// The largest file that the index may hold: about twice the largest file on
/// 2026-10-20 (a UI snapshot of 486,664 bytes), and well under a song or a
/// model file.
pub const MAX_TRACKED_BYTES: u64 = 1_048_576;

/// ASSET-1: no audio or model weights in the repo.  It fails a tracked
/// file with an audio or weight extension, and a file over
/// `MAX_TRACKED_BYTES`.
pub fn assets_in_the_index(tracked: &[TrackedFile]) -> Vec<Problem> {
    tracked
        .iter()
        .flat_map(|file| {
            let path = &file.path;
            let asset = extension(path)
                .filter(|extension| {
                    AUDIO_EXTENSIONS.contains(&extension.as_str())
                        || WEIGHT_EXTENSIONS.contains(&extension.as_str())
                })
                .map(|extension| format!(
                    "ASSET-1: {path} has the audio or model weight extension .{extension}.  Fix: remove it from the index with git rm --cached '{path}'.  Keep it in an ignored folder (audio/, stems/ or third_party/).  Link to its source."
                ));
            let too_big = (file.size > MAX_TRACKED_BYTES).then(|| format!(
                "ASSET-1: {path} is {} bytes, over MAX_TRACKED_BYTES ({MAX_TRACKED_BYTES}).  Fix: if it is audio or weights, remove it with git rm --cached '{path}' and link to its source; else raise MAX_TRACKED_BYTES in xtask/src/rules.rs, with a reason in the PR.",
                file.size
            ));
            [asset, too_big].into_iter().flatten().map(|message| Problem {
                rule: "ASSET-1",
                file: path.clone(),
                line: None,
                message,
            })
        })
        .collect()
}

/// VERIFY-1: no build file sets `UPDATE_SNAPSHOTS`, which makes
/// egui_kittest rewrite a snapshot that does not match and pass.  It skips
/// comment lines, which start with `#` or `//`.  A line that names the
/// variable only in a trailing comment, or sets it to `0`, still counts.
pub fn snapshot_updates(texts: &[(String, String)]) -> Vec<Problem> {
    texts
        .iter()
        .filter(|(file, _)| is_build_file(file))
        .flat_map(|(file, text)| {
            text.lines()
                .enumerate()
                .filter(|(_, line)| {
                    let code = line.trim_start();
                    code.contains("UPDATE_SNAPSHOTS")
                        && !code.starts_with('#')
                        && !code.starts_with("//")
                })
                .map(move |(index, _)| Problem {
                    rule: "VERIFY-1",
                    file: file.clone(),
                    line: Some(index + 1),
                    message: format!(
                        "VERIFY-1: {file}:{} names UPDATE_SNAPSHOTS, which makes egui_kittest rewrite a snapshot that does not match, so the test passes.  Fix: remove it.  If a snapshot fails and you did not change the UI, stop and ask the maintainer (TD-29).  To change a snapshot on purpose, run `UPDATE_SNAPSHOTS=1 cargo test -p app --test ui`.  Then commit only the PNGs of the views that you changed.",
                        index + 1
                    ),
                })
        })
        .collect()
}

/// Crates below 1.0 that DEPS-1 counts as settled, not young.  Each date is
/// the crate's first version on crates.io, two years or more before the
/// crate was listed.  The check reads only the date's form; a reviewer
/// checks it at `https://crates.io/api/v1/crates/<crate>`.
pub const SETTLED_CRATES: [(&str, &str); 13] = [
    ("burn", "2022-07-27"),
    ("console_error_panic_hook", "2018-05-23"),
    ("cpal", "2014-12-17"),
    ("eframe", "2021-01-04"),
    ("futures-channel", "2018-03-05"),
    ("js-sys", "2018-07-19"),
    ("log", "2014-12-13"),
    ("safetensors", "2022-12-13"),
    ("symphonia", "2020-09-01"),
    ("wasm-bindgen", "2018-03-06"),
    ("wasm-bindgen-futures", "2018-08-13"),
    ("wasm-bindgen-test", "2018-07-26"),
    ("web-sys", "2018-07-19"),
];

const RULES_FILE: &str = "xtask/src/rules.rs";

fn deps1(file: &str, line: Option<usize>, message: String) -> Problem {
    Problem {
        rule: "DEPS-1",
        file: file.into(),
        line,
        message,
    }
}

fn is_registry(source: &str) -> bool {
    source.starts_with("registry+") || source.starts_with("sparse+")
}

/// One comparator `=M.m.p`, with or without a pre-release.
fn is_pin(requirement: &str) -> bool {
    let Some(version) = requirement.trim().strip_prefix('=') else {
        return false;
    };
    let version = version.trim();
    let core = version.split_once('-').map_or(version, |(core, _)| core);
    let parts: Vec<&str> = core.split('.').collect();
    !requirement.contains(',')
        && parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// No comparator has a pre-release, and one sets a lower bound of 1.0.0 or
/// more, so the requirement admits no version below 1.0.
fn is_settled_above_1(requirement: &str) -> bool {
    let comparators: Vec<&str> = requirement.split(',').map(str::trim).collect();
    comparators
        .iter()
        .all(|comparator| !comparator.contains('-'))
        && comparators
            .iter()
            .any(|comparator| sets_a_bound_of_1(comparator))
}

fn sets_a_bound_of_1(comparator: &str) -> bool {
    let operator_end = comparator
        .find(|c: char| c.is_ascii_digit() || c == '*')
        .unwrap_or(comparator.len());
    let (operator, version) = comparator.split_at(operator_end);
    let major: Option<u64> = version
        .split('.')
        .next()
        .and_then(|major| major.parse().ok());
    let at_least_1 = major.is_some_and(|major| major >= 1);
    match operator.trim() {
        "" | "^" | "~" | "=" | ">=" => at_least_1,
        ">" => at_least_1 || version == "0",
        _ => false,
    }
}

fn is_date(text: &str) -> bool {
    text.len() == 10
        && text.char_indices().all(|(index, c)| match index {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        })
}

/// A member's dependency on a crate that is not a path.
struct SourcedDependency<'a> {
    manifest: String,
    name: &'a str,
    source: &'a str,
    requirement: Option<&'a str>,
}

/// DEPS-1: a young crate is pinned with `=`.  A crate below 1.0 is young
/// unless `settled` names it (`SETTLED_CRATES` on the real tree).  It also
/// fails every source that is not a registry, in the members' manifests
/// and in the whole build, since a `[patch]` to git shows only there.
pub fn young_crates_are_pinned_in(
    metadata: &str,
    settled: &[(&str, &str)],
) -> Result<Vec<Problem>> {
    let metadata: Metadata =
        serde_json::from_str(metadata).context("reading cargo metadata's JSON")?;
    let is_member = |id: &str| metadata.workspace_members.iter().any(|member| member == id);
    let dependencies = metadata
        .packages
        .iter()
        .filter(|package| is_member(&package.id))
        .flat_map(|package| {
            let manifest = package
                .manifest_path
                .strip_prefix(&metadata.workspace_root)
                .unwrap_or(&package.manifest_path)
                .to_string_lossy()
                .replace('\\', "/");
            package.dependencies.iter().filter_map(move |dependency| {
                Some(SourcedDependency {
                    manifest: manifest.clone(),
                    name: &dependency.name,
                    source: dependency.source.as_deref()?,
                    requirement: dependency.req.as_deref(),
                })
            })
        })
        .collect::<Vec<_>>();
    let registry: Vec<&SourcedDependency> = dependencies
        .iter()
        .filter(|dependency| is_registry(dependency.source))
        .collect();
    let unpinned = registry
        .iter()
        .map(|dependency| {
            let requirement = dependency.requirement.with_context(|| {
                format!(
                    "cargo metadata gave no requirement for {} in {}",
                    dependency.name, dependency.manifest
                )
            })?;
            Ok((*dependency, requirement))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, requirement)| !is_pin(requirement) && !is_settled_above_1(requirement))
        .collect::<Vec<_>>();
    let young = unpinned
        .iter()
        .filter(|(dependency, _)| !settled.iter().any(|(name, _)| *name == dependency.name))
        .map(|(dependency, requirement)| {
            deps1(
                &dependency.manifest,
                None,
                format!(
                    "DEPS-1: {} takes {} {requirement}, which admits a version below 1.0 and is not a pin.  Fix: pin it, or the [workspace.dependencies] entry it inherits, with = and its version in Cargo.lock, or, if its first version on crates.io is two years old or more, add it to SETTLED_CRATES in {RULES_FILE} with that date.",
                    dependency.manifest, dependency.name
                ),
            )
        });
    let not_registry = dependencies
        .iter()
        .filter(|dependency| !is_registry(dependency.source))
        .map(|dependency| {
            deps1(
                &dependency.manifest,
                None,
                format!(
                    "DEPS-1: {} takes {} from {}, which is not a registry.  Fix: use a registry release, or ask the maintainer.",
                    dependency.manifest, dependency.name, dependency.source
                ),
            )
        });
    let built_from_elsewhere = metadata
        .packages
        .iter()
        .filter_map(|package| Some((package, package.source.as_deref()?)))
        .filter(|(_, source)| !is_registry(source))
        .map(|(package, source)| {
            deps1(
                "Cargo.toml",
                None,
                format!(
                    "DEPS-1: Cargo.toml and the manifests it names bring {} from {source}, which is not a registry; a [patch] can do this.  Fix: use a registry release, or ask the maintainer.",
                    package.name
                ),
            )
        });
    let stale = settled
        .iter()
        .filter(|(name, _)| !unpinned.iter().any(|(dependency, _)| dependency.name == *name))
        .map(|(name, _)| {
            deps1(
                RULES_FILE,
                None,
                format!(
                    "DEPS-1: {RULES_FILE} lists {name} in SETTLED_CRATES, but no member takes it with a range below 1.0.  Fix: remove the entry."
                ),
            )
        });
    let bad_dates = settled
        .iter()
        .filter(|(_, date)| !is_date(date))
        .map(|(name, date)| {
            deps1(
                RULES_FILE,
                None,
                format!(
                    "DEPS-1: {RULES_FILE} gives {name} the date {date:?} in SETTLED_CRATES.  Fix: write the date of its first version on crates.io as YYYY-MM-DD."
                ),
            )
        });
    let unread = registry.is_empty().then(|| {
        deps1(
            "Cargo.toml",
            None,
            "DEPS-1: Cargo.toml gives no member with a registry dependency, so this check read nothing.  Fix: run the check on cargo metadata's output for the workspace.".into(),
        )
    });
    Ok(young
        .chain(not_registry)
        .chain(built_from_elsewhere)
        .chain(stale)
        .chain(bad_dates)
        .chain(unread)
        .collect())
}

const THIRD_PARTY: &str = "THIRD-PARTY.md";
const BROWSER_TESTS_HEADING: &str = "## Browser tests only";
const THIRD_PARTY_HEADER: &str = "| Crate | Version | Licence | Source |";

/// The `-e` values of the two commands in `THIRD-PARTY.md`: what ships, and
/// what ships plus the browser tests.
pub const THIRD_PARTY_EDGES: [&str; 2] = ["normal", "normal,dev"];

/// The args of a command in `THIRD-PARTY.md`, in its order, after `cargo`.
pub fn third_party_args(edges: &str) -> [&str; 15] {
    [
        "tree",
        "-p",
        "app",
        "-p",
        "stem-worker",
        "-p",
        "decode-worker",
        "--target",
        "wasm32-unknown-unknown",
        "-e",
        edges,
        "--prefix",
        "none",
        "-f",
        "{p}|{l}|{r}",
    ]
}

/// The command as `THIRD-PARTY.md` writes it.
fn third_party_command(edges: &str) -> String {
    let args = third_party_args(edges);
    let (format, rest) = args.split_last().expect("the args are not empty");
    format!("cargo {} \"{format}\"", rest.join(" "))
}

/// Runs a command of `THIRD-PARTY.md` in `root`, with `--locked` and
/// `--color never` added, as `web_app_tree` does.
pub fn third_party_tree(root: &Path, edges: &str) -> Result<String> {
    let [tree, rest @ ..] = third_party_args(edges);
    let output = Command::new(env!("CARGO"))
        .current_dir(root)
        .arg(tree)
        .args(["--locked", "--color", "never"])
        .args(rest)
        .output()
        .with_context(|| format!("running cargo tree -e {edges}"))?;
    anyhow::ensure!(
        output.status.success(),
        "cargo tree -e {edges} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .with_context(|| format!("cargo tree -e {edges} printed text that is not UTF-8"))
}

/// The folder of each workspace crate, from `cargo metadata`'s JSON, as
/// `cargo tree` prints it after a path crate's version.
pub fn member_folders(metadata: &str) -> Result<Vec<PathBuf>> {
    let metadata: Metadata =
        serde_json::from_str(metadata).context("reading cargo metadata's JSON")?;
    Ok(metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| folder_of(package).to_path_buf())
        .collect())
}

/// A crate in a table of `THIRD-PARTY.md`, or in a command's output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LicenceRow {
    name: String,
    version: String,
    licence: String,
    source: String,
}

impl LicenceRow {
    fn key(&self) -> (&str, &str) {
        (&self.name, &self.version)
    }

    fn text(&self) -> String {
        format!(
            "| {} | {} | {} | {} |",
            self.name, self.version, self.licence, self.source
        )
    }
}

/// The rows of a command's output, without workspace crates and repeats,
/// and the lines that the check cannot read, without their ` (*)`.
fn tree_rows<'a>(
    output: &'a str,
    members: &[PathBuf],
) -> (BTreeSet<LicenceRow>, BTreeSet<&'a str>) {
    let read: Vec<std::result::Result<Option<LicenceRow>, &str>> = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| tree_row(line, members).ok_or(line))
        .collect();
    let rows = read
        .iter()
        .filter_map(|row| row.as_ref().ok().cloned().flatten())
        .collect();
    let unread = read
        .iter()
        .filter_map(|row| row.as_ref().err())
        .map(|line| line.strip_suffix(" (*)").unwrap_or(line))
        .collect();
    (rows, unread)
}

/// The crate and version of a line that the check cannot read, if the line
/// gives them, so that its row is neither "add" nor "remove": the line's
/// own problem says what to do.
fn unread_crate(line: &str) -> Option<(&str, &str)> {
    let package = line.split('|').next()?;
    let package = package.strip_suffix(" (proc-macro)").unwrap_or(package);
    let package = package
        .split_once(" (")
        .map_or(package, |(package, _)| package);
    package.split_once(" v")
}

fn unread_line(line: &str) -> Problem {
    deps1(
        THIRD_PARTY,
        None,
        format!(
            "DEPS-1: {THIRD_PARTY} cannot be checked against the cargo tree line {line:?}.  Fix: if the crate comes from git, another registry or a vendored folder, ask the maintainer.  Otherwise add its rows by hand, and extend the parser in {RULES_FILE}."
        ),
    )
}

/// `Some(None)` for a workspace crate, `None` for a line it cannot read.
fn tree_row(line: &str, members: &[PathBuf]) -> Option<Option<LicenceRow>> {
    let cells: Vec<&str> = line.split('|').collect();
    let [package, licence, source] = cells.as_slice() else {
        return None;
    };
    let source = source.strip_suffix(" (*)").unwrap_or(source);
    let package = package.strip_suffix(" (proc-macro)").unwrap_or(package);
    let (package, origin) = match package.split_once(" (") {
        Some((package, origin)) => (package, Some(origin.strip_suffix(')')?)),
        None => (package, None),
    };
    let (name, version) = package.split_once(" v")?;
    let well_formed = !name.is_empty()
        && !name.contains(' ')
        && version.starts_with(|c: char| c.is_ascii_digit())
        && !version.contains(' ');
    if !well_formed {
        return None;
    }
    match origin {
        Some(origin) if members.iter().any(|member| member == Path::new(origin)) => Some(None),
        Some(_) => None,
        None => Some(Some(LicenceRow {
            name: name.into(),
            version: version.into(),
            licence: (*licence).into(),
            source: source.into(),
        })),
    }
}

/// The rows of the table in one section of `THIRD-PARTY.md`, with the line
/// of each, from 1.  `first_line` is the section's first line.
fn table_rows(section: &[&str], first_line: usize) -> (Vec<(usize, LicenceRow)>, Vec<Problem>) {
    let Some(header) = section.iter().position(|line| *line == THIRD_PARTY_HEADER) else {
        return (vec![], vec![]);
    };
    let read: Vec<(usize, Option<LicenceRow>)> = section
        .iter()
        .enumerate()
        .skip(header + 2)
        .take_while(|(_, line)| line.starts_with('|'))
        .map(|(index, line)| (first_line + index, doc_row(line)))
        .collect();
    let rows = read
        .iter()
        .filter_map(|(line, row)| Some((*line, row.clone()?)))
        .collect();
    let unread = read
        .iter()
        .filter(|(_, row)| row.is_none())
        .map(|(line, _)| {
            deps1(
                THIRD_PARTY,
                Some(*line),
                format!(
                    "DEPS-1: {THIRD_PARTY}:{line} is not a row of four cells.  Fix: write it as | Crate | Version | Licence | Source |."
                ),
            )
        })
        .collect();
    (rows, unread)
}

fn doc_row(line: &str) -> Option<LicenceRow> {
    let inner = line.strip_prefix("| ")?.strip_suffix(" |")?;
    let cells: Vec<&str> = inner.split(" | ").collect();
    let [name, version, licence, source] = cells.as_slice() else {
        return None;
    };
    Some(LicenceRow {
        name: (*name).into(),
        version: (*version).into(),
        licence: (*licence).into(),
        source: (*source).into(),
    })
}

/// DEPS-1: `THIRD-PARTY.md` lists what ships.  `normal` and
/// `normal_and_dev` are the output of its two commands
/// (`third_party_tree`).  Its first table must equal the first command's
/// rows; its second table, the second command's rows that are not in the
/// first.  `members` holds the folder of each workspace crate
/// (`member_folders`); a path crate in another folder, such as a vendored
/// copy, is a line that the check cannot read.
pub fn third_party_matches(
    doc: &str,
    normal: &str,
    normal_and_dev: &str,
    members: &[PathBuf],
) -> Vec<Problem> {
    let lines: Vec<&str> = doc.lines().collect();
    let split = lines
        .iter()
        .position(|line| *line == BROWSER_TESTS_HEADING)
        .unwrap_or(lines.len());
    let (first_section, second_section) = lines.split_at(split);
    let missing_commands = [
        (first_section, THIRD_PARTY_EDGES[0], "above"),
        (second_section, THIRD_PARTY_EDGES[1], "below"),
    ]
    .into_iter()
    .filter_map(|(section, edges, place)| {
        let command = third_party_command(edges);
        (!section.contains(&command.as_str())).then(|| {
            deps1(
                THIRD_PARTY,
                None,
                format!(
                    "DEPS-1: {THIRD_PARTY} lacks the command {command} {place} {BROWSER_TESTS_HEADING:?}.  Fix: restore the command; the check runs it."
                ),
            )
        })
    });
    let (shipped, shipped_unread) = tree_rows(normal, members);
    let (with_tests, with_tests_unread) = tree_rows(normal_and_dev, members);
    let shipped_keys: BTreeSet<(&str, &str)> = shipped.iter().map(LicenceRow::key).collect();
    let tests_only: BTreeSet<LicenceRow> = with_tests
        .iter()
        .filter(|row| !shipped_keys.contains(&row.key()))
        .cloned()
        .collect();
    let (first_table, first_unread) = table_rows(first_section, 1);
    let (second_table, second_unread) = table_rows(second_section, split + 1);
    let unread_crates: BTreeSet<(&str, &str)> = shipped_unread
        .union(&with_tests_unread)
        .filter_map(|line| unread_crate(line))
        .collect();
    let readable = |table: Vec<(usize, LicenceRow)>| -> Vec<(usize, LicenceRow)> {
        table
            .into_iter()
            .filter(|(_, row)| !unread_crates.contains(&row.key()))
            .collect()
    };
    let differences = table_differences(&readable(first_table), &shipped)
        .into_iter()
        .chain(table_differences(&readable(second_table), &tests_only));
    let no_licence = shipped
        .iter()
        .chain(&with_tests)
        .filter(|row| row.licence.is_empty())
        .map(LicenceRow::key)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|(name, version)| {
            deps1(
                THIRD_PARTY,
                None,
                format!(
                    "DEPS-1: {THIRD_PARTY} cannot credit {name} {version}, whose manifest gives no licence.  Fix: ask the maintainer; the check has no exception for a crate without a licence."
                ),
            )
        });
    let unread = (!["wgpu", "eframe"]
        .iter()
        .all(|known| shipped.iter().any(|row| row.name == *known)))
    .then(|| {
        deps1(
            THIRD_PARTY,
            None,
            format!(
                "DEPS-1: {THIRD_PARTY} cannot be checked: its first command gave no wgpu or no eframe, so the check read too little.  Fix: run the check on the output of {}.",
                third_party_command(THIRD_PARTY_EDGES[0])
            ),
        )
    });
    let unread_lines = shipped_unread
        .union(&with_tests_unread)
        .map(|line| unread_line(line));
    missing_commands
        .chain(unread_lines)
        .chain(first_unread)
        .chain(second_unread)
        .chain(differences)
        .chain(no_licence)
        .chain(unread)
        .collect()
}

/// The rows to add, remove or change for a table to equal `rows`, and the
/// rows it holds twice.
fn table_differences(table: &[(usize, LicenceRow)], rows: &BTreeSet<LicenceRow>) -> Vec<Problem> {
    let twice = table
        .iter()
        .enumerate()
        .filter(|(index, (_, row))| {
            table[..*index]
                .iter()
                .any(|(_, earlier)| earlier.key() == row.key())
        })
        .map(|(_, (line, row))| {
            deps1(
                THIRD_PARTY,
                Some(*line),
                format!(
                    "DEPS-1: {THIRD_PARTY}:{line} lists {} {} a second time.  Fix: remove the row.",
                    row.name, row.version
                ),
            )
        });
    let changed_or_stale = table
        .iter()
        .filter(|(line, row)| {
            table
                .iter()
                .find(|(_, first)| first.key() == row.key())
                .is_some_and(|(first_line, _)| first_line == line)
        })
        .filter_map(|(line, row)| {
            let message = match rows.iter().find(|tree| tree.key() == row.key()) {
                Some(tree) if tree == row => return None,
                Some(tree) => format!(
                    "DEPS-1: {THIRD_PARTY}:{line} gives {} {} the licence {:?} and source {:?}, but cargo tree gives {:?} and {:?}.  Fix: read the new licence; if it is not permissive, ask the maintainer; then update the row to {}.",
                    row.name, row.version, row.licence, row.source, tree.licence, tree.source, tree.text()
                ),
                None => format!(
                    "DEPS-1: {THIRD_PARTY}:{line} lists {} {}, which its command does not give.  Fix: remove the row {}.",
                    row.name, row.version, row.text()
                ),
            };
            Some(deps1(THIRD_PARTY, Some(*line), message))
        });
    let missing = rows
        .iter()
        .filter(|tree| !table.iter().any(|(_, row)| row.key() == tree.key()))
        .map(|tree| {
            deps1(
                THIRD_PARTY,
                None,
                format!(
                    "DEPS-1: {THIRD_PARTY} lacks {} {}, which its command gives.  Fix: add the row {}.",
                    tree.name, tree.version, tree.text()
                ),
            )
        });
    twice.chain(changed_or_stale).chain(missing).collect()
}

const PROTOCOL_LIB: &str = "protocol/src/lib.rs";
const E2E_SCRIPTS: &str = "xtask/e2e";
const E2E_RUST: &str = "xtask/src/e2e.rs";

/// Words in a message type's place in the e2e scripts that
/// `stem_protocol::kind` does not have, each with its reason.
pub const SCRIPT_ONLY_TYPES: [(&str, &str); 2] = [
    (
        "worker-error",
        "the reply that `e2e.ask` makes when the worker throws",
    ),
    (
        "module",
        "the `Worker` constructor's option in `worker.js`, not a message",
    ),
];

fn proto1(file: &str, line: Option<usize>, message: String) -> Problem {
    Problem {
        rule: "PROTO-1",
        file: file.into(),
        line,
        message,
    }
}

/// Each `*.js` file in `xtask/e2e`, in name order, with its text.
pub fn e2e_scripts(root: &Path) -> Result<Vec<(String, String)>> {
    let folder = root.join(E2E_SCRIPTS);
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .with_context(|| format!("reading {E2E_SCRIPTS}"))?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_>>()?;
    names.retain(|name| name.ends_with(".js"));
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let file = format!("{E2E_SCRIPTS}/{name}");
            let text = std::fs::read_to_string(folder.join(&name))
                .with_context(|| format!("reading {file}"))?;
            Ok((file, text))
        })
        .collect()
}

/// A JavaScript token, as far as the scan of message types needs one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum JsToken {
    Ident(String),
    Str(String),
    Punct(char),
}

fn is_js_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The tokens of `text`, each with its line from 1.  It skips comments and
/// template literals.  A string goes on past `\` and a newline, as in JS.
/// A string without its closing quote on its line gives no token, so
/// prose with an apostrophe gives no type, and the lex never fails.  It
/// reads no regex literal.
fn js_tokens(text: &str) -> Vec<(usize, JsToken)> {
    let chars: Vec<char> = text.chars().collect();
    let at = |index: usize| chars.get(index).copied();
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut index = 0;
    while let Some(c) = at(index) {
        if c == '\n' {
            line += 1;
            index += 1;
        } else if c.is_whitespace() {
            index += 1;
        } else if c == '/' && at(index + 1) == Some('/') {
            while at(index).is_some_and(|c| c != '\n') {
                index += 1;
            }
        } else if c == '/' && at(index + 1) == Some('*') {
            index += 2;
            while at(index).is_some() && !(at(index) == Some('*') && at(index + 1) == Some('/')) {
                line += usize::from(at(index) == Some('\n'));
                index += 1;
            }
            index += 2;
        } else if c == '`' {
            index += 1;
            while let Some(c) = at(index).filter(|c| *c != '`') {
                let skipped = if c == '\\' { 2 } else { 1 };
                line += (index..index + skipped)
                    .filter(|i| at(*i) == Some('\n'))
                    .count();
                index += skipped;
            }
            index += 1;
        } else if c == '"' || c == '\'' {
            let start_line = line;
            let mut value = String::new();
            let mut end = index + 1;
            let closed = loop {
                match at(end) {
                    None | Some('\n') => break false,
                    Some(quote) if quote == c => break true,
                    Some('\\') if at(end + 1).is_some() => {
                        line += usize::from(at(end + 1) == Some('\n'));
                        value.extend(at(end + 1));
                        end += 2;
                    }
                    Some(other) => {
                        value.push(other);
                        end += 1;
                    }
                }
            };
            if closed {
                tokens.push((start_line, JsToken::Str(value)));
                index = end + 1;
            } else {
                index = end;
            }
        } else if is_js_ident_char(c) {
            let start = index;
            while at(index).is_some_and(is_js_ident_char) {
                index += 1;
            }
            tokens.push((line, JsToken::Ident(chars[start..index].iter().collect())));
        } else {
            tokens.push((line, JsToken::Punct(c)));
            index += 1;
        }
    }
    tokens
}

/// The message types in `stem_protocol::kind` and `stem_protocol::decode::kind`:
/// every string in each brace group after `mod kind`, at the top level of
/// `protocol` (the text of one file, or of several joined).  The error says
/// why there is none.
fn protocol_kinds(protocol: &str) -> std::result::Result<BTreeSet<String>, String> {
    let tokens = TokenStream::from_str(protocol).map_err(|e| format!("does not tokenise: {e}"))?;
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let groups: Vec<TokenStream> = trees
        .windows(3)
        .filter_map(|window| match window {
            [
                TokenTree::Ident(word),
                TokenTree::Ident(name),
                TokenTree::Group(group),
            ] if word == "mod" && name == "kind" && group.delimiter() == Delimiter::Brace => {
                Some(group.stream())
            }
            _ => None,
        })
        .collect();
    if groups.is_empty() {
        return Err("has no `mod kind`".into());
    }
    let kinds: BTreeSet<String> = groups
        .into_iter()
        .flat_map(string_literals)
        .map(|(_, value)| value)
        .collect();
    if kinds.is_empty() {
        Err("has no string in `mod kind`".into())
    } else {
        Ok(kinds)
    }
}

fn js_ident(token: Option<&(usize, JsToken)>, name: &str) -> bool {
    matches!(token, Some((_, JsToken::Ident(ident))) if ident == name)
}

fn js_punct(token: Option<&(usize, JsToken)>, char: char) -> bool {
    matches!(token, Some((_, JsToken::Punct(punct))) if *punct == char)
}

fn js_str(token: Option<&(usize, JsToken)>) -> Option<(usize, String)> {
    match token {
        Some((line, JsToken::Str(value))) => Some((*line, value.clone())),
        _ => None,
    }
}

/// The index after `===`, `==`, `!==` or `!=` at `start`.
fn js_comparison_end(tokens: &[(usize, JsToken)], start: usize) -> Option<usize> {
    let run: String = tokens[start.min(tokens.len())..]
        .iter()
        .map_while(|(_, token)| match token {
            JsToken::Punct(c @ ('=' | '!')) => Some(*c),
            _ => None,
        })
        .collect();
    matches!(run.as_str(), "===" | "==" | "!==" | "!=").then_some(start + run.len())
}

/// The last ident of the chain `a.b.c` at `start`, and the index after it.
fn js_chain_end(tokens: &[(usize, JsToken)], start: usize) -> Option<(&str, usize)> {
    let Some((_, JsToken::Ident(first))) = tokens.get(start) else {
        return None;
    };
    let mut last = first.as_str();
    let mut end = start + 1;
    while let (true, Some((_, JsToken::Ident(next)))) =
        (js_punct(tokens.get(end), '.'), tokens.get(end + 1))
    {
        last = next;
        end += 2;
    }
    Some((last, end))
}

/// True when the chain that ends with the ident at `end` follows `typeof`.
fn js_after_typeof(tokens: &[(usize, JsToken)], end: usize) -> bool {
    let mut start = end;
    while start >= 2
        && js_punct(tokens.get(start - 1), '.')
        && matches!(tokens.get(start - 2), Some((_, JsToken::Ident(_))))
    {
        start -= 2;
    }
    start >= 1 && js_ident(tokens.get(start - 1), "typeof")
}

/// The strings of `[ "a", "b", ]` at `start`, if `.includes(<chain>.type)`
/// follows it.
fn js_included_types(tokens: &[(usize, JsToken)], start: usize) -> Vec<(usize, String)> {
    if !js_punct(tokens.get(start), '[') {
        return Vec::new();
    }
    let mut strings = Vec::new();
    let mut end = start + 1;
    while let Some(string) = js_str(tokens.get(end)) {
        strings.push(string);
        end += 1;
        if !js_punct(tokens.get(end), ',') {
            break;
        }
        end += 1;
    }
    let includes_a_type = !strings.is_empty()
        && js_punct(tokens.get(end), ']')
        && js_punct(tokens.get(end + 1), '.')
        && js_ident(tokens.get(end + 2), "includes")
        && js_punct(tokens.get(end + 3), '(')
        && js_chain_end(tokens, end + 4)
            .is_some_and(|(last, after)| last == "type" && js_punct(tokens.get(after), ')'));
    if includes_a_type { strings } else { Vec::new() }
}

/// Each message type in a script, with its line: `type: "x"` or
/// `"type": "x"`; `d.type === "x"` or `type === "x"`, and the reverse
/// order; and `["x", "y"].includes(d.type)`.
fn js_types(tokens: &[(usize, JsToken)]) -> Vec<(usize, String)> {
    (0..tokens.len())
        .flat_map(|index| {
            let token = tokens.get(index);
            let is_type_word = js_ident(token, "type");
            let is_type_key =
                is_type_word || js_str(token).is_some_and(|(_, value)| value == "type");
            let key_value = (is_type_key && js_punct(tokens.get(index + 1), ':'))
                .then(|| js_str(tokens.get(index + 2)))
                .flatten();
            let compared = (is_type_word && !js_after_typeof(tokens, index))
                .then(|| js_comparison_end(tokens, index + 1))
                .flatten()
                .and_then(|end| js_str(tokens.get(end)));
            let reversed = js_str(token).filter(|_| {
                js_comparison_end(tokens, index + 1)
                    .and_then(|end| js_chain_end(tokens, end))
                    .is_some_and(|(last, _)| last == "type")
            });
            key_value
                .into_iter()
                .chain(compared)
                .chain(reversed)
                .chain(js_included_types(tokens, index))
        })
        .collect()
}

/// `\\` as `\` and `\"` as `"`, so a plain Rust literal reads as the JS it
/// holds.  Other escapes stay, and so do the newlines.
fn unescape_quotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&next)) if next == '\\' || next == '"' => {
                out.push(next);
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

fn is_type_literal(tree: Option<&TokenTree>) -> bool {
    matches!(tree, Some(TokenTree::Literal(literal)) if string_value(literal).as_deref() == Some("type"))
}

fn rust_string(tree: Option<&TokenTree>) -> Option<(usize, String)> {
    match tree {
        Some(TokenTree::Literal(literal)) => {
            string_value(literal).map(|value| (literal.span().start().line, value))
        }
        _ => None,
    }
}

/// A string literal at `start`, or `json!("x")`.
fn rust_type_value(trees: &[TokenTree], start: usize) -> Option<(usize, String)> {
    rust_string(trees.get(start)).or_else(|| {
        let is_json = ident_name(trees.get(start)).as_deref() == Some("json")
            && punct_spacing(trees.get(start + 1), '!').is_some();
        match trees.get(start + 2) {
            Some(TokenTree::Group(group))
                if is_json && group.delimiter() == Delimiter::Parenthesis =>
            {
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                match inner.as_slice() {
                    [only] => rust_string(Some(only)),
                    _ => None,
                }
            }
            _ => None,
        }
    })
}

/// Each message type in Rust tokens, with its line: in each string literal,
/// read as JS; after `["type"] ==` or `!=`; and after `"type":` in a brace
/// group, as in `json!([{ "type": "x" }])`.
fn rust_types(tokens: TokenStream, in_braces: bool) -> Vec<(usize, String)> {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    trees
        .iter()
        .enumerate()
        .flat_map(|(index, tree)| match tree {
            TokenTree::Group(group) => {
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                let is_type_index = group.delimiter() == Delimiter::Bracket
                    && inner.len() == 1
                    && is_type_literal(inner.first());
                let compares = (joint(trees.get(index + 1), '=')
                    || joint(trees.get(index + 1), '!'))
                    && punct_spacing(trees.get(index + 2), '=').is_some();
                let compared = (is_type_index && compares)
                    .then(|| rust_type_value(&trees, index + 3))
                    .flatten();
                let nested = rust_types(group.stream(), group.delimiter() == Delimiter::Brace);
                compared.into_iter().chain(nested).collect::<Vec<_>>()
            }
            TokenTree::Literal(literal) => {
                let script = string_value(literal)
                    .map(|value| {
                        let first_line = literal.span().start().line;
                        let is_raw = literal.to_string().starts_with('r');
                        let text = if is_raw {
                            value
                        } else {
                            unescape_quotes(&value)
                        };
                        js_types(&js_tokens(&text))
                            .into_iter()
                            .map(|(line, value)| (first_line + line - 1, value))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let keyed = (in_braces
                    && is_type_literal(Some(tree))
                    && punct_spacing(trees.get(index + 1), ':').is_some())
                .then(|| rust_type_value(&trees, index + 2))
                .flatten();
                script.into_iter().chain(keyed).collect()
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) => Vec::new(),
        })
        .collect()
}

/// A message type in an e2e script.
struct ScriptType {
    file: String,
    line: usize,
    value: String,
}

/// PROTO-1: each message type that the e2e scripts write by hand is in
/// `stem_protocol::kind`, or `script_only` names it.  `scripts` holds the
/// `xtask/e2e/*.js` files; `e2e_rust` is the text of `xtask/src/e2e.rs`.
pub fn e2e_types_match_the_protocol(
    protocol: &str,
    scripts: &[(String, String)],
    e2e_rust: &str,
    script_only: &[(&str, &str)],
) -> Vec<Problem> {
    let kinds = match protocol_kinds(protocol) {
        Ok(kinds) => kinds,
        Err(why) => {
            return vec![proto1(
                PROTOCOL_LIB,
                None,
                format!(
                    "PROTO-1: {PROTOCOL_LIB} {why}, so the check has no message type.  Fix: keep the message types as string constants in `stem_protocol::kind`, or update the check in {RULES_FILE}."
                ),
            )];
        }
    };
    let in_js: Vec<ScriptType> = scripts
        .iter()
        .flat_map(|(file, text)| {
            js_types(&js_tokens(text))
                .into_iter()
                .map(move |(line, value)| ScriptType {
                    file: file.clone(),
                    line,
                    value,
                })
        })
        .collect();
    let no_type = |file: &str, place: &str| {
        proto1(
            file,
            None,
            format!(
                "PROTO-1: {file} has no message type in {place}.  Fix: write each type in a shape that the check reads, or update the check in {RULES_FILE}."
            ),
        )
    };
    let e2e_tokens = TokenStream::from_str(e2e_rust);
    let rust_was_read = e2e_tokens.is_ok();
    let (in_rust, rust_problems): (Vec<ScriptType>, Vec<Problem>) = match e2e_tokens {
        Err(e) => (
            Vec::new(),
            vec![proto1(
                E2E_RUST,
                None,
                format!(
                    "PROTO-1: {E2E_RUST} does not tokenise: {e}.  Fix: make the file valid Rust."
                ),
            )],
        ),
        Ok(tokens) => {
            let found: Vec<ScriptType> = rust_types(tokens, false)
                .into_iter()
                .map(|(line, value)| ScriptType {
                    file: E2E_RUST.into(),
                    line,
                    value,
                })
                .collect();
            let empty = found.is_empty().then(|| no_type(E2E_RUST, "its scripts"));
            (found, empty.into_iter().collect())
        }
    };
    let js_problems = in_js
        .is_empty()
        .then(|| no_type(E2E_SCRIPTS, "its `*.js` files"));
    let known = kinds
        .iter()
        .map(|kind| format!("\"{kind}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let is_script_only = |value: &str| script_only.iter().any(|(entry, _)| *entry == value);
    let unknown = in_js
        .iter()
        .chain(&in_rust)
        .filter(|found| !kinds.contains(&found.value) && !is_script_only(&found.value))
        .map(|found| {
            let ScriptType { file, line, value } = found;
            proto1(
                file,
                Some(*line),
                format!(
                    "PROTO-1: {file}:{line} uses the message type \"{value}\", which `stem_protocol::kind` does not have.  Fix: use a type from `kind` ({known}); or change `kind`, its encoder and its decoder together; or, for a type that only the scripts make or an option that is not a message, add it to `SCRIPT_ONLY_TYPES` in {RULES_FILE} with a reason."
                ),
            )
        });
    let stale = script_only.iter().filter_map(|(entry, _)| {
        let why = if kinds.contains(*entry) {
            "`stem_protocol::kind` has it"
        } else if rust_was_read && !in_js.iter().chain(&in_rust).any(|found| found.value == *entry) {
            "no e2e script uses it"
        } else {
            return None;
        };
        Some(proto1(
            RULES_FILE,
            None,
            format!(
                "PROTO-1: {RULES_FILE} lists \"{entry}\" in `SCRIPT_ONLY_TYPES`, but {why}.  Fix: remove the entry."
            ),
        ))
    });
    js_problems
        .into_iter()
        .chain(rust_problems)
        .chain(unknown)
        .chain(stale)
        .collect()
}

const SETTINGS_FILE: &str = ".claude/settings.json";

/// The deny rules that `.claude/settings.json` holds for GIT-1: an agent
/// never pushes to `main` or `gh-pages`, publishes the site or merges a PR
/// (decision 0010).  A push rule sees only a branch that the command names:
/// `*main` matches a command that ends in `main`, such as ` main`, `:main`,
/// `/main` or `+main`, and also refuses `fix-main`.  Each rule starts
/// with the program name, so a commit message or a grep that holds the
/// words is not refused on its own.  Claude Code matched a rule against a
/// whole pipeline in runs on 2026-10-21, so `cargo run --help | grep
/// publish` is refused; a chain with `&&` was matched part by part.  The `rtk` forms are the commands that the
/// maintainer's RTK hook makes, and that an agent can write itself.
pub const REQUIRED_DENY_RULES: [&str; 26] = [
    "Bash(git push *main)",
    "Bash(git push *main *)",
    "Bash(git push *gh-pages*)",
    "Bash(git push *--all*)",
    "Bash(git push *--mirror*)",
    "Bash(git push *--branches*)",
    "Bash(rtk git push *main)",
    "Bash(rtk git push *main *)",
    "Bash(rtk git push *gh-pages*)",
    "Bash(rtk git push *--all*)",
    "Bash(rtk git push *--mirror*)",
    "Bash(rtk git push *--branches*)",
    "Bash(rtk proxy git push *main)",
    "Bash(rtk proxy git push *main *)",
    "Bash(rtk proxy git push *gh-pages*)",
    "Bash(rtk proxy git push *--all*)",
    "Bash(rtk proxy git push *--mirror*)",
    "Bash(rtk proxy git push *--branches*)",
    "Bash(gh pr merge *)",
    "Bash(rtk gh pr merge *)",
    "Bash(rtk proxy gh pr merge *)",
    "Bash(cargo r* publish*)",
    "Bash(rtk cargo r* publish*)",
    "Bash(rtk proxy cargo r* publish*)",
    "Bash(target/*/xtask publish*)",
    "Bash(./target/*/xtask publish*)",
];

/// The top-level keys of `.claude/settings.json` that the check knows.
/// Claude Code drops a settings file with a value that its schema rejects,
/// deny rules included, so a key joins this list only after a run of M14's
/// step 3 (`docs/plans/completed/harness-phase-2.md`) shows the rules still
/// hold.
pub const SETTINGS_KEYS: [&str; 3] = ["$schema", "permissions", "hooks"];

/// GIT-1: `.claude/settings.json` denies each rule of
/// `REQUIRED_DENY_RULES`, and holds only keys that the check knows.  A file
/// of the wrong shape gives one problem.
pub fn claude_settings_problems(settings: &str) -> Vec<Problem> {
    let git1 = |message: String| Problem {
        rule: "GIT-1",
        file: SETTINGS_FILE.into(),
        line: None,
        message,
    };
    match deny_rules(settings) {
        Err(what) => vec![git1(format!(
            "GIT-1: {SETTINGS_FILE} {what}.  Fix: Claude Code drops a settings file that it cannot read or that its schema rejects, deny rules included.  Keep to the keys this check knows, or add a key to SETTINGS_KEYS in xtask/src/rules.rs after a run of M14's step 3 (docs/plans/completed/harness-phase-2.md) shows the rules still hold."
        ))],
        Ok(deny) => REQUIRED_DENY_RULES
            .iter()
            .filter(|rule| !deny.contains(**rule))
            .map(|rule| {
                git1(format!(
                    "GIT-1: {SETTINGS_FILE} does not deny `{rule}`.  Fix: put it back in permissions.deny; GIT-1 keeps these commands from agents."
                ))
            })
            .collect(),
    }
}

/// The rules in `permissions.deny`, or what is wrong with the file's shape.
fn deny_rules(settings: &str) -> Result<BTreeSet<String>, String> {
    let value: serde_json::Value =
        serde_json::from_str(settings).map_err(|e| format!("is not JSON ({e})"))?;
    let top = value
        .as_object()
        .ok_or_else(|| "is not a JSON object".to_string())?;
    if let Some(key) = top
        .keys()
        .find(|key| !SETTINGS_KEYS.contains(&key.as_str()))
    {
        return Err(format!(
            "has the key `{key}`, which the check does not know"
        ));
    }
    if top.get("$schema").is_some_and(|schema| !schema.is_string()) {
        return Err("has a `$schema` that is not a string".to_string());
    }
    let permissions = top
        .get("permissions")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "has no `permissions` object".to_string())?;
    if let Some(key) = permissions.keys().find(|key| *key != "deny") {
        return Err(format!(
            "has the key `permissions.{key}`, which the check does not know"
        ));
    }
    permissions
        .get("deny")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "has no `permissions.deny` list".to_string())?
        .iter()
        .map(|rule| {
            rule.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("has `{rule}` in `permissions.deny`, which is not a string"))
        })
        .collect()
}

/// The folder of the project subagents.
pub const AGENTS_FOLDER: &str = ".claude/agents/";

/// The end of every agent's frontmatter: the read-only guard on each Bash
/// call.  `onFailure: block` refuses the call when the guard cannot run.
pub const AGENT_HOOK_BLOCK: &str = r#"hooks:
  PreToolUse:
    - matcher: Bash
      hooks:
        - type: command
          command: '"$CLAUDE_PROJECT_DIR"/.claude/hooks/read-only-bash.sh'
          onFailure: block"#;

/// The keys of an agent's frontmatter, each present once, `hooks` last.
pub const AGENT_KEYS: [&str; 5] = ["name", "description", "tools", "model", "hooks"];

/// The tools that an agent may have.  None of them writes a file.
pub const READ_ONLY_TOOLS: [&str; 4] = ["Read", "Grep", "Glob", "Bash"];

/// What is wrong with an agent's `description`, which must be one plain
/// YAML scalar.
fn description_faults(description: &str) -> Vec<&'static str> {
    [
        (description.is_empty(), "is empty"),
        (
            !description.is_empty() && !description.starts_with(|c: char| c.is_ascii_uppercase()),
            "does not start with a capital letter",
        ),
        (
            description.contains(": ") || description.contains(":\t"),
            "holds a `:` before a space or a tab",
        ),
        (
            description.contains(" #") || description.contains("\t#"),
            "holds a blank before `#`",
        ),
        (description.contains(['\'', '"']), "holds a quote"),
        (description.ends_with(':'), "ends with `:`"),
    ]
    .into_iter()
    .filter_map(|(fault, what)| fault.then_some(what))
    .collect()
}

/// The agents rule (harness phase 3, M2): the frontmatter of
/// `.claude/agents/<file_name>` keeps the agent read only by its tools and
/// by its hook, and stays plain enough to read line by line.  `text` is the
/// text of the file.
pub fn agent_problems(file_name: &str, text: &str) -> Vec<Problem> {
    let file = format!("{AGENTS_FOLDER}{file_name}");
    let problem = |line: Option<usize>, what: String, fix: &str| Problem {
        rule: "agents",
        file: file.clone(),
        line,
        message: format!("agents: {file} {what}.  Fix: {fix}"),
    };
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    if lines.first() != Some(&"---") {
        return vec![problem(
            Some(1),
            "does not start with a `---` line".to_string(),
            "start the file with its frontmatter, between two `---` lines.",
        )];
    }
    let Some(close) = lines
        .iter()
        .skip(1)
        .position(|line| *line == "---")
        .map(|i| i + 1)
    else {
        return vec![problem(
            None,
            "has no `---` line that closes the frontmatter".to_string(),
            "close the frontmatter with a `---` line; Claude Code does not list an agent whose frontmatter does not parse.",
        )];
    };
    // Each line of the frontmatter, with its line number in the file.
    let frontmatter: Vec<(usize, &str)> = lines[1..close]
        .iter()
        .enumerate()
        .map(|(i, line)| (i + 2, *line))
        .collect();
    let nested = |line: &str| line.starts_with([' ', '\t', '-']);
    let keys: Vec<(usize, &str, &str)> = frontmatter
        .iter()
        .filter(|(_, line)| !line.is_empty() && !nested(line))
        .map(|(number, line)| {
            let (key, value) = line.split_once(':').unwrap_or((line, ""));
            (*number, key, value.trim())
        })
        .collect();
    let find = |key: &str| keys.iter().find(|(_, k, _)| *k == key).copied();
    // A key line is `key:` at its end, or `key: value`; YAML reads no other form
    // as a key, and Claude Code does not list the agent.
    let shapes = frontmatter
        .iter()
        .filter(|(_, line)| !line.is_empty() && !nested(line))
        .filter(|(_, line)| {
            line.split_once(':')
                .is_none_or(|(_, value)| !(value.is_empty() || value.starts_with(' ')))
        })
        .map(|(number, line)| {
            problem(
                Some(*number),
                format!("has the line `{line}`, which is not `key: value`"),
                "write each key as `key: value`, with one space after the colon, or as `key:` before a list or a block.",
            )
        });
    let keep_the_keys = "keep only `name`, `description`, `tools`, `model` and `hooks`, each once, with `hooks` last.";

    let unknown = keys
        .iter()
        .filter(|(_, key, _)| !AGENT_KEYS.contains(key))
        .map(|(number, key, _)| {
            problem(
                Some(*number),
                format!("has the key `{key}`, which the check does not allow"),
                keep_the_keys,
            )
        });
    let twice = keys
        .iter()
        .enumerate()
        .filter(|(i, (_, key, _))| keys[..*i].iter().any(|(_, earlier, _)| earlier == key))
        .map(|(_, (number, key, _))| {
            problem(
                Some(*number),
                format!("has the key `{key}` twice"),
                keep_the_keys,
            )
        });
    let missing = AGENT_KEYS
        .iter()
        .filter(|key| find(key).is_none())
        .map(|key| problem(None, format!("has no `{key}` key"), keep_the_keys));
    let stem = file_name.strip_suffix(".md").unwrap_or(file_name);
    let name = find("name")
        .filter(|(_, _, name)| *name != stem)
        .map(|(number, _, name)| {
            problem(
                Some(number),
                format!("has the name `{name}`, not `{stem}`"),
                "give the agent the name of its file; `xtask review` and the guide name each agent by its file.",
            )
        });
    let description = find("description")
        .into_iter()
        .flat_map(|(number, _, description)| {
            description_faults(description)
                .into_iter()
                .map(move |fault| (number, fault))
        })
        .map(|(number, fault)| {
            problem(
                Some(number),
                format!("has a `description` that {fault}"),
                "write the description on one line, as a plain YAML scalar: start it with a capital letter, and put no `: `, no ` #`, no quote and no `:` at its end in it.",
            )
        });
    let model = find("model")
        .filter(|(_, _, model)| *model != "inherit")
        .map(|(number, _, model)| {
            problem(
                Some(number),
                format!("has `model: {model}`, not `model: inherit`"),
                "set `model: inherit`; every agent takes the model of the main session.",
            )
        });
    let hooks_line = find("hooks").map(|(number, _, _)| number);
    let hooks_last = keys
        .last()
        .filter(|(_, key, _)| hooks_line.is_some() && *key != "hooks")
        .map(|(number, key, _)| {
            problem(
                Some(*number),
                format!("has the key `{key}` after `hooks`, but `hooks` must be the last key"),
                "put `hooks` last; the hook block runs to the closing `---`.",
            )
        });
    let tools = find("tools");
    // The `- item` lines of a `tools` list, below a `tools:` line with no value.
    let tool_items: Vec<(usize, &str)> = tools
        .filter(|(_, _, value)| value.is_empty())
        .map(|(number, _, _)| {
            frontmatter
                .iter()
                .skip_while(|(n, _)| *n <= number)
                .map_while(|(n, line)| {
                    line.trim_start()
                        .strip_prefix('-')
                        .map(|item| (*n, item.trim()))
                })
                .collect()
        })
        .unwrap_or_default();
    let named_tools: Vec<&str> = match tools {
        Some((_, _, value)) if !value.is_empty() => value
            .split(',')
            .map(str::trim)
            .filter(|tool| !tool.is_empty())
            .collect(),
        _ => tool_items.iter().map(|(_, tool)| *tool).collect(),
    };
    let tool_faults: Vec<String> = tools
        .map(|_| {
            let not_read_only = named_tools
                .iter()
                .filter(|tool| !READ_ONLY_TOOLS.contains(tool))
                .map(|tool| format!("has `{tool}` in `tools`, which is not a read-only tool"));
            let no_bash =
                (!named_tools.contains(&"Bash")).then(|| "has no `Bash` in `tools`".to_string());
            let searches = ["Grep", "Glob"]
                .into_iter()
                .filter(|tool| named_tools.contains(tool))
                .map(|tool| format!("has `{tool}` in `tools`, which the guard does not see"));
            not_read_only.chain(no_bash).chain(searches).collect()
        })
        .unwrap_or_default();
    let tools_line = tools.map(|(number, _, _)| number);
    let tool_problems = tool_faults.into_iter().map(|what| {
        problem(
            tools_line,
            what,
            "keep `tools: Read, Bash`; with Bash, the agent searches through Bash, and the guard sees each command.",
        )
    });
    let indented = frontmatter
        .iter()
        .filter(|(number, line)| {
            nested(line)
                && !tool_items.iter().any(|(item, _)| item == number)
                && hooks_line.is_none_or(|hooks| *number < hooks)
        })
        .map(|(number, _)| {
            problem(
                Some(*number),
                "has an indented line outside the `tools` list and the hook block".to_string(),
                "write each key on one line; a description that wraps can break the YAML.",
            )
        });
    let hook_block = hooks_line
        .filter(|hooks| {
            let block: Vec<&str> = frontmatter
                .iter()
                .filter(|(number, _)| number >= hooks)
                .map(|(_, line)| *line)
                .collect();
            block.join("\n") != AGENT_HOOK_BLOCK
        })
        .map(|hooks| {
            problem(
                Some(hooks),
                "has a hook block that is not `AGENT_HOOK_BLOCK`".to_string(),
                "copy AGENT_HOOK_BLOCK from xtask/src/rules.rs: the guard on each Bash call, with `onFailure: block`, and no other hook.",
            )
        });
    shapes
        .chain(unknown)
        .chain(twice)
        .chain(missing)
        .chain(name)
        .chain(description)
        .chain(model)
        .chain(hooks_last)
        .chain(tool_problems)
        .chain(indented)
        .chain(hook_block)
        .collect()
}

/// Files in `.claude/agents/` that are not agents and that Claude Code does not
/// load: the file that keeps the folder in git, and the one that Finder makes.
pub const IGNORED_IN_AGENTS: [&str; 2] = [".gitkeep", ".DS_Store"];

/// The agents rule for the whole of `.claude/agents/`.  `entries` holds each
/// entry that a walk finds there, into each subfolder: its path below the
/// folder, and its text if it is a file (`None` for a folder).  Claude Code
/// reads the subfolders too, and knows an agent only by its `name`.
pub fn agents_folder_problems(entries: &[(String, Option<String>)]) -> Vec<Problem> {
    let problem = |path: &str, what: String, fix: &str| Problem {
        rule: "agents",
        file: format!("{AGENTS_FOLDER}{path}"),
        line: None,
        message: format!("agents: {AGENTS_FOLDER}{path} {what}.  Fix: {fix}"),
    };
    let at_top = |path: &str| !path.contains('/');
    let not_at_top = entries
        .iter()
        .filter(|(path, text)| text.is_none() || !at_top(path))
        .map(|(path, text)| {
            problem(
                path,
                if text.is_none() { "is a subfolder" } else { "is in a subfolder" }.to_string(),
                "keep each agent at the top of the folder.  Claude Code reads the subfolders too, but no check reads them.",
            )
        });
    let files: Vec<(&str, &str)> = entries
        .iter()
        .filter_map(|(path, text)| text.as_deref().map(|text| (path.as_str(), text)))
        .collect();
    let others = files
        .iter()
        .filter(|(path, _)| at_top(path) && !path.ends_with(".md") && !IGNORED_IN_AGENTS.contains(path))
        .map(|(path, _)| {
            problem(
                path,
                "is not an agent file".to_string(),
                "remove it; the folder holds only `<name>.md` agent files, `.gitkeep` and `.DS_Store`.",
            )
        });
    let agents = files
        .iter()
        .filter(|(path, _)| at_top(path) && path.ends_with(".md"))
        .flat_map(|(path, text)| agent_problems(path, text));
    let names: Vec<(&str, &str)> = files
        .iter()
        .filter(|(path, _)| path.ends_with(".md"))
        .filter_map(|(path, text)| agent_name(text).map(|name| (*path, name)))
        .collect();
    let shared = names.iter().enumerate().filter_map(|(i, (path, name))| {
        names[..i]
            .iter()
            .find(|(_, earlier)| earlier == name)
            .map(|(earlier, _)| {
                problem(
                    path,
                    format!("has the name `{name}`, which {earlier} has too"),
                    "give each agent its own name.  Claude Code loads one file for a name, in the order that it reads them.",
                )
            })
    });
    not_at_top
        .chain(others)
        .chain(agents)
        .chain(shared)
        .collect()
}

/// The `name` in the frontmatter of an agent file, if it has one.
fn agent_name(text: &str) -> Option<&str> {
    let mut lines = text.lines();
    (lines.next()?.trim_end() == "---")
        .then(|| {
            lines
                .take_while(|line| line.trim_end() != "---")
                .find_map(|line| line.strip_prefix("name:"))
                .map(str::trim)
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_package_names_its_manifest() {
        for (package, manifest) in [
            ("app", "app/Cargo.toml"),
            ("stem-worker", "worker/Cargo.toml"),
            ("decode-worker", "decoder/Cargo.toml"),
            ("stem-protocol", "protocol/Cargo.toml"),
        ] {
            assert_eq!(manifest_of(package), manifest, "{package}");
        }
    }

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
        let current: Vec<(String, String)> = SKIP_ALLOWLIST
            .iter()
            .map(|(file, literal, _)| {
                let text = format!("fn f() {{ console_log!(\"{literal}\"); }}");
                (file.to_string(), text)
            })
            .collect();
        assert_eq!(allowlisted_skips_exist(&current), []);
        let (file, literal, _) = SKIP_ALLOWLIST[0];
        let mut reworded = current.clone();
        reworded[0].1 = "fn f() { console_log!(\"skipped: suspended\"); }".to_string();
        let problems = allowlisted_skips_exist(&reworded);
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
            vec![None; SKIP_ALLOWLIST.len()],
            "files that are not in the tree"
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
| Shells | `decode-worker` | `dsp`, `engine`, `stem-protocol` |
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
                krate(
                    "decode-worker",
                    "decoder/Cargo.toml",
                    &[
                        ("dsp", Normal),
                        ("engine", Normal),
                        ("stem-protocol", Normal),
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

    fn shared(page: &str, names: &str) -> Problem {
        layer1(
            "Cargo.toml",
            format!(
                "LAYER-1: {page} and stem-worker both depend on {names}, directly or through other workspace crates.  The page side and the worker side meet only through stem-protocol.  Fix: keep each crate on one side, and send what the other side needs as a message in stem-protocol."
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
        assert_problems(
            shells(&workspace),
            vec![shared("app", "roformer, roformer-gpu")],
        );
    }

    #[test]
    fn a_crate_reached_through_another_is_found() {
        let workspace = with_edges(today(), &[("engine", "roformer", Kind::Normal)]);
        assert_problems(
            shells(&workspace),
            vec![
                shared("app", "roformer"),
                shared("decode-worker", "roformer"),
            ],
        );
    }

    #[test]
    fn the_worker_side_is_followed_to_every_depth() {
        let workspace = with_edges(
            without_edge(today(), "stem-worker", "roformer"),
            &[("roformer", "dsp", Kind::Normal)],
        );
        assert_problems(
            shells(&workspace),
            vec![shared("app", "dsp"), shared("decode-worker", "dsp")],
        );
    }

    #[test]
    fn a_crate_reached_through_stem_protocol_is_found() {
        let workspace = with_edges(today(), &[("stem-protocol", "test-support", Kind::Normal)]);
        assert_problems(
            shells(&workspace),
            vec![
                shared("app", "test-support"),
                shared("decode-worker", "test-support"),
            ],
        );
    }

    #[test]
    fn a_shell_is_in_its_own_set() {
        let workspace = with_edges(today(), &[("stem-worker", "app", Kind::Normal)]);
        assert_problems(
            shells(&workspace),
            vec![
                shared("app", "app, dsp, engine, synth"),
                shared("decode-worker", "dsp, engine"),
            ],
        );
        let workspace = with_edges(today(), &[("app", "stem-worker", Kind::Normal)]);
        assert_problems(
            shells(&workspace),
            vec![shared("app", "roformer, roformer-gpu, stem-worker")],
        );
        let workspace = with_edges(today(), &[("decode-worker", "stem-worker", Kind::Normal)]);
        assert_problems(
            shells(&workspace),
            vec![shared(
                "decode-worker",
                "roformer, roformer-gpu, stem-worker",
            )],
        );
    }

    #[test]
    fn the_decode_worker_and_the_stem_worker_that_share_a_crate_are_a_problem() {
        let workspace = with_edges(today(), &[("decode-worker", "roformer", Kind::Normal)]);
        assert_problems(
            shells(&workspace),
            vec![shared("decode-worker", "roformer")],
        );
        let workspace = with_edges(today(), &[("stem-worker", "engine", Kind::Normal)]);
        assert_problems(
            shells(&workspace),
            vec![
                shared("app", "dsp, engine"),
                shared("decode-worker", "dsp, engine"),
            ],
        );
    }

    #[test]
    fn the_decode_worker_and_the_app_may_share_engine() {
        let workspace = today();
        let takes_engine = |name: &str| {
            workspace
                .crates
                .iter()
                .any(|c| c.name == name && c.edges.iter().any(|e| e.to == "engine"))
        };
        assert!(takes_engine("app") && takes_engine("decode-worker"));
        assert_problems(shells(&workspace), vec![]);
    }

    #[test]
    fn a_dev_or_build_edge_does_not_join_the_sides() {
        let workspace = with_edges(
            today(),
            &[
                ("app", "roformer", Kind::Dev),
                ("engine", "roformer", Kind::Dev),
                ("decode-worker", "roformer", Kind::Dev),
                ("app", "roformer", Kind::Build),
                ("engine", "roformer", Kind::Build),
                ("decode-worker", "roformer", Kind::Build),
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
        for shell in ["stem-worker", "app", "decode-worker"] {
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

    pub(super) fn id(name: &str, folder: &str) -> String {
        format!("path+file://{folder}#{name}@0.1.0")
    }

    pub(super) fn package(
        name: &str,
        folder: &str,
        source: Option<&str>,
        dependencies: Vec<Value>,
    ) -> Value {
        json!({
            "id": source.map_or_else(|| id(name, folder), |s| format!("{s}#{name}@0.1.0")),
            "name": name,
            "source": source,
            "manifest_path": format!("{folder}/Cargo.toml"),
            "dependencies": dependencies,
            "features": {},
        })
    }

    pub(super) fn path_dep(name: &str, path: &str, kind: Option<&str>) -> Value {
        json!({ "name": name, "source": null, "rename": null, "kind": kind, "target": null, "path": path })
    }

    fn source_dep(name: &str, source: &str) -> Value {
        json!({ "name": name, "source": source, "rename": null, "kind": null, "target": null })
    }

    /// The JSON, with a resolve node for each member (no features, no
    /// dependencies) and the `other_nodes`.
    pub(super) fn metadata(
        packages: Vec<Value>,
        members: &[String],
        other_nodes: Vec<Value>,
    ) -> String {
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

mod max;
mod mix;

pub use max::max;
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

#[cfg(test)]
mod tracked_tests {
    use super::*;

    fn tracked(paths: &[&str]) -> Vec<TrackedFile> {
        paths
            .iter()
            .map(|path| TrackedFile {
                path: path.to_string(),
                size: 1,
            })
            .collect()
    }

    fn texts(files: &[(&str, &str)]) -> Vec<(String, String)> {
        files
            .iter()
            .map(|(path, text)| (path.to_string(), text.to_string()))
            .collect()
    }

    /// Asserts one problem for each case, with `rule`, the file and the
    /// line, and a fix.
    pub(super) fn assert_one_each(rule: &str, cases: &[(&str, Vec<Problem>, Option<usize>)]) {
        for (file, problems, line) in cases {
            assert_eq!(problems.len(), 1, "{file}\n{problems:#?}");
            let problem = &problems[0];
            assert_eq!(problem.rule, rule, "{problem}");
            assert_eq!(&problem.file, file, "{problem}");
            assert_eq!(&problem.line, line, "{problem}");
            let at = line.map_or(file.to_string(), |line| format!("{file}:{line}"));
            assert!(
                problem.message.starts_with(&format!("{rule}: {at} ")),
                "{problem}"
            );
            assert!(problem.message.contains("Fix: "), "{problem}");
        }
    }

    #[test]
    fn ls_files_output_is_parsed() {
        assert_eq!(
            parse_ls_files("1376 .claude/rules/dsp.md\x0010 a b.txt\x00").unwrap(),
            [
                TrackedFile {
                    path: ".claude/rules/dsp.md".into(),
                    size: 1376,
                },
                TrackedFile {
                    path: "a b.txt".into(),
                    size: 10,
                },
            ]
        );
        assert_eq!(parse_ls_files("").unwrap(), []);
        let submodule = parse_ls_files("- sub\x00").unwrap_err();
        assert!(format!("{submodule:#}").contains("sub"), "{submodule:#}");
        let no_space = parse_ls_files("12\x00").unwrap_err();
        assert!(format!("{no_space:#}").contains("12"), "{no_space:#}");
    }

    #[test]
    fn build_files_are_chosen() {
        for path in [
            ".github/workflows/ci.yml",
            ".github/dependabot.yml",
            "app/Trunk.toml",
            "engine/build.rs",
            "roformer/build.rs",
            ".cargo/config.toml",
            ".cargo/config",
            "app/.cargo/config.toml",
            "app/.cargo/config",
        ] {
            assert!(is_build_file(path), "{path}");
        }
        for path in [
            "README.md",
            "xtask/src/serve.rs",
            "tools/roformer_export.py",
            "config.toml",
            "docs/.cargo.md",
        ] {
            assert!(!is_build_file(path), "{path}");
        }
    }

    #[test]
    fn python_files_are_named() {
        let cases: Vec<(&str, Vec<Problem>, Option<usize>)> = [
            "tools/x.py",
            "Tools/X.PY",
            "a.pyc",
            "a.ipynb",
            "requirements.txt",
            "pyproject.toml",
            "Pipfile",
            "uv.lock",
            "engine/tools/roformer_export.py",
            "tools/ROFORMER_EXPORT.PY",
        ]
        .into_iter()
        .map(|path| (path, python_in_the_build(&tracked(&[path]), &[]), None))
        .collect();
        assert_one_each("BUILD-1", &cases);
        assert_eq!(
            python_in_the_build(&tracked(&["tools/roformer_export.py"]), &[]),
            []
        );
    }

    #[test]
    fn python_in_a_build_file_is_named() {
        let ci = ".github/workflows/ci.yml";
        let cases: Vec<(&str, Vec<Problem>, Option<usize>)> = [
            (ci, "      - run: pip install numpy"),
            (ci, "      - uses: actions/setup-python@v5"),
            (ci, "      - name: Set up Python"),
            (ci, "      - run: uv sync"),
            (ci, "      - run: pipx run x"),
            (ci, "      - run: python -m pip install x"),
            (".github/dependabot.yml", "  - package-ecosystem: \"pip\""),
            ("app/Trunk.toml", "command = \"python3\""),
            ("engine/build.rs", "    Command::new(\"python\");"),
            (".cargo/config", "runner = \"pip3\""),
            (".cargo/config", "x = \"uvx ruff\""),
        ]
        .into_iter()
        .map(|(file, line)| {
            let text = format!("# line 1\n{line}\n# line 3\n");
            (
                file,
                python_in_the_build(&[], &texts(&[(file, &text)])),
                Some(2),
            )
        })
        .collect();
        assert_one_each("BUILD-1", &cases);
    }

    #[test]
    fn clean_build_files_pass() {
        let ci = "      - run: set -o pipefail\n      # the pipeline, a snippet, Pythonic\n";
        let build = "    let uv_scale = 2;\n";
        assert_eq!(
            python_in_the_build(
                &[],
                &texts(&[
                    (".github/workflows/ci.yml", ci),
                    ("engine/build.rs", build),
                    ("README.md", "pip install x\n")
                ])
            ),
            []
        );
    }

    #[test]
    fn a_python_server_is_named() {
        let server = concat!("python3 -m http", ".server 8080");
        let old_server = concat!("python -m SimpleHTTP", "Server");
        let ci_server = concat!("      - run: serve http", ".server");
        let cases: Vec<(&str, Vec<Problem>, Option<usize>)> = [
            ("README.md", server),
            ("notes/docs/plans/x.md", server),
            ("xtask/src/serve.rs", old_server),
            (".github/workflows/ci.yml", ci_server),
        ]
        .into_iter()
        .map(|(file, line)| {
            let text = format!("Serve it:\n{line}\n");
            (
                file,
                python_in_the_build(&[], &texts(&[(file, &text)])),
                Some(2),
            )
        })
        .collect();
        assert_one_each("BUILD-1", &cases);
        let history = format!("Serve it:\n{server}\n");
        assert_eq!(
            python_in_the_build(
                &[],
                &texts(&[
                    ("docs/plans/x.md", &history),
                    ("docs/decisions/x.md", &history),
                    ("xtask/src/serve.rs", "struct SimpleHttpServer;\n")
                ])
            ),
            []
        );
    }

    #[test]
    fn the_python_server_may_be_named_in_a_review_log() {
        let server = concat!("python3 -m http", ".server 8080");
        let log = format!("Serve it:\n{server}\n");
        assert_eq!(
            python_in_the_build(&[], &texts(&[("docs/reviews/x.md", &log)])),
            []
        );
    }

    #[test]
    fn assets_are_named() {
        let cases: Vec<(&str, Vec<Problem>, Option<usize>)> = [
            "song.mp3",
            "song.wav",
            "song.flac",
            "song.ogg",
            "song.opus",
            "song.m4a",
            "song.aac",
            "song.aif",
            "song.aiff",
            "a/Model.SafeTensors",
            "bs_roformer_sw.f32.safetensors",
            "model.ckpt",
            "model.pt",
            "model.pth",
            "weights.bin",
            "model.onnx",
            "x.npy",
            "x.npz",
            "model.gguf",
        ]
        .into_iter()
        .map(|path| (path, assets_in_the_index(&tracked(&[path])), None))
        .collect();
        assert_one_each("ASSET-1", &cases);
        assert_eq!(
            assets_in_the_index(&tracked(&[
                "engine/src/wav.rs",
                "xtask/src/bin/x.rs",
                "docs/prompt",
                "app/tests/snapshots/six_lanes.png",
            ])),
            []
        );
    }

    #[test]
    fn a_file_over_the_cap_is_named() {
        let file = |size| TrackedFile {
            path: "docs/big.txt".into(),
            size,
        };
        let over = assets_in_the_index(&[file(1_048_577)]);
        assert_one_each("ASSET-1", &[("docs/big.txt", over.clone(), None)]);
        assert!(over[0].message.contains("1048577"), "{}", over[0]);
        assert_eq!(assets_in_the_index(&[file(1_048_576)]), []);
    }

    #[test]
    fn a_snapshot_update_in_a_build_file_is_named() {
        let cases: Vec<(&str, Vec<Problem>, Option<usize>)> = [
            (
                ".github/workflows/ci.yml",
                "jobs:\n  test:\n    env:\n      UPDATE_SNAPSHOTS: \"1\"\n",
                4,
            ),
            (
                ".github/actions/test/action.yml",
                "runs:\n  steps:\n    - run: UPDATE_SNAPSHOTS=1 cargo test\n",
                3,
            ),
            (".cargo/config.toml", "[env]\nUPDATE_SNAPSHOTS = \"1\"\n", 2),
        ]
        .into_iter()
        .map(|(file, text, line)| (file, snapshot_updates(&texts(&[(file, text)])), Some(line)))
        .collect();
        assert_one_each("VERIFY-1", &cases);
        let two = snapshot_updates(&texts(&[(
            ".github/workflows/ci.yml",
            "env:\n  UPDATE_SNAPSHOTS: \"1\"\nrun: UPDATE_SNAPSHOTS=1 cargo test\n",
        )]));
        let lines: Vec<Option<usize>> = two.iter().map(|problem| problem.line).collect();
        assert_eq!(lines, [Some(2), Some(3)], "{two:#?}");
        assert_eq!(
            snapshot_updates(&texts(&[
                (
                    ".github/workflows/ci.yml",
                    "      # UPDATE_SNAPSHOTS made egui_kittest rewrite\n"
                ),
                ("app/build.rs", "    // UPDATE_SNAPSHOTS is not set here\n"),
                ("README.md", "run `UPDATE_SNAPSHOTS=1 cargo test`\n"),
                (
                    "app/tests/ui.rs",
                    "// UPDATE_SNAPSHOTS=1 rewrites\nlet x = \"UPDATE_SNAPSHOTS\";\n"
                ),
            ])),
            []
        );
    }

    #[test]
    fn tracked_texts_skips_what_has_no_text() {
        let root = std::env::temp_dir().join(format!("xtask-tracked-texts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("README.md"), "text\n").unwrap();
        std::fs::write(root.join("a.png"), [0x89, b'P', b'N', b'G', 0xff]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("docs", root.join("link")).unwrap();
        let texts = tracked_texts(
            &root,
            &tracked(&["README.md", "a.png", "deleted.md", "link", "docs"]),
        );
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(texts.unwrap(), [("README.md".into(), "text\n".into())]);
    }
}

#[cfg(test)]
mod deps_tests {
    use super::tests::{id, metadata, package, path_dep};
    use super::tracked_tests::assert_one_each;
    use super::*;
    use serde_json::{Value, json};

    // DEPS-1: the pins, over `cargo metadata`'s JSON in its real shape.

    const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";
    const SETTLED_LOG: [(&str, &str); 1] = [("log", "2014-12-13")];

    fn requirement(name: &str, req: &str) -> Value {
        json!({
            "name": name, "source": CRATES_IO, "req": req, "rename": null,
            "kind": null, "target": null, "optional": false,
        })
    }

    fn with(dependency: Value, key: &str, value: Value) -> Value {
        let mut dependency = dependency;
        dependency[key] = value;
        dependency
    }

    /// `app` takes `anyhow ^1`, and `engine` takes `engine_dependencies`, so
    /// a check that reads only the first member misses the cases.
    fn two_members(engine_dependencies: Vec<Value>, others: Vec<Value>) -> String {
        let app = package("app", "/x/app", None, vec![requirement("anyhow", "^1")]);
        let engine = package("engine", "/x/engine", None, engine_dependencies);
        let members = [id("app", "/x/app"), id("engine", "/x/engine")];
        metadata([vec![app, engine], others].concat(), &members, vec![])
    }

    fn pins(metadata: &str, settled: &[(&str, &str)]) -> Vec<Problem> {
        young_crates_are_pinned_in(metadata, settled).expect("the fixture is cargo metadata's JSON")
    }

    #[test]
    fn pins_and_settled_crates_pass() {
        let dependencies = vec![
            requirement("a", "=0.1.0"),
            requirement("b", "=0.1.0-alpha.1"),
            requirement("c", "^1"),
            requirement("d", "^1.0.107"),
            requirement("e", "1.*"),
            requirement("f", "1.2.*"),
            requirement("g", ">=1.0, <2"),
            requirement("h", ">0"),
            requirement("i", "~1"),
            requirement("j", "=1"),
            with(path_dep("dsp", "/x/dsp", None), "req", json!("^0.1.0")),
            requirement("log", "^0.4.34"),
        ];
        let outside = package(
            "outside",
            "/r/outside",
            Some(CRATES_IO),
            vec![requirement("wsola", "^0.1.0")],
        );
        let dsp = package("dsp", "/x/dsp", None, vec![]);
        let app = package("app", "/x/app", None, dependencies);
        let members = [id("app", "/x/app"), id("dsp", "/x/dsp")];
        let metadata = metadata(vec![app, dsp, outside], &members, vec![]);
        assert_eq!(pins(&metadata, &SETTLED_LOG), []);
    }

    #[test]
    fn a_young_crate_with_a_range_is_named() {
        let requirements = [
            "^0.1.0",
            "^0.0.41",
            "*",
            "<1",
            "<2.0",
            "0.*",
            "^0",
            ">0.1",
            ">0.0.1",
            ">=0.1",
            ">=0.1, <0.3",
            "~0.1.2",
            "=0.1",
            "^1.0.0-rc.1",
            ">=1.0.0-rc.1, <2",
            "=0.1.x",
            "=0.1.0-a, <0.2",
        ];
        let shapes: [fn(Value) -> Value; 6] = [
            |dependency| dependency,
            |dependency| with(dependency, "kind", json!("dev")),
            |dependency| with(dependency, "kind", json!("build")),
            |dependency| with(dependency, "target", json!("cfg(target_arch = \"wasm32\")")),
            |dependency| {
                with(
                    dependency,
                    "source",
                    json!("sparse+https://index.crates.io/"),
                )
            },
            |dependency| with(dependency, "rename", json!("young_crate")),
        ];
        for (index, req) in requirements.into_iter().enumerate() {
            let shape = shapes[index % shapes.len()];
            let problems = pins(
                &two_members(vec![shape(requirement("young-crate", req))], vec![]),
                &SETTLED_LOG[..0],
            );
            assert!(
                problems.first().is_some_and(|problem| problem
                    .message
                    .contains(&format!("young-crate {req},"))),
                "{req}: {problems:#?}"
            );
            assert_one_each("DEPS-1", &[("engine/Cargo.toml", problems, None)]);
        }
    }

    #[test]
    fn a_source_that_is_not_a_registry_is_named() {
        let git = "git+https://github.com/acme/wsola?branch=main#0123abc";
        let cases = [
            two_members(
                vec![with(requirement("wsola", "*"), "source", json!(git))],
                vec![],
            ),
            two_members(
                vec![with(
                    requirement("wsola", "^0.1.0"),
                    "source",
                    json!("path+file:///r/wsola"),
                )],
                vec![],
            ),
        ];
        for metadata in &cases {
            assert_one_each(
                "DEPS-1",
                &[("engine/Cargo.toml", pins(metadata, &[]), None)],
            );
        }
        let patched = two_members(
            vec![requirement("wsola", "=0.1.0")],
            vec![package("wsola", "/r/wsola", Some(git), vec![])],
        );
        let problems = pins(&patched, &[]);
        assert!(problems[0].message.contains(git), "{problems:#?}");
        assert_one_each("DEPS-1", &[("Cargo.toml", problems, None)]);
    }

    #[test]
    fn a_stale_settled_entry_is_named() {
        let unused = pins(&two_members(vec![], vec![]), &SETTLED_LOG);
        let pinned = pins(
            &two_members(vec![requirement("log", "=0.4.34")], vec![]),
            &SETTLED_LOG,
        );
        let bad_date = pins(
            &two_members(vec![requirement("log", "^0.4.34")], vec![]),
            &[("log", "2024-1-5")],
        );
        assert!(bad_date[0].message.contains("2024-1-5"), "{bad_date:#?}");
        let long_date = pins(
            &two_members(vec![requirement("log", "^0.4.34")], vec![]),
            &[("log", "2024-01-055")],
        );
        assert_one_each(
            "DEPS-1",
            &[
                (RULES_FILE, unused, None),
                (RULES_FILE, pinned, None),
                (RULES_FILE, bad_date, None),
                (RULES_FILE, long_date, None),
            ],
        );
    }

    #[test]
    fn nothing_to_read_is_a_problem() {
        let no_member = metadata(vec![], &[], vec![]);
        let no_registry = metadata(
            vec![package("dsp", "/x/dsp", None, vec![])],
            &[id("dsp", "/x/dsp")],
            vec![],
        );
        assert_one_each(
            "DEPS-1",
            &[
                ("Cargo.toml", pins(&no_member, &[]), None),
                ("Cargo.toml", pins(&no_registry, &[]), None),
            ],
        );
        let git_only = metadata(
            vec![package(
                "app",
                "/x/app",
                None,
                vec![with(
                    requirement("wsola", "*"),
                    "source",
                    json!("git+https://github.com/acme/wsola#0123abc"),
                )],
            )],
            &[id("app", "/x/app")],
            vec![],
        );
        let problems = pins(&git_only, &[]);
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(
            problems[1].message.contains("so this check read nothing"),
            "{problems:#?}"
        );
    }

    // DEPS-1: the licence list, over `cargo tree`'s output in its real shape.

    fn members() -> Vec<PathBuf> {
        ["/root/app", "/root/roformer", "/root/worker"]
            .map(PathBuf::from)
            .to_vec()
    }

    const SHIPPED: &str = "\
app v0.1.0 (/root/app)||
eframe v0.36.2|MIT OR Apache-2.0|https://github.com/emilk/egui
epaint_default_fonts v0.36.2|(MIT OR Apache-2.0) AND OFL-1.1|https://github.com/emilk/egui
hashbrown v0.16.1|MIT OR Apache-2.0|https://github.com/rust-lang/hashbrown
hashbrown v0.17.1|MIT OR Apache-2.0|https://github.com/rust-lang/hashbrown (*)
roformer v0.1.0 (/root/roformer) (proc-macro)|MIT OR Apache-2.0| (*)
serde_derive v1.0.229 (proc-macro)|MIT OR Apache-2.0|https://github.com/serde-rs/serde
unsourced v1.0.0|MIT|
wgpu v30.0.1|MIT OR Apache-2.0|https://github.com/gfx-rs/wgpu

stem-worker v0.1.0 (/root/worker)|MIT OR Apache-2.0|
wgpu v30.0.1|MIT OR Apache-2.0|https://github.com/gfx-rs/wgpu (*)
";

    const TESTS_ONLY: &str = "wasm-bindgen-test v0.3.79|MIT OR Apache-2.0|https://github.com/wasm-bindgen/wasm-bindgen\n";

    const SHIPPED_ROWS: [&str; 7] = [
        "| eframe | 0.36.2 | MIT OR Apache-2.0 | https://github.com/emilk/egui |",
        "| epaint_default_fonts | 0.36.2 | (MIT OR Apache-2.0) AND OFL-1.1 | https://github.com/emilk/egui |",
        "| hashbrown | 0.16.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |",
        "| hashbrown | 0.17.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |",
        "| serde_derive | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |",
        "| unsourced | 1.0.0 | MIT |  |",
        "| wgpu | 30.0.1 | MIT OR Apache-2.0 | https://github.com/gfx-rs/wgpu |",
    ];

    const TESTS_ONLY_ROWS: [&str; 1] = [
        "| wasm-bindgen-test | 0.3.79 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen |",
    ];

    fn row(name: &str, version: &str, licence: &str, source: &str) -> LicenceRow {
        LicenceRow {
            name: name.into(),
            version: version.into(),
            licence: licence.into(),
            source: source.into(),
        }
    }

    /// A `THIRD-PARTY.md` in the shape of the real one.
    fn third_party_doc(commands: [&str; 2], first: &[&str], second: &[&str]) -> String {
        let table = |rows: &[&str]| {
            [THIRD_PARTY_HEADER, "| - | - | - | - |"]
                .iter()
                .chain(rows)
                .copied()
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "# Third-party software\n\nRegenerate the table with:\n\n```bash\n{}\n```\n\n{}\n\n{BROWSER_TESTS_HEADING}\n\nRegenerate the table with:\n\n```bash\n{}\n```\n\n{}\n",
            commands[0],
            table(first),
            commands[1],
            table(second)
        )
    }

    fn commands() -> [String; 2] {
        THIRD_PARTY_EDGES.map(third_party_command)
    }

    fn check(doc: &str, shipped: &str, tests_only: &str) -> Vec<Problem> {
        third_party_matches(doc, shipped, &format!("{shipped}{tests_only}"), &members())
    }

    fn today_doc() -> String {
        let [normal, dev] = commands();
        third_party_doc([&normal, &dev], &SHIPPED_ROWS, &TESTS_ONLY_ROWS)
    }

    /// One DEPS-1 problem with `THIRD-PARTY.md`, that holds `part`.
    fn assert_one_drift(problems: &[Problem], part: &str) {
        assert_eq!(problems.len(), 1, "{part}: {problems:#?}");
        let problem = &problems[0];
        assert_eq!(
            (problem.rule, problem.file.as_str()),
            ("DEPS-1", THIRD_PARTY)
        );
        let at = problem.line.map_or(THIRD_PARTY.to_string(), |line| {
            format!("{THIRD_PARTY}:{line}")
        });
        assert!(
            problem.message.starts_with(&format!("DEPS-1: {at} ")),
            "{problem}"
        );
        assert!(problem.message.contains(part), "{part}: {problem}");
        assert!(problem.message.contains("Fix: "), "{problem}");
    }

    #[test]
    fn member_folders_leave_out_a_vendored_crate() {
        let metadata = metadata(
            vec![
                package("app", "/x/app", None, vec![]),
                package("log", "/x/vendor/log", None, vec![]),
            ],
            &[id("app", "/x/app")],
            vec![],
        );
        assert_eq!(
            member_folders(&metadata).unwrap(),
            [PathBuf::from("/x/app")]
        );
    }

    #[test]
    fn the_commands_are_the_docs() {
        assert_eq!(
            commands(),
            [
                "cargo tree -p app -p stem-worker -p decode-worker --target wasm32-unknown-unknown -e normal --prefix none -f \"{p}|{l}|{r}\"",
                "cargo tree -p app -p stem-worker -p decode-worker --target wasm32-unknown-unknown -e normal,dev --prefix none -f \"{p}|{l}|{r}\"",
            ]
        );
    }

    #[test]
    fn third_party_rows_are_read() {
        let (rows, unread) = tree_rows(SHIPPED, &members());
        assert!(unread.is_empty(), "{unread:?}");
        let names: Vec<(&str, &str)> = rows.iter().map(LicenceRow::key).collect();
        assert_eq!(
            names,
            [
                ("eframe", "0.36.2"),
                ("epaint_default_fonts", "0.36.2"),
                ("hashbrown", "0.16.1"),
                ("hashbrown", "0.17.1"),
                ("serde_derive", "1.0.229"),
                ("unsourced", "1.0.0"),
                ("wgpu", "30.0.1"),
            ]
        );
        assert!(rows.contains(&row(
            "epaint_default_fonts",
            "0.36.2",
            "(MIT OR Apache-2.0) AND OFL-1.1",
            "https://github.com/emilk/egui"
        )));
        assert!(rows.contains(&row("unsourced", "1.0.0", "MIT", "")));
        assert!(rows.contains(&row(
            "hashbrown",
            "0.17.1",
            "MIT OR Apache-2.0",
            "https://github.com/rust-lang/hashbrown"
        )));
        for line in [
            "x v0.1.0 (https://github.com/acme/x#0123abc)|MIT|https://github.com/acme/x",
            "x v0.1.0 (registry `acme`)|MIT|https://github.com/acme/x",
            "x v0.1.0 (/elsewhere/x)|MIT|",
            "x v0.1.0 (/root-other/x)|MIT|",
            "log v0.4.34 (/root/vendor/log)|MIT OR Apache-2.0|https://github.com/rust-lang/log",
            "log v0.4.34 (/root/app/vendor/log)|MIT OR Apache-2.0|https://github.com/rust-lang/log",
            "foo v1.0.0|MIT",
            "x v0.1.0 extra|MIT|",
            "garbage",
        ] {
            let (rows, unread) = tree_rows(line, &members());
            assert_eq!(rows, BTreeSet::new(), "{line}");
            let problems: Vec<Problem> = unread.into_iter().map(unread_line).collect();
            assert_one_drift(&problems, "cannot be checked against the cargo tree line");
        }
        let (rows, unread) = table_rows(&today_doc().lines().collect::<Vec<_>>(), 1);
        assert_eq!(unread, []);
        assert_eq!(rows.len(), SHIPPED_ROWS.len());
    }

    #[test]
    fn the_third_party_tables_match() {
        assert_eq!(check(&today_doc(), SHIPPED, TESTS_ONLY), []);
    }

    #[test]
    fn a_third_party_drift_is_named() {
        let [normal, dev] = commands();
        let doc = |first: &[&str], second: &[&str]| third_party_doc([&normal, &dev], first, second);
        let without = |rows: &[&'static str], left_out: &str| -> Vec<&'static str> {
            rows.iter()
                .copied()
                .filter(|row| !row.contains(left_out))
                .collect()
        };
        let with_rows = |rows: &[&'static str], added: &[&'static str]| -> Vec<&'static str> {
            rows.iter().chain(added).copied().collect()
        };
        let replaced = |rows: &[&'static str], from: &str, to: &str| -> Vec<String> {
            rows.iter().map(|row| row.replace(from, to)).collect()
        };
        let doc_of = |first: &[String]| {
            let first: Vec<&str> = first.iter().map(String::as_str).collect();
            doc(&first, &TESTS_ONLY_ROWS)
        };
        let cases: Vec<(String, String, &str)> = vec![
            (
                doc(&without(&SHIPPED_ROWS, "0.16.1"), &TESTS_ONLY_ROWS),
                SHIPPED.into(),
                "lacks hashbrown 0.16.1",
            ),
            (
                doc(
                    &with_rows(&SHIPPED_ROWS, &["| zzz | 1.0.0 | MIT | https://zzz |"]),
                    &TESTS_ONLY_ROWS,
                ),
                SHIPPED.into(),
                "Fix: remove the row | zzz",
            ),
            (
                doc_of(&replaced(
                    &SHIPPED_ROWS,
                    "| 0.36.2 | MIT OR Apache-2.0 |",
                    "| 0.36.2 | MIT |",
                )),
                SHIPPED.into(),
                "gives eframe 0.36.2 the licence \"MIT\"",
            ),
            (
                doc_of(&replaced(
                    &SHIPPED_ROWS,
                    "https://github.com/serde-rs/serde",
                    "https://acme",
                )),
                SHIPPED.into(),
                "gives serde_derive 1.0.229 the licence",
            ),
            (
                doc(
                    &with_rows(&SHIPPED_ROWS, &[SHIPPED_ROWS[0]]),
                    &TESTS_ONLY_ROWS,
                ),
                SHIPPED.into(),
                "lists eframe 0.36.2 a second time",
            ),
            (
                doc(
                    &with_rows(&SHIPPED_ROWS, &TESTS_ONLY_ROWS),
                    &TESTS_ONLY_ROWS,
                ),
                SHIPPED.into(),
                "lists wasm-bindgen-test 0.3.79, which its command does not give",
            ),
            (
                third_party_doc(["", &dev], &SHIPPED_ROWS, &TESTS_ONLY_ROWS),
                SHIPPED.into(),
                "-e normal --prefix",
            ),
            (
                third_party_doc([&normal, ""], &SHIPPED_ROWS, &TESTS_ONLY_ROWS),
                SHIPPED.into(),
                "-e normal,dev --prefix",
            ),
            (
                third_party_doc(
                    ["", &format!("{normal}\n{dev}")],
                    &SHIPPED_ROWS,
                    &TESTS_ONLY_ROWS,
                ),
                SHIPPED.into(),
                "-e normal --prefix none -f \"{p}|{l}|{r}\" above",
            ),
            (
                doc(
                    &with_rows(&SHIPPED_ROWS, &["| nameless | 1.0.0 |  | https://x |"]),
                    &TESTS_ONLY_ROWS,
                ),
                format!("{SHIPPED}nameless v1.0.0||https://x\n"),
                "cannot credit nameless 1.0.0",
            ),
            (doc(&[], &[]), String::new(), "gave no wgpu or no eframe"),
            (
                doc(&without(&SHIPPED_ROWS, "wgpu"), &TESTS_ONLY_ROWS),
                SHIPPED
                    .lines()
                    .filter(|line| !line.contains("wgpu"))
                    .map(|line| format!("{line}\n"))
                    .collect(),
                "gave no wgpu or no eframe",
            ),
            (
                doc(&without(&SHIPPED_ROWS, "eframe"), &TESTS_ONLY_ROWS),
                SHIPPED.replace(
                    "eframe v0.36.2|MIT OR Apache-2.0|https://github.com/emilk/egui\n",
                    "",
                ),
                "gave no wgpu or no eframe",
            ),
        ];
        for (doc, shipped, part) in &cases {
            let tests_only = if shipped.is_empty() { "" } else { TESTS_ONLY };
            assert_one_drift(&check(doc, shipped, tests_only), part);
        }
        let vendored =
            "log v0.4.34 (/root/vendor/log)|MIT OR Apache-2.0|https://github.com/rust-lang/log";
        assert_one_drift(
            &check(
                &today_doc(),
                &format!("{SHIPPED}{vendored}\n{vendored} (*)\n"),
                TESTS_ONLY,
            ),
            "(/root/vendor/log)",
        );
        let log_row = "| log | 0.4.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/log |";
        assert_one_drift(
            &check(
                &doc(&with_rows(&SHIPPED_ROWS, &[log_row]), &TESTS_ONLY_ROWS),
                &format!("{SHIPPED}{vendored}\n"),
                TESTS_ONLY,
            ),
            "(/root/vendor/log)",
        );
        assert_one_drift(
            &check(
                &doc(&SHIPPED_ROWS, &with_rows(&TESTS_ONLY_ROWS, &[log_row])),
                SHIPPED,
                &format!("{TESTS_ONLY}{vendored}\n"),
            ),
            "(/root/vendor/log)",
        );
        assert_one_drift(
            &third_party_matches(
                &today_doc(),
                &format!("{SHIPPED}{vendored}\n"),
                &format!("{SHIPPED}{TESTS_ONLY}"),
                &members(),
            ),
            "(/root/vendor/log)",
        );
        let second = |rows: &[&str]| doc(&SHIPPED_ROWS, rows);
        assert_one_drift(
            &check(&second(&[]), SHIPPED, TESTS_ONLY),
            "lacks wasm-bindgen-test 0.3.79",
        );
        let stale_second = second(&[TESTS_ONLY_ROWS[0], "| zzz | 1.0.0 | MIT | https://zzz |"]);
        let zzz_line = stale_second
            .lines()
            .position(|line| line.starts_with("| zzz |"))
            .map(|index| index + 1);
        let problems = check(&stale_second, SHIPPED, TESTS_ONLY);
        assert_one_drift(&problems, "Fix: remove the row | zzz");
        assert_eq!(problems[0].line, zzz_line);
        assert_one_drift(
            &check(
                &second(&[TESTS_ONLY_ROWS[0], "| nameless | 1.0.0 |  | https://x |"]),
                SHIPPED,
                &format!("{TESTS_ONLY}nameless v1.0.0||https://x\n"),
            ),
            "cannot credit nameless 1.0.0",
        );
        assert_one_drift(
            &check(
                &third_party_doc(
                    [&format!("{normal}\n{dev}"), ""],
                    &SHIPPED_ROWS,
                    &TESTS_ONLY_ROWS,
                ),
                SHIPPED,
                TESTS_ONLY,
            ),
            "-e normal,dev --prefix none -f \"{p}|{l}|{r}\" below",
        );
        let stale_twice = check(
            &doc(
                &with_rows(&SHIPPED_ROWS, &["| zzz | 1.0.0 | MIT | https://zzz |"; 2]),
                &TESTS_ONLY_ROWS,
            ),
            SHIPPED,
            TESTS_ONLY,
        );
        assert_eq!(stale_twice.len(), 2, "{stale_twice:#?}");
        let no_wgpu: String = SHIPPED
            .lines()
            .filter(|line| !line.contains("wgpu"))
            .map(|line| format!("{line}\n"))
            .collect();
        let wgpu_in_tests_only = third_party_matches(
            &doc(&without(&SHIPPED_ROWS, "wgpu"), &TESTS_ONLY_ROWS),
            &no_wgpu,
            &format!(
                "{no_wgpu}{TESTS_ONLY}wgpu v30.0.1|MIT OR Apache-2.0|https://github.com/gfx-rs/wgpu\n"
            ),
            &members(),
        );
        assert!(
            wgpu_in_tests_only
                .iter()
                .any(|problem| problem.message.contains("gave no wgpu or no eframe")),
            "{wgpu_in_tests_only:#?}"
        );
        let new_version = check(
            &today_doc(),
            &SHIPPED.replace("eframe v0.36.2", "eframe v0.37.0"),
            TESTS_ONLY,
        );
        assert_eq!(new_version.len(), 2, "{new_version:#?}");
        assert!(
            new_version
                .iter()
                .any(|problem| problem.message.contains("lacks eframe 0.37.0"))
        );
        assert!(new_version.iter().any(|problem| {
            problem
                .message
                .contains("lists eframe 0.36.2, which its command does not give")
        }));
    }
}

#[cfg(test)]
mod proto_tests {
    use super::tracked_tests::assert_one_each;
    use super::*;

    // PROTO-1: the message types in the e2e scripts.

    /// `field` comes first, so a reader of the first `mod` fails.
    const PROTOCOL: &str = r#"
mod field {
    pub const TEXT: &str = "text";
}

/// The `type` of each message.
pub mod kind {
    /// Sent once the worker listens.
    pub const ALIVE: &str = "alive";
    pub const SEGMENT: &str = "segment";
    pub const DONE: &str = "done";
}
"#;
    const WORKER_JS: &str = "xtask/e2e/worker.js";
    /// One type in each source, so neither is empty.
    const JS_OK: &str = "w.onmessage = (e) => { if (e.data.type === \"alive\") go(); };";
    const RUST_OK: &str = "fn ok() { in_page(\"return e2e.ask([{ type: 'done' }]);\"); }";

    fn check(js: &str, rust: &str, script_only: &[(&str, &str)]) -> Vec<Problem> {
        e2e_types_match_the_protocol(
            PROTOCOL,
            &[(WORKER_JS.to_string(), js.to_string())],
            rust,
            script_only,
        )
    }

    fn values(tokens: &[(usize, JsToken)]) -> Vec<(usize, &JsToken)> {
        tokens.iter().map(|(line, token)| (*line, token)).collect()
    }

    #[test]
    fn js_is_lexed() {
        let text = "a.type === 'x'\n// type: 'y'\n\"it\\\"s\" /* type: 'z'\n */ b `t\\` ${ \"w\" }\n` c page's\ntypeof d 'e\\\nf' g";
        let ident = |name: &str| JsToken::Ident(name.into());
        let string = |value: &str| JsToken::Str(value.into());
        assert_eq!(
            values(&js_tokens(text)),
            vec![
                (1, &ident("a")),
                (1, &JsToken::Punct('.')),
                (1, &ident("type")),
                (1, &JsToken::Punct('=')),
                (1, &JsToken::Punct('=')),
                (1, &JsToken::Punct('=')),
                (1, &string("x")),
                (3, &string("it\"s")),
                (4, &ident("b")),
                (5, &ident("c")),
                (5, &ident("page")),
                (6, &ident("typeof")),
                (6, &ident("d")),
                (6, &string("e\nf")),
                (7, &ident("g")),
            ]
        );
    }

    #[test]
    fn the_kinds_are_read() {
        let kinds = protocol_kinds(PROTOCOL).expect("the fixture has kinds");
        assert_eq!(
            kinds.into_iter().collect::<Vec<_>>(),
            ["alive", "done", "segment"]
        );
    }

    /// The decode worker's messages have a `mod kind` of their own
    /// (`protocol/src/decode.rs`), read with `lib.rs` after it.
    #[test]
    fn every_kind_module_is_read() {
        let both = format!(
            "{PROTOCOL}\n/// The decode worker's.\npub mod kind {{\n    pub const DECODED: &str = \"decoded\";\n}}\n"
        );
        let kinds = protocol_kinds(&both).expect("the fixture has kinds");
        assert_eq!(
            kinds.into_iter().collect::<Vec<_>>(),
            ["alive", "decoded", "done", "segment"]
        );
    }

    #[test]
    fn the_e2e_types_match() {
        let js = r#"
const a = { type: 'alive' }, b = { "type": "segment" };
if (d.type === 'alive' || type == "done" || 'alive' !== e.data.type || "done" != d.type) {}
if (["alive", 'done',].includes(e.data.type)) {}
"#;
        let rust = r##"
fn f() {
    in_page("return e2e.ask([{ type: 'alive' }]);");
    in_page(r#"return e2e.ask([{ type: "done" }]);"#);
    p.check(r["end"]["type"] == json!("done"));
    p.check(r["end"]["type"] != "alive");
    in_page("return e2e.ask(args[0]);", json!([{ "type": "segment" }]));
    in_page("return e2e.ask(args[0]);", json!([{ "type": json!("segment") }]));
}
"##;
        assert_eq!(check(js, rust, &[]), vec![]);
    }

    #[test]
    fn a_misspelt_type_is_named() {
        let js_cases = [
            "{ type: 'segmnt' }",
            "{ \"type\": \"segmnt\" }",
            "if (d.type === \"segmnt\") {}",
            "if (type == 'segmnt') {}",
            "if (d.type !== 'segmnt') {}",
            "if (d.type != 'segmnt') {}",
            "if ('segmnt' === e.data.type) {}",
            "if ('segmnt' != type) {}",
            "if (['alive', 'segmnt', 'done',].includes(d.type)) {}",
            "if (['alive', 'segmnt', 'done'].includes(type)) {}",
            "if (typeof d === 'object' && d.type === 'segmnt') {}",
            "{ type: 'text' }",
        ];
        for case in js_cases {
            assert_one_each(
                "PROTO-1",
                &[(
                    WORKER_JS,
                    check(&format!("{JS_OK}\n{case}"), RUST_OK, &[]),
                    Some(2),
                )],
            );
        }
        let rust_cases = [
            "fn f() {\n    in_page(\"const a = 1;\n        const b = 2;\n        return e2e.ask([{ type: \\\"segmnt\\\" }]);\");\n}",
            "fn f() {\n    in_page(r#\"const a = 1;\n        const b = 2;\n        return e2e.ask([{ type: 'segmnt' }]);\"#);\n}",
            "fn f() {\n    in_page(\"const a = 1;\n        const b = 2;\n        return e2e.ask([{ type: 'segmnt' }]);\");\n}",
            "fn f() {\n    p.check(\n        \n        r[\"end\"][\"type\"] == json!(\"segmnt\"));\n}",
            "fn f() {\n    p.check(\n        \n        r[\"end\"][\"type\"] != \"segmnt\");\n}",
            "fn f() {\n    p.check(\n        \n        r[\"end\"][\"type\"] == \"segmnt\");\n}",
            "fn f() {\n    in_page(\"\", json!([{\n        \n        \"type\": \"segmnt\" }]));\n}",
            "fn f() {\n    in_page(\"\", json!([{\n        \n        \"type\": json!(\"segmnt\") }]));\n}",
            "fn f() {\n    in_page(\"\n        \n        const p = \\\"a\\\\\\\"b\\\"; return e2e.ask([{ type: 'segmnt' }]);\");\n}",
            "fn f() {\n    in_page(r#\"\n        \n        const p = \"C:\\\\\"; return e2e.ask([{ type: 'segmnt' }]);\"#);\n}",
        ];
        for case in rust_cases {
            assert_one_each(
                "PROTO-1",
                &[(
                    E2E_RUST,
                    check(JS_OK, &format!("{case}\n{RUST_OK}"), &[]),
                    Some(4),
                )],
            );
        }
    }

    #[test]
    fn other_type_words_are_not_types() {
        let js_cases = [
            "new File([b], f.name, { type: f.type })",
            "typeof a === \"string\"",
            "typeof d.type === \"string\"",
            "\"file\" === h.kind",
            "[\"a\"].includes(d.kind)",
            "const { type, of } = d;",
            "// { type: 'x' }",
            "/* d.type === 'x' */",
            "`${ { type: 'x' } }`",
        ];
        for case in js_cases {
            assert_eq!(
                check(&format!("{JS_OK}\n{case}"), RUST_OK, &[]),
                vec![],
                "{case}"
            );
        }
        let rust = r#"fn f() {
    let files = json!({ "url": url, "type": kind });
    let opts = json!({ "mm": "s64" });
    p.check(c["state"] == "running" && f("type") == "x");
    let pairs = m!["type": "x"];
    p.check(l.contains("Restored \"song\" from this browser"), "the page's song");
}"#;
        assert_eq!(check(JS_OK, &format!("{rust}\n{RUST_OK}"), &[]), vec![]);
        let hidden = "fn f() {\n    p.note(\"the page's lanes\");\n    in_page(\"return e2e.ask([{ type: 'segmnt' }]);\");\n}";
        assert_one_each(
            "PROTO-1",
            &[(
                E2E_RUST,
                check(JS_OK, &format!("{hidden}\n{RUST_OK}"), &[]),
                Some(3),
            )],
        );
    }

    #[test]
    fn the_script_only_list_is_checked() {
        let worker_error = ("worker-error", "the reply that `e2e.ask` makes");
        let js = format!("{JS_OK}\nout.replies.push({{ type: \"worker-error\" }});");
        assert_eq!(check(&js, RUST_OK, &[worker_error]), vec![]);
        let unused = ("zzz", "nothing");
        let in_kind = ("done", "a type");
        assert_one_each(
            "PROTO-1",
            &[
                (
                    RULES_FILE,
                    check(&js, RUST_OK, &[worker_error, unused]),
                    None,
                ),
                (
                    RULES_FILE,
                    check(&js, RUST_OK, &[worker_error, in_kind]),
                    None,
                ),
                (
                    E2E_RUST,
                    check(&js, "fn f() {", &[worker_error, unused]),
                    None,
                ),
                (
                    WORKER_JS,
                    check(&format!("{JS_OK}\n{{ type: \"module\" }}"), RUST_OK, &[]),
                    Some(2),
                ),
            ],
        );
    }

    #[test]
    fn e2e_nothing_to_read_is_a_problem() {
        let protocol_problem = |protocol: &str| {
            e2e_types_match_the_protocol(
                protocol,
                &[(WORKER_JS.to_string(), JS_OK.to_string())],
                RUST_OK,
                &[],
            )
        };
        assert_one_each(
            "PROTO-1",
            &[
                (
                    PROTOCOL_LIB,
                    protocol_problem("pub mod field { pub const A: &str = \"alive\"; }"),
                    None,
                ),
                (PROTOCOL_LIB, protocol_problem("pub mod kind {}"), None),
                (PROTOCOL_LIB, protocol_problem("pub mod kind { \""), None),
                (E2E_RUST, check(JS_OK, "fn f() {", &[]), None),
                (E2E_SCRIPTS, check("const a = 1;", RUST_OK, &[]), None),
                (
                    E2E_SCRIPTS,
                    e2e_types_match_the_protocol(PROTOCOL, &[], RUST_OK, &[]),
                    None,
                ),
                (E2E_RUST, check(JS_OK, "fn f() {}", &[]), None),
            ],
        );
    }
}

#[cfg(test)]
mod settings_tests {
    use super::tracked_tests::assert_one_each;
    use super::*;
    use serde_json::{Value, json};

    fn settings() -> Value {
        json!({
            "$schema": "https://json.schemastore.org/claude-code-settings.json",
            "permissions": { "deny": REQUIRED_DENY_RULES }
        })
    }

    fn with(change: impl FnOnce(&mut Value)) -> String {
        let mut value = settings();
        change(&mut value);
        value.to_string()
    }

    #[test]
    fn settings_that_deny_every_rule_pass() {
        let cases = [
            settings().to_string(),
            with(|v| {
                v["permissions"]["deny"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("Bash(rm -rf *)"))
            }),
            with(|v| v["hooks"] = json!({ "PostToolUse": [] })),
        ];
        for case in cases {
            assert_eq!(claude_settings_problems(&case), [], "{case}");
        }
    }

    #[test]
    fn a_missing_deny_rule_is_named() {
        let without_merge = with(|v| {
            v["permissions"]["deny"]
                .as_array_mut()
                .unwrap()
                .retain(|rule| rule != "Bash(gh pr merge *)")
        });
        let problems = claude_settings_problems(&without_merge);
        assert_one_each("GIT-1", &[(SETTINGS_FILE, problems.clone(), None)]);
        assert!(
            problems[0]
                .message
                .contains("does not deny `Bash(gh pr merge *)`"),
            "{}",
            problems[0]
        );

        let removed = ["Bash(git push *main)", "Bash(cargo r* publish*)"];
        let without_two = with(|v| {
            v["permissions"]["deny"]
                .as_array_mut()
                .unwrap()
                .retain(|rule| !removed.iter().any(|gone| rule == gone))
        });
        let problems = claude_settings_problems(&without_two);
        assert_eq!(problems.len(), 2, "{problems:#?}");
        for (problem, gone) in problems.iter().zip(removed) {
            assert!(
                problem.message.contains(&format!("does not deny `{gone}`")),
                "{problem}"
            );
        }
    }

    #[test]
    fn unreadable_settings_are_named() {
        let cases = [
            ("{".to_string(), "is not JSON (EOF"),
            ("[]".to_string(), "is not a JSON object"),
            ("{}".to_string(), "has no `permissions` object"),
            (
                r#"{"permissions":1}"#.to_string(),
                "has no `permissions` object",
            ),
            (
                r#"{"permissions":{}}"#.to_string(),
                "has no `permissions.deny` list",
            ),
            (
                r#"{"permissions":{"deny":"Bash(gh pr merge *)"}}"#.to_string(),
                "has no `permissions.deny` list",
            ),
            (
                r#"{"permissions":{"deny":[1]}}"#.to_string(),
                "has `1` in `permissions.deny`, which is not a string",
            ),
            (
                with(|v| v["permissions"]["allow"] = json!(["Bash(ls *)"])),
                "has the key `permissions.allow`",
            ),
            (with(|v| v["model"] = json!(5)), "has the key `model`"),
            (
                with(|v| v["$schema"] = json!(5)),
                "has a `$schema` that is not a string",
            ),
        ];
        let checked: Vec<_> = cases
            .iter()
            .map(|(case, _)| (SETTINGS_FILE, claude_settings_problems(case), None))
            .collect();
        assert_one_each("GIT-1", &checked);
        for ((case, what), (_, problems, _)) in cases.iter().zip(&checked) {
            assert!(
                problems[0]
                    .message
                    .starts_with(&format!("GIT-1: {SETTINGS_FILE} {what}")),
                "{case}: {}",
                problems[0]
            );
        }
    }
}

#[cfg(test)]
mod agents_tests {
    use super::*;

    const FILE: &str = "reviewer.md";

    /// A good agent file for `reviewer.md`.
    fn good() -> String {
        format!(
            "---\nname: reviewer\ndescription: Reviews one commit range of this repo in a review round.  Read only.  Use when a review round routes reviewer.\ntools: Read, Bash\nmodel: inherit\n{AGENT_HOOK_BLOCK}\n---\n\nThe body.\n"
        )
    }

    /// The good file with `from` replaced by `to`, once.
    fn with(from: &str, to: &str) -> String {
        let good = good();
        assert!(good.contains(from), "the good file has no {from:?}");
        good.replacen(from, to, 1)
    }

    #[test]
    fn a_read_only_agent_has_no_problem() {
        let listed = with("tools: Read, Bash\n", "tools:\n  - Read\n  - Bash\n");
        let unindented = with("tools: Read, Bash\n", "tools:\n- Read\n- Bash\n");
        let blank_line = with("model: inherit\n", "model: inherit\n\n");
        for case in [
            good(),
            listed,
            unindented,
            blank_line,
            good().replace('\n', "\r\n"),
        ] {
            assert_eq!(agent_problems(FILE, &case), [], "{case}");
        }
    }

    /// Each plant gives a problem of the agents rule for the file, and one
    /// problem holds the text that names the plant.
    #[test]
    fn each_plant_is_named() {
        let hooks = format!("{AGENT_HOOK_BLOCK}\n");
        let cases = [
            ("no hooks key", with(&hooks, ""), "has no `hooks` key"),
            (
                "no name key",
                with("name: reviewer\n", ""),
                "has no `name` key",
            ),
            (
                "no model key",
                with("model: inherit\n", ""),
                "has no `model` key",
            ),
            (
                "a description that wraps",
                with("Read only.  Use", "Read only.\n  Use"),
                "has an indented line outside",
            ),
            (
                "no tools key",
                with("tools: Read, Bash\n", ""),
                "has no `tools` key",
            ),
            (
                "Edit in tools",
                with("tools: Read, Bash", "tools: Read, Edit, Bash"),
                "has `Edit` in `tools`",
            ),
            (
                "Task in a YAML list",
                with(
                    "tools: Read, Bash\n",
                    "tools:\n  - Read\n  - Task\n  - Bash\n",
                ),
                "has `Task` in `tools`",
            ),
            (
                "Grep beside Bash",
                with("tools: Read, Bash", "tools: Read, Grep, Bash"),
                "has `Grep` in `tools`",
            ),
            (
                "Glob beside Bash",
                with("tools: Read, Bash", "tools: Read, Glob, Bash"),
                "has `Glob` in `tools`",
            ),
            (
                "no Bash",
                with("tools: Read, Bash", "tools: Read"),
                "has no `Bash` in `tools`",
            ),
            (
                "a memory key",
                with("model: inherit\n", "model: inherit\nmemory: project\n"),
                "has the key `memory`",
            ),
            (
                "a key twice",
                with("model: inherit\n", "model: inherit\nmodel: inherit\n"),
                "has the key `model` twice",
            ),
            (
                "no description",
                with(
                    "description: Reviews one commit range of this repo in a review round.  Read only.  Use when a review round routes reviewer.\n",
                    "",
                ),
                "has no `description` key",
            ),
            (
                "an empty description",
                with(
                    "description: Reviews one commit range of this repo in a review round.  Read only.  Use when a review round routes reviewer.\n",
                    "description:\n",
                ),
                "has a `description` that is empty",
            ),
            (
                "a description with a colon and a tab",
                with("Read only.", "Read only:\tyes."),
                "has a `description` that holds a `:` before a space or a tab",
            ),
            (
                "no space after the colon of a key",
                with("name: reviewer", "name:reviewer"),
                "has the line `name:reviewer`, which is not `key: value`",
            ),
            (
                "no space after the colon of model",
                with("model: inherit", "model:inherit"),
                "has the line `model:inherit`, which is not `key: value`",
            ),
            (
                "a line with no colon",
                with("model: inherit\n", "model: inherit\nplain\n"),
                "has the line `plain`, which is not `key: value`",
            ),
            (
                "a description with a colon",
                with("Read only.", "Read only: yes."),
                "has a `description` that holds a `:` before a space or a tab",
            ),
            (
                "a description that ends with a colon",
                with("routes reviewer.\n", "routes reviewer:\n"),
                "has a `description` that ends with `:`",
            ),
            (
                "a description with a comment",
                with("Read only.", "Read only #1."),
                "has a `description` that holds a blank before `#`",
            ),
            (
                "a description with a tab before a comment",
                with("Read only.", "Read only\t#1."),
                "has a `description` that holds a blank before `#`",
            ),
            (
                "a description with a quote",
                with("Read only.", "Read 'only'."),
                "has a `description` that holds a quote",
            ),
            (
                "a description that starts with a small letter",
                with("description: Reviews", "description: reviews"),
                "has a `description` that does not start with a capital letter",
            ),
            (
                "another model",
                with("model: inherit", "model: opus"),
                "has `model: opus`, not `model: inherit`",
            ),
            (
                "a key after hooks",
                with(&hooks, &format!("{hooks}color: red\n")),
                "has the key `color` after `hooks`",
            ),
            (
                "another name",
                with("name: reviewer", "name: mem-reviewer"),
                "has the name `mem-reviewer`, not `reviewer`",
            ),
            (
                "double quotes outside the command",
                with(
                    r#"command: '"$CLAUDE_PROJECT_DIR"/.claude/hooks/read-only-bash.sh'"#,
                    r#"command: "$CLAUDE_PROJECT_DIR/.claude/hooks/read-only-bash.sh""#,
                ),
                "has a hook block that is not",
            ),
            (
                "another path",
                with("read-only-bash.sh", "other.sh"),
                "has a hook block that is not",
            ),
            (
                "the hook on PostToolUse",
                with("PreToolUse", "PostToolUse"),
                "has a hook block that is not",
            ),
            (
                "the matcher Edit",
                with("matcher: Bash", "matcher: Edit"),
                "has a hook block that is not",
            ),
            (
                "no onFailure",
                with("\n          onFailure: block", ""),
                "has a hook block that is not",
            ),
            (
                "a second hook",
                with(
                    "onFailure: block\n",
                    "onFailure: block\n        - type: command\n          command: echo\n",
                ),
                "has a hook block that is not",
            ),
            (
                "no closing line",
                with("---\n\nThe body", "\nThe body"),
                "has no `---` line that closes the frontmatter",
            ),
            (
                "no opening line",
                good().replacen("---\n", "", 1),
                "does not start with a `---` line",
            ),
        ];
        for (plant, text, named) in cases {
            let problems = agent_problems(FILE, &text);
            assert!(
                problems.iter().all(|problem| problem.rule == "agents"
                    && problem.file == ".claude/agents/reviewer.md"),
                "{plant}: {problems:#?}"
            );
            assert!(
                problems.iter().any(|problem| problem
                    .message
                    .starts_with("agents: .claude/agents/reviewer.md ")
                    && problem.message.contains(named)
                    && problem.message.contains("Fix: ")),
                "{plant}: no problem names {named:?} in {problems:#?}"
            );
        }
    }

    #[test]
    fn a_renamed_file_is_named() {
        let problems = agent_problems("mem-reviewer.md", &good());
        assert_eq!(problems.len(), 1, "{problems:#?}");
        assert!(
            problems[0]
                .message
                .contains("has the name `reviewer`, not `mem-reviewer`"),
            "{}",
            problems[0]
        );
    }

    fn entry(path: &str, text: Option<String>) -> (String, Option<String>) {
        (path.to_string(), text)
    }

    /// A good folder: two agents and the `.gitkeep`.
    fn folder() -> Vec<(String, Option<String>)> {
        vec![
            entry("reviewer.md", Some(good())),
            entry(
                "validator.md",
                Some(good().replacen("name: reviewer", "name: validator", 1)),
            ),
            entry(".gitkeep", Some(String::new())),
        ]
    }

    #[test]
    fn a_folder_of_read_only_agents_has_no_problem() {
        assert_eq!(agents_folder_problems(&folder()), []);
        let finder = [folder(), vec![entry(".DS_Store", Some("\0".to_string()))]].concat();
        assert_eq!(agents_folder_problems(&finder), []);
    }

    /// Each plant gives a problem of the agents rule that names it.
    #[test]
    fn each_folder_plant_is_named() {
        let plus = |extra: Vec<(String, Option<String>)>| [folder(), extra].concat();
        let cases = [
            (
                "a subfolder",
                plus(vec![entry("sub", None)]),
                "sub is a subfolder",
            ),
            (
                "an agent in a subfolder",
                plus(vec![
                    entry("sub", None),
                    entry(
                        "sub/writer.md",
                        Some(good().replacen(
                            "tools: Read, Bash",
                            "tools: Read, Edit, Write, Bash",
                            1,
                        )),
                    ),
                ]),
                "sub/writer.md is in a subfolder",
            ),
            (
                "a name in a subfolder that an agent has",
                plus(vec![entry("sub/writer.md", Some(good()))]),
                "sub/writer.md has the name `reviewer`, which reviewer.md has too",
            ),
            (
                "a file that is not an agent",
                plus(vec![entry("notes.txt", Some("x".to_string()))]),
                "notes.txt is not an agent file",
            ),
            (
                "two agents with one name",
                plus(vec![entry("reviewer2.md", Some(good()))]),
                "reviewer2.md has the name `reviewer`, which reviewer.md has too",
            ),
            (
                "an agent with a write tool",
                vec![entry(
                    "reviewer.md",
                    Some(good().replacen("tools: Read, Bash", "tools: Read, Edit, Bash", 1)),
                )],
                "reviewer.md has `Edit` in `tools`",
            ),
        ];
        for (plant, entries, named) in cases {
            let problems = agents_folder_problems(&entries);
            assert!(
                problems.iter().all(|problem| problem.rule == "agents"),
                "{plant}: {problems:#?}"
            );
            assert!(
                problems.iter().any(|problem| problem
                    .message
                    .starts_with(&format!("agents: {AGENTS_FOLDER}{named}"))
                    && problem.message.contains("Fix: ")),
                "{plant}: no problem names {named:?} in {problems:#?}"
            );
        }
    }
}
