//! `xtask review`: chooses the reviewers from a diff, guards a review round,
//! and runs cargo-mutants on a round's diff.
//!
//! Each failure exits 3, so a skill that runs the command stops on it.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use crate::docs::glob_matches;

/// A reviewer, and the paths that route a diff to it.
pub struct Reviewer {
    pub name: &'static str,
    pub include: &'static [&'static str],
    pub exclude: &'static [&'static str],
}

/// The reviewers, in the order of the table in `harness-phase-3.md`.
/// `reviewer` reads every diff, so it has no paths.
pub const REVIEWERS: &[Reviewer] = &[
    Reviewer {
        name: "reviewer",
        include: &[],
        exclude: &[],
    },
    Reviewer {
        name: "gpu-reviewer",
        include: &[
            "roformer-gpu/**",
            "roformer/src/**",
            "roformer/clippy.toml",
            ".claude/rules/gpu.md",
        ],
        exclude: &[],
    },
    Reviewer {
        name: "memory-reviewer",
        include: &[
            "engine/**",
            "app/src/**",
            "worker/**",
            "protocol/**",
            "roformer/src/**",
            "roformer-gpu/src/**",
            "synth/src/**",
            "**/clippy.toml",
            ".claude/rules/memory.md",
        ],
        exclude: &[],
    },
    Reviewer {
        name: "wire-reviewer",
        include: &[
            "protocol/**",
            "worker/src/web.rs",
            "worker/src/web/**",
            "app/src/separator.rs",
            "xtask/src/e2e.rs",
            "xtask/e2e/**",
            ".claude/rules/protocol.md",
        ],
        exclude: &[],
    },
    Reviewer {
        name: "supply-chain-reviewer",
        include: &[
            "**/Cargo.toml",
            "Cargo.lock",
            "THIRD-PARTY.md",
            "LICENSE*",
            "tools/**",
            "**/build.rs",
            ".cargo/**",
            ".github/**",
            ".claude/**",
            ".mcp.json",
            ".gitignore",
            "xtask/src/serve.rs",
            "xtask/tests/serve.rs",
            "xtask/src/publish.rs",
            "xtask/tests/publish.rs",
            "xtask/src/rules.rs",
            "xtask/src/docs.rs",
            "xtask/tests/rules.rs",
            "xtask/tests/docs.rs",
            "test-support/**",
            "app/index.html",
            "app/Trunk.toml",
        ],
        exclude: &[],
    },
    Reviewer {
        name: "docs-reviewer",
        include: &["docs/**", "CLAUDE.md", "README.md", ".claude/rules/**"],
        exclude: &["docs/plans/**", "docs/reviews/**"],
    },
];

/// A path that matches one of these changes the review harness itself, so
/// it routes every reviewer.
pub const ROUTE_ALL: &[&str] = &[
    "xtask/src/review.rs",
    ".claude/agents/**",
    ".claude/skills/**",
    ".claude/hooks/**",
];

/// Include globs that may match no tracked file, each with the reason.
pub const MAY_MATCH_NOTHING: &[(&str, &str)] = &[
    ("**/build.rs", "no crate has a build script yet"),
    (".mcp.json", "the repo has no MCP config yet"),
];

/// Content signs, by reviewer.  A short denylist, not a full list.
pub const SIGNS: &[(&str, &[&str])] = &[
    // TD-26: copies of song- or model-sized buffers (MEM-1).
    (
        "memory-reviewer",
        &[
            "to_vec",
            "to_owned",
            "Vec::from",
            "extend_from_slice",
            "copy_to",
            ".concat()",
        ],
    ),
    // TD-27: GPU buffers made outside `GpuBuffer` (MEM-3).
    (
        "gpu-reviewer",
        &[
            "wgpu::",
            "create_buffer",
            "create_texture",
            "create_query_set",
        ],
    ),
    // PROTO-2: a model URL from another site.
    ("wire-reviewer", &["model_url"]),
    // TD-23: code or files that come in from outside the source tree.
    (
        "supply-chain-reviewer",
        &[
            "#[path",
            "include!",
            "include_str!",
            "include_bytes!",
            "unsafe",
            "Command::new",
        ],
    ),
];

const USAGE: &str =
    "usage: xtask review start <base> <round> | check <round> | mutants <round> [--config <file>]";

/// The exit code of every failure, refusal and usage error.
const REFUSED: i32 = 3;

/// Options for every `git diff`, so a person's config does not change the
/// output.  `core.quotePath=false` goes on every `git` call.
const DIFF_OPTIONS: &[&str] = &[
    "--no-ext-diff",
    "--no-color",
    "--no-textconv",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// Why a reviewer reads a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    EveryDiff,
    Path { path: String, glob: &'static str },
    Harness(String),
    Symlink(String),
    Sign { sign: &'static str, path: String },
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::EveryDiff => write!(f, "every diff"),
            Reason::Path { path, glob } => write!(f, "{path} ({glob})"),
            Reason::Harness(path) => write!(f, "{path} changes the review harness"),
            Reason::Symlink(path) => write!(f, "{path} is a symlink"),
            Reason::Sign { sign, path } => write!(f, "`{sign}` in {path}"),
        }
    }
}

/// The reviewers that a diff needs, in table order, each with its reasons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Route(pub Vec<(&'static str, Vec<Reason>)>);

impl Route {
    pub fn reviewers(&self) -> Vec<&'static str> {
        self.0.iter().map(|(name, _)| *name).collect()
    }

    pub fn reasons(&self, name: &str) -> Vec<String> {
        self.0
            .iter()
            .filter(|(n, _)| *n == name)
            .flat_map(|(_, reasons)| reasons.iter().map(ToString::to_string))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The route of a diff, from `git diff -z --raw -M` and `git diff -U0 -M`.
pub fn route(raw: &str, patch: &str) -> Route {
    let entries = parse_raw(raw);
    if entries.is_empty() {
        return Route::default();
    }
    let path_reasons = entries.iter().flat_map(|entry| {
        entry
            .paths
            .iter()
            .flat_map(move |path| reasons_for_path(path, entry.is_symlink()))
    });
    let sign_reasons = added_lines(patch)
        .into_iter()
        .flat_map(|(path, lines)| reasons_for_signs(&path, &lines));
    let all: Vec<(&'static str, Reason)> = std::iter::once(("reviewer", Reason::EveryDiff))
        .chain(path_reasons)
        .chain(sign_reasons)
        .collect();
    Route(
        REVIEWERS
            .iter()
            .map(|r| {
                let mine = all
                    .iter()
                    .filter(|(name, _)| *name == r.name)
                    .map(|(_, reason)| reason);
                (r.name, first_of_each(mine))
            })
            .filter(|(_, reasons)| !reasons.is_empty())
            .collect(),
    )
}

/// The reasons in order, each once.
fn first_of_each<'a>(reasons: impl Iterator<Item = &'a Reason>) -> Vec<Reason> {
    let mut unique: Vec<Reason> = Vec::new();
    for reason in reasons {
        if !unique.contains(reason) {
            unique.push(reason.clone());
        }
    }
    unique
}

fn is_harness(path: &str) -> bool {
    ROUTE_ALL.iter().any(|glob| glob_matches(glob, path))
}

fn reasons_for_path(path: &str, symlink: bool) -> Vec<(&'static str, Reason)> {
    if is_harness(path) {
        return REVIEWERS
            .iter()
            .map(|r| (r.name, Reason::Harness(path.to_string())))
            .collect();
    }
    let globs = REVIEWERS.iter().filter_map(|r| {
        let excluded = r.exclude.iter().any(|glob| glob_matches(glob, path));
        let glob = r.include.iter().find(|glob| glob_matches(glob, path))?;
        (!excluded).then(|| {
            (
                r.name,
                Reason::Path {
                    path: path.to_string(),
                    glob,
                },
            )
        })
    });
    let link = symlink.then(|| ("supply-chain-reviewer", Reason::Symlink(path.to_string())));
    globs.chain(link).collect()
}

fn reasons_for_signs(path: &str, lines: &[String]) -> Vec<(&'static str, Reason)> {
    let counts =
        path.ends_with(".rs") && !path.split('/').any(|s| s == "tests") && !is_harness(path);
    if !counts {
        return Vec::new();
    }
    SIGNS
        .iter()
        .filter(|(name, _)| {
            !REVIEWERS
                .iter()
                .filter(|r| r.name == *name)
                .flat_map(|r| r.include)
                .any(|glob| glob_matches(glob, path))
        })
        .flat_map(|(name, signs)| {
            signs
                .iter()
                .filter(|sign| lines.iter().any(|line| has_token(line, sign)))
                .map(|sign| {
                    (
                        *name,
                        Reason::Sign {
                            sign,
                            path: path.to_string(),
                        },
                    )
                })
        })
        .collect()
}

/// `sign` appears in `line` as a whole token: a sign that starts or ends
/// with a word character does not touch another word character there.
fn has_token(line: &str, sign: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let check_before = sign.starts_with(is_word);
    let check_after = sign.ends_with(is_word);
    line.match_indices(sign).any(|(at, _)| {
        let before = line[..at].chars().next_back().is_some_and(is_word);
        let after = line[at + sign.len()..].chars().next().is_some_and(is_word);
        !(check_before && before) && !(check_after && after)
    })
}

/// One entry of `git diff -z --raw`.
struct RawEntry {
    old_mode: String,
    new_mode: String,
    paths: Vec<String>,
}

impl RawEntry {
    fn is_symlink(&self) -> bool {
        self.old_mode == "120000" || self.new_mode == "120000"
    }
}

/// Parses `git diff -z --raw`: `:<old mode> <new mode> <old sha> <new sha>
/// <status>`, then one path, or two for a rename (`R<score>`) or a copy
/// (`C<score>`), each ended by NUL.
fn parse_raw(raw: &str) -> Vec<RawEntry> {
    let fields: Vec<&str> = raw.split('\0').collect();
    let mut entries = Vec::new();
    let mut at = 0;
    while at < fields.len() {
        let Some(meta) = fields[at].strip_prefix(':') else {
            at += 1;
            continue;
        };
        let parts: Vec<&str> = meta.split(' ').collect();
        let status = parts.get(4).copied().unwrap_or_default();
        let count = if status.starts_with(['R', 'C']) { 2 } else { 1 };
        let paths: Vec<String> = fields
            .iter()
            .skip(at + 1)
            .take(count)
            .map(ToString::to_string)
            .collect();
        entries.push(RawEntry {
            old_mode: parts.first().copied().unwrap_or_default().to_string(),
            new_mode: parts.get(1).copied().unwrap_or_default().to_string(),
            paths,
        });
        at += 1 + count;
    }
    entries
}

/// The added lines of each file in a `git diff -U0` patch, by the file's
/// new path.  A deleted file has no new path, and adds no lines.
fn added_lines(patch: &str) -> Vec<(String, Vec<String>)> {
    let mut files: Vec<(String, Vec<String>)> = Vec::new();
    let mut path: Option<String> = None;
    let mut in_hunk = false;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            path = None;
            in_hunk = false;
        } else if !in_hunk {
            if let Some(name) = line.strip_prefix("+++ ") {
                path = new_path(name);
            } else if line.starts_with("@@") {
                in_hunk = true;
            }
        } else if let (Some(added), Some(p)) = (line.strip_prefix('+'), &path) {
            match files.last_mut() {
                Some((last, lines)) if last == p => lines.push(added.to_string()),
                _ => files.push((p.clone(), vec![added.to_string()])),
            }
        }
    }
    files
}

/// The path of a `+++ ` header: `b/<path>`, maybe C-quoted, maybe with a
/// tab after it; `/dev/null` for a deleted file.
fn new_path(header: &str) -> Option<String> {
    let text = header.strip_suffix('\t').unwrap_or(header);
    let text = if text.starts_with('"') {
        unquote(text)
    } else {
        text.to_string()
    };
    text.strip_prefix("b/").map(ToString::to_string)
}

/// Undoes git's C quoting of a path: `"a\"b\303\251"` is `a"bé`.
fn unquote(quoted: &str) -> String {
    let inner = quoted
        .strip_prefix('"')
        .and_then(|q| q.strip_suffix('"'))
        .unwrap_or(quoted)
        .as_bytes();
    let mut bytes = Vec::new();
    let mut at = 0;
    while at < inner.len() {
        if inner[at] != b'\\' || at + 1 == inner.len() {
            bytes.push(inner[at]);
            at += 1;
            continue;
        }
        let next = inner[at + 1];
        let octal = inner
            .get(at + 1..at + 4)
            .filter(|digits| digits.iter().all(|d| (b'0'..=b'7').contains(d)));
        if let Some(digits) = octal {
            let value = digits.iter().fold(0u32, |v, d| v * 8 + u32::from(d - b'0'));
            bytes.push(u8::try_from(value).unwrap_or(b'?'));
            at += 4;
            continue;
        }
        bytes.push(match next {
            b'a' => 0x07,
            b'b' => 0x08,
            b't' => b'\t',
            b'n' => b'\n',
            b'v' => 0x0b,
            b'f' => 0x0c,
            b'r' => b'\r',
            other => other,
        });
        at += 2;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The route of `<from>..<to>` in the repo at `toplevel`.
pub fn diff_route(toplevel: &Path, from: &str, to: &str) -> Result<Route> {
    let raw = git_diff(toplevel, &["-z", "--raw", "-M", from, to])?;
    let patch = git_diff(toplevel, &["-U0", "-M", from, to])?;
    Ok(route(&raw, &patch))
}

/// Runs `xtask review` from the shell: prints the lines and returns the
/// exit code.
pub fn main(args: &[String]) -> i32 {
    let dir = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("cannot read the current folder: {e}");
            return REFUSED;
        }
    };
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let Some(home) = home else {
        eprintln!("HOME is not set");
        return REFUSED;
    };
    // An unknown binary time gives the warning that the binary is old.
    let exe_time = std::env::current_exe()
        .and_then(std::fs::metadata)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let (code, lines) = run(args, &dir, &home, exe_time);
    for line in &lines {
        if code == 0 {
            println!("{line}");
        } else {
            eprintln!("{line}");
        }
    }
    code
}

