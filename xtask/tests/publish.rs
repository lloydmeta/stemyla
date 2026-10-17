//! `xtask publish`: the gh-pages commit and push, against a throwaway
//! repository and a bare "remote" (no Trunk build involved).

use std::path::{Path, PathBuf};
use std::process::Command;

use xtask::publish::{
    KEPT_BUILDS, Options, Prepared, build_id, check_build, check_id, check_kept_builds, commit_dir,
    deploy, kept_ids, parse_versions, prepare_site,
};

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
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A committer identity for the git commands `xtask::publish` runs (every
/// test sets the same values, so running them in parallel is harmless).
fn identity() {
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "t"),
        ("GIT_AUTHOR_EMAIL", "t@example.com"),
        ("GIT_COMMITTER_NAME", "t"),
        ("GIT_COMMITTER_EMAIL", "t@example.com"),
    ] {
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
    std::fs::write(
        dir.join("index.html"),
        format!("<script src=\"/stemyla/app.js\"></script>{version}"),
    )
    .unwrap();
    for f in [
        "app.js",
        "app_bg.wasm",
        "stem-worker.js",
        "stem-worker_bg.wasm",
        "stem-worker_loader.js",
    ] {
        std::fs::write(dir.join(f), version).unwrap();
    }
    std::fs::write(dir.join("snippets/x/inline0.js"), version).unwrap();
    std::fs::write(dir.join(".nojekyll"), "").unwrap();
}

/// A fresh site directory as `xtask publish` hands it to `deploy`: only
/// one Trunk build, in `v/<id>/`, built for `/stemyla/v/<id>/`.
fn versioned_site(root: &Path, id: &str, version: &str) -> PathBuf {
    let site = root.join(format!("site-{id}-{version}"));
    let _ = std::fs::remove_dir_all(&site);
    let build = site.join("v").join(id);
    std::fs::create_dir_all(build.join("snippets/x")).unwrap();
    std::fs::write(
        build.join("index.html"),
        format!(
            "<base href=\"/stemyla/v/{id}/\" /><script src=\"/stemyla/v/{id}/app.js\"></script>{version}"
        ),
    )
    .unwrap();
    for f in [
        "app.js",
        "app_bg.wasm",
        "stem-worker.js",
        "stem-worker_bg.wasm",
        "stem-worker_loader.js",
    ] {
        std::fs::write(build.join(f), format!("{id} {version}")).unwrap();
    }
    std::fs::write(build.join("snippets/x/inline0.js"), version).unwrap();
    site
}

fn ls(r: &Path, rev: &str, path: &str) -> Vec<String> {
    let mut names: Vec<String> = git(r, &["ls-tree", "--name-only", rev, path])
        .lines()
        .map(str::to_string)
        .collect();
    names.sort();
    names
}

fn no_push() -> Options {
    // No remote named origin exists in these repositories, so `deploy`
    // fetches nothing.
    Options::parse(&["--no-push".into()]).unwrap()
}

