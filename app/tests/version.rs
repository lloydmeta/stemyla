//! The About menu's version line, from the build id `xtask publish` sets.

use app::build_version;

#[test]
fn a_published_build_links_to_its_commit() {
    assert_eq!(
        build_version(Some("0123456789ab")),
        (
            "Version: 0123456789ab".to_string(),
            Some("https://github.com/lloydmeta/stemyla/commit/0123456789ab".to_string())
        )
    );
}

#[test]
fn a_dirty_build_links_to_the_commit_it_started_from() {
    let (label, commit) = build_version(Some("0123456789ab-dirty-1792195200"));
    assert_eq!(label, "Version: 0123456789ab-dirty-1792195200");
    assert_eq!(
        commit.as_deref(),
        Some("https://github.com/lloydmeta/stemyla/commit/0123456789ab")
    );
}

#[test]
fn other_builds_have_no_link() {
    assert_eq!(
        build_version(None),
        ("Version: unpublished build".to_string(), None)
    );
    assert_eq!(build_version(Some("local")).1, None);
    assert_eq!(build_version(Some("not-a-hash-at-all")).1, None);
    assert_eq!(
        build_version(Some("01234567890é")).1,
        None,
        "12 bytes end inside a character"
    );
}
