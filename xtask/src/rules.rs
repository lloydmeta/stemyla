//! Structural checks of the tree.  Each one is a pure function over the
//! files, or over Cargo's output, that returns problems, and each problem
//! names its rule (`docs/invariants.md`) and the fix.  `xtask/tests/rules.rs`
//! reads the real tree and Cargo's output, and runs them.

use std::fmt;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use anyhow::{Context, Result};
use proc_macro2::{Literal, TokenStream, TokenTree};

/// A breach of a rule, found in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub rule: &'static str,
    pub file: String,
    /// `None` for a problem with the whole file.
    pub line: Option<usize>,
    /// The full text: the rule, what is wrong and the fix.
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Every `.rs` file in the tree that `docs::repo_files` walks, with its text.
pub fn rust_files(root: &Path) -> Result<Vec<(String, String)>> {
    crate::docs::repo_files(root)?
        .into_iter()
        .filter(|file| file.ends_with(".rs"))
        .map(|file| {
            let text = std::fs::read_to_string(root.join(&file))
                .with_context(|| format!("reading {file}"))?;
            Ok((file, text))
        })
        .collect()
}

/// The words that start a skip message, in any case.
const SKIP_WORDS: [&str; 2] = ["skipping", "skipped"];

/// Files that hold the skip words on purpose: the helper itself, and this
/// checker's allowlist and planted inputs.
fn holds_skip_words_on_purpose(file: &str) -> bool {
    file.starts_with("test-support/") || file == "xtask/src/rules.rs"
}

/// Skip messages that cannot go through `test_support::skip`: the file, the
/// literal, and why.
const SKIP_ALLOWLIST: &[(&str, &str, &str)] = &[(
    "app/tests/web_output.rs",
    "skipped: the audio context is suspended",
    "a wasm test cannot read STEMYLA_REQUIRE; the test asserts that the browser is not Chrome, and CI runs Chrome",
)];

/// VERIFY-1: every skip message goes through `test_support::skip`, so that
/// `STEMYLA_REQUIRE` can make it a failure.  A problem is a string literal,
/// plain or raw, that starts with `skipping` or `skipped`, in any case.
/// The files are tokenised, so comments, char literals such as `'"'`, raw
/// strings and escapes cannot confuse the check.
pub fn skips_use_the_helper(files: &[(String, String)]) -> Vec<Problem> {
    files
        .iter()
        .filter(|(file, _)| !holds_skip_words_on_purpose(file))
        .flat_map(|(file, text)| skips_in(file, text))
        .collect()
}

/// VERIFY-1: each entry of the skip allowlist still matches a literal in
/// its file, so a stale entry cannot wait there to hide a new skip.
pub fn allowlisted_skips_exist(files: &[(String, String)]) -> Vec<Problem> {
    SKIP_ALLOWLIST
        .iter()
        .filter(|(path, literal, _)| {
            !files.iter().any(|(file, text)| {
                file == path
                    && TokenStream::from_str(text)
                        .is_ok_and(|tokens| string_literals(tokens).iter().any(|(_, value)| value == literal))
            })
        })
        .map(|(path, literal, _)| Problem {
            rule: "VERIFY-1",
            file: path.to_string(),
            line: None,
            message: format!(
                "VERIFY-1: the skip allowlist in xtask/src/rules.rs names \"{literal}\" in {path}, but the file has no such literal.  Fix: remove the entry, or change it to the file's skip message."
            ),
        })
        .collect()
}

fn skips_in(file: &str, text: &str) -> Vec<Problem> {
    match TokenStream::from_str(text) {
        Err(e) => vec![Problem {
            rule: "VERIFY-1",
            file: file.to_string(),
            line: None,
            message: format!(
                "VERIFY-1: {file} does not tokenise: {e}.  Fix: make the file valid Rust, or leave it out of the tree."
            ),
        }],
        Ok(tokens) => string_literals(tokens)
            .into_iter()
            .filter(|(_, value)| {
                !SKIP_ALLOWLIST
                    .iter()
                    .any(|(path, literal, _)| *path == file && literal == value)
            })
            .filter_map(|(line, value)| {
                let word = SKIP_WORDS.into_iter().find_map(|word| {
                    value
                        .get(..word.len())
                        .filter(|start| start.eq_ignore_ascii_case(word))
                })?;
                Some(Problem {
                    rule: "VERIFY-1",
                    file: file.to_string(),
                    line: Some(line),
                    message: format!(
                        "VERIFY-1: {file}:{line} prints \"{word}\" itself.  Fix: call test_support::skip(Need::..., why), so STEMYLA_REQUIRE can make it a failure.  Outside test code, add the literal to the allowlist with a reason."
                    ),
                })
            })
            .collect(),
    }
}

