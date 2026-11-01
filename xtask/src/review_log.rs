//! The fix rule of decision 0024 (`docs/decisions/0024-what-a-review-round-fixes.md`),
//! checked on the findings table of a review log.
//!
//! Rounds 1 to 3 fix each confirmed finding.  From round 4, a round fixes
//! only blockers, significant should-fix and confirmed findings on a strict
//! rule.  The check reads only a log that has a `Significant` column, so the
//! logs before harness phase 3b, M3 pass.  These are pure functions of text.

/// The strict rules, as decision 0024 gives them.
pub const STRICT_RULES_TEXT: &str =
    "PROTO-1, PROTO-2, MEM-1 to MEM-5, RT-1, GPU-1 to GPU-3, VERIFY-1, GIT-1, DEPS-1, ASSET-1";

/// The IDs of [`STRICT_RULES_TEXT`], in order.  `X-a to X-b` gives each ID
/// from `X-a` to `X-b`.
pub fn strict_rules() -> Vec<String> {
    let mut ids = Vec::new();
    for item in STRICT_RULES_TEXT.split(", ") {
        let (first, last) = item.split_once(" to ").unwrap_or((item, item));
        let (prefix, from) = first.split_once('-').expect("an ID has a dash");
        let (_, to) = last.split_once('-').expect("an ID has a dash");
        let from: u32 = from.parse().expect("an ID ends with a number");
        let to: u32 = to.parse().expect("an ID ends with a number");
        ids.extend((from..=to).map(|n| format!("{prefix}-{n}")));
    }
    ids
}

/// The columns that the check reads, by name.
const COLUMNS: [&str; 9] = [
    "Round",
    "Finding",
    "Reviewers",
    "Severity",
    "Significant",
    "Rule",
    "Verdict",
    "Repeat of",
    "Action",
];

const SEVERITIES: &[&str] = &["blocker", "should-fix", "nit"];

const VERDICTS: &[&str] = &[
    "confirmed",
    "pre-existing",
    "refuted",
    "unproven",
    "unvalidated",
    "known",
];

/// The verdicts of a finding that may be real.  A blocker or should-fix
/// with one of them gets `yes` or `no` in Significant.
const MAY_BE_REAL: &[&str] = &["confirmed", "pre-existing", "unproven"];

/// The starts of a part of an Action cell that names no fix.  A TD ID is
/// one too ([`starts_with_td`]).
const NOT_FIXED: &[&str] = &["reported, not fixed", "refuted", "known: ", "no change:"];

/// The cells of a table line.  A cell ends at a `|` with no `\` before it.
fn cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for c in line.chars() {
        if c == '|' && !escaped {
            cells.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(c);
        }
        escaped = c == '\\';
    }
    cells.push(cell.trim().to_string());
    // The text before the first `|` and after the last one is no cell.
    cells.remove(0);
    cells.pop();
    cells
}

/// The header and the rows of the first table under `## Findings`.
fn findings_table(text: &str) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let section = crate::docs::section(text, "## Findings")?;
    let mut lines = section
        .lines()
        .skip_while(|line| !line.trim_start().starts_with('|'))
        .take_while(|line| line.trim_start().starts_with('|'));
    let header = cells(lines.next()?);
    // The separator line below the header.
    lines.next();
    Some((header, lines.map(cells).collect()))
}

/// The findings table of `text` has a `Significant` column.
pub fn has_significant_column(text: &str) -> bool {
    findings_table(text).is_some_and(|(header, _)| header.iter().any(|c| c == "Significant"))
}

