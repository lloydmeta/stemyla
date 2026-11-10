//! `xtask review`: chooses the reviewers from a diff, guards a review round,
//! runs cargo-mutants on a round's diff, removes a round's worktrees, and
//! checks a run's log against the fix rule of decision 0024.
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
            "decoder/**",
            "protocol/**",
            "roformer/src/**",
            "roformer-gpu/src/**",
            "synth/src/**",
            "dsp/src/**",
            "m4a/src/**",
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
            "decoder/src/**",
            "app/src/separator.rs",
            "app/src/decoder.rs",
            "app/src/decoder/**",
            "xtask/src/e2e.rs",
            "xtask/src/e2e/**",
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
    "xtask/src/review_log.rs",
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

const USAGE: &str = "usage: xtask review start <base> <round> | check <round> | mutants <round> [--config <file>] | clean <round> | log-check <file>";

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
            ["clean", round] => parse_round(round).and_then(|round| clean(&toplevel(dir)?, round)),
            ["log-check", file] => log_check(&dir.join(file)),
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

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("xtask"));
    let mut source_warnings = Vec::new();
    for (source, what) in WATCHED_SOURCES {
        let source_time = std::fs::metadata(toplevel.join(source))
            .and_then(|m| m.modified())
            .ok();
        let blob = |rev: &str| {
            git(
                toplevel,
                &["rev-parse", "--verify", "-q", &format!("{rev}:{source}")],
            )
            .ok()
            .map(|sha| sha.trim().to_string())
        };
        source_warnings.extend(warnings(
            &exe,
            source,
            what,
            exe_time,
            source_time,
            blob(&base).as_deref(),
            blob(&head).as_deref(),
        ));
    }
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
    .chain(source_warnings)
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

/// The sources of `xtask review` that a branch can change, each with what
/// then comes from the branch.  `review start` warns on each.
const WATCHED_SOURCES: &[(&str, &str)] = &[
    ("xtask/src/review.rs", "the route"),
    ("xtask/src/review_log.rs", "the log check"),
];

/// The warnings of `review start` for one source, such as
/// `xtask/src/review.rs`, which do not change its exit code.  `what` comes
/// from the branch when the branch changes, adds or removes the source.
pub fn warnings(
    exe: &Path,
    source: &str,
    what: &str,
    exe_time: SystemTime,
    source_time: Option<SystemTime>,
    base_blob: Option<&str>,
    head_blob: Option<&str>,
) -> Vec<String> {
    let old_binary = source_time
        .is_some_and(|source| exe_time < source)
        .then(|| {
            format!(
                "warning: {} is older than {source}; run cargo build -p xtask",
                exe.display()
            )
        });
    let changed = match (base_blob, head_blob) {
        (Some(base), Some(head)) if base != head => Some(format!(
            "warning: this branch changes {source}, so {what} comes from the branch"
        )),
        (None, Some(_)) => Some(format!(
            "warning: this branch adds {source}, so {what} comes from the branch"
        )),
        (Some(_), None) => Some(format!(
            "warning: this branch removes {source}, so {what} comes from the branch"
        )),
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
    /// The number of missed lines that the known-missed list matches.
    pub known: usize,
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
        known: 0,
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
            "mutants: {class}: tested {tested} (caught {}, missed {}, unviable {}), timeouts {}, not tested {not_tested}, known {}",
            c.caught, c.missed, c.unviable, c.timeout, self.known
        )?;
        match &self.reason {
            Some(reason) => write!(f, "; {reason}"),
            None => Ok(()),
        }
    }
}

// Known-missed mutants: the missed mutants that a review round sent to tech
// debt.  `review mutants` logs them as known, and sends the rest to the
// validator.

/// One missed mutant that a review round sent to tech debt.
///
/// An entry matches a missed line by four parts, and each must be equal.
/// Line numbers move with each edit, so an entry has no line number.  The
/// mutation text alone does not tell two operator mutants of one function
/// apart, so the entry also has the code line.  The code line does not tell
/// two mutants on one line apart, so the entry also has the offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Known {
    /// The TD ID of `docs/tech-debt.md` that holds the gap.
    pub debt: &'static str,
    /// The file, as a path from the repo root.
    pub file: &'static str,
    /// The text of the missed line after `<file>:<line>:<col>: `.
    pub mutation: &'static str,
    /// The line of the file at `<line>`, with the spaces at its ends removed.
    pub code: &'static str,
    /// `<col>` minus the spaces and tabs at the start of that line.  It
    /// stays the same when the code line moves or its indent changes.
    pub offset: usize,
}

/// The known-missed list: the missed mutants of TD-39 and TD-66 that the
/// review rounds of PR #39 sent to tech debt.  `cargo mutants --list` still
/// shows each.  Each comment names the round's log and finding.
/// `each_known_missed_entry_is_open_and_in_the_code` checks that each TD is
/// open, and that each code line is once in a function of the mutation's
/// name.  It does not check that the mutant is still at that code line.  An
/// edit can keep the line and move the mutant, such as a new first
/// statement in front of a body mutant.  Then the entry goes stale, and its
/// mutant goes to the validator, so nothing is hidden.
pub const KNOWN_MISSED: &[Known] = &[
    // M8, round 2, finding 2-15.  Since Step 6 of M4 of
    // `docs/plans/completed/m4a.md`, `restore` gives the mark of the log at
    // its reload, so its body has two mutants (TD-39).
    Known {
        debt: "TD-39",
        file: "xtask/src/e2e/save.rs",
        mutation: "replace restore -> Result<usize> with Ok(0)",
        code: "let id = session_id(seconds);",
        offset: 1,
    },
    Known {
        debt: "TD-39",
        file: "xtask/src/e2e/save.rs",
        mutation: "replace restore -> Result<usize> with Ok(1)",
        code: "let id = session_id(seconds);",
        offset: 1,
    },
    // M4 of `docs/plans/completed/m4a.md`, review round 2, finding 2-10:
    // `check_heap` only gives the result of `heap_report` to a `Page`, which
    // only a live WebDriver makes (TD-39).
    Known {
        debt: "TD-39",
        file: "xtask/src/e2e/save.rs",
        mutation: "replace check_heap with ()",
        code: "match heap_report(measured, ok, what) {",
        offset: 1,
    },
    // M8, round 2, finding 2-17.
    Known {
        debt: "TD-39",
        file: "xtask/src/e2e/save.rs",
        mutation: "replace save_once -> Result<Save> with Ok(Default::default())",
        code: "let page = restore(p, seconds)?;",
        offset: 1,
    },
    // M8, round speed 2, finding 2-16.
    Known {
        debt: "TD-39",
        file: "xtask/src/e2e/save.rs",
        mutation: "replace click_save -> Result<String> with Ok(String::new())",
        code: "let mark = p.mark();",
        offset: 1,
    },
    // M8, round speed 2, finding 2-17.
    Known {
        debt: "TD-39",
        file: "xtask/src/e2e/save.rs",
        mutation: "replace click_save -> Result<String> with Ok(\"xyzzy\".into())",
        code: "let mark = p.mark();",
        offset: 1,
    },
    // M8, round speed 2, finding 2-18.
    Known {
        debt: "TD-39",
        file: "xtask/src/e2e/save.rs",
        mutation: "replace save_playing -> Result<Playing> with Ok(Default::default())",
        code: "let seconds = SECONDS[1];",
        offset: 1,
    },
    // M9, round 1, finding 1-21.
    Known {
        debt: "TD-66",
        file: "xtask/src/e2e/weights.rs",
        mutation: "replace note_run -> Result<Vec<String>> with Ok(vec![String::new()])",
        code: "p.note(&format!(\"{label}: replies {:?}\", texts(&run[\"replies\"])));",
        offset: 1,
    },
    // M9, round 1, finding 1-28.
    Known {
        debt: "TD-66",
        file: "xtask/src/e2e/weights.rs",
        mutation: "replace note_run -> Result<Vec<String>> with Ok(vec![])",
        code: "p.note(&format!(\"{label}: replies {:?}\", texts(&run[\"replies\"])));",
        offset: 1,
    },
    // M9, round 1, finding 1-31.
    Known {
        debt: "TD-66",
        file: "xtask/src/e2e/weights.rs",
        mutation: "replace && with || in note_run",
        code: "run[\"replies\"][0][\"ok\"] == json!(true) && run[\"replies\"][1][\"type\"] == json!(\"done\"),",
        offset: 40,
    },
    // M9, round 1, finding 1-37.
    Known {
        debt: "TD-66",
        file: "xtask/src/e2e/weights.rs",
        mutation: "replace || with && in note_run",
        code: "|| line.contains(\"kernels: \")",
        offset: 1,
    },
    // M9, round 1, finding 1-44.
    Known {
        debt: "TD-66",
        file: "xtask/src/e2e/weights.rs",
        mutation: "replace || with && in note_run",
        code: "|| line.contains(\"weights: \")",
        offset: 1,
    },
    // Catch-up, round 1, finding 1-28.
    Known {
        debt: "TD-39",
        file: "app/src/main.rs",
        mutation: "replace main -> eframe::Result<()> with Ok(())",
        code: "let folder = std::env::args().nth(1);",
        offset: 1,
    },
    // Catch-up, round 1, finding 1-30.
    Known {
        debt: "TD-39",
        file: "xtask/src/e2e/decode.rs",
        mutation: "replace drop_and_open -> Result<()> with Ok(())",
        code: "let drop: Vec<(&str, &str, &str)> = files",
        offset: 1,
    },
];

