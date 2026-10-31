//! `xtask clean` on a temporary repo with planted leftovers.  No test runs
//! on this repo, and no test calls the real `gh`: a fake one is first on
//! `PATH`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

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
    git(
        dir,
        &[
            "commit",
            "-q",
            "--no-verify",
            "--allow-empty",
            "-m",
            message,
        ],
    );
    git(dir, &["rev-parse", "HEAD"])
}

/// A child process, stopped and reaped when the test ends, also when an
/// assert fails.
struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Kill {
    fn pid(&self) -> u32 {
        self.0.id()
    }

    fn runs(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }
}

/// `sleep`, with `held` open as its standard input.
fn sleep_holding(held: &Path) -> Kill {
    Kill(
        Command::new("sleep")
            .arg("300")
            .stdin(std::fs::File::open(held).unwrap())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

fn ps_stat(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "stat="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The fake `gh` that prints these PRs, or fails.
enum Gh {
    Prs { open: String, merged: String },
    Fails,
}

/// A real `xtask serve`, which a thread reaps so that it does not stay a
/// zombie.  It gets SIGKILL when the test ends, if it still runs.
struct Serve {
    pid: u32,
    reaped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Serve {
    fn drop(&mut self) {
        if !self.reaped.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = Command::new("kill")
                .args(["-KILL", &self.pid.to_string()])
                .status();
        }
    }
}

/// A temporary folder with a repo and its leftovers.  The fields drop in
/// order: the processes stop before the folder goes.
struct Fixture {
    serve: Option<Serve>,
    children: Vec<Kill>,
    top: PathBuf,
    repo: PathBuf,
    bin: PathBuf,
    /// The kept and the removable scratch roots.
    scratch_busy: PathBuf,
    scratch_free: PathBuf,
    /// The pid of the zombie, which is no longer a running child.
    zombie: u32,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.serve = None;
        self.children.clear();
        let _ = std::fs::remove_dir_all(&self.top);
    }
}

impl Fixture {
    /// The repo `top/repo` on `main`, with an `origin/main`, and nothing
    /// planted yet.
    fn empty(name: &str) -> Fixture {
        let top = std::env::temp_dir().join(format!("xtask-clean-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        std::fs::create_dir_all(&top).unwrap();
        let top = top.canonicalize().unwrap();
        let repo = top.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        let hooks = repo.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        git(
            &repo,
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
        );
        write(&repo.join(".gitignore"), "target/\n*.wav\n");
        write(&repo.join("held.txt"), "held\n");
        write(&repo.join("docs/reviews/old-m1.md"), "old log\n");
        write(&repo.join("docs/reviews/agent-x-m1.md"), "agent-x log\n");
        write(&repo.join("docs/reviews/open-pr.md"), "open-pr log\n");
        let start = commit(&repo, "start");
        git(&repo, &["update-ref", "refs/remotes/origin/main", &start]);
        Fixture {
            serve: None,
            children: Vec::new(),
            bin: top.join("bin"),
            scratch_busy: top.join("scratch-busy"),
            scratch_free: top.join("scratch-free"),
            zombie: 0,
            top,
            repo,
        }
    }

    /// The fixtures of step 3 of the plan, but the real `xtask serve`,
    /// which [`Fixture::start_serve`] adds.
    fn planted(name: &str) -> Fixture {
        let mut f = Fixture::empty(name);
        let repo = f.repo.clone();
        let start = git(&repo, &["rev-parse", "HEAD"]);

        // origin/main: a commit on top of `start`, then a squash of
        // `squashed`.
        git(&repo, &["switch", "-q", "-c", "ancestor"]);
        write(&repo.join("m1.txt"), "m1\n");
        let m1 = commit(&repo, "m1");
        git(&repo, &["branch", "ancestor-wt", &m1]);
        git(&repo, &["branch", "agent-x", &m1]);
        let branch_from_start = |name: &str, files: &[&str]| -> String {
            git(&repo, &["switch", "-q", "-c", name, &start]);
            let mut tip = String::new();
            for file in files {
                write(&repo.join(file), file);
                tip = commit(&repo, file);
            }
            tip
        };
        let squashed = branch_from_start("squashed", &["s1.txt", "s2.txt"]);
        branch_from_start("unmerged", &["u.txt"]);
        let moved_on_merged = branch_from_start("moved-on", &["p1.txt"]);
        write(&repo.join("p2.txt"), "p2\n");
        commit(&repo, "p2");
        let merged_pr = branch_from_start("merged-pr", &["q.txt"]);
        branch_from_start("agent-y", &["y.txt"]);
        git(&repo, &["switch", "-q", "main"]);
        let tree = git(&repo, &["rev-parse", &format!("{squashed}^{{tree}}")]);
        let squash = git(&repo, &["commit-tree", &tree, "-p", &m1, "-m", "squash"]);
        git(&repo, &["update-ref", "refs/remotes/origin/main", &squash]);

        let gh = Gh::Prs {
            open: r#"[{"headRefName":"open-pr"}]"#.to_string(),
            merged: format!(
                r#"[{{"headRefName":"merged-pr","headRefOid":"{merged_pr}"}},{{"headRefName":"moved-on","headRefOid":"{moved_on_merged}"}}]"#
            ),
        };
        f.fake_gh(&gh);

        // Worktrees.
        let add = |rel: &str, at: &str| -> PathBuf {
            git(&repo, &["worktree", "add", "-q", "--detach", rel, at]);
            repo.join(rel)
        };
        let dirty = add("target/review/1-reviewer-1", &start);
        write(&dirty.join("held.txt"), "changed\n");
        add("target/review/1-reviewer-2", &start);
        let wav = add("target/review/1-reviewer-3", &start);
        write(&wav.join("x.wav"), "RIFF");
        let built = add("target/review/1-reviewer-4", &start);
        write(&built.join("target/debug/out"), "built");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                ".claude/worktrees/agent-x",
                "agent-x",
            ],
        );
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                ".claude/worktrees/agent-y",
                "agent-y",
            ],
        );
        let used = add("target/review/1-reviewer-5", &start);
        f.children.push(sleep_holding(&used.join("held.txt")));
        let lost = add("target/review/1-reviewer-6", &start);
        commit(&lost, "lost");
        let outside = f.top.join("outside-wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                outside.to_str().unwrap(),
                "ancestor-wt",
            ],
        );
        let gone = add("target/review/1-reviewer-7", &start);
        std::fs::remove_dir_all(gone).unwrap();
        add("target/review/2-reviewer-1", &start);
        add("target/review/3-reviewer-1", &start);

        // Run folders.
        let run = repo.join("target/review/run");
        write(&run.join("old/log.md"), "old log\n");
        write(&run.join("nolog-copy/log.md"), "not a copy\n");
        write(&run.join("open-pr/log.md"), "open-pr log\n");
        write(&run.join("agent-x/log.md"), "agent-x log\n");
        write(&run.join("main/2/state.json"), "{}");
        write(&run.join("other/3/state.json"), "{}");

        // Build output.
        write(&repo.join("target/e2e/x"), "x");
        write(&repo.join("app/dist/index.html"), "<p>hi</p>");

        // Pid files: an ended pid, a zombie, and another program.
        let pids = repo.join("target/xtask-serve");
        let mut ended = Command::new("true").spawn().unwrap();
        ended.wait().unwrap();
        write(&pids.join("9001.pid"), &format!("{}\n", ended.id()));
        let zombie = Kill(Command::new("true").spawn().unwrap());
        let zombie_pid = zombie.pid();
        f.children.push(zombie);
        f.zombie = zombie_pid;
        let until = Instant::now() + Duration::from_secs(5);
        while !ps_stat(zombie_pid).starts_with('Z') {
            assert!(Instant::now() < until, "pid {zombie_pid} is no zombie");
            std::thread::sleep(Duration::from_millis(20));
        }
        write(&pids.join("9002.pid"), &format!("{zombie_pid}\n"));
        let other = sleep_holding(&repo.join("held.txt"));
        write(&pids.join("9003.pid"), &format!("{}\n", other.pid()));
        f.children.push(other);

        // Scratch roots.
        write(&f.scratch_free.join("clone/a.txt"), "a");
        write(&f.scratch_busy.join("b.txt"), "b");
        let busy = sleep_holding(&f.scratch_busy.join("b.txt"));
        f.children.push(busy);
        f
    }

    fn fake_gh(&self, gh: &Gh) {
        use std::os::unix::fs::PermissionsExt;
        let script = match gh {
            Gh::Fails => "#!/bin/sh\necho 'gh: no network' >&2\nexit 1\n".to_string(),
            Gh::Prs { open, merged } => format!(
                "#!/bin/sh\ncase \"$*\" in\n  *\"--state open\"*) printf '%s\\n' '{open}' ;;\n  *\"--state merged\"*) printf '%s\\n' '{merged}' ;;\n  *) echo \"fake gh: $*\" >&2; exit 1 ;;\nesac\n"
            ),
        };
        let path = self.bin.join("gh");
        write(&path, &script);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Starts the real `xtask serve` with its working folder in the clean
    /// worktree `target/review/4-reviewer-1`, and gives its port.
    fn start_serve(&mut self) -> u16 {
        use std::io::BufRead;
        let start = git(&self.repo, &["rev-parse", "HEAD"]);
        let rel = "target/review/4-reviewer-1";
        git(
            &self.repo,
            &["worktree", "add", "-q", "--detach", rel, &start],
        );
        let site = self.top.join("site");
        write(&site.join("index.html"), "<p>hi</p>");
        let mut child = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["serve", site.to_str().unwrap(), "--port", "0"])
            .current_dir(self.repo.join(rel))
            .envs(GIT_ENV)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let reaped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.serve = Some(Serve {
            pid: child.id(),
            reaped: reaped.clone(),
        });
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        std::thread::spawn(move || {
            let _ = child.wait();
            reaped.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        line.trim()
            .rsplit(':')
            .next()
            .and_then(|p| p.strip_suffix('/'))
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("no port in {line:?}"))
    }

    fn also(&self) -> Vec<String> {
        [&self.scratch_free, &self.scratch_busy]
            .iter()
            .flat_map(|p| ["--also".to_string(), p.to_str().unwrap().to_string()])
            .collect()
    }

    /// Runs `xtask clean` in the repo, with the fake `gh` first on `PATH`.
    fn clean(&self, args: &[String]) -> Output {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("clean")
            .args(args)
            .current_dir(&self.repo)
            .env("PATH", path)
            .envs(GIT_ENV)
            .output()
            .unwrap()
    }

    /// The branches, the worktrees, and each file under `top` but in a
    /// `.git`, with its size.
    fn state(&self) -> (String, String, BTreeSet<(PathBuf, u64)>) {
        fn walk(dir: &Path, out: &mut BTreeSet<(PathBuf, u64)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_name() == ".git" {
                    continue;
                }
                let meta = path.symlink_metadata().unwrap();
                out.insert((path.clone(), if meta.is_dir() { 0 } else { meta.len() }));
                if meta.is_dir() {
                    walk(&path, out);
                }
            }
        }
        let mut files = BTreeSet::new();
        walk(&self.top, &mut files);
        (
            git(
                &self.repo,
                &["for-each-ref", "--format=%(refname) %(objectname)"],
            ),
            git(&self.repo, &["worktree", "list", "--porcelain"]),
            files,
        )
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The lines of the "can remove" part and of the "keeps" part.
fn parts(stdout: &str) -> (Vec<String>, Vec<String>) {
    let mut removable = Vec::new();
    let mut kept = Vec::new();
    let mut part = None;
    for line in stdout.lines() {
        if line.starts_with("can remove (") {
            part = Some(true);
        } else if line.starts_with("keeps (") {
            part = Some(false);
        } else if let Some(item) = line.strip_prefix("  ") {
            match part {
                Some(true) => removable.push(item.to_string()),
                Some(false) => kept.push(item.to_string()),
                None => panic!("an item before a part: {line}"),
            }
        }
    }
    (removable, kept)
}

/// Asserts that `lines` has an item `<head> ...` that ends with `why`.
fn assert_item(lines: &[String], head: &str, why: &str) {
    assert!(
        lines.iter().any(|l| (l.starts_with(&format!("{head} ("))
            || l.starts_with(&format!("{head}: ")))
            && l.ends_with(why)),
        "no `{head}: {why}` in {lines:#?}"
    );
}

/// Items as `(head, why)`.
type Items = Vec<(String, String)>;

/// The removable and the kept items of the planted fixture, as
/// `(head, why)`.
fn expected(f: &Fixture) -> (Items, Items) {
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    let free = f.scratch_free.display();
    let busy = f.scratch_busy.display();
    let removable = vec![
        s("worktree target/review/1-reviewer-2", "clean and unused"),
        s("worktree target/review/1-reviewer-4", "clean and unused"),
        s(
            "worktree .claude/worktrees/agent-x",
            "clean, unused and merged",
        ),
        s(
            "worktree target/review/1-reviewer-7",
            "prunable: its folder is gone",
        ),
        s("worktree target/review/3-reviewer-1", "clean and unused"),
        s(
            "branch squashed",
            "its tree is the tree of a commit on origin/main",
        ),
        s("branch merged-pr", "a merged PR has this branch and tip"),
        s("branch ancestor", "its tip is an ancestor of origin/main"),
        s("branch agent-x", "its tip is an ancestor of origin/main"),
        s(
            "run-folder target/review/run/old",
            "its log.md has a copy in docs/reviews/",
        ),
        s(
            "run-folder target/review/run/agent-x",
            "its log.md has a copy in docs/reviews/",
        ),
        s("build target/e2e", "build output"),
        s("build app/dist", "build output"),
        s("pid-file target/xtask-serve/9001.pid", "has ended"),
        s("pid-file target/xtask-serve/9002.pid", "has ended"),
        s(
            "pid-file target/xtask-serve/9003.pid",
            "runs another program, which stays",
        ),
        (
            format!("scratch {free}"),
            "a scratch root named with --also".to_string(),
        ),
    ];
    let kept = vec![
        s("worktree target/review/1-reviewer-1", "has changes"),
        s("worktree target/review/1-reviewer-3", "holds ignored files"),
        s("worktree .claude/worktrees/agent-y", "not merged"),
        s("worktree target/review/1-reviewer-5", "a process uses it"),
        s(
            "worktree target/review/2-reviewer-1",
            "round 2 of main has no findings.md",
        ),
        s("branch main", "main and gh-pages stay"),
        s("branch unmerged", "no proof that it is merged"),
        s("branch moved-on", "no proof that it is merged"),
        s("branch agent-y", "the current branch of a worktree"),
        s("branch ancestor-wt", "the current branch of a worktree"),
        s(
            "run-folder target/review/run/nolog-copy",
            "log.md has no copy in docs/reviews/",
        ),
        s(
            "run-folder target/review/run/open-pr",
            "a branch with an open PR",
        ),
        s(
            "run-folder target/review/run/main",
            "the current branch of a worktree",
        ),
        s("run-folder target/review/run/other", "round 3 is open"),
        (format!("scratch {busy}"), "a process uses it".to_string()),
    ];
    (removable, kept)
}

#[test]
fn the_list_mode_names_each_item_and_changes_nothing() {
    let mut f = Fixture::planted("list");
    let before = f.state();
    let out = f.clean(&f.also());
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);
    let (removable, kept) = parts(&stdout);
    let (want_removable, want_kept) = expected(&f);
    for (head, why) in &want_removable {
        assert_item(&removable, head, why);
    }
    for (head, why) in &want_kept {
        assert_item(&kept, head, why);
    }
    // The detached worktree on a new commit names its HEAD.
    assert!(
        kept.iter().any(
            |l| l.starts_with("worktree target/review/1-reviewer-6: its HEAD ")
                && l.ends_with(" is on no ref")
        ),
        "{kept:#?}"
    );
    assert_eq!(removable.len(), want_removable.len(), "{removable:#?}");
    assert_eq!(kept.len(), want_kept.len() + 1, "{kept:#?}");
    assert!(stdout.contains("\ntarget/ holds "), "{stdout}");
    assert!(
        stdout
            .lines()
            .last()
            .unwrap()
            .starts_with("run `cargo run -p xtask -- clean --yes --also "),
        "{stdout}"
    );

    assert_eq!(f.state(), before, "the list mode changed the repo");
    let zombie = f.zombie;
    for child in f.children.iter_mut().filter(|c| c.pid() != zombie) {
        assert!(child.runs(), "pid {} stopped", child.pid());
    }
}

