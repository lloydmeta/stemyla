//! Turns a test that cannot run into a failure where evidence is claimed
//! (VERIFY-1 in `docs/invariants.md`).
//!
//! Stable libtest has no run-time skip: a test that returns early reports
//! `ok`, and `cargo test` hides its stderr.  So a test that has no GPU
//! adapter, no adapter feature, no fixture or no AAC encoder calls
//! [`skip`] before it returns.  [`VAR`] lists what the run claims to have
//! (`STEMYLA_REQUIRE=gpu,timestamps,f16,fixtures,aac`); a skip of a listed
//! need panics instead.  An unknown word panics too, so a typo cannot turn the
//! check off.
//!
//! A native test reads the list from the value of [`VAR`] at run time.  A
//! wasm test has no environment at run time, so it also reads the value of
//! the build of this crate for wasm32.  Cargo builds this crate again when
//! that value changes.  A native build ignores the value of the build, so
//! the value of an earlier build cannot fail a native test.  stderr of a
//! wasm test goes nowhere, so a wasm test logs [`skip_line`] on the console.

use std::ffi::{OsStr, OsString};
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
    /// An adapter with the downlevel flag `SHADER_F16_IN_F32`, so the f16
    /// weight kernels compile (decision 0018).  Implies [`Need::Gpu`].
    F16,
    /// The PyTorch fixtures that `tools/roformer_export.py` writes.
    Fixtures,
    /// A browser with an AAC encoder for WebCodecs (TD-92).
    Aac,
}

const NEEDS: [Need; 5] = [
    Need::Gpu,
    Need::Timestamps,
    Need::F16,
    Need::Fixtures,
    Need::Aac,
];

const ONLY_THE_WORDS: &str =
    "Fix: use only gpu, timestamps, f16, fixtures and aac, separated by commas.";

impl Need {
    /// The word for this need in [`VAR`].
    pub fn word(self) -> &'static str {
        match self {
            Need::Gpu => "gpu",
            Need::Timestamps => "timestamps",
            Need::F16 => "f16",
            Need::Fixtures => "fixtures",
            Need::Aac => "aac",
        }
    }

    fn fix(self) -> &'static str {
        match self {
            Need::Gpu => "run it where a GPU adapter opens",
            Need::Timestamps => "run it on an adapter with timestamp-query",
            Need::F16 => "run it on an adapter with the downlevel flag SHADER_F16_IN_F32",
            Need::Fixtures => "write the fixtures with tools/roformer_export.py",
            Need::Aac => {
                "run it in a browser with an AAC encoder, such as Chrome or Safari on macOS"
            }
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

/// The needs that cannot hold without a GPU adapter, so a `gpu` skip fails
/// when the list has one of them.
const IMPLY_GPU: [Need; 2] = [Need::Timestamps, Need::F16];

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
    } else if let Some(implier) = IMPLY_GPU
        .into_iter()
        .find(|implier| need == Need::Gpu && required.contains(implier))
    {
        let implied = implier.word();
        Err(format!(
            "VERIFY-1: {VAR} has {implied}, which needs gpu, but this test cannot run: {why}.  Fix: {}, or remove {implied} from {VAR}.",
            implier.fix()
        ))
    } else {
        Ok(format!(
            "skipping ({word}): {why}.  Set {VAR}={word} to make this a failure."
        ))
    }
}

/// The list that a run claims: the words of [`VAR`] at build time
/// (`built`) and at run time (`run`).  `None` when neither is set.
pub fn claimed(built: Option<&str>, run: Option<&OsStr>) -> Option<OsString> {
    match (built, run) {
        (None, None) => None,
        (Some(built), None) => Some(OsString::from(built)),
        (None, Some(run)) => Some(run.to_os_string()),
        (Some(built), Some(run)) => {
            let mut both = OsString::from(built);
            both.push(",");
            both.push(run);
            Some(both)
        }
    }
}

/// The value of [`VAR`] when this crate was built for wasm32, where a test
/// has no environment at run time.
#[cfg(target_arch = "wasm32")]
const BUILT: Option<&str> = option_env!("STEMYLA_REQUIRE");

/// A native test reads only the value of its run.
#[cfg(not(target_arch = "wasm32"))]
const BUILT: Option<&str> = None;