/// The seam for tests: runs a subcommand in `dir`, and returns the exit
/// code and the lines to print.
pub fn run(args: &[String], dir: &Path, home: &Path, exe_time: SystemTime) -> (i32, Vec<String>) {
    run_with(args, dir, home, exe_time, OsStr::new("cargo"))
}

/// [`run`], with the program that runs `cargo mutants`, so a test can pass
/// a fake one.
pub fn run_with(
    args: &[String],
    dir: &Path,
    home: &Path,
    exe_time: SystemTime,
    cargo: &OsStr,
) -> (i32, Vec<String>) {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome =
        match words.as_slice() {
            ["start", base, round] => parse_round(round)
                .and_then(|round| start(&toplevel(dir)?, home, exe_time, base, round)),
            ["check", round] => {
                parse_round(round).and_then(|round| check(&toplevel(dir)?, home, round))
            }
            ["mutants", round] => parse_round(round)
                .and_then(|round| mutants(&toplevel(dir)?, dir, round, None, cargo)),
            ["mutants", round, "--config", file] => parse_round(round)
                .and_then(|round| mutants(&toplevel(dir)?, dir, round, Some(file), cargo)),
            _ => Ok((REFUSED, vec![USAGE.to_string()])),
        };
    outcome.unwrap_or_else(|e| (REFUSED, vec![format!("{e:#}")]))
}

/// A round is a whole number of 1 or more.
fn parse_round(text: &str) -> Result<u32> {
    match text.parse::<u32>() {
        Ok(round) if round >= 1 => Ok(round),
        _ => bail!("the round must be a whole number of 1 or more\n{USAGE}"),
    }
}

/// The repo of the folder `dir`.
fn toplevel(dir: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(
        git(dir, &["rev-parse", "--show-toplevel"])?.trim_end(),
    ))
}

/// The state of the repo that a round must not change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// The merge base; `start` fills it.
    pub base: String,
    /// The start of the round's range; `start` fills it.
    pub since: String,
    pub head: String,
    /// The branch name, or `detached`.
    pub branch: String,
    /// Unix seconds; `start` fills it.
    pub started: u64,
    pub status: Vec<String>,
    pub stash: Vec<String>,
    pub branches: Vec<String>,
    pub tags: Vec<String>,
    /// Every other ref but `refs/stash`, such as `refs/remotes/`, notes and
    /// replacements, with its object.
    #[serde(default)]
    pub other_refs: Vec<String>,
    pub remotes: Vec<String>,
    pub worktrees: Vec<String>,
    /// A label, and `git hash-object` of its file, or `absent`.
    pub hashes: BTreeMap<String, String>,
}

/// The state of the repo at `toplevel` now.  `base`, `since` and `started`
/// stay empty.
pub fn take_state(toplevel: &Path, home: &Path) -> Result<State> {
    let lines = |args: &[&str]| -> Result<Vec<String>> {
        Ok(git(toplevel, args)?
            .lines()
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect())
    };
    let branch = git(toplevel, &["branch", "--show-current"])?;
    let status = git(
        toplevel,
        &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    )?;
    Ok(State {
        base: String::new(),
        since: String::new(),
        head: git(toplevel, &["rev-parse", "HEAD"])?.trim().to_string(),
        branch: match branch.trim() {
            "" => "detached".to_string(),
            name => name.to_string(),
        },
        started: 0,
        status: status
            .split('\0')
            .filter(|entry| !entry.is_empty())
            .map(ToString::to_string)
            .collect(),
        stash: lines(&["stash", "list", "--format=%H"])?,
        branches: lines(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
        ])?,
        tags: lines(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/tags",
        ])?,
        other_refs: lines(&["for-each-ref", "--format=%(refname) %(objectname)"])?
            .into_iter()
            .filter(|line| {
                !(line.starts_with("refs/heads/")
                    || line.starts_with("refs/tags/")
                    || line.starts_with("refs/stash "))
            })
            .collect(),
        remotes: lines(&["remote", "-v"])?,
        worktrees: worktrees(toplevel)?,
        hashes: hashes(toplevel, home)?,
    })
}

/// `git worktree list --porcelain`, one line for each worktree, without
/// those under `target/review/` of the toplevel or of the main worktree.
fn worktrees(toplevel: &Path) -> Result<Vec<String>> {
    let listing = git(toplevel, &["worktree", "list", "--porcelain"])?;
    let records: Vec<Vec<&str>> = listing
        .split("\n\n")
        .map(|record| record.lines().filter(|l| !l.is_empty()).collect())
        .filter(|record: &Vec<&str>| !record.is_empty())
        .collect();
    let path_of = |record: &[&str]| {
        record
            .first()
            .and_then(|line| line.strip_prefix("worktree "))
            .map(canonical)
    };
    let scratch_roots: Vec<PathBuf> = [
        Some(canonical(toplevel)),
        records.first().and_then(|r| path_of(r)),
    ]
    .into_iter()
    .flatten()
    .map(|root| root.join("target/review"))
    .collect();
    Ok(records
        .iter()
        .filter(|record| {
            !path_of(record)
                .is_some_and(|path| scratch_roots.iter().any(|root| path.starts_with(root)))
        })
        .map(|record| record.join(" "))
        .collect())
}

/// The canonical form of a path, or the path itself if it is gone.
fn canonical(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Each hashed file, by its label.
pub fn hashed_paths(toplevel: &Path, home: &Path) -> Result<Vec<(String, PathBuf)>> {
    // A relative path from `git rev-parse` is relative to the toplevel,
    // where git ran.
    let git_path =
        |args: &[&str]| -> Result<PathBuf> { Ok(toplevel.join(git(toplevel, args)?.trim_end())) };
    let common = git_path(&["rev-parse", "--git-common-dir"])?;
    let hooks_folder = git_path(&["rev-parse", "--git-path", "hooks"])?;
    let hooks: Vec<(String, PathBuf)> = match std::fs::read_dir(&hooks_folder) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_file())
            .map(|entry| {
                (
                    format!("hooks/{}", entry.file_name().to_string_lossy()),
                    entry.path(),
                )
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    Ok([
        ("config".to_string(), common.join("config")),
        (
            "config.worktree".to_string(),
            git_path(&["rev-parse", "--git-path", "config.worktree"])?,
        ),
        (
            ".claude/settings.local.json".to_string(),
            toplevel.join(".claude/settings.local.json"),
        ),
        (
            "~/.claude/settings.json".to_string(),
            home.join(".claude/settings.json"),
        ),
    ]
    .into_iter()
    .chain(hooks)
    .collect())
}

/// `git hash-object --no-filters` of each hashed file, or `absent`.
fn hashes(toplevel: &Path, home: &Path) -> Result<BTreeMap<String, String>> {
    let (present, absent): (Vec<_>, Vec<_>) = hashed_paths(toplevel, home)?
        .into_iter()
        .partition(|(_, path)| path.is_file());
    let paths: Vec<String> = present
        .iter()
        .map(|(_, path)| path.to_string_lossy().into_owned())
        .collect();
    let args: Vec<&str> = ["hash-object", "--no-filters", "--"]
        .into_iter()
        .chain(paths.iter().map(String::as_str))
        .collect();
    let sums = if paths.is_empty() {
        String::new()
    } else {
        git(toplevel, &args)?
    };
    let sums: Vec<&str> = sums.lines().collect();
    if sums.len() != present.len() {
        bail!(
            "git hash-object gave {} sums for {} files",
            sums.len(),
            present.len()
        );
    }
    Ok(present
        .into_iter()
        .zip(sums)
        .map(|((label, _), sum)| (label, sum.to_string()))
        .chain(
            absent
                .into_iter()
                .map(|(label, _)| (label, "absent".to_string())),
        )
        .collect())
}

/// Each change from `old` to `new`, by field.  `base`, `since`, `branch`
/// and `started` are not compared.
pub fn changes(old: &State, new: &State) -> Vec<String> {
    let head = (old.head != new.head).then(|| format!("head: {} -> {}", old.head, new.head));
    let lists = [
        ("status", &old.status, &new.status),
        ("stash", &old.stash, &new.stash),
        ("branches", &old.branches, &new.branches),
        ("tags", &old.tags, &new.tags),
        ("other_refs", &old.other_refs, &new.other_refs),
        ("remotes", &old.remotes, &new.remotes),
        ("worktrees", &old.worktrees, &new.worktrees),
    ]
    .into_iter()
    .flat_map(|(field, before, after)| {
        let gone = before
            .iter()
            .filter(|line| !after.contains(line))
            .map(move |line| format!("{field}: goes: {line}"));
        let come = after
            .iter()
            .filter(|line| !before.contains(line))
            .map(move |line| format!("{field}: appears: {line}"));
        gone.chain(come).collect::<Vec<_>>()
    });
    let labels: std::collections::BTreeSet<&String> =
        old.hashes.keys().chain(new.hashes.keys()).collect();
    let hashes = labels.into_iter().filter_map(|label| {
        match (old.hashes.get(label), new.hashes.get(label)) {
            (Some(before), Some(after)) if before == after => None,
            (Some(before), Some(after)) => Some(format!("hashes: {label}: {before} -> {after}")),
            (Some(_), None) => Some(format!("hashes: {label}: goes")),
            (None, _) => Some(format!("hashes: {label}: appears")),
        }
    });
    head.into_iter().chain(lists).chain(hashes).collect()
}

/// `target/review/run/<branch>/`, with `/` in the branch as `-`, or
/// `detached-<short sha>`.
fn run_folder(toplevel: &Path) -> Result<PathBuf> {
    let branch = git(toplevel, &["branch", "--show-current"])?;
    let name = match branch.trim() {
        "" => format!(
            "detached-{}",
            git(toplevel, &["rev-parse", "--short", "HEAD"])?.trim()
        ),
        name => name.replace('/', "-"),
    };
    Ok(toplevel.join("target/review/run").join(name))
}

fn state_path(toplevel: &Path, round: u32) -> Result<PathBuf> {
    Ok(run_folder(toplevel)?
        .join(round.to_string())
        .join("state.json"))
}

fn read_state(path: &Path) -> Result<State> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// `review start <base> <round>`: records the round, and prints its
/// ranges and routes.
fn start(
    toplevel: &Path,
    home: &Path,
    exe_time: SystemTime,
    base_name: &str,
    round: u32,
) -> Result<(i32, Vec<String>)> {
    let dirty = git(
        toplevel,
        &["status", "--porcelain", "--untracked-files=normal"],
    )?;
    if !dirty.trim().is_empty() {
        let first = "commit first: the tree has changes".to_string();
        return Ok((
            REFUSED,
            std::iter::once(first)
                .chain(dirty.lines().map(ToString::to_string))
                .collect(),
        ));
    }
    let base = git(toplevel, &["merge-base", base_name, "HEAD"])
        .with_context(|| format!("no merge base of {base_name} and HEAD"))?
        .trim()
        .to_string();
    let folder = run_folder(toplevel)?;
    if round == 1 && folder.join("log.md").exists() {
        return Ok((
            REFUSED,
            vec!["the run folder holds a log: copy it out, or remove it".to_string()],
        ));
    }
    let since = if round == 1 {
        base.clone()
    } else {
        let last = folder.join((round - 1).to_string()).join("state.json");
        if !last.exists() {
            return Ok((
                REFUSED,
                vec![format!(
                    "no record of round {}: {}",
                    round - 1,
                    last.display()
                )],
            ));
        }
        read_state(&last)?.head
    };
    let started = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let state = State {
        base: base.clone(),
        since: since.clone(),
        started,
        ..take_state(toplevel, home)?
    };
    let head = state.head.clone();
    let branch_route = diff_route(toplevel, &base, &head)?;
    let since_route = diff_route(toplevel, &since, &head)?;
    let ranges = |cap: Option<usize>| -> Vec<String> {
        [
            (format!("branch {base}..{head}:"), &branch_route),
            (format!("since {since}..{head}:"), &since_route),
        ]
        .into_iter()
        .flat_map(|(title, route)| std::iter::once(title).chain(route_lines(route, cap)))
        .collect()
    };

    let round_folder = folder.join(round.to_string());
    std::fs::create_dir_all(&round_folder)
        .with_context(|| format!("making {}", round_folder.display()))?;
    let save = |name: &str, text: &str| {
        let path = round_folder.join(name);
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    };
    save("branch.diff", &git_diff(toplevel, &["-M", &base, &head])?)?;
    save("since.diff", &git_diff(toplevel, &["-M", &since, &head])?)?;
    save("route.txt", &(ranges(None).join("\n") + "\n"))?;
    save("state.json", &serde_json::to_string_pretty(&state)?)?;

    const SOURCE: &str = "xtask/src/review.rs";
    let source_time = std::fs::metadata(toplevel.join(SOURCE))
        .and_then(|m| m.modified())
        .ok();
    let blob = |rev: &str| {
        git(
            toplevel,
            &["rev-parse", "--verify", "-q", &format!("{rev}:{SOURCE}")],
        )
        .ok()
        .map(|sha| sha.trim().to_string())
    };
    let (base_blob, head_blob) = (blob(&base), blob(&head));
    let lines = [
        format!("base: {base} (merge base of {base_name} and HEAD)"),
        format!("since: {since}"),
        format!("run folder: {}", folder.display()),
        format!(
            "target folder: {}",
            toplevel.join("target/review/target").display()
        ),
    ]
    .into_iter()
    .chain(ranges(Some(PATH_REASONS_SHOWN)))
    .chain(warnings(
        &std::env::current_exe().unwrap_or_else(|_| PathBuf::from("xtask")),
        exe_time,
        source_time,
        base_blob.as_deref(),
        head_blob.as_deref(),
    ))
    .collect();
    Ok((0, lines))
}

/// `review start` prints at most this many path reasons for each reviewer;
/// `route.txt` holds them all.
const PATH_REASONS_SHOWN: usize = 5;

/// One line for each reviewer of a route, or `no changes`.  With a cap,
/// path reasons past it give `(+n more)`; other reasons always show.
fn route_lines(route: &Route, cap: Option<usize>) -> Vec<String> {
    if route.is_empty() {
        return vec!["  no changes".to_string()];
    }
    route
        .0
        .iter()
        .map(|(name, reasons)| {
            let limit = cap.unwrap_or(usize::MAX);
            let paths = reasons
                .iter()
                .filter(|r| matches!(r, Reason::Path { .. }))
                .count();
            let hidden = paths.saturating_sub(limit);
            let mut seen_paths = 0;
            let mut parts = Vec::new();
            for reason in reasons {
                if !matches!(reason, Reason::Path { .. }) {
                    parts.push(reason.to_string());
                    continue;
                }
                seen_paths += 1;
                if seen_paths <= limit {
                    parts.push(reason.to_string());
                }
                if seen_paths == limit && hidden > 0 {
                    parts.push(format!("(+{hidden} more)"));
                }
            }
            format!("  {name}: {}", parts.join("; "))
        })
        .collect()
}

/// The warnings of `review start`, which do not change its exit code.
pub fn warnings(
    exe: &Path,
    exe_time: SystemTime,
    source_time: Option<SystemTime>,
    base_blob: Option<&str>,
    head_blob: Option<&str>,
) -> Vec<String> {
    let old_binary = source_time
        .is_some_and(|source| exe_time < source)
        .then(|| {
            format!(
                "warning: {} is older than xtask/src/review.rs; run cargo build -p xtask",
                exe.display()
            )
        });
    let changed = match (base_blob, head_blob) {
        (Some(base), Some(head)) if base != head => Some(
            "warning: this branch changes xtask/src/review.rs, so the route comes from the branch"
                .to_string(),
        ),
        _ => None,
    };
    old_binary.into_iter().chain(changed).collect()
}

/// How a cargo-mutants run ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunEnd {
    /// The exit code, or none after a signal.
    pub code: Option<i32>,
    /// The run reached its own time limit, and got SIGTERM.
    pub own_stop: bool,
    /// The run ignored SIGTERM, and got SIGKILL.
    pub killed: bool,
    /// cargo-mutants is not installed, so nothing ran.
    pub missing: bool,
}

/// The results in `mutants.out`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub caught: u64,
    pub missed: u64,
    pub timeout: u64,
    pub unviable: u64,
    /// The length of `mutants.json`: every mutant, tested or not.
    pub listed: Option<u64>,
}

