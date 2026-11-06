//! `xtask check --tier commit|push [--dry-run]`: the local checks of
//! GIT-2, by tier (decision 0026).
//!
//! Tier `commit` reads the changed paths of one commit, and picks the
//! checks that those paths need.  Tier `push` runs the fmt, clippy, test,
//! `docs-check` and Trunk build lines of `CLAUDE.md`'s Commands.  A commit
//! that touches a strict path ([`STRICT_PATHS`]) runs tier `push`.  No
//! check is a browser test or `publish`, and CI stays the authority.
//!
//! The code has parts, so the tests can check the rules without a run.
//! [`plan`] is a pure function of the tier, the paths and the workspace
//! graph.  [`with_gpu_require`] gives the GPU test line the caller's
//! `STEMYLA_REQUIRE` when it names a GPU need.  [`changed_paths`] and
//! [`graph`] read the repo.
//! [`run_checks`] runs a plan with a runner that the caller gives, and
//! [`run_command`] is the real runner.
//!
//! `plans/completed/harness-phase-3b-m4.md` gives the tables and the reasons.

use crate::docs::glob_matches;
use crate::review_log::strict_rules;
use crate::rules;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub const USAGE: &str = "usage: xtask check --tier commit|push [--dry-run]";

/// The exit code of a failed check.
const FAILED: i32 = 1;

/// The exit code of a usage error, and of an error that stops the plan.
const REFUSED: i32 = 2;

/// The tier of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Commit,
    Push,
}

/// One command: its argv, its environment and its folder, relative to the
/// repo root (`""` is the root).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub folder: String,
}

impl Check {
    /// A command in the repo root with no environment, from its words.
    fn root(words: &str) -> Check {
        Check {
            argv: words.split_whitespace().map(str::to_string).collect(),
            env: Vec::new(),
            folder: String::new(),
        }
    }

    /// The command as `CLAUDE.md`'s Commands gives it: the environment and
    /// the argv, joined by spaces, in `(cd <folder> && ...)` for a folder.
    pub fn line(&self) -> String {
        let words: Vec<String> = self
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .chain(self.argv.iter().cloned())
            .collect();
        let words = words.join(" ");
        if self.folder.is_empty() {
            words
        } else {
            format!("(cd {} && {words})", self.folder)
        }
    }
}

/// The index of each line of [`lines`] that the plan reads by name.
const FMT: usize = 0;
const DOCS_CHECK: usize = 1;
const CLIPPY: usize = 2;
const SYNTH_CLIPPY: usize = 3;
const APP_CLIPPY: usize = 4;
const APP_ALL_CLIPPY: usize = 5;
const WASM_CLIPPY: usize = 6;
const WORKSPACE_TESTS: usize = 7;
const GPU_LIB: usize = 8;
const GPU_DOC: usize = 9;
const GPU_TESTS: usize = 10;
const TRUNK: usize = 11;

/// The packages of the wasm32 clippy line.
const WASM_PACKAGES: [&str; 5] = [
    "app",
    "stem-worker",
    "decode-worker",
    "stem-protocol",
    "engine",
];

/// The package that the workspace test line leaves out.
const ROFORMER_GPU: &str = "roformer-gpu";

/// The variable that names what a test must not skip (`test-support`).
const REQUIRE: &str = "STEMYLA_REQUIRE";

/// The GPU test line's value of [`REQUIRE`] in `CLAUDE.md`'s Commands.
/// [`with_gpu_require`] can replace it.
const GPU_FORM: &str = "gpu,f16";

/// The 12 lines of tier `push`, in the order that it runs them.
pub fn lines() -> Vec<Check> {
    let wasm = format!(
        "cargo clippy --target wasm32-unknown-unknown -p {} --all-targets -- -D warnings",
        WASM_PACKAGES.join(" -p ")
    );
    let mut lines: Vec<Check> = [
        "cargo fmt --all --check",
        "cargo run -p xtask -- docs-check",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "cargo clippy -p synth --all-targets -- -D warnings",
        "cargo clippy -p app -- -D warnings",
        "cargo clippy -p app --all-targets -- -D warnings",
        &wasm,
        "cargo test --workspace --exclude roformer-gpu",
        "cargo test -p roformer-gpu --lib",
        "cargo test -p roformer-gpu --doc",
        "cargo test -p roformer-gpu --release",
        "trunk build --release",
    ]
    .iter()
    .map(|words| Check::root(words))
    .collect();
    lines[GPU_TESTS].env = vec![(REQUIRE.to_string(), GPU_FORM.to_string())];
    lines[TRUNK].folder = "app".to_string();
    lines
}

/// The tests of `xtask` that read the whole repo.  They take seconds; the
/// other tests of `xtask` take about a minute.
fn xtask_repo_tests() -> Check {
    Check::root("cargo test -p xtask --lib --test rules --test docs --test review")
}

/// The lines of `CLAUDE.md`'s Commands that no tier runs, with the reason.
/// Each plan skips them.
pub fn never_run() -> Vec<(Check, &'static str)> {
    const BROWSER: &str =
        "CI runs the browser tests; run a scenario by hand only when CI failed it (decision 0026)";
    vec![
        (
            Check::root("cargo run -p xtask --release -- serve app/dist --port 8080"),
            BROWSER,
        ),
        (Check::root("cargo run -p xtask -- web-test"), BROWSER),
        (Check::root("cargo run -p xtask -- e2e <scenario>"), BROWSER),
        (
            Check::root("cargo +nightly-2026-10-30 test -p roformer-gpu --doc"),
            "CI runs it; it needs the pinned nightly, and the stable doctest line runs the same doctests",
        ),
        (
            Check::root("cargo run -p xtask --release -- publish"),
            "a person only (GIT-1)",
        ),
    ]
}

/// The paths outside every member that change every build, at the root
/// only.
const WORKSPACE_FILES: [&str; 9] = [
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/config.toml",
    ".cargo/config",
    "rust-toolchain",
    "rust-toolchain.toml",
    "clippy.toml",
    "rustfmt.toml",
    ".rustfmt.toml",
];

fn is_workspace_file(path: &str) -> bool {
    WORKSPACE_FILES.contains(&path)
}

/// The folders whose tests the GPU test line runs.
const GPU_FOLDERS: [&str; 3] = ["roformer/", "roformer-gpu/", "test-support/"];

/// The strict-rule path table: the globs of the files that hold the code
/// of each strict rule (decision 0024), in the order of
/// [`strict_rules`].  The globs are narrower than the `paths:` of
/// `.claude/rules/`.  VERIFY-1 is on most findings and GIT-1 has no code,
/// so their rows are empty.  [`strict_hits`] also gives ASSET-1 each path
/// with an audio or weight extension.
///
/// `app/src/lib.rs` is in no row, but it holds `MEMORY_PATH_MAX_FRAMES` and
/// some transfer code.  Most commits of the app change it, so a row with it
/// would send most commits to tier `push`.
pub const STRICT_PATHS: &[(&str, &[&str])] = &[
    (
        "PROTO-1",
        &[
            "protocol/src/**",
            "xtask/e2e/*.js",
            "xtask/src/e2e.rs",
            "xtask/src/e2e/**",
        ],
    ),
    (
        "PROTO-2",
        &[
            "protocol/src/lib.rs",
            "app/src/separator.rs",
            "worker/src/web.rs",
        ],
    ),
    (
        "MEM-1",
        &[
            "engine/src/song.rs",
            "engine/src/test_alloc.rs",
            "**/clippy.toml",
        ],
    ),
    (
        "MEM-2",
        &[
            "protocol/src/**",
            "app/src/separator.rs",
            "app/src/decoder/client.rs",
            "worker/src/web.rs",
            "decoder/src/web.rs",
        ],
    ),
    (
        "MEM-3",
        &[
            "roformer-gpu/src/lib.rs",
            "roformer-gpu/src/net.rs",
            "roformer-gpu/clippy.toml",
        ],
    ),
    (
        "MEM-4",
        &[
            "roformer/src/lib.rs",
            "roformer/src/weights.rs",
            "roformer-gpu/src/lib.rs",
        ],
    ),
    (
        "MEM-5",
        &[
            "app/src/store.rs",
            "app/src/export.rs",
            "app/src/streamtest.rs",
            "worker/src/web.rs",
            "worker/src/web/encode.rs",
            "worker/src/web/stems.rs",
            "decoder/src/web.rs",
        ],
    ),
    (
        "RT-1",
        &[
            "engine/src/slot.rs",
            "engine/src/source.rs",
            "engine/src/player.rs",
            "engine/src/live.rs",
            "engine/src/readahead.rs",
            "engine/src/readahead/**",
            "engine/src/stretch/**",
            "engine/src/params.rs",
            "engine/tests/realtime.rs",
        ],
    ),
    (
        "GPU-1",
        &["roformer-gpu/src/shaders/**", "roformer-gpu/src/lib.rs"],
    ),
    ("GPU-2", GPU_DEFAULTS),
    ("GPU-3", GPU_DEFAULTS),
    ("VERIFY-1", &[]),
    ("GIT-1", &[]),
    ("DEPS-1", &["**/Cargo.toml", "Cargo.lock", "THIRD-PARTY.md"]),
    ("ASSET-1", &[".gitignore"]),
];

