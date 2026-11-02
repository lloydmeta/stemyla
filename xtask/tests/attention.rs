//! `xtask attention` as a binary, on a temporary repo of two commits.  No
//! test runs on this repo.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use xtask::attention::{HAND_RUN, MESSAGES};

/// A committer identity, and no user or system git config, for each git
/// command that the tests and the binary run.
const GIT_ENV: [(&str, &str); 6] = [
    ("GIT_AUTHOR_NAME", "t"),
    ("GIT_AUTHOR_EMAIL", "t@example.com"),
    ("GIT_COMMITTER_NAME", "t"),
    ("GIT_COMMITTER_EMAIL", "t@example.com"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .envs(GIT_ENV)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Commits every change in `dir`, and gives the new commit.
fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "--no-verify", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A repo in the temporary folder, which goes when the test ends.
struct Repo(PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("attention")
        .args(args)
        .current_dir(dir)
        .envs(GIT_ENV)
        .output()
        .unwrap()
}

const PLAN: &str = "# P\n\n## Decision log\n\n* 2026-11-01, maintainer: start.\n";

#[test]
fn the_hand_run_line_comes_first_and_a_docs_line_is_none() {
    let top = std::env::temp_dir().join(format!("xtask-attention-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(&top).unwrap();
    let repo = Repo(top.canonicalize().unwrap());
    let dir = &repo.0;
    git(dir, &["init", "-q", "-b", "main"]);
    write(&dir.join("protocol/src/lib.rs"), "pub struct A;\n");
    write(&dir.join("docs/README.md"), "# Docs\n");
    write(&dir.join("gone.txt"), "gone\n");
    write(&dir.join("docs/plans/active/p.md"), PLAN);
    write(
        &dir.join("docs/plans/completed/p-m1.md"),
        "## Decision log\n\n* 2026-10-31, agent: old.\n",
    );
    let base = commit(dir, "start");
    write(
        &dir.join("protocol/src/lib.rs"),
        "pub struct A;\npub struct B;\n",
    );
    write(&dir.join("docs/README.md"), "# Docs\n\nMore.\n");
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    write(
        &dir.join("docs/plans/active/p.md"),
        &format!("{PLAN}* 2026-11-02, agent (implementer): a choice.\n"),
    );
    commit(dir, "change");
    let out = run(dir, &[&base, "docs/plans/active/p.md"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!(
            "hand-run protocol/src/lib.rs:2 RIGHT: {HAND_RUN}; a strict path of PROTO-2; a strict path of MEM-2; {MESSAGES}\n\
             judgement docs/plans/active/p.md:6 RIGHT: agent (implementer): a choice.\n\
             judgement docs/plans/completed/p-m1.md (no diff line; list it in the PR): agent: old.\n\
             attention: 3 candidates; pick at most 10, the hand-run lines first (decision 0027)\n"
        )
    );
    let out = run(dir, &["no-such-base", "docs/plans/active/p.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

#[test]
fn no_argument_exits_2() {
    let out = run(&std::env::temp_dir(), &[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "usage: xtask attention <base> <plan>\n"
    );
    assert_eq!(run(&std::env::temp_dir(), &["a"]).status.code(), Some(2));
}
