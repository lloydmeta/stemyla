//! `xtask pr-body` as a binary: a good body, the template and a usage
//! error.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A temporary folder that goes when the test ends.
struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn folder(name: &str) -> Folder {
    let dir = std::env::temp_dir().join(format!("xtask-pr-body-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Folder(dir)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("pr-body")
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

const GOOD: &str = "## Summary\n\nThis change adds the PR body check.\n\n## Needs your attention\n\nNothing\n\n## Not verified\n\nNothing\n\n## Verification\n\n* The tests pass.\n\n<details><summary>For agents</summary>\n\n* M7.\n\n</details>\n";

#[test]
fn a_good_body_exits_0() {
    let dir = folder("good");
    let file = dir.0.join("body.md");
    std::fs::write(&file, GOOD).unwrap();
    let file = file.to_str().unwrap();
    let out = run(&[file]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert_eq!(
        stdout(&out),
        format!("pr-body: {file} follows the form of decision 0027\n")
    );
}

#[test]
fn the_template_exits_1_with_the_summary_and_verification_problems() {
    let template: PathBuf = xtask::workspace_root().join(".github/pull_request_template.md");
    let out = run(&[template.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert_eq!(
        stdout(&out),
        "pr-body: the summary is empty (decision 0027).  Fix: say what changed and why, in 150 words or fewer.\npr-body: the Verification section is empty (decision 0027).  Fix: give 2 or 3 lines: the checks that ran and their results.\n"
    );
}

#[test]
fn no_argument_or_no_file_exits_2() {
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "usage: xtask pr-body <file>\n"
    );
    let out = run(&["a.md", "b.md"]);
    assert_eq!(out.status.code(), Some(2));
    let dir = folder("none");
    let none = dir.0.join("none.md");
    let out = run(&[none.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with(&format!(
            "pr-body: cannot read {}: ",
            Path::new(&none).display()
        )),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout(&out), "");
}