/// The kernels, and the code that chooses the default kernels and
/// weights (GPU-2, GPU-3).
const GPU_DEFAULTS: &[&str] = &[
    "roformer-gpu/src/shaders/**",
    "roformer-gpu/src/bench.rs",
    "roformer-gpu/src/lib.rs",
];

/// The strict paths of `rule`, or none.
pub fn strict_paths(rule: &str) -> &'static [&'static str] {
    STRICT_PATHS
        .iter()
        .find(|(id, _)| *id == rule)
        .map_or(&[], |(_, globs)| globs)
}

/// Each (rule, path) pair where `path` is a strict path of `rule`, in the
/// order of [`strict_rules`], then of `paths`.
pub fn strict_hits(paths: &[String]) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    for rule in strict_rules() {
        let globs = strict_paths(&rule);
        for path in paths {
            let by_glob = globs.iter().any(|glob| glob_matches(glob, path));
            let by_extension = rule == "ASSET-1" && rules::is_audio_or_weight(path);
            if by_glob || by_extension {
                hits.push((rule.clone(), path.clone()));
            }
        }
    }
    hits
}

/// The workspace members and the edges between them.  The edges are of
/// all kinds (normal, dev, build) and for all targets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    /// Each member by package name: its folder, and the members that it
    /// depends on.
    members: BTreeMap<String, (String, BTreeSet<String>)>,
}

impl Graph {
    /// A graph from (folder, package name, the members that it depends on).
    pub fn new(members: &[(&str, &str, &[&str])]) -> Graph {
        Graph {
            members: members
                .iter()
                .map(|(folder, name, deps)| {
                    (
                        name.to_string(),
                        (
                            folder.to_string(),
                            deps.iter().map(|d| d.to_string()).collect(),
                        ),
                    )
                })
                .collect(),
        }
    }

    /// The graph of `workspace`, which `rules::workspace` reads.
    pub fn from_workspace(workspace: &rules::Workspace) -> Graph {
        Graph {
            members: workspace
                .crates
                .iter()
                .map(|member| {
                    let folder = member
                        .manifest
                        .rsplit_once('/')
                        .map_or("", |(folder, _)| folder);
                    let deps = member.edges.iter().map(|edge| edge.to.clone()).collect();
                    (member.name.clone(), (folder.to_string(), deps))
                })
                .collect(),
        }
    }

    /// The package names of the members.
    pub fn names(&self) -> BTreeSet<String> {
        self.members.keys().cloned().collect()
    }

    /// The member that holds `path`: the one with the longest folder that
    /// `path` starts with, then `/`.
    pub fn member_of(&self, path: &str) -> Option<&str> {
        self.members
            .iter()
            .filter(|(_, (folder, _))| {
                !folder.is_empty()
                    && path
                        .strip_prefix(folder.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|(_, (folder, _))| folder.len())
            .map(|(name, _)| name.as_str())
    }

    /// `set`, and each member that it depends on, again and again.
    pub fn closure(&self, set: &BTreeSet<String>) -> BTreeSet<String> {
        self.reach(set, |from, to| {
            self.members
                .get(from)
                .is_some_and(|(_, deps)| deps.contains(to))
        })
    }

    /// `set`, and each member that depends on one of them, again and again.
    pub fn dependents(&self, set: &BTreeSet<String>) -> BTreeSet<String> {
        self.reach(set, |from, to| {
            self.members
                .get(to)
                .is_some_and(|(_, deps)| deps.contains(from))
        })
    }

    /// `set`, and each member `to` with `step(from, to)` for a member
    /// `from` of the result, again and again.
    fn reach(&self, set: &BTreeSet<String>, step: impl Fn(&str, &str) -> bool) -> BTreeSet<String> {
        let mut found = set.clone();
        let mut next: Vec<String> = set.iter().cloned().collect();
        while let Some(from) = next.pop() {
            for to in self.members.keys() {
                if !found.contains(to) && step(&from, to) {
                    found.insert(to.clone());
                    next.push(to.clone());
                }
            }
        }
        found
    }
}

/// What a plan does with one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Run,
    Skip,
}

/// One check of a plan, with its decision and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub check: Check,
    pub decision: Decision,
    pub reason: String,
}

/// The checks of one run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The tier that runs.
    pub tier: Tier,
    /// Why that tier runs.
    pub why: String,
    /// Each (rule, path) pair of [`strict_hits`].
    pub strict: Vec<(String, String)>,
    /// Each check, in order.
    pub choices: Vec<Choice>,
}

/// The checks of `tier` for `paths`.  Tier `commit` needs `graph` to choose
/// the crates; with no graph, it runs tier `push`.
pub fn plan(tier: Tier, paths: &[String], graph: Option<&Graph>) -> Plan {
    let strict = strict_hits(paths);
    let workspace_file = paths.iter().any(|path| is_workspace_file(path));
    let (choices, tier, why) = match (tier, graph) {
        (Tier::Push, _) => (push_choices(paths), Tier::Push, "the tier that was asked"),
        (Tier::Commit, _) if !strict.is_empty() => (
            push_choices(paths),
            Tier::Push,
            "a strict path changed (D2)",
        ),
        (Tier::Commit, None) => (
            push_choices(paths),
            Tier::Push,
            "there is no workspace graph, so tier commit cannot choose the crates",
        ),
        (Tier::Commit, Some(_)) if workspace_file => (
            workspace_file_choices(),
            Tier::Commit,
            "a workspace file changed",
        ),
        (Tier::Commit, Some(graph)) => (
            commit_choices(paths, graph),
            Tier::Commit,
            "the changed paths choose the checks",
        ),
    };
    let mut choices = choices;
    choices.extend(
        never_run()
            .into_iter()
            .map(|(check, reason)| skip(check, reason)),
    );
    Plan {
        tier,
        why: why.to_string(),
        strict,
        choices,
    }
}

fn run(check: Check, reason: &str) -> Choice {
    Choice {
        check,
        decision: Decision::Run,
        reason: reason.to_string(),
    }
}

fn skip(check: Check, reason: &str) -> Choice {
    Choice {
        check,
        decision: Decision::Skip,
        reason: reason.to_string(),
    }
}

fn run_if(check: Check, yes: bool, run_reason: &str, skip_reason: &str) -> Choice {
    if yes {
        run(check, run_reason)
    } else {
        skip(check, skip_reason)
    }
}

const COMMIT_GPU: &str = "it needs a GPU adapter, and tier push runs it when its paths change GPU code; GPU-2 asks for it on a kernel change, and a pass counts for GPU-3 only on a real GPU named in the PR";
const COMMIT_TRUNK: &str = "tier push runs it";

/// Each line of tier `push`.  The GPU test line runs only when `paths`
/// change GPU code, or when `paths` is empty.  Then no path changed, or no
/// path is known, so a GPU change cannot be ruled out.
fn push_choices(paths: &[String]) -> Vec<Choice> {
    let gpu = paths.is_empty()
        || paths.iter().any(|path| {
            is_workspace_file(path) || GPU_FOLDERS.iter().any(|folder| path.starts_with(folder))
        });
    let gpu_reason = if paths.is_empty() {
        "no path changed, or no path is known, so a GPU change cannot be ruled out"
    } else {
        "a path is in roformer/, roformer-gpu/ or test-support/, or is a workspace file"
    };
    lines()
        .into_iter()
        .enumerate()
        .map(|(index, check)| {
            if index == GPU_TESTS {
                run_if(
                    check,
                    gpu,
                    gpu_reason,
                    "the paths change no GPU code: nothing in roformer/, roformer-gpu/ or test-support/, and no workspace file",
                )
            } else {
                run(check, "tier push runs every line")
            }
        })
        .collect()
}

/// Tier `commit` when a workspace file changed: each line but the GPU test
/// line and the Trunk build.
fn workspace_file_choices() -> Vec<Choice> {
    lines()
        .into_iter()
        .enumerate()
        .map(|(index, check)| match index {
            GPU_TESTS => skip(check, COMMIT_GPU),
            TRUNK => skip(check, COMMIT_TRUNK),
            _ => run(check, "a workspace file changed"),
        })
        .collect()
}