#[test]
fn also_refuses_a_path_that_is_not_a_scratch_root() {
    let f = Fixture::empty("also");
    let home = std::env::var("HOME").unwrap();
    let missing = f.top.join("missing");
    for path in [
        "/",
        home.as_str(),
        f.repo.to_str().unwrap(),
        f.top.to_str().unwrap(),
        "relative/path",
        missing.to_str().unwrap(),
        std::env::temp_dir().to_str().unwrap(),
    ] {
        let out = f.clean(&["--also".to_string(), path.to_string()]);
        assert_eq!(out.status.code(), Some(2), "--also {path}: {out:?}");
        assert_eq!(text(&out.stdout), "", "--also {path} listed items");
        assert!(text(&out.stderr).starts_with("--also "), "{out:?}");
    }
    let inside = f.repo.join("docs");
    let out = f.clean(&["--also".to_string(), inside.to_str().unwrap().to_string()]);
    assert_eq!(out.status.code(), Some(2), "a folder in the repo: {out:?}");
}

#[test]
fn an_unknown_word_prints_the_usage_and_exits_2() {
    let f = Fixture::empty("usage");
    let out = f.clean(&["--force".to_string()]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(text(&out.stdout), "");
    assert!(
        text(&out.stderr).starts_with("usage: xtask clean"),
        "{out:?}"
    );
}

/// The `removed ` and `failed ` lines of a `--yes` run.
fn actions(stdout: &str) -> (Vec<String>, Vec<String>) {
    let take = |prefix: &str| -> Vec<String> {
        stdout
            .lines()
            .filter_map(|l| l.strip_prefix(prefix))
            .map(ToString::to_string)
            .collect()
    };
    (take("removed "), take("failed "))
}

fn branches(repo: &Path) -> BTreeSet<String> {
    git(
        repo,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
    .lines()
    .map(ToString::to_string)
    .collect()
}

fn names_in(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn yes_removes_only_the_removable_items_and_a_second_run_finds_nothing() {
    let mut f = Fixture::planted("yes");
    let port = f.start_serve();
    let serve_pid = f.serve.as_ref().unwrap().pid;
    let pid_file = f.repo.join(format!("target/xtask-serve/{port}.pid"));
    assert!(pid_file.is_file(), "xtask serve wrote no pid file");

    let mut args = vec!["--yes".to_string()];
    args.extend(f.also());
    let out = f.clean(&args);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);

    // The first list: the server uses its worktree.
    let (removable, kept) = parts(&stdout);
    assert_item(
        &kept,
        "worktree target/review/4-reviewer-1",
        "a process uses it",
    );
    assert_item(
        &removable,
        &format!("pid-file target/xtask-serve/{port}.pid"),
        &format!("a running xtask serve on port {port} (pid {serve_pid})"),
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("run `cargo run -p xtask -- clean --yes --also ")),
        "{stdout}"
    );

    // What it removed.
    let (removed, failed) = actions(&stdout);
    assert_eq!(failed, Vec::<String>::new(), "{stdout}");
    let (want_removable, want_kept) = expected(&f);
    for (head, why) in &want_removable {
        assert_item(&removed, head, why);
    }
    assert_item(
        &removed,
        &format!("pid-file target/xtask-serve/{port}.pid"),
        &format!("a running xtask serve on port {port} (pid {serve_pid})"),
    );
    assert_item(
        &removed,
        "worktree target/review/4-reviewer-1",
        "clean and unused",
    );
    assert_eq!(removed.len(), want_removable.len() + 2, "{removed:#?}");
    for (head, _) in &want_kept {
        assert!(
            !removed
                .iter()
                .any(|l| l.starts_with(&format!("{head} ")) || l.starts_with(&format!("{head}:"))),
            "removed the kept {head}"
        );
    }

    // The worktrees.
    let listing = git(&f.repo, &["worktree", "list", "--porcelain"]);
    let review = f.repo.join("target/review");
    for gone in [
        "1-reviewer-2",
        "1-reviewer-4",
        "1-reviewer-7",
        "3-reviewer-1",
        "4-reviewer-1",
    ] {
        let path = review.join(gone);
        assert!(!path.exists(), "{gone} is still there");
        assert!(
            !listing.contains(&format!("worktree {}\n", path.display())),
            "{listing}"
        );
    }
    let agent_x = f.repo.join(".claude/worktrees/agent-x");
    assert!(!agent_x.exists() && !listing.contains(&agent_x.display().to_string()));
    for stays in [
        review.join("1-reviewer-1"),
        review.join("1-reviewer-3"),
        review.join("1-reviewer-5"),
        review.join("1-reviewer-6"),
        review.join("2-reviewer-1"),
        f.repo.join(".claude/worktrees/agent-y"),
        f.top.join("outside-wt"),
    ] {
        assert!(stays.is_dir(), "{} went", stays.display());
        assert!(
            listing.contains(&format!("worktree {}\n", stays.display())),
            "{listing}"
        );
    }
    assert!(review.join("1-reviewer-3/x.wav").is_file());

    // The branches, run folders, build output, pid files and scratch roots.
    let want: BTreeSet<String> = ["agent-y", "ancestor-wt", "main", "moved-on", "unmerged"]
        .map(String::from)
        .into();
    assert_eq!(branches(&f.repo), want);
    let want: BTreeSet<String> = ["main", "nolog-copy", "open-pr", "other"]
        .map(String::from)
        .into();
    assert_eq!(names_in(&review.join("run")), want);
    assert!(!f.repo.join("target/e2e").exists());
    assert!(!f.repo.join("app/dist").exists());
    assert_eq!(
        names_in(&f.repo.join("target/xtask-serve")),
        BTreeSet::new()
    );
    assert!(!f.scratch_free.exists());
    assert!(f.scratch_busy.join("b.txt").is_file());

    // The processes: the server stopped, and the others still run.
    let reaped = f.serve.as_ref().unwrap().reaped.clone();
    let until = Instant::now() + Duration::from_secs(5);
    while !reaped.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(Instant::now() < until, "xtask serve still runs");
        std::thread::sleep(Duration::from_millis(20));
    }
    let zombie = f.zombie;
    for child in f.children.iter_mut().filter(|c| c.pid() != zombie) {
        assert!(child.runs(), "pid {} stopped", child.pid());
    }

    // A second run finds nothing more to remove.
    let busy = f.scratch_busy.to_str().unwrap().to_string();
    let out = f.clean(&["--yes".to_string(), "--also".to_string(), busy]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);
    assert_eq!(stdout.lines().next(), Some("nothing to remove"), "{stdout}");
    assert!(!stdout.contains("removed "), "{stdout}");
    assert!(!stdout.contains("run `cargo"), "{stdout}");
}

