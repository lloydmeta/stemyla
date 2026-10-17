//! `xtask publish`: build the web app for GitHub Pages and commit it to the
//! `gh-pages` branch, then push that branch.
//!
//! ```text
//! cargo run -p xtask --release -- publish [--public-url /stemyla/] [--remote origin]
//!                                         [--branch gh-pages] [--no-push] [--allow-dirty]
//!                                         [--skip-if-behind main]
//! ```
//!
//! Every build lives in its own folder, `v/<id>/`, which is written once
//! and never changed.  The CDN in front of the site may let browsers keep
//! `.js` files for a month but `.wasm` files for ten minutes, so a file
//! name reused across builds can pair one build's wasm-bindgen glue with
//! another's wasm.  Only the root `index.html` changes in place: it is the
//! newest build's page, and its `<base href>` points into that build's
//! folder.
//!
//! ```text
//! .nojekyll
//! index.html          the newest build's page
//! v/VERSIONS          the kept build ids, newest first
//! v/<id>/index.html   that build's page
//! v/<id>/app.js, app_bg.wasm, stem-worker*.{js,wasm}, snippets/…
//! ```
//!
//! Steps:
//! 1. Refuses to run with uncommitted changes (`--allow-dirty` overrides),
//!    so every published site maps to a commit.  The build id is the
//!    commit's 12-character hash, plus `-dirty-<unix seconds>` for a dirty
//!    tree.  With `--skip-if-behind <branch>` (used by CI), publishes HEAD
//!    only if it is that branch's tip on the remote, or a commit of the
//!    branch newer than the published build, so a re-run of an old CI run
//!    cannot roll the site back (see [`skip_reason`]).
//! 2. `trunk build --release --public-url <url>v/<id>/` into
//!    `target/pages/v/<id>`, and checks the result: the page, both wasm
//!    modules, the worker loader, and the base and asset paths under the
//!    build's URL.
//! 3. Fetches the branch from the remote if it exists there.  Then adds the
//!    newest [`KEPT_BUILDS`] builds: the new one, and the older ones from
//!    the branch, byte for byte.  A build id the branch has ever published
//!    keeps its published copy, even if it was pruned since (so the clone
//!    must not be shallow, and the branch's history must never be
//!    rewritten).  Older builds are dropped.  Writes the root
//!    `index.html`, `v/VERSIONS` and `.nojekyll`, and checks the new id's
//!    build again (it may be the published copy).
//! 4. Commits the result as the branch's whole content, with the previous
//!    publish as parent.  This uses git's plumbing (temporary indexes,
//!    `read-tree`, `checkout-index`, `write-tree`, `commit-tree`,
//!    `update-ref`), so the working tree, the index and the checked-out
//!    branch are never touched and no hooks run.  Nothing is committed when
//!    the result is identical to what is already published.
//! 5. `git push <remote> <branch>` unless `--no-push`.
//!
//! Only Cargo, Trunk and git are involved: no non-Rust build steps.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// How many builds `v/` keeps, the new one included.  The root page may be
/// cached for 600 s, so a browser can ask for the previous build's files
/// for ten minutes after a publish; three leaves room for two publishes
/// close together.
pub const KEPT_BUILDS: usize = 3;

