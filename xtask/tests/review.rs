//! `xtask review` as a person or a skill runs it: the exit code, and which
//! stream gets the lines.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A scratch repo on `main` with one commit, removed when the test ends.
struct Repo(PathBuf);

impl Repo {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("xtask-review-bin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Repo(dir);
        repo.git(&["init", "-q", "-b", "main"]);
        let hooks = repo.0.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        repo.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
        std::fs::write(repo.0.join(".gitignore"), "target/\n").unwrap();
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "--no-verify", "-m", "start"]);
        repo
    }

    /// Runs git with an identity and no signing, for CI and for a machine
    /// that signs every commit.
    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=lloyd",
                "-c",
                "user.email=lloyd@beachape.example",
            ])
            .args(args)
            .current_dir(&self.0)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn xtask(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .current_dir(&self.0)
            .env("HOME", self.home())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
    }

    fn home(&self) -> &Path {
        &self.0
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_refusal_exits_3_on_stderr() {
    let repo = Repo::new("refusal");
    let out = repo.xtask(&["review", "check", "1"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(
        text(&out.stderr).starts_with("no record for this branch and round"),
        "{out:?}"
    );
    assert_eq!(text(&out.stdout), "");
}

#[test]
fn a_usage_error_exits_3_on_stderr() {
    let repo = Repo::new("usage");
    let out = repo.xtask(&["review", "nope"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(
        text(&out.stderr).starts_with("usage: xtask review"),
        "{out:?}"
    );
    assert_eq!(text(&out.stdout), "");
}

#[test]
fn a_success_exits_0_on_stdout() {
    let repo = Repo::new("success");
    let out = repo.xtask(&["review", "start", "main", "1"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(text(&out.stdout).starts_with("base: "), "{out:?}");
    assert_eq!(text(&out.stderr), "");
    let out = repo.xtask(&["review", "check", "1"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(text(&out.stdout), "no change\n");
}

fn worktree_paths(repo: &Repo) -> String {
    let out = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repo.0)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    text(&out.stdout)
}

#[test]
fn review_clean_removes_the_worktrees_of_one_round_only() {
    let repo = Repo::new("clean");
    let review = repo.0.canonicalize().unwrap().join("target/review");
    let names = [
        "1-reviewer-1",
        "1-docs-reviewer-1",
        "1-validator-1-3-1",
        "10-reviewer-1",
        "2-reviewer-1",
        "1-stranger-1",
    ];
    for name in names {
        let path = review.join(name);
        repo.git(&[
            "worktree",
            "add",
            "-q",
            "--detach",
            path.to_str().unwrap(),
            "HEAD",
        ]);
    }
    // A reviewer's trial change, which is scratch by design.
    std::fs::write(review.join("1-reviewer-1/.gitignore"), "changed\n").unwrap();
    std::fs::create_dir_all(review.join("1-baseline-1")).unwrap();
    std::fs::write(review.join("1-baseline-1/a.txt"), "a").unwrap();
    for keep in ["run/main/1", "target/debug"] {
        std::fs::create_dir_all(review.join(keep)).unwrap();
    }
    // A worktree with a round's name outside `target/review/`, with a
    // change, which stays.
    let agent = repo
        .0
        .canonicalize()
        .unwrap()
        .join(".claude/worktrees/1-reviewer-1");
    repo.git(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        agent.to_str().unwrap(),
        "HEAD",
    ]);
    std::fs::write(agent.join(".gitignore"), "kept\n").unwrap();

    let out = repo.xtask(&["review", "clean", "1"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);
    let removed: Vec<&str> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("removed target/review/"))
        .collect();
    assert_eq!(
        removed,
        [
            "1-docs-reviewer-1",
            "1-reviewer-1",
            "1-validator-1-3-1",
            "1-baseline-1"
        ],
        "{stdout}"
    );
    let listing = worktree_paths(&repo);
    for name in &names[..3] {
        assert!(!review.join(name).exists(), "{name} is still there");
        assert!(
            !listing.contains(&format!("{}\n", review.join(name).display())),
            "{listing}"
        );
    }
    assert!(!review.join("1-baseline-1").exists());
    for name in &names[3..] {
        assert!(
            listing.contains(&format!("worktree {}\n", review.join(name).display())),
            "{name}: {listing}"
        );
    }
    assert!(review.join("run/main/1").is_dir());
    assert!(review.join("target/debug").is_dir());
    assert!(
        listing.contains(&format!("worktree {}\n", agent.display())),
        "{listing}"
    );
    assert_eq!(
        std::fs::read_to_string(agent.join(".gitignore")).unwrap(),
        "kept\n"
    );

    let out = repo.xtask(&["review", "clean", "1"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(text(&out.stdout), "no worktrees of round 1\n");
}

#[test]
fn review_clean_refuses_a_bad_round() {
    let repo = Repo::new("clean-round");
    for round in ["0", "x"] {
        let out = repo.xtask(&["review", "clean", round]);
        assert_eq!(out.status.code(), Some(3), "{round}: {out:?}");
        assert!(text(&out.stderr).contains("usage: xtask review"), "{out:?}");
    }
    let out = repo.xtask(&["review", "clean"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(
        text(&out.stderr).contains("clean <round>"),
        "the usage names clean: {out:?}"
    );
}

/// TD-33: `xtask review mutants` builds with the `mutants` profile, which
/// must exist and keep debug assertions and overflow checks.  Without them a
/// mutant that only an overflow panic catches shows as missed.
#[test]
fn the_mutants_profile_keeps_debug_assertions_and_overflow_checks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let read = |path: &str| std::fs::read_to_string(root.join(path)).unwrap();
    let config = read(".cargo/mutants.toml");
    assert!(
        config.lines().any(|l| l.trim() == r#"profile = "mutants""#),
        ".cargo/mutants.toml selects the mutants profile"
    );
    let manifest = read("Cargo.toml");
    let table: Vec<&str> = manifest
        .lines()
        .skip_while(|l| l.trim() != "[profile.mutants]")
        .skip(1)
        .take_while(|l| !l.trim_start().starts_with('['))
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert!(
        table.contains(&r#"inherits = "test""#),
        "[profile.mutants] inherits the test profile: {table:?}"
    );
    for key in ["debug-assertions", "overflow-checks"] {
        assert!(
            !table.iter().any(|l| l.starts_with(key)),
            "[profile.mutants] keeps {key} as the test profile has it: {table:?}"
        );
    }
}

#[test]
fn review_clean_fails_on_a_locked_worktree_and_keeps_it() {
    let repo = Repo::new("clean-locked");
    let review = repo.0.canonicalize().unwrap().join("target/review");
    let locked = review.join("1-reviewer-1");
    let path = locked.to_str().unwrap();
    repo.git(&["worktree", "add", "-q", "--detach", path, "HEAD"]);
    repo.git(&["worktree", "lock", path]);

    let out = repo.xtask(&["review", "clean", "1"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(
        text(&out.stderr).starts_with("failed target/review/1-reviewer-1: "),
        "{out:?}"
    );
    assert!(locked.join(".gitignore").is_file(), "the folder went");
    assert!(
        worktree_paths(&repo).contains(&format!("worktree {path}\n")),
        "the worktree is no longer registered"
    );
}

#[test]
fn review_clean_removes_registered_worktrees_through_a_linked_target() {
    let repo = Repo::new("clean-link");
    let elsewhere = repo.0.with_extension("target");
    let _ = std::fs::remove_dir_all(&elsewhere);
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, repo.0.join("target")).unwrap();
    repo.git(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        "target/review/1-reviewer-1",
        "HEAD",
    ]);

    let out = repo.xtask(&["review", "clean", "1"]);
    let listing = worktree_paths(&repo);
    let _ = std::fs::remove_dir_all(&elsewhere);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(text(&out.stdout), "removed target/review/1-reviewer-1\n");
    assert!(
        !listing.contains("1-reviewer-1"),
        "git still has the worktree: {listing}"
    );
}