/// Tier `commit`: the checks that the changed members need.
fn commit_choices(paths: &[String], graph: &Graph) -> Vec<Choice> {
    let changed: BTreeSet<String> = paths
        .iter()
        .filter_map(|path| graph.member_of(path))
        .map(str::to_string)
        .collect();
    let dependents = graph.dependents(&changed);
    let in_closure = |packages: &[&str]| {
        let packages: BTreeSet<String> = packages.iter().map(|p| p.to_string()).collect();
        let closure = graph.closure(&packages);
        changed.iter().any(|member| closure.contains(member))
    };
    let tested: BTreeSet<&String> = dependents
        .iter()
        .filter(|name| name.as_str() != ROFORMER_GPU)
        .collect();
    let all_but_gpu: BTreeSet<String> = graph
        .names()
        .into_iter()
        .filter(|name| name != ROFORMER_GPU)
        .collect();
    let every_member = !tested.is_empty() && tested.len() == all_but_gpu.len();

    let lines = lines();
    let line = |index: usize| lines[index].clone();
    let mut choices: Vec<Choice> = [FMT, DOCS_CHECK, CLIPPY]
        .map(|index| run(line(index), "every change"))
        .to_vec();
    choices.push(run_if(
        line(SYNTH_CLIPPY),
        in_closure(&["synth"]),
        "a changed member is in the closure of synth",
        "no changed member is in the closure of synth",
    ));
    let app = in_closure(&["app"]);
    for index in [APP_CLIPPY, APP_ALL_CLIPPY] {
        choices.push(run_if(
            line(index),
            app,
            "a changed member is in the closure of app",
            "no changed member is in the closure of app",
        ));
    }
    choices.push(run_if(
        line(WASM_CLIPPY),
        in_closure(&WASM_PACKAGES),
        "a changed member is in the closure of the wasm32 packages",
        "no changed member is in the closure of the wasm32 packages",
    ));
    choices.push(run_if(
        xtask_repo_tests(),
        !dependents.contains("xtask"),
        "these tests read the whole repo, and xtask is not a dependent of a changed member",
        "the crate tests run every test of xtask",
    ));
    if every_member {
        choices.push(run(
            line(WORKSPACE_TESTS),
            "the changed members and their dependents are every member but roformer-gpu",
        ));
    } else if tested.is_empty() {
        choices.push(skip(
            line(WORKSPACE_TESTS),
            "no changed path is in a member",
        ));
    } else {
        let words: Vec<String> = tested.iter().map(|name| format!("-p {name}")).collect();
        choices.push(run(
            Check::root(&format!("cargo test {}", words.join(" "))),
            "the changed members and their dependents, in one command",
        ));
        choices.push(skip(
            line(WORKSPACE_TESTS),
            "the crate tests cover the changed members and their dependents; tier push runs it",
        ));
    }
    let gpu = dependents.contains(ROFORMER_GPU);
    for index in [GPU_LIB, GPU_DOC] {
        choices.push(run_if(
            line(index),
            gpu,
            "roformer-gpu is a dependent of a changed member",
            "roformer-gpu is not a dependent of a changed member",
        ));
    }
    choices.push(skip(line(GPU_TESTS), COMMIT_GPU));
    choices.push(skip(line(TRUNK), COMMIT_TRUNK));
    choices
}

/// The words of [`REQUIRE`] that need a GPU adapter in `test-support`:
/// `gpu`, and the words that imply it.
const GPU_WORDS: [&str; 3] = ["gpu", "f16", "timestamps"];

/// `plan` with the GPU test line in the form that the caller asks for.
/// `require` is the caller's `STEMYLA_REQUIRE`.  It replaces `gpu,f16`
/// only when one of its words is in [`GPU_WORDS`].  An example is `gpu`
/// on an adapter without `SHADER_F16_IN_F32` (GPU-2).  Another non-empty
/// value keeps `gpu,f16`, so the line cannot pass with no adapter
/// (VERIFY-1).  The reason of the line says which of the two occurs.
pub fn with_gpu_require(mut plan: Plan, require: Option<&str>) -> Plan {
    let require = require.map_or("", str::trim);
    if require.is_empty() || require == GPU_FORM {
        return plan;
    }
    let needs_gpu = require
        .split(',')
        .any(|word| GPU_WORDS.contains(&word.trim()));
    let default = &lines()[GPU_TESTS];
    for choice in &mut plan.choices {
        if choice.check != *default {
            continue;
        }
        if needs_gpu {
            choice.check.env = vec![(REQUIRE.to_string(), require.to_string())];
            choice.reason = format!(
                "{}; {REQUIRE}={require} from the environment replaces {GPU_FORM}",
                choice.reason
            );
        } else {
            choice.reason = format!(
                "{}; {REQUIRE}={require} from the environment is not used, since it names no GPU need (gpu, f16 or timestamps)",
                choice.reason
            );
        }
    }
    plan
}

/// The root of the worktree that holds `dir`
/// (`git rev-parse --show-toplevel`).
pub fn repo_root(dir: &Path) -> Result<PathBuf> {
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(top.trim_end_matches('\n')))
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
    String::from_utf8(out.stdout)
        .with_context(|| format!("git {} gave text that is not UTF-8", args.join(" ")))
}

/// `text` on one line: its lines, trimmed, joined by `; `.  So each line
/// of `check` starts with `check:` or `warning:`.
fn one_line(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines.join("; ")
}

/// True when git in `dir` reads `rev` as a commit.
fn has_commit(dir: &Path, rev: &str) -> bool {
    git(
        dir,
        &["rev-parse", "--verify", "-q", &format!("{rev}^{{commit}}")],
    )
    .is_ok()
}

/// The paths of the `-z` output of git, in `found`.
fn add_paths(found: &mut BTreeSet<String>, output: &str) {
    found.extend(
        output
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_string),
    );
}

/// The changed paths of `tier`, relative to `root`, sorted, and a phrase
/// that says where they came from.  A phrase that starts with `warning:`
/// says that tier `push` found no merge base, so it reads no path.
///
/// Tier `commit` reads the changes of the index and of the working tree
/// against `HEAD`, and the untracked files.  It reads the index alone too,
/// so a staged change that the working tree undoes stays.  On a clean tree,
/// it reads the last commit, against its first parent, or each file of a
/// root commit.  Tier `push` reads the changes since the merge base of
/// `HEAD` and `origin/main`, in the working tree and in `HEAD`, and the
/// untracked files.  So a committed change that the working tree reverts
/// stays.  A rename gives both paths.
pub fn changed_paths(root: &Path, tier: Tier) -> Result<(Vec<String>, String)> {
    if !has_commit(root, "HEAD") {
        bail!("the repo has no commit, so there is no HEAD to compare with");
    }
    let untracked = ["ls-files", "--others", "--exclude-standard", "-z"];
    let mut found = BTreeSet::new();
    let source = match tier {
        Tier::Commit => {
            add_paths(
                &mut found,
                &git(root, &["diff", "--name-only", "--no-renames", "-z", "HEAD"])?,
            );
            add_paths(
                &mut found,
                &git(
                    root,
                    &[
                        "diff",
                        "--cached",
                        "--name-only",
                        "--no-renames",
                        "-z",
                        "HEAD",
                    ],
                )?,
            );
            add_paths(&mut found, &git(root, &untracked)?);
            if !found.is_empty() {
                "the changes against HEAD and the untracked files".to_string()
            } else if has_commit(root, "HEAD^") {
                add_paths(
                    &mut found,
                    &git(
                        root,
                        &["diff", "--name-only", "--no-renames", "-z", "HEAD^", "HEAD"],
                    )?,
                );
                "the last commit (HEAD^..HEAD)".to_string()
            } else {
                add_paths(
                    &mut found,
                    &git(
                        root,
                        &[
                            "diff-tree",
                            "-r",
                            "--root",
                            "--no-commit-id",
                            "--name-only",
                            "--no-renames",
                            "-z",
                            "HEAD",
                        ],
                    )?,
                );
                "the root commit".to_string()
            }
        }
        Tier::Push => match git(root, &["merge-base", "HEAD", "origin/main"]) {
            Err(e) => {
                return Ok((
                    Vec::new(),
                    format!(
                        "warning: no merge base of HEAD and origin/main ({}), so tier push runs every line",
                        one_line(&format!("{e:#}"))
                    ),
                ));
            }
            Ok(base) => {
                let base = base.trim();
                add_paths(
                    &mut found,
                    &git(root, &["diff", "--name-only", "--no-renames", "-z", base])?,
                );
                add_paths(
                    &mut found,
                    &git(
                        root,
                        &["diff", "--name-only", "--no-renames", "-z", base, "HEAD"],
                    )?,
                );
                add_paths(&mut found, &git(root, &untracked)?);
                let short = base.get(..12).unwrap_or(base);
                format!(
                    "the changes since the merge base {short} with origin/main, and the untracked files"
                )
            }
        },
    };
    Ok((found.into_iter().collect(), source))
}

/// The workspace graph of `root`, from `cargo metadata --locked`.
pub fn graph(root: &Path) -> Result<Graph> {
    let metadata = rules::workspace_metadata(root)?;
    Ok(Graph::from_workspace(&rules::workspace(&metadata)?))
}

/// How a check ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It exited with this code.
    Exit(i32),
    /// A signal stopped it.
    Signal(i32),
}