pub struct Options {
    pub public_url: String,
    pub remote: String,
    pub branch: String,
    pub push: bool,
    pub allow_dirty: bool,
    /// Publish HEAD only if it is this branch's tip on the remote, or on it
    /// and newer than the published build (see [`skip_reason`]).
    pub skip_if_behind: Option<String>,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self> {
        let mut o = Options {
            public_url: "/stemyla/".into(),
            remote: "origin".into(),
            branch: "gh-pages".into(),
            push: true,
            allow_dirty: false,
            skip_if_behind: None,
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
                "--skip-if-behind" => o.skip_if_behind = Some(value("--skip-if-behind")?),
                other => bail!(
                    "unknown option {other}; usage: xtask publish [--public-url /stemyla/] [--remote origin] [--branch gh-pages] [--no-push] [--allow-dirty] [--skip-if-behind main]"
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
    let head = git(&repo, &["rev-parse", "HEAD"])?;
    // Before the build, so an unreachable remote fails in seconds, not
    // minutes; `deploy` fetches again, in case another clone published since.
    fetch_published(&repo, &o)?;
    if let Some(source) = &o.skip_if_behind
        && let Some(why) = skip_reason(&repo, &o, source, &head)?
    {
        println!("not publishing {head}: {why}");
        return Ok(());
    }
    let dirty_at = if dirty.is_empty() {
        None
    } else {
        Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
    };
    let id = build_id(&head, dirty_at)?;
    let build_url = format!("{}v/{id}/", o.public_url);

    let site = repo.join("target/pages");
    if site.exists() {
        std::fs::remove_dir_all(&site).with_context(|| format!("removing {}", site.display()))?;
    }
    let build = site.join("v").join(&id);
    println!("building the web app for {build_url} …");
    let build_s = build.to_string_lossy().to_string();
    // Trunk's own output is shown: the build takes a few minutes.
    let status = Command::new("trunk")
        .args([
            "build",
            "--release",
            "--public-url",
            &build_url,
            "--dist",
            &build_s,
        ])
        .current_dir(repo.join("app"))
        .status()
        .context("could not run trunk (install it with `cargo install --locked trunk`)")?;
    if !status.success() {
        bail!(
            "trunk build failed (the wasm target is added with `rustup target add wasm32-unknown-unknown`)"
        );
    }
    check_build(&build, &build_url)?;

    let message = format!("Publish {id} at {}", o.public_url);
    deploy(&repo, &site, &o, &id, &message)?;
    Ok(())
}

/// The commit `refs/heads/<branch>` points at on `remote`, or `None` if
/// the remote has no such branch.  Fails if the remote cannot be asked.
pub fn remote_head(repo: &Path, remote: &str, branch: &str) -> Result<Option<String>> {
    let full_ref = format!("refs/heads/{branch}");
    let listed = git(repo, &["ls-remote", remote, &full_ref])?;
    // "<object> TAB <ref>" per line; the pattern also matches longer refs
    // that end in it, such as refs/heads/old/<branch>.
    Ok(listed
        .lines()
        .find_map(|l| l.split_once('\t').filter(|(_, r)| *r == full_ref))
        .map(|(tip, _)| tip.to_string()))
}

/// Updates the local publish branch from the remote, if the remote has it,
/// so a branch published from another clone is built on, not over.
fn fetch_published(repo: &Path, o: &Options) -> Result<()> {
    let published = remote_head(repo, &o.remote, &o.branch).with_context(|| {
        format!(
            "could not reach {} (for an offline local publish, pass --remote . with --no-push)",
            o.remote
        )
    })?;
    if published.is_none() {
        return Ok(());
    }
    let refspec = format!("refs/heads/{0}:refs/heads/{0}", o.branch);
    git(repo, &["fetch", &o.remote, &refspec]).with_context(|| {
        format!("could not update the local {} from {} (has it diverged? compare with `git log {} {}/{}`)", o.branch, o.remote, o.branch, o.remote, o.branch)
    })?;
    Ok(())
}

/// Why `head` should not be published from CI, or `None` if it should.
/// A re-run of an old CI run builds its original commit, so `head` is not
/// always `source`'s newest.  It is published when it is `source`'s tip on
/// the remote (even after a force-push back to it), or when it is on
/// `source` and no published build is a newer commit of `source`.  Builds
/// of commits that are not on `source` (a preview published by hand) or
/// that this clone does not have are not counted.  So an old re-run cannot
/// roll the site back, and a commit is still published when the next one
/// never is (its run failed, or it skipped CI).  Needs the full history of
/// both branches.
pub fn skip_reason(repo: &Path, o: &Options, source: &str, head: &str) -> Result<Option<String>> {
    require_full_history(repo)?;
    // One snapshot of the tip, which may be newer than this clone.  An empty
    // --refmap leaves remote-tracking branches alone: only FETCH_HEAD moves.
    git(
        repo,
        &[
            "fetch",
            "--quiet",
            "--refmap=",
            &o.remote,
            &format!("refs/heads/{source}"),
        ],
    )
    .with_context(|| format!("could not fetch {source} from {}", o.remote))?;
    let tip = git(repo, &["rev-parse", "FETCH_HEAD^{commit}"])?;
    if tip == head {
        return Ok(None);
    }
    if !is_ancestor(repo, head, &tip)? {
        return Ok(Some(format!("it is not on {source} (now at {tip})")));
    }
    fetch_published(repo, o)?;
    let versions = match branch_commit(repo, &o.branch) {
        Some(p) if object_type(repo, &p, "v/VERSIONS")?.as_deref() == Some("blob") => {
            parse_versions(&git(repo, &["show", &format!("{p}:v/VERSIONS")])?)?
        }
        _ => Vec::new(),
    };
    let mut newest_on_source = None;
    for id in &versions {
        if let Some(c) = published_commit(repo, id)
            && is_ancestor(repo, &c, &tip)?
        {
            newest_on_source = Some(c);
            break;
        }
    }
    match newest_on_source {
        Some(p) if p != head && is_ancestor(repo, head, &p)? => Ok(Some(format!(
            "the site already has {p}, a newer commit of {source}"
        ))),
        _ => Ok(None),
    }
}

/// The commit a build id was built from, if this clone has it.
fn published_commit(repo: &Path, id: &str) -> Option<String> {
    let short = id.get(..12)?;
    git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{short}^{{commit}}"),
        ],
    )
    .ok()
}

/// Whether `a` is `b` or one of its ancestors.
fn is_ancestor(repo: &Path, a: &str, b: &str) -> Result<bool> {
    let status = Command::new("git")
        .args(["merge-base", "--is-ancestor", a, b])
        .current_dir(repo)
        .status()
        .context("could not run git")?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!("git merge-base --is-ancestor {a} {b} failed"),
    }
}

