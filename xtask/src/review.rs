//! `xtask review`: chooses the reviewers from a diff, guards a review round,
//! and runs cargo-mutants on a round's diff.
//!
//! Each failure exits 3, so a skill that runs the command stops on it.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

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
pub fn run(
    _args: &[String],
    _dir: &Path,
    _home: &Path,
    _exe_time: SystemTime,
) -> (i32, Vec<String>) {
    (REFUSED, vec![USAGE.to_string()])
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

    /// A scratch repo on `main` with one commit, which ignores `target/`
    /// as the real repo does.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-review-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git_in(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        commit_all(&dir, "start");
        dir
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
            ("dsp/clippy.toml", "memory-reviewer", "**/clippy.toml"),
            (
                ".claude/rules/memory.md",
                "memory-reviewer",
                ".claude/rules/memory.md",
            ),
            ("protocol/src/lib.rs", "wire-reviewer", "protocol/**"),
            ("worker/src/web.rs", "wire-reviewer", "worker/src/web.rs"),
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
    fn an_empty_diff_gives_an_empty_route() {
        assert!(route("", "").is_empty());
    }

    // The route: real git output.

    #[test]
    fn a_rename_routes_by_both_paths() {
        let dir = scratch("rename");
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
