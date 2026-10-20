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

/// One `cargo metadata` run for both layer tests.
fn workspace() -> &'static xtask::rules::Workspace {
    static WORKSPACE: std::sync::OnceLock<xtask::rules::Workspace> = std::sync::OnceLock::new();
    WORKSPACE.get_or_init(|| {
        let root = xtask::workspace_root();
        let metadata = xtask::rules::workspace_metadata(&root).expect("cargo metadata runs");
        xtask::rules::workspace(&metadata).expect("cargo metadata's JSON gives the workspace")
    })
}

#[test]
fn crates_depend_only_downwards() {
    let architecture =
        std::fs::read_to_string(xtask::workspace_root().join("docs/ARCHITECTURE.md"))
            .expect("docs/ARCHITECTURE.md can be read");
    let problems = xtask::rules::crates_follow_the_layers(workspace(), &architecture);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} problem(s) with the crate layers:\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn the_page_and_the_worker_meet_only_through_stem_protocol() {
    let problems = xtask::rules::shells_meet_only_through_the_protocol(&workspace().crates);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} problem(s) with the page side and the worker side:\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn dsp_is_beginner_style() {
    let files = xtask::rules::dsp_files(&xtask::workspace_root()).expect("dsp/ can be read");
    for known in ["dsp/Cargo.toml", "dsp/src/lib.rs", "dsp/src/mix.rs"] {
        assert!(
            files.iter().any(|(file, _)| file == known),
            "the walk did not reach {known}"
        );
    }
    let problems = xtask::rules::dsp_keeps_beginner_style(&files);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} DSP-1 problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}