/// Fails in a shallow clone: write-once and the CI skip both look through
/// history that a shallow clone does not have.
fn require_full_history(repo: &Path) -> Result<()> {
    if git(repo, &["rev-parse", "--is-shallow-repository"])? == "true" {
        bail!(
            "this clone is shallow, so earlier publishes cannot be found in its history; fetch it in full (`git fetch --unshallow`, or `fetch-depth: 0` in actions/checkout)"
        );
    }
    Ok(())
}

/// Checks that `id` can name a folder under `v/`: non-empty, and only
/// lower-case letters, digits and `-`, so it can never be `..` or hold a
/// `/`.
pub fn check_id(id: &str) -> Result<()> {
    let allowed = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
    if id.is_empty() || !id.bytes().all(allowed) {
        bail!("{id:?} is not a build id (lower-case letters, digits and -)");
    }
    Ok(())
}

/// The id of a build of the commit `head` (a full hash): its first 12
/// characters, so that every clone, shallow or not, names a commit the same
/// way.  A build with uncommitted changes adds `-dirty-<unix seconds>`, so
/// two of them never share a folder.
pub fn build_id(head: &str, dirty_at: Option<u64>) -> Result<String> {
    let Some(short) = head.get(..12) else {
        bail!("{head:?} is not a commit hash");
    };
    let id = match dirty_at {
        None => short.to_string(),
        Some(secs) => format!("{short}-dirty-{secs}"),
    };
    check_id(&id)?;
    Ok(id)
}

/// The build ids in a `v/VERSIONS` file, newest first.  A line that is not
/// a build id is an error, never a path.
pub fn parse_versions(text: &str) -> Result<Vec<String>> {
    text.lines()
        .map(|line| {
            check_id(line).context("v/VERSIONS has a bad line")?;
            Ok(line.to_string())
        })
        .collect()
}

/// The builds to keep after publishing `new`: `new` first, then the
/// published ones in order, each once, at most [`KEPT_BUILDS`] in all.
pub fn kept_ids(new: &str, published: &[String]) -> Vec<String> {
    let mut kept = vec![new.to_string()];
    for id in published {
        if kept.len() == KEPT_BUILDS {
            break;
        }
        if !kept.contains(id) {
            kept.push(id.clone());
        }
    }
    kept
}

