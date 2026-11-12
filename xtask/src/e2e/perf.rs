//! The pure parts of `xtask e2e perf` (M2 of `plans/active/gpu-speed.md`):
//! the result of a run, read from the page's log; the GPU time of a
//! profile run in four groups; the reasons that a run is never a pass;
//! the gate; and the baselines file, `xtask/perf-baselines.json`.
//!
//! The chunk lower quartile is the lower quartile of the times of chunks 2
//! to N of one run, by nearest rank ([`lower_quartile`]).  Chunk 1 is the
//! warm-up.  The result also gives the chunk median.  A baseline is the
//! median of the chunk lower quartiles of the runs of an update.  A run
//! fails the gate when its chunk lower quartile is more than the baseline
//! times (1 + tolerance / 100).  Interference only adds time to a chunk,
//! and a slower kernel adds time to each chunk, so a low quantile is the
//! figure that the noise moves least.

use super::{Opts, chunk_done, number_after};
use crate::drivers::Browser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The lowest tolerance of a baseline, in percent.
pub(super) const MIN_TOLERANCE_PCT: f64 = 10.0;

/// The fewest runs that an update of a baseline takes.
pub(super) const MIN_RUNS: usize = 5;

/// The query keys (`stem_protocol::Settings::from_query`) that change the
/// work of a separation or the kernel choice: the chunk, its overlap, the
/// GPU step budget, the kernels and the weight format.  `bench` and
/// `selftest` make the page run a bench or a self-test at the model load,
/// which comes before the timed separation.  [`NOT_MEASURED`] gives each
/// other key, with its reason.
const MEASURE_KEYS: [&str; 8] = [
    "chunk",
    "overlap",
    "max_elems",
    "mm",
    "attn",
    "weights",
    "bench",
    "selftest",
];

/// The query keys of `stem_protocol::Settings::from_query` that do not
/// change what a run measures, each with the reason.
/// `each_query_key_is_measured_or_not` keeps the two lists and the
/// protocol the same.
#[cfg(test)]
const NOT_MEASURED: [(&str, &str); 6] = [
    ("step_limit", "only a time-out for a GPU step"),
    (
        "bench_rounds",
        "the rounds of `?bench`, which `bench` refuses; calibration has its own rounds",
    ),
    (
        "bench_quick",
        "the warm-up of `?bench`, which `bench` refuses; calibration has its own",
    ),
    ("gpu", "picks the adapter, whose text is in the key"),
    ("model_url", "where Download model fetches the model from"),
    ("profile", "a profile run is never gated (`gate`)"),
];

/// A small slack for float compares of seconds and percents.  A power of
/// two (2^-30), so a test can add it to a value exactly.
const EPSILON: f64 = 1.0 / 1_073_741_824.0;

/// GPU seconds of a profile run, in four groups, and their total.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Groups {
    /// Matmuls, softmaxes and fused attentions over the time axis.
    pub time_attention: f64,
    /// The same over the frequency axis.
    pub freq_attention: f64,
    /// Each other matmul.
    pub linear: f64,
    /// Each other kernel.
    pub other: f64,
    /// The `total GPU time` line.
    pub total: f64,
}

/// What one run of `e2e perf` measured.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct PerfResult {
    /// `chrome` or `safari`.
    pub browser: String,
    /// The browser's version, from the WebDriver session.
    pub version: String,
    /// The adapter text of the `model loaded` line.
    pub adapter: String,
    /// `yes`, `no` or `unknown`: the adapter's `isFallbackAdapter`.
    pub fallback: String,
    /// The choice of the `kernels:` line: `matmul v64,v64, attention freq`.
    pub kernels: String,
    /// `f16` or `f32`.
    pub weights: String,
    /// The time of each chunk, in seconds, chunk 1 first.
    pub chunks: Vec<f64>,
    /// The median of chunks 2 to N.
    pub chunk_median: f64,
    /// The lower quartile of chunks 2 to N ([`lower_quartile`]): the
    /// figure that the gate and the baselines read.
    pub chunk_lower_quartile: f64,
    /// The whole separation, in seconds.
    pub whole_secs: f64,
    /// The matmul rate of the whole separation, in TFLOP/s.
    pub tflops: f64,
    /// The GPU time by group, for a run with `profile=1` only.
    pub groups: Option<Groups>,
}

/// The browser, the adapter text and the host's chip: one baseline each.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Key {
    pub browser: String,
    pub adapter: String,
    pub chip: String,
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}, {}, {}", self.browser, self.adapter, self.chip)
    }
}

/// One run of an update.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Run {
    /// 1 for the first run of the update.
    pub order: u32,
    pub kernels: String,
    pub chunk_lower_quartile: f64,
}

/// The baseline of one key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Entry {
    pub key: Key,
    /// The browser version of the runs.
    pub version: String,
    /// The day of the update, `YYYY-MM-DD`.
    pub date: String,
    /// The short SHA of `HEAD` at the update.
    pub commit: String,
    pub runs: Vec<Run>,
    /// The median of the runs' chunk lower quartiles, in seconds.
    pub baseline: f64,
    pub tolerance_pct: f64,
    /// The reason of a raise over the old tolerance, or empty.
    pub raise: String,
}

/// The gate's verdict on one run, with its text.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Verdict {
    Pass(String),
    Fail(String),
    NotGated(String),
}

/// The text after `key` in the last line of `log` that holds `key`.
fn after_last<'a>(log: &'a [String], key: &str) -> Option<&'a str> {
    log.iter()
        .rev()
        .find_map(|line| line.find(key).map(|at| &line[at + key.len()..]))
}

/// The median of `values`; for an even count, the mean of the middle two.
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// The lower quartile of `values`, by nearest rank: the value at index
/// floor((n - 1) / 4) of the sorted values.  For 10 values, the third
/// fastest.  `values` must not be empty.
fn lower_quartile(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[(sorted.len() - 1) / 4]
}

/// The time of each chunk, chunk 1 first, from the `chunk k/N: X s`
/// lines.  Each chunk of 1 to N must have one line.
fn chunk_times(log: &[String]) -> Result<Vec<f64>, String> {
    let mut found: Vec<(u32, u32, f64)> = Vec::new();
    for line in log {
        let Some(k) = chunk_done(line) else { continue };
        let at = &line[line.find("chunk ").unwrap_or(0)..];
        let Some((_, rest)) = at.split_once('/') else {
            continue;
        };
        let Some((total, _)) = rest.split_once(':') else {
            continue;
        };
        let Ok(total) = total.parse::<u32>() else {
            continue;
        };
        let Some(secs) = number_after(at, &format!("chunk {k}/{total}: ")) else {
            continue;
        };
        found.push((k, total, secs));
    }
    let total = found.last().map_or(0, |f| f.1);
    let times: Vec<f64> = (1..=total)
        .filter_map(|k| found.iter().find(|f| f.0 == k && f.1 == total).map(|f| f.2))
        .collect();
    if total < 2 || times.len() != total as usize {
        return Err(format!(
            "the chunk lines: {} of {total} chunks found, and a median needs chunks 1 to N with N of 2 or more",
            times.len()
        ));
    }
    Ok(times)
}