/// Runs each `run` check of `plan` in order, with `run`, and writes the
/// progress to `out`.  It stops at the first failure, names each check
/// that did not run, and gives 1.  Else it gives 0.  A runner error is a
/// failure.
pub fn run_checks(
    plan: &Plan,
    out: &mut impl Write,
    mut run: impl FnMut(&Check) -> Result<Outcome>,
) -> i32 {
    let started = Instant::now();
    let runs: Vec<&Check> = plan
        .choices
        .iter()
        .filter(|choice| choice.decision == Decision::Run)
        .map(|choice| &choice.check)
        .collect();
    // A write to the terminal that fails cannot be reported there.
    let mut say = |line: String| {
        let _ = writeln!(out, "{line}");
    };
    for (index, check) in runs.iter().enumerate() {
        say(format!("check: start: {}", check.line()));
        let one = Instant::now();
        let failure = match run(check) {
            Ok(Outcome::Exit(0)) => None,
            Ok(Outcome::Exit(code)) => Some(format!("exit {code}")),
            Ok(Outcome::Signal(signal)) => Some(format!("signal {signal}")),
            Err(e) => Some(format!("{e:#}")),
        };
        let seconds = one.elapsed().as_secs_f64();
        let Some(failure) = failure else {
            say(format!("check: ok in {seconds:.1} s"));
            continue;
        };
        say(format!("check: failed ({failure}) in {seconds:.1} s"));
        for rest in &runs[index + 1..] {
            say(format!("check: not run: {}", rest.line()));
        }
        say(format!(
            "check: {} failed, after {index} passed, in {:.1} s",
            check.line(),
            started.elapsed().as_secs_f64()
        ));
        return FAILED;
    }
    say(format!(
        "check: {} passed in {:.1} s",
        runs.len(),
        started.elapsed().as_secs_f64()
    ));
    0
}

/// Runs `check` in its folder under `root`, with the terminal as its
/// output.
pub fn run_command(root: &Path, check: &Check) -> Result<Outcome> {
    let (program, args) = check
        .argv
        .split_first()
        .context("the check has no program")?;
    let status = Command::new(program)
        .args(args)
        .envs(check.env.iter().map(|(key, value)| (key, value)))
        .current_dir(root.join(&check.folder))
        .status()
        .with_context(|| format!("starting {}", check.line()))?;
    Ok(match status.code() {
        Some(code) => Outcome::Exit(code),
        None => Outcome::Signal(signal_of(&status)),
    })
}

#[cfg(unix)]
fn signal_of(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.signal().unwrap_or(0)
}

#[cfg(not(unix))]
fn signal_of(_: &std::process::ExitStatus) -> i32 {
    0
}

/// The tier and `--dry-run` of the arguments, or `None` on a usage error.
fn parse_args(args: &[String]) -> Option<(Tier, bool)> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let (tier, dry_run) = match words.as_slice() {
        ["--tier", tier] => (*tier, false),
        ["--tier", tier, "--dry-run"] | ["--dry-run", "--tier", tier] => (*tier, true),
        _ => return None,
    };
    let tier = match tier {
        "commit" => Tier::Commit,
        "push" => Tier::Push,
        _ => return None,
    };
    Some((tier, dry_run))
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Commit => "commit",
        Tier::Push => "push",
    }
}