/// A build copied from the branch's history into the site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedBuild {
    pub id: String,
    /// A commit of the branch that holds `v/<id>/` as it was published.
    pub from: String,
}

/// What [`prepare_site`] found on the branch and did to the site.
#[derive(Debug)]
pub struct Prepared {
    /// The branch's commit the site builds on, if the branch exists.  The
    /// publish commit must have exactly this parent.
    pub parent: Option<String>,
    /// The builds now in `v/`, newest first, as written to `v/VERSIONS`.
    pub kept: Vec<String>,
    /// The builds copied from the branch, which must stay byte for byte.
    pub copied: Vec<CopiedBuild>,
    /// Whether the new id was published before (even if since pruned), so
    /// its published copy replaced the fresh build.
    pub reused: bool,
}

/// Turns `site`, which holds only the fresh build in `v/<id>/`, into the
/// branch's next tree: adds the kept older builds from the branch's current
/// commit, the root `index.html` (a copy of the newest build's),
/// `v/VERSIONS` and `.nojekyll`.
///
/// A published `v/<id>/` is never changed, because browsers may hold its
/// files for a month: if the branch's history has ever had `v/<id>/`, for
/// example when CI publishes the same commit again, that copy replaces the
/// fresh build, even if it was pruned since.
pub fn prepare_site(repo: &Path, site: &Path, branch: &str, id: &str) -> Result<Prepared> {
    check_id(id)?;
    if dir_names(site)? != ["v"] || dir_names(&site.join("v"))? != [id] {
        bail!(
            "{} should hold only the new build, in v/{id}/",
            site.display()
        );
    }
    let parent = branch_commit(repo, branch);
    let Some(p) = &parent else {
        return finish_site(site, id, None, vec![id.to_string()], Vec::new(), false);
    };
    require_full_history(repo)?;
    let published = match object_type(repo, p, "v/VERSIONS")?.as_deref() {
        None => Vec::new(),
        Some("blob") => parse_versions(&git(repo, &["show", &format!("{p}:v/VERSIONS")])?)?,
        Some(other) => bail!("{branch}:v/VERSIONS is a {other}, not a file"),
    };
    let earlier = published_in_history(repo, p, id)?;
    let kept = kept_ids(id, &published);
    let mut copied = Vec::new();
    for k in &kept {
        if k == id {
            if let Some(from) = &earlier {
                copied.push(CopiedBuild {
                    id: k.clone(),
                    from: from.clone(),
                });
            }
        } else if object_type(repo, p, &format!("v/{k}"))?.as_deref() == Some("tree") {
            copied.push(CopiedBuild {
                id: k.clone(),
                from: p.clone(),
            });
        } else {
            bail!("{branch} lists build {k} in v/VERSIONS but has no folder v/{k}/");
        }
    }
    let reused = earlier.is_some();
    if reused {
        let fresh = site.join("v").join(id);
        std::fs::remove_dir_all(&fresh).with_context(|| format!("removing {}", fresh.display()))?;
    }
    check_out_builds(repo, site, &copied)?;
    finish_site(site, id, parent, kept, copied, reused)
}

/// Writes the root `index.html`, `v/VERSIONS` and `.nojekyll` once `v/`
/// holds every kept build.
fn finish_site(
    site: &Path,
    id: &str,
    parent: Option<String>,
    kept: Vec<String>,
    copied: Vec<CopiedBuild>,
    reused: bool,
) -> Result<Prepared> {
    std::fs::copy(
        site.join("v").join(id).join("index.html"),
        site.join("index.html"),
    )
    .context("copying the newest build's index.html to the root")?;
    std::fs::write(site.join("v/VERSIONS"), format!("{}\n", kept.join("\n")))?;
    std::fs::write(site.join(".nojekyll"), b"")?;
    Ok(Prepared {
        parent,
        kept,
        copied,
        reused,
    })
}

