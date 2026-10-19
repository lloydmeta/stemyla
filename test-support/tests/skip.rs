//! [`skip`] itself: it reads `STEMYLA_REQUIRE` at run time, and its panic
//! names the caller's line.  Each test runs this binary again with only the
//! child test, so the variable is set on the child's `Command`; `set_var` is
//! unsafe while other tests run.

use std::process::{Command, Output};

use test_support::{Need, VAR, skip};

const CHILD: &str = "child_skips_without_a_gpu";

/// Set by the tests below, so an ordinary run (`--include-ignored` too)
/// passes the child at once.
const MARKER: &str = "STEMYLA_TEST_CHILD";

#[test]
fn child_skips_without_a_gpu() {
    if std::env::var_os(MARKER).is_none() {
        return;
    }
    skip(Need::Gpu, "no adapter");
}

/// Runs the child test, with `require` as `STEMYLA_REQUIRE` or the variable
/// removed.
fn child(require: Option<&str>) -> (Output, String) {
    let mut command = Command::new(std::env::current_exe().expect("the test binary"));
    command
        .args(["--exact", CHILD, "--nocapture"])
        .env(MARKER, "1");
    match require {
        Some(value) => command.env(VAR, value),
        None => command.env_remove(VAR),
    };
    let output = command.output().expect("the child test runs");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output, stderr)
}

/// The line of the child's `skip` call in this file.
fn skip_call_line() -> usize {
    let lines: Vec<usize> = include_str!("skip.rs")
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            line.trim_start()
                .starts_with("skip(Need::Gpu, \"no adapter\");")
        })
        .map(|(index, _)| index + 1)
        .collect();
    assert_eq!(lines.len(), 1, "one skip call in skip.rs: {lines:?}");
    lines[0]
}

#[test]
fn a_required_skip_fails_and_names_the_caller() {
    let (output, stderr) = child(Some("gpu"));
    assert!(!output.status.success(), "the child passed:\n{stderr}");
    assert!(
        stderr.contains("VERIFY-1: STEMYLA_REQUIRE has gpu, but this test cannot run: no adapter."),
        "{stderr}"
    );
    let location = format!("tests/skip.rs:{}:", skip_call_line());
    assert!(stderr.contains(&location), "no {location} in:\n{stderr}");
}

#[test]
fn a_skip_that_is_not_required_passes() {
    let (output, stderr) = child(None);
    assert!(output.status.success(), "the child failed:\n{stderr}");
    assert!(
        stderr.contains(
            "skipping (gpu): no adapter.  Set STEMYLA_REQUIRE=gpu to make this a failure."
        ),
        "{stderr}"
    );
}