/// The line and the text between the quotes of every string literal, plain
/// or raw, in groups and macro calls too.  Doc comments, which proc-macro2
/// makes into `#[doc = "..."]` literals, are left out.
fn string_literals(tokens: TokenStream) -> Vec<(usize, String)> {
    tokens
        .into_iter()
        .flat_map(|tree| match tree {
            TokenTree::Group(group) => string_literals(group.stream()),
            TokenTree::Literal(literal) => string_value(&literal)
                .map(|value| (literal.span().start().line, value))
                .into_iter()
                .collect(),
            TokenTree::Ident(_) | TokenTree::Punct(_) => Vec::new(),
        })
        .collect()
}

fn string_value(literal: &Literal) -> Option<String> {
    let source = literal.span().source_text()?;
    if source.starts_with("//") || source.starts_with("/*") {
        return None;
    }
    let text = literal.to_string();
    let quoted = text
        .strip_prefix('r')
        .map_or(text.as_str(), |raw| raw.trim_matches('#'));
    quoted
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .map(str::to_string)
}

/// The packages of the web app, one WebAssembly module each.
pub const WEB_APP: [&str; 2] = ["app", "stem-worker"];

/// The wasm tree of one package of the web app, as `cargo tree --prefix
/// depth` prints it: normal edges only, and one package per line.  One run
/// per package, because one run with both merges their features, and Trunk
/// builds them apart.  `--color never`, because CI's `CARGO_TERM_COLOR:
/// always` reaches this process and would colour each `(*)`.
pub fn web_app_tree(root: &Path, package: &str) -> Result<String> {
    let output = Command::new(env!("CARGO"))
        .current_dir(root)
        .args(["tree", "--locked", "--color", "never", "-p", package])
        .args(["-e", "normal", "--target", "wasm32-unknown-unknown"])
        .args(["--prefix", "depth", "-f", "{p}"])
        .output()
        .with_context(|| format!("running cargo tree -p {package}"))?;
    anyhow::ensure!(
        output.status.success(),
        "cargo tree -p {package} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .with_context(|| format!("cargo tree -p {package} printed text that is not UTF-8"))
}

/// LAYER-1: in the normal dependency tree of the web app, `synth` brings no
/// other crate.  `trees` holds `(package, tree)` pairs from `web_app_tree`.
/// A tree that the check cannot read is a problem too, so that it cannot
/// pass without reading anything.
pub fn synth_brings_no_crate(trees: &[(&str, &str)]) -> Vec<Problem> {
    let problems: Vec<Problem> = trees
        .iter()
        .flat_map(|(package, tree)| tree_problems(package, tree))
        .collect();
    let synth_seen = trees.iter().any(|(_, tree)| {
        tree.lines()
            .filter_map(tree_line)
            .any(|line| line.name == "synth")
    });
    let unread = (!synth_seen).then(|| Problem {
        rule: "LAYER-1",
        file: manifest_of("app"),
        line: None,
        message: "LAYER-1: the web app's trees have no synth, so this check read nothing.  Fix: if the web app no longer takes synth, remove synth_brings_no_crate_into_the_web_app.  Update the docs that name it (DOC-1).".into(),
    });
    problems.into_iter().chain(unread).collect()
}

/// One line of `cargo tree --prefix depth`: the depth, then the package.
struct TreeLine<'a> {
    depth: usize,
    name: &'a str,
}

fn tree_line(line: &str) -> Option<TreeLine<'_>> {
    let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
    let depth = line[..line.len() - rest.len()].parse().ok()?;
    let name = rest.split_whitespace().next()?;
    Some(TreeLine { depth, name })
}

fn manifest_of(package: &str) -> String {
    match package {
        "stem-worker" => "worker/Cargo.toml".into(),
        _ => format!("{package}/Cargo.toml"),
    }
}