/// The newest commit in `commit`'s history that holds `v/<id>/`, if any.
/// A folder is only ever added whole and later removed whole, so the
/// commits that touch it alternate between having it and not; the first
/// one that has it is its published state.
fn published_in_history(repo: &Path, commit: &str, id: &str) -> Result<Option<String>> {
    let path = format!("v/{id}");
    let touching = git(repo, &["rev-list", commit, "--", &path])?;
    for c in touching.lines() {
        if object_type(repo, c, &path)?.as_deref() == Some("tree") {
            return Ok(Some(c.to_string()));
        }
    }
    Ok(None)
}

/// Writes each build's `v/<id>/` from its commit into `site`, through a
/// temporary index, so neither the checkout nor its index changes and no
/// hooks run.
fn check_out_builds(repo: &Path, site: &Path, builds: &[CopiedBuild]) -> Result<()> {
    if builds.is_empty() {
        return Ok(());
    }
    with_temp_index(repo, "publish-kept-index", |_, index| {
        let envs: [(&str, &Path); 1] = [("GIT_INDEX_FILE", index)];
        for b in builds {
            run(
                repo,
                "git",
                &[
                    "read-tree",
                    &format!("--prefix=v/{}/", b.id),
                    &format!("{}:v/{}", b.from, b.id),
                ],
                &envs,
            )?;
        }
        let prefix = format!("--prefix={}/", site.display());
        run(repo, "git", &["checkout-index", "-a", "-f", &prefix], &envs)?;
        Ok(())
    })
}

/// Fails unless each build copied from the branch has the same tree in
/// `tree` as where it was copied from: proof that publishing left it as it
/// was published.
pub fn check_kept_builds(repo: &Path, tree: &str, prepared: &Prepared) -> Result<()> {
    for b in &prepared.copied {
        let before = git(repo, &["rev-parse", &format!("{}:v/{}", b.from, b.id)])?;
        let after = git(repo, &["rev-parse", &format!("{tree}:v/{}", b.id)])?;
        if before != after {
            bail!(
                "publishing would change the published build v/{}/ ({before} -> {after})",
                b.id
            );
        }
    }
    Ok(())
}

