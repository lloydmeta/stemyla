//! `xtask publish`: the gh-pages commit and push, against a throwaway
//! repository and a bare "remote" (no Trunk build involved).

use std::path::{Path, PathBuf};
use std::process::Command;

use xtask::publish::{Options, check_build, commit_dir, deploy};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A committer identity for the git commands `xtask::publish` runs (every
/// test sets the same values, so running them in parallel is harmless).
fn identity() {
    for (k, v) in [("GIT_AUTHOR_NAME", "t"), ("GIT_AUTHOR_EMAIL", "t@example.com"), ("GIT_COMMITTER_NAME", "t"), ("GIT_COMMITTER_EMAIL", "t@example.com")] {
        // SAFETY: tests only read these through child processes.
        unsafe { std::env::set_var(k, v) };
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xtask-publish-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A repository with one commit on `main`, an uncommitted change and a
/// staged one, so the test can check that publishing leaves them alone.
fn repo(root: &Path) -> PathBuf {
    let r = root.join("repo");
    std::fs::create_dir_all(&r).unwrap();
    git(&r, &["init", "-q", "-b", "main"]);
    std::fs::write(r.join("README.md"), "source\n").unwrap();
    std::fs::write(r.join("staged.txt"), "a\n").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-q", "-m", "source"]);
    std::fs::write(r.join("staged.txt"), "b\n").unwrap();
    git(&r, &["add", "staged.txt"]);
    std::fs::write(r.join("README.md"), "edited\n").unwrap();
    // A global-style ignore rule that would drop built files without --force.
    std::fs::write(r.join(".git/info/exclude"), "*.wasm\n").unwrap();
    r
}

fn site(dir: &Path, version: &str) {
    std::fs::create_dir_all(dir.join("snippets/x")).unwrap();
    std::fs::write(dir.join("index.html"), format!("<script src=\"/stemyla/app.js\"></script>{version}")).unwrap();
    for f in ["app.js", "app_bg.wasm", "stem-worker.js", "stem-worker_bg.wasm", "stem-worker_loader.js"] {
        std::fs::write(dir.join(f), version).unwrap();
    }
    std::fs::write(dir.join("snippets/x/inline0.js"), version).unwrap();
    std::fs::write(dir.join(".nojekyll"), "").unwrap();
}

#[test]
fn commits_the_site_without_touching_the_checkout() {
    let root = scratch("commit");
    let r = repo(&root);
    let dist = root.join("dist");
    site(&dist, "v1");
    let before = (git(&r, &["status", "--porcelain"]), git(&r, &["rev-parse", "HEAD"]), git(&r, &["symbolic-ref", "HEAD"]));

    identity();
    let c1 = commit_dir(&r, &dist, "gh-pages", "Publish 1").unwrap().expect("first commit");
    let files = git(&r, &["ls-tree", "-r", "--name-only", "gh-pages"]);
    let mut files: Vec<&str> = files.lines().collect();
    files.sort();
    assert_eq!(
        files,
        [".nojekyll", "app.js", "app_bg.wasm", "index.html", "snippets/x/inline0.js", "stem-worker.js", "stem-worker_bg.wasm", "stem-worker_loader.js"]
    );
    assert_eq!(git(&r, &["show", "gh-pages:app_bg.wasm"]), "v1");
    assert_eq!(git(&r, &["rev-list", "--count", "gh-pages"]), "1", "no parent on the first publish");
    let after = (git(&r, &["status", "--porcelain"]), git(&r, &["rev-parse", "HEAD"]), git(&r, &["symbolic-ref", "HEAD"]));
    assert_eq!(before, after, "working tree, index or HEAD changed");

    // Same content: nothing new.
    assert_eq!(commit_dir(&r, &dist, "gh-pages", "Publish 1 again").unwrap(), None);
    // New content, a file removed: a child commit with exactly the new files.
    std::fs::remove_file(dist.join("snippets/x/inline0.js")).unwrap();
    site(&dist, "v2");
    std::fs::remove_file(dist.join("snippets/x/inline0.js")).unwrap();
    let c2 = commit_dir(&r, &dist, "gh-pages", "Publish 2").unwrap().expect("second commit");
    assert_eq!(git(&r, &["rev-parse", &format!("{c2}^")]), c1);
    assert!(!git(&r, &["ls-tree", "-r", "--name-only", "gh-pages"]).contains("inline0.js"));
    assert_eq!(git(&r, &["log", "-1", "--format=%s", "gh-pages"]), "Publish 2");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deploys_to_a_remote_and_builds_on_its_branch() {
    let root = scratch("deploy");
    identity();
    let remote = root.join("remote.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare"]);
    let r = repo(&root);
    git(&r, &["remote", "add", "origin", remote.to_str().unwrap()]);
    let o = Options::parse(&[]).unwrap();
    let dist = root.join("dist");
    site(&dist, "v1");
    let c1 = deploy(&r, &dist, &o, "Publish 1").unwrap().unwrap();
    assert_eq!(git(&remote, &["rev-parse", "gh-pages"]), c1);

    // A second clone without a local gh-pages publishes on top of the
    // remote's, not over it.
    let other = root.join("other");
    git(&root, &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
    git(&other, &["checkout", "-q", "-b", "main"]);
    site(&dist, "v2");
    let c2 = deploy(&other, &dist, &o, "Publish 2").unwrap().unwrap();
    assert_eq!(git(&remote, &["rev-parse", "gh-pages"]), c2);
    assert_eq!(git(&remote, &["rev-parse", &format!("{c2}^")]), c1);

    // --no-push commits locally only.
    site(&dist, "v3");
    let o3 = Options::parse(&["--no-push".into()]).unwrap();
    let c3 = deploy(&r, &dist, &o3, "Publish 3").unwrap().unwrap();
    assert_eq!(git(&remote, &["rev-parse", "gh-pages"]), c2);
    assert_eq!(git(&r, &["rev-parse", &format!("{c3}^")]), c2, "fetched the other clone's publish first");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn checks_the_build() {
    let root = scratch("check");
    let dist = root.join("dist");
    site(&dist, "v1");
    check_build(&dist, "/stemyla/").unwrap();
    assert!(check_build(&dist, "/other/").is_err(), "wrong public URL");
    std::fs::remove_file(dist.join("stem-worker_bg.wasm")).unwrap();
    assert!(check_build(&dist, "/stemyla/").unwrap_err().to_string().contains("stem-worker_bg.wasm"));
    let _ = std::fs::remove_dir_all(&root);

    let o = Options::parse(&["--public-url".into(), "pages".into(), "--remote".into(), "up".into()]).unwrap();
    assert_eq!((o.public_url.as_str(), o.remote.as_str(), o.push), ("/pages/", "up", true));
    assert!(Options::parse(&["--bogus".into()]).is_err());
}