/// The skip line of a test that cannot run because it lacks `need`, or a
/// panic when the run requires `need`.  The caller prints or logs the
/// line, then returns.  The panic is in this body, not in a closure, so
/// `#[track_caller]` names the caller's line.
#[track_caller]
pub fn skip_line(need: Need, why: impl Display) -> String {
    let list = claimed(BUILT, std::env::var_os(VAR).as_deref());
    match verdict(list.as_deref(), need, &why.to_string()) {
        Ok(line) => line,
        Err(message) => panic!("{message}"),
    }
}

/// Reports that a test cannot run because it lacks `need`: prints the skip
/// line on stderr, or panics when the run requires `need`.  The caller
/// then returns.
#[track_caller]
pub fn skip(need: Need, why: impl Display) {
    eprintln!("{}", skip_line(need, why));
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

    const UNKNOWN_GPUS: &str = "VERIFY-1: STEMYLA_REQUIRE has the unknown word \"gpus\".  Fix: use only gpu, timestamps, f16, fixtures and aac, separated by commas.";

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
    fn a_word_of_the_build_is_required_with_no_run_time_value() {
        let built = claimed(Some("gpu"), None);
        assert_eq!(
            verdict(built.as_deref(), Need::Gpu, "no GPU adapter: none"),
            Err("VERIFY-1: STEMYLA_REQUIRE has gpu, but this test cannot run: no GPU adapter: none.  Fix: run it where a GPU adapter opens, or remove gpu from STEMYLA_REQUIRE.".into())
        );
        assert_eq!(claimed(None, None), None);
        assert_eq!(claimed(None, list("gpu")), Some(OsString::from("gpu")));
    }

    #[test]
    fn the_words_of_the_build_and_of_the_run_are_both_required() {
        let both = claimed(Some("fixtures"), list("gpu"));
        assert_eq!(both, Some(OsString::from("fixtures,gpu")));
        assert!(verdict(both.as_deref(), Need::Gpu, "none").is_err());
        assert!(verdict(both.as_deref(), Need::Fixtures, "none").is_err());
        assert_eq!(
            verdict(both.as_deref(), Need::Timestamps, "none"),
            skip_line("timestamps", "none")
        );
        assert_eq!(
            verdict(
                claimed(Some("gpus"), None).as_deref(),
                Need::Fixtures,
                "x not found"
            ),
            Err(UNKNOWN_GPUS.into())
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
            Err("VERIFY-1: STEMYLA_REQUIRE has the unknown word \"GPU\".  Fix: use only gpu, timestamps, f16, fixtures and aac, separated by commas.".into())
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
            Err("VERIFY-1: STEMYLA_REQUIRE is not Unicode.  Fix: use only gpu, timestamps, f16, fixtures and aac, separated by commas.".into())
        );
    }

    #[test]
    fn f16_is_a_need_that_requires_a_gpu() {
        assert_eq!(parse("gpu,f16"), Ok(vec![Need::Gpu, Need::F16]));
        assert_eq!(
            verdict(list("f16"), Need::F16, "adapter without SHADER_F16_IN_F32"),
            Err("VERIFY-1: STEMYLA_REQUIRE has f16, but this test cannot run: adapter without SHADER_F16_IN_F32.  Fix: run it on an adapter with the downlevel flag SHADER_F16_IN_F32, or remove f16 from STEMYLA_REQUIRE.".into())
        );
        assert_eq!(
            verdict(list("gpu"), Need::F16, "adapter without SHADER_F16_IN_F32"),
            skip_line("f16", "adapter without SHADER_F16_IN_F32")
        );
        assert_eq!(
            verdict(list("f16"), Need::Gpu, "no GPU adapter: none"),
            Err("VERIFY-1: STEMYLA_REQUIRE has f16, which needs gpu, but this test cannot run: no GPU adapter: none.  Fix: run it on an adapter with the downlevel flag SHADER_F16_IN_F32, or remove f16 from STEMYLA_REQUIRE.".into())
        );
    }

    #[test]
    fn aac_is_the_need_of_an_aac_encoder() {
        assert_eq!(parse("gpu,aac"), Ok(vec![Need::Gpu, Need::Aac]));
        assert_eq!(
            verdict(list("aac"), Need::Aac, "this browser has no AAC encoder"),
            Err("VERIFY-1: STEMYLA_REQUIRE has aac, but this test cannot run: this browser has no AAC encoder.  Fix: run it in a browser with an AAC encoder, such as Chrome or Safari on macOS, or remove aac from STEMYLA_REQUIRE.".into())
        );
        assert_eq!(
            verdict(list("gpu"), Need::Aac, "this browser has no AAC encoder"),
            skip_line("aac", "this browser has no AAC encoder")
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
