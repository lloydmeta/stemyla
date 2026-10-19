//! Turns a test that cannot run into a failure where evidence is claimed
//! (VERIFY-1 in `docs/invariants.md`).
//!
//! Stable libtest has no run-time skip: a test that returns early reports
//! `ok`, and `cargo test` hides its stderr.  So a test that has no GPU
//! adapter, no adapter feature or no fixture calls [`skip`] before it
//! returns.  [`VAR`] lists what the run claims to have
//! (`STEMYLA_REQUIRE=gpu,timestamps,fixtures`); a skip of a listed need
//! panics instead.  An unknown word panics too, so a typo cannot turn the
//! check off.

use std::ffi::OsStr;
use std::fmt::Display;

/// The environment variable that lists the needs a run requires.
pub const VAR: &str = "STEMYLA_REQUIRE";

/// What a test can lack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// A GPU adapter and device (`Gpu::new` succeeds), software or not.
    Gpu,
    /// An adapter with `timestamp-query`.  Implies [`Need::Gpu`].
    Timestamps,
    /// The PyTorch fixtures that `tools/roformer_export.py` writes.
    Fixtures,
}

const NEEDS: [Need; 3] = [Need::Gpu, Need::Timestamps, Need::Fixtures];

const ONLY_THE_WORDS: &str = "Fix: use only gpu, timestamps and fixtures, separated by commas.";

impl Need {
    /// The word for this need in [`VAR`].
    pub fn word(self) -> &'static str {
        match self {
            Need::Gpu => "gpu",
            Need::Timestamps => "timestamps",
            Need::Fixtures => "fixtures",
        }
    }

    fn fix(self) -> &'static str {
        match self {
            Need::Gpu => "run it where a GPU adapter opens",
            Need::Timestamps => "run it on an adapter with timestamp-query",
            Need::Fixtures => "write the fixtures with tools/roformer_export.py",
        }
    }
}

/// The words of a list, or the message for the first unknown word.  Items
/// are trimmed, empty items are ignored, and repeats are allowed.
pub fn parse(list: &str) -> Result<Vec<Need>, String> {
    list.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            NEEDS
                .into_iter()
                .find(|need| need.word() == item)
                .ok_or_else(|| {
                    format!("VERIFY-1: {VAR} has the unknown word \"{item}\".  {ONLY_THE_WORDS}")
                })
        })
        .collect()
}

/// Ok: the line a skip prints.  Err: the message a required skip panics
/// with.  `list` is the value of [`VAR`], `None` when it is unset.
pub fn verdict(list: Option<&OsStr>, need: Need, why: &str) -> Result<String, String> {
    let required = match list {
        None => Vec::new(),
        Some(value) => parse(
            value
                .to_str()
                .ok_or_else(|| format!("VERIFY-1: {VAR} is not Unicode.  {ONLY_THE_WORDS}"))?,
        )?,
    };
    let why = why.strip_suffix('.').unwrap_or(why);
    let word = need.word();
    if required.contains(&need) {
        Err(format!(
            "VERIFY-1: {VAR} has {word}, but this test cannot run: {why}.  Fix: {}, or remove {word} from {VAR}.",
            need.fix()
        ))
    } else if need == Need::Gpu && required.contains(&Need::Timestamps) {
        Err(format!(
            "VERIFY-1: {VAR} has timestamps, which needs gpu, but this test cannot run: {why}.  Fix: {}, or remove timestamps from {VAR}.",
            Need::Timestamps.fix()
        ))
    } else {
        Ok(format!(
            "skipping ({word}): {why}.  Set {VAR}={word} to make this a failure."
        ))
    }
}