/// The result of one run, from the page's `log`.  `frames` and `bands` are
/// the tokens of the time axis and of the frequency axis
/// ([`profile_groups`]).  It fails, and names the line, when a line is
/// missing.
pub(super) fn perf_result(
    log: &[String],
    browser: &str,
    version: &str,
    frames: usize,
    bands: usize,
) -> Result<PerfResult, String> {
    let missing = |what: &str| format!("no {what} line in the log");
    let loaded = after_last(log, "separator: model loaded in ").ok_or(missing("model loaded"))?;
    let gpu = &loaded[loaded.find(" (").ok_or(missing("model loaded"))? + 2..];
    let (adapter, rest) = gpu
        .split_once(", fallback adapter ")
        .ok_or("no fallback adapter text in the model loaded line")?;
    let fallback = rest.split(';').next().unwrap_or_default();
    let kernels_line = after_last(log, "separator: kernels: ").ok_or(missing("kernels:"))?;
    let (kernels, source) = kernels_line
        .split_once(" (")
        .ok_or(missing("kernels: (source; format)"))?;
    let weights = source
        .split_once("; ")
        .and_then(|(_, w)| w.split_once(" weights"))
        .map(|(w, _)| w)
        .ok_or(missing("kernels: with a weight format"))?;
    let chunks = chunk_times(log)?;
    let separation = after_last(log, "separator: separation: ").ok_or(missing("separation:"))?;
    let tflops = separation
        .split(" TFLOP/s")
        .next()
        .and_then(|before| before.rsplit(' ').next())
        .and_then(|n| n.parse().ok())
        .ok_or(missing("separation: with a TFLOP/s rate"))?;
    let whole_secs = log
        .iter()
        .rev()
        .find(|l| l.contains("separator: ") && l.contains(" stems in "))
        .and_then(|l| number_after(l, " stems in "))
        .ok_or(missing("stems in"))?;
    Ok(PerfResult {
        browser: browser.to_string(),
        version: version.to_string(),
        adapter: adapter.to_string(),
        fallback: fallback.to_string(),
        kernels: kernels.to_string(),
        weights: weights.to_string(),
        chunk_median: median(&chunks[1..]),
        chunk_lower_quartile: lower_quartile(&chunks[1..]),
        chunks,
        whole_secs,
        tflops,
        groups: profile_groups(log, frames, bands),
    })
}

/// The label and the GPU seconds of a `profile: <label>: <secs> s GPU in`
/// line.
fn profile_entry(line: &str) -> Option<(&str, f64)> {
    let rest = &line[line.find("profile: ")? + "profile: ".len()..];
    let (label, tail) = rest.rsplit_once(": ")?;
    let secs = tail.split_once(" s GPU in ")?.0.parse().ok()?;
    Some((label, secs))
}

/// The shape `AxB` after `kind ` in a label, as numbers.
fn dims<const N: usize>(text: &str) -> Option<[usize; N]> {
    let parts: Vec<usize> = text
        .split('x')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    parts.try_into().ok()
}

/// The axis of a label: `Some(true)` for the time axis, `Some(false)` for
/// the frequency axis, `None` for neither.  `tokens` gives the tokens of
/// the label's sequences, or `None` for a kernel of no sequence.
fn axis(tokens: &[usize], frames: usize, bands: usize) -> Option<bool> {
    if tokens.contains(&frames) {
        Some(true)
    } else if tokens.contains(&bands) {
        Some(false)
    } else {
        None
    }
}

/// The GPU seconds of a profile run in four groups.  Time-axis attention:
/// a matmul whose `m` and (`n` or `k`) are `frames`, a softmax whose
/// `len` is `frames`, a fused attention whose `seq` is `frames`.
/// Frequency-axis attention: the same with `bands`.  Linear products:
/// each other matmul.  Other: each other label.  `None` when the log has
/// no `total GPU time` line.  It reads the labels of `matmul_label`,
/// `softmax_label` and `attention_label` in `roformer-gpu/src/lib.rs`.
pub(super) fn profile_groups(log: &[String], frames: usize, bands: usize) -> Option<Groups> {
    let total = log
        .iter()
        .find_map(|l| number_after(l, "profile: total GPU time "))?;
    let mut g = Groups {
        total,
        ..Groups::default()
    };
    for (label, secs) in log.iter().filter_map(|l| profile_entry(l)) {
        let words: Vec<&str> = label.split(' ').collect();
        let side = match words[..] {
            ["matmul", shape, ..] => match dims::<3>(shape) {
                Some([m, n, k]) if m == frames && (n == frames || k == frames) => Some(true),
                Some([m, n, k]) if m == bands && (n == bands || k == bands) => Some(false),
                _ => {
                    g.linear += secs;
                    continue;
                }
            },
            ["softmax", shape] => dims::<2>(shape).and_then(|[_, len]| axis(&[len], frames, bands)),
            ["attention", "seq", seq, _] => seq
                .parse()
                .ok()
                .and_then(|seq: usize| axis(&[seq], frames, bands)),
            _ => None,
        };
        match side {
            Some(true) => g.time_attention += secs,
            Some(false) => g.freq_attention += secs,
            None => g.other += secs,
        }
    }
    Some(g)
}

/// The reason that a run can never pass the gate or be a baseline, or
/// `None`.
pub(super) fn never_a_pass(result: &PerfResult, o: &Opts) -> Option<String> {
    if result.fallback == "yes" {
        return Some("the adapter is a fallback adapter (GPU-3)".into());
    }
    if result.adapter.to_lowercase().contains("swiftshader") {
        return Some(format!(
            "the adapter {} is swiftshader, a software adapter (GPU-3)",
            result.adapter
        ));
    }
    if o.swiftshader {
        return Some("--swiftshader asks for a software adapter (GPU-3)".into());
    }
    if o.headed && result.browser == "chrome" {
        return Some("--headed: the Chrome baselines are headless".into());
    }
    if result.weights != "f16" {
        return Some(format!(
            "the weights are {}, and the baselines are f16",
            result.weights
        ));
    }
    let query = o.query.as_deref().unwrap_or_default();
    let set: Vec<String> = query
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split('=').next())
        .filter(|key| MEASURE_KEYS.contains(key))
        .map(|key| format!("{key}="))
        .collect();
    if !set.is_empty() {
        return Some(format!(
            "the query sets {}, which changes what the run measures",
            set.join(", ")
        ));
    }
    None
}

/// The gate's verdict on `result` against `baseline`.
pub(super) fn gate(result: &PerfResult, baseline: &Entry) -> Verdict {
    if result.groups.is_some() {
        return Verdict::NotGated(
            "a profile run (profile=1) runs one pass for each dispatch, so its times are not gated"
                .into(),
        );
    }
    let tol = baseline.tolerance_pct;
    let bound = baseline.baseline * (1.0 + tol / 100.0);
    let margin = bound - result.chunk_lower_quartile;
    let mut text = format!(
        "chunk lower quartile {:.2} s (chunk median {:.2} s), baseline {:.2} s, tolerance {tol:.0} %, bound {bound:.2} s, margin {margin:.2} s ({:.1} % of the baseline)",
        result.chunk_lower_quartile,
        result.chunk_median,
        baseline.baseline,
        100.0 * margin / baseline.baseline
    );
    if result.version != baseline.version {
        text.push_str(&format!(
            "\nthe browser version {} is not the baseline's {}",
            result.version, baseline.version
        ));
    }
    if !baseline.runs.iter().any(|r| r.kernels == result.kernels) {
        let theirs: Vec<&str> = baseline.runs.iter().map(|r| r.kernels.as_str()).collect();
        text.push_str(&format!(
            "\nthe kernels {} are not the baseline's ({})",
            result.kernels,
            theirs.join("; ")
        ));
    }
    if result.chunk_lower_quartile < baseline.baseline * (1.0 - tol / 100.0) - EPSILON {
        text.push_str("\nfaster than the baseline by more than the tolerance: update the baseline");
    }
    if result.chunk_lower_quartile > bound + EPSILON {
        Verdict::Fail(text)
    } else {
        Verdict::Pass(text)
    }
}

/// The tolerance of a baseline from the chunk lower quartiles of its runs: the
/// larger of 10 and 2 x (slowest - baseline) / baseline x 100, rounded up
/// to a whole percent.  It refuses fewer than [`MIN_RUNS`] runs.
pub(super) fn tolerance_pct(quartiles: &[f64]) -> Result<f64, String> {
    if quartiles.len() < MIN_RUNS {
        return Err(format!(
            "{} runs: a tolerance needs {MIN_RUNS} or more",
            quartiles.len()
        ));
    }
    Ok(tolerance_of(quartiles))
}