#[test]
fn commits_the_site_without_touching_the_checkout() {
    let root = scratch("commit");
    let r = repo(&root);
    let dist = root.join("dist");
    site(&dist, "v1");
    let before = (
        git(&r, &["status", "--porcelain"]),
        git(&r, &["rev-parse", "HEAD"]),
        git(&r, &["symbolic-ref", "HEAD"]),
    );

    identity();
    let c1 = commit_dir(&r, &dist, "gh-pages", "Publish 1")
        .unwrap()
        .expect("first commit");
    let files = git(&r, &["ls-tree", "-r", "--name-only", "gh-pages"]);
    let mut files: Vec<&str> = files.lines().collect();
    files.sort();
    assert_eq!(
        files,
        [
            ".nojekyll",
            "app.js",
            "app_bg.wasm",
            "index.html",
            "snippets/x/inline0.js",
            "stem-worker.js",
            "stem-worker_bg.wasm",
            "stem-worker_loader.js"
        ]
    );
    assert_eq!(git(&r, &["show", "gh-pages:app_bg.wasm"]), "v1");
    assert_eq!(
        git(&r, &["rev-list", "--count", "gh-pages"]),
        "1",
        "no parent on the first publish"
    );
    let after = (
        git(&r, &["status", "--porcelain"]),
        git(&r, &["rev-parse", "HEAD"]),
        git(&r, &["symbolic-ref", "HEAD"]),
    );
    assert_eq!(before, after, "working tree, index or HEAD changed");

    // Same content: nothing new.
    assert_eq!(
        commit_dir(&r, &dist, "gh-pages", "Publish 1 again").unwrap(),
        None
    );
    // New content, a file removed: a child commit with exactly the new files.
    std::fs::remove_file(dist.join("snippets/x/inline0.js")).unwrap();
    site(&dist, "v2");
    std::fs::remove_file(dist.join("snippets/x/inline0.js")).unwrap();
    let c2 = commit_dir(&r, &dist, "gh-pages", "Publish 2")
        .unwrap()
        .expect("second commit");
    assert_eq!(git(&r, &["rev-parse", &format!("{c2}^")]), c1);
    assert!(!git(&r, &["ls-tree", "-r", "--name-only", "gh-pages"]).contains("inline0.js"));
    assert_eq!(
        git(&r, &["log", "-1", "--format=%s", "gh-pages"]),
        "Publish 2"
    );
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
    let s1 = versioned_site(&root, "aaaaaaaaaaa1", "v1");
    let c1 = deploy(&r, &s1, &o, "aaaaaaaaaaa1", "Publish 1")
        .unwrap()
        .unwrap();
    assert_eq!(git(&remote, &["rev-parse", "gh-pages"]), c1);

    // A second clone without a local gh-pages publishes on top of the
    // remote's, not over it, and keeps the remote's build.
    let other = root.join("other");
    git(
        &root,
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    git(&other, &["checkout", "-q", "-b", "main"]);
    let s2 = versioned_site(&root, "aaaaaaaaaaa2", "v2");
    let c2 = deploy(&other, &s2, &o, "aaaaaaaaaaa2", "Publish 2")
        .unwrap()
        .unwrap();
    assert_eq!(git(&remote, &["rev-parse", "gh-pages"]), c2);
    assert_eq!(git(&remote, &["rev-parse", &format!("{c2}^")]), c1);
    assert_eq!(
        git(&remote, &["show", "gh-pages:v/VERSIONS"]),
        "aaaaaaaaaaa2\naaaaaaaaaaa1"
    );

    // --no-push commits locally only.
    let s3 = versioned_site(&root, "aaaaaaaaaaa3", "v3");
    let c3 = deploy(&r, &s3, &no_push(), "aaaaaaaaaaa3", "Publish 3")
        .unwrap()
        .unwrap();
    assert_eq!(git(&remote, &["rev-parse", "gh-pages"]), c2);
    assert_eq!(
        git(&r, &["rev-parse", &format!("{c3}^")]),
        c2,
        "fetched the other clone's publish first"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Four publishes on top of the old flat layout: each leaves the newest
/// three builds, newest first, with the root page from the newest.
#[test]
fn keeps_the_newest_three_builds() {
    let root = scratch("keep");
    identity();
    let r = repo(&root);
    let flat = root.join("flat");
    site(&flat, "flat");
    let mut parent = commit_dir(&r, &flat, "gh-pages", "Publish flat")
        .unwrap()
        .unwrap();
    let ids = [
        "0123456789ab",
        "bbbbbbbbbbbb",
        "cccccccccccc",
        "dddddddddddd",
    ];
    for (n, id) in ids.iter().enumerate() {
        let site = versioned_site(&root, id, "v1");
        let fresh_page = std::fs::read(site.join("v").join(id).join("index.html")).unwrap();
        let c = deploy(&r, &site, &no_push(), id, &format!("Publish {id}"))
            .unwrap()
            .unwrap();
        assert_eq!(git(&r, &["rev-parse", &format!("{c}^")]), parent);

        let newest_first: Vec<&str> = ids[..=n].iter().rev().take(KEPT_BUILDS).copied().collect();
        assert_eq!(
            git(&r, &["show", "gh-pages:v/VERSIONS"]),
            newest_first.join("\n")
        );
        let mut folders: Vec<String> = newest_first.iter().map(|k| format!("v/{k}")).collect();
        folders.push("v/VERSIONS".into());
        folders.sort();
        assert_eq!(ls(&r, "gh-pages", "v/"), folders);
        assert_eq!(
            ls(&r, "gh-pages", "v/0123456789ab/").len(),
            if n < KEPT_BUILDS { 7 } else { 0 },
            "the oldest build is dropped only once three newer ones exist"
        );
        assert_eq!(ls(&r, "gh-pages", "."), [".nojekyll", "index.html", "v"]);
        assert_eq!(
            git(&r, &["show", "gh-pages:index.html"]).as_bytes(),
            fresh_page.as_slice(),
            "the root page is the newest build's"
        );
        assert_eq!(
            git(&r, &["show", &format!("gh-pages:v/{id}/index.html")]).as_bytes(),
            fresh_page.as_slice()
        );
        if n > 0 {
            assert_eq!(
                git(
                    &r,
                    &["diff", "--name-only", &parent, &c, "--", ".", ":(exclude)v"]
                ),
                "index.html",
                "only the root page changes outside v/"
            );
            let previous = ids[n - 1];
            assert_eq!(
                git(&r, &["rev-parse", &format!("{c}:v/{previous}")]),
                git(&r, &["rev-parse", &format!("{parent}:v/{previous}")]),
                "a kept build is unchanged"
            );
        }
        parent = c;
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A build id the branch already has keeps its published files, whatever
/// the new build holds, and becomes the root page again.
#[test]
fn a_published_build_is_never_rewritten() {
    let root = scratch("once");
    identity();
    let r = repo(&root);
    let (a, b) = ("aaaaaaaaaaaa", "bbbbbbbbbbbb");
    let a1 = versioned_site(&root, a, "first");
    let a_page = std::fs::read_to_string(a1.join("v").join(a).join("index.html")).unwrap();
    deploy(&r, &a1, &no_push(), a, "Publish a")
        .unwrap()
        .unwrap();
    let a_tree = git(&r, &["rev-parse", &format!("gh-pages:v/{a}")]);

    // The same id again, straight away: nothing changes.
    let a2 = versioned_site(&root, a, "second");
    assert_eq!(
        deploy(&r, &a2, &no_push(), a, "Publish a again").unwrap(),
        None
    );

    // After another build, the same id once more: its old files return.
    deploy(
        &r,
        &versioned_site(&root, b, "first"),
        &no_push(),
        b,
        "Publish b",
    )
    .unwrap()
    .unwrap();
    let a3 = versioned_site(&root, a, "third");
    deploy(&r, &a3, &no_push(), a, "Publish a once more")
        .unwrap()
        .unwrap();
    assert_eq!(git(&r, &["rev-parse", &format!("gh-pages:v/{a}")]), a_tree);
    assert_eq!(git(&r, &["show", "gh-pages:index.html"]), a_page);
    assert_eq!(
        git(&r, &["show", "gh-pages:v/VERSIONS"]),
        format!("{a}\n{b}")
    );
    assert_eq!(
        std::fs::read_to_string(a3.join("v").join(a).join("app.js")).unwrap(),
        format!("{a} first"),
        "the site on disk matches what was committed"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn refuses_a_branch_it_cannot_trust() {
    let root = scratch("refuse");
    identity();
    let r = repo(&root);
    let id = "aaaaaaaaaaaa";

    // A VERSIONS line that would be a path outside v/.
    let bad = root.join("bad");
    std::fs::create_dir_all(bad.join("v")).unwrap();
    std::fs::write(bad.join("v/VERSIONS"), "bbbbbbbbbbbb\n..\n").unwrap();
    commit_dir(&r, &bad, "bad-line", "bad").unwrap().unwrap();
    let err = prepare_site(&r, &versioned_site(&root, id, "v1"), "bad-line", id).unwrap_err();
    assert!(format!("{err:#}").contains("bad line"), "{err:#}");

    // A VERSIONS line without its folder.
    std::fs::write(bad.join("v/VERSIONS"), "bbbbbbbbbbbb\n").unwrap();
    commit_dir(&r, &bad, "no-folder", "no folder")
        .unwrap()
        .unwrap();
    let err = prepare_site(&r, &versioned_site(&root, id, "v1"), "no-folder", id).unwrap_err();
    assert!(
        err.to_string().contains("no folder v/bbbbbbbbbbbb/"),
        "{err}"
    );

    // A site with more than the one fresh build.
    let site = versioned_site(&root, id, "v1");
    std::fs::write(site.join("stray.txt"), "x").unwrap();
    assert!(prepare_site(&r, &site, "new-branch", id).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

/// The final check catches a kept build whose files would change.
#[test]
fn notices_a_changed_kept_build() {
    let root = scratch("changed");
    identity();
    let r = repo(&root);
    let id = "aaaaaaaaaaaa";
    let c = deploy(&r, &versioned_site(&root, id, "v1"), &no_push(), id, "p")
        .unwrap()
        .unwrap();
    let tampered = versioned_site(&root, id, "v1");
    std::fs::write(tampered.join("v").join(id).join("app.js"), "changed").unwrap();
    commit_dir(&r, &tampered, "tampered", "t").unwrap().unwrap();
    let tree = git(&r, &["rev-parse", "tampered^{tree}"]);
    let prepared = Prepared {
        parent: Some(c.clone()),
        kept: vec![id.into()],
        from_branch: vec![id.into()],
        reused: true,
    };
    let err = check_kept_builds(&r, &tree, &prepared).unwrap_err();
    assert!(err.to_string().contains("v/aaaaaaaaaaaa/"), "{err}");
    let same = git(&r, &["rev-parse", &format!("{c}^{{tree}}")]);
    check_kept_builds(&r, &same, &prepared).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn build_ids_and_the_kept_list() {
    let head = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(build_id(head, None).unwrap(), "0123456789ab");
    assert_eq!(
        build_id(head, Some(1_792_195_200)).unwrap(),
        "0123456789ab-dirty-1792195200"
    );
    assert!(build_id("abc", None).is_err());
    for bad in ["", "/", "..", "a/b", "ABC123", "abc.def", "abc\n"] {
        assert!(check_id(bad).is_err(), "{bad:?}");
    }

    assert_eq!(parse_versions("").unwrap(), Vec::<String>::new());
    assert_eq!(parse_versions("b\na\n").unwrap(), ["b", "a"]);
    assert!(parse_versions("b\n../x\n").is_err());
    assert!(parse_versions("b\n\na\n").is_err(), "a blank line");

    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(kept_ids("n", &[]), ["n"]);
    assert_eq!(kept_ids("n", &ids(&["c", "b", "a"])), ["n", "c", "b"]);
    assert_eq!(kept_ids("n", &ids(&["c", "c", "b"])), ["n", "c", "b"]);
    assert_eq!(kept_ids("b", &ids(&["c", "b", "a"])), ["b", "c", "a"]);
    assert_eq!(kept_ids("c", &ids(&["c", "b", "a"])), ["c", "b", "a"]);
}

#[test]
fn checks_the_build() {
    let root = scratch("check");
    let id = "aaaaaaaaaaaa";
    let build = versioned_site(&root, id, "v1").join("v").join(id);
    let url = format!("/stemyla/v/{id}/");
    check_build(&build, &url).unwrap();
    assert!(check_build(&build, "/stemyla/").is_err(), "the site's URL");
    assert!(
        check_build(&build, "/stemyla/v/bbbbbbbbbbbb/").is_err(),
        "another build's URL"
    );

    let page = build.join("index.html");
    let html = std::fs::read_to_string(&page).unwrap();
    std::fs::write(&page, html.replace(&format!("<base href=\"{url}\" />"), "")).unwrap();
    let err = check_build(&build, &url).unwrap_err();
    assert!(err.to_string().contains("<base href"), "{err}");
    std::fs::write(
        &page,
        html.replace(&format!("{url}app.js"), "/stemyla/app.js"),
    )
    .unwrap();
    let err = check_build(&build, &url).unwrap_err();
    assert!(err.to_string().contains("app.js"), "{err}");
    std::fs::write(&page, &html).unwrap();

    std::fs::remove_file(build.join("stem-worker_bg.wasm")).unwrap();
    assert!(
        check_build(&build, &url)
            .unwrap_err()
            .to_string()
            .contains("stem-worker_bg.wasm")
    );
    let _ = std::fs::remove_dir_all(&root);

    let o = Options::parse(&[
        "--public-url".into(),
        "pages".into(),
        "--remote".into(),
        "up".into(),
    ])
    .unwrap();
    assert_eq!(
        (o.public_url.as_str(), o.remote.as_str(), o.push),
        ("/pages/", "up", true)
    );
    assert!(Options::parse(&["--bogus".into()]).is_err());
}