/// The names in `dir`, sorted.
fn dir_names(dir: &Path) -> Result<Vec<String>> {
    let mut names = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

/// The commit `branch` points at, if the branch exists.
fn branch_commit(repo: &Path, branch: &str) -> Option<String> {
    let full_ref = format!("refs/heads/{branch}");
    git(repo, &["rev-parse", "--verify", "--quiet", &full_ref]).ok()
}

/// The type (`blob`, `tree`) of `path` in `commit`, or `None` if it is not
/// there.  `ls-tree` prints nothing for a missing path and fails for a bad
/// commit, which `git show` would not tell apart.
fn object_type(repo: &Path, commit: &str, path: &str) -> Result<Option<String>> {
    // "<mode> SP <type> SP <object> TAB <path>"
    let line = git(repo, &["ls-tree", "--full-tree", commit, path])?;
    Ok(line.split_whitespace().nth(1).map(str::to_string))
}

/// Runs `f` with the git directory and a temporary index file in it, and
/// removes the index afterwards.
fn with_temp_index<T>(
    repo: &Path,
    name: &str,
    f: impl FnOnce(&Path, &Path) -> Result<T>,
) -> Result<T> {
    let git_dir = PathBuf::from(git(repo, &["rev-parse", "--absolute-git-dir"])?);
    let index = git_dir.join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_file(&index);
    let result = f(&git_dir, &index);
    let _ = std::fs::remove_file(&index);
    result
}

/// Commits `site` (holding the fresh build in `v/<id>/`) to the branch,
/// after fetching it from the remote if it is there and adding the kept
/// builds ([`prepare_site`]), and pushes it unless `o.push` is off.
/// Returns the new commit, or `None` when the branch already held this
/// content.
pub fn deploy(
    repo: &Path,
    site: &Path,
    o: &Options,
    id: &str,
    message: &str,
) -> Result<Option<String>> {
    fetch_published(repo, o)?;
    let prepared = prepare_site(repo, site, &o.branch, id)?;
    let reused_from = prepared
        .copied
        .iter()
        .find(|b| b.id == id)
        .map(|b| b.from.clone());
    if let Some(from) = &reused_from {
        println!(
            "{} published v/{id}/ in {from}; keeping that copy, not this build",
            o.branch
        );
    }
    // Also covers a reused copy, which may have been built for another URL.
    check_build(
        &site.join("v").join(id),
        &format!("{}v/{id}/", o.public_url),
    )
    .with_context(|| match &reused_from {
        Some(from) => format!("checking v/{id}/ as {} published it in {from}", o.branch),
        None => format!("checking v/{id}/"),
    })?;
    println!("builds in v/: {}", prepared.kept.join(", "));
    let tree = write_tree(repo, site)?;
    check_kept_builds(repo, &tree, &prepared)?;
    let commit = commit_tree(repo, &tree, &o.branch, message, prepared.parent.as_deref())?;
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

/// Checks one Trunk build, in `dir`, before it is published: the files the
/// page needs exist, and the page's base and assets are under `url`, the
/// build's own folder on the site.  The base matters as much as the
/// assets: the app starts the worker from a relative URL, which resolves
/// against it.
pub fn check_build(dir: &Path, url: &str) -> Result<()> {
    let need = [
        "index.html",
        "app.js",
        "app_bg.wasm",
        "stem-worker.js",
        "stem-worker_bg.wasm",
        "stem-worker_loader.js",
    ];
    for f in need {
        let p = dir.join(f);
        let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if len == 0 {
            bail!("the build is missing {} (or it is empty)", p.display());
        }
    }
    let html = std::fs::read_to_string(dir.join("index.html"))?;
    if !html.contains(&format!("<base href=\"{url}\"")) {
        bail!(
            "index.html has no <base href=\"{url}\">; the page would start the worker from the wrong folder"
        );
    }
    if !html.contains(&format!("{url}app.js")) {
        bail!(
            "index.html does not load app.js from {url}; the page would not find its files on the site"
        );
    }
    Ok(())
}

/// Commits the contents of `dir` as the whole tree of `branch` in `repo`,
/// with the branch's current commit (if any) as parent.  Returns the new
/// commit, or `None` when the tree is unchanged.  Leaves the working tree,
/// the index and HEAD alone.
pub fn commit_dir(repo: &Path, dir: &Path, branch: &str, message: &str) -> Result<Option<String>> {
    let tree = write_tree(repo, dir)?;
    let parent = branch_commit(repo, branch);
    commit_tree(repo, &tree, branch, message, parent.as_deref())
}

/// Writes the contents of `dir` to git's object store as a tree, through
/// a temporary index, and returns the tree.
fn write_tree(repo: &Path, dir: &Path) -> Result<String> {
    with_temp_index(repo, "publish-index", |git_dir, index| {
        // A throwaway index over `dir` as the work tree; `--force` so that
        // ignore rules (global or .git/info/exclude) cannot drop built files.
        let envs: [(&str, &Path); 3] = [
            ("GIT_DIR", git_dir),
            ("GIT_INDEX_FILE", index),
            ("GIT_WORK_TREE", dir),
        ];
        run(dir, "git", &["add", "--all", "--force", "."], &envs)?;
        run(dir, "git", &["write-tree"], &envs)
    })
}

/// Commits `tree` to `branch` with `parent` as the only parent (none for a
/// new branch).  Returns `None`, committing nothing, when `parent` already
/// has this tree.  Fails if the branch is no longer at `parent`.
fn commit_tree(
    repo: &Path,
    tree: &str,
    branch: &str,
    message: &str,
    parent: Option<&str>,
) -> Result<Option<String>> {
    if let Some(p) = parent
        && git(repo, &["rev-parse", &format!("{p}^{{tree}}")])? == tree
    {
        return Ok(None);
    }
    let mut args = vec!["commit-tree", tree, "-m", message];
    if let Some(p) = parent {
        args.extend(["-p", p]);
    }
    let commit = git(repo, &args)?;
    // Only moves the branch if it is still where it was read.
    let full_ref = format!("refs/heads/{branch}");
    let old = parent.unwrap_or_default(); // empty: must not exist yet
    git(repo, &["update-ref", &full_ref, &commit, old])?;
    Ok(Some(commit))
}