/// How much of the diff the run tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Full,
    Partial,
    None,
}

/// The one line that `review mutants` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub class: Class,
    pub counts: Counts,
    pub reason: Option<String>,
}

/// The class of a run, and why.
pub fn classify(end: RunEnd, counts: Option<Counts>) -> Line {
    let counted = counts.unwrap_or_default();
    let tested = counted.caught + counted.missed + counted.unviable;
    let (class, reason): (Class, Option<String>) = if end.missing {
        (Class::None, Some(INSTALL.to_string()))
    } else if end.own_stop {
        let kill = if end.killed {
            "; SIGKILL 60 s after SIGTERM"
        } else {
            ""
        };
        if tested > 0 {
            (Class::Partial, Some(format!("stopped at the limit{kill}")))
        } else {
            (
                Class::None,
                Some(format!("stopped before a mutant finished{kill}")),
            )
        }
    } else {
        match end.code {
            Some(0) if counts.is_none() => (Class::None, Some("no mutants in the diff".into())),
            Some(0 | 2) => (Class::Full, None),
            Some(3) => (Class::Partial, Some("timeouts have no result".into())),
            Some(code) => (
                Class::None,
                Some(format!("exit {code}: {}", exit_meaning(code))),
            ),
            None => (
                Class::None,
                Some("stopped by a signal that was not its own".into()),
            ),
        }
    };
    Line {
        class,
        counts: counted,
        reason,
    }
}

/// What an exit code of cargo-mutants that gives `none` means.
fn exit_meaning(code: i32) -> &'static str {
    match code {
        1 => "a usage error, or a stop by a signal that was not its own",
        4 => "the baseline failed",
        5 => "the diff does not fit the tree",
        6 => "the diff cannot be read",
        70 => "an internal error of cargo-mutants",
        _ => "a code that cargo-mutants does not document",
    }
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let class = match self.class {
            Class::Full => "full",
            Class::Partial => "partial",
            Class::None => "none",
        };
        let c = self.counts;
        let tested = c.caught + c.missed + c.unviable;
        let not_tested = c
            .listed
            .map_or(0, |listed| listed.saturating_sub(tested + c.timeout));
        write!(
            f,
            "mutants: {class}: tested {tested} (caught {}, missed {}, unviable {}), timeouts {}, not tested {not_tested}",
            c.caught, c.missed, c.unviable, c.timeout
        )?;
        match &self.reason {
            Some(reason) => write!(f, "; {reason}"),
            None => Ok(()),
        }
    }
}

/// Removes the round's old `mutants.out`, `mutants.out.old` and
/// `mutants.txt`.  A diff with no mutants leaves an old `mutants.out` as it
/// was, and an old line would show until the new run ends.
pub fn clear_old_output(round_folder: &Path) -> Result<()> {
    for old in ["mutants.out", "mutants.out.old"] {
        let path = round_folder.join(old);
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing {}", path.display()))?;
        }
    }
    let line = round_folder.join("mutants.txt");
    if line.exists() {
        std::fs::remove_file(&line).with_context(|| format!("removing {}", line.display()))?;
    }
    Ok(())
}

/// Writes the diff of `*.rs` files from `since` to `head`, with renames
/// shown as a deletion and an addition.
pub fn write_mutants_diff(toplevel: &Path, since: &str, head: &str, path: &Path) -> Result<()> {
    let diff = git_diff(toplevel, &["--no-renames", since, head, "--", "*.rs"])?;
    std::fs::write(path, diff).with_context(|| format!("writing {}", path.display()))
}

/// The config file: `--config` resolved against `dir`, or the repo's own.
pub fn config_path(toplevel: &Path, dir: &Path, arg: Option<&str>) -> PathBuf {
    match arg {
        Some(file) => dir.join(file),
        None => toplevel.join(".cargo/mutants.toml"),
    }
}

/// Runs `command` until it ends, or until `limit`; then sends SIGTERM,
/// and SIGKILL if it has not ended after [`KILL_AFTER`].
fn run_with_limit(command: Command, limit: Duration) -> RunEnd {
    run_with_limit_and_grace(command, limit, KILL_AFTER)
}

const KILL_AFTER: Duration = Duration::from_secs(60);

fn run_with_limit_and_grace(mut command: Command, limit: Duration, grace: Duration) -> RunEnd {
    let Ok(mut child) = command.spawn() else {
        return RunEnd {
            missing: true,
            ..RunEnd::default()
        };
    };
    let poll = Duration::from_secs(1);
    let begun = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return RunEnd {
                    code: status.code(),
                    ..RunEnd::default()
                };
            }
            Ok(None) if begun.elapsed() < limit => {
                std::thread::sleep(poll.min(limit.saturating_sub(begun.elapsed())));
            }
            _ => break,
        }
    }
    signal(child.id(), "-TERM");
    let stopped = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return RunEnd {
                    code: status.code(),
                    own_stop: true,
                    ..RunEnd::default()
                };
            }
            Ok(None) if stopped.elapsed() < grace => std::thread::sleep(poll),
            _ => break,
        }
    }
    signal(child.id(), "-KILL");
    RunEnd {
        code: child.wait().ok().and_then(|status| status.code()),
        own_stop: true,
        killed: true,
        missing: false,
    }
}

/// Sends a signal with `kill`, since `Child::kill` sends SIGKILL only.
fn signal(pid: u32, which: &str) {
    let _ = Command::new("kill")
        .args([which, &pid.to_string()])
        .status();
}

