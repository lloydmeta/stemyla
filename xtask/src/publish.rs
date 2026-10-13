//! `xtask publish`: build the web app for GitHub Pages and commit it to the
//! `gh-pages` branch, then push that branch.
//!
//! ```text
//! cargo run -p xtask --release -- publish [--public-url /stemyla/] [--remote origin]
//!                                         [--branch gh-pages] [--no-push] [--allow-dirty]
//! ```
//!
//! Steps:
//! 1. Refuses to run with uncommitted changes (`--allow-dirty` overrides),
//!    so every published site maps to a commit, named in the publish
//!    commit's message.
//! 2. `trunk build --release --public-url <url>` into `target/pages`, and
//!    checks the result: the page, both wasm modules, the worker loader, and
//!    asset paths under the public URL.  Adds `.nojekyll`.
//! 3. Fetches the branch from the remote if it exists there, then commits
//!    the build as the branch's whole content, with the previous publish as
//!    parent.  This uses git's plumbing (a temporary index, `write-tree`,
//!    `commit-tree`, `update-ref`), so the working tree, the index and the
//!    checked-out branch are never touched.  Nothing is committed when the
//!    build is identical to what is already published.
//! 4. `git push <remote> <branch>` unless `--no-push`.
//!
//! Only Cargo, Trunk and git are involved: no non-Rust build steps.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Options {
    pub public_url: String,
    pub remote: String,
    pub branch: String,
    pub push: bool,
    pub allow_dirty: bool,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self> {
        let mut o = Options {
            public_url: "/stemyla/".into(),
            remote: "origin".into(),
            branch: "gh-pages".into(),
            push: true,
            allow_dirty: false,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = |name: &str| {
                it.next()
                    .cloned()
                    .with_context(|| format!("{name} needs a value"))
            };
            match a.as_str() {
                "--public-url" => o.public_url = value("--public-url")?,
                "--remote" => o.remote = value("--remote")?,
                "--branch" => o.branch = value("--branch")?,
                "--no-push" => o.push = false,
                "--allow-dirty" => o.allow_dirty = true,
                other => bail!(
                    "unknown option {other}; usage: xtask publish [--public-url /stemyla/] [--remote origin] [--branch gh-pages] [--no-push] [--allow-dirty]"
                ),
            }
        }
        if !o.public_url.starts_with('/') {
            o.public_url.insert(0, '/');
        }
        if !o.public_url.ends_with('/') {
            o.public_url.push('/');
        }
        Ok(o)
    }
}

/// Runs a command in `dir` and returns its trimmed stdout, or fails with
/// its stderr.
fn run(dir: &Path, program: &str, args: &[&str], envs: &[(&str, &Path)]) -> Result<String> {
    let mut c = Command::new(program);
    c.args(args).current_dir(dir);
    for (k, v) in envs {
        c.env(k, v);
    }
    let out = c
        .output()
        .with_context(|| format!("could not run {program} (is it installed?)"))?;
    if !out.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    run(repo, "git", args, &[])
}