/// The missed lines of a run, split in two.  Each list keeps the order of
/// the input.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Split<'a> {
    /// The lines for the validator.
    pub to_validate: Vec<&'a str>,
    /// Each known line, with the TD of its entry.
    pub known: Vec<(&'a str, &'static str)>,
}

/// Splits the missed lines of cargo-mutants (`missed.txt`) by `list`.
/// `line_at(file, n)` gives line `n` of the file, untrimmed.  A line that
/// does not parse, or whose file `line_at` cannot read, goes to the
/// validator.  So an old entry sends its mutant to the validator.
pub fn split<'a>(
    missed: &'a str,
    list: &[Known],
    line_at: impl Fn(&str, usize) -> Option<String>,
) -> Split<'a> {
    let mut out = Split::default();
    for line in missed.lines().filter(|line| !line.trim().is_empty()) {
        match known_debt(line, list, &line_at) {
            Some(debt) => out.known.push((line, debt)),
            None => out.to_validate.push(line),
        }
    }
    out
}

/// The TD of the entry of `list` that matches `line`, if one does.
fn known_debt(
    line: &str,
    list: &[Known],
    line_at: &impl Fn(&str, usize) -> Option<String>,
) -> Option<&'static str> {
    let (place, mutation) = line.split_once(": ")?;
    let mut parts = place.rsplitn(3, ':');
    let col: usize = parts.next()?.parse().ok()?;
    let number: usize = parts.next()?.parse().ok()?;
    let file = parts.next()?;
    let entries: Vec<&Known> = list
        .iter()
        .filter(|e| e.file == file && e.mutation == mutation)
        .collect();
    if entries.is_empty() {
        return None;
    }
    let code = line_at(file, number)?;
    let indent = code.len() - code.trim_start_matches([' ', '\t']).len();
    let offset = col.checked_sub(indent)?;
    entries
        .iter()
        .find(|e| e.code == code.trim() && e.offset == offset)
        .map(|e| e.debt)
}

/// The function that a mutation names: the text after the last ` in `,
/// else the text after `replace ` up to ` -> ` or ` with `.  It is the last
/// part after `::`, without a `<…>` part.
pub fn mutation_function(mutation: &str) -> &str {
    let path = match mutation.rsplit_once(" in ") {
        Some((_, function)) => function,
        None => {
            let rest = mutation.strip_prefix("replace ").unwrap_or(mutation);
            let end = [" -> ", " with "]
                .iter()
                .filter_map(|stop| rest.find(stop))
                .min()
                .unwrap_or(rest.len());
            &rest[..end]
        }
    };
    let last = path.rsplit("::").next().unwrap_or(path);
    last.split('<').next().unwrap_or(last)
}

/// A Rust identifier, such as `save_once`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name != "_"
}

/// If `line` starts the function `name`: `fn`, a space, the name, then `(`
/// or `<`.  `pub`, `pub(…)`, `async`, `const` or `unsafe` can come first.
fn is_fn_line(line: &str, name: &str) -> bool {
    let mut rest = line.trim_start();
    loop {
        if let Some(inside) = rest.strip_prefix("pub(") {
            match inside.split_once(") ") {
                Some((_, after)) => rest = after,
                None => return false,
            }
        } else if let Some(after) = ["pub ", "async ", "const ", "unsafe "]
            .iter()
            .find_map(|word| rest.strip_prefix(word))
        {
            rest = after;
        } else {
            break;
        }
    }
    rest.strip_prefix("fn ")
        .and_then(|after| after.strip_prefix(name))
        .is_some_and(|after| after.starts_with(['(', '<']))
}

/// The lines of each function `name` in `text`.  A body runs from its `fn`
/// line to the first later line that is `}` at the `fn` line's indent.  A
/// function that ends on its `fn` line has that one line.
fn fn_bodies<'a>(text: &'a str, name: &str) -> Vec<Vec<&'a str>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut bodies = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if !is_fn_line(line, name) {
            continue;
        }
        let end = if line.trim_end().ends_with(['}', ';']) {
            at
        } else {
            let indent = &line[..line.len() - line.trim_start().len()];
            let close = format!("{indent}}}");
            lines[at + 1..]
                .iter()
                .position(|later| later.trim_end() == close)
                .map_or(lines.len() - 1, |n| at + 1 + n)
        };
        bodies.push(lines[at..=end].to_vec());
    }
    bodies
}

/// If `docs/tech-debt.md` (`tech_debt`) has a `### <debt>: ` heading, and
/// no bullet `* <debt>:` under `## Closed`.
fn is_open_debt(tech_debt: &str, debt: &str) -> bool {
    let heading = format!("### {debt}: ");
    let bullet = format!("* {debt}:");
    let mut closed = false;
    let mut in_closed = false;
    for line in tech_debt.lines() {
        if line.starts_with("## ") {
            in_closed = line.trim_end() == "## Closed";
        } else if in_closed && line.starts_with(&bullet) {
            closed = true;
        }
    }
    !closed && tech_debt.lines().any(|line| line.starts_with(&heading))
}

