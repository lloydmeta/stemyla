//! `xtask pr-body <file>`: checks a PR body against the form of decision
//! 0027 and `.github/pull_request_template.md`.
//!
//! The body has a part for people and a `<details>` block for agents.  The
//! part for people has four headings, in order.  The summary has at most
//! [`SUMMARY_LIMIT`] words, and the Verification section has at most
//! [`VERIFICATION_LIMIT`] lines.  [`problems`] is a pure function of the
//! body's text, and [`main`] reads the file.
//!
//! `plans/active/harness-phase-3b-m7.md` (step 1) gives the rules.

pub const USAGE: &str = "usage: xtask pr-body <file>";

/// The exit code of a body with problems.
const FAILED: i32 = 1;

/// The exit code of a usage error, and of a file that cannot be read.
const REFUSED: i32 = 2;

/// The headings of the part for people, in their order.
pub const HEADINGS: [&str; 4] = [
    "## Summary",
    "## Needs your attention",
    "## Not verified",
    "## Verification",
];

/// The line that starts the block for agents.
pub const DETAILS: &str = "<details><summary>For agents</summary>";

/// The line that ends the block for agents.
pub const DETAILS_END: &str = "</details>";

/// The most words in the summary.
pub const SUMMARY_LIMIT: usize = 150;

/// The most lines that are not blank in the Verification section.  The
/// least is 1: decision 0027 asks for 2 or 3, and the check takes 1.
pub const VERIFICATION_LIMIT: usize = 3;

/// Each problem of `body`, or none.  Each problem names decision 0027 and
/// a fix.
pub fn problems(body: &str) -> Vec<String> {
    let text = without_comments(&body.replace("\r\n", "\n"));
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let people = match lines.iter().position(|line| line.trim() == DETAILS) {
        Some(start) => {
            if !lines[start + 1..]
                .iter()
                .any(|line| line.trim() == DETAILS_END)
            {
                found.push(format!(
                    "the body has no line `{DETAILS_END}` after `{DETAILS}` (decision 0027).  Fix: close the block."
                ));
            }
            &lines[..start]
        }
        None => {
            found.push(format!(
                "the body has no line `{DETAILS}` (decision 0027).  Fix: put the text for agents in that block, after the part for people."
            ));
            &lines[..]
        }
    };
    let mut firsts = Vec::new();
    for heading in HEADINGS {
        let at: Vec<usize> = (0..people.len())
            .filter(|i| people[*i].trim() == heading)
            .collect();
        match at.len() {
            0 => found.push(format!(
                "the heading `{heading}` is missing from the part for people (decision 0027).  Fix: add it, as in `.github/pull_request_template.md`."
            )),
            1 => {}
            n => found.push(format!(
                "the heading `{heading}` is in the part for people {n} times (decision 0027).  Fix: keep one."
            )),
        }
        if let Some(first) = at.first() {
            firsts.push(*first);
        }
    }
    if firsts.windows(2).any(|pair| pair[0] > pair[1]) {
        found.push(format!(
            "the headings are out of order (decision 0027).  Fix: put them in the order {}.",
            HEADINGS.map(|h| format!("`{h}`")).join(", ")
        ));
    }
    if let Some(summary) = section(people, HEADINGS[0]) {
        let words: usize = summary
            .iter()
            .map(|line| line.split_whitespace().count())
            .sum();
        if words == 0 {
            found.push(format!(
                "the summary is empty (decision 0027).  Fix: say what changed and why, in {SUMMARY_LIMIT} words or fewer."
            ));
        } else if words > SUMMARY_LIMIT {
            found.push(format!(
                "the summary has {words} words, over the limit of {SUMMARY_LIMIT} (decision 0027).  Fix: move the detail to the block for agents."
            ));
        }
    }
    if let Some(verification) = section(people, HEADINGS[3]) {
        let n = verification
            .iter()
            .filter(|line| !line.trim().is_empty())
            .count();
        if n == 0 {
            found.push(
                "the Verification section is empty (decision 0027).  Fix: give 2 or 3 lines: the checks that ran and their results.".to_string(),
            );
        } else if n > VERIFICATION_LIMIT {
            found.push(format!(
                "the Verification section has {n} lines, over the limit of {VERIFICATION_LIMIT} (decision 0027).  Fix: give 2 or 3 lines, and move the rest to the block for agents."
            ));
        }
    }
    found
}