/// `TD-` and a digit start the text.
fn starts_with_td(text: &str) -> bool {
    text.strip_prefix("TD-")
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// `TD-` and digits, and nothing else.
fn is_td(text: &str) -> bool {
    text.strip_prefix("TD-")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// A word of 7 to 40 hex characters with at least one digit.
fn is_sha(word: &str) -> bool {
    (7..=40).contains(&word.len())
        && word.chars().all(|c| c.is_ascii_hexdigit())
        && word.chars().any(|c| c.is_ascii_digit())
}

/// The kind of an Action cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// A part names a fix commit or a diff.
    Fix,
    /// No part names a fix, and a part names the reason.  `td`: a part
    /// is a TD ID.  `no_change`: a part starts with `no change:`.
    NotFixed { td: bool, no_change: bool },
    /// Neither.
    Other,
}

fn action_kind(action: &str) -> Action {
    let parts: Vec<&str> = action.split(';').map(str::trim).collect();
    let not_fixed =
        |part: &&str| NOT_FIXED.iter().any(|s| part.starts_with(s)) || starts_with_td(part);
    let merge = |part: &&str| part.starts_with("merged into ");
    let fix = parts
        .iter()
        .filter(|part| !not_fixed(part) && !merge(part))
        .any(|part| {
            part.split(|c: char| !c.is_alphanumeric())
                .any(|word| word == "diff" || is_sha(word))
        });
    if fix {
        Action::Fix
    } else if parts.iter().any(not_fixed) && parts.iter().all(|p| not_fixed(p) || merge(p)) {
        Action::NotFixed {
            td: parts.iter().any(|part| starts_with_td(part)),
            no_change: parts.iter().any(|part| part.starts_with("no change:")),
        }
    } else {
        Action::Other
    }
}

/// The cells of one row, by column name.
struct Row<'a> {
    round: &'a str,
    id: &'a str,
    reviewers: &'a str,
    severity: &'a str,
    significant: &'a str,
    rule: &'a str,
    verdict: &'a str,
    repeat_of: &'a str,
    action: &'a str,
}

impl Row<'_> {
    /// The Rule cell names a strict rule, or cargo-mutants (VERIFY-1) is a
    /// source.  The cell splits at each character that is not a letter, a
    /// digit or `-`, so `MEM-1/MEM-2` and `RT-1's rule` name rules.
    fn is_strict(&self, strict: &[String]) -> bool {
        let named = self
            .rule
            .split(|c: char| !c.is_alphanumeric() && c != '-')
            .any(|token| strict.iter().any(|id| id == token));
        named
            || self
                .reviewers
                .split(',')
                .any(|source| source.trim() == "cargo-mutants")
    }

    /// The problems of the row's values: problems 2 to 5.
    fn value_problems(&self) -> Vec<String> {
        let id = self.id;
        let mut problems = Vec::new();
        if !self.round.parse::<u32>().is_ok_and(|r| r >= 1) {
            problems.push(format!("{id}: the round `{}` is not a whole number of 1 or more.  Fix: write the number of the round.", self.round));
        }
        if !SEVERITIES.contains(&self.severity) {
            problems.push(format!("{id}: the severity `{}` is not `blocker`, `should-fix` or `nit`.  Fix: write one of them.", self.severity));
        }
        if !VERDICTS.contains(&self.verdict) {
            problems.push(format!("{id}: the verdict `{}` is not a verdict of the review-rounds skill.  Fix: write `confirmed`, `pre-existing`, `refuted`, `unproven`, `unvalidated` or `known`.", self.verdict));
        }
        if !["yes", "no", "-"].contains(&self.significant) {
            problems.push(format!(
                "{id}: Significant is `{}`, not `yes`, `no` or `-`.  Fix: write one of them.",
                self.significant
            ));
        }
        problems
    }

    /// The problems of the fix rule: problems 6 to 11.  The values are good.
    fn rule_problems(&self, strict: &[String]) -> Vec<String> {
        let (id, verdict, action) = (self.id, self.verdict, self.action);
        let round: u32 = self.round.parse().unwrap_or(1);
        let late = round >= 4;
        let mut problems = Vec::new();
        let needs_value = self.severity != "nit" && MAY_BE_REAL.contains(&verdict);
        if needs_value && self.significant == "-" {
            problems.push(format!("{id}: a {} with the verdict `{verdict}` has `-` in Significant.  Fix: write `yes` or `no`, by the test of decision 0024.", self.severity));
        }
        if !needs_value && self.significant != "-" {
            problems.push(format!("{id}: Significant is `{}`, but only a blocker or should-fix with the verdict `confirmed`, `pre-existing` or `unproven` gets a value.  Fix: write `-`.", self.significant));
        }
        if verdict == "known" {
            let td = action
                .split(';')
                .find_map(|part| part.trim().strip_prefix("known: "))
                .map(str::trim);
            if !td.is_some_and(|td| is_td(td) && td == self.repeat_of) {
                problems.push(format!("{id}: a `known` row needs the action `known: <TD>`, with that TD in Repeat of.  Fix: write the TD in both cells."));
            }
        }
        let kind = action_kind(action);
        if kind == Action::Other {
            problems.push(format!("{id}: the action `{action}` names no fix and no reason for no fix.  Fix: name the fix commit or diff, or write `reported, not fixed`, `refuted`, `known: <TD>`, `no change: <reason>` or a TD ID."));
        }
        if kind == Action::Fix && (verdict == "refuted" || verdict == "known") {
            problems.push(format!("{id}: a `{verdict}` finding has the fix `{action}`.  Fix: do not fix it, and write the reason for no change."));
        }
        let named =
            self.severity == "blocker" || self.significant == "yes" || self.is_strict(strict);
        let must_fix = verdict == "confirmed" && (!late || named);
        if must_fix && matches!(kind, Action::NotFixed { td: false, .. }) {
            problems.push(format!("{id}: round {round} must fix this finding, but the action is `{action}`.  Fix: fix it, or send it to a TD (decision 0024)."));
        }
        // A `pre-existing` finding can also go to its own PR (`no change:`).
        let must_handle = verdict == "pre-existing" && (!late || named);
        if must_handle
            && kind
                == (Action::NotFixed {
                    td: false,
                    no_change: false,
                })
        {
            problems.push(format!("{id}: round {round} must fix this `pre-existing` finding, but the action is `{action}`.  Fix: fix it, send it to a TD, or write `no change:` and its own PR (decision 0024)."));
        }
        // Problem 9 covers a fix of a `refuted` or `known` finding.
        let may_fix = MAY_BE_REAL.contains(&verdict) && named;
        let answered = verdict == "refuted" || verdict == "known";
        if late && !may_fix && !answered && kind == Action::Fix {
            problems.push(format!("{id}: round {round} must not fix this finding, but the action is `{action}`.  Fix: undo the fix, and write `reported, not fixed` (decision 0024)."));
        }
        problems
    }
}