/// The rule of [`tolerance_pct`], for any number of values but none.
fn tolerance_of(quartiles: &[f64]) -> f64 {
    let base = median(quartiles);
    let slowest = quartiles.iter().copied().fold(f64::MIN, f64::max);
    let spread = 2.0 * (slowest - base) / base * 100.0;
    // Round off float noise first, so 12.000000000000002 gives 12.
    let spread = ((spread * 1e6).round() / 1e6).ceil();
    spread.max(MIN_TOLERANCE_PCT)
}

/// The entry of a key from the `results` of an update, in their order.
/// It refuses a profile run, and runs of more than one browser, adapter
/// or browser version.
pub(super) fn new_entry(
    results: &[PerfResult],
    chip: &str,
    date: &str,
    commit: &str,
    raise: &str,
) -> Result<Entry, String> {
    let first = results.first().ok_or("no runs")?;
    if results.iter().any(|r| r.groups.is_some()) {
        return Err("a profile run (profile=1) cannot be a baseline".into());
    }
    let same = |r: &PerfResult| {
        (&r.browser, &r.adapter, &r.version) == (&first.browser, &first.adapter, &first.version)
    };
    if !results.iter().all(same) {
        return Err("the runs differ in browser, adapter or browser version".into());
    }
    let quartiles: Vec<f64> = results.iter().map(|r| r.chunk_lower_quartile).collect();
    let tolerance_pct = tolerance_pct(&quartiles)?;
    Ok(Entry {
        key: Key {
            browser: first.browser.clone(),
            adapter: first.adapter.clone(),
            chip: chip.to_string(),
        },
        version: first.version.clone(),
        date: date.to_string(),
        commit: commit.to_string(),
        runs: results
            .iter()
            .zip(1..)
            .map(|(r, order)| Run {
                order,
                kernels: r.kernels.clone(),
                chunk_lower_quartile: r.chunk_lower_quartile,
            })
            .collect(),
        baseline: median(&quartiles),
        tolerance_pct,
        raise: raise.to_string(),
    })
}

/// The tokens of the time axis and of the frequency axis at the default
/// chunk: the frames of a chunk and the bands.
pub(super) fn perf_axes() -> (usize, usize) {
    let frames = roformer::CHUNK / roformer::HOP + 1;
    let bands = roformer::band_groups().iter().map(|g| g.count).sum();
    (frames, bands)
}

/// The name of `browser` in a key and a file name.
pub(super) fn browser_name(browser: Browser) -> &'static str {
    match browser {
        Browser::Chrome => "chrome",
        Browser::Safari => "safari",
    }
}

/// The first line of a command's output, trimmed, or `unknown`.
pub(super) fn command_text(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| {
            String::from_utf8(out.stdout)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// What the scenario does with one run: print a note, or fail the run.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Outcome {
    Note(String),
    Fail(String),
}

/// The key of `result` on a host with `chip`.
pub(super) fn key_of(result: &PerfResult, chip: &str) -> Key {
    Key {
        browser: result.browser.clone(),
        adapter: result.adapter.clone(),
        chip: chip.to_string(),
    }
}

/// The entry of `key` in `baselines`, if any.
pub(super) fn entry_of<'a>(baselines: &'a [Entry], key: &Key) -> Option<&'a Entry> {
    baselines.iter().find(|e| e.key == *key)
}

/// The reason to refuse `--runs` before any run, or `None`.
pub(super) fn runs_refusal(runs: usize, update: bool) -> Option<String> {
    if runs == 0 {
        return Some("--runs 0 runs nothing".into());
    }
    if update && runs < MIN_RUNS {
        return Some(format!(
            "--update-baselines needs --runs {MIN_RUNS} or more, not {runs}"
        ));
    }
    None
}

/// What the scenario does with `result`, whose key is `key`.  `never` is
/// the reason of [`never_a_pass`], `baseline` the entry of the key,
/// `gated` the option `--gate` and `update` the option
/// `--update-baselines`.
pub(super) fn outcome(
    result: &PerfResult,
    key: &Key,
    never: Option<&str>,
    baseline: Option<&Entry>,
    gated: bool,
    update: bool,
) -> Outcome {
    match (never, baseline) {
        (Some(reason), _) if gated || update => Outcome::Fail(format!("never a pass: {reason}")),
        (Some(reason), _) => Outcome::Note(format!("not gated: {reason}")),
        (None, None) if gated => Outcome::Fail(format!("no baseline for {key}: not gated")),
        (None, None) => Outcome::Note(format!("no baseline for {key}: not gated")),
        (None, Some(entry)) => match gate(result, entry) {
            Verdict::Pass(text) => Outcome::Note(format!("the gate passed: {text}")),
            Verdict::Fail(text) if update => {
                Outcome::Note(format!("over the old bound, for the update: {text}"))
            }
            Verdict::Fail(text) => Outcome::Fail(format!("the gate: {text}")),
            Verdict::NotGated(text) => Outcome::Note(format!("not gated: {text}")),
        },
    }
}

/// The problems of one entry of the baselines file, or none: fewer than
/// [`MIN_RUNS`] runs, a baseline that is not the median of the runs'
/// chunk lower quartiles, or a tolerance that is not the rule's for them.
pub(super) fn entry_problems(e: &Entry) -> Vec<String> {
    let quartiles: Vec<f64> = e.runs.iter().map(|r| r.chunk_lower_quartile).collect();
    let mut problems = Vec::new();
    if e.runs.len() < MIN_RUNS {
        problems.push(format!("{} runs, fewer than {MIN_RUNS}", e.runs.len()));
    }
    if !e.runs.is_empty() && (e.baseline - median(&quartiles)).abs() > EPSILON {
        problems.push(format!(
            "the baseline {} is not the median of the runs, {}",
            e.baseline,
            median(&quartiles)
        ));
    }
    let want = if quartiles.is_empty() {
        MIN_TOLERANCE_PCT
    } else {
        tolerance_of(&quartiles)
    };
    if (want - e.tolerance_pct).abs() > EPSILON {
        problems.push(format!(
            "the tolerance {} % is not the rule's {want} % for the runs",
            e.tolerance_pct
        ));
    }
    problems
}

/// The committed baselines file.
pub(super) fn baselines_path() -> PathBuf {
    crate::workspace_root().join("xtask/perf-baselines.json")
}

/// The entries of a baselines file.
pub(super) fn read_baselines(text: &str) -> Result<Vec<Entry>, String> {
    serde_json::from_str(text).map_err(|e| format!("the baselines file: {e}"))
}

/// The text of a baselines file, with a newline at the end.
pub(super) fn write_baselines(entries: &[Entry]) -> String {
    let mut text = serde_json::to_string_pretty(entries).unwrap_or_default();
    text.push('\n');
    text
}

/// `baselines` with `entry` in the place of the entry of its key, or at
/// the end for a new key.
pub(super) fn with_entry(baselines: Vec<Entry>, entry: Entry) -> Vec<Entry> {
    let mut out = baselines;
    match out.iter_mut().find(|e| e.key == entry.key) {
        Some(old) => *old = entry,
        None => out.push(entry),
    }
    out
}

/// The bound of an entry: its baseline times (1 + tolerance / 100).
fn bound_of(e: &Entry) -> f64 {
    e.baseline * (1.0 + e.tolerance_pct / 100.0)
}

