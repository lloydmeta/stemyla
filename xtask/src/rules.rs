//! Structural checks of the tree.  Each one is a pure function over the
//! files that returns problems, and each problem names its rule
//! (`docs/invariants.md`) and the fix.  `xtask/tests/rules.rs` runs them on
//! the real tree.

use std::fmt;
use std::path::Path;
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
/// plain or raw, that starts with `skipping` or `skipped`, in any case.  The files are
/// tokenised, so comments, char literals such as `'"'`, raw strings and
/// escapes cannot confuse the check.
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

#[cfg(test)]
mod tests {
    use super::*;

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
