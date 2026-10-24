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