/// `text` with each HTML comment (`<!--` to `-->`) removed, also over
/// lines.  GitHub does not show a comment.  A comment with no end runs to
/// the end of `text`.
fn without_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start + 4..].find("-->") {
            Some(end) => &rest[start + 4 + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// True when `line` is a heading of level 1 or 2 with text.
fn is_top_heading(line: &str) -> bool {
    let line = line.trim();
    let text = line.strip_prefix("# ").or_else(|| line.strip_prefix("## "));
    text.is_some_and(|t| !t.trim().is_empty())
}

/// The lines after the first line that is `heading`, to the next heading
/// line of level 1 or 2 (`# ` or `## `, then text) or the end of `lines`.
/// So `### More` and `#39` stay in the section.  `None` when no line is
/// `heading`.
fn section<'a>(lines: &'a [&'a str], heading: &str) -> Option<&'a [&'a str]> {
    let start = lines.iter().position(|line| line.trim() == heading)? + 1;
    let len = lines[start..]
        .iter()
        .position(|line| is_top_heading(line))
        .unwrap_or(lines.len() - start);
    Some(&lines[start..start + len])
}

/// Runs `xtask pr-body <file>`.  It gives 0 for a body with no problem, 1
/// for a body with problems, and 2 for a usage error or a file that it
/// cannot read.
pub fn main(args: &[String]) -> i32 {
    let [file] = args else {
        eprintln!("{USAGE}");
        return REFUSED;
    };
    let body = match std::fs::read_to_string(file) {
        Ok(body) => body,
        Err(e) => {
            eprintln!("pr-body: cannot read {file}: {e}");
            return REFUSED;
        }
    };
    let found = problems(&body);
    if found.is_empty() {
        println!("pr-body: {file} follows the form of decision 0027");
        return 0;
    }
    for problem in &found {
        println!("pr-body: {problem}");
    }
    FAILED
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = include_str!("../../.github/pull_request_template.md");

    /// A good body with `summary` as its summary.
    fn body_with_summary(summary: &str) -> String {
        format!(
            "## Summary\n\n{summary}\n\n## Needs your attention\n\nNothing\n\n## Not verified\n\n* Safari\n\n## Verification\n\n* fmt, clippy and tests pass.\n* CI is green.\n\n{DETAILS}\n\n* M1 done.\n\n{DETAILS_END}\n"
        )
    }

    fn good() -> String {
        body_with_summary("This change adds the PR body check.")
    }

    fn words(n: usize) -> String {
        vec!["word"; n].join(" ")
    }

    fn missing(heading: &str) -> String {
        format!(
            "the heading `{heading}` is missing from the part for people (decision 0027).  Fix: add it, as in `.github/pull_request_template.md`."
        )
    }

    fn twice(heading: &str, n: usize) -> String {
        format!(
            "the heading `{heading}` is in the part for people {n} times (decision 0027).  Fix: keep one."
        )
    }

    const ORDER: &str = "the headings are out of order (decision 0027).  Fix: put them in the order `## Summary`, `## Needs your attention`, `## Not verified`, `## Verification`.";

    const EMPTY: &str = "the summary is empty (decision 0027).  Fix: say what changed and why, in 150 words or fewer.";

    const NO_VERIFICATION: &str = "the Verification section is empty (decision 0027).  Fix: give 2 or 3 lines: the checks that ran and their results.";

    const NO_DETAILS: &str = "the body has no line `<details><summary>For agents</summary>` (decision 0027).  Fix: put the text for agents in that block, after the part for people.";

    const NO_DETAILS_END: &str = "the body has no line `</details>` after `<details><summary>For agents</summary>` (decision 0027).  Fix: close the block.";

    fn long_summary(n: usize) -> String {
        format!(
            "the summary has {n} words, over the limit of 150 (decision 0027).  Fix: move the detail to the block for agents."
        )
    }

    fn long_verification(n: usize) -> String {
        format!(
            "the Verification section has {n} lines, over the limit of 3 (decision 0027).  Fix: give 2 or 3 lines, and move the rest to the block for agents."
        )
    }

    #[test]
    fn a_good_body_passes() {
        assert_eq!(problems(&good()), Vec::<String>::new());
    }

    #[test]
    fn a_summary_of_150_words_passes_and_of_151_fails() {
        assert_eq!(
            problems(&body_with_summary(&words(150))),
            Vec::<String>::new()
        );
        assert_eq!(
            problems(&body_with_summary(&words(151))),
            vec![long_summary(151)]
        );
    }

    #[test]
    fn the_summary_ends_only_at_a_heading_of_level_1_or_2() {
        let body = good().replace(
            "## Needs your attention",
            &format!(
                "### More\n\n#39 and #40\n{}\n\n## Needs your attention",
                words(140)
            ),
        );
        assert_eq!(problems(&body), vec![long_summary(152)]);
        let top = good().replace(
            "## Needs your attention",
            &format!("# Top\n\n{}\n\n## Needs your attention", words(200)),
        );
        assert_eq!(problems(&top), Vec::<String>::new());
    }

    #[test]
    fn each_missing_heading_fails() {
        for heading in HEADINGS {
            let body = good().replace(&format!("{heading}\n"), "");
            assert_eq!(problems(&body), vec![missing(heading)], "{heading}");
        }
        let none = good()
            .lines()
            .filter(|line| !HEADINGS.contains(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            problems(&none),
            HEADINGS.iter().map(|h| missing(h)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_heading_twice_fails() {
        let body = good().replace("## Not verified\n", "## Not verified\n\n## Not verified\n");
        assert_eq!(problems(&body), vec![twice("## Not verified", 2)]);
    }

    #[test]
    fn two_headings_out_of_order_fail() {
        let body = good()
            .replace("## Not verified", "@@")
            .replace("## Needs your attention", "## Not verified")
            .replace("@@", "## Needs your attention");
        assert_eq!(problems(&body), vec![ORDER.to_string()]);
    }

    #[test]
    fn other_headings_and_spaces_around_a_heading_pass() {
        let body = good()
            .replace("## Not verified", "  ## Not verified  ")
            .replace("## Verification", "## Risks\n\nNone\n\n## Verification");
        assert_eq!(problems(&body), Vec::<String>::new());
    }

    #[test]
    fn a_heading_that_is_only_in_details_is_missing() {
        let body = good().replace("## Not verified\n", "").replace(
            &format!("{DETAILS}\n"),
            &format!("{DETAILS}\n\n## Not verified\n"),
        );
        assert_eq!(problems(&body), vec![missing("## Not verified")]);
    }

    #[test]
    fn no_details_line_fails() {
        let body = good().replace(&format!("{DETAILS}\n\n* M1 done.\n\n{DETAILS_END}\n"), "");
        assert_eq!(problems(&body), vec![NO_DETAILS.to_string()]);
    }

    #[test]
    fn no_details_end_fails() {
        let body = good().replace(DETAILS_END, "");
        assert_eq!(problems(&body), vec![NO_DETAILS_END.to_string()]);
        let before = good()
            .replace(&format!("{DETAILS_END}\n"), "")
            .replace("## Summary", &format!("{DETAILS_END}\n\n## Summary"));
        assert_eq!(problems(&before), vec![NO_DETAILS_END.to_string()]);
        let just_before = good().replace(&format!("{DETAILS_END}\n"), "").replace(
            &format!("{DETAILS}\n"),
            &format!("{DETAILS_END}\n{DETAILS}\n"),
        );
        assert_eq!(problems(&just_before), vec![NO_DETAILS_END.to_string()]);
    }

    #[test]
    fn a_body_that_starts_with_the_details_line_has_no_part_for_people() {
        let body = format!("{DETAILS}\n\n* M1.\n\n{DETAILS_END}\n");
        assert_eq!(
            problems(&body),
            HEADINGS.iter().map(|h| missing(h)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_words_of_an_html_comment_do_not_count() {
        let body = body_with_summary(&format!(
            "<!-- {} -->\nTen words.  <!-- {}\n{} -->",
            words(200),
            words(100),
            words(100)
        ));
        assert_eq!(problems(&body), Vec::<String>::new());
        let only = body_with_summary(&format!("<!-- {} -->", words(3)));
        assert_eq!(problems(&only), vec![EMPTY.to_string()]);
        let around = body_with_summary(&format!("{} <!-- x y z -->w <!--a-->b", words(150)));
        assert_eq!(problems(&around), vec![long_summary(152)]);
    }

    #[test]
    fn a_comment_with_no_end_hides_the_rest() {
        let body = good().replace(
            "Nothing\n\n## Not verified",
            "<!-- Nothing\n\n## Not verified",
        );
        assert_eq!(
            problems(&body),
            vec![
                NO_DETAILS.to_string(),
                missing("## Not verified"),
                missing("## Verification"),
            ]
        );
    }

    #[test]
    fn a_heading_in_an_html_comment_does_not_count() {
        let body = good().replace("## Not verified\n", "<!--\n## Not verified\n-->\n");
        assert_eq!(problems(&body), vec![missing("## Not verified")]);
    }

    #[test]
    fn verification_of_3_lines_passes_and_of_4_fails() {
        let three = good().replace("* CI is green.\n", "* CI is green.\n\n* One more.\n");
        assert_eq!(problems(&three), Vec::<String>::new());
        let four = three.replace("* One more.\n", "* One more.\n* And one more.\n");
        assert_eq!(problems(&four), vec![long_verification(4)]);
    }

    #[test]
    fn a_body_with_crlf_passes() {
        let body = good().replace('\n', "\r\n");
        assert_eq!(problems(&body), Vec::<String>::new());
    }

    #[test]
    fn verification_of_1_line_passes_and_of_0_fails() {
        let one = good().replace("* CI is green.\n", "");
        assert_eq!(problems(&one), Vec::<String>::new());
        let none = one.replace("* fmt, clippy and tests pass.\n", "<!-- x -->\n");
        assert_eq!(problems(&none), vec![NO_VERIFICATION.to_string()]);
    }

    #[test]
    fn the_template_has_only_an_empty_summary_and_verification() {
        assert_eq!(
            problems(TEMPLATE),
            vec![EMPTY.to_string(), NO_VERIFICATION.to_string()]
        );
    }
}