/// Runs `xtask check` from the shell, and gives the exit code.
pub fn main(args: &[String]) -> i32 {
    let Some((tier, dry_run)) = parse_args(args) else {
        eprintln!("{USAGE}");
        return REFUSED;
    };
    let root = match std::env::current_dir()
        .context("reading the current folder")
        .and_then(|dir| repo_root(&dir))
    {
        Ok(root) => root,
        Err(e) => {
            eprintln!("{e:#}");
            return REFUSED;
        }
    };
    println!("check: tier {}", tier_name(tier));
    let (paths, source) = match changed_paths(&root, tier) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("{e:#}");
            return REFUSED;
        }
    };
    if source.starts_with("warning:") {
        println!("{source}");
        println!("check: paths: {}", paths.len());
    } else {
        println!("check: paths: {}, from {source}", paths.len());
    }
    // Tier `push` and a strict hit need no graph.
    let graph = if tier == Tier::Commit && strict_hits(&paths).is_empty() {
        match graph(&root) {
            Ok(graph) => Some(graph),
            Err(e) => {
                println!(
                    "warning: cannot read the workspace graph: {}",
                    one_line(&format!("{e:#}"))
                );
                None
            }
        }
    } else {
        None
    };
    let require = std::env::var(REQUIRE).ok();
    let plan = with_gpu_require(plan(tier, &paths, graph.as_ref()), require.as_deref());
    for (rule, path) in &plan.strict {
        println!("check: strict {rule}: {path}");
    }
    println!("check: tier {} runs: {}", tier_name(plan.tier), plan.why);
    for choice in &plan.choices {
        let word = match choice.decision {
            Decision::Run => "run",
            Decision::Skip => "skip",
        };
        println!("check: {word}: {} ({})", choice.check.line(), choice.reason);
    }
    if dry_run {
        return 0;
    }
    run_checks(&plan, &mut std::io::stdout(), |check| {
        run_command(&root, check)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FMT: &str = "cargo fmt --all --check";
    const DOCS: &str = "cargo run -p xtask -- docs-check";
    const CLIPPY: &str = "cargo clippy --workspace --all-targets -- -D warnings";
    const SYNTH: &str = "cargo clippy -p synth --all-targets -- -D warnings";
    const APP: &str = "cargo clippy -p app -- -D warnings";
    const APP_ALL: &str = "cargo clippy -p app --all-targets -- -D warnings";
    const WASM: &str = "cargo clippy --target wasm32-unknown-unknown -p app -p stem-worker -p decode-worker -p stem-protocol -p engine --all-targets -- -D warnings";
    const XTASK_REPO: &str = "cargo test -p xtask --lib --test rules --test docs --test review";
    const TESTS: &str = "cargo test --workspace --exclude roformer-gpu";
    const GPU_LIB: &str = "cargo test -p roformer-gpu --lib";
    const GPU_DOC: &str = "cargo test -p roformer-gpu --doc";
    const GPU: &str = "STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release";
    const TRUNK: &str = "(cd app && trunk build --release)";
    const SERVE: &str = "cargo run -p xtask --release -- serve app/dist --port 8080";
    const WEB_TEST: &str = "cargo run -p xtask -- web-test";
    const E2E: &str = "cargo run -p xtask -- e2e <scenario>";
    const NIGHTLY_DOC: &str = "cargo +nightly-2026-10-30 test -p roformer-gpu --doc";
    const PUBLISH: &str = "cargo run -p xtask --release -- publish";

    /// The 11 members of this repo and their real edges, of all kinds.
    fn fixture() -> Graph {
        Graph::new(&[
            ("app", "app", &["dsp", "engine", "synth", "stem-protocol"]),
            (
                "decoder",
                "decode-worker",
                &["dsp", "engine", "stem-protocol"],
            ),
            ("dsp", "dsp", &[]),
            ("engine", "engine", &["dsp"]),
            ("protocol", "stem-protocol", &[]),
            ("roformer", "roformer", &["test-support"]),
            (
                "roformer-gpu",
                "roformer-gpu",
                &["roformer", "test-support"],
            ),
            ("synth", "synth", &[]),
            ("test-support", "test-support", &[]),
            (
                "worker",
                "stem-worker",
                &["roformer", "roformer-gpu", "stem-protocol"],
            ),
            ("xtask", "xtask", &["dsp", "engine", "roformer", "synth"]),
        ])
    }

    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    /// The plan as `run: <line>` and `skip: <line>`, in order.
    fn decisions(plan: &Plan) -> Vec<String> {
        plan.choices
            .iter()
            .map(|choice| match choice.decision {
                Decision::Run => format!("run: {}", choice.check.line()),
                Decision::Skip => format!("skip: {}", choice.check.line()),
            })
            .collect()
    }

    fn commit(given: &[&str]) -> Plan {
        plan(Tier::Commit, &paths(given), Some(&fixture()))
    }

    fn run(line: &str) -> String {
        format!("run: {line}")
    }

    fn skip(line: &str) -> String {
        format!("skip: {line}")
    }

    /// The skips at the end of each plan of tier `commit`.
    fn commit_tail() -> Vec<String> {
        [GPU, TRUNK, SERVE, WEB_TEST, E2E, NIGHTLY_DOC, PUBLISH]
            .map(skip)
            .to_vec()
    }

    /// The skips at the end of each plan of tier `push`.
    fn push_tail() -> Vec<String> {
        [SERVE, WEB_TEST, E2E, NIGHTLY_DOC, PUBLISH]
            .map(skip)
            .to_vec()
    }

    fn with_tail(head: Vec<String>, tail: Vec<String>) -> Vec<String> {
        head.into_iter().chain(tail).collect()
    }

    /// Tier `push` with the GPU test line decided as `gpu`.
    fn push_list(gpu: String) -> Vec<String> {
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            run(SYNTH),
            run(APP),
            run(APP_ALL),
            run(WASM),
            run(TESTS),
            run(GPU_LIB),
            run(GPU_DOC),
            gpu,
            run(TRUNK),
        ];
        with_tail(head, push_tail())
    }

    /// Tier `commit` for paths in no member: the first four rows.
    fn no_member_list() -> Vec<String> {
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            skip(SYNTH),
            skip(APP),
            skip(APP_ALL),
            skip(WASM),
            run(XTASK_REPO),
            skip(TESTS),
            skip(GPU_LIB),
            skip(GPU_DOC),
        ];
        with_tail(head, commit_tail())
    }

    #[test]
    fn lines_are_the_twelve_of_the_push_tier() {
        let got: Vec<String> = lines().iter().map(Check::line).collect();
        assert_eq!(
            got,
            [
                FMT, DOCS, CLIPPY, SYNTH, APP, APP_ALL, WASM, TESTS, GPU_LIB, GPU_DOC, GPU, TRUNK
            ]
        );
        let gpu = &lines()[10];
        assert_eq!(
            gpu.env,
            [("STEMYLA_REQUIRE".to_string(), "gpu,f16".to_string())]
        );
        assert_eq!(lines()[11].folder, "app");
        assert_eq!(lines()[11].argv, ["trunk", "build", "--release"]);
    }

    #[test]
    fn a_docs_path_runs_the_first_four_rows_and_no_crate_tests() {
        let plan = commit(&["docs/x.md"]);
        assert_eq!(plan.tier, Tier::Commit);
        assert_eq!(decisions(&plan), no_member_list());
    }

    #[test]
    fn no_path_runs_the_first_four_rows_only() {
        assert_eq!(decisions(&commit(&[])), no_member_list());
    }

    /// Tier `commit` for a change in `dsp`.
    fn dsp_list() -> Vec<String> {
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            skip(SYNTH),
            run(APP),
            run(APP_ALL),
            run(WASM),
            skip(XTASK_REPO),
            run("cargo test -p app -p decode-worker -p dsp -p engine -p xtask"),
            skip(TESTS),
            skip(GPU_LIB),
            skip(GPU_DOC),
        ];
        with_tail(head, commit_tail())
    }

    #[test]
    fn a_dsp_path_tests_its_dependents_in_one_command() {
        assert_eq!(decisions(&commit(&["dsp/src/lib.rs"])), dsp_list());
    }

    #[test]
    fn a_deleted_path_counts_as_a_change_of_its_member() {
        assert_eq!(decisions(&commit(&["dsp/src/gone.rs"])), dsp_list());
    }

    #[test]
    fn a_synth_path_runs_the_synth_app_and_wasm32_clippy_lines() {
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            run(SYNTH),
            run(APP),
            run(APP_ALL),
            run(WASM),
            skip(XTASK_REPO),
            run("cargo test -p app -p synth -p xtask"),
            skip(TESTS),
            skip(GPU_LIB),
            skip(GPU_DOC),
        ];
        assert_eq!(
            decisions(&commit(&["synth/src/lib.rs"])),
            with_tail(head, commit_tail())
        );
    }

    #[test]
    fn a_roformer_path_is_in_roformer_and_runs_the_roformer_gpu_tests() {
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            skip(SYNTH),
            skip(APP),
            skip(APP_ALL),
            run(WASM),
            skip(XTASK_REPO),
            run("cargo test -p roformer -p stem-worker -p xtask"),
            skip(TESTS),
            run(GPU_LIB),
            run(GPU_DOC),
        ];
        assert_eq!(
            decisions(&commit(&["roformer/src/x.rs"])),
            with_tail(head, commit_tail())
        );
    }

    #[test]
    fn a_roformer_gpu_path_is_in_roformer_gpu_only() {
        assert_eq!(
            fixture().member_of("roformer-gpu/src/x.rs"),
            Some("roformer-gpu")
        );
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            skip(SYNTH),
            skip(APP),
            skip(APP_ALL),
            run(WASM),
            run(XTASK_REPO),
            run("cargo test -p stem-worker"),
            skip(TESTS),
            run(GPU_LIB),
            run(GPU_DOC),
        ];
        assert_eq!(
            decisions(&commit(&["roformer-gpu/src/x.rs"])),
            with_tail(head, commit_tail())
        );
    }

    #[test]
    fn an_engine_path_tests_engine_and_app_in_one_command() {
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            skip(SYNTH),
            run(APP),
            run(APP_ALL),
            run(WASM),
            skip(XTASK_REPO),
            run("cargo test -p app -p decode-worker -p engine -p xtask"),
            skip(TESTS),
            skip(GPU_LIB),
            skip(GPU_DOC),
        ];
        let plan = commit(&["engine/src/decode.rs"]);
        assert_eq!(decisions(&plan), with_tail(head, commit_tail()));
        let tests: Vec<&Choice> = plan
            .choices
            .iter()
            .filter(|c| {
                c.decision == Decision::Run && c.check.argv.get(1).is_some_and(|w| w == "test")
            })
            .collect();
        assert_eq!(tests.len(), 1, "the crate tests are one command");
    }

    #[test]
    fn a_path_with_a_space_stays_whole() {
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            skip(SYNTH),
            run(APP),
            run(APP_ALL),
            run(WASM),
            run(XTASK_REPO),
            run("cargo test -p app"),
            skip(TESTS),
            skip(GPU_LIB),
            skip(GPU_DOC),
        ];
        assert_eq!(
            decisions(&commit(&["app/src/a b.rs"])),
            with_tail(head, commit_tail())
        );
    }

    /// No one member of this repo has all 11 members as its dependents.
    /// So the paths of four members give the workspace test line.
    #[test]
    fn the_crate_tests_of_every_member_but_roformer_gpu_are_the_workspace_test_line() {
        let graph = fixture();
        for name in graph.names() {
            let one = BTreeSet::from([name.clone()]);
            assert_ne!(graph.dependents(&one), graph.names(), "{name}");
        }
        let all =
            BTreeSet::from(["dsp", "test-support", "stem-protocol", "synth"].map(str::to_string));
        assert_eq!(graph.dependents(&all), graph.names());
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            run(SYNTH),
            run(APP),
            run(APP_ALL),
            run(WASM),
            skip(XTASK_REPO),
            run(TESTS),
            run(GPU_LIB),
            run(GPU_DOC),
        ];
        let plan = commit(&[
            "dsp/src/lib.rs",
            "test-support/src/lib.rs",
            "protocol/tests/x.rs",
            "synth/src/lib.rs",
        ]);
        assert_eq!(decisions(&plan), with_tail(head, commit_tail()));
    }

    #[test]
    fn a_workspace_file_runs_every_line_but_the_gpu_line_and_trunk() {
        let plan = commit(&[".cargo/config.toml"]);
        assert_eq!(plan.tier, Tier::Commit);
        let head = vec![
            run(FMT),
            run(DOCS),
            run(CLIPPY),
            run(SYNTH),
            run(APP),
            run(APP_ALL),
            run(WASM),
            run(TESTS),
            run(GPU_LIB),
            run(GPU_DOC),
        ];
        assert_eq!(decisions(&plan), with_tail(head, commit_tail()));
    }

    #[test]
    fn a_strict_path_runs_tier_push() {
        let plan = commit(&["Cargo.lock"]);
        assert_eq!(plan.tier, Tier::Push);
        assert_eq!(
            plan.strict,
            [("DEPS-1".to_string(), "Cargo.lock".to_string())]
        );
        // `Cargo.lock` is a workspace file, so the GPU test line runs.
        assert_eq!(decisions(&plan), push_list(run(GPU)));
    }

    #[test]
    fn an_rt_1_path_runs_tier_push_with_no_gpu_line() {
        let plan = commit(&["engine/src/slot.rs"]);
        assert_eq!(plan.tier, Tier::Push);
        assert_eq!(
            plan.strict,
            [("RT-1".to_string(), "engine/src/slot.rs".to_string())]
        );
        assert_eq!(decisions(&plan), push_list(skip(GPU)));
    }

    #[test]
    fn tier_commit_with_no_graph_runs_tier_push() {
        let plan = plan(Tier::Commit, &paths(&["docs/x.md"]), None);
        assert_eq!(plan.tier, Tier::Push);
        assert!(plan.strict.is_empty());
        assert_eq!(decisions(&plan), push_list(skip(GPU)));
    }

    #[test]
    fn tier_push_runs_the_gpu_line_for_a_gpu_path() {
        let plan = plan(Tier::Push, &paths(&["roformer-gpu/src/x.rs"]), None);
        assert_eq!(plan.tier, Tier::Push);
        assert_eq!(decisions(&plan), push_list(run(GPU)));
        assert_eq!(
            reason_of(&plan, GPU),
            "a path is in roformer/, roformer-gpu/ or test-support/, or is a workspace file"
        );
        for folder in [
            "roformer/x.rs",
            "test-support/src/lib.rs",
            "rust-toolchain.toml",
        ] {
            let plan = super::plan(Tier::Push, &paths(&[folder]), None);
            assert_eq!(decisions(&plan), push_list(run(GPU)), "{folder}");
        }
    }

    #[test]
    fn tier_push_skips_the_gpu_line_for_a_docs_path_with_its_reason() {
        let plan = plan(Tier::Push, &paths(&["docs/x.md"]), Some(&fixture()));
        assert_eq!(decisions(&plan), push_list(skip(GPU)));
        let gpu = plan.choices.iter().find(|c| c.check.line() == GPU).unwrap();
        assert!(gpu.reason.contains("no GPU code"), "{}", gpu.reason);
    }

    /// With no path, tier `push` cannot rule out a GPU change: no path
    /// changed since the merge base, or the merge base is not known.
    #[test]
    fn tier_push_with_no_path_runs_the_gpu_line() {
        let plan = plan(Tier::Push, &[], None);
        assert_eq!(decisions(&plan), push_list(run(GPU)));
        assert_eq!(
            reason_of(&plan, GPU),
            "no path changed, or no path is known, so a GPU change cannot be ruled out"
        );
    }

    const GPU_ALONE: &str = "STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release";
    const FROM_ENV: &str = "; STEMYLA_REQUIRE=gpu from the environment replaces gpu,f16";

    #[test]
    fn the_callers_stemyla_require_replaces_gpu_f16_and_the_reason_says_so() {
        let before = plan(Tier::Push, &[], None);
        let after = with_gpu_require(before.clone(), Some("gpu"));
        assert_eq!(decisions(&after), push_list(run(GPU_ALONE)));
        let reason = |plan: &Plan, line: &str| {
            plan.choices
                .iter()
                .find(|c| c.check.line() == line)
                .map(|c| c.reason.clone())
                .unwrap_or_default()
        };
        assert_eq!(
            reason(&after, GPU_ALONE),
            format!("{}{FROM_ENV}", reason(&before, GPU))
        );
        assert_eq!((after.tier, &after.why), (before.tier, &before.why));
        for (old, new) in before.choices.iter().zip(&after.choices) {
            if old.check.line() != GPU {
                assert_eq!(old, new);
            }
        }
    }

    #[test]
    fn the_override_also_names_the_skipped_gpu_line_of_tier_commit() {
        let before = commit(&["docs/x.md"]);
        let after = with_gpu_require(before.clone(), Some(" gpu "));
        let mut want = no_member_list();
        want[11] = skip(GPU_ALONE);
        assert_eq!(decisions(&after), want);
        assert_eq!(
            after.choices[11].reason,
            format!("{}{FROM_ENV}", before.choices[11].reason)
        );
    }

    #[test]
    fn no_or_an_empty_stemyla_require_keeps_gpu_f16() {
        for given in [&[][..], &["docs/x.md"]] {
            let plan = super::plan(Tier::Push, &paths(given), None);
            for require in [None, Some(""), Some("  "), Some("gpu,f16")] {
                assert_eq!(with_gpu_require(plan.clone(), require), plan, "{require:?}");
            }
        }
    }

    /// The reason of the check with `line` in `plan`, or an empty string.
    fn reason_of(plan: &Plan, line: &str) -> String {
        plan.choices
            .iter()
            .find(|c| c.check.line() == line)
            .map(|c| c.reason.clone())
            .unwrap_or_default()
    }

    /// A value that names no GPU need keeps `gpu,f16`.  Else the GPU test
    /// line could pass with no adapter (VERIFY-1).
    #[test]
    fn a_stemyla_require_with_no_gpu_word_keeps_gpu_f16_and_the_reason_says_so() {
        let before = plan(Tier::Push, &[], None);
        for (given, shown) in [
            ("fixtures", "fixtures"),
            (" fixtures,gpus ", "fixtures,gpus"),
            ("x, gpu f16", "x, gpu f16"),
        ] {
            let after = with_gpu_require(before.clone(), Some(given));
            assert_eq!(decisions(&after), push_list(run(GPU)), "{given:?}");
            assert_eq!(
                reason_of(&after, GPU),
                format!(
                    "{}; STEMYLA_REQUIRE={shown} from the environment is not used, since it names no GPU need (gpu, f16 or timestamps)",
                    reason_of(&before, GPU)
                ),
                "{given:?}"
            );
            assert_eq!((after.tier, &after.why), (before.tier, &before.why));
            for (old, new) in before.choices.iter().zip(&after.choices) {
                if old.check.line() != GPU {
                    assert_eq!(old, new, "{given:?}");
                }
            }
        }
    }

    /// [`GPU_WORDS`] is `gpu` and the words of `IMPLY_GPU` in
    /// `test-support`.  A change of `IMPLY_GPU` fails this test.
    #[test]
    fn gpu_words_are_gpu_and_the_words_that_imply_it() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-support/src/lib.rs");
        let text = std::fs::read_to_string(path).unwrap();
        let imply: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("const IMPLY_GPU"))
            .collect();
        assert_eq!(
            imply,
            ["const IMPLY_GPU: [Need; 2] = [Need::Timestamps, Need::F16];"]
        );
        // The words of `Need::word` for the three GPU needs.
        for word in [
            r#"Need::Gpu => "gpu","#,
            r#"Need::Timestamps => "timestamps","#,
            r#"Need::F16 => "f16","#,
        ] {
            assert!(
                text.lines().any(|line| line.trim() == word),
                "test-support/src/lib.rs has no line `{word}`, so GPU_WORDS can be stale"
            );
        }
        assert_eq!(GPU_WORDS, ["gpu", "f16", "timestamps"]);
    }

    /// A value replaces `gpu,f16` when one of its words, trimmed, is
    /// `gpu`, `f16` or `timestamps`.
    #[test]
    fn a_stemyla_require_with_a_gpu_word_replaces_gpu_f16() {
        let before = plan(Tier::Push, &[], None);
        for given in ["gpu,fixtures", "fixtures, f16", "timestamps"] {
            let line = format!("STEMYLA_REQUIRE={given} cargo test -p roformer-gpu --release");
            let after = with_gpu_require(before.clone(), Some(given));
            assert_eq!(decisions(&after), push_list(run(&line)), "{given:?}");
            assert_eq!(
                reason_of(&after, &line),
                format!(
                    "{}; STEMYLA_REQUIRE={given} from the environment replaces gpu,f16",
                    reason_of(&before, GPU)
                )
            );
        }
    }

    #[test]
    fn tier_commit_always_skips_the_gpu_line_and_trunk_with_their_reasons() {
        let plan = commit(&["roformer-gpu/src/x.rs"]);
        let reason = |line: &str| {
            plan.choices
                .iter()
                .find(|c| c.check.line() == line)
                .map(|c| c.reason.clone())
                .unwrap_or_default()
        };
        assert!(
            reason(GPU).starts_with(
                "it needs a GPU adapter, and tier push runs it when its paths change GPU code;"
            ),
            "{}",
            reason(GPU)
        );
        assert!(
            reason(GPU).contains("counts for GPU-3 only on a real GPU named in the PR"),
            "{}",
            reason(GPU)
        );
        assert!(
            reason(TRUNK).contains("tier push runs it"),
            "{}",
            reason(TRUNK)
        );
        assert!(reason(E2E).contains("decision 0026"), "{}", reason(E2E));
        assert!(
            reason(NIGHTLY_DOC).contains("CI runs it"),
            "{}",
            reason(NIGHTLY_DOC)
        );
        assert!(reason(PUBLISH).contains("GIT-1"), "{}", reason(PUBLISH));
    }

    /// Each set of paths of these tests.
    const ALL_PATHS: &[&[&str]] = &[
        &[],
        &["docs/x.md"],
        &["docs/a b.md"],
        &["dsp/src/lib.rs"],
        &["synth/src/lib.rs"],
        &["roformer/src/x.rs"],
        &["roformer-gpu/src/x.rs"],
        &["engine/src/decode.rs"],
        &["app/src/a b.rs"],
        &[".cargo/config.toml"],
        &["Cargo.lock"],
        &["engine/src/slot.rs"],
        &[
            "dsp/src/lib.rs",
            "test-support/src/lib.rs",
            "protocol/tests/x.rs",
            "synth/src/lib.rs",
        ],
    ];

    #[test]
    fn check_never_runs_the_browser_or_publish() {
        let graph = fixture();
        for tier in [Tier::Commit, Tier::Push] {
            for graph in [Some(&graph), None] {
                for given in ALL_PATHS {
                    let plan = plan(tier, &paths(given), graph);
                    let runs: Vec<&Choice> = plan
                        .choices
                        .iter()
                        .filter(|c| c.decision == Decision::Run)
                        .collect();
                    assert!(!runs.is_empty(), "{tier:?} {given:?} runs a check");
                    for choice in runs {
                        for word in &choice.check.argv {
                            assert!(
                                !["serve", "web-test", "e2e", "publish"].contains(&word.as_str()),
                                "{tier:?} {given:?} runs {}",
                                choice.check.line()
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_strict_path_table_follows_strict_rules() {
        let ids: Vec<String> = STRICT_PATHS.iter().map(|(id, _)| id.to_string()).collect();
        assert_eq!(ids, strict_rules());
        for (id, globs) in STRICT_PATHS {
            let empty = ["VERIFY-1", "GIT-1"].contains(id);
            assert_eq!(globs.is_empty(), empty, "{id}");
        }
    }

    #[test]
    fn the_fixture_graph_is_the_real_graph() {
        let metadata =
            rules::workspace_metadata(&crate::workspace_root()).expect("cargo metadata runs");
        let workspace =
            rules::workspace(&metadata).expect("cargo metadata's JSON gives the workspace");
        assert_eq!(Graph::from_workspace(&workspace), fixture());
    }

    /// `GPU_FOLDERS` is the closure of `roformer-gpu` in the real graph, so
    /// a new edge of `roformer-gpu` fails this test or
    /// `the_fixture_graph_is_the_real_graph`.
    #[test]
    fn gpu_folders_are_the_closure_of_roformer_gpu() {
        let graph = fixture();
        let closure = graph.closure(&BTreeSet::from([ROFORMER_GPU.to_string()]));
        let folders: BTreeSet<String> = closure
            .iter()
            .map(|name| format!("{}/", graph.members[name].0))
            .collect();
        let want: BTreeSet<String> = GPU_FOLDERS.iter().map(|f| f.to_string()).collect();
        assert_eq!(folders, want);
    }

    #[test]
    fn a_path_is_in_the_member_with_the_longest_folder() {
        let graph = Graph::new(&[("a", "outer", &[]), ("a/b", "inner", &[])]);
        assert_eq!(graph.member_of("a/b/x.rs"), Some("inner"));
        assert_eq!(graph.member_of("a/x.rs"), Some("outer"));
        assert_eq!(graph.member_of("a/bc/x.rs"), Some("outer"));
        assert_eq!(graph.member_of("ab/x.rs"), None);
        assert_eq!(graph.member_of("a"), None);
        let graph = Graph::new(&[("a/b", "inner", &[]), ("a", "zouter", &[])]);
        assert_eq!(graph.member_of("a/b/x.rs"), Some("inner"));
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn parse_args_takes_the_tier_and_dry_run_in_either_order() {
        for (words, want) in [
            (&["--tier", "commit"][..], Some((Tier::Commit, false))),
            (&["--tier", "push"], Some((Tier::Push, false))),
            (
                &["--tier", "commit", "--dry-run"],
                Some((Tier::Commit, true)),
            ),
            (
                &["--dry-run", "--tier", "commit"],
                Some((Tier::Commit, true)),
            ),
            (&["--tier", "push", "--dry-run"], Some((Tier::Push, true))),
            (&["--dry-run", "--tier", "push"], Some((Tier::Push, true))),
            (&[], None),
            (&["--dry-run"], None),
            (&["--tier"], None),
            (&["--tier", "nothing"], None),
            (&["--dry-run", "--tier", "nothing"], None),
            (&["--tier", "commit", "--now"], None),
            (&["--tier", "commit", "--dry-run", "--dry-run"], None),
            (&["commit"], None),
        ] {
            assert_eq!(parse_args(&args(words)), want, "{words:?}");
        }
    }

    #[test]
    fn one_line_joins_the_trimmed_lines_that_are_not_empty() {
        assert_eq!(
            one_line("cargo metadata failed: error: no\n\n   Caused by:\n  \t\n  a b  \nend"),
            "cargo metadata failed: error: no; Caused by:; a b; end"
        );
        assert_eq!(one_line("one"), "one");
        assert_eq!(one_line("\n \n"), "");
    }

    /// The rules that `path` hits.
    fn hits(path: &str) -> Vec<String> {
        strict_hits(&paths(&[path]))
            .into_iter()
            .map(|(rule, hit)| {
                assert_eq!(hit, path);
                rule
            })
            .collect()
    }

    /// Each path of `hit` hits `rule`, and each path of `near` does not.
    fn assert_row(rule: &str, hit: &[&str], near: &[&str]) {
        for path in hit {
            assert!(hits(path).contains(&rule.to_string()), "{path} hits {rule}");
        }
        for path in near {
            assert!(
                !hits(path).contains(&rule.to_string()),
                "{path} does not hit {rule}"
            );
        }
    }

    #[test]
    fn proto_1_has_the_codecs_and_the_hand_written_messages() {
        assert_row(
            "PROTO-1",
            &[
                "protocol/src/lib.rs",
                "protocol/src/decode.rs",
                "xtask/e2e/worker.js",
                "xtask/e2e/save_spy.js",
                "xtask/src/e2e.rs",
                "xtask/src/e2e/decode.rs",
                "xtask/src/e2e/weights.rs",
            ],
            &[
                "protocol/tests/x.rs",
                "xtask/e2e/blank.html",
                "xtask/src/e2e_x.rs",
            ],
        );
    }

    #[test]
    fn proto_2_has_the_site_check_and_its_callers() {
        assert_row(
            "PROTO-2",
            &[
                "protocol/src/lib.rs",
                "app/src/separator.rs",
                "worker/src/web.rs",
            ],
            &[
                "protocol/src/decode.rs",
                "app/src/separator_x.rs",
                "worker/src/web/x.rs",
            ],
        );
    }

    #[test]
    fn mem_1_has_samples_and_the_clippy_files() {
        assert_row(
            "MEM-1",
            &[
                "engine/src/song.rs",
                "engine/src/test_alloc.rs",
                "app/clippy.toml",
                "clippy.toml",
            ],
            &["engine/src/songs.rs", "app/clippy.txt", "engine/tests/x.rs"],
        );
    }

    #[test]
    fn mem_2_has_the_transfer_lists() {
        assert_row(
            "MEM-2",
            &[
                "protocol/src/decode.rs",
                "app/src/separator.rs",
                "app/src/decoder/client.rs",
                "worker/src/web.rs",
                "decoder/src/web.rs",
            ],
            &["app/src/decoder/x.rs", "decoder/src/pass.rs"],
        );
    }

    #[test]
    fn mem_3_has_gpu_buffer_and_its_lints() {
        assert_row(
            "MEM-3",
            &[
                "roformer-gpu/src/lib.rs",
                "roformer-gpu/src/net.rs",
                "roformer-gpu/clippy.toml",
            ],
            &["roformer-gpu/src/x.rs", "roformer-gpu/tests/x.rs"],
        );
    }

    #[test]
    fn mem_4_has_the_chunk_and_the_weight_format() {
        assert_row(
            "MEM-4",
            &[
                "roformer/src/lib.rs",
                "roformer/src/weights.rs",
                "roformer-gpu/src/lib.rs",
            ],
            &["roformer/src/stft.rs", "roformer-gpu/src/net.rs"],
        );
    }

    #[test]
    fn mem_5_has_the_opfs_writes() {
        assert_row(
            "MEM-5",
            &[
                "app/src/store.rs",
                "app/src/export.rs",
                "app/src/streamtest.rs",
                "worker/src/web.rs",
                "worker/src/web/encode.rs",
                "worker/src/web/stems.rs",
                "decoder/src/web.rs",
            ],
            &[
                "app/src/reset.rs",
                "app/src/lib.rs",
                "worker/src/web/tests.rs",
            ],
        );
    }

    #[test]
    fn rt_1_has_the_audio_callback_and_its_sources() {
        assert_row(
            "RT-1",
            &[
                "engine/src/slot.rs",
                "engine/src/source.rs",
                "engine/src/player.rs",
                "engine/src/live.rs",
                "engine/src/readahead.rs",
                "engine/src/readahead/x.rs",
                "engine/src/stretch/a/b.rs",
                "engine/src/params.rs",
                "engine/tests/realtime.rs",
            ],
            &[
                "engine/src/decode.rs",
                "engine/src/test_alloc.rs",
                "engine/tests/x.rs",
            ],
        );
    }

    #[test]
    fn gpu_1_has_the_kernels_and_the_limits() {
        assert_row(
            "GPU-1",
            &["roformer-gpu/src/shaders/x.wgsl", "roformer-gpu/src/lib.rs"],
            &["roformer-gpu/src/bench.rs", "roformer-gpu/src/net.rs"],
        );
    }

    #[test]
    fn gpu_2_and_gpu_3_have_the_kernels_and_the_choice_of_defaults() {
        for rule in ["GPU-2", "GPU-3"] {
            assert_row(
                rule,
                &[
                    "roformer-gpu/src/shaders/x.wgsl",
                    "roformer-gpu/src/bench.rs",
                    "roformer-gpu/src/lib.rs",
                ],
                &["roformer-gpu/src/net.rs", "roformer/src/lib.rs"],
            );
        }
    }

    #[test]
    fn verify_1_and_git_1_have_no_path() {
        for given in ALL_PATHS.iter().flat_map(|p| p.iter()) {
            for rule in hits(given) {
                assert!(rule != "VERIFY-1" && rule != "GIT-1", "{given} hits {rule}");
            }
        }
        assert!(strict_paths("VERIFY-1").is_empty());
        assert!(strict_paths("GIT-1").is_empty());
        assert!(!strict_paths("DEPS-1").is_empty());
    }

    #[test]
    fn deps_1_has_the_manifests_the_lock_file_and_the_licences() {
        assert_row(
            "DEPS-1",
            &[
                "Cargo.toml",
                "app/Cargo.toml",
                "Cargo.lock",
                "THIRD-PARTY.md",
            ],
            &["docs/THIRD-PARTY.md", "app/Cargo.lock", "Cargo.toml.orig"],
        );
    }

    #[test]
    fn asset_1_has_the_ignore_file_and_each_audio_or_weight_path() {
        assert_row(
            "ASSET-1",
            &[".gitignore", "wav/a.WAV", "model.safetensors", "a/b.mp3"],
            &["app/.gitignore", "docs/wav.md", "a/mp3"],
        );
    }

    #[test]
    fn strict_hits_follow_the_rules_then_the_paths() {
        let got = strict_hits(&paths(&["docs/x.md", "worker/src/web.rs", "Cargo.lock"]));
        let want: Vec<(String, String)> = [
            ("PROTO-2", "worker/src/web.rs"),
            ("MEM-2", "worker/src/web.rs"),
            ("MEM-5", "worker/src/web.rs"),
            ("DEPS-1", "Cargo.lock"),
        ]
        .map(|(r, p)| (r.to_string(), p.to_string()))
        .to_vec();
        assert_eq!(got, want);
    }

    #[test]
    fn the_graph_of_a_workspace_has_its_members_and_edges_of_all_kinds() {
        let id = |name: &str, folder: &str| format!("path+file://{folder}#{name}@0.1.0");
        let dep = |name: &str, path: &str, kind: Option<&str>, target: Option<&str>| serde_json::json!({ "name": name, "source": null, "rename": null, "kind": kind, "target": target, "path": path });
        let package = |name: &str, folder: &str, deps: Vec<serde_json::Value>| {
            serde_json::json!({
                "id": id(name, folder), "name": name, "source": null,
                "manifest_path": format!("{folder}/Cargo.toml"),
                "dependencies": deps, "features": {},
            })
        };
        let serde = serde_json::json!({
            "name": "serde", "source": "registry+https://github.com/rust-lang/crates.io-index",
            "rename": null, "kind": null, "target": null, "req": "^1",
        });
        let members = [id("a", "/x/a"), id("b", "/x/b"), id("c", "/x/nested/c")];
        let nodes: Vec<serde_json::Value> = members
            .iter()
            .map(|m| serde_json::json!({ "id": m, "deps": [], "features": [] }))
            .collect();
        let metadata = serde_json::json!({
            "packages": [
                package("a", "/x/a", vec![serde]),
                package("b", "/x/b", vec![dep("a", "/x/a", Some("dev"), None)]),
                package("c", "/x/nested/c", vec![dep("b", "/x/b", None, Some("cfg(unix)"))]),
            ],
            "workspace_members": members,
            "workspace_root": "/x",
            "resolve": { "nodes": nodes },
        })
        .to_string();
        let workspace = rules::workspace(&metadata).unwrap();
        assert_eq!(
            Graph::from_workspace(&workspace),
            Graph::new(&[
                ("a", "a", &[]),
                ("b", "b", &["a"]),
                ("nested/c", "c", &["b"]),
            ])
        );
    }

    /// A plan that runs `true`, then the given checks, then `true`, and
    /// skips `false`.
    fn three_checks(second: &str) -> Plan {
        let choice = |words: &str, decision| Choice {
            check: Check::root(words),
            decision,
            reason: String::new(),
        };
        Plan {
            tier: Tier::Commit,
            why: String::new(),
            strict: Vec::new(),
            choices: vec![
                choice("true one", Decision::Run),
                choice("false skipped", Decision::Skip),
                choice(second, Decision::Run),
                choice("true three", Decision::Run),
            ],
        }
    }

    /// `text` with each time `in <n>.<n> s` at the end of a line as
    /// `in <t> s`, so a compare does not depend on the speed of the
    /// machine.  A line with a time in another form stays as it is.
    fn timeless(text: &str) -> String {
        let mut out = String::new();
        for line in text.lines() {
            let timed = line.rsplit_once(" in ").filter(|(_, time)| {
                time.strip_suffix(" s")
                    .and_then(|time| time.split_once('.'))
                    .is_some_and(|(whole, tenths)| {
                        !whole.is_empty()
                            && whole.bytes().all(|b| b.is_ascii_digit())
                            && tenths.len() == 1
                            && tenths.bytes().all(|b| b.is_ascii_digit())
                    })
            });
            match timed {
                Some((head, _)) => out.push_str(&format!("{head} in <t> s\n")),
                None => out.push_str(&format!("{line}\n")),
            }
        }
        out
    }

    #[test]
    fn timeless_replaces_only_a_time_at_the_end_of_a_line() {
        assert_eq!(
            timeless("check: ok in 12.3 s\ncheck: 3 passed in 0.0 s\n"),
            "check: ok in <t> s\ncheck: 3 passed in <t> s\n"
        );
        for kept in [
            "check: ok in 1.23 s",
            "check: ok in .2 s",
            "check: ok in 1. s",
            "check: ok in 1.2s",
            "check: ok in x.2 s",
            "check: ok in 1.x s",
            "check: start: true in one",
        ] {
            assert_eq!(timeless(kept), format!("{kept}\n"));
        }
    }

    /// The text of `run_checks` with a fake runner that reads the outcome
    /// from the first word, with each time as `in <t> s`.  `slow` takes
    /// 150 ms, so its time is not `0.0`.
    fn fake_run(plan: &Plan) -> (i32, String) {
        let (code, raw) = fake_run_raw(plan);
        (code, timeless(&raw))
    }

    /// [`fake_run`] with the times as `run_checks` prints them.
    fn fake_run_raw(plan: &Plan) -> (i32, String) {
        let mut out = Vec::new();
        let mut ran = Vec::new();
        let code = run_checks(plan, &mut out, |check| {
            ran.push(check.line());
            match check.argv[0].as_str() {
                "true" => Ok(Outcome::Exit(0)),
                "slow" => {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    Ok(Outcome::Exit(0))
                }
                "false" => Ok(Outcome::Exit(3)),
                "killed" => Ok(Outcome::Signal(9)),
                _ => bail!("no such program"),
            }
        });
        let text = String::from_utf8(out).unwrap();
        assert!(
            !ran.contains(&"false skipped".to_string()),
            "a skip never runs"
        );
        (code, text)
    }

    /// The time in seconds at the end of `line`, `... in <time> s`.
    fn time_of(line: &str) -> f64 {
        let (_, time) = line.rsplit_once(" in ").unwrap();
        time.strip_suffix(" s").unwrap().parse().unwrap()
    }

    #[test]
    fn run_checks_runs_each_run_check_and_gives_0() {
        let (code, raw) = fake_run_raw(&three_checks("slow two"));
        let ok: Vec<f64> = raw
            .lines()
            .filter(|line| line.starts_with("check: ok in "))
            .map(time_of)
            .collect();
        assert_eq!(ok.len(), 3, "{raw}");
        assert!(ok[0] < 0.1, "{raw}");
        assert!(ok[1] >= 0.1, "{raw}");
        assert!(ok[2] < 0.1, "{raw}");
        assert!(time_of(raw.lines().last().unwrap()) >= 0.1, "{raw}");
        let text = timeless(&raw);
        assert_eq!(code, 0);
        assert_eq!(
            text,
            "check: start: true one\n\
             check: ok in <t> s\n\
             check: start: slow two\n\
             check: ok in <t> s\n\
             check: start: true three\n\
             check: ok in <t> s\n\
             check: 3 passed in <t> s\n"
        );
    }

    #[test]
    fn run_checks_stops_at_a_failed_check_and_names_the_rest() {
        let (code, text) = fake_run(&three_checks("false two"));
        assert_eq!(code, 1);
        assert_eq!(
            text,
            "check: start: true one\n\
             check: ok in <t> s\n\
             check: start: false two\n\
             check: failed (exit 3) in <t> s\n\
             check: not run: true three\n\
             check: false two failed, after 1 passed, in <t> s\n"
        );
    }

    #[test]
    fn run_checks_fails_a_signal_and_a_runner_error() {
        let (code, text) = fake_run(&three_checks("killed two"));
        assert_eq!(code, 1);
        assert_eq!(
            text,
            "check: start: true one\n\
             check: ok in <t> s\n\
             check: start: killed two\n\
             check: failed (signal 9) in <t> s\n\
             check: not run: true three\n\
             check: killed two failed, after 1 passed, in <t> s\n"
        );
        let (code, text) = fake_run(&three_checks("missing two"));
        assert_eq!(code, 1);
        assert_eq!(
            text,
            "check: start: true one\n\
             check: ok in <t> s\n\
             check: start: missing two\n\
             check: failed (no such program) in <t> s\n\
             check: not run: true three\n\
             check: missing two failed, after 1 passed, in <t> s\n"
        );
    }

    /// The real runner, on `sh` in a folder of the temporary folder.  One
    /// test of `xtask/tests/check.rs` runs `xtask check` without
    /// `--dry-run`, in a temporary workspace.
    #[test]
    fn run_command_gives_the_exit_code_the_signal_and_a_start_error() {
        let root = std::env::temp_dir().join(format!("xtask-check-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let sh = |script: &str, folder: &str| Check {
            argv: ["sh", "-c", script].map(str::to_string).to_vec(),
            env: vec![("CHECK_WORD".to_string(), "seen".to_string())],
            folder: folder.to_string(),
        };
        let marked = "test \"$CHECK_WORD\" = seen && test \"$(basename \"$PWD\")\" = sub";
        assert_eq!(
            run_command(&root, &sh(marked, "sub")).unwrap(),
            Outcome::Exit(0)
        );
        assert_eq!(
            run_command(&root, &sh(marked, "")).unwrap(),
            Outcome::Exit(1)
        );
        assert_eq!(
            run_command(&root, &sh("exit 4", "")).unwrap(),
            Outcome::Exit(4)
        );
        assert_eq!(
            run_command(&root, &sh("kill -9 $$", "")).unwrap(),
            Outcome::Signal(9)
        );
        let missing = Check::root("no-such-program-for-xtask-check");
        let error = run_command(&root, &missing).unwrap_err();
        assert!(
            format!("{error:#}").contains("starting no-such-program-for-xtask-check"),
            "{error:#}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