#[test]
fn with_gh_failing_only_the_proofs_from_git_remove_branches() {
    let f = Fixture::empty("gh-fails");
    let repo = &f.repo;
    let start = git(repo, &["rev-parse", "HEAD"]);
    git(repo, &["switch", "-q", "-c", "squashed"]);
    write(&repo.join("s.txt"), "s");
    let squashed = commit(repo, "s");
    git(repo, &["switch", "-q", "-c", "merged-pr", &start]);
    write(&repo.join("q.txt"), "q");
    commit(repo, "q");
    git(repo, &["switch", "-q", "main"]);
    let tree = git(repo, &["rev-parse", &format!("{squashed}^{{tree}}")]);
    let squash = git(repo, &["commit-tree", &tree, "-p", &start, "-m", "squash"]);
    git(repo, &["update-ref", "refs/remotes/origin/main", &squash]);
    write(&repo.join("target/review/run/old/log.md"), "old log\n");
    f.fake_gh(&Gh::Fails);

    let out = f.clean(&["--yes".to_string()]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);
    let (_, kept) = parts(&stdout);
    assert_item(&kept, "branch merged-pr", "cannot read merged PRs");
    assert_item(
        &kept,
        "run-folder target/review/run/old",
        "cannot read open PRs",
    );
    let (removed, _) = actions(&stdout);
    assert_item(
        &removed,
        "branch squashed",
        "its tree is the tree of a commit on origin/main",
    );
    let want: BTreeSet<String> = ["main", "merged-pr"].map(String::from).into();
    assert_eq!(branches(repo), want);
    assert!(repo.join("target/review/run/old/log.md").is_file());
}