/// The reason to refuse an update from `old` to `new`, or `None`.  An
/// update may not raise the bound, the baseline times (1 + tolerance /
/// 100), unless the browser version differs from the old entry's, or
/// `raise` gives a reason.  So a wider tolerance is a raise too.
pub(super) fn update_refusal(old: Option<&Entry>, new: &Entry, raise: &str) -> Option<String> {
    let old = old?;
    let (old_bound, new_bound) = (bound_of(old), bound_of(new));
    if new_bound <= old_bound + EPSILON || new.version != old.version || !raise.trim().is_empty() {
        return None;
    }
    Some(format!(
        "the new bound {new_bound:.2} s ({:.2} s + {:.0} %) is over the old bound {old_bound:.2} s ({:.2} s + {:.0} %) with the same browser version {}: give --raise \"<reason>\" to raise it",
        new.baseline, new.tolerance_pct, old.baseline, old.tolerance_pct, old.version
    ))
}

/// The line that an update prints: the key, and the old and the new
/// baseline, tolerance, bound and browser version.
pub(super) fn update_line(old: Option<&Entry>, new: &Entry) -> String {
    let figures = |e: &Entry| {
        format!(
            "{:.2} s, tolerance {:.0} %, bound {:.2} s, browser {}",
            e.baseline,
            e.tolerance_pct,
            bound_of(e),
            e.version
        )
    };
    format!(
        "the baseline of {}: {}, from {} runs (was {})",
        new.key,
        figures(new),
        new.runs.len(),
        old.map_or("none".to_string(), figures)
    )
}