fn tree_problems(package: &str, tree: &str) -> Vec<Problem> {
    let problem = |message: String| Problem {
        rule: "LAYER-1",
        file: manifest_of(package),
        line: None,
        message,
    };
    let starts_with_package = tree
        .lines()
        .next()
        .and_then(tree_line)
        .is_some_and(|first| first.depth == 0 && first.name == package);
    let bad_start = (!starts_with_package).then(|| {
        problem(format!(
            "LAYER-1: the tree of {package} does not start with {package}, so this check cannot read it.  Fix: run cargo tree -p {package} with --prefix depth."
        ))
    });
    let without_depth = tree
        .lines()
        .filter(|line| !line.trim().is_empty() && tree_line(line).is_none())
        .map(|line| {
            problem(format!(
                "LAYER-1: the tree of {package} has a line without a depth: \"{line}\".  Fix: run cargo tree with --prefix depth."
            ))
        });
    let lines: Vec<TreeLine> = tree.lines().filter_map(tree_line).collect();
    let synth_children = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.name == "synth")
        .map(|(at, synth)| {
            lines[at + 1..]
                .iter()
                .take_while(|line| line.depth > synth.depth)
                .filter(|line| line.depth == synth.depth + 1)
                .map(|line| line.name)
                .collect::<Vec<_>>()
        })
        .filter(|children| !children.is_empty())
        .map(|children| {
            problem(format!(
                "LAYER-1: synth brings {} into the web app, through {package}.  Fix: make each dependency of synth optional, behind its files feature.  Take synth without its files feature in the web app.  Tools and tests take features = [\"files\"].",
                children.join(", ")
            ))
        });
    bad_start
        .into_iter()
        .chain(without_depth)
        .chain(synth_children)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_problem(file: &str, package: &str, children: &str) -> Problem {
        Problem {
            rule: "LAYER-1",
            file: file.into(),
            line: None,
            message: format!(
                "LAYER-1: synth brings {children} into the web app, through {package}.  Fix: make each dependency of synth optional, behind its files feature.  Take synth without its files feature in the web app.  Tools and tests take features = [\"files\"]."
            ),
        }
    }

    const NO_SYNTH: &str = "LAYER-1: the web app's trees have no synth, so this check read nothing.  Fix: if the web app no longer takes synth, remove synth_brings_no_crate_into_the_web_app.  Update the docs that name it (DOC-1).";

    #[test]
    fn synth_with_children_is_a_problem() {
        let tree = "\
0app v0.1.0 (/x/app)
1anyhow v1.0.104
1synth v0.1.0 (/x/synth)
2anyhow v1.0.104
2hound v3.5.1
2serde v1.0.229
3serde_core v1.0.229
3serde_derive v1.0.229 (proc-macro)
4proc-macro2 v1.0.107 (*)
4quote v1.0.47 (*)
4syn v3.0.7 (*)
2serde_json v1.0.152
3itoa v1.0.18
3memchr v2.8.3
3serde_core v1.0.229
3zmij v1.0.23
1wasm-bindgen v0.2.129 (*)
1web-sys v0.3.106 (*)
";
        let problems = synth_brings_no_crate(&[("app", tree)]);
        assert_eq!(
            problems,
            [Problem {
                rule: "LAYER-1",
                file: "app/Cargo.toml".into(),
                line: None,
                message: "LAYER-1: synth brings anyhow, hound, serde, serde_json into the web app, through app.  Fix: make each dependency of synth optional, behind its files feature.  Take synth without its files feature in the web app.  Tools and tests take features = [\"files\"].".into(),
            }]
        );
    }

    #[test]
    fn synth_without_children_is_no_problem() {
        let tree = "0app v0.1.0 (/x/app)\n1synth v0.1.0 (/x/synth)\n1web-sys v0.3.106\n";
        assert_eq!(synth_brings_no_crate(&[("app", tree)]), []);
    }

    #[test]
    fn a_line_at_synths_depth_is_not_its_child() {
        let tree = "0app v0.1.0\n1synth v0.1.0\n1engine v0.1.0\n2hound v3.5.1\n";
        assert_eq!(synth_brings_no_crate(&[("app", tree)]), []);
    }

    #[test]
    fn a_shallower_line_ends_synths_children() {
        let tree = "0app v0.1.0\n1engine v0.1.0\n2synth v0.1.0\n1dsp v0.1.0\n3hound v3.5.1\n";
        assert_eq!(synth_brings_no_crate(&[("app", tree)]), []);
    }

    #[test]
    fn a_deeper_synth_is_read() {
        let tree = "0app v0.1.0\n9engine v0.1.0\n10synth v0.1.0\n11hound v3.5.1\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", tree)]),
            [synth_problem("app/Cargo.toml", "app", "hound")]
        );
    }

    #[test]
    fn a_repeated_synth_gives_one_problem() {
        let tree = "\
0app v0.1.0
1synth v0.1.0
2hound v3.5.1
1engine v0.1.0
2synth v0.1.0 (*)
";
        assert_eq!(
            synth_brings_no_crate(&[("app", tree)]),
            [synth_problem("app/Cargo.toml", "app", "hound")]
        );
    }

    #[test]
    fn a_tree_without_synth_is_a_problem() {
        let app = "0app v0.1.0\n1engine v0.1.0\n";
        let worker = "0stem-worker v0.1.0\n1synthx v1.0.0\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", app), ("stem-worker", worker)]),
            [Problem {
                rule: "LAYER-1",
                file: "app/Cargo.toml".into(),
                line: None,
                message: NO_SYNTH.into(),
            }]
        );
    }

    #[test]
    fn synth_under_stem_worker_is_named() {
        let app = "0app v0.1.0 (/x/app)\n1synth v0.1.0 (/x/synth)\n1web-sys v0.3.106\n";
        let worker = "\
0stem-worker v0.1.0 (/x/worker)
1synth v0.1.0 (/x/synth)
2anyhow v1.0.104
2hound v3.5.1
2serde v1.0.229 (*)
2serde_json v1.0.152 (*)
";
        assert_eq!(
            synth_brings_no_crate(&[("app", app), ("stem-worker", worker)]),
            [synth_problem(
                "worker/Cargo.toml",
                "stem-worker",
                "anyhow, hound, serde, serde_json"
            )]
        );
    }

    #[test]
    fn a_line_without_a_depth_is_a_problem() {
        let tree = "0stem-worker v0.1.0\n1synth v0.1.0\n\nsynth v0.1.0\n";
        assert_eq!(
            synth_brings_no_crate(&[("stem-worker", tree)]),
            [Problem {
                rule: "LAYER-1",
                file: "worker/Cargo.toml".into(),
                line: None,
                message: "LAYER-1: the tree of stem-worker has a line without a depth: \"synth v0.1.0\".  Fix: run cargo tree with --prefix depth.".into(),
            }]
        );
    }

    #[test]
    fn a_tree_that_does_not_start_with_its_package_is_a_problem() {
        let worker = "0app v0.1.0\n1synth v0.1.0\n";
        assert_eq!(
            synth_brings_no_crate(&[("app", ""), ("stem-worker", worker)]),
            [
                Problem {
                    rule: "LAYER-1",
                    file: "app/Cargo.toml".into(),
                    line: None,
                    message: "LAYER-1: the tree of app does not start with app, so this check cannot read it.  Fix: run cargo tree -p app with --prefix depth.".into(),
                },
                Problem {
                    rule: "LAYER-1",
                    file: "worker/Cargo.toml".into(),
                    line: None,
                    message: "LAYER-1: the tree of stem-worker does not start with stem-worker, so this check cannot read it.  Fix: run cargo tree -p stem-worker with --prefix depth.".into(),
                },
            ]
        );
    }

    fn check(file: &str, text: &str) -> Vec<Problem> {
        skips_use_the_helper(&[(file.to_string(), text.to_string())])
    }

    fn lines(problems: &[Problem]) -> Vec<Option<usize>> {
        problems.iter().map(|p| p.line).collect()
    }

    #[test]
    fn a_skip_message_in_a_test_is_a_problem() {
        let problems = check(
            "roformer/tests/x.rs",
            "fn gpu() {\n    eprintln!(\"skipping: no adapter\");\n}\n",
        );
        assert_eq!(
            problems,
            [Problem {
                rule: "VERIFY-1",
                file: "roformer/tests/x.rs".into(),
                line: Some(2),
                message: "VERIFY-1: roformer/tests/x.rs:2 prints \"skipping\" itself.  Fix: call test_support::skip(Need::..., why), so STEMYLA_REQUIRE can make it a failure.  Outside test code, add the literal to the allowlist with a reason.".into(),
            }]
        );
        assert_eq!(problems[0].to_string(), problems[0].message);
    }

    #[test]
    fn a_skipped_literal_in_src_is_a_problem() {
        let problems = check("engine/src/a.rs", "const A: &str = \"skipped: x\";\n");
        assert_eq!(lines(&problems), [Some(1)]);
        assert!(
            problems[0]
                .message
                .starts_with("VERIFY-1: engine/src/a.rs:1 prints \"skipped\" itself."),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_skip_word_in_any_case_is_a_problem() {
        let text = "fn f() {\n    eprintln!(\"Skipping: x\");\n    eprintln!(\"SKIPPED: x\");\n}\n";
        let problems = check("a/tests/b.rs", text);
        assert_eq!(lines(&problems), [Some(2), Some(3)]);
        assert!(
            problems[0].message.contains("prints \"Skipping\" itself."),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_literal_on_the_line_after_a_macro_call_has_that_line() {
        let text = "fn f() {\n    eprintln!(\n        \"skipping: {}\",\n        1\n    );\n}\n";
        assert_eq!(lines(&check("a/tests/b.rs", text)), [Some(3)]);
    }

    #[test]
    fn a_skip_literal_after_a_quote_char_is_found() {
        let text = r#"fn f() { let q = '"'; eprintln!("skipping: x"); }"#;
        assert_eq!(lines(&check("a/tests/b.rs", text)), [Some(1)]);
    }

    #[test]
    fn a_raw_skip_literal_is_found() {
        let text = "fn f() {\n    let a = r\"skipping\";\n    let b = r##\"skipped \"x\"\"##;\n}\n";
        assert_eq!(lines(&check("a/tests/b.rs", text)), [Some(2), Some(3)]);
    }

    #[test]
    fn literals_that_only_look_like_skips_are_not_problems() {
        let text = r##"fn f() {
    let a = r#"a "b"#;
    let b = "\\";
    let c = "selftest: skipping the full chunk";
    let d = "\"skipping\"";
    let e = b"skipping";
}
"##;
        assert_eq!(check("a/tests/b.rs", text), []);
    }

    /// Each doc comment has no space after its marker, so it is a problem
    /// unless doc comments are left out.
    #[test]
    fn skip_words_in_comments_are_not_problems() {
        let text = "//!skipping x\n// skipping\n///skipping: x\n/**skipped */\nfn f() {\n    /*skipped*/\n}\n";
        assert_eq!(check("a/tests/b.rs", text), []);
    }

    #[test]
    fn the_allowlisted_skip_is_not_a_problem_in_its_own_file_only() {
        let text = "fn f() { console_log!(\"skipped: the audio context is suspended\"); }";
        assert_eq!(check("app/tests/web_output.rs", text), []);
        assert_eq!(lines(&check("app/tests/web.rs", text)), [Some(1)]);
    }

    #[test]
    fn an_allowlist_entry_whose_literal_is_gone_is_a_problem() {
        let (file, literal, _) = SKIP_ALLOWLIST[0];
        let current = format!("fn f() {{ console_log!(\"{literal}\"); }}");
        assert_eq!(allowlisted_skips_exist(&[(file.to_string(), current)]), []);
        let reworded = "fn f() { console_log!(\"skipped: suspended\"); }".to_string();
        let problems = allowlisted_skips_exist(&[(file.to_string(), reworded)]);
        assert_eq!(
            problems,
            [Problem {
                rule: "VERIFY-1",
                file: file.into(),
                line: None,
                message: format!(
                    "VERIFY-1: the skip allowlist in xtask/src/rules.rs names \"{literal}\" in {file}, but the file has no such literal.  Fix: remove the entry, or change it to the file's skip message."
                ),
            }]
        );
        assert_eq!(
            lines(&allowlisted_skips_exist(&[])),
            [None],
            "a file that is not in the tree"
        );
    }

    #[test]
    fn each_allowlist_entry_has_a_reason() {
        for (file, literal, reason) in SKIP_ALLOWLIST {
            assert!(!reason.trim().is_empty(), "{file}: {literal}");
        }
    }

    #[test]
    fn the_helper_and_this_checker_are_left_out() {
        let text = "fn f() { eprintln!(\"skipping: x\"); }";
        assert_eq!(check("test-support/src/lib.rs", text), []);
        assert_eq!(check("test-support/tests/skip.rs", text), []);
        assert_eq!(check("xtask/src/rules.rs", text), []);
        assert_eq!(lines(&check("xtask/src/docs.rs", text)), [Some(1)]);
    }

    #[test]
    fn a_file_that_does_not_tokenise_is_a_problem() {
        let problems = check("a/src/b.rs", "fn f() { let s = \"unterminated; }\n");
        assert_eq!(lines(&problems), [None]);
        assert!(
            problems[0]
                .message
                .starts_with("VERIFY-1: a/src/b.rs does not tokenise: "),
            "{}",
            problems[0]
        );
        assert!(
            problems[0]
                .message
                .ends_with(".  Fix: make the file valid Rust, or leave it out of the tree."),
            "{}",
            problems[0]
        );
    }
}