#[test]
fn yes_keeps_a_merged_worktree_that_holds_a_changed_worktree() {
    let f = Fixture::empty("nested");
    let repo = &f.repo;
    git(repo, &["switch", "-q", "-c", "agent-x"]);
    write(&repo.join("m1.txt"), "m1\n");
    let m1 = commit(repo, "m1");
    git(repo, &["switch", "-q", "main"]);
    git(repo, &["update-ref", "refs/remotes/origin/main", &m1]);
    git(
        repo,
        &[
            "worktree",
            "add",
            "-q",
            ".claude/worktrees/agent-x",
            "agent-x",
        ],
    );
    // A reviewer's worktree, made from inside the agent's worktree, under
    // its ignored `target/`.
    let agent = repo.join(".claude/worktrees/agent-x");
    git(
        &agent,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            "target/review/1-reviewer-1",
            "HEAD",
        ],
    );
    let nested = agent.join("target/review/1-reviewer-1/held.txt");
    write(&nested, "a change\n");
    f.fake_gh(&Gh::Prs {
        open: "[]".to_string(),
        merged: "[]".to_string(),
    });

    let out = f.clean(&["--yes".to_string()]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);
    let (_, kept) = parts(&stdout);
    assert_item(
        &kept,
        "worktree .claude/worktrees/agent-x",
        "holds a worktree that stays",
    );
    assert_item(
        &kept,
        "worktree .claude/worktrees/agent-x/target/review/1-reviewer-1",
        "has changes",
    );
    assert_item(&kept, "branch agent-x", "the current branch of a worktree");
    assert_eq!(
        std::fs::read_to_string(&nested).ok().as_deref(),
        Some("a change\n"),
        "{stdout}"
    );
    assert!(branches(repo).contains("agent-x"));
}