/// The problems of the entries of `list`, so an entry stays true as the code
/// and the tech debt change.  The problems are:
///
/// * a TD that is not open in `tech_debt`;
/// * an entry that is in the list twice;
/// * a file that `read` cannot read;
/// * a mutation whose function name is not a Rust identifier;
/// * a file with no `fn` line of that name;
/// * a code line in no function of that name;
/// * a code line in the functions of that name more than once.
pub fn known_problems(
    list: &[Known],
    tech_debt: &str,
    read: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (at, entry) in list.iter().enumerate() {
        let id = format!("{} {}: {}", entry.debt, entry.file, entry.mutation);
        let mut problem = |text: String| problems.push(format!("{id}: {text}"));
        if !is_open_debt(tech_debt, entry.debt) {
            problem(format!("{} is not an open TD", entry.debt));
        }
        if list[..at].contains(entry) {
            problem("the entry is there twice".to_string());
        }
        let Some(text) = read(entry.file) else {
            problem(format!("cannot read {}", entry.file));
            continue;
        };
        let name = mutation_function(entry.mutation);
        if !is_identifier(name) {
            problem(format!("the function {name:?} is not a Rust identifier"));
            continue;
        }
        let bodies = fn_bodies(&text, name);
        if bodies.is_empty() {
            problem(format!("no `fn {name}` line"));
            continue;
        }
        let count = bodies
            .iter()
            .flatten()
            .filter(|line| line.trim() == entry.code)
            .count();
        match count {
            0 => problem(format!("the code line is in no `fn {name}`")),
            1 => {}
            _ => problem(format!("the code line is in `fn {name}` more than once")),
        }
    }
    problems
}

/// Removes the round's old `mutants.out`, `mutants.out.old`, `mutants.txt`,
/// `missed.txt` and `known.txt`.  A diff with no mutants leaves an old
/// `mutants.out` as it was, and an old line would show until the new run
/// ends.
pub fn clear_old_output(round_folder: &Path) -> Result<()> {
    for old in ["mutants.out", "mutants.out.old"] {
        let path = round_folder.join(old);
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing {}", path.display()))?;
        }
    }
    for old in ["mutants.txt", "missed.txt", "known.txt"] {
        let path = round_folder.join(old);
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
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
pub fn prepare_mutants(toplevel: &Path, round: u32) -> Result<Option<(PathBuf, State)>> {
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
    Ok(Some((round_folder, state)))
}

/// `review mutants <round> [--config <file>]`: runs cargo-mutants on the
/// round's `since` diff, under the limit, and prints one line.  It splits
/// the missed lines by [`KNOWN_MISSED`] into `missed.txt` and `known.txt`
/// of the round folder.
fn mutants(
    toplevel: &Path,
    dir: &Path,
    round: u32,
    config: Option<&str>,
    cargo: &OsStr,
) -> Result<(i32, Vec<String>)> {
    mutants_with(toplevel, dir, round, config, cargo, KNOWN_MISSED)
}

/// [`mutants`], with the known-missed list, so a test can pass its own.
fn mutants_with(
    toplevel: &Path,
    dir: &Path,
    round: u32,
    config: Option<&str>,
    cargo: &OsStr,
    list: &[Known],
) -> Result<(i32, Vec<String>)> {
    let limit = parse_limit(std::env::var("STEMYLA_MUTANTS_MINUTES").ok().as_deref())?;
    let Some((round_folder, state)) = prepare_mutants(toplevel, round)? else {
        return Ok((
            REFUSED,
            vec![format!(
                "no record for this branch and round ({}); run review start first",
                state_path(toplevel, round)?.display()
            )],
        ));
    };
    // cargo-mutants reads the source as it starts, and finds its mutants
    // there: their lines, columns and texts.  `split_missed` reads the code
    // lines at the round's head.  So the list applies only to a source that
    // is the tree at that head as the run starts.
    let at_head = tree_is_at(toplevel, &state.head)?;
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
    let mut line = classify(end, read_counts(&round_folder.join("mutants.out")));
    let list = if at_head { list } else { &[] };
    line.known = split_missed(toplevel, &state.head, &round_folder, list)?;
    if !at_head {
        line.reason = Some(match line.reason {
            Some(reason) => format!("{reason}; {NOT_AT_HEAD}"),
            None => NOT_AT_HEAD.to_string(),
        });
    }
    let line = line.to_string();
    let saved = round_folder.join("mutants.txt");
    std::fs::write(&saved, format!("{line}\n"))
        .with_context(|| format!("writing {}", saved.display()))?;
    Ok((0, vec![line]))
}

/// The reason in the mutants line when the tree was not at the round's head
/// as the run started.
const NOT_AT_HEAD: &str =
    "the tree was not clean at the round's head, so each missed line goes to missed.txt";

/// If `HEAD` is `head`, and the tree has no change, by the check of
/// `review start`.
fn tree_is_at(toplevel: &Path, head: &str) -> Result<bool> {
    let now = git(toplevel, &["rev-parse", "HEAD"])?;
    let dirty = git(
        toplevel,
        &["status", "--porcelain", "--untracked-files=normal"],
    )?;
    Ok(now.trim() == head && dirty.trim().is_empty())
}

/// Splits `mutants.out/missed.txt` of the round, if it exists, and writes
/// `missed.txt` and `known.txt`, empty if need be.  It reads each file at
/// `head`.  `mutants_with` gives it an empty list unless the tree was clean
/// at `head` as cargo-mutants copied it.  Returns the number of known lines.
fn split_missed(toplevel: &Path, head: &str, round_folder: &Path, list: &[Known]) -> Result<usize> {
    let from = round_folder.join("mutants.out/missed.txt");
    let missed = match std::fs::read_to_string(&from) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", from.display())),
    };
    let line_at = |file: &str, n: usize| -> Option<String> {
        let text = git(toplevel, &["show", &format!("{head}:{file}")]).ok()?;
        text.lines().nth(n.checked_sub(1)?).map(str::to_string)
    };
    let parts = split(&missed, list, line_at);
    let to_validate: String = parts
        .to_validate
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    let known: String = parts
        .known
        .iter()
        .map(|(line, debt)| format!("{line}\t{debt}\n"))
        .collect();
    for (name, text) in [("missed.txt", to_validate), ("known.txt", known)] {
        let path = round_folder.join(name);
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(parts.known.len())
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

/// If `name` starts with `<round>-`.  It does not read the agent's name,
/// since reviewers also make worktrees with names of their own, such as
/// `1-correctness-1`.  `10-reviewer-1` is not of round 1, and `run` starts
/// with no digit.
fn of_round(name: &str, round: u32) -> bool {
    name.starts_with(&format!("{round}-"))
}

/// `review clean <round>`: removes each worktree under `target/review/`
/// whose name starts with `<round>-`, with `--force`, since a reviewer's
/// trial changes are scratch by design.  It also removes a folder of such
/// a name that is not a worktree.  It runs no `git worktree prune`, so it
/// acts on no other worktree.
fn clean(toplevel: &Path, round: u32) -> Result<(i32, Vec<String>)> {
    // The same canonical form as the listed paths, also through a linked
    // `target/`.
    let review = crate::clean::canonical(&toplevel.join("target/review"));
    let listing = git(toplevel, &["worktree", "list", "--porcelain", "-z"])?;
    let mut registered: Vec<PathBuf> = listing
        .split('\0')
        .filter_map(|field| field.strip_prefix("worktree "))
        .map(|path| crate::clean::canonical(Path::new(path)))
        .filter(|path| path.parent() == Some(review.as_path()))
        .collect();
    registered.sort();
    let name_of = |path: &Path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    };
    let mut code = 0;
    let mut lines = Vec::new();
    for path in registered.iter().filter(|p| of_round(&name_of(p), round)) {
        let shown = format!("target/review/{}", name_of(path));
        let text = path.to_str().context("a worktree path is UTF-8")?;
        match git(toplevel, &["worktree", "remove", "--force", text]) {
            Ok(_) => lines.push(format!("removed {shown}")),
            Err(e) => {
                code = REFUSED;
                lines.push(format!("failed {shown}: {e:#}"));
            }
        }
    }
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&review)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    folders.sort();
    for path in folders
        .iter()
        .filter(|p| of_round(&name_of(p), round) && !registered.contains(p))
    {
        let shown = format!("target/review/{}", name_of(path));
        match std::fs::remove_dir_all(path) {
            Ok(()) => lines.push(format!("removed {shown}")),
            Err(e) => {
                code = REFUSED;
                lines.push(format!("failed {shown}: {e}"));
            }
        }
    }
    if lines.is_empty() {
        lines.push(format!("no worktrees of round {round}"));
    }
    Ok((code, lines))
}

