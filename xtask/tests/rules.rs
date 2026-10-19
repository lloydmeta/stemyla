//! The structural rules hold on the real tree (`xtask::rules`).

fn rust_files() -> Vec<(String, String)> {
    let files = xtask::rules::rust_files(&xtask::workspace_root()).expect("the tree can be read");
    for known in [
        "roformer/tests/reference.rs",
        "roformer-gpu/tests/kernels.rs",
    ] {
        assert!(
            files.iter().any(|(file, _)| file == known),
            "the walk did not reach {known}"
        );
    }
    files
}

#[test]
fn every_skip_can_be_required() {
    let files = rust_files();
    let problems = xtask::rules::skips_use_the_helper(&files);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} skip problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn the_skip_allowlist_is_current() {
    let problems = xtask::rules::allowlisted_skips_exist(&rust_files());
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(problems.is_empty(), "{}", report.join("\n"));
}