/// The counts in `<round folder>/mutants.out`, if cargo-mutants wrote them.
fn read_counts(mutants_out: &Path) -> Option<Counts> {
    let outcomes: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(mutants_out.join("outcomes.json")).ok()?)
            .ok()?;
    let count = |key: &str| {
        outcomes
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let listed = std::fs::read_to_string(mutants_out.join("mutants.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<serde_json::Value>>(&text).ok())
        .map(|all| all.len() as u64);
    Some(Counts {
        caught: count("caught"),
        missed: count("missed"),
        timeout: count("timeout"),
        unviable: count("unviable"),
        listed,
    })
}

/// The run's limit, from `STEMYLA_MUTANTS_MINUTES`: 60 minutes when unset.
pub fn parse_limit(minutes: Option<&str>) -> Result<Duration> {
    let Some(text) = minutes else {
        return Ok(Duration::from_secs(60 * 60));
    };
    let limit = text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|minutes| *minutes > 0.0)
        .and_then(|minutes| Duration::try_from_secs_f64(minutes * 60.0).ok());
    match limit {
        Some(limit) => Ok(limit),
        None => bail!("STEMYLA_MUTANTS_MINUTES must be a number of minutes above 0, not {text:?}"),
    }
}

const INSTALL: &str = "cargo-mutants is not installed: cargo install --locked cargo-mutants@27.1.0";

/// The arguments of `cargo` for a round's run.
pub fn mutants_args(diff: &Path, round_folder: &Path, config: &Path) -> Vec<OsString> {
    let words = |list: &[&str]| -> Vec<OsString> { list.iter().map(OsString::from).collect() };
    [
        words(&["mutants", "--in-diff"]),
        vec![diff.into()],
        words(&["--output"]),
        vec![round_folder.into()],
        words(&[
            "--copy-vcs=true",
            "--gitignore=true",
            "--shuffle",
            "-j2",
            "--timeout",
            "300",
            "--config",
        ]),
        vec![config.into()],
    ]
    .concat()
}

/// `cargo` (or a fake for tests) with `args`, at the toplevel, without
/// `CARGO_TARGET_DIR`, so the run builds in its own copies.
pub fn mutants_command(cargo: &OsStr, toplevel: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new(cargo);
    command
        .args(args)
        .current_dir(toplevel)
        .env_remove("CARGO_TARGET_DIR")
        .stdin(std::process::Stdio::null());
    command
}

/// The round's folder, with its old output removed and `mutants.diff`
/// written from the round's `since` to its `head`; none without a record.
pub fn prepare_mutants(toplevel: &Path, round: u32) -> Result<Option<PathBuf>> {
    let round_folder = run_folder(toplevel)?.join(round.to_string());
    let record = round_folder.join("state.json");
    if !record.exists() {
        return Ok(None);
    }
    let state = read_state(&record)?;
    clear_old_output(&round_folder)?;
    write_mutants_diff(
        toplevel,
        &state.since,
        &state.head,
        &round_folder.join("mutants.diff"),
    )?;
    Ok(Some(round_folder))
}

/// `review mutants <round> [--config <file>]`: runs cargo-mutants on the
/// round's `since` diff, under the limit, and prints one line.
fn mutants(
    toplevel: &Path,
    dir: &Path,
    round: u32,
    config: Option<&str>,
    cargo: &OsStr,
) -> Result<(i32, Vec<String>)> {
    let limit = parse_limit(std::env::var("STEMYLA_MUTANTS_MINUTES").ok().as_deref())?;
    let Some(round_folder) = prepare_mutants(toplevel, round)? else {
        return Ok((
            REFUSED,
            vec![format!(
                "no record for this branch and round ({}); run review start first",
                state_path(toplevel, round)?.display()
            )],
        ));
    };
    let installed = Command::new(cargo)
        .args(["mutants", "--version"])
        .current_dir(toplevel)
        .output()
        .is_ok_and(|out| out.status.success());
    let end = if installed {
        let log_path = round_folder.join("mutants.log");
        let log = std::fs::File::create(&log_path)
            .with_context(|| format!("making {}", log_path.display()))?;
        let args = mutants_args(
            &round_folder.join("mutants.diff"),
            &round_folder,
            &config_path(toplevel, dir, config),
        );
        let mut command = mutants_command(cargo, toplevel, &args);
        command.stdout(log.try_clone()?).stderr(log);
        run_with_limit(command, limit)
    } else {
        RunEnd {
            missing: true,
            ..RunEnd::default()
        }
    };
    let line = classify(end, read_counts(&round_folder.join("mutants.out"))).to_string();
    let saved = round_folder.join("mutants.txt");
    std::fs::write(&saved, format!("{line}\n"))
        .with_context(|| format!("writing {}", saved.display()))?;
    Ok((0, vec![line]))
}

/// `review check <round>`: the changes since the round's record.
fn check(toplevel: &Path, home: &Path, round: u32) -> Result<(i32, Vec<String>)> {
    let path = state_path(toplevel, round)?;
    if !path.exists() {
        return Ok((
            REFUSED,
            vec![format!(
                "no record for this branch and round ({}); HEAD may have moved, see git reflog",
                path.display()
            )],
        ));
    }
    let found = changes(&read_state(&path)?, &take_state(toplevel, home)?);
    if found.is_empty() {
        Ok((0, vec!["no change".to_string()]))
    } else {
        Ok((REFUSED, found))
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(["-c", "core.quotePath=false"])
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

fn git_diff(dir: &Path, args: &[&str]) -> Result<String> {
    let all: Vec<&str> = ["diff"]
        .iter()
        .chain(DIFF_OPTIONS)
        .chain(args)
        .copied()
        .collect();
    git(dir, &all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Helpers.

    /// Runs git in a scratch repo, with an identity and no signing, so it
    /// works in CI and on a machine that signs every commit.
    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
                "-c",
                "user.name=lloyd",
                "-c",
                "user.email=lloyd@beachape.example",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit_all(dir: &Path, message: &str) {
        git_in(dir, &["add", "-A"]);
        git_in(dir, &["commit", "-q", "--no-verify", "-m", message]);
    }

    /// A folder in the temporary folder, removed when the test ends, also
    /// when it fails.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("xtask-review-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        /// A path next to the folder, for a worktree; removed with it.
        fn beside(&self, suffix: &str) -> Scratch {
            let path = self.0.with_extension(suffix);
            let _ = std::fs::remove_dir_all(&path);
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    impl std::ops::Deref for Scratch {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.0
        }
    }

    /// A scratch repo on `main` with one commit, which ignores `target/`
    /// as the real repo does.  Its hooks folder is its own, so a person's
    /// global `core.hooksPath` never gets a test's hook.
    fn scratch(name: &str) -> Scratch {
        let dir = Scratch::new(name);
        git_in(&dir, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(dir.join(".git/hooks")).unwrap();
        let hooks = dir.join(".git/hooks").canonicalize().unwrap();
        git_in(&dir, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        commit_all(&dir, "start");
        dir
    }

    /// Sets a person's diff and status options that change git's output,
    /// so a test fails if the code does not pass its own options.
    fn hostile_config(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let junk = dir.join(".git/junk-diff.sh");
        std::fs::write(&junk, "#!/bin/sh\necho junk\n").unwrap();
        std::fs::set_permissions(&junk, std::fs::Permissions::from_mode(0o755)).unwrap();
        for (key, value) in [
            ("diff.noprefix", "true"),
            ("diff.mnemonicPrefix", "true"),
            ("diff.external", junk.to_str().unwrap()),
            ("status.showUntrackedFiles", "no"),
        ] {
            git_in(dir, &["config", key, value]);
        }
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    const SHA: &str = "0000000000000000000000000000000000000000";

    /// `git diff -z --raw` for a changed file.
    fn raw_change(path: &str) -> String {
        format!(":100644 100644 {SHA} {SHA} M\0{path}\0")
    }

    /// The raw and patch text of one added line in `path`.
    fn added(path: &str, line: &str) -> (String, String) {
        (
            raw_change(path),
            format!(
                "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,0 +2 @@\n+{line}\n"
            ),
        )
    }

    fn route_of_path(path: &str) -> Route {
        route(&raw_change(path), "")
    }

    // The route: paths.

    #[test]
    fn each_include_glob_routes_its_reviewer() {
        let cases = [
            ("roformer-gpu/src/net.rs", "gpu-reviewer", "roformer-gpu/**"),
            ("roformer/src/stft.rs", "gpu-reviewer", "roformer/src/**"),
            (
                "roformer/clippy.toml",
                "gpu-reviewer",
                "roformer/clippy.toml",
            ),
            (
                ".claude/rules/gpu.md",
                "gpu-reviewer",
                ".claude/rules/gpu.md",
            ),
            ("engine/src/lib.rs", "memory-reviewer", "engine/**"),
            ("app/src/lib.rs", "memory-reviewer", "app/src/**"),
            ("worker/src/lib.rs", "memory-reviewer", "worker/**"),
            ("protocol/src/lib.rs", "memory-reviewer", "protocol/**"),
            ("roformer/src/stft.rs", "memory-reviewer", "roformer/src/**"),
            (
                "roformer-gpu/src/net.rs",
                "memory-reviewer",
                "roformer-gpu/src/**",
            ),
            ("synth/src/lib.rs", "memory-reviewer", "synth/src/**"),
            ("dsp/clippy.toml", "memory-reviewer", "**/clippy.toml"),
            (
                ".claude/rules/memory.md",
                "memory-reviewer",
                ".claude/rules/memory.md",
            ),
            ("protocol/src/lib.rs", "wire-reviewer", "protocol/**"),
            ("worker/src/web.rs", "wire-reviewer", "worker/src/web.rs"),
            (
                "worker/src/web/tests.rs",
                "wire-reviewer",
                "worker/src/web/**",
            ),
            (
                "app/src/separator.rs",
                "wire-reviewer",
                "app/src/separator.rs",
            ),
            ("xtask/src/e2e.rs", "wire-reviewer", "xtask/src/e2e.rs"),
            ("xtask/e2e/worker.js", "wire-reviewer", "xtask/e2e/**"),
            (
                ".claude/rules/protocol.md",
                "wire-reviewer",
                ".claude/rules/protocol.md",
            ),
            (
                "engine/Cargo.toml",
                "supply-chain-reviewer",
                "**/Cargo.toml",
            ),
            ("Cargo.lock", "supply-chain-reviewer", "Cargo.lock"),
            ("THIRD-PARTY.md", "supply-chain-reviewer", "THIRD-PARTY.md"),
            ("LICENSE-MIT", "supply-chain-reviewer", "LICENSE*"),
            (
                "tools/roformer_export.py",
                "supply-chain-reviewer",
                "tools/**",
            ),
            ("engine/build.rs", "supply-chain-reviewer", "**/build.rs"),
            (".cargo/mutants.toml", "supply-chain-reviewer", ".cargo/**"),
            (
                ".github/workflows/ci.yml",
                "supply-chain-reviewer",
                ".github/**",
            ),
            (
                ".claude/settings.json",
                "supply-chain-reviewer",
                ".claude/**",
            ),
            (".mcp.json", "supply-chain-reviewer", ".mcp.json"),
            (".gitignore", "supply-chain-reviewer", ".gitignore"),
            (
                "xtask/src/serve.rs",
                "supply-chain-reviewer",
                "xtask/src/serve.rs",
            ),
            (
                "xtask/tests/serve.rs",
                "supply-chain-reviewer",
                "xtask/tests/serve.rs",
            ),
            (
                "xtask/src/publish.rs",
                "supply-chain-reviewer",
                "xtask/src/publish.rs",
            ),
            (
                "xtask/tests/publish.rs",
                "supply-chain-reviewer",
                "xtask/tests/publish.rs",
            ),
            (
                "xtask/src/rules.rs",
                "supply-chain-reviewer",
                "xtask/src/rules.rs",
            ),
            (
                "xtask/src/docs.rs",
                "supply-chain-reviewer",
                "xtask/src/docs.rs",
            ),
            (
                "xtask/tests/rules.rs",
                "supply-chain-reviewer",
                "xtask/tests/rules.rs",
            ),
            (
                "xtask/tests/docs.rs",
                "supply-chain-reviewer",
                "xtask/tests/docs.rs",
            ),
            (
                "test-support/src/lib.rs",
                "supply-chain-reviewer",
                "test-support/**",
            ),
            ("app/index.html", "supply-chain-reviewer", "app/index.html"),
            ("app/Trunk.toml", "supply-chain-reviewer", "app/Trunk.toml"),
            ("docs/invariants.md", "docs-reviewer", "docs/**"),
            ("CLAUDE.md", "docs-reviewer", "CLAUDE.md"),
            ("README.md", "docs-reviewer", "README.md"),
            (".claude/rules/dsp.md", "docs-reviewer", ".claude/rules/**"),
        ];
        for (path, reviewer, glob) in cases {
            let reasons = route_of_path(path).reasons(reviewer);
            let expected = format!("{path} ({glob})");
            assert!(
                reasons.contains(&expected),
                "{path} should route {reviewer} with {expected:?}, got {reasons:?}"
            );
        }
    }

    #[test]
    fn the_first_glob_of_a_row_is_the_reason() {
        assert_eq!(
            route_of_path("engine/clippy.toml").reasons("memory-reviewer"),
            ["engine/clippy.toml (engine/**)"]
        );
    }

    #[test]
    fn excluded_paths_do_not_route_docs_reviewer() {
        for path in ["docs/plans/active/x.md", "docs/reviews/x.md"] {
            assert_eq!(route_of_path(path).reviewers(), ["reviewer"], "{path}");
        }
    }

    #[test]
    fn a_path_in_no_row_routes_reviewer_only() {
        let route = route_of_path("dsp/src/lib.rs");
        assert_eq!(route.reviewers(), ["reviewer"]);
        assert_eq!(route.reasons("reviewer"), ["every diff"]);
    }

    const ALL_SIX: [&str; 6] = [
        "reviewer",
        "gpu-reviewer",
        "memory-reviewer",
        "wire-reviewer",
        "supply-chain-reviewer",
        "docs-reviewer",
    ];

    fn assert_routes_all_six(path: &str) {
        let route = route_of_path(path);
        assert_eq!(route.reviewers(), ALL_SIX, "{path}");
        let harness = format!("{path} changes the review harness");
        assert_eq!(route.reasons("reviewer"), ["every diff", harness.as_str()]);
        for name in &ALL_SIX[1..] {
            assert_eq!(route.reasons(name), [harness.as_str()], "{name}");
        }
    }

    #[test]
    fn review_rs_routes_all_six() {
        assert_routes_all_six("xtask/src/review.rs");
    }

    #[test]
    fn an_agent_file_routes_all_six() {
        assert_routes_all_six(".claude/agents/reviewer.md");
    }

    #[test]
    fn a_skill_file_routes_all_six() {
        assert_routes_all_six(".claude/skills/review-rounds/SKILL.md");
    }

    #[test]
    fn a_hook_file_routes_all_six() {
        assert_routes_all_six(".claude/hooks/read-only-bash.sh");
    }

    #[test]
    fn a_harness_file_adds_no_sign_reason() {
        let (raw, patch) = added("xtask/src/review.rs", "let a = b.to_vec();");
        assert_eq!(
            route(&raw, &patch).reasons("memory-reviewer"),
            ["xtask/src/review.rs changes the review harness"]
        );
    }

    // The route: content signs.

    #[test]
    fn each_sign_routes_its_reviewer() {
        let cases = [
            ("let a = b.to_vec();", "memory-reviewer", "to_vec"),
            ("let a = b.to_owned();", "memory-reviewer", "to_owned"),
            ("let a = Vec::from(b);", "memory-reviewer", "Vec::from"),
            (
                "a.extend_from_slice(b);",
                "memory-reviewer",
                "extend_from_slice",
            ),
            ("a.copy_to(b);", "memory-reviewer", "copy_to"),
            ("let a = [b, c].concat();", "memory-reviewer", ".concat()"),
            ("let a = parts.concat();", "memory-reviewer", ".concat()"),
            ("let a: wgpu::Device = b;", "gpu-reviewer", "wgpu::"),
            (
                "let a = d.create_buffer(b);",
                "gpu-reviewer",
                "create_buffer",
            ),
            (
                "let a = d.create_texture(b);",
                "gpu-reviewer",
                "create_texture",
            ),
            (
                "let a = d.create_query_set(b);",
                "gpu-reviewer",
                "create_query_set",
            ),
            ("let a = q.model_url;", "wire-reviewer", "model_url"),
            ("#[path = \"y.rs\"]", "supply-chain-reviewer", "#[path"),
            ("include!(\"y.rs\");", "supply-chain-reviewer", "include!"),
            (
                "let a = include_str!(\"y\");",
                "supply-chain-reviewer",
                "include_str!",
            ),
            (
                "let a = include_bytes!(\"y\");",
                "supply-chain-reviewer",
                "include_bytes!",
            ),
            ("unsafe { a() }", "supply-chain-reviewer", "unsafe"),
            (
                "let a = Command::new(\"y\");",
                "supply-chain-reviewer",
                "Command::new",
            ),
        ];
        for (line, reviewer, sign) in cases {
            let (raw, patch) = added("xtask/src/x.rs", line);
            let reasons = route(&raw, &patch).reasons(reviewer);
            let expected = format!("`{sign}` in xtask/src/x.rs");
            assert_eq!(reasons, [expected], "{line}");
        }
    }

    #[test]
    fn a_path_given_twice_gives_one_reason() {
        // `docs/a.md` moves to `docs/b.md`, and a new `docs/a.md` comes in.
        let raw = format!(
            ":100644 100644 {SHA} {SHA} R090\0docs/a.md\0docs/b.md\0:000000 100644 {SHA} {SHA} A\0docs/a.md\0"
        );
        assert_eq!(
            route(&raw, "").reasons("docs-reviewer"),
            ["docs/a.md (docs/**)", "docs/b.md (docs/**)"]
        );
    }

    #[test]
    fn a_sign_repeated_in_one_file_gives_one_reason() {
        let (raw, patch) = added(
            "xtask/src/x.rs",
            "let a = b.to_vec();\n+let c = d.to_vec();",
        );
        assert_eq!(
            route(&raw, &patch).reasons("memory-reviewer"),
            ["`to_vec` in xtask/src/x.rs"]
        );
    }

    #[test]
    fn a_sign_inside_a_longer_name_routes_nothing() {
        let cases = [
            ("let a = b.into_owned();", "to_owned"),
            ("let a = my_to_vec(b);", "to_vec"),
            ("unsafely();", "unsafe"),
            ("my_include!(x);", "include!"),
            ("let a = Vec::from_iter(b);", "Vec::from"),
            ("b.copy_to_slice(c);", "copy_to"),
            ("let a: xwgpu::Device = b;", "wgpu::"),
        ];
        for (line, sign) in cases {
            assert!(line.contains(sign), "the case holds {sign} as text");
            let (raw, patch) = added("xtask/src/x.rs", line);
            assert_eq!(route(&raw, &patch).reviewers(), ["reviewer"], "{line}");
        }
    }

    #[test]
    fn a_sign_in_the_reviewers_own_paths_adds_no_reason() {
        let (raw, patch) = added("engine/src/x.rs", "let a = b.to_vec();");
        assert_eq!(
            route(&raw, &patch).reasons("memory-reviewer"),
            ["engine/src/x.rs (engine/**)"]
        );
    }

    #[test]
    fn a_sign_on_a_removed_line_adds_no_reason() {
        let patch = "diff --git a/xtask/src/x.rs b/xtask/src/x.rs\n--- a/xtask/src/x.rs\n+++ b/xtask/src/x.rs\n@@ -2 +1,0 @@\n-let a = b.to_vec();\n";
        let route = route(&raw_change("xtask/src/x.rs"), patch);
        assert_eq!(route.reviewers(), ["reviewer"]);
    }

    #[test]
    fn a_sign_in_a_tests_folder_adds_no_reason() {
        let (raw, patch) = added("engine/tests/x.rs", "let a: wgpu::Device = b;");
        assert!(!route(&raw, &patch).reviewers().contains(&"gpu-reviewer"));
        let (raw, patch) = added("xtask/tests/x.rs", "let a = b.to_vec();");
        assert_eq!(route(&raw, &patch).reviewers(), ["reviewer"]);
    }

    #[test]
    fn a_sign_in_a_markdown_file_adds_no_reason() {
        let (raw, patch) = added("x.md", "let a = b.to_vec();");
        assert_eq!(route(&raw, &patch).reviewers(), ["reviewer"]);
    }

    #[test]
    fn an_added_line_that_starts_with_two_pluses_is_not_a_header() {
        let (raw, patch) = added("xtask/src/x.rs", "++x.to_vec();");
        assert!(patch.contains("\n+++x.to_vec();\n"));
        assert_eq!(
            route(&raw, &patch).reasons("memory-reviewer"),
            ["`to_vec` in xtask/src/x.rs"]
        );
    }

    #[test]
    fn unquote_reads_octal_and_named_escapes() {
        assert_eq!(
            unquote("\"b/caf\\303\\251 \\\"q\\\"\\t.rs\""),
            "b/café \"q\"\t.rs"
        );
    }

    #[test]
    fn a_deleted_symlink_routes_supply_chain_reviewer() {
        let raw = format!(":120000 000000 {SHA} {SHA} D\0link\0");
        assert_eq!(
            route(&raw, "").reasons("supply-chain-reviewer"),
            ["link is a symlink"]
        );
    }

    #[test]
    fn a_raw_path_that_starts_with_a_colon_is_a_path() {
        let raw = format!(
            ":100644 100644 {SHA} {SHA} M\0:odd.md\0:100644 100644 {SHA} {SHA} R100\0docs/a.md\0:b.md\0:100644 100644 {SHA} {SHA} M\0docs/c.md\0"
        );
        assert_eq!(
            route(&raw, "").reasons("docs-reviewer"),
            ["docs/a.md (docs/**)", "docs/c.md (docs/**)"]
        );
        let paths: Vec<Vec<String>> = parse_raw(&raw).into_iter().map(|e| e.paths).collect();
        assert_eq!(
            paths,
            [
                vec![":odd.md"],
                vec!["docs/a.md", ":b.md"],
                vec!["docs/c.md"]
            ]
        );
    }

    #[test]
    fn unquote_reads_an_escape_at_either_end() {
        assert_eq!(unquote("\"\\303\\251x\""), "éx");
        assert_eq!(unquote("\"\\tx\""), "\tx");
        assert_eq!(unquote("\"x\\\\\""), "x\\");
    }

    #[test]
    fn an_empty_diff_gives_an_empty_route() {
        assert!(route("", "").is_empty());
    }

    // The route: real git output.

    #[test]
    fn a_rename_routes_by_both_paths() {
        let dir = scratch("rename");
        hostile_config(&dir);
        let text: String = (0..20).map(|i| format!("line {i}\n")).collect();
        write(&dir, "docs/x.md", &text);
        commit_all(&dir, "doc");
        std::fs::create_dir_all(dir.join("engine/src")).unwrap();
        git_in(&dir, &["mv", "docs/x.md", "engine/src/x.rs"]);
        commit_all(&dir, "rename");
        let route = diff_route(&dir, "HEAD~1", "HEAD").unwrap();
        assert_eq!(
            route.reviewers(),
            ["reviewer", "memory-reviewer", "docs-reviewer"]
        );
        assert_eq!(route.reasons("docs-reviewer"), ["docs/x.md (docs/**)"]);
        assert_eq!(
            route.reasons("memory-reviewer"),
            ["engine/src/x.rs (engine/**)"]
        );
    }

    #[test]
    fn a_sign_in_a_file_with_a_space_or_a_quote_routes() {
        let dir = scratch("space");
        hostile_config(&dir);
        write(&dir, "xtask/src/a b.rs", "let a = b.to_vec();\n");
        write(&dir, "xtask/src/q\"x.rs", "let a = q.model_url;\n");
        commit_all(&dir, "signs");
        let route = diff_route(&dir, "HEAD~1", "HEAD").unwrap();
        assert_eq!(
            route.reasons("memory-reviewer"),
            ["`to_vec` in xtask/src/a b.rs"]
        );
        assert_eq!(
            route.reasons("wire-reviewer"),
            ["`model_url` in xtask/src/q\"x.rs"]
        );
    }

    #[test]
    fn a_symlink_routes_supply_chain_reviewer() {
        let dir = scratch("symlink");
        std::os::unix::fs::symlink("elsewhere", dir.join("link")).unwrap();
        commit_all(&dir, "link");
        let route = diff_route(&dir, "HEAD~1", "HEAD").unwrap();
        assert_eq!(
            route.reasons("supply-chain-reviewer"),
            ["link is a symlink"]
        );
    }

    // The state record: the comparison.

    fn a_state() -> State {
        State {
            base: "b".into(),
            since: "s".into(),
            head: "h1".into(),
            branch: "main".into(),
            started: 1,
            status: vec!["?? a.txt".into()],
            stash: vec!["s1".into()],
            branches: vec!["refs/heads/main h1".into()],
            tags: vec!["refs/tags/v1 t1".into()],
            other_refs: vec!["refs/remotes/origin/main h1".into()],
            remotes: vec!["origin /x (fetch)".into()],
            worktrees: vec!["worktree /r HEAD h1 branch refs/heads/main".into()],
            hashes: BTreeMap::from([
                ("config".to_string(), "c1".to_string()),
                ("hooks/pre-commit".to_string(), "p1".to_string()),
                (
                    ".claude/settings.local.json".to_string(),
                    "absent".to_string(),
                ),
            ]),
        }
    }

    /// Records that differ only in `field` give changes, and each names it.
    fn assert_changes_name(new: State, field: &str) {
        let found = changes(&a_state(), &new);
        assert!(!found.is_empty(), "no change for {field}");
        assert!(
            found
                .iter()
                .all(|line| line.starts_with(&format!("{field}: "))),
            "{found:?}"
        );
    }

    #[test]
    fn a_different_head_is_a_change() {
        assert_changes_name(
            State {
                head: "h2".into(),
                ..a_state()
            },
            "head",
        );
    }

    #[test]
    fn a_different_status_is_a_change() {
        assert_changes_name(
            State {
                status: vec![],
                ..a_state()
            },
            "status",
        );
    }

    #[test]
    fn a_different_stash_is_a_change() {
        assert_changes_name(
            State {
                stash: vec!["s1".into(), "s2".into()],
                ..a_state()
            },
            "stash",
        );
    }

    #[test]
    fn different_branches_are_a_change() {
        assert_changes_name(
            State {
                branches: vec!["refs/heads/main h2".into()],
                ..a_state()
            },
            "branches",
        );
    }

    #[test]
    fn different_tags_are_a_change() {
        assert_changes_name(
            State {
                tags: vec![],
                ..a_state()
            },
            "tags",
        );
    }

    #[test]
    fn different_other_refs_are_a_change() {
        assert_changes_name(
            State {
                other_refs: vec!["refs/remotes/origin/main h2".into()],
                ..a_state()
            },
            "other_refs",
        );
    }

    #[test]
    fn different_remotes_are_a_change() {
        assert_changes_name(
            State {
                remotes: vec!["origin /y (fetch)".into()],
                ..a_state()
            },
            "remotes",
        );
    }

    #[test]
    fn different_worktrees_are_a_change() {
        assert_changes_name(
            State {
                worktrees: vec![],
                ..a_state()
            },
            "worktrees",
        );
    }

    #[test]
    fn a_different_hash_is_a_change() {
        let mut hashes = a_state().hashes;
        hashes.insert("config".into(), "c2".into());
        assert_changes_name(
            State {
                hashes,
                ..a_state()
            },
            "hashes",
        );
    }

    #[test]
    fn a_hooks_file_that_appears_or_goes_is_a_change() {
        let mut more = a_state().hashes;
        more.insert("hooks/post-checkout".into(), "q1".into());
        assert_changes_name(
            State {
                hashes: more,
                ..a_state()
            },
            "hashes",
        );
        let mut fewer = a_state().hashes;
        fewer.remove("hooks/pre-commit");
        assert_changes_name(
            State {
                hashes: fewer,
                ..a_state()
            },
            "hashes",
        );
    }

    #[test]
    fn a_hashed_file_that_comes_from_absent_is_a_change() {
        let mut hashes = a_state().hashes;
        hashes.insert(".claude/settings.local.json".into(), "l1".into());
        let found = changes(
            &a_state(),
            &State {
                hashes,
                ..a_state()
            },
        );
        assert_eq!(found, ["hashes: .claude/settings.local.json: absent -> l1"]);
    }

    #[test]
    fn base_since_branch_and_started_are_not_compared() {
        let new = State {
            base: "b2".into(),
            since: "s2".into(),
            branch: "other".into(),
            started: 2,
            ..a_state()
        };
        assert!(changes(&a_state(), &new).is_empty());
    }

    // The state record: the capture, with real git.

    /// A scratch repo and a scratch home folder, with round 1's record.
    fn recorded(name: &str) -> (Scratch, Scratch) {
        let dir = scratch(name);
        let home = Scratch::new(&format!("home-{name}"));
        let top = toplevel(&dir).unwrap();
        let state = take_state(&top, &home).unwrap();
        let path = state_path(&top, 1).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string(&state).unwrap()).unwrap();
        (dir, home)
    }

    fn check_round_1(dir: &Path, home: &Path) -> (i32, Vec<String>) {
        run(
            &["check".to_string(), "1".to_string()],
            dir,
            home,
            SystemTime::UNIX_EPOCH,
        )
    }

    fn assert_check_names(dir: &Path, home: &Path, prefix: &str) {
        let (code, lines) = check_round_1(dir, home);
        assert_eq!(code, 3, "{lines:?}");
        assert!(
            lines.iter().any(|line| line.starts_with(prefix)),
            "no line starts with {prefix:?}: {lines:?}"
        );
    }

    #[test]
    fn check_sees_a_new_commit() {
        let (dir, home) = recorded("cap-head");
        git_in(
            &dir,
            &["commit", "-q", "--no-verify", "--allow-empty", "-m", "x"],
        );
        assert_check_names(&dir, &home, "head: ");
    }

    #[test]
    fn check_sees_an_untracked_file() {
        let (dir, home) = recorded("cap-status");
        write(&dir, "x.txt", "x\n");
        assert_check_names(&dir, &home, "status: ");
    }

    #[test]
    fn check_sees_a_stash() {
        let (dir, home) = recorded("cap-stash");
        write(&dir, ".gitignore", "target/\ny\n");
        git_in(&dir, &["stash", "-q"]);
        let (code, lines) = check_round_1(&dir, &home);
        assert_eq!(code, 3);
        assert!(lines[0].starts_with("stash: "), "{lines:?}");
        assert!(
            !lines.iter().any(|line| line.starts_with("status: ")),
            "the stash leaves the tree clean: {lines:?}"
        );
    }

    #[test]
    fn check_sees_a_tag() {
        let (dir, home) = recorded("cap-tags");
        git_in(&dir, &["tag", "t"]);
        assert_check_names(&dir, &home, "tags: ");
    }

    #[test]
    fn check_sees_a_push() {
        let (dir, home) = recorded("cap-push");
        let origin = dir.beside("origin");
        git_in(&dir, &["init", "-q", "--bare", origin.to_str().unwrap()]);
        git_in(&dir, &["remote", "add", "origin", origin.to_str().unwrap()]);
        let top = toplevel(&dir).unwrap();
        let state = take_state(&top, &home).unwrap();
        std::fs::write(
            state_path(&top, 1).unwrap(),
            serde_json::to_string(&state).unwrap(),
        )
        .unwrap();
        git_in(&dir, &["push", "-q", "--no-verify", "origin", "main:acme"]);
        let (code, lines) = check_round_1(&dir, &home);
        assert_eq!(code, 3, "{lines:?}");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("other_refs: appears: refs/remotes/origin/acme "),
            "{lines:?}"
        );
    }

    #[test]
    fn other_refs_leave_out_heads_tags_and_the_stash() {
        let (dir, home) = recorded("cap-other-refs");
        git_in(&dir, &["update-ref", "refs/notes/commits", "HEAD"]);
        write(&dir, ".gitignore", "target/\nz\n");
        git_in(&dir, &["stash", "-q"]);
        git_in(&dir, &["tag", "t"]);
        let state = take_state(&toplevel(&dir).unwrap(), &home).unwrap();
        assert_eq!(state.other_refs.len(), 1, "{:?}", state.other_refs);
        assert!(
            state.other_refs[0].starts_with("refs/notes/commits "),
            "{:?}",
            state.other_refs
        );
    }

    #[test]
    fn check_sees_a_remote() {
        let (dir, home) = recorded("cap-remotes");
        git_in(&dir, &["remote", "add", "o", "/x"]);
        assert_check_names(&dir, &home, "remotes: ");
    }

    #[test]
    fn check_sees_a_config_change() {
        let (dir, home) = recorded("cap-config");
        git_in(&dir, &["config", "x.y", "z"]);
        assert_check_names(&dir, &home, "hashes: config: ");
    }

    #[test]
    fn check_sees_a_new_hook() {
        let (dir, home) = recorded("cap-hooks");
        let hooks = git_in(&dir, &["rev-parse", "--git-path", "hooks"]);
        let hooks = dir.join(hooks);
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(hooks.join("beachape-hook"), "#!/bin/sh\n").unwrap();
        assert_check_names(&dir, &home, "hashes: hooks/beachape-hook: ");
    }

    #[test]
    fn check_sees_local_claude_settings() {
        let (dir, home) = recorded("cap-local");
        write(&dir, ".claude/settings.local.json", "{}\n");
        assert_check_names(&dir, &home, "hashes: .claude/settings.local.json: ");
    }

    #[test]
    fn check_sees_the_home_claude_settings() {
        let (dir, home) = recorded("cap-home");
        write(&home, ".claude/settings.json", "{}\n");
        assert_check_names(&dir, &home, "hashes: ~/.claude/settings.json: ");
    }

    #[test]
    fn check_passes_an_unchanged_repo_and_sees_a_branch() {
        let (dir, home) = recorded("cap-branches");
        assert_eq!(
            check_round_1(&dir, &home),
            (0, vec!["no change".to_string()])
        );
        git_in(&dir, &["branch", "x"]);
        assert_check_names(&dir, &home, "branches: ");
    }

    #[test]
    fn check_without_a_record_exits_3() {
        let dir = scratch("no-record");
        let (code, lines) = check_round_1(&dir, &dir);
        assert_eq!(code, 3);
        assert!(
            lines[0].starts_with("no record for this branch and round"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_worktree_under_target_review_is_left_out() {
        let (dir, home) = recorded("cap-wt-in");
        let inside = dir.join("target/review/m1-x");
        git_in(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                inside.to_str().unwrap(),
            ],
        );
        assert_eq!(
            check_round_1(&dir, &home),
            (0, vec!["no change".to_string()])
        );
    }

    #[test]
    fn a_worktree_elsewhere_is_a_change() {
        let (dir, home) = recorded("cap-wt-out");
        let outside = dir.beside("wt");
        git_in(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                outside.to_str().unwrap(),
            ],
        );
        assert_check_names(&dir, &home, "worktrees: ");
        git_in(
            &dir,
            &["worktree", "remove", "--force", outside.to_str().unwrap()],
        );
    }

    #[test]
    fn a_relative_hooks_path_is_followed() {
        let dir = scratch("hooks-path");
        git_in(&dir, &["config", "core.hooksPath", "githooks"]);
        write(&dir, "githooks/beachape-hook", "#!/bin/sh\n");
        let paths: BTreeMap<String, PathBuf> = hashed_paths(&toplevel(&dir).unwrap(), &dir)
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(
            paths["hooks/beachape-hook"].canonicalize().unwrap(),
            dir.join("githooks/beachape-hook").canonicalize().unwrap()
        );
        assert!(
            !paths.keys().any(|label| label.ends_with(".sample")),
            "the default hooks folder is not read: {paths:?}"
        );
    }

    #[test]
    fn a_worktree_under_the_main_worktrees_target_review_is_left_out() {
        let dir = scratch("main-target");
        let linked = dir.beside("linked-main");
        git_in(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                linked.to_str().unwrap(),
            ],
        );
        let top = toplevel(&linked).unwrap();
        let before = take_state(&top, &dir).unwrap();
        let inside = dir.join("target/review/m1-x");
        git_in(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                inside.to_str().unwrap(),
            ],
        );
        let after = take_state(&top, &dir).unwrap();
        assert_eq!(changes(&before, &after), Vec::<String>::new());
    }

    #[test]
    fn a_linked_worktree_hashes_the_common_config_and_hooks() {
        let dir = scratch("linked");
        let common = dir.join(".git").canonicalize().unwrap();
        std::fs::create_dir_all(common.join("hooks")).unwrap();
        std::fs::write(common.join("hooks/beachape-hook"), "#!/bin/sh\n").unwrap();
        let linked = dir.beside("linked");
        git_in(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                linked.to_str().unwrap(),
            ],
        );
        let paths: BTreeMap<String, PathBuf> = hashed_paths(&toplevel(&linked).unwrap(), &dir)
            .unwrap()
            .into_iter()
            .collect();
        let canonical_parent = |label: &str| paths[label].parent().unwrap().canonicalize().unwrap();
        assert_eq!(canonical_parent("config"), common);
        assert_eq!(
            canonical_parent("hooks/beachape-hook"),
            common.join("hooks")
        );
        assert_eq!(
            canonical_parent("config.worktree"),
            common.join("worktrees").join(linked.file_name().unwrap())
        );
        git_in(
            &dir,
            &["worktree", "remove", "--force", linked.to_str().unwrap()],
        );
    }

    // `review start`.

    fn start_in(dir: &Path, base: &str, round: &str) -> (i32, Vec<String>) {
        run(
            &["start".to_string(), base.to_string(), round.to_string()],
            dir,
            &dir.join("target/no-home"),
            SystemTime::now(),
        )
    }

    fn value_of(lines: &[String], key: &str) -> String {
        lines
            .iter()
            .find_map(|line| line.strip_prefix(key))
            .unwrap_or_else(|| panic!("no {key:?} line in {lines:?}"))
            .to_string()
    }

    fn sha(dir: &Path, rev: &str) -> String {
        git_in(dir, &["rev-parse", rev])
    }

    #[test]
    fn start_resolves_base_to_the_merge_base() {
        let dir = scratch("start-base");
        let fork = sha(&dir, "HEAD");
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "a.txt", "a\n");
        commit_all(&dir, "on the branch");
        git_in(&dir, &["checkout", "-q", "main"]);
        write(&dir, "b.txt", "b\n");
        commit_all(&dir, "on main");
        git_in(&dir, &["checkout", "-q", "feature"]);
        let (code, lines) = start_in(&dir, "main", "1");
        assert_eq!(code, 0, "{lines:?}");
        assert_eq!(
            value_of(&lines, "base: "),
            format!("{fork} (merge base of main and HEAD)")
        );
        let folder = PathBuf::from(value_of(&lines, "run folder: "));
        let diff = std::fs::read_to_string(folder.join("1/branch.diff")).unwrap();
        assert!(diff.contains("a.txt"), "{diff}");
        assert!(!diff.contains("b.txt"), "{diff}");
        let state = read_state(&folder.join("1/state.json")).unwrap();
        assert_eq!(state.base, fork);
        assert_eq!(state.head, sha(&dir, "HEAD"));
        assert!(state.started > 0, "start records its time");
    }

    #[test]
    fn since_is_the_base_then_the_last_rounds_head() {
        let dir = scratch("start-since");
        let base = sha(&dir, "HEAD");
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "a.txt", "a\n");
        write(&dir, "docs/b.md", "only in round 1\n");
        commit_all(&dir, "one");
        let first_head = sha(&dir, "HEAD");
        let (code, lines) = start_in(&dir, "main", "1");
        assert_eq!(code, 0, "{lines:?}");
        assert_eq!(value_of(&lines, "since: "), base);
        write(&dir, "a.txt", "a fix\n");
        commit_all(&dir, "fix");
        let (code, lines) = start_in(&dir, "main", "2");
        assert_eq!(code, 0, "{lines:?}");
        assert_eq!(value_of(&lines, "since: "), first_head);
        let folder = PathBuf::from(value_of(&lines, "run folder: "));
        let since_diff = std::fs::read_to_string(folder.join("2/since.diff")).unwrap();
        assert!(since_diff.contains("+a fix"), "{since_diff}");
        assert!(!since_diff.contains("b.md"), "{since_diff}");
        let branch_diff = std::fs::read_to_string(folder.join("2/branch.diff")).unwrap();
        assert!(branch_diff.contains("docs/b.md"), "{branch_diff}");
        let branch_at = lines.iter().position(|l| l.starts_with("branch ")).unwrap();
        let since_at = lines.iter().position(|l| l.starts_with("since ")).unwrap();
        let branch_lines = &lines[branch_at + 1..since_at];
        assert!(
            branch_lines.contains(&"  docs-reviewer: docs/b.md (docs/**)".to_string()),
            "the branch route of round 2 starts at the merge base: {lines:?}"
        );
        assert!(
            !lines[since_at + 1..]
                .iter()
                .any(|l| l.starts_with("  docs-reviewer")),
            "{lines:?}"
        );
    }

    #[test]
    fn round_2_without_a_round_1_record_exits_3() {
        let dir = scratch("start-no-1");
        let (code, lines) = start_in(&dir, "main", "2");
        assert_eq!(code, 3, "{lines:?}");
        assert!(lines.join("\n").contains("1/state.json"), "{lines:?}");
    }

    #[test]
    fn round_1_refuses_a_run_folder_with_a_log() {
        let dir = scratch("start-log");
        let folder = dir.join("target/review/run/main");
        std::fs::create_dir_all(folder.join("1")).unwrap();
        let (code, lines) = start_in(&dir, "main", "1");
        assert_eq!(code, 0, "a round folder without a log is fine: {lines:?}");
        std::fs::write(folder.join("log.md"), "# log\n").unwrap();
        let (code, lines) = start_in(&dir, "main", "1");
        assert_eq!(code, 3);
        assert_eq!(
            lines,
            ["the run folder holds a log: copy it out, or remove it"]
        );
    }

    #[test]
    fn a_log_does_not_stop_round_2() {
        let dir = round_1_of_a_rust_change("start-log-2");
        let folder = toplevel(&dir).unwrap().join("target/review/run/feature");
        std::fs::write(folder.join("log.md"), "# log\n").unwrap();
        write(&dir, "a.rs", "fn a() { let _ = 1; }\n");
        commit_all(&dir, "a fix");
        let (code, lines) = start_in(&dir, "main", "2");
        assert_eq!(code, 0, "{lines:?}");
    }

    #[test]
    fn a_dirty_tree_exits_3() {
        let dir = scratch("start-dirty");
        hostile_config(&dir);
        write(&dir, "x.txt", "x\n");
        let (code, lines) = start_in(&dir, "main", "1");
        assert_eq!(code, 3);
        assert!(lines[0].starts_with("commit first"), "{lines:?}");
    }

    #[test]
    fn an_unknown_base_exits_3() {
        let dir = scratch("start-unknown");
        let (code, _) = start_in(&dir, "acme", "1");
        assert_eq!(code, 3);
    }

    #[test]
    fn the_run_folder_is_named_for_the_branch_or_the_commit() {
        let dir = scratch("start-names");
        git_in(&dir, &["checkout", "-q", "-b", "a/b"]);
        let (_, lines) = start_in(&dir, "main", "1");
        let top = toplevel(&dir).unwrap();
        assert_eq!(
            PathBuf::from(value_of(&lines, "run folder: ")),
            top.join("target/review/run/a-b")
        );
        git_in(&dir, &["checkout", "-q", "--detach"]);
        let short = git_in(&dir, &["rev-parse", "--short", "HEAD"]);
        let (_, lines) = start_in(&dir, "main", "1");
        assert_eq!(
            PathBuf::from(value_of(&lines, "run folder: ")),
            top.join(format!("target/review/run/detached-{short}"))
        );
    }

    #[test]
    fn start_warns_when_the_branch_changes_review_rs() {
        let dir = scratch("start-changed-source");
        write(&dir, "xtask/src/review.rs", "fn a() {}\n");
        commit_all(&dir, "the route");
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "xtask/src/review.rs", "fn b() {}\n");
        commit_all(&dir, "a new route");
        let later = SystemTime::now() + Duration::from_secs(3600);
        let (code, lines) = run(
            &["start".to_string(), "main".to_string(), "1".to_string()],
            &dir,
            &dir.join("target/no-home"),
            later,
        );
        assert_eq!(code, 0, "{lines:?}");
        let warnings: Vec<&String> = lines.iter().filter(|l| l.starts_with("warning")).collect();
        assert_eq!(
            warnings,
            [
                "warning: this branch changes xtask/src/review.rs, so the route comes from the branch"
            ]
        );
    }

    #[test]
    fn an_older_binary_warns() {
        let now = SystemTime::now();
        let earlier = now - std::time::Duration::from_secs(60);
        assert_eq!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                earlier,
                Some(now),
                None,
                None
            ),
            [
                "warning: /r/target/debug/xtask is older than xtask/src/review.rs; run cargo build -p xtask"
            ]
        );
        assert!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                now,
                Some(earlier),
                None,
                None
            )
            .is_empty()
        );
        assert!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                earlier,
                None,
                None,
                None
            )
            .is_empty()
        );
    }

    #[test]
    fn a_changed_review_rs_warns() {
        let now = SystemTime::now();
        assert_eq!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                now,
                None,
                Some("b1"),
                Some("b2")
            ),
            [
                "warning: this branch changes xtask/src/review.rs, so the route comes from the branch"
            ]
        );
        assert!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                now,
                None,
                Some("b1"),
                Some("b1")
            )
            .is_empty()
        );
        assert!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                now,
                None,
                None,
                Some("b2")
            )
            .is_empty()
        );
        assert!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                now,
                None,
                Some("b1"),
                None
            )
            .is_empty()
        );
    }

    #[test]
    fn a_repo_without_review_rs_gives_no_warning() {
        let dir = scratch("start-no-source");
        let (code, lines) = run(
            &["start".to_string(), "main".to_string(), "1".to_string()],
            &dir,
            &dir.join("target/no-home"),
            SystemTime::UNIX_EPOCH,
        );
        assert_eq!(code, 0, "{lines:?}");
        assert!(!lines.iter().any(|l| l.starts_with("warning")), "{lines:?}");
    }

    #[test]
    fn exactly_5_path_reasons_give_no_more() {
        let raw: String = (1..=5)
            .map(|n| format!(":100644 100644 {SHA} {SHA} M\0engine/src/f{n}.rs\0"))
            .collect();
        let lines = route_lines(&route(&raw, ""), Some(PATH_REASONS_SHOWN));
        let memory = lines
            .iter()
            .find(|l| l.starts_with("  memory-reviewer: "))
            .unwrap();
        assert_eq!(memory.split("; ").count(), 5, "{memory}");
        assert!(!memory.contains("more)"), "{memory}");
    }

    #[test]
    fn start_prints_at_most_5_path_reasons_and_every_sign() {
        let dir = scratch("start-output");
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        for n in 1..=6 {
            write(&dir, &format!("engine/src/f{n}.rs"), "fn f() {}\n");
        }
        write(&dir, "xtask/src/x.rs", "let a = b.to_vec();\n");
        commit_all(&dir, "six files and a sign");
        let (code, lines) = start_in(&dir, "main", "1");
        assert_eq!(code, 0, "{lines:?}");
        let memory: Vec<&String> = lines
            .iter()
            .filter(|l| l.starts_with("  memory-reviewer: "))
            .collect();
        assert_eq!(memory.len(), 2, "one line for each range: {lines:?}");
        let shown: Vec<&str> = memory[0]
            .trim_start_matches("  memory-reviewer: ")
            .split("; ")
            .collect();
        assert_eq!(
            shown,
            [
                "engine/src/f1.rs (engine/**)",
                "engine/src/f2.rs (engine/**)",
                "engine/src/f3.rs (engine/**)",
                "engine/src/f4.rs (engine/**)",
                "engine/src/f5.rs (engine/**)",
                "(+1 more)",
                "`to_vec` in xtask/src/x.rs",
            ]
        );
        let folder = PathBuf::from(value_of(&lines, "run folder: "));
        let route_txt = std::fs::read_to_string(folder.join("1/route.txt")).unwrap();
        let full = route_txt
            .lines()
            .find(|l| l.starts_with("  memory-reviewer: "))
            .unwrap();
        assert_eq!(full.split("; ").count(), 7, "{full}");
        assert!(full.contains("engine/src/f6.rs (engine/**)"), "{full}");
        let (code, lines) = start_in(&dir, "main", "2");
        assert_eq!(code, 0, "{lines:?}");
        let since_at = lines.iter().position(|l| l.starts_with("since ")).unwrap();
        assert_eq!(lines[since_at + 1], "  no changes", "{lines:?}");
    }

    #[test]
    fn start_prints_the_ranges_and_the_target_folder() {
        let dir = scratch("start-lines");
        let (code, lines) = start_in(&dir, "main", "1");
        assert_eq!(code, 0, "{lines:?}");
        let top = toplevel(&dir).unwrap();
        let head = sha(&dir, "HEAD");
        assert_eq!(
            lines,
            [
                format!("base: {head} (merge base of main and HEAD)"),
                format!("since: {head}"),
                format!(
                    "run folder: {}",
                    top.join("target/review/run/main").display()
                ),
                format!(
                    "target folder: {}",
                    top.join("target/review/target").display()
                ),
                format!("branch {head}..{head}:"),
                "  no changes".to_string(),
                format!("since {head}..{head}:"),
                "  no changes".to_string(),
            ]
        );
    }

    #[test]
    fn a_round_that_is_not_a_whole_number_of_1_or_more_exits_3() {
        let dir = scratch("start-round");
        for round in ["0", "one", "-1"] {
            let (code, lines) = start_in(&dir, "main", round);
            assert_eq!(code, 3, "{round}");
            assert!(lines.join("\n").contains(USAGE), "{lines:?}");
        }
    }

    // `review mutants`.

    fn ended(code: i32) -> RunEnd {
        RunEnd {
            code: Some(code),
            ..RunEnd::default()
        }
    }

    fn some_counts() -> Option<Counts> {
        Some(Counts {
            caught: 3,
            missed: 2,
            timeout: 1,
            unviable: 4,
            listed: Some(15),
        })
    }

    const COUNTED: &str = "tested 9 (caught 3, missed 2, unviable 4), timeouts 1, not tested 5";
    const NOTHING: &str = "tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0";

    fn assert_line(end: RunEnd, counts: Option<Counts>, expected: &str) {
        assert_eq!(classify(end, counts).to_string(), expected);
    }

    #[test]
    fn exit_0_is_full() {
        assert_line(
            ended(0),
            some_counts(),
            &format!("mutants: full: {COUNTED}"),
        );
    }

    #[test]
    fn exit_2_is_full() {
        assert_line(
            ended(2),
            some_counts(),
            &format!("mutants: full: {COUNTED}"),
        );
    }

    #[test]
    fn exit_3_is_partial() {
        assert_line(
            ended(3),
            some_counts(),
            &format!("mutants: partial: {COUNTED}; timeouts have no result"),
        );
    }

    #[test]
    fn exit_1_is_none() {
        assert_line(
            ended(1),
            None,
            &format!(
                "mutants: none: {NOTHING}; exit 1: a usage error, or a stop by a signal that was not its own"
            ),
        );
    }

    #[test]
    fn exit_4_is_none() {
        assert_line(
            ended(4),
            None,
            &format!("mutants: none: {NOTHING}; exit 4: the baseline failed"),
        );
    }

    #[test]
    fn exit_5_is_none() {
        assert_line(
            ended(5),
            None,
            &format!("mutants: none: {NOTHING}; exit 5: the diff does not fit the tree"),
        );
    }

    #[test]
    fn exit_6_is_none() {
        assert_line(
            ended(6),
            None,
            &format!("mutants: none: {NOTHING}; exit 6: the diff cannot be read"),
        );
    }

    #[test]
    fn exit_70_is_none() {
        assert_line(
            ended(70),
            None,
            &format!("mutants: none: {NOTHING}; exit 70: an internal error of cargo-mutants"),
        );
    }

    #[test]
    fn another_exit_code_is_none() {
        assert_line(
            ended(9),
            None,
            &format!(
                "mutants: none: {NOTHING}; exit 9: a code that cargo-mutants does not document"
            ),
        );
    }

    #[test]
    fn a_signal_is_none() {
        assert_line(
            RunEnd::default(),
            None,
            &format!("mutants: none: {NOTHING}; stopped by a signal that was not its own"),
        );
    }

    #[test]
    fn missing_cargo_mutants_is_none() {
        let end = RunEnd {
            missing: true,
            ..RunEnd::default()
        };
        assert_line(end, None, &format!("mutants: none: {NOTHING}; {INSTALL}"));
    }

    #[test]
    fn its_own_stop_with_some_tested_is_partial() {
        let end = RunEnd {
            code: Some(1),
            own_stop: true,
            ..RunEnd::default()
        };
        assert_line(
            end,
            some_counts(),
            &format!("mutants: partial: {COUNTED}; stopped at the limit"),
        );
    }

    #[test]
    fn its_own_stop_with_only_unviable_tested_is_partial() {
        let end = RunEnd {
            code: Some(1),
            own_stop: true,
            ..RunEnd::default()
        };
        let unviable = Some(Counts {
            unviable: 2,
            listed: Some(5),
            ..Counts::default()
        });
        assert_line(
            end,
            unviable,
            "mutants: partial: tested 2 (caught 0, missed 0, unviable 2), timeouts 0, not tested 3; stopped at the limit",
        );
    }

    #[test]
    fn its_own_stop_with_none_tested_is_none() {
        let end = RunEnd {
            code: Some(1),
            own_stop: true,
            ..RunEnd::default()
        };
        let only_timeouts = Some(Counts {
            timeout: 2,
            listed: Some(7),
            ..Counts::default()
        });
        assert_line(
            end,
            only_timeouts,
            "mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 2, not tested 5; stopped before a mutant finished",
        );
    }

    #[test]
    fn a_kill_after_its_own_stop_is_named() {
        let end = RunEnd {
            code: None,
            own_stop: true,
            killed: true,
            ..RunEnd::default()
        };
        assert_line(
            end,
            some_counts(),
            &format!(
                "mutants: partial: {COUNTED}; stopped at the limit; SIGKILL 60 s after SIGTERM"
            ),
        );
    }

    #[test]
    fn not_tested_is_0_without_mutants_json() {
        let counts = Some(Counts {
            caught: 1,
            listed: None,
            ..Counts::default()
        });
        assert_line(
            ended(0),
            counts,
            "mutants: full: tested 1 (caught 1, missed 0, unviable 0), timeouts 0, not tested 0",
        );
    }

    #[test]
    fn the_clear_step_removes_old_output() {
        let dir = scratch("clear");
        let round_folder = dir.join("target/review/run/main/1");
        for old in ["mutants.out", "mutants.out.old"] {
            std::fs::create_dir_all(round_folder.join(old)).unwrap();
            std::fs::write(
                round_folder.join(old).join("outcomes.json"),
                r#"{"caught": 5, "missed": 0, "timeout": 0, "unviable": 0}"#,
            )
            .unwrap();
        }
        clear_old_output(&round_folder).unwrap();
        assert!(!round_folder.join("mutants.out").exists());
        assert!(!round_folder.join("mutants.out.old").exists());
        assert!(round_folder.exists());
        assert_line(
            ended(0),
            read_counts(&round_folder.join("mutants.out")),
            &format!("mutants: none: {NOTHING}; no mutants in the diff"),
        );
    }

    #[test]
    fn the_clear_step_removes_the_old_mutants_line() {
        let dir = scratch("clear-line");
        let round_folder = dir.join("target/review/run/main/1");
        std::fs::create_dir_all(&round_folder).unwrap();
        let old_line = round_folder.join("mutants.txt");
        std::fs::write(&old_line, format!("mutants: full: {COUNTED}\n")).unwrap();
        clear_old_output(&round_folder).unwrap();
        assert!(
            !old_line.exists(),
            "a round started again must not show the line of the last run"
        );
        assert!(round_folder.exists());
    }

    #[test]
    fn read_counts_reads_outcomes_and_mutants_json() {
        let dir = scratch("counts");
        let out = dir.join("mutants.out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(
            out.join("outcomes.json"),
            r#"{"total_mutants": 10, "caught": 3, "missed": 2, "timeout": 1, "unviable": 4}"#,
        )
        .unwrap();
        std::fs::write(
            out.join("mutants.json"),
            format!("[{}]", vec!["{}"; 15].join(",")),
        )
        .unwrap();
        assert_eq!(read_counts(&out), some_counts());
    }

    #[test]
    fn the_limit_sends_sigterm_not_sigkill() {
        let mut command = Command::new("sh");
        command.args(["-c", "trap \"exit 7\" TERM; sleep 30 & wait"]);
        let begun = std::time::Instant::now();
        let end = run_with_limit(command, Duration::from_secs(2));
        assert!(
            begun.elapsed() < Duration::from_secs(10),
            "{:?}",
            begun.elapsed()
        );
        assert_eq!(
            end,
            RunEnd {
                code: Some(7),
                own_stop: true,
                killed: false,
                missing: false,
            }
        );
    }

    #[test]
    fn a_run_that_ignores_sigterm_gets_sigkill() {
        // The shell ends by itself after 30 s, so a run that never sends
        // SIGKILL fails the wait below and leaves nothing behind.
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "trap \"\" TERM; i=0; while [ $i -lt 30 ]; do sleep 1; i=$((i+1)); done; exit 9",
        ]);
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let end =
                run_with_limit_and_grace(command, Duration::from_secs(1), Duration::from_secs(2));
            let _ = send.send(end);
        });
        let end = receive
            .recv_timeout(Duration::from_secs(15))
            .expect("the run ends within 15 s");
        assert_eq!(
            end,
            RunEnd {
                code: None,
                own_stop: true,
                killed: true,
                missing: false,
            }
        );
    }

    #[test]
    fn a_run_that_ends_in_time_is_not_stopped() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 2"]);
        assert_eq!(run_with_limit(command, Duration::from_secs(5)), ended(2));
    }

    #[test]
    fn mutants_without_a_record_exits_3() {
        let dir = scratch("mutants-no-record");
        let (code, lines) = run(
            &["mutants".to_string(), "1".to_string()],
            &dir,
            &dir,
            SystemTime::UNIX_EPOCH,
        );
        assert_eq!(code, 3);
        assert!(
            lines[0].starts_with("no record for this branch and round"),
            "{lines:?}"
        );
    }

    #[test]
    fn the_mutants_diff_holds_rust_files_only_with_renames_in_full() {
        let dir = scratch("mutants-diff");
        let body: String = (0..20).map(|i| format!("fn f{i}() {{}}\n")).collect();
        write(&dir, "a.rs", "fn a() {}\n");
        write(&dir, "b.md", "# b\n");
        write(&dir, "src/c.rs", &body);
        commit_all(&dir, "first");
        write(&dir, "a.rs", "fn a() { let _ = 1; }\n");
        write(&dir, "b.md", "# b, changed\n");
        git_in(&dir, &["mv", "src/c.rs", "src/d.rs"]);
        write(
            &dir,
            "src/d.rs",
            &body.replace("fn f3() {}", "fn f3() { let _ = 3; }"),
        );
        commit_all(&dir, "second");
        let path = dir.join("target/mutants.diff");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let top = toplevel(&dir).unwrap();
        write_mutants_diff(&top, &sha(&dir, "HEAD~1"), &sha(&dir, "HEAD"), &path).unwrap();
        let diff = std::fs::read_to_string(&path).unwrap();
        assert!(diff.contains("+++ b/a.rs"), "{diff}");
        assert!(diff.contains("--- a/src/c.rs\n+++ /dev/null"), "{diff}");
        assert!(diff.contains("--- /dev/null\n+++ b/src/d.rs"), "{diff}");
        assert!(!diff.contains("b.md"), "{diff}");
        assert!(!diff.contains("rename"), "{diff}");
    }

    #[test]
    fn the_limit_is_60_minutes_or_the_variable() {
        assert_eq!(parse_limit(None).unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_limit(Some("2.5")).unwrap(), Duration::from_secs(150));
        for wrong in ["0", "x", "-1", "1e300", "inf", "NaN"] {
            assert!(parse_limit(Some(wrong)).is_err(), "{wrong}");
        }
    }

    #[test]
    fn the_cargo_mutants_arguments() {
        let args = mutants_args(
            Path::new("/r/run/1/mutants.diff"),
            Path::new("/r/run/1"),
            Path::new("/r/.cargo/mutants.toml"),
        );
        assert_eq!(
            args,
            [
                "mutants",
                "--in-diff",
                "/r/run/1/mutants.diff",
                "--output",
                "/r/run/1",
                "--copy-vcs=true",
                "--gitignore=true",
                "--shuffle",
                "-j2",
                "--timeout",
                "300",
                "--config",
                "/r/.cargo/mutants.toml",
            ]
        );
    }

    #[test]
    fn the_command_runs_cargo_at_the_toplevel_without_cargo_target_dir() {
        let args = vec![OsString::from("mutants"), OsString::from("--list")];
        let command = mutants_command(OsStr::new("cargo"), Path::new("/r"), &args);
        assert_eq!(command.get_program(), "cargo");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["mutants", "--list"]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/r")));
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(
            envs,
            [(std::ffi::OsStr::new("CARGO_TARGET_DIR"), None)],
            "CARGO_TARGET_DIR is removed"
        );
    }

    #[test]
    fn the_mutants_diff_starts_at_since() {
        let dir = scratch("mutants-since");
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "a.rs", "fn a() {}\n");
        commit_all(&dir, "round 1");
        assert_eq!(start_in(&dir, "main", "1").0, 0);
        write(&dir, "b.rs", "fn b() {}\n");
        commit_all(&dir, "a fix");
        assert_eq!(start_in(&dir, "main", "2").0, 0);
        let top = toplevel(&dir).unwrap();
        let round_folder = prepare_mutants(&top, 2).unwrap().unwrap();
        let diff = std::fs::read_to_string(round_folder.join("mutants.diff")).unwrap();
        assert!(diff.contains("+++ b/b.rs"), "{diff}");
        assert!(!diff.contains("a.rs"), "{diff}");
        assert_eq!(prepare_mutants(&top, 3).unwrap(), None);
    }

    /// A fake `cargo` in POSIX sh.  `mutants --version` passes.  A run
    /// writes counts to `<output>/mutants.out`, keeps the `--config` that
    /// it got in `<output>/config-seen.txt` and the `--in-diff` in
    /// `<output>/in-diff-seen.txt`, and exits 2 (missed mutants).
    const FAKE_CARGO: &str = r#"#!/bin/sh