/// `review log-check <file>`: checks the findings table of a run's log
/// against the fix rule of decision 0024 (`crate::review_log`).  A log
/// with no `Significant` column fails, since the skill writes the column.
fn log_check(path: &Path) -> Result<(i32, Vec<String>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if !crate::review_log::has_significant_column(&text) {
        return Ok((
            REFUSED,
            vec!["log-check: the findings table has no `Significant` column.  Fix: write the log in the form of step 11 of the review-rounds skill.".to_string()],
        ));
    }
    let problems = crate::review_log::fix_rule_problems(&text);
    if problems.is_empty() {
        Ok((
            0,
            vec!["log-check: the log follows the fix rule".to_string()],
        ))
    } else {
        Ok((REFUSED, problems))
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
            ("decoder/src/lib.rs", "memory-reviewer", "decoder/**"),
            ("decoder/clippy.toml", "memory-reviewer", "decoder/**"),
            ("app/src/decoder.rs", "memory-reviewer", "app/src/**"),
            ("decoder/src/lib.rs", "wire-reviewer", "decoder/src/**"),
            ("app/src/decoder.rs", "wire-reviewer", "app/src/decoder.rs"),
            (
                "app/src/decoder/client.rs",
                "wire-reviewer",
                "app/src/decoder/**",
            ),
            ("synth/src/lib.rs", "memory-reviewer", "synth/src/**"),
            ("dsp/src/mix.rs", "memory-reviewer", "dsp/src/**"),
            ("m4a/src/lib.rs", "memory-reviewer", "m4a/src/**"),
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
            (
                "xtask/src/e2e/decode.rs",
                "wire-reviewer",
                "xtask/src/e2e/**",
            ),
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
        let route = route_of_path("xtask/src/metrics.rs");
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
    fn review_log_rs_routes_all_six() {
        assert_routes_all_six("xtask/src/review_log.rs");
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
                "xtask/src/review.rs",
                "the route",
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
                "xtask/src/review.rs",
                "the route",
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
                "xtask/src/review.rs",
                "the route",
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
                "xtask/src/review.rs",
                "the route",
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
                "xtask/src/review.rs",
                "the route",
                now,
                None,
                Some("b1"),
                Some("b1")
            )
            .is_empty()
        );
        assert_eq!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                "xtask/src/review.rs",
                "the route",
                now,
                None,
                None,
                Some("b2")
            ),
            ["warning: this branch adds xtask/src/review.rs, so the route comes from the branch"]
        );
        assert_eq!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                "xtask/src/review.rs",
                "the route",
                now,
                None,
                Some("b1"),
                None
            ),
            [
                "warning: this branch removes xtask/src/review.rs, so the route comes from the branch"
            ]
        );
        assert!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                "xtask/src/review.rs",
                "the route",
                now,
                None,
                None,
                None
            )
            .is_empty()
        );
    }

    #[test]
    fn the_log_check_source_warns_on_its_own() {
        let now = SystemTime::now();
        let earlier = now - Duration::from_secs(60);
        assert_eq!(
            warnings(
                Path::new("/r/target/debug/xtask"),
                "xtask/src/review_log.rs",
                "the log check",
                earlier,
                Some(now),
                Some("b1"),
                Some("b2")
            ),
            [
                "warning: /r/target/debug/xtask is older than xtask/src/review_log.rs; run cargo build -p xtask",
                "warning: this branch changes xtask/src/review_log.rs, so the log check comes from the branch",
            ]
        );
    }

    #[test]
    fn start_warns_when_the_branch_changes_review_log_rs_or_the_binary_is_older() {
        let dir = scratch("start-changed-log-source");
        write(&dir, "xtask/src/review_log.rs", "fn a() {}\n");
        commit_all(&dir, "the log check");
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "xtask/src/review_log.rs", "fn b() {}\n");
        commit_all(&dir, "a new log check");
        let (code, lines) = run(
            &["start".to_string(), "main".to_string(), "1".to_string()],
            &dir,
            &dir.join("target/no-home"),
            SystemTime::UNIX_EPOCH,
        );
        assert_eq!(code, 0, "{lines:?}");
        let warnings: Vec<String> = lines
            .iter()
            .filter(|l| l.starts_with("warning"))
            .map(|l| {
                l.replace(
                    &std::env::current_exe().unwrap().display().to_string(),
                    "<exe>",
                )
            })
            .collect();
        assert_eq!(
            warnings,
            [
                "warning: <exe> is older than xtask/src/review_log.rs; run cargo build -p xtask",
                "warning: this branch changes xtask/src/review_log.rs, so the log check comes from the branch",
            ]
        );
    }

    #[test]
    fn start_warns_when_the_branch_adds_review_log_rs() {
        let dir = scratch("start-added-log-source");
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "xtask/src/review_log.rs", "fn a() {}\n");
        commit_all(&dir, "a log check");
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
                "warning: this branch adds xtask/src/review_log.rs, so the log check comes from the branch"
            ]
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

    const COUNTED: &str =
        "tested 9 (caught 3, missed 2, unviable 4), timeouts 1, not tested 5, known 0";
    const NOTHING: &str =
        "tested 0 (caught 0, missed 0, unviable 0), timeouts 0, not tested 0, known 0";

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
            "mutants: partial: tested 2 (caught 0, missed 0, unviable 2), timeouts 0, not tested 3, known 0; stopped at the limit",
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
            "mutants: none: tested 0 (caught 0, missed 0, unviable 0), timeouts 2, not tested 5, known 0; stopped before a mutant finished",
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
            "mutants: full: tested 1 (caught 1, missed 0, unviable 0), timeouts 0, not tested 0, known 0",
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
        let old_missed = round_folder.join("missed.txt");
        std::fs::write(&old_missed, format!("{LINE_1_41}\n")).unwrap();
        let old_known = round_folder.join("known.txt");
        std::fs::write(&old_known, format!("{LINE_2_17}\tTD-39\n")).unwrap();
        clear_old_output(&round_folder).unwrap();
        assert!(
            !old_line.exists(),
            "a round started again must not show the line of the last run"
        );
        assert!(!old_missed.exists(), "nor the missed lines of the last run");
        assert!(!old_known.exists(), "nor the known lines of the last run");
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
        let (round_folder, _) = prepare_mutants(&top, 2).unwrap().unwrap();
        let diff = std::fs::read_to_string(round_folder.join("mutants.diff")).unwrap();
        assert!(diff.contains("+++ b/b.rs"), "{diff}");
        assert!(!diff.contains("a.rs"), "{diff}");
        assert_eq!(prepare_mutants(&top, 3).unwrap(), None);
    }

    /// A fake `cargo` in POSIX sh.  `mutants --version` passes.  A run
    /// writes counts to `<output>/mutants.out`, keeps the `--config` that
    /// it got in `<output>/config-seen.txt` and the `--in-diff` in
    /// `<output>/in-diff-seen.txt`, and exits 2 (missed mutants).  If its
    /// folder holds `missed.txt`, the run copies it to `mutants.out`.  If its
    /// folder holds `during.sh`, the run runs it at the toplevel.
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
here=$(dirname "$0")
if [ -f "$here/missed.txt" ]; then cp "$here/missed.txt" "$out/mutants.out/missed.txt"; fi
if [ -f "$here/during.sh" ]; then sh "$here/during.sh"; fi
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
        assert_lists(&round_folder, "", "");
    }

    /// `missed.txt` and `known.txt` of the round hold `missed` and `known`.
    fn assert_lists(round_folder: &Path, missed: &str, known: &str) {
        let read = |name: &str| std::fs::read_to_string(round_folder.join(name)).ok();
        assert_eq!(read("missed.txt").as_deref(), Some(missed), "missed.txt");
        assert_eq!(read("known.txt").as_deref(), Some(known), "known.txt");
    }

    /// A round 1 of a change to `SAVE_RS`, whose lines 427 and 503 hold the
    /// code of 1-41 and of 2-17, and a fake `cargo` that misses both.
    /// Returns the repo, the text of `SAVE_RS`, the fake's folder and the
    /// fake.
    fn round_1_that_misses_2_17(name: &str) -> (Scratch, String, Scratch, PathBuf) {
        let dir = scratch(name);
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        let save: String = (1..=503)
            .map(|n| match n {
                427 => format!("{CODE_427}\n"),
                503 => format!("{CODE_503}\n"),
                _ => format!("// line {n}\n"),
            })
            .collect();
        write(&dir, SAVE_RS, &save);
        commit_all(&dir, "round 1");
        assert_eq!(start_in(&dir, "main", "1").0, 0);
        let bin = Scratch::new(&format!("{name}-bin"));
        let cargo = fake_cargo(&bin, 0);
        std::fs::write(
            bin.join("missed.txt"),
            format!("{LINE_1_41}\n{LINE_2_17}\n"),
        )
        .unwrap();
        (dir, save, bin, cargo)
    }

    #[test]
    fn mutants_splits_the_missed_lines_by_the_list() {
        let (dir, save, bin, cargo) = round_1_that_misses_2_17("mutants-split");
        // As cargo-mutants runs, line 503 changes in a new commit, and again
        // in the tree.  The code line still comes from the round's head.
        std::fs::write(
            bin.join("later.rs"),
            save.replace(CODE_503, "    committed_later();"),
        )
        .unwrap();
        std::fs::write(
            bin.join("tree.rs"),
            save.replace(CODE_503, "    changed_in_the_tree();"),
        )
        .unwrap();
        std::fs::write(
            bin.join("during.sh"),
            format!(
                "here=$(dirname \"$0\")\ncp \"$here/later.rs\" {SAVE_RS}\ngit -c commit.gpgsign=false -c user.name=t -c user.email=t@example.com commit -q --no-verify -am later\ncp \"$here/tree.rs\" {SAVE_RS}\n"
            ),
        )
        .unwrap();
        let head = sha(&dir, "HEAD");
        let top = toplevel(&dir).unwrap();
        let (code, lines) =
            mutants_with(&top, &dir, 1, None, cargo.as_os_str(), &[ENTRY_2_17]).unwrap();
        assert_ne!(sha(&dir, "HEAD"), head, "the fake made a commit");
        let expected = "mutants: full: tested 9 (caught 3, missed 2, unviable 4), timeouts 1, not tested 5, known 1";
        assert_eq!((code, lines), (0, vec![expected.to_string()]));
        let round_folder = top.join("target/review/run/feature/1");
        assert_eq!(
            std::fs::read_to_string(round_folder.join("mutants.txt")).unwrap(),
            format!("{expected}\n")
        );
        assert_lists(
            &round_folder,
            &format!("{LINE_1_41}\n"),
            &format!("{LINE_2_17}\tTD-39\n"),
        );
        assert_eq!(
            std::fs::read_to_string(round_folder.join("mutants.out/missed.txt")).unwrap(),
            format!("{LINE_1_41}\n{LINE_2_17}\n"),
            "mutants.out stays as cargo-mutants wrote it"
        );
    }

    /// Runs the round of [`round_1_that_misses_2_17`] after `change`, which
    /// moves the tree off the round's head before cargo-mutants starts.
    /// Each missed line goes to the validator, and the line says why.
    fn assert_a_moved_tree_knows_no_line(name: &str, change: impl Fn(&Path, &str)) {
        let (dir, save, _bin, cargo) = round_1_that_misses_2_17(name);
        change(&dir, &save);
        let top = toplevel(&dir).unwrap();
        let (code, lines) =
            mutants_with(&top, &dir, 1, None, cargo.as_os_str(), &[ENTRY_2_17]).unwrap();
        let expected = format!(
            "mutants: full: tested 9 (caught 3, missed 2, unviable 4), timeouts 1, not tested 5, known 0; {NOT_AT_HEAD}"
        );
        assert_eq!((code, lines), (0, vec![expected]));
        assert_lists(
            &top.join("target/review/run/feature/1"),
            &format!("{LINE_1_41}\n{LINE_2_17}\n"),
            "",
        );
    }

    #[test]
    fn a_change_in_the_tree_before_the_run_knows_no_line() {
        // The code of 2-17 stays at line 503; another line changes.
        assert_a_moved_tree_knows_no_line("mutants-dirty", |dir, save| {
            write(dir, SAVE_RS, &save.replace("// line 1\n", "// one\n"));
        });
    }

    #[test]
    fn a_commit_before_the_run_knows_no_line() {
        assert_a_moved_tree_knows_no_line("mutants-moved", |dir, save| {
            write(dir, SAVE_RS, &save.replace("// line 1\n", "// one\n"));
            commit_all(dir, "a fix after review start");
        });
    }

    #[test]
    fn an_untracked_file_before_the_run_knows_no_line() {
        assert_a_moved_tree_knows_no_line("mutants-untracked", |dir, _| {
            write(dir, "notes.txt", "not ignored\n");
        });
    }

    #[test]
    fn a_reason_and_a_moved_tree_are_both_in_the_line() {
        let (dir, save, bin, _) = round_1_that_misses_2_17("mutants-moved-partial");
        // The fake exits 3 (timeouts), so the line has its own reason.
        let cargo = fake_cargo(&bin, 0);
        let script = std::fs::read_to_string(&cargo)
            .unwrap()
            .replace("\nexit 2\n", "\nexit 3\n");
        std::fs::write(&cargo, script).unwrap();
        write(&dir, SAVE_RS, &save.replace("// line 1\n", "// one\n"));
        let top = toplevel(&dir).unwrap();
        let (_, lines) =
            mutants_with(&top, &dir, 1, None, cargo.as_os_str(), &[ENTRY_2_17]).unwrap();
        assert_eq!(
            lines,
            [format!(
                "mutants: partial: tested 9 (caught 3, missed 2, unviable 4), timeouts 1, not tested 5, known 0; timeouts have no result; {NOT_AT_HEAD}"
            )]
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
        assert_lists(&round_folder, "", "");
    }

    #[test]
    fn prepare_mutants_removes_the_old_output() {
        let dir = round_1_of_a_rust_change("mutants-clear");
        let top = toplevel(&dir).unwrap();
        let round_folder = top.join("target/review/run/feature/1");
        std::fs::create_dir_all(round_folder.join("mutants.out")).unwrap();
        std::fs::create_dir_all(round_folder.join("mutants.out.old")).unwrap();
        let (folder, state) = prepare_mutants(&top, 1).unwrap().unwrap();
        assert_eq!(folder, round_folder);
        assert_eq!(state.head, sha(&dir, "HEAD"), "the round's state");
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

    // Known-missed mutants.

    const SAVE_RS: &str = "xtask/src/e2e/save.rs";

    /// Finding 2-17 of round 2 of M8, which went to TD-39.
    const LINE_2_17: &str = "xtask/src/e2e/save.rs:503:5: replace save_once -> Result<Save> with Ok(Default::default())";

    /// Findings 1-41 and 1-54 of round 1 of M8, which `2fdecda` fixed with
    /// tests.
    const LINE_1_41: &str = "xtask/src/e2e/save.rs:427:39: replace * with + in save_once";
    const LINE_1_54: &str = "xtask/src/e2e/save.rs:427:39: replace * with / in save_once";

    /// The code of `save.rs:503` at `9c69f22`, and of `save.rs:427` at
    /// `f28d22d`.
    const CODE_503: &str = "    restore(p, seconds)?;";
    const CODE_427: &str = "    let item_y = o.item_y + o.item_dy * row as f64;";

    /// A fixture entry for the mutant of 2-17.
    const ENTRY_2_17: Known = Known {
        debt: "TD-39",
        file: SAVE_RS,
        mutation: "replace save_once -> Result<Save> with Ok(Default::default())",
        code: "restore(p, seconds)?;",
        offset: 1,
    };

    /// A fake `line_at` that knows the lines of `lines` only.
    fn lines_of(
        lines: &'static [(&'static str, usize, &'static str)],
    ) -> impl Fn(&str, usize) -> Option<String> {
        move |file, n| {
            lines
                .iter()
                .find(|(f, line, _)| *f == file && *line == n)
                .map(|(_, _, code)| code.to_string())
        }
    }

    const SAVE_LINES: &[(&str, usize, &str)] =
        &[(SAVE_RS, 503, CODE_503), (SAVE_RS, 427, CODE_427)];

    /// The first line of the body of the function whose line starts with
    /// `signature` in `text`.  cargo-mutants puts the mutant of a body on
    /// it.
    fn first_body_line(text: &str, signature: &str) -> Option<String> {
        let mut lines = text.lines().skip_while(|l| !l.starts_with(signature));
        lines.by_ref().find(|l| l.trim_end().ends_with('{'))?;
        lines.find(|l| !l.trim().is_empty()).map(String::from)
    }

    #[test]
    fn the_mutant_of_2_17_is_known() {
        let missed = format!("{LINE_2_17}\n");
        assert_eq!(
            split(&missed, &[ENTRY_2_17], lines_of(SAVE_LINES)),
            Split {
                to_validate: vec![],
                known: vec![(LINE_2_17, "TD-39")],
            }
        );
    }

    #[test]
    fn the_mutants_of_1_41_and_1_54_go_to_the_validator() {
        let missed = format!("{LINE_1_41}\n{LINE_1_54}\n");
        assert_eq!(
            split(&missed, &[ENTRY_2_17], lines_of(SAVE_LINES)),
            Split {
                to_validate: vec![LINE_1_41, LINE_1_54],
                known: vec![],
            }
        );
    }

    #[test]
    fn another_code_line_or_another_file_goes_to_the_validator() {
        let moved = "xtask/src/e2e/save.rs:427:5: replace save_once -> Result<Save> with Ok(Default::default())";
        let elsewhere = "xtask/src/e2e/load.rs:503:5: replace save_once -> Result<Save> with Ok(Default::default())";
        const LINES: &[(&str, usize, &str)] = &[
            (SAVE_RS, 427, CODE_427),
            ("xtask/src/e2e/load.rs", 503, CODE_503),
        ];
        let missed = format!("{moved}\n{elsewhere}\n");
        assert_eq!(
            split(&missed, &[ENTRY_2_17], lines_of(LINES)),
            Split {
                to_validate: vec![moved, elsewhere],
                known: vec![],
            }
        );
    }

    #[test]
    fn the_offset_tells_two_mutants_on_one_line_apart() {
        // `xtask/src/e2e/weights.rs:498` at `759abfa`.
        const LINES: &[(&str, usize, &str)] = &[(
            "xtask/src/e2e/weights.rs",
            498,
            "        k16.as_ref().is_some_and(|(m, a, _)| *m == mm && *a == attn),",
        )];
        let first = "xtask/src/e2e/weights.rs:498:49: replace == with != in run";
        let second = "xtask/src/e2e/weights.rs:498:61: replace == with != in run";
        let entry = Known {
            debt: "TD-66",
            file: "xtask/src/e2e/weights.rs",
            mutation: "replace == with != in run",
            code: "k16.as_ref().is_some_and(|(m, a, _)| *m == mm && *a == attn),",
            offset: 53,
        };
        let missed = format!("{first}\n{second}\n");
        assert_eq!(
            split(&missed, &[entry], lines_of(LINES)),
            Split {
                to_validate: vec![first],
                known: vec![(second, "TD-66")],
            }
        );
    }

    #[test]
    fn an_unread_file_or_a_line_without_a_place_goes_to_the_validator() {
        let no_place =
            "xtask/src/e2e/save.rs: replace save_once -> Result<Save> with Ok(Default::default())";
        let missed = format!("{LINE_2_17}\n{no_place}\n");
        assert_eq!(
            split(&missed, &[ENTRY_2_17], |_: &str, _| None),
            Split {
                to_validate: vec![LINE_2_17, no_place],
                known: vec![],
            }
        );
    }

    #[test]
    fn split_reads_no_file_for_a_line_that_no_entry_names() {
        let missed = format!("{LINE_1_41}\n");
        let read = |file: &str, n: usize| -> Option<String> {
            panic!("read {file}:{n}, which no entry names")
        };
        assert_eq!(
            split(&missed, &[ENTRY_2_17], read),
            Split {
                to_validate: vec![LINE_1_41],
                known: vec![],
            }
        );
    }

    #[test]
    fn split_keeps_the_order_and_drops_empty_lines() {
        let missed = format!("{LINE_1_41}\n\n{LINE_2_17}\n\n{LINE_1_54}\n");
        assert_eq!(
            split(&missed, &[ENTRY_2_17], lines_of(SAVE_LINES)),
            Split {
                to_validate: vec![LINE_1_41, LINE_1_54],
                known: vec![(LINE_2_17, "TD-39")],
            }
        );
    }

    // The problems of known-missed entries.

    /// Tech debt with TD-39 open and TD-5 closed.
    const TECH_DEBT: &str = "# Tech debt\n\n## Enforcement gaps\n\n### TD-39: e2e has no cargo test coverage\n\nText.\n\n## Closed\n\n* TD-5: closed by a test.\n";

    /// A file with `save_once`, and `restore(p, seconds)?;` in two
    /// functions.
    const SAVE_FILE: &str = "\
fn restore(p: &mut Page, seconds: u32) -> Result<()> {
    let id = session_id(seconds);
    Ok(())
}

fn save_once(p: &mut Page, o: &Opts, format: &str, row: usize, seconds: u32) -> Result<Save> {
    restore(p, seconds)?;
    p.idle(3.0)?;
}

pub(super) fn save_playing(p: &mut Page, o: &Opts) -> Result<Playing> {
    restore(p, seconds)?;
}
";

    /// A reader that knows `SAVE_RS` only, with `text`.
    fn reader(text: &'static str) -> impl Fn(&str) -> Option<String> {
        move |file| (file == SAVE_RS).then(|| text.to_string())
    }

    /// The problems of `entry`, with [`TECH_DEBT`] and [`SAVE_FILE`].
    fn problems_of(entry: Known) -> Vec<String> {
        known_problems(&[entry], TECH_DEBT, reader(SAVE_FILE))
    }

    #[test]
    fn a_good_entry_has_no_problem() {
        // `save_playing` has `pub(super)` before its `fn`, and the same code
        // line as `save_once`.
        let playing = Known {
            mutation: "replace save_playing -> Result<Playing> with Ok(Default::default())",
            ..ENTRY_2_17
        };
        assert_eq!(
            known_problems(&[ENTRY_2_17, playing], TECH_DEBT, reader(SAVE_FILE)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_missing_or_closed_debt_is_a_problem() {
        let missing = Known {
            debt: "TD-99",
            ..ENTRY_2_17
        };
        let closed = Known {
            debt: "TD-5",
            ..ENTRY_2_17
        };
        assert_eq!(
            known_problems(&[missing, closed], TECH_DEBT, reader(SAVE_FILE)),
            [
                "TD-99 xtask/src/e2e/save.rs: replace save_once -> Result<Save> with Ok(Default::default()): TD-99 is not an open TD",
                "TD-5 xtask/src/e2e/save.rs: replace save_once -> Result<Save> with Ok(Default::default()): TD-5 is not an open TD",
            ]
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_is_a_problem() {
        let entry = Known {
            file: "xtask/src/e2e/gone.rs",
            ..ENTRY_2_17
        };
        assert_eq!(
            problems_of(entry),
            [
                "TD-39 xtask/src/e2e/gone.rs: replace save_once -> Result<Save> with Ok(Default::default()): cannot read xtask/src/e2e/gone.rs"
            ]
        );
    }

    #[test]
    fn a_function_that_is_not_an_identifier_is_a_problem() {
        // A mutant outside a function, such as one of the constant
        // `WAITS_S` at `2c7c29b:xtask/src/e2e/save.rs:156`.
        let entry = Known {
            mutation: "replace * with /",
            ..ENTRY_2_17
        };
        assert_eq!(
            problems_of(entry),
            [
                "TD-39 xtask/src/e2e/save.rs: replace * with /: the function \"*\" is not a Rust identifier"
            ]
        );
    }

    #[test]
    fn a_function_that_is_not_in_the_file_is_a_problem() {
        let entry = Known {
            mutation: "replace restore_all -> Result<()> with Ok(())",
            ..ENTRY_2_17
        };
        assert_eq!(
            problems_of(entry),
            [
                "TD-39 xtask/src/e2e/save.rs: replace restore_all -> Result<()> with Ok(()): no `fn restore_all` line"
            ]
        );
    }

    #[test]
    fn a_longer_function_name_is_not_the_function() {
        let file = "fn restore_check(p: &mut Page) -> Result<()> {\n    let id = session_id(seconds);\n}\n";
        let entry = Known {
            mutation: "replace restore -> Result<()> with Ok(())",
            code: "let id = session_id(seconds);",
            ..ENTRY_2_17
        };
        assert_eq!(
            known_problems(&[entry], TECH_DEBT, reader(file)),
            [
                "TD-39 xtask/src/e2e/save.rs: replace restore -> Result<()> with Ok(()): no `fn restore` line"
            ]
        );
    }

    #[test]
    fn a_code_line_in_no_function_of_the_name_is_a_problem() {
        // `restore(p, seconds)?;` is in `save_once` and `save_playing`, and
        // the entry names `restore`.
        let entry = Known {
            mutation: "replace restore -> Result<()> with Ok(())",
            ..ENTRY_2_17
        };
        assert_eq!(
            problems_of(entry),
            [
                "TD-39 xtask/src/e2e/save.rs: replace restore -> Result<()> with Ok(()): the code line is in no `fn restore`"
            ]
        );
    }

    #[test]
    fn a_code_line_twice_in_the_function_is_a_problem() {
        let file = "fn save_once() -> Result<Save> {\n    restore(p, seconds)?;\n    if again {\n        restore(p, seconds)?;\n    }\n}\n";
        assert_eq!(
            known_problems(&[ENTRY_2_17], TECH_DEBT, reader(file)),
            [
                "TD-39 xtask/src/e2e/save.rs: replace save_once -> Result<Save> with Ok(Default::default()): the code line is in `fn save_once` more than once"
            ]
        );
    }

    #[test]
    fn a_code_line_twice_in_two_functions_of_the_name_is_a_problem() {
        let file = "impl A {\n    pub fn save_once() {\n        restore(p, seconds)?;\n    }\n}\n\nimpl B {\n    async fn save_once() {\n        restore(p, seconds)?;\n    }\n}\n";
        assert_eq!(
            known_problems(&[ENTRY_2_17], TECH_DEBT, reader(file)),
            [
                "TD-39 xtask/src/e2e/save.rs: replace save_once -> Result<Save> with Ok(Default::default()): the code line is in `fn save_once` more than once"
            ]
        );
    }

    #[test]
    fn the_body_ends_at_the_brace_of_the_fn_lines_indent() {
        // `save_once` ends on its `fn` line, and the line after it is in
        // `other`.  In `impl A`, `run` ends at its own brace, not at the
        // brace of its `if`.
        let file = "fn save_once() -> Result<Save> { Ok(Save::default()) }\nfn other() {\n    restore(p, seconds)?;\n}\nimpl A {\n    fn run() {\n        if ready {\n            go();\n        }\n        start();\n    }\n    fn after() {\n        stop();\n    }\n}\n";
        let one_line = known_problems(&[ENTRY_2_17], TECH_DEBT, reader(file));
        let run = Known {
            mutation: "replace == with != in run",
            code: "stop();",
            ..ENTRY_2_17
        };
        let in_run = Known {
            mutation: "replace == with != in run",
            code: "start();",
            ..ENTRY_2_17
        };
        assert_eq!(
            (
                one_line,
                known_problems(&[run, in_run], TECH_DEBT, reader(file))
            ),
            (
                vec![
                    "TD-39 xtask/src/e2e/save.rs: replace save_once -> Result<Save> with Ok(Default::default()): the code line is in no `fn save_once`".to_string()
                ],
                vec![
                    "TD-39 xtask/src/e2e/save.rs: replace == with != in run: the code line is in no `fn run`".to_string()
                ]
            )
        );
    }

    #[test]
    fn a_body_without_its_closing_brace_runs_to_the_end_of_the_file() {
        let file = "fn save_once() {\n    restore(p, seconds)?;\n";
        assert_eq!(
            known_problems(&[ENTRY_2_17], TECH_DEBT, reader(file)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_body_holds_its_fn_line_to_its_closing_brace() {
        let file = "fn save_once() {\n    if ready {\n        go();\n    }\n    restore(p, seconds)?;\n}\nfn other() {}\n";
        assert_eq!(
            fn_bodies(file, "save_once"),
            [[
                "fn save_once() {",
                "    if ready {",
                "        go();",
                "    }",
                "    restore(p, seconds)?;",
                "}",
            ]]
        );
        assert_eq!(
            fn_bodies("fn save_once() {\n    go();\n", "save_once"),
            [["fn save_once() {", "    go();"]]
        );
    }

    #[test]
    fn a_debt_bullet_outside_closed_keeps_the_debt_open() {
        let tech_debt = "## Enforcement gaps\n\n### TD-39: e2e\n\n* TD-39: a bullet that names it.\n\n## Known breaches\n\n* TD-39: another bullet.\n\n## Closed\n\n* TD-5: closed.\n";
        assert!(is_open_debt(tech_debt, "TD-39"));
    }

    #[test]
    fn a_debt_with_a_heading_and_a_closed_bullet_is_not_open() {
        let tech_debt =
            "## Enforcement gaps\n\n### TD-39: e2e\n\n## Closed\n\n* TD-39: closed by a test.\n";
        assert!(!is_open_debt(tech_debt, "TD-39"));
        assert_eq!(
            known_problems(&[ENTRY_2_17], tech_debt, reader(SAVE_FILE)),
            [
                "TD-39 xtask/src/e2e/save.rs: replace save_once -> Result<Save> with Ok(Default::default()): TD-39 is not an open TD"
            ]
        );
    }

    #[test]
    fn the_fn_line_forms() {
        let starts: Vec<bool> = [
            "fn save_once(p: &mut Page) {",
            "    pub fn save_once<T>(p: T) {",
            "pub(crate) fn save_once() {",
            "pub(in crate::e2e) async fn save_once() {",
            "const fn save_once() -> u8 {",
            "pub const unsafe fn save_once() {",
            "unsafe fn save_once() {",
            "fn save_once_all() {",
            "// fn save_once() {",
            "pub(crate fn save_once() {",
            "pub(crate fn save_once(){",
            "let save_once = 1;",
        ]
        .iter()
        .map(|line| is_fn_line(line, "save_once"))
        .collect();
        assert_eq!(
            starts,
            [
                true, true, true, true, true, true, true, false, false, false, false, false
            ]
        );
    }

    #[test]
    fn a_fn_line_that_ends_with_a_semicolon_is_its_own_body() {
        let file = "trait T {\n    fn save_once(&self) -> Result<Save>;\n    fn other(&self) {\n        restore(p, seconds)?;\n    }\n}\n";
        assert_eq!(
            fn_bodies(file, "save_once"),
            [["    fn save_once(&self) -> Result<Save>;"]]
        );
    }

    #[test]
    fn an_underscore_is_not_a_function() {
        let entry = Known {
            mutation: "replace _ with ()",
            ..ENTRY_2_17
        };
        assert_eq!(
            problems_of(entry),
            [
                "TD-39 xtask/src/e2e/save.rs: replace _ with (): the function \"_\" is not a Rust identifier"
            ]
        );
    }

    #[test]
    fn a_tab_counts_in_the_indent_of_the_offset() {
        const LINES: &[(&str, usize, &str)] = &[(SAVE_RS, 503, "\t\trestore(p, seconds)?;")];
        let line = "xtask/src/e2e/save.rs:503:3: replace save_once -> Result<Save> with Ok(Default::default())";
        let missed = format!("{line}\n");
        assert_eq!(
            split(&missed, &[ENTRY_2_17], lines_of(LINES)),
            Split {
                to_validate: vec![],
                known: vec![(line, "TD-39")],
            }
        );
    }

    #[test]
    fn a_missed_file_that_cannot_be_read_is_an_error() {
        let dir = Scratch::new("split-unreadable");
        std::fs::create_dir_all(dir.join("mutants.out/missed.txt")).unwrap();
        let result = split_missed(&dir, "HEAD", &dir, &[ENTRY_2_17]);
        assert!(result.is_err(), "{result:?}");
        assert!(!dir.join("missed.txt").exists(), "no lists after an error");
        assert!(!dir.join("known.txt").exists(), "no lists after an error");
    }

    #[test]
    fn the_function_of_a_mutation() {
        let names: Vec<&str> = [
            "replace StemApp::can_encode -> bool with true",
            "replace == with != in run",
            "replace save_once -> Result<Save> with Ok(Default::default())",
            "replace * with + in <impl Display for Line>::fmt",
            "replace Wrap<T>::get<U> with ()",
            "replace main with ()",
            "replace * with /",
        ]
        .iter()
        .map(|m| mutation_function(m))
        .collect();
        assert_eq!(
            names,
            ["can_encode", "run", "save_once", "fmt", "get", "main", "*"]
        );
    }

    #[test]
    fn the_same_entry_twice_is_a_problem() {
        assert_eq!(
            known_problems(&[ENTRY_2_17, ENTRY_2_17], TECH_DEBT, reader(SAVE_FILE)),
            [
                "TD-39 xtask/src/e2e/save.rs: replace save_once -> Result<Save> with Ok(Default::default()): the entry is there twice"
            ]
        );
    }

    // The real known-missed list.

    #[test]
    fn each_known_missed_entry_is_open_and_in_the_code() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let tech_debt = std::fs::read_to_string(root.join("docs/tech-debt.md")).unwrap();
        let read = |file: &str| std::fs::read_to_string(root.join(file)).ok();
        assert_eq!(
            known_problems(KNOWN_MISSED, &tech_debt, read),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_real_list_knows_2_17_and_not_1_41_or_1_54() {
        let missed = format!("{LINE_1_41}\n{LINE_2_17}\n{LINE_1_54}\n");
        // Finding 1-3 of review round 1 of M4: the line of the body mutant is
        // the first statement of `save_once` in the real file, so a new
        // first statement shows as a drift.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let real = std::fs::read_to_string(root.join(SAVE_RS)).expect("save.rs");
        let first = first_body_line(&real, "fn save_once(");
        assert!(first.is_some(), "save_once has a body");
        let line_at = move |file: &str, n: usize| match (file, n) {
            (SAVE_RS, 503) => first.clone(),
            (SAVE_RS, 427) => Some(CODE_427.to_string()),
            _ => None,
        };
        assert_eq!(
            split(&missed, KNOWN_MISSED, line_at),
            Split {
                to_validate: vec![LINE_1_41, LINE_1_54],
                known: vec![(LINE_2_17, "TD-39")],
            }
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

    /// Runs `review log-check` on `text`, written to a file in a scratch
    /// folder; or on a missing file if `text` is `None`.
    fn log_check_of(name: &str, text: Option<&str>) -> (i32, Vec<String>) {
        let dir = Scratch::new(&format!("log-check-{name}"));
        if let Some(text) = text {
            std::fs::write(dir.join("log.md"), text).unwrap();
        }
        run(
            &["log-check".to_string(), "log.md".to_string()],
            &dir,
            &dir,
            SystemTime::UNIX_EPOCH,
        )
    }

    /// A log with a `Significant` column and one nit in round 4.
    fn log_with_nit(action: &str) -> String {
        format!(
            "# Review log\n\n## Findings\n\n| Round | Finding | Reviewers | Severity | Significant | Rule | Verdict | Repeat of | Action |\n| - | - | - | - | - | - | - | - | - |\n| 4 | 4-1 | docs-reviewer | nit | - | none | unvalidated | none | {action} |\n"
        )
    }

    #[test]
    fn log_check_passes_a_log_that_follows_the_fix_rule() {
        assert_eq!(
            log_check_of("good", Some(&log_with_nit("reported, not fixed"))),
            (
                0,
                vec!["log-check: the log follows the fix rule".to_string()]
            )
        );
    }

    #[test]
    fn log_check_fails_a_fixed_nit_in_round_4() {
        assert_eq!(
            log_check_of("nit", Some(&log_with_nit("abc1234 review round 4 fixes"))),
            (
                3,
                vec!["4-1: round 4 must not fix this finding, but the action is `abc1234 review round 4 fixes`.  Fix: undo the fix, and write `reported, not fixed` (decision 0024).".to_string()]
            )
        );
    }

    #[test]
    fn log_check_fails_a_log_with_no_significant_column() {
        let old = "# Review log\n\n## Findings\n\n| Round | Finding | Action |\n| - | - | - |\n| 4 | 4-1 | abc1234 review round 4 fixes |\n";
        assert_eq!(
            log_check_of("old", Some(old)),
            (
                3,
                vec!["log-check: the findings table has no `Significant` column.  Fix: write the log in the form of step 11 of the review-rounds skill.".to_string()]
            )
        );
    }

    #[test]
    fn log_check_fails_a_missing_file() {
        let (code, lines) = log_check_of("missing", None);
        assert_eq!(code, 3);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("reading ") && lines[0].contains("log.md"),
            "{lines:?}"
        );
    }
}