/// Reports that a test cannot run because it lacks `need`: prints the skip
/// line, or panics when [`VAR`] requires `need`.  The caller then returns.
/// The panic is in this body, not in a closure, so `#[track_caller]` names
/// the caller's line.
#[track_caller]
pub fn skip(need: Need, why: impl Display) {
    match verdict(std::env::var_os(VAR).as_deref(), need, &why.to_string()) {
        Ok(line) => eprintln!("{line}"),
        Err(message) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(value: &str) -> Option<&OsStr> {
        Some(OsStr::new(value))
    }

    fn skip_line(word: &str, why: &str) -> Result<String, String> {
        Ok(format!(
            "skipping ({word}): {why}.  Set STEMYLA_REQUIRE={word} to make this a failure."
        ))
    }

    const UNKNOWN_GPUS: &str = "VERIFY-1: STEMYLA_REQUIRE has the unknown word \"gpus\".  Fix: use only gpu, timestamps and fixtures, separated by commas.";

    #[test]
    fn a_required_need_that_cannot_run_fails() {
        assert_eq!(
            verdict(list("gpu"), Need::Gpu, "no GPU adapter: none"),
            Err("VERIFY-1: STEMYLA_REQUIRE has gpu, but this test cannot run: no GPU adapter: none.  Fix: run it where a GPU adapter opens, or remove gpu from STEMYLA_REQUIRE.".into())
        );
        assert_eq!(
            verdict(list("timestamps"), Need::Timestamps, "no timestamp-query"),
            Err("VERIFY-1: STEMYLA_REQUIRE has timestamps, but this test cannot run: no timestamp-query.  Fix: run it on an adapter with timestamp-query, or remove timestamps from STEMYLA_REQUIRE.".into())
        );
        assert_eq!(
            verdict(list("fixtures"), Need::Fixtures, "x not found"),
            Err("VERIFY-1: STEMYLA_REQUIRE has fixtures, but this test cannot run: x not found.  Fix: write the fixtures with tools/roformer_export.py, or remove fixtures from STEMYLA_REQUIRE.".into())
        );
    }

    #[test]
    fn a_need_not_required_skips_with_a_hint() {
        assert_eq!(
            verdict(list("fixtures"), Need::Gpu, "no GPU adapter: none"),
            skip_line("gpu", "no GPU adapter: none")
        );
        assert_eq!(
            verdict(list("gpu"), Need::Timestamps, "no timestamp-query"),
            skip_line("timestamps", "no timestamp-query")
        );
        assert_eq!(
            verdict(list("gpu,timestamps"), Need::Fixtures, "x not found"),
            skip_line("fixtures", "x not found")
        );
    }

    #[test]
    fn an_unknown_word_fails_even_when_not_needed() {
        assert_eq!(
            verdict(list("gpus"), Need::Fixtures, "x not found"),
            Err(UNKNOWN_GPUS.into())
        );
        assert_eq!(
            verdict(list("fixtures,gpus"), Need::Fixtures, "x not found"),
            Err(UNKNOWN_GPUS.into())
        );
        assert_eq!(parse("gpu,gpus"), Err(UNKNOWN_GPUS.into()));
    }

    #[test]
    fn items_are_trimmed_and_empty_items_ignored() {
        assert_eq!(
            parse(" gpu , ,fixtures,,\ttimestamps ,gpu,"),
            Ok(vec![Need::Gpu, Need::Fixtures, Need::Timestamps, Need::Gpu])
        );
    }

    #[test]
    fn words_are_lowercase_only() {
        assert_eq!(parse("gpu"), Ok(vec![Need::Gpu]));
        assert_eq!(
            parse("GPU"),
            Err("VERIFY-1: STEMYLA_REQUIRE has the unknown word \"GPU\".  Fix: use only gpu, timestamps and fixtures, separated by commas.".into())
        );
        assert!(parse("Fixtures").is_err());
    }

    #[test]
    fn a_full_stop_at_the_end_of_why_is_not_doubled() {
        assert_eq!(
            verdict(None, Need::Gpu, "GPU device: Not enough memory left."),
            skip_line("gpu", "GPU device: Not enough memory left")
        );
        assert_eq!(
            verdict(list("gpu"), Need::Gpu, "GPU device: Not enough memory left."),
            Err("VERIFY-1: STEMYLA_REQUIRE has gpu, but this test cannot run: GPU device: Not enough memory left.  Fix: run it where a GPU adapter opens, or remove gpu from STEMYLA_REQUIRE.".into())
        );
    }

    #[test]
    fn an_unset_or_empty_list_requires_nothing() {
        for value in [None, list(""), list(" , ")] {
            assert_eq!(
                verdict(value, Need::Gpu, "no GPU adapter: none"),
                skip_line("gpu", "no GPU adapter: none"),
                "{value:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_list_that_is_not_unicode_fails() {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            verdict(Some(OsStr::from_bytes(b"gpu\xff")), Need::Fixtures, "x not found"),
            Err("VERIFY-1: STEMYLA_REQUIRE is not Unicode.  Fix: use only gpu, timestamps and fixtures, separated by commas.".into())
        );
    }

    #[test]
    fn timestamps_requires_a_gpu() {
        assert_eq!(
            verdict(list("timestamps"), Need::Gpu, "no GPU adapter: none"),
            Err("VERIFY-1: STEMYLA_REQUIRE has timestamps, which needs gpu, but this test cannot run: no GPU adapter: none.  Fix: run it on an adapter with timestamp-query, or remove timestamps from STEMYLA_REQUIRE.".into())
        );
        assert_eq!(
            verdict(list("gpu,timestamps"), Need::Gpu, "no GPU adapter: none"),
            Err("VERIFY-1: STEMYLA_REQUIRE has gpu, but this test cannot run: no GPU adapter: none.  Fix: run it where a GPU adapter opens, or remove gpu from STEMYLA_REQUIRE.".into())
        );
        assert_eq!(
            verdict(list("timestamps"), Need::Fixtures, "x not found"),
            skip_line("fixtures", "x not found")
        );
    }
}