pub fn main(args: &[String]) -> Result<()> {
    let o = Options::parse(args)?;
    let repo = PathBuf::from(git(Path::new("."), &["rev-parse", "--show-toplevel"])?);
    let dirty = git(&repo, &["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.is_empty() && !o.allow_dirty {
        bail!(
            "uncommitted changes; commit them first so the site matches a commit (or pass --allow-dirty):\n{dirty}"
        );
    }
    let source = git(&repo, &["rev-parse", "--short", "HEAD"])?;

    let dist = repo.join("target/pages");
    if dist.exists() {
        std::fs::remove_dir_all(&dist).with_context(|| format!("removing {}", dist.display()))?;
    }
    println!("building the web app for {} …", o.public_url);
    let dist_s = dist.to_string_lossy().to_string();
    // Trunk's own output is shown: the build takes a few minutes.
    let status = Command::new("trunk")
        .args([
            "build",
            "--release",
            "--public-url",
            &o.public_url,
            "--dist",
            &dist_s,
        ])
        .current_dir(repo.join("app"))
        .status()
        .context("could not run trunk (install it with `cargo install --locked trunk`)")?;
    if !status.success() {
        bail!(
            "trunk build failed (the wasm target is added with `rustup target add wasm32-unknown-unknown`)"
        );
    }
    check_build(&dist, &o.public_url)?;
    std::fs::write(dist.join(".nojekyll"), b"")?;

    let dirty_note = if dirty.is_empty() {
        ""
    } else {
        " plus uncommitted changes"
    };
    let message = format!("Publish {source}{dirty_note} at {}", o.public_url);
    deploy(&repo, &dist, &o, &message)?;
    Ok(())
}

/// Commits `dist` to the branch (after fetching it from the remote, if it
/// is there) and pushes it unless `o.push` is off.  Returns the new commit,
/// or `None` when the branch already held this content.
pub fn deploy(repo: &Path, dist: &Path, o: &Options, message: &str) -> Result<Option<String>> {
    // A branch published from another clone: start from it, not over it.
    let refspec = format!("refs/heads/{0}:refs/heads/{0}", o.branch);
    if git(
        repo,
        &["ls-remote", "--exit-code", "--heads", &o.remote, &o.branch],
    )
    .is_ok()
    {
        git(repo, &["fetch", &o.remote, &refspec]).with_context(|| {
            format!("could not update the local {} from {} (has it diverged? compare with `git log {} {}/{}`)", o.branch, o.remote, o.branch, o.remote, o.branch)
        })?;
    }
    let commit = commit_dir(repo, dist, &o.branch, message)?;
    match &commit {
        Some(c) => println!("committed {c} to {}: {message}", o.branch),
        None => println!("{} already holds this build; nothing to commit", o.branch),
    }
    if o.push {
        git(
            repo,
            &[
                "push",
                &o.remote,
                &format!("refs/heads/{0}:refs/heads/{0}", o.branch),
            ],
        )?;
        println!("pushed {} to {}", o.branch, o.remote);
    } else {
        println!(
            "not pushed (--no-push); push later with: git push {} {}",
            o.remote, o.branch
        );
    }
    Ok(commit)
}

/// Checks a Trunk build before it is published: the files the page needs
/// exist and the page loads them from under `public_url`.
pub fn check_build(dist: &Path, public_url: &str) -> Result<()> {
    let need = [
        "index.html",
        "app.js",
        "app_bg.wasm",
        "stem-worker.js",
        "stem-worker_bg.wasm",
        "stem-worker_loader.js",
    ];
    for f in need {
        let p = dist.join(f);
        let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if len == 0 {
            bail!("the build is missing {} (or it is empty)", p.display());
        }
    }
    let html = std::fs::read_to_string(dist.join("index.html"))?;
    if !html.contains(&format!("{public_url}app.js")) {
        bail!(
            "index.html does not load app.js from {public_url}; the page would not find its files on the site"
        );
    }
    Ok(())
}

/// Commits the contents of `dir` as the whole tree of `branch` in `repo`,
/// with the branch's current commit (if any) as parent.  Returns the new
/// commit, or `None` when the tree is unchanged.  Leaves the working tree,
/// the index and HEAD alone.
pub fn commit_dir(repo: &Path, dir: &Path, branch: &str, message: &str) -> Result<Option<String>> {
    let git_dir = PathBuf::from(git(repo, &["rev-parse", "--absolute-git-dir"])?);
    let index = git_dir.join(format!("publish-index-{}", std::process::id()));
    let _ = std::fs::remove_file(&index);
    // A throwaway index over `dir` as the work tree; `--force` so that
    // ignore rules (global or .git/info/exclude) cannot drop built files.
    let envs: [(&str, &Path); 3] = [
        ("GIT_DIR", &git_dir),
        ("GIT_INDEX_FILE", &index),
        ("GIT_WORK_TREE", dir),
    ];
    let tree = run(dir, "git", &["add", "--all", "--force", "."], &envs)
        .and_then(|_| run(dir, "git", &["write-tree"], &envs));
    let _ = std::fs::remove_file(&index);
    let tree = tree?;
    let full_ref = format!("refs/heads/{branch}");
    let parent = git(repo, &["rev-parse", "--verify", "--quiet", &full_ref]).ok();
    if let Some(p) = &parent
        && git(repo, &["rev-parse", &format!("{p}^{{tree}}")])? == tree
    {
        return Ok(None);
    }
    let mut args = vec!["commit-tree", tree.as_str(), "-m", message];
    if let Some(p) = &parent {
        args.extend(["-p", p.as_str()]);
    }
    let commit = git(repo, &args)?;
    // Only moves the branch if it is still where we read it.
    let old = parent.clone().unwrap_or_default(); // empty: must not exist yet
    git(repo, &["update-ref", &full_ref, &commit, &old])?;
    Ok(Some(commit))
}
