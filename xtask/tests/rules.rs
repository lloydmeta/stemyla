//! The structural rules hold on the real tree (`xtask::rules`).

#[test]
fn every_skip_can_be_required() {
    let files = xtask::rules::rust_files(&xtask::workspace_root()).expect("the tree can be read");
    let problems = xtask::rules::skips_use_the_helper(&files);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} skip problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}
