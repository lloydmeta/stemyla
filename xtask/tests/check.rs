//! `xtask check` on temporary repos: the changed paths of each tier, and
//! the binary.  No test runs on this repo.  One test runs `xtask check`
//! without `--dry-run`, in a temporary workspace with its own target
//! folder, where the first check fails.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use xtask::check::{Tier, changed_paths, repo_root};

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
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
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

/// A folder `top` in the temporary folder, with the repo `top/repo`.  The
/// folder goes when the test ends.
struct Fixture {
    top: PathBuf,
    repo: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.top);
    }
}

impl Fixture {
    /// An empty repo on `main`, with no commit.
    fn empty(name: &str) -> Fixture {
        let top = std::env::temp_dir().join(format!("xtask-check-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        std::fs::create_dir_all(&top).unwrap();
        let top = top.canonicalize().unwrap();
        let repo = top.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        let hooks = repo.join(".git/hooks");
        git(
            &repo,
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
        );
        Fixture { top, repo }
    }

    /// A repo with one commit of `a.txt`, `b.txt` and `docs/c.md`.
    fn committed(name: &str) -> Fixture {
        let f = Fixture::empty(name);
        write(&f.repo.join(".gitignore"), "ignored/\n");
        write(&f.repo.join("a.txt"), "a\n");
        write(&f.repo.join("b.txt"), "b\n");
        write(&f.repo.join("docs/c.md"), "c\n");
        commit(&f.repo, "start");
        f
    }

    fn paths(&self, tier: Tier) -> (Vec<String>, String) {
        changed_paths(&self.repo, tier).unwrap()
    }
}

fn strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

const AGAINST_HEAD: &str = "the changes against HEAD and the untracked files";
const LAST_COMMIT: &str = "the last commit (HEAD^..HEAD)";
const ROOT_COMMIT: &str = "the root commit";

#[test]
fn unstaged_staged_and_untracked_changes_give_their_paths() {
    let f = Fixture::committed("uncommitted");
    write(&f.repo.join("a.txt"), "a2\n");
    assert_eq!(
        f.paths(Tier::Commit),
        (strings(&["a.txt"]), AGAINST_HEAD.to_string())
    );
    write(&f.repo.join("b.txt"), "b2\n");
    git(&f.repo, &["add", "b.txt"]);
    write(&f.repo.join("new/d.txt"), "d\n");
    write(&f.repo.join("ignored/e.txt"), "e\n");
    assert_eq!(
        f.paths(Tier::Commit),
        (
            strings(&["a.txt", "b.txt", "new/d.txt"]),
            AGAINST_HEAD.to_string()
        )
    );
}

#[test]
fn a_clean_tree_gives_the_last_commit_and_a_root_commit_gives_each_file() {
    let f = Fixture::committed("clean");
    assert_eq!(
        f.paths(Tier::Commit),
        (
            strings(&[".gitignore", "a.txt", "b.txt", "docs/c.md"]),
            ROOT_COMMIT.to_string()
        )
    );
    write(&f.repo.join("b.txt"), "b2\n");
    write(&f.repo.join("docs/deep/f.md"), "f\n");
    commit(&f.repo, "second");
    assert_eq!(
        f.paths(Tier::Commit),
        (
            strings(&["b.txt", "docs/deep/f.md"]),
            LAST_COMMIT.to_string()
        )
    );
}

#[test]
fn a_clean_tree_at_a_merge_reads_its_first_parent() {
    let f = Fixture::committed("merge");
    git(&f.repo, &["switch", "-q", "-c", "side"]);
    write(&f.repo.join("side.txt"), "side\n");
    commit(&f.repo, "side");
    git(&f.repo, &["switch", "-q", "main"]);
    write(&f.repo.join("main.txt"), "main\n");
    commit(&f.repo, "main");
    git(&f.repo, &["merge", "-q", "--no-ff", "--no-edit", "side"]);
    assert_eq!(
        f.paths(Tier::Commit),
        (strings(&["side.txt"]), LAST_COMMIT.to_string())
    );
}

#[test]
fn a_run_from_a_subfolder_reads_the_whole_repo() {
    let f = Fixture::committed("subfolder");
    let sub = f.repo.join("docs");
    assert_eq!(repo_root(&sub).unwrap(), f.repo);
    write(&f.repo.join("a.txt"), "a2\n");
    write(&f.repo.join("top.txt"), "top\n");
    write(&f.repo.join("docs/new.md"), "new\n");
    let from_sub = changed_paths(&repo_root(&sub).unwrap(), Tier::Commit).unwrap();
    assert_eq!(
        from_sub,
        (
            strings(&["a.txt", "docs/new.md", "top.txt"]),
            AGAINST_HEAD.to_string()
        )
    );
    assert_eq!(from_sub, f.paths(Tier::Commit));
}

#[test]
fn a_rename_gives_both_paths_and_a_space_stays_whole() {
    let f = Fixture::committed("rename");
    git(&f.repo, &["mv", "a.txt", "docs/a b.txt"]);
    assert_eq!(
        f.paths(Tier::Commit),
        (
            strings(&["a.txt", "docs/a b.txt"]),
            AGAINST_HEAD.to_string()
        )
    );
    commit(&f.repo, "rename");
    assert_eq!(
        f.paths(Tier::Commit),
        (strings(&["a.txt", "docs/a b.txt"]), LAST_COMMIT.to_string())
    );
}

/// A staged change that the working tree undoes is still in the commit,
/// so tier `commit` sees it.
#[test]
fn tier_commit_sees_a_staged_change_that_the_tree_undoes() {
    let f = Fixture::committed("staged-undone");
    write(&f.repo.join("a.txt"), "a2\n");
    git(&f.repo, &["add", "a.txt"]);
    write(&f.repo.join("a.txt"), "a\n");
    assert_eq!(
        f.paths(Tier::Commit),
        (strings(&["a.txt"]), AGAINST_HEAD.to_string())
    );
    write(&f.repo.join("new.rs"), "// new\n");
    git(&f.repo, &["add", "new.rs"]);
    std::fs::remove_file(f.repo.join("new.rs")).unwrap();
    assert_eq!(
        f.paths(Tier::Commit),
        (strings(&["a.txt", "new.rs"]), AGAINST_HEAD.to_string())
    );
}

#[test]
fn tier_push_reads_the_paths_since_the_merge_base() {
    let f = Fixture::committed("push");
    let base = git(&f.repo, &["rev-parse", "HEAD"]);
    write(&f.repo.join("main-only.txt"), "main\n");
    let main = commit(&f.repo, "main only");
    git(&f.repo, &["update-ref", "refs/remotes/origin/main", &main]);
    git(&f.repo, &["switch", "-q", "-c", "branch", &base]);
    write(&f.repo.join("b.txt"), "b2\n");
    commit(&f.repo, "one");
    // A committed rename and an uncommitted rename each give both paths.
    git(&f.repo, &["mv", "docs/c.md", "docs/moved.md"]);
    commit(&f.repo, "two");
    git(&f.repo, &["mv", "a.txt", "docs/a b.txt"]);
    write(&f.repo.join("new.txt"), "new\n");
    assert_eq!(
        f.paths(Tier::Push),
        (
            strings(&[
                "a.txt",
                "b.txt",
                "docs/a b.txt",
                "docs/c.md",
                "docs/moved.md",
                "new.txt"
            ]),
            since_the_merge_base(&base)
        )
    );
}

/// The phrase of tier `push` for the merge base `base`.
fn since_the_merge_base(base: &str) -> String {
    let short = &base[..12];
    format!("the changes since the merge base {short} with origin/main, and the untracked files")
}

/// A committed change that the working tree reverts is a change of the
/// branch, so tier `push` sees it.  So does a committed rename that the
/// working tree reverts.
#[test]
fn tier_push_sees_a_committed_change_that_the_tree_reverts() {
    let f = Fixture::committed("push-revert");
    let base = git(&f.repo, &["rev-parse", "HEAD"]);
    git(&f.repo, &["update-ref", "refs/remotes/origin/main", &base]);
    git(&f.repo, &["switch", "-q", "-c", "branch"]);
    write(&f.repo.join("b.txt"), "b2\n");
    git(&f.repo, &["mv", "a.txt", "moved.txt"]);
    commit(&f.repo, "one");
    write(&f.repo.join("b.txt"), "b\n");
    git(&f.repo, &["mv", "moved.txt", "a.txt"]);
    assert_eq!(
        f.paths(Tier::Push),
        (
            strings(&["a.txt", "b.txt", "moved.txt"]),
            since_the_merge_base(&base)
        )
    );
}

#[test]
fn tier_push_with_no_origin_main_gives_a_warning_and_no_path() {
    let f = Fixture::committed("no-origin");
    write(&f.repo.join("a.txt"), "a2\n");
    let (paths, source) = f.paths(Tier::Push);
    assert_eq!(paths, Vec::<String>::new());
    assert!(
        source.starts_with("warning: no merge base of HEAD and origin/main"),
        "{source}"
    );
    assert!(source.ends_with("so tier push runs every line"), "{source}");
}

/// The binary in `dir` with `args`, and the git config of [`GIT_ENV`].
/// It has no `STEMYLA_REQUIRE`, so the GPU test line has its form of
/// `CLAUDE.md`.
fn command(dir: &Path, args: &[&str], target: Option<&Path>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .args(args)
        .current_dir(dir)
        .envs(GIT_ENV)
        .env_remove("STEMYLA_REQUIRE");
    if let Some(target) = target {
        command.env("CARGO_TARGET_DIR", target);
    }
    command
}

/// Runs the binary of [`command`].
fn xtask(dir: &Path, args: &[&str], target: Option<&Path>) -> Output {
    command(dir, args, target).output().unwrap()
}

#[test]
fn a_repo_with_no_commit_gives_exit_2() {
    let f = Fixture::empty("no-commit");
    write(&f.repo.join("a.txt"), "a\n");
    let error = match changed_paths(&f.repo, Tier::Commit) {
        Ok(paths) => format!("no error: {paths:?}"),
        Err(e) => format!("{e:#}"),
    };
    assert!(error.contains("the repo has no commit"), "{error}");
    let out = xtask(&f.repo, &["check", "--tier", "commit", "--dry-run"], None);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("the repo has no commit"), "{stderr}");
}

#[test]
fn a_bad_tier_gives_exit_2_and_the_usage() {
    let f = Fixture::committed("bad-tier");
    for args in [
        &["check", "--tier", "nothing"][..],
        &["check"],
        &["check", "--tier"],
        &["check", "--tier", "commit", "--now"],
    ] {
        let out = xtask(&f.repo, args, None);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
        assert_eq!(
            stderr, "usage: xtask check --tier commit|push [--dry-run]\n",
            "{args:?}"
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{args:?}");
    }
}

/// A repo with its own workspace of one small crate with no
/// dependencies, so cargo needs no network and does not walk up to this
/// repo.  Its build folder is outside the repo.
fn small_workspace(name: &str) -> (Fixture, PathBuf) {
    let f = Fixture::empty(name);
    let target = f.top.join("target");
    write(
        &f.repo.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\nmembers = [\"small\"]\n",
    );
    write(
        &f.repo.join("small/Cargo.toml"),
        "[package]\nname = \"small\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    write(&f.repo.join("small/src/lib.rs"), "pub fn f() {}\n");
    let lock = Command::new(env!("CARGO"))
        .args(["generate-lockfile", "--offline"])
        .current_dir(&f.repo)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    commit(&f.repo, "small");
    (f, target)
}

#[test]
fn the_dry_run_prints_the_whole_plan_and_changes_no_file() {
    let (f, target) = small_workspace("dry-run");
    // `cargo fmt --all --check` fails on this file.
    write(&f.repo.join("small/src/lib.rs"), "pub fn  f( ) {}\n");
    let out = xtask(
        &f.repo,
        &["check", "--tier", "commit", "--dry-run"],
        Some(&target),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stdout}{stderr}");
    assert_eq!(stderr, "");
    let want = "\
check: tier commit
check: paths: 1, from the changes against HEAD and the untracked files
check: tier commit runs: the changed paths choose the checks
check: run: cargo fmt --all --check (every change)
check: run: cargo run -p xtask -- docs-check (every change)
check: run: cargo clippy --workspace --all-targets -- -D warnings (every change)
check: skip: cargo clippy -p synth --all-targets -- -D warnings (no changed member is in the closure of synth)
check: skip: cargo clippy -p app -- -D warnings (no changed member is in the closure of app)
check: skip: cargo clippy -p app --all-targets -- -D warnings (no changed member is in the closure of app)
check: skip: cargo clippy --target wasm32-unknown-unknown -p app -p stem-worker -p decode-worker -p stem-protocol -p engine --all-targets -- -D warnings (no changed member is in the closure of the wasm32 packages)
check: run: cargo test -p xtask --lib --test rules --test docs --test review (these tests read the whole repo, and xtask is not a dependent of a changed member)
check: run: cargo test --workspace --exclude roformer-gpu (the changed members and their dependents are every member but roformer-gpu)
check: skip: cargo test -p roformer-gpu --lib (roformer-gpu is not a dependent of a changed member)
check: skip: cargo test -p roformer-gpu --doc (roformer-gpu is not a dependent of a changed member)
check: skip: STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release (it needs a GPU adapter, and tier push runs it when its paths change GPU code; GPU-2 asks for it on a kernel change, and a pass counts for GPU-3 only on a real GPU named in the PR)
check: skip: (cd app && trunk build --release) (tier push runs it)
check: skip: cargo run -p xtask --release -- serve app/dist --port 8080 (CI runs the browser tests; run a scenario by hand only when CI failed it (decision 0026))
check: skip: cargo run -p xtask -- web-test (CI runs the browser tests; run a scenario by hand only when CI failed it (decision 0026))
check: skip: cargo run -p xtask -- e2e <scenario> (CI runs the browser tests; run a scenario by hand only when CI failed it (decision 0026))
check: skip: cargo +nightly-2026-10-30 test -p roformer-gpu --doc (CI runs it; it needs the pinned nightly, and the stable doctest line runs the same doctests)
check: skip: cargo run -p xtask --release -- publish (a person only (GIT-1))
";
    assert_eq!(stdout, want);
    assert_eq!(
        git(&f.repo, &["status", "--porcelain"]),
        "M small/src/lib.rs"
    );
}

#[test]
fn the_dry_run_prints_each_strict_hit_and_tier_push() {
    let f = Fixture::committed("strict");
    write(&f.repo.join("engine/src/slot.rs"), "// x\n");
    write(&f.repo.join("worker/src/web.rs"), "// x\n");
    let out = xtask(&f.repo, &["check", "--tier", "commit", "--dry-run"], None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let head: Vec<&str> = stdout.lines().take(8).collect();
    assert_eq!(
        head,
        [
            "check: tier commit",
            "check: paths: 2, from the changes against HEAD and the untracked files",
            "check: strict PROTO-2: worker/src/web.rs",
            "check: strict MEM-2: worker/src/web.rs",
            "check: strict MEM-5: worker/src/web.rs",
            "check: strict RT-1: engine/src/slot.rs",
            "check: tier push runs: a strict path changed (D2)",
            "check: run: cargo fmt --all --check (tier push runs every line)",
        ]
    );
    assert_eq!(stdout.lines().count(), 7 + 17);
}

#[test]
fn the_dry_run_of_tier_push_with_no_origin_main_warns() {
    let f = Fixture::committed("push-warning");
    let out = xtask(&f.repo, &["check", "--tier", "push", "--dry-run"], None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let head: Vec<&str> = stdout.lines().take(4).collect();
    assert_eq!(head.len(), 4, "{stdout}");
    assert_eq!(head[0], "check: tier push");
    assert!(
        head[1].starts_with("warning: no merge base of HEAD and origin/main"),
        "{stdout}"
    );
    assert_eq!(head[2], "check: paths: 0");
    assert_eq!(head[3], "check: tier push runs: the tier that was asked");
    assert!(
        stdout.contains(
            "check: run: STEMYLA_REQUIRE=gpu,f16 cargo test -p roformer-gpu --release (no path changed, or no path is known, so a GPU change cannot be ruled out)"
        ),
        "{stdout}"
    );
}

#[test]
fn a_graph_that_fails_gives_a_warning_and_tier_push() {
    // No `Cargo.toml`, so `cargo metadata` fails.  CI sets
    // `CARGO_TERM_COLOR=always`, so the test sets it too: the warning must
    // not hold colour codes.
    let f = Fixture::committed("no-graph");
    write(&f.repo.join("docs/c.md"), "c2\n");
    let out = command(&f.repo, &["check", "--tier", "commit", "--dry-run"], None)
        .env("CARGO_TERM_COLOR", "always")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let head: Vec<&str> = stdout.lines().take(4).collect();
    assert_eq!(head.len(), 4, "{stdout}");
    assert_eq!(head[0], "check: tier commit");
    assert_eq!(
        head[1],
        "check: paths: 1, from the changes against HEAD and the untracked files"
    );
    assert_eq!(
        head[2],
        format!(
            "warning: cannot read the workspace graph: cargo metadata failed: error: could not find `Cargo.toml` in `{}` or any parent directory",
            f.repo.display()
        ),
        "{stdout}"
    );
    assert_eq!(
        head[3],
        "check: tier push runs: there is no workspace graph, so tier commit cannot choose the crates"
    );
    for line in stdout.lines() {
        assert!(
            line.starts_with("check: ") || line.starts_with("warning: "),
            "{line}"
        );
    }
}

#[test]
fn the_callers_stemyla_require_gives_the_gpu_test_line_its_form() {
    let f = Fixture::committed("push-require");
    let out = command(&f.repo, &["check", "--dry-run", "--tier", "push"], None)
        .env("STEMYLA_REQUIRE", "gpu")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let gpu: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("cargo test -p roformer-gpu --release"))
        .collect();
    assert_eq!(
        gpu,
        [
            "check: run: STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release (no path changed, or no path is known, so a GPU change cannot be ruled out; STEMYLA_REQUIRE=gpu from the environment replaces gpu,f16)"
        ],
        "{stdout}"
    );
}

/// `text` with each time `in <n>.<n> s` at the end of a line as
/// `in <t> s`.
fn timeless(line: &str) -> String {
    match line.rsplit_once(" in ") {
        Some((head, time))
            if time.strip_suffix(" s").is_some_and(|time| {
                time.split_once('.').is_some_and(|(whole, tenth)| {
                    !whole.is_empty()
                        && whole.bytes().all(|b| b.is_ascii_digit())
                        && tenth.len() == 1
                        && tenth.bytes().all(|b| b.is_ascii_digit())
                })
            }) =>
        {
            format!("{head} in <t> s")
        }
        _ => line.to_string(),
    }
}

/// The one real run of `xtask check`.  It runs in a temporary workspace
/// outside this repo, with its own target folder, so it never runs the
/// tests of this repo.  Its first check fails, so it runs one check only.
#[test]
fn a_real_run_stops_at_the_first_failure_and_changes_no_file() {
    let (f, target) = small_workspace("real-run");
    let bad = "pub fn  f( ) {}\n";
    write(&f.repo.join("small/src/lib.rs"), bad);
    let lock = std::fs::read(f.repo.join("Cargo.lock")).unwrap();
    let out = xtask(&f.repo, &["check", "--tier", "commit"], Some(&target));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stdout}{stderr}");
    let run: Vec<String> = stdout
        .lines()
        .filter(|line| {
            [
                "check: start:",
                "check: ok",
                "check: failed",
                "check: not run:",
            ]
            .iter()
            .any(|start| line.starts_with(start))
                || line.contains(" failed, after ")
        })
        .map(timeless)
        .collect();
    assert_eq!(
        run,
        [
            "check: start: cargo fmt --all --check",
            "check: failed (exit 1) in <t> s",
            "check: not run: cargo run -p xtask -- docs-check",
            "check: not run: cargo clippy --workspace --all-targets -- -D warnings",
            "check: not run: cargo test -p xtask --lib --test rules --test docs --test review",
            "check: not run: cargo test --workspace --exclude roformer-gpu",
            "check: cargo fmt --all --check failed, after 0 passed, in <t> s",
        ],
        "{stdout}{stderr}"
    );
    assert_eq!(
        stdout.lines().last().map(timeless).as_deref(),
        Some("check: cargo fmt --all --check failed, after 0 passed, in <t> s"),
        "{stdout}"
    );
    assert_eq!(
        git(&f.repo, &["status", "--porcelain"]),
        "M small/src/lib.rs"
    );
    assert_eq!(
        std::fs::read_to_string(f.repo.join("small/src/lib.rs")).unwrap(),
        bad
    );
    assert_eq!(std::fs::read(f.repo.join("Cargo.lock")).unwrap(), lock);
}