/// The problems of one log's findings table against the fix rule.  Each
/// problem names the finding and its fix.  A log with no `Significant`
/// column gives no problem.
pub fn fix_rule_problems(text: &str) -> Vec<String> {
    let Some((header, rows)) = findings_table(text) else {
        return Vec::new();
    };
    if !header.iter().any(|c| c == "Significant") {
        return Vec::new();
    }
    let index: Vec<Option<usize>> = COLUMNS
        .iter()
        .map(|name| header.iter().position(|c| c == name))
        .collect();
    let missing: Vec<String> = COLUMNS
        .iter()
        .zip(&index)
        .filter(|(_, i)| i.is_none())
        .map(|(name, _)| format!("findings table: the header has no `{name}` column.  Fix: use the header of step 11 of the review-rounds skill."))
        .collect();
    if !missing.is_empty() {
        return missing;
    }
    let index: Vec<usize> = index.into_iter().flatten().collect();
    let strict = strict_rules();
    let mut problems = Vec::new();
    for (n, cells) in rows.iter().enumerate() {
        let cell = |column: usize| cells.get(index[column]).map_or("", String::as_str);
        let id = match cell(1) {
            "" => format!("row {}", n + 1),
            id => id.to_string(),
        };
        if cells.len() != header.len() {
            problems.push(format!("{id}: the row has {} cells, and the header has {}.  Fix: give the row one cell for each column, and write a `|` in a cell as `\\|`.", cells.len(), header.len()));
            continue;
        }
        let row = Row {
            round: cell(0),
            id: &id,
            reviewers: cell(2),
            severity: cell(3),
            significant: cell(4),
            rule: cell(5),
            verdict: cell(6),
            repeat_of: cell(7),
            action: cell(8),
        };
        let values = row.value_problems();
        if values.is_empty() {
            problems.extend(row.rule_problems(&strict));
        } else {
            problems.extend(values);
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = concat!(
        "| Date | Round | Commit | Finding | Reviewers | Severity | Significant | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |\n",
        "| - | - | - | - | - | - | - | - | - | - | - | - | - | - |\n",
    );

    /// A full row from `round | finding | reviewers | severity | significant
    /// | rule | verdict | repeat of | action`.
    fn row(cells: &str) -> String {
        let c: Vec<&str> = cells.splitn(9, " | ").collect();
        assert_eq!(c.len(), 9, "a fixture row has 9 cells: {cells}");
        format!(
            "| 2026-10-31 | {} | 1234abc | {} | {} | {} | {} | doc | {} | x.rs:1 | a claim | {} | {} | {} |",
            c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7], c[8]
        )
    }

    /// A log with `header` and these rows in its findings table, and a
    /// rounds table after it.
    fn log_with(header: &str, rows: &[String]) -> String {
        format!(
            "# Review log\n\n## Findings\n\n{header}{}\n\n## Rounds\n\n| Round | Agents |\n| - | - |\n| speed 1 | 2 |\n",
            rows.join("\n")
        )
    }

    fn problems_of(rows: &[&str]) -> Vec<String> {
        let rows: Vec<String> = rows.iter().map(|r| row(r)).collect();
        fix_rule_problems(&log_with(HEADER, &rows))
    }

    #[test]
    fn the_strict_rules_are_the_15_ids_in_order() {
        assert_eq!(
            strict_rules(),
            [
                "PROTO-1", "PROTO-2", "MEM-1", "MEM-2", "MEM-3", "MEM-4", "MEM-5", "RT-1", "GPU-1",
                "GPU-2", "GPU-3", "VERIFY-1", "GIT-1", "DEPS-1", "ASSET-1",
            ]
        );
    }

    #[test]
    fn a_log_with_no_significant_column_gives_no_problem() {
        let header = concat!(
            "| Date | Round | Commit | Finding | Reviewers | Severity | Kind | Rule | Where | Claim | Verdict | Repeat of | Action |\n",
            "| - | - | - | - | - | - | - | - | - | - | - | - | - | - |\n",
        );
        let rows = [
            "| 2026-10-31 | speed 1 | abc | 1-1 | reviewer | major | doc | none | x | c | confirmd | none | own PR |".to_string(),
            "| 2026-10-31 | 4 | abc | 4-1 | reviewer | nit | doc | none | x | c | unvalidated | none | abc1234 review round 4 fixes |".to_string(),
            "| too | few |".to_string(),
        ];
        let text = log_with(header, &rows);
        assert!(!has_significant_column(&text));
        assert_eq!(fix_rule_problems(&text), Vec::<String>::new());
        assert_eq!(fix_rule_problems("# Review log\n"), Vec::<String>::new());
    }

    #[test]
    fn a_good_log_gives_no_problem() {
        let mut rows: Vec<String> = [
            "2 | 2-1 | reviewer | should-fix | no | DOC-1 | confirmed | none | abc1234 review round 2 fixes",
            "2 | 2-2 | docs-reviewer | nit | - | none | unvalidated | none | abc1234 review round 2 fixes",
            "2 | 2-3 | reviewer | should-fix | no | none | pre-existing | none | no change: its own PR",
            "2 | 2-4 | reviewer | should-fix | no | none | pre-existing | none | TD-39",
            "3 | 3-1 | docs-reviewer | nit | - | none | unvalidated | none | abc1234 review round 3 fixes",
            "3 | 3-2 | docs-reviewer | should-fix | no | DOC-1 | confirmed | none | abc1234 review round 3 fixes",
            "4 | 4-1 | reviewer | blocker | no | none | confirmed | none | def5678 review round 4 fixes",
            "4 | 4-2 | reviewer | should-fix | yes | none | confirmed | none | def5678 review round 4 fixes",
            "4 | 4-3 | reviewer | should-fix | no | VERIFY-1 | confirmed | none | def5678 review round 4 fixes",
            "4 | 4-4 | reviewer, cargo-mutants | should-fix | no | none | confirmed | none | def5678 review round 4 fixes",
            "4 | 4-5 | docs-reviewer | nit | - | none | unvalidated | none | reported, not fixed",
            "4 | 4-6 | docs-reviewer | should-fix | no | DOC-1 | confirmed | none | reported, not fixed",
            "4 | 4-7 | reviewer | should-fix | no | VERIFY-1 | confirmed | none | TD-39",
            "4 | 4-8 | reviewer | blocker | no | none | unproven | none | def5678 review round 4 fixes",
            "4 | 4-9 | reviewer | should-fix | yes | none | pre-existing | none | def5678 review round 4 fixes",
            "4 | 4-10 | baseline | nit | - | VERIFY-1 | confirmed | none | def5678 review round 4 fixes",
            "4 | 4-11 | memory-reviewer | should-fix | no | MEM-1, MEM-5 | confirmed | none | def5678 review round 4 fixes",
            "4 | 4-12 | gpu-reviewer | should-fix | no | GPU-2 (a quote) | confirmed | none | def5678 review round 4 fixes",
            "4 | 4-13 | reviewer | should-fix | no | VERIFY-1 | confirmed | none | TD-39; def5678 review round 4 fixes",
            "4 | 4-14 | reviewer | should-fix | no | none | unproven | none | no change: the since diff does not touch it",
            "4 | 4-15 | cargo-mutants | should-fix | no | VERIFY-1 | confirmed | none | merged into 4-1; def5678 review round 4 fixes",
            "4 | 4-16 | cargo-mutants | should-fix | - | VERIFY-1 | known | TD-39 | known: TD-39",
            "4 | 4-17 | reviewer | should-fix | - | none | refuted | none | refuted",
            "4 | 4-18 | reviewer | should-fix | yes | none | confirmed | none | the orchestrator's commit of m3-r4.diff",
            "4 | 4-20 | reviewer | should-fix | no | VERIFY-1; D9 a quote | confirmed | none | def5678 review round 4 fixes",
            "4 | 4-21 | reviewer | blocker | no | none | pre-existing | none | no change: its own PR",
            "4 | 4-22 | reviewer | should-fix | no | VERIFY-1 | pre-existing | none | TD-39",
            "4 | 4-23 | cargo-mutants | should-fix | - | VERIFY-1 | known | TD-39 | merged into 4-1; known: TD-39",
        ]
        .iter()
        .map(|r| row(r))
        .collect();
        rows.push("| 2026-10-31 | 4 | 1234abc | 4-19 | reviewer | should-fix | yes | doc | none | x.rs:1 | replace && with \\|\\| in note_run | confirmed | none | def5678 review round 4 fixes |".to_string());
        let text = log_with(HEADER, &rows);
        assert!(has_significant_column(&text));
        assert_eq!(fix_rule_problems(&text), Vec::<String>::new());
    }

    #[test]
    fn a_late_round_may_not_fix_a_nit_or_a_should_fix_that_is_not_significant() {
        assert_eq!(
            problems_of(&[
                "4 | 4-1 | docs-reviewer | nit | - | none | unvalidated | none | def5678 review round 4 fixes",
                "4 | 4-2 | supply-chain-reviewer | nit | - | VERIFY-1 | unvalidated | none | def5678 review round 4 fixes",
                "4 | 4-3 | docs-reviewer | should-fix | no | DOC-1 | confirmed | none | def5678 review round 4 fixes",
                "5 | 5-1 | docs-reviewer | nit | - | none | unvalidated | none | TD-39; def5678 review round 5 fixes",
                "4 | 4-5 | reviewer | should-fix | no | none | pre-existing | none | def5678 review round 4 fixes",
            ]),
            [
                "4-1: round 4 must not fix this finding, but the action is `def5678 review round 4 fixes`.  Fix: undo the fix, and write `reported, not fixed` (decision 0024).",
                "4-2: round 4 must not fix this finding, but the action is `def5678 review round 4 fixes`.  Fix: undo the fix, and write `reported, not fixed` (decision 0024).",
                "4-3: round 4 must not fix this finding, but the action is `def5678 review round 4 fixes`.  Fix: undo the fix, and write `reported, not fixed` (decision 0024).",
                "5-1: round 5 must not fix this finding, but the action is `TD-39; def5678 review round 5 fixes`.  Fix: undo the fix, and write `reported, not fixed` (decision 0024).",
                "4-5: round 4 must not fix this finding, but the action is `def5678 review round 4 fixes`.  Fix: undo the fix, and write `reported, not fixed` (decision 0024).",
            ]
        );
    }

    #[test]
    fn a_round_must_fix_each_finding_that_the_rule_names() {
        assert_eq!(
            problems_of(&[
                "4 | 4-1 | baseline | nit | - | VERIFY-1 | confirmed | none | reported, not fixed",
                "4 | 4-2 | reviewer | should-fix | no | VERIFY-1 | confirmed | none | reported, not fixed",
                "2 | 2-1 | reviewer | should-fix | no | none | confirmed | none | reported, not fixed",
                "3 | 3-1 | reviewer | should-fix | no | none | confirmed | none | reported, not fixed",
                "4 | 4-3 | memory-reviewer | should-fix | no | MEM-1/MEM-2 | confirmed | none | reported, not fixed",
                "4 | 4-4 | reviewer | should-fix | no | RT-1's rule | confirmed | none | reported, not fixed",
                "4 | 4-5 | wire-reviewer | should-fix | no | PROTO-1/PROTO-2 | confirmed | none | reported, not fixed",
                "4 | 4-6 | memory-reviewer | should-fix | no | MEM-1,MEM-5 | confirmed | none | reported, not fixed",
                "4 | 4-7 | reviewer | should-fix | no | VERIFY-1; D9 a quote | confirmed | none | reported, not fixed",
                "2 | 2-2 | reviewer | should-fix | no | none | confirmed | none | no change: later",
                "4 | 4-8 | reviewer | blocker | no | none | confirmed | none | no change: later",
            ]),
            [
                "4-1: round 4 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "4-2: round 4 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "2-1: round 2 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "3-1: round 3 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "4-3: round 4 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "4-4: round 4 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "4-5: round 4 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "4-6: round 4 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "4-7: round 4 must fix this finding, but the action is `reported, not fixed`.  Fix: fix it, or send it to a TD (decision 0024).",
                "2-2: round 2 must fix this finding, but the action is `no change: later`.  Fix: fix it, or send it to a TD (decision 0024).",
                "4-8: round 4 must fix this finding, but the action is `no change: later`.  Fix: fix it, or send it to a TD (decision 0024).",
            ]
        );
    }

    #[test]
    fn a_round_fixes_a_pre_existing_finding_that_the_rule_names_or_gives_the_reason() {
        assert_eq!(
            problems_of(&[
                "2 | 2-1 | reviewer | should-fix | no | none | pre-existing | none | reported, not fixed",
                "2 | 2-2 | baseline | nit | - | none | pre-existing | none | reported, not fixed",
                "4 | 4-1 | reviewer | blocker | no | none | pre-existing | none | reported, not fixed",
                "4 | 4-2 | reviewer | should-fix | no | VERIFY-1 | pre-existing | none | reported, not fixed",
                "4 | 4-3 | reviewer | should-fix | no | none | pre-existing | none | reported, not fixed",
            ]),
            [
                "2-1: round 2 must fix this `pre-existing` finding, but the action is `reported, not fixed`.  Fix: fix it, send it to a TD, or write `no change:` and its own PR (decision 0024).",
                "2-2: round 2 must fix this `pre-existing` finding, but the action is `reported, not fixed`.  Fix: fix it, send it to a TD, or write `no change:` and its own PR (decision 0024).",
                "4-1: round 4 must fix this `pre-existing` finding, but the action is `reported, not fixed`.  Fix: fix it, send it to a TD, or write `no change:` and its own PR (decision 0024).",
                "4-2: round 4 must fix this `pre-existing` finding, but the action is `reported, not fixed`.  Fix: fix it, send it to a TD, or write `no change:` and its own PR (decision 0024).",
            ]
        );
    }

    #[test]
    fn a_known_row_names_its_td_in_the_action_and_in_repeat_of() {
        assert_eq!(
            problems_of(&[
                "4 | 4-1 | cargo-mutants | should-fix | - | VERIFY-1 | known | TD-39 | refuted",
                "4 | 4-2 | cargo-mutants | should-fix | - | VERIFY-1 | known | TD-66 | known: TD-39",
                "4 | 4-3 | cargo-mutants | should-fix | - | VERIFY-1 | known | none | known: none",
                "4 | 4-4 | cargo-mutants | should-fix | - | VERIFY-1 | known | TD-x | known: TD-x",
            ]),
            [
                "4-1: a `known` row needs the action `known: <TD>`, with that TD in Repeat of.  Fix: write the TD in both cells.",
                "4-2: a `known` row needs the action `known: <TD>`, with that TD in Repeat of.  Fix: write the TD in both cells.",
                "4-3: a `known` row needs the action `known: <TD>`, with that TD in Repeat of.  Fix: write the TD in both cells.",
                "4-4: a `known` row needs the action `known: <TD>`, with that TD in Repeat of.  Fix: write the TD in both cells.",
            ]
        );
    }

    #[test]
    fn a_refuted_finding_has_no_fix() {
        assert_eq!(
            problems_of(&[
                "1 | 1-1 | reviewer | should-fix | - | none | refuted | none | abc1234 review round 1 fixes",
                "1 | 1-2 | reviewer | should-fix | - | none | refuted | none | refuted; abc1234 review round 1 fixes",
                "4 | 4-1 | reviewer | should-fix | - | none | refuted | none | def5678 review round 4 fixes",
                "4 | 4-2 | cargo-mutants | should-fix | - | VERIFY-1 | known | TD-39 | known: TD-39; def5678 review round 4 fixes",
            ]),
            [
                "1-1: a `refuted` finding has the fix `abc1234 review round 1 fixes`.  Fix: do not fix it, and write the reason for no change.",
                "1-2: a `refuted` finding has the fix `refuted; abc1234 review round 1 fixes`.  Fix: do not fix it, and write the reason for no change.",
                "4-1: a `refuted` finding has the fix `def5678 review round 4 fixes`.  Fix: do not fix it, and write the reason for no change.",
                "4-2: a `known` finding has the fix `known: TD-39; def5678 review round 4 fixes`.  Fix: do not fix it, and write the reason for no change.",
            ]
        );
    }

    #[test]
    fn an_action_names_a_fix_or_a_reason_for_no_fix() {
        let fix = "Fix: name the fix commit or diff, or write `reported, not fixed`, `refuted`, `known: <TD>`, `no change: <reason>` or a TD ID.";
        assert_eq!(
            problems_of(&[
                "1 | 1-1 | reviewer | should-fix | no | none | pre-existing | none | own PR",
                "1 | 1-2 | cargo-mutants | should-fix | no | VERIFY-1 | pre-existing | none | merged into 1-1",
                "1 | 1-3 | reviewer | should-fix | no | none | pre-existing | none | own PR (defaced)",
                "1 | 1-4 | reviewer | should-fix | no | none | pre-existing | none | own PR #41",
                "1 | 1-5 | reviewer | should-fix | no | none | pre-existing | none | see round 4",
                "1 | 1-6 | reviewer | should-fix | no | none | pre-existing | none | reported, not fixed; see round 4",
                "1 | 1-7 | reviewer | should-fix | no | none | pre-existing | none | no change needed",
                "4 | 4-1 | reviewer | nit | - | none | unvalidated | none | merged into 1-1; reported, not fixed",
            ]),
            [
                format!("1-1: the action `own PR` names no fix and no reason for no fix.  {fix}"),
                format!(
                    "1-2: the action `merged into 1-1` names no fix and no reason for no fix.  {fix}"
                ),
                format!(
                    "1-3: the action `own PR (defaced)` names no fix and no reason for no fix.  {fix}"
                ),
                format!(
                    "1-4: the action `own PR #41` names no fix and no reason for no fix.  {fix}"
                ),
                format!(
                    "1-5: the action `see round 4` names no fix and no reason for no fix.  {fix}"
                ),
                format!(
                    "1-6: the action `reported, not fixed; see round 4` names no fix and no reason for no fix.  {fix}"
                ),
                format!(
                    "1-7: the action `no change needed` names no fix and no reason for no fix.  {fix}"
                ),
            ]
        );
    }

    #[test]
    fn significant_has_a_value_only_on_a_blocker_or_should_fix_that_may_be_real() {
        assert_eq!(
            problems_of(&[
                "2 | 2-1 | reviewer | should-fix | maybe | none | confirmed | none | abc1234 review round 2 fixes",
                "2 | 2-2 | docs-reviewer | nit | yes | none | unvalidated | none | abc1234 review round 2 fixes",
                "2 | 2-3 | reviewer | should-fix | - | none | confirmed | none | abc1234 review round 2 fixes",
                "2 | 2-4 | reviewer | should-fix | - | none | pre-existing | none | abc1234 review round 2 fixes",
                "2 | 2-5 | reviewer | blocker | - | none | unproven | none | abc1234 review round 2 fixes",
            ]),
            [
                "2-1: Significant is `maybe`, not `yes`, `no` or `-`.  Fix: write one of them.",
                "2-2: Significant is `yes`, but only a blocker or should-fix with the verdict `confirmed`, `pre-existing` or `unproven` gets a value.  Fix: write `-`.",
                "2-3: a should-fix with the verdict `confirmed` has `-` in Significant.  Fix: write `yes` or `no`, by the test of decision 0024.",
                "2-4: a should-fix with the verdict `pre-existing` has `-` in Significant.  Fix: write `yes` or `no`, by the test of decision 0024.",
                "2-5: a blocker with the verdict `unproven` has `-` in Significant.  Fix: write `yes` or `no`, by the test of decision 0024.",
            ]
        );
    }

    #[test]
    fn a_row_with_bad_cells_gives_only_those_problems() {
        let mut rows = vec![
            "| 2026-10-31 | 1 | 1234abc | 1-1 | reviewer | nit | - | doc | none | x.rs:1 | a claim | unvalidated | none |".to_string(),
        ];
        rows.extend(
            [
                "speed 1 | 1-2 | reviewer | nit | - | none | unvalidated | none | reported, not fixed",
                "1 | 1-3 | reviewer | major | yes | none | confirmd | none | own PR",
                "0 | 1-4 | reviewer | nit | - | none | unvalidated | none | reported, not fixed",
                "1 |  | reviewer | huge | - | none | unvalidated | none | reported, not fixed",
            ]
            .iter()
            .map(|r| row(r)),
        );
        assert_eq!(
            fix_rule_problems(&log_with(HEADER, &rows)),
            [
                "1-1: the row has 13 cells, and the header has 14.  Fix: give the row one cell for each column, and write a `|` in a cell as `\\|`.",
                "1-2: the round `speed 1` is not a whole number of 1 or more.  Fix: write the number of the round.",
                "1-3: the severity `major` is not `blocker`, `should-fix` or `nit`.  Fix: write one of them.",
                "1-3: the verdict `confirmd` is not a verdict of the review-rounds skill.  Fix: write `confirmed`, `pre-existing`, `refuted`, `unproven`, `unvalidated` or `known`.",
                "1-4: the round `0` is not a whole number of 1 or more.  Fix: write the number of the round.",
                "row 5: the severity `huge` is not `blocker`, `should-fix` or `nit`.  Fix: write one of them.",
            ]
        );
    }

    #[test]
    fn a_header_with_a_missing_column_gives_one_problem_and_no_row_problems() {
        let header = concat!(
            "| Date | Round | Commit | Finding | Reviewers | Severity | Significant | Kind | Rule | Where | Claim | Verdict | Repeat of |\n",
            "| - | - | - | - | - | - | - | - | - | - | - | - | - |\n",
        );
        let rows = ["| 2026-10-31 | speed 1 | abc | 1-1 | reviewer | major | maybe | doc | none | x | c | confirmd | none |".to_string()];
        assert_eq!(
            fix_rule_problems(&log_with(header, &rows)),
            [
                "findings table: the header has no `Action` column.  Fix: use the header of step 11 of the review-rounds skill."
            ]
        );
    }
}
