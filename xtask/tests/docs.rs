//! The agent map and docs stay true to the code (`xtask docs-check`).

#[test]
fn the_map_and_docs_match_the_code() {
    let problems = xtask::docs::check(&xtask::workspace_root()).expect("the docs can be read");
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} docs problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}