/// The reason to refuse the baselines file at `path` before any run: the
/// first entry with `entry_problems`.
pub(super) fn file_refusal(entries: &[Entry], path: &std::path::Path) -> Option<String> {
    entries.iter().find_map(|e| {
        let problems = entry_problems(e);
        (!problems.is_empty()).then(|| {
            format!(
                "the entry of {} in {} is not one that --update-baselines writes: {}",
                e.key,
                path.display(),
                problems.join("; ")
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES: usize = 1151;
    const BANDS: usize = 62;

    /// The log lines of one run in Chrome, as the page logs them, with
    /// `chunks` as the time of each chunk.
    fn log_of(chunks: &[f64]) -> Vec<String> {
        let n = chunks.len();
        let at = "[1.0s] console.info: [app] app/src/lib.rs:4329: separator: ";
        let mut log = vec![
            format!(
                "{at}kernels for this GPU, f16 weights: matmul v64,v64 (large products: v64 0.79 s), attention freq (time axis: split 1.90 s)"
            ),
            format!("{at}kernels: matmul v64,v64, attention freq (calibrated; f16 weights)"),
            format!(
                "{at}memory: worker wasm 30 MB, model on the GPU 350 MB (f16 weight matrices), GPU buffers live 350 MB"
            ),
            format!(
                "{at}model loaded in 13.4 s, saved in browser storage (apple metal-3, fallback adapter no; own WGSL kernels at WebGPU default limits: invocations/workgroup 256)"
            ),
            format!("{at}chunk 0/{n} after 0.0 s"),
        ];
        for (i, secs) in chunks.iter().enumerate() {
            log.push(format!(
                "{at}chunk {}/{n}: {secs:.1} s; so far 1022 GPU steps, 1.22 TFLOP/s on average (steps after the first of each kind); memory: worker wasm 91 MB",
                i + 1
            ));
        }
        log.push(format!("{at}separation: 11242 GPU steps, longest 0.01 s (layer 9 frequency transformer, frames 880..968), 1.33 TFLOP/s on average (steps after the first of each kind)"));
        log.push(format!("{at}6 stems in 52.0 s"));
        log.push(format!("{at}memory: page wasm 33 MB with the stems shown"));
        log
    }

    fn without(log: &[String], text: &str) -> Vec<String> {
        log.iter().filter(|l| !l.contains(text)).cloned().collect()
    }

    fn result_of(log: &[String]) -> Result<PerfResult, String> {
        perf_result(log, "chrome", "157.0.7000.0", FRAMES, BANDS)
    }

    /// Asserts, for a full log of a run with no profile lines:
    /// * the browser, its version, the adapter text and the fallback value;
    /// * the kernels and the weight format of the `kernels:` line;
    /// * the time of each chunk, the chunk median, the chunk lower
    ///   quartile, the whole time and the matmul rate;
    /// * no profile groups.
    ///
    /// The log has a `separator:` line after the `stems in` line, as the
    /// page logs it.
    #[test]
    fn a_full_log_gives_each_value() {
        let r = result_of(&log_of(&[5.3, 4.7, 4.6, 4.8]));
        assert!(r.is_ok(), "{r:?}");
        let r = r.unwrap();
        assert_eq!(r.browser, "chrome");
        assert_eq!(r.version, "157.0.7000.0");
        assert_eq!(r.adapter, "apple metal-3");
        assert_eq!(r.fallback, "no");
        assert_eq!(r.kernels, "matmul v64,v64, attention freq");
        assert_eq!(r.weights, "f16");
        assert_eq!(r.chunks, [5.3, 4.7, 4.6, 4.8]);
        assert_eq!(r.chunk_median, 4.7);
        assert_eq!(r.chunk_lower_quartile, 4.6);
        assert_eq!(r.whole_secs, 52.0);
        assert_eq!(r.tflops, 1.33);
        assert_eq!(r.groups, None);
    }

    /// Asserts:
    /// * 10 values give the third fastest;
    /// * one of the 10 made much faster (4.7 s to 1.0 s) does not move it;
    /// * 3 values give the fastest, 1 value gives itself;
    /// * 5 values give the second fastest;
    /// * 4 values (4.0, 1.0, 3.0, 2.0) give the fastest, 1.0.
    #[test]
    fn the_lower_quartile_is_the_value_at_a_quarter_by_nearest_rank() {
        let ten = [5.9, 4.7, 5.7, 4.8, 4.7, 5.4, 5.6, 5.2, 4.9, 5.2];
        assert_eq!(lower_quartile(&ten), 4.8);
        let mut outlier = ten;
        outlier[1] = 1.0;
        assert_eq!(lower_quartile(&outlier), 4.8);
        assert_eq!(lower_quartile(&[7.4, 8.2, 7.9]), 7.4);
        assert_eq!(lower_quartile(&[7.4]), 7.4);
        assert_eq!(lower_quartile(&[3.0, 1.0, 5.0, 2.0, 4.0]), 2.0);
        assert_eq!(lower_quartile(&[4.0, 1.0, 3.0, 2.0]), 1.0);
    }

    /// Asserts:
    /// * the median of chunks 2 to 4 (9.0, 4.0, 5.0, 6.0) is 5.0, not 5.5;
    /// * the median of chunks 2 to 5 (9.0, 4.0, 6.0, 5.0, 7.0) is 5.5;
    /// * the lower quartile of chunks 2 to 4 (1.0, 4.0, 5.0, 6.0) is 4.0,
    ///   not 1.0;
    /// * `chunks` keeps chunk 1;
    /// * a log of 2 chunks (5.0, 4.0) gives a chunk median of 4.0.
    #[test]
    fn the_chunk_median_leaves_chunk_1_out() {
        let r = result_of(&log_of(&[9.0, 4.0, 5.0, 6.0])).unwrap();
        assert_eq!(r.chunk_median, 5.0);
        assert_eq!(r.chunks[0], 9.0);
        let r = result_of(&log_of(&[9.0, 4.0, 6.0, 5.0, 7.0])).unwrap();
        assert_eq!(r.chunk_median, 5.5);
        let r = result_of(&log_of(&[1.0, 4.0, 5.0, 6.0])).unwrap();
        assert_eq!(r.chunk_lower_quartile, 4.0);
        let r = result_of(&log_of(&[5.0, 4.0]));
        assert_eq!(r.map(|r| r.chunk_median), Ok(4.0));
    }

    /// Asserts that a log fails, with a message that holds the given text,
    /// when it has:
    /// * no fallback text in the `model loaded` line (`fallback adapter`);
    /// * no `model loaded` line (`model loaded`);
    /// * no `kernels:` line (`kernels:`);
    /// * no line of chunk 3 of 4 (`chunk`);
    /// * a single chunk (`chunk`);
    /// * no `separation:` line (`separation:`);
    /// * no `stems in` line (`stems in`).
    #[test]
    fn a_log_without_a_needed_line_fails() {
        let full = log_of(&[5.3, 4.7, 4.6, 4.8]);
        let no_fallback: Vec<String> = full
            .iter()
            .map(|l| l.replace(", fallback adapter no", ""))
            .collect();
        let cases = [
            (no_fallback, "fallback adapter"),
            (without(&full, "model loaded"), "model loaded"),
            (without(&full, "separator: kernels: "), "kernels:"),
            (without(&full, "chunk 3/4:"), "chunk"),
            (log_of(&[5.0]), "chunk"),
            (without(&full, "separation: "), "separation:"),
            (without(&full, "stems in"), "stems in"),
        ];
        for (log, text) in cases {
            let got = result_of(&log);
            assert!(
                matches!(&got, Err(e) if e.contains(text)),
                "{text}: {got:?}"
            );
        }
    }

    /// A profile line of `label` with `secs`, as `log_profile` logs it.
    fn profile_line(label: &str, secs: f64) -> String {
        format!(
            "[72.6s] console.info: [app] app/src/lib.rs:4329: separator: profile: {label}: {secs:.3} s GPU in 8184 dispatches (16.9%)"
        )
    }

    /// Asserts, for one profile line of each kind:
    /// * the time-axis group holds the scores matmul, the P.v matmul, the
    ///   softmax and the fused attention of 1,151 tokens;
    /// * the frequency-axis group holds the same four of 62 tokens;
    /// * the linear group holds a linear product, the gates' matmul
    ///   (1151x8x256), and a matmul of 62 rows that is not attention
    ///   (62x8x256);
    /// * the other group holds `rmsnorm` and `rope`;
    /// * the total is the `total GPU time` line, and equals the sum of
    ///   the four groups;
    /// * the lines `GPU busy` and `this adapter has no timestamp-query`
    ///   go to no group.
    #[test]
    fn each_profile_line_goes_to_its_group() {
        let mut log = vec![
            profile_line("matmul 1151x1151x64 x8 i64", 1.0),
            profile_line("matmul 1151x64x1151 x8 v64", 2.0),
            profile_line("softmax 9208x1151", 4.0),
            profile_line("attention seq 1151 x4", 8.0),
            profile_line("matmul 62x62x64 x704 v64", 16.0),
            profile_line("matmul 62x64x62 x704 v64", 32.0),
            profile_line("softmax 43648x62", 64.0),
            profile_line("attention seq 62 x88", 128.0),
            profile_line("matmul 5456x1536x256 x1 v64 f16", 256.0),
            profile_line("matmul 1151x8x256 x1 v64 f16", 512.0),
            profile_line("matmul 62x8x256 x1151 v64 f16", 4096.0),
            profile_line("rmsnorm", 1024.0),
            profile_line("rope", 2048.0),
        ];
        log.push("[72.6s] separator: profile: total GPU time 8191.000 s".into());
        log.push("[72.6s] separator: profile: GPU busy 8191.00 s of 8200.00 s of step wall time (0% waiting, submitting or between dispatches); one pass per dispatch while profiling".into());
        log.push(
            "[2.0s] separator: profile: this adapter has no timestamp-query; no GPU times".into(),
        );
        let g = profile_groups(&log, FRAMES, BANDS).unwrap();
        assert_eq!(g.time_attention, 15.0);
        assert_eq!(g.freq_attention, 240.0);
        assert_eq!(g.linear, 4864.0);
        assert_eq!(g.other, 3072.0);
        assert_eq!(g.total, 8191.0);
        assert_eq!(
            g.time_attention + g.freq_attention + g.linear + g.other,
            g.total
        );
    }

    /// Asserts that a log with no profile lines gives no groups.
    #[test]
    fn a_log_without_profile_lines_gives_no_groups() {
        assert_eq!(profile_groups(&log_of(&[5.0, 5.0]), FRAMES, BANDS), None);
    }

    fn opts(args: &[&str]) -> Opts {
        let args: Vec<String> = std::iter::once("perf")
            .chain(args.iter().copied())
            .map(str::to_string)
            .collect();
        Opts::parse(&args).unwrap()
    }

    fn sample() -> PerfResult {
        result_of(&log_of(&[5.3, 4.7, 4.6, 4.8])).unwrap()
    }

    /// Asserts that `never_a_pass` gives a reason that holds the given
    /// text for each of:
    /// * the fallback value `yes`;
    /// * an adapter text with `SwiftShader` (any case);
    /// * `--swiftshader`;
    /// * `--headed` in Chrome;
    /// * f32 weights;
    /// * a query with each of `chunk=`, `overlap=`,
    ///   `max_elems=`, `mm=`, `attn=`, `weights=`, `bench=` and
    ///   `selftest=`;
    /// * the query `?chunk=16384`, with its `?`.
    ///
    /// Asserts that a run on `apple metal-3`, fallback `no`, f16 weights,
    /// gives no reason with no query, with `x=1`, with each of
    /// `step_limit=9`, `bench_rounds=1`, `bench_quick=1`, `gpu=integrated`
    /// and `model_url=m`, and in Safari with `--headed`.
    #[test]
    fn never_a_pass_names_each_reason() {
        let fallback = PerfResult {
            fallback: "yes".into(),
            ..sample()
        };
        let swiftshader = PerfResult {
            adapter: "google vulkan SwiftShader Device (Subzero)".into(),
            ..sample()
        };
        let f32 = PerfResult {
            weights: "f32".into(),
            ..sample()
        };
        let none = opts(&[]);
        let cases = [
            (never_a_pass(&fallback, &none), "fallback"),
            (never_a_pass(&swiftshader, &none), "swiftshader"),
            (
                never_a_pass(&sample(), &opts(&["--swiftshader"])),
                "--swiftshader",
            ),
            (never_a_pass(&sample(), &opts(&["--headed"])), "--headed"),
            (never_a_pass(&f32, &none), "f16"),
            (
                never_a_pass(&sample(), &opts(&["--query", "?chunk=16384"])),
                "chunk=",
            ),
        ];
        for (reason, text) in cases {
            let reason = reason.unwrap_or_default();
            assert!(reason.contains(text), "{text}: {reason:?}");
        }
        let keys = [
            "chunk",
            "overlap",
            "max_elems",
            "mm",
            "attn",
            "weights",
            "bench",
            "selftest",
        ];
        for key in keys {
            let o = opts(&["--query", &format!("x=1&{key}=2")]);
            let reason = never_a_pass(&sample(), &o).unwrap_or_default();
            assert!(reason.contains(&format!("{key}=")), "{key}: {reason:?}");
        }
        assert_eq!(never_a_pass(&sample(), &none), None);
        assert_eq!(never_a_pass(&sample(), &opts(&["--query", "x=1"])), None);
        for query in [
            "step_limit=9",
            "bench_rounds=1",
            "bench_quick=1",
            "gpu=integrated",
            "model_url=m",
        ] {
            let reason = never_a_pass(&sample(), &opts(&["--query", query]));
            assert_eq!(reason, None, "{query}");
        }
        let safari = PerfResult {
            browser: "safari".into(),
            ..sample()
        };
        assert_eq!(never_a_pass(&safari, &opts(&["--headed"])), None);
    }

    fn key() -> Key {
        Key {
            browser: "chrome".into(),
            adapter: "apple metal-3".into(),
            chip: "Apple M2 Pro".into(),
        }
    }

    fn entry(baseline: f64, tolerance_pct: f64) -> Entry {
        Entry {
            key: key(),
            version: "157.0.7000.0".into(),
            date: "2026-11-12".into(),
            commit: "242db28".into(),
            runs: (1..=5)
                .map(|order| Run {
                    order,
                    kernels: "matmul v64,v64, attention freq".into(),
                    chunk_lower_quartile: baseline,
                })
                .collect(),
            baseline,
            tolerance_pct,
            raise: String::new(),
        }
    }

    /// The kind of a verdict, `pass`, `fail` or `not gated`, and its text.
    fn told(v: Verdict) -> (&'static str, String) {
        match v {
            Verdict::Pass(text) => ("pass", text),
            Verdict::Fail(text) => ("fail", text),
            Verdict::NotGated(text) => ("not gated", text),
        }
    }

    /// The sample run with `chunk_lower_quartile`, and its chunk median
    /// of 4.7 s.
    fn with_quartile(chunk_lower_quartile: f64) -> PerfResult {
        PerfResult {
            chunk_lower_quartile,
            ..sample()
        }
    }

    /// Asserts, with a baseline of 5.0 s and a tolerance of 10 %:
    /// * a chunk lower quartile of 5.5 s (the bound) passes, and the text
    ///   gives the chunk lower quartile, the baseline, the tolerance and the
    ///   margin, and no line on the version or the kernels;
    /// * a chunk lower quartile of 5.51 s fails, with a chunk median of
    ///   4.7 s;
    /// * a run with profile groups is not gated, with the reason;
    /// * a run of a new browser version passes, with a line on the version;
    /// * a run of other kernels passes, with a line on the kernels;
    /// * a chunk lower quartile of 4.49 s passes, with the line to update
    ///   the baseline;
    /// * a chunk lower quartile of 4.5 s passes with no such line.
    #[test]
    fn the_gate_fails_only_over_the_bound() {
        let base = entry(5.0, 10.0);
        let (kind, text) = told(gate(&with_quartile(5.5), &base));
        assert_eq!(kind, "pass", "5.5 s: {text}");
        assert!(!text.contains("is not the baseline's"), "{text}");
        for part in [
            "chunk lower quartile 5.50 s",
            "baseline 5.00 s",
            "10 %",
            "margin 0.00 s",
        ] {
            assert!(text.contains(part), "{part}: {text}");
        }
        let (kind, text) = told(gate(&with_quartile(5.51), &base));
        assert_eq!(kind, "fail", "5.51 s: {text}");
        let profiled = PerfResult {
            groups: Some(Groups::default()),
            ..with_quartile(9.0)
        };
        let (kind, text) = told(gate(&profiled, &base));
        assert_eq!(kind, "not gated", "a profile run: {text}");
        assert!(text.contains("profile"), "{text}");
        let newer = PerfResult {
            version: "158.0.7100.0".into(),
            ..with_quartile(5.0)
        };
        let (kind, text) = told(gate(&newer, &base));
        assert_eq!(kind, "pass", "a new version: {text}");
        assert!(text.contains("158.0.7100.0"), "{text}");
        let other = PerfResult {
            kernels: "matmul v64,v64, attention fused".into(),
            ..with_quartile(5.0)
        };
        let (kind, text) = told(gate(&other, &base));
        assert_eq!(kind, "pass", "other kernels: {text}");
        assert!(text.contains("attention fused"), "{text}");
        let (kind, text) = told(gate(&with_quartile(4.49), &base));
        assert_eq!(kind, "pass", "4.49 s: {text}");
        assert!(text.contains("update the baseline"), "{text}");
        let (kind, text) = told(gate(&with_quartile(4.5), &base));
        assert_eq!(kind, "pass", "4.5 s: {text}");
        assert!(!text.contains("update the baseline"), "{text}");
    }

    /// Each value is one run's chunk lower quartile.  Asserts:
    /// * 5 equal values give 10;
    /// * a slowest value 6 % over the median of 5 gives 12;
    /// * a slowest value 4 % over gives 10 (8 is under 10);
    /// * a slowest value 6.1 % over gives 13 (12.2, rounded up);
    /// * 4 values are refused.
    #[test]
    fn the_tolerance_is_twice_the_spread_and_at_least_10() {
        assert_eq!(tolerance_pct(&[5.0; 5]), Ok(10.0));
        assert_eq!(tolerance_pct(&[5.0, 5.0, 5.3, 5.0, 4.9]), Ok(12.0));
        assert_eq!(tolerance_pct(&[5.0, 5.0, 5.2, 5.0, 4.9]), Ok(10.0));
        assert_eq!(tolerance_pct(&[5.0, 5.0, 5.305, 5.0, 4.9]), Ok(13.0));
        assert!(tolerance_pct(&[5.0; 4]).is_err());
    }

    /// Asserts, for 5 runs:
    /// * the entry's key, version, date, commit and raise reason;
    /// * each run's order, kernels and chunk lower quartile, in the order of the
    ///   runs;
    /// * the baseline is the median of the chunk lower quartiles, and the
    ///   tolerance is `tolerance_pct` of them.
    ///
    /// Asserts that it refuses a run with profile groups, and runs of two
    /// adapters.
    #[test]
    fn an_entry_holds_the_runs_and_the_median_of_their_lower_quartiles() {
        let runs: Vec<PerfResult> = [5.0, 5.3, 4.9, 5.1, 5.0]
            .into_iter()
            .map(with_quartile)
            .collect();
        let e = new_entry(&runs, "Apple M2 Pro", "2026-11-12", "242db28", "").unwrap();
        assert_eq!(e.key, key());
        assert_eq!(e.version, "157.0.7000.0");
        assert_eq!(
            (e.date.as_str(), e.commit.as_str()),
            ("2026-11-12", "242db28")
        );
        assert_eq!(e.raise, "");
        let quartiles: Vec<f64> = e.runs.iter().map(|r| r.chunk_lower_quartile).collect();
        assert_eq!(quartiles, [5.0, 5.3, 4.9, 5.1, 5.0]);
        let orders: Vec<u32> = e.runs.iter().map(|r| r.order).collect();
        assert_eq!(orders, [1, 2, 3, 4, 5]);
        assert!(
            e.runs
                .iter()
                .all(|r| r.kernels == "matmul v64,v64, attention freq")
        );
        assert_eq!(e.baseline, 5.0);
        assert_eq!(e.tolerance_pct, 12.0);
        let mut profiled = runs.clone();
        profiled[2].groups = Some(Groups::default());
        assert!(new_entry(&profiled, "Apple M2 Pro", "d", "c", "").is_err());
        let mut mixed = runs;
        mixed[4].adapter = "apple apple apple".into();
        assert!(new_entry(&mixed, "Apple M2 Pro", "d", "c", "").is_err());
    }

    /// Asserts that an entry written and read again keeps each field.
    #[test]
    fn baselines_keep_each_field_through_a_round_trip() {
        let mut e = entry(4.7, 12.0);
        e.raise = "Chrome 158 is slower".into();
        let text = write_baselines(&[e.clone()]);
        assert_eq!(read_baselines(&text), Ok(vec![e]));
    }

    /// Asserts that `with_entry`:
    /// * replaces the entry of the same key, in its place;
    /// * keeps the entries of other keys;
    /// * adds an entry of a new key at the end.
    #[test]
    fn with_entry_replaces_only_the_entry_of_its_key() {
        let chrome = entry(4.7, 10.0);
        let mut safari = entry(7.4, 10.0);
        safari.key.browser = "safari".into();
        let newer = entry(4.5, 11.0);
        let got = with_entry(vec![chrome, safari.clone()], newer.clone());
        assert_eq!(got, vec![newer.clone(), safari.clone()]);
        let mut m1 = entry(9.0, 10.0);
        m1.key.chip = "Apple M1".into();
        let got = with_entry(vec![newer.clone(), safari.clone()], m1.clone());
        assert_eq!(got, vec![newer, safari, m1]);
    }

    /// Asserts, for the committed `xtask/perf-baselines.json`:
    /// * it reads as a list of entries;
    /// * each entry has no `entry_problems`;
    /// * no value holds `/Users/`, `/home/` or `~/`.
    #[test]
    fn the_committed_baselines_hold_tolerances_and_no_machine_path() {
        let text = std::fs::read_to_string(baselines_path()).unwrap();
        let entries = read_baselines(&text);
        assert!(entries.is_ok(), "{entries:?}");
        for e in entries.iter().flatten() {
            assert_eq!(entry_problems(e), Vec::<String>::new(), "{}", e.key);
        }
        for path in ["/Users/", "/home/", "~/"] {
            assert!(!text.contains(path), "{path}");
        }
    }

    /// Asserts, with an old baseline of 5.0 s and a tolerance of 10 % (a
    /// bound of 5.5 s):
    /// * a lower baseline (4.0 s, bound 4.4 s) passes;
    /// * the same baseline and tolerance (the same bound) pass;
    /// * the same baseline with a tolerance of 20 % (bound 6.0 s) is
    ///   refused;
    /// * a baseline of 5.6 s is refused, and the reason names `--raise`;
    /// * the same 5.6 s passes with a new browser version, or with a
    ///   reason;
    /// * the same 5.6 s is refused with a blank reason (`"  "`);
    /// * any baseline passes with no old entry.
    #[test]
    fn a_raise_over_the_tolerance_needs_a_reason() {
        let old = entry(5.0, 10.0);
        assert_eq!(update_refusal(Some(&old), &entry(4.0, 10.0), ""), None);
        assert_eq!(update_refusal(Some(&old), &entry(5.0, 10.0), ""), None);
        let wider = update_refusal(Some(&old), &entry(5.0, 20.0), "");
        assert!(wider.is_some(), "a wider tolerance raises the bound");
        let reason = update_refusal(Some(&old), &entry(5.6, 10.0), "").unwrap_or_default();
        assert!(reason.contains("--raise"), "{reason:?}");
        let newer = Entry {
            version: "158.0.7100.0".into(),
            ..entry(5.6, 10.0)
        };
        assert_eq!(update_refusal(Some(&old), &newer, ""), None);
        assert_eq!(
            update_refusal(Some(&old), &entry(5.6, 10.0), "a slower kernel"),
            None
        );
        assert!(update_refusal(Some(&old), &entry(5.6, 10.0), "  ").is_some());
        assert_eq!(update_refusal(None, &entry(9.0, 10.0), ""), None);
    }

    /// Asserts that `entry_problems`:
    /// * finds none in an entry made by `new_entry` from 5 runs;
    /// * finds 3 in a hand edit of that entry to a baseline of 6.5 s, a
    ///   tolerance of 40 % and 1 run: too few runs, a baseline that is not
    ///   the median of the runs, and a tolerance that is not
    ///   `tolerance_pct` of them;
    /// * finds 1 in an entry with a tolerance of 9 %;
    /// * finds none when the baseline is off the median by `EPSILON`, and 1
    ///   when it is off by 2 x `EPSILON`.
    #[test]
    fn entry_problems_finds_a_hand_edit() {
        let runs: Vec<PerfResult> = [5.0, 5.3, 4.9, 5.1, 5.0]
            .into_iter()
            .map(with_quartile)
            .collect();
        let good = new_entry(&runs, "Apple M2 Pro", "2026-11-12", "242db28", "").unwrap();
        assert_eq!(entry_problems(&good), Vec::<String>::new());
        let mut edited = good.clone();
        edited.baseline = 6.5;
        edited.tolerance_pct = 40.0;
        edited.runs.truncate(1);
        assert_eq!(
            entry_problems(&edited).len(),
            3,
            "{:?}",
            entry_problems(&edited)
        );
        let mut low = entry(5.0, 9.0);
        low.raise = String::new();
        assert_eq!(entry_problems(&low).len(), 1, "{:?}", entry_problems(&low));
        let mut near = entry(4.0, 10.0);
        near.baseline = 4.0 + EPSILON;
        assert_eq!(entry_problems(&near), Vec::<String>::new());
        near.baseline = 4.0 + 2.0 * EPSILON;
        assert_eq!(
            entry_problems(&near).len(),
            1,
            "{:?}",
            entry_problems(&near)
        );
    }

    /// Asserts the text of a key: browser, adapter text and chip.
    #[test]
    fn a_key_shows_its_three_parts() {
        assert_eq!(key().to_string(), "chrome, apple metal-3, Apple M2 Pro");
    }

    /// Asserts:
    /// * `perf_axes` gives 1,151 frames and 62 bands;
    /// * `browser_name` gives `chrome` and `safari`;
    /// * `command_text` gives the first line of the output, trimmed;
    /// * `command_text` gives `unknown` for a command that prints a line
    ///   and fails, and for a first line of spaces.
    #[test]
    fn the_scenario_helpers_give_their_values() {
        assert_eq!(perf_axes(), (1151, 62));
        assert_eq!(browser_name(Browser::Chrome), "chrome");
        assert_eq!(browser_name(Browser::Safari), "safari");
        assert_eq!(command_text("printf", &["  lloyd  \\nacme\\n"]), "lloyd");
        assert_eq!(command_text("sh", &["-c", "echo acme; exit 1"]), "unknown");
        assert_eq!(command_text("printf", &["   \\n"]), "unknown");
    }

    /// Asserts that `runs_refusal`:
    /// * refuses `--runs 0`, also with `--update-baselines`;
    /// * refuses 4 runs with `--update-baselines`;
    /// * accepts 5 runs with `--update-baselines`, and 1 run without it.
    #[test]
    fn the_run_count_is_checked_before_any_run() {
        assert!(runs_refusal(0, false).is_some());
        assert!(runs_refusal(0, true).is_some());
        assert!(runs_refusal(4, true).is_some());
        assert_eq!(runs_refusal(5, true), None);
        assert_eq!(runs_refusal(1, false), None);
    }

    /// Asserts that:
    /// * `key_of` gives the browser and adapter of the run, and the chip;
    /// * `entry_of` gives no entry for a Chrome run when the file holds only
    ///   the Safari entry;
    /// * `entry_of` gives the entry of its own key from a file of both;
    /// * the key of an update's new entry finds its old entry.
    #[test]
    fn a_run_reads_only_the_entry_of_its_key() {
        let chrome = entry(4.7, 10.0);
        let mut safari = entry(7.4, 10.0);
        safari.key.browser = "safari".into();
        let run_key = key_of(&sample(), "Apple M2 Pro");
        assert_eq!(run_key, key());
        assert_eq!(entry_of(std::slice::from_ref(&safari), &run_key), None);
        let both = [safari.clone(), chrome.clone()];
        assert_eq!(entry_of(&both, &run_key), Some(&chrome));
        let new_safari = entry(7.3, 10.0);
        let new_safari = Entry {
            key: safari.key.clone(),
            ..new_safari
        };
        assert_eq!(entry_of(&both, &new_safari.key), Some(&safari));
    }

    /// The kind of an outcome, `note` or `fail`, and its text.
    fn kind_of(o: Outcome) -> (&'static str, String) {
        match o {
            Outcome::Note(text) => ("note", text),
            Outcome::Fail(text) => ("fail", text),
        }
    }

    /// Asserts, with a baseline of 5.0 s and a tolerance of 10 %:
    /// * a run that is never a pass fails with `--gate`, fails with
    ///   `--update-baselines`, and gives a note with neither;
    /// * a run with no baseline fails with `--gate`, and gives a note
    ///   without it, with the key in the text;
    /// * a run of 5.0 s gives the note "the gate passed";
    /// * a run of 6.0 s fails, and gives a note with `--update-baselines`;
    /// * a profile run of 9.0 s gives a note, also with `--gate`.
    #[test]
    fn each_outcome_follows_the_options() {
        let base = entry(5.0, 10.0);
        let k = key();
        let run = with_quartile(5.0);
        let never = Some("the adapter is a fallback adapter (GPU-3)");
        let case = |r: &PerfResult, never, baseline, gated, update| {
            kind_of(outcome(r, &k, never, baseline, gated, update))
        };
        assert_eq!(case(&run, never, Some(&base), true, false).0, "fail");
        assert_eq!(case(&run, never, Some(&base), false, true).0, "fail");
        assert_eq!(case(&run, never, Some(&base), false, false).0, "note");
        let (kind, text) = case(&run, None, None, true, false);
        assert_eq!(kind, "fail", "{text}");
        assert!(
            text.contains("chrome, apple metal-3, Apple M2 Pro"),
            "{text}"
        );
        assert_eq!(case(&run, None, None, false, false).0, "note");
        let (kind, text) = case(&run, None, Some(&base), true, false);
        assert_eq!(kind, "note", "{text}");
        assert!(text.contains("the gate passed"), "{text}");
        let slow = with_quartile(6.0);
        assert_eq!(case(&slow, None, Some(&base), true, false).0, "fail");
        assert_eq!(case(&slow, None, Some(&base), false, false).0, "fail");
        assert_eq!(case(&slow, None, Some(&base), false, true).0, "note");
        let profiled = PerfResult {
            groups: Some(Groups::default()),
            ..with_quartile(9.0)
        };
        assert_eq!(case(&profiled, None, Some(&base), true, false).0, "note");
    }

    /// The keys of the `field::X =>` arms of `Settings::from_query` in
    /// `protocol` (the text of `protocol/src/lib.rs`), resolved through
    /// its `mod field`, that are in neither `MEASURE_KEYS` nor
    /// `NOT_MEASURED`.  Also each entry of the two lists that is not a key.
    fn unlisted_query_keys(protocol: &str) -> Result<Vec<String>, String> {
        let field = protocol
            .split_once("mod field {")
            .and_then(|(_, rest)| rest.split_once("\n}"))
            .map(|(body, _)| body)
            .ok_or("no `mod field`")?;
        let value = |name: &str| {
            field.lines().find_map(|line| {
                let rest = line.trim().strip_prefix("pub const ")?;
                let (n, v) = rest.split_once(": &str = \"")?;
                (n == name).then(|| v.trim_end_matches("\";").to_string())
            })
        };
        let body = protocol
            .split_once("pub fn from_query(")
            .and_then(|(_, rest)| rest.split_once("\n    }\n"))
            .map(|(body, _)| body)
            .ok_or("no `from_query`")?;
        let mut keys = Vec::new();
        for arm in body.split("field::").skip(1) {
            let Some((name, _)) = arm.split_once(" =>") else {
                continue;
            };
            keys.push(value(name).ok_or(format!("no `field::{name}`"))?);
        }
        if keys.is_empty() {
            return Err("no `field::X =>` arm".into());
        }
        let listed: Vec<&str> = MEASURE_KEYS
            .iter()
            .copied()
            .chain(NOT_MEASURED.iter().map(|(key, _)| *key))
            .collect();
        let mut problems: Vec<String> = keys
            .iter()
            .filter(|key| !listed.contains(&key.as_str()))
            .cloned()
            .collect();
        problems.extend(
            listed
                .iter()
                .filter(|key| !keys.iter().any(|k| k == *key))
                .map(|key| format!("{key} (listed, not a key)")),
        );
        Ok(problems)
    }

    /// Asserts:
    /// * each query key of `Settings::from_query` in
    ///   `protocol/src/lib.rs` is in `MEASURE_KEYS` or `NOT_MEASURED`, and
    ///   each listed key is a query key;
    /// * a protocol text with a key `zzz` in neither list gives `zzz`.
    #[test]
    fn each_query_key_is_measured_or_not() {
        let path = crate::workspace_root().join("protocol/src/lib.rs");
        let protocol = std::fs::read_to_string(path).unwrap();
        assert_eq!(unlisted_query_keys(&protocol), Ok(Vec::new()));
        let more = protocol
            .replacen(
                "    pub const WEIGHTS: &str = \"weights\";",
                "    pub const WEIGHTS: &str = \"weights\";\n    pub const ZZZ: &str = \"zzz\";",
                1,
            )
            .replacen(
                "                field::PROFILE =>",
                "                field::ZZZ => {}\n                field::PROFILE =>",
                1,
            );
        assert_eq!(unlisted_query_keys(&more), Ok(vec!["zzz".to_string()]));
    }

    /// Asserts that the line of an update gives, with an old entry, the
    /// old and the new baseline, tolerance, bound and browser version, and
    /// "none" with no old entry.
    #[test]
    fn the_update_line_gives_the_old_and_the_new_figures() {
        let old = entry(5.0, 10.0);
        let new = Entry {
            version: "158.0.7100.0".into(),
            ..entry(5.6, 12.0)
        };
        let line = update_line(Some(&old), &new);
        for part in [
            "chrome, apple metal-3, Apple M2 Pro",
            "5.60 s",
            "12 %",
            "bound 6.27 s",
            "158.0.7100.0",
            "5.00 s",
            "10 %",
            "bound 5.50 s",
            "157.0.7000.0",
        ] {
            assert!(line.contains(part), "{part}: {line}");
        }
        let first = update_line(None, &new);
        assert!(first.contains("was none"), "{first}");
    }

    /// Asserts that `file_refusal`:
    /// * refuses a file with one entry of 1 run, and names its key, the
    ///   path and the problem;
    /// * refuses nothing in the committed file.
    #[test]
    fn a_hand_edited_file_is_refused_before_any_run() {
        let mut bad = entry(5.0, 10.0);
        bad.runs.truncate(1);
        let path = std::path::Path::new("xtask/perf-baselines.json");
        let refusal = file_refusal(&[entry(4.0, 10.0), bad], path).unwrap_or_default();
        for part in [
            "chrome, apple metal-3, Apple M2 Pro",
            "xtask/perf-baselines.json",
            "1 runs",
        ] {
            assert!(refusal.contains(part), "{part}: {refusal:?}");
        }
        let text = std::fs::read_to_string(baselines_path()).unwrap();
        let entries = read_baselines(&text).unwrap_or_default();
        assert_eq!(file_refusal(&entries, &baselines_path()), None);
    }

    /// Asserts, for a log of two loads and a chunk line of an earlier run:
    /// * the adapter, fallback value, kernels and weight format come from
    ///   the last `model loaded` and `kernels:` lines;
    /// * a `chunk 1/3` line before the run's `chunk k/4` lines is left out.
    #[test]
    fn the_last_load_and_the_run_s_own_chunks_count() {
        let at = "[0.5s] console.info: [app] app/src/lib.rs:4329: separator: ";
        let mut log = vec![
            format!("{at}kernels: matmul i64,i64, attention split (stored; f32 weights)"),
            format!(
                "{at}model loaded in 9.0 s, saved in browser storage (old gpu, fallback adapter unknown; own WGSL kernels)"
            ),
            format!("{at}chunk 1/3: 9.9 s; so far 1 GPU steps"),
        ];
        log.extend(log_of(&[5.3, 4.7, 4.6, 4.8]));
        let r = result_of(&log);
        assert!(r.is_ok(), "{r:?}");
        let r = r.unwrap();
        assert_eq!(r.adapter, "apple metal-3");
        assert_eq!(r.fallback, "no");
        assert_eq!(r.kernels, "matmul v64,v64, attention freq");
        assert_eq!(r.weights, "f16");
        assert_eq!(r.chunks, [5.3, 4.7, 4.6, 4.8]);
    }
}