if [ "$2" = "--version" ]; then exit VERSION_EXIT; fi
out=""
config=""
while [ $# -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift ;;
    --config) config="$2"; shift ;;
    --in-diff) diff="$2"; shift ;;
  esac
  shift
done
mkdir -p "$out/mutants.out"
printf '{"caught": 3, "missed": 2, "timeout": 1, "unviable": 4}' > "$out/mutants.out/outcomes.json"
printf '[{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}]' > "$out/mutants.out/mutants.json"
printf '%s' "$config" > "$out/config-seen.txt"
printf '%s' "$diff" > "$out/in-diff-seen.txt"
exit 2
"#;

    /// Writes [`FAKE_CARGO`] to `bin`; its `mutants --version` exits
    /// `version_exit`.
    fn fake_cargo(bin: &Path, version_exit: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let cargo = bin.join("cargo");
        let script = FAKE_CARGO.replace("VERSION_EXIT", &version_exit.to_string());
        std::fs::write(&cargo, script).unwrap();
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
        cargo
    }

    /// A scratch repo with a round 1 record of one Rust change.
    fn round_1_of_a_rust_change(name: &str) -> Scratch {
        let dir = scratch(name);
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "a.rs", "fn a() {}\n");
        commit_all(&dir, "round 1");
        assert_eq!(start_in(&dir, "main", "1").0, 0);
        dir
    }

    #[test]
    fn mutants_runs_the_program_and_saves_its_line() {
        let dir = round_1_of_a_rust_change("mutants-fake");
        let bin = Scratch::new("mutants-fake-bin");
        let cargo = fake_cargo(&bin, 0);
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let round_folder = toplevel(&dir).unwrap().join("target/review/run/feature/1");
        std::fs::create_dir_all(round_folder.join("mutants.out")).unwrap();
        std::fs::write(
            round_folder.join("mutants.out/outcomes.json"),
            r#"{"caught": 99, "missed": 0, "timeout": 0, "unviable": 0}"#,
        )
        .unwrap();
        let args: Vec<String> = ["mutants", "1", "--config", "x.toml"]
            .iter()
            .map(ToString::to_string)
            .collect();
        let (code, lines) = run_with(&args, &sub, &dir, SystemTime::now(), cargo.as_os_str());
        let expected = format!("mutants: full: {COUNTED}");
        assert_eq!((code, lines), (0, vec![expected.clone()]));
        assert_eq!(
            std::fs::read_to_string(round_folder.join("mutants.txt")).unwrap(),
            format!("{expected}\n")
        );
        assert_eq!(
            PathBuf::from(std::fs::read_to_string(round_folder.join("config-seen.txt")).unwrap()),
            sub.join("x.toml"),
            "--config reached cargo, resolved against the current folder"
        );
        assert_eq!(
            PathBuf::from(std::fs::read_to_string(round_folder.join("in-diff-seen.txt")).unwrap()),
            round_folder.join("mutants.diff"),
            "the round's mutants.diff reached cargo"
        );
    }

    #[test]
    fn mutants_without_the_program_is_none() {
        let dir = round_1_of_a_rust_change("mutants-missing");
        let args = vec!["mutants".to_string(), "1".to_string()];
        // cargo without cargo-mutants: `cargo mutants --version` fails.
        let bin = Scratch::new("mutants-missing-bin");
        let cargo = fake_cargo(&bin, 101);
        let (code, lines) = run_with(&args, &dir, &dir, SystemTime::now(), cargo.as_os_str());
        let expected = format!("mutants: none: {NOTHING}; {INSTALL}");
        assert_eq!((code, lines), (0, vec![expected.clone()]));
        let round_folder = toplevel(&dir).unwrap().join("target/review/run/feature/1");
        assert_eq!(
            std::fs::read_to_string(round_folder.join("mutants.txt")).unwrap(),
            format!("{expected}\n")
        );
        assert!(!round_folder.join("mutants.out").exists(), "nothing ran");
    }

    #[test]
    fn prepare_mutants_removes_the_old_output() {
        let dir = round_1_of_a_rust_change("mutants-clear");
        let top = toplevel(&dir).unwrap();
        let round_folder = top.join("target/review/run/feature/1");
        std::fs::create_dir_all(round_folder.join("mutants.out")).unwrap();
        std::fs::create_dir_all(round_folder.join("mutants.out.old")).unwrap();
        assert_eq!(
            prepare_mutants(&top, 1).unwrap(),
            Some(round_folder.clone())
        );
        assert!(!round_folder.join("mutants.out").exists());
        assert!(!round_folder.join("mutants.out.old").exists());
    }

    #[test]
    fn a_program_that_does_not_start_is_missing() {
        let command = Command::new("/nonexistent/xtask-review-no-such-program");
        assert_eq!(
            run_with_limit(command, Duration::from_secs(1)),
            RunEnd {
                missing: true,
                ..RunEnd::default()
            }
        );
    }

    #[test]
    fn the_config_path_resolves_against_the_current_folder() {
        let top = Path::new("/r");
        let dir = Path::new("/r/sub");
        assert_eq!(
            config_path(top, dir, None),
            PathBuf::from("/r/.cargo/mutants.toml")
        );
        assert_eq!(
            config_path(top, dir, Some("x.toml")),
            PathBuf::from("/r/sub/x.toml")
        );
        assert_eq!(
            config_path(top, dir, Some("/elsewhere/x.toml")),
            PathBuf::from("/elsewhere/x.toml")
        );
    }

    // The command.

    #[test]
    fn an_unknown_subcommand_exits_3_with_the_usage() {
        let (code, lines) = run(
            &["nope".to_string()],
            Path::new("."),
            Path::new("."),
            SystemTime::UNIX_EPOCH,
        );
        assert_eq!(code, 3);
        assert_eq!(lines, [USAGE]);
    }
}
