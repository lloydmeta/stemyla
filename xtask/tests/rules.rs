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
fn synth_brings_no_crate_into_the_web_app() {
    let root = xtask::workspace_root();
    let trees: Vec<(&str, String)> = xtask::rules::WEB_APP
        .into_iter()
        .map(|package| {
            let tree = xtask::rules::web_app_tree(&root, package).expect("cargo tree runs");
            (package, tree)
        })
        .collect();
    let pairs: Vec<(&str, &str)> = trees
        .iter()
        .map(|(package, tree)| (*package, tree.as_str()))
        .collect();
    let problems = xtask::rules::synth_brings_no_crate(&pairs);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} problem(s) with synth in the web app:\n{}",
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
