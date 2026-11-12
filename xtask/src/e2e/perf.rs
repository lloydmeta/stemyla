//! The pure parts of `xtask e2e perf` (M2 of `plans/active/gpu-speed.md`):
//! the result of a run, read from the page's log; the GPU time of a
//! profile run in four groups; the reasons that a run is never a pass;
//! the gate; and the baselines file, `xtask/perf-baselines.json`.
//!
//! The chunk median is the median of the times of chunks 2 to N of one
//! run.  Chunk 1 is the warm-up.  A baseline is the median of the chunk
//! medians of the runs of an update.  A run fails the gate when its chunk
//! median is more than the baseline times (1 + tolerance / 100).

use super::{Opts, chunk_done, number_after};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The lowest tolerance of a baseline, in percent.
pub(super) const MIN_TOLERANCE_PCT: f64 = 10.0;

/// The fewest runs that an update of a baseline takes.
pub(super) const MIN_RUNS: usize = 5;

/// The query keys that change what a run measures.
const MEASURE_KEYS: [&str; 5] = ["chunk", "overlap", "mm", "attn", "weights"];

/// A small slack for float compares of seconds and percents.
const EPSILON: f64 = 1e-9;

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
    pub chunk_median: f64,
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
    /// The median of the runs' chunk medians, in seconds.
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
/// no `total GPU time` line.
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
    if o.headed {
        return Some("--headed: the baselines are headless".into());
    }
    if result.weights != "f16" {
        return Some(format!(
            "the weights are {}, and the baselines are f16",
            result.weights
        ));
    }
    let query = o.query.as_deref().unwrap_or_default();
    let set: Vec<String> = query
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
    let margin = bound - result.chunk_median;
    let mut text = format!(
        "chunk median {:.2} s, baseline {:.2} s, tolerance {tol:.0} %, bound {bound:.2} s, margin {margin:.2} s ({:.1} % of the baseline)",
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
    if result.chunk_median < baseline.baseline * (1.0 - tol / 100.0) - EPSILON {
        text.push_str("\nfaster than the baseline by more than the tolerance: update the baseline");
    }
    if result.chunk_median > bound + EPSILON {
        Verdict::Fail(text)
    } else {
        Verdict::Pass(text)
    }
}

/// The tolerance of a baseline from the chunk medians of its runs: the
/// larger of 10 and 2 x (slowest - baseline) / baseline x 100, rounded up
/// to a whole percent.  It refuses fewer than [`MIN_RUNS`] medians.
pub(super) fn tolerance_pct(medians: &[f64]) -> Result<f64, String> {
    if medians.len() < MIN_RUNS {
        return Err(format!(
            "{} runs: a tolerance needs {MIN_RUNS} or more",
            medians.len()
        ));
    }
    let base = median(medians);
    let slowest = medians.iter().copied().fold(f64::MIN, f64::max);
    let spread = 2.0 * (slowest - base) / base * 100.0;
    // Round off float noise first, so 12.000000000000002 gives 12.
    let spread = ((spread * 1e6).round() / 1e6).ceil();
    Ok(spread.max(MIN_TOLERANCE_PCT))
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
    let medians: Vec<f64> = results.iter().map(|r| r.chunk_median).collect();
    let tolerance_pct = tolerance_pct(&medians)?;
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
                chunk_median: r.chunk_median,
            })
            .collect(),
        baseline: median(&medians),
        tolerance_pct,
        raise: raise.to_string(),
    })
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

/// The reason to refuse an update from `old` to `new`, or `None`.  An
/// update may not raise a baseline above the old baseline times (1 + old
/// tolerance / 100), unless the browser version differs or `raise` gives
/// a reason.
pub(super) fn update_refusal(old: Option<&Entry>, new: &Entry, raise: &str) -> Option<String> {
    let old = old?;
    let bound = old.baseline * (1.0 + old.tolerance_pct / 100.0);
    if new.baseline <= bound + EPSILON || new.version != old.version || !raise.trim().is_empty() {
        return None;
    }
    Some(format!(
        "the new baseline {:.2} s is over the old bound {bound:.2} s ({:.2} s + {:.0} %) with the same browser version {}: give --raise \"<reason>\" to raise it",
        new.baseline, old.baseline, old.tolerance_pct, old.version
    ))
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
    /// * the time of each chunk, the chunk median, the whole time and the
    ///   matmul rate;
    /// * no profile groups.
    #[test]
    fn a_full_log_gives_each_value() {
        let r = result_of(&log_of(&[5.3, 4.7, 4.6, 4.8])).unwrap();
        assert_eq!(r.browser, "chrome");
        assert_eq!(r.version, "157.0.7000.0");
        assert_eq!(r.adapter, "apple metal-3");
        assert_eq!(r.fallback, "no");
        assert_eq!(r.kernels, "matmul v64,v64, attention freq");
        assert_eq!(r.weights, "f16");
        assert_eq!(r.chunks, [5.3, 4.7, 4.6, 4.8]);
        assert_eq!(r.chunk_median, 4.7);
        assert_eq!(r.whole_secs, 52.0);
        assert_eq!(r.tflops, 1.33);
        assert_eq!(r.groups, None);
    }

    /// Asserts:
    /// * the median of chunks 2 to 4 (9.0, 4.0, 5.0, 6.0) is 5.0, not 5.5;
    /// * the median of chunks 2 to 5 (9.0, 4.0, 6.0, 5.0, 7.0) is 5.5;
    /// * `chunks` keeps chunk 1.
    #[test]
    fn the_chunk_median_leaves_chunk_1_out() {
        let r = result_of(&log_of(&[9.0, 4.0, 5.0, 6.0])).unwrap();
        assert_eq!(r.chunk_median, 5.0);
        assert_eq!(r.chunks[0], 9.0);
        let r = result_of(&log_of(&[9.0, 4.0, 6.0, 5.0, 7.0])).unwrap();
        assert_eq!(r.chunk_median, 5.5);
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
    /// * the linear group holds a linear product and the gates' matmul
    ///   (1151x8x256);
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
            profile_line("rmsnorm", 1024.0),
            profile_line("rope", 2048.0),
        ];
        log.push("[72.6s] separator: profile: total GPU time 4095.000 s".into());
        log.push("[72.6s] separator: profile: GPU busy 4095.00 s of 4100.00 s of step wall time (0% waiting, submitting or between dispatches); one pass per dispatch while profiling".into());
        log.push(
            "[2.0s] separator: profile: this adapter has no timestamp-query; no GPU times".into(),
        );
        let g = profile_groups(&log, FRAMES, BANDS).unwrap();
        assert_eq!(g.time_attention, 15.0);
        assert_eq!(g.freq_attention, 240.0);
        assert_eq!(g.linear, 768.0);
        assert_eq!(g.other, 3072.0);
        assert_eq!(g.total, 4095.0);
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
    /// * `--headed`;
    /// * f32 weights;
    /// * a query with `chunk=`, `overlap=`, `mm=`, `attn=` or `weights=`.
    ///
    /// Asserts that a run on `apple metal-3`, fallback `no`, f16 weights,
    /// with no query or with `x=1`, gives no reason.
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
        ];
        for (reason, text) in cases {
            let reason = reason.unwrap_or_default();
            assert!(reason.contains(text), "{text}: {reason:?}");
        }
        for key in ["chunk", "overlap", "mm", "attn", "weights"] {
            let o = opts(&["--query", &format!("x=1&{key}=2")]);
            let reason = never_a_pass(&sample(), &o).unwrap_or_default();
            assert!(reason.contains(&format!("{key}=")), "{key}: {reason:?}");
        }
        assert_eq!(never_a_pass(&sample(), &none), None);
        assert_eq!(never_a_pass(&sample(), &opts(&["--query", "x=1"])), None);
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
                    chunk_median: baseline,
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

    fn with_median(chunk_median: f64) -> PerfResult {
        PerfResult {
            chunk_median,
            ..sample()
        }
    }

    /// Asserts, with a baseline of 5.0 s and a tolerance of 10 %:
    /// * a median of 5.5 s (the bound) passes, and the text gives the
    ///   median, the baseline, the tolerance and the margin;
    /// * a median of 5.51 s fails;
    /// * a run with profile groups is not gated, with the reason;
    /// * a run of a new browser version passes, with a line on the version;
    /// * a run of other kernels passes, with a line on the kernels;
    /// * a median of 4.49 s passes, with the line to update the baseline;
    /// * a median of 4.5 s passes with no such line.
    #[test]
    fn the_gate_fails_only_over_the_bound() {
        let base = entry(5.0, 10.0);
        let (kind, text) = told(gate(&with_median(5.5), &base));
        assert_eq!(kind, "pass", "5.5 s: {text}");
        for part in ["5.50 s", "5.00 s", "10 %", "margin 0.00 s"] {
            assert!(text.contains(part), "{part}: {text}");
        }
        let (kind, text) = told(gate(&with_median(5.51), &base));
        assert_eq!(kind, "fail", "5.51 s: {text}");
        let profiled = PerfResult {
            groups: Some(Groups::default()),
            ..with_median(9.0)
        };
        let (kind, text) = told(gate(&profiled, &base));
        assert_eq!(kind, "not gated", "a profile run: {text}");
        assert!(text.contains("profile"), "{text}");
        let newer = PerfResult {
            version: "158.0.7100.0".into(),
            ..with_median(5.0)
        };
        let (kind, text) = told(gate(&newer, &base));
        assert_eq!(kind, "pass", "a new version: {text}");
        assert!(text.contains("158.0.7100.0"), "{text}");
        let other = PerfResult {
            kernels: "matmul v64,v64, attention fused".into(),
            ..with_median(5.0)
        };
        let (kind, text) = told(gate(&other, &base));
        assert_eq!(kind, "pass", "other kernels: {text}");
        assert!(text.contains("attention fused"), "{text}");
        let (kind, text) = told(gate(&with_median(4.49), &base));
        assert_eq!(kind, "pass", "4.49 s: {text}");
        assert!(text.contains("update the baseline"), "{text}");
        let (kind, text) = told(gate(&with_median(4.5), &base));
        assert_eq!(kind, "pass", "4.5 s: {text}");
        assert!(!text.contains("update the baseline"), "{text}");
    }

    /// Asserts:
    /// * 5 equal medians give 10;
    /// * a slowest median 6 % over the median of 5 gives 12;
    /// * a slowest median 4 % over gives 10 (8 is under 10);
    /// * a slowest median 6.1 % over gives 13 (12.2, rounded up);
    /// * 4 medians are refused.
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
    /// * each run's order, kernels and chunk median, in the order of the
    ///   runs;
    /// * the baseline is the median of the chunk medians, and the
    ///   tolerance is `tolerance_pct` of them.
    ///
    /// Asserts that it refuses a run with profile groups, and runs of two
    /// adapters.
    #[test]
    fn an_entry_holds_the_runs_and_their_median() {
        let runs: Vec<PerfResult> = [5.0, 5.3, 4.9, 5.1, 5.0]
            .into_iter()
            .map(with_median)
            .collect();
        let e = new_entry(&runs, "Apple M2 Pro", "2026-11-12", "242db28", "").unwrap();
        assert_eq!(e.key, key());
        assert_eq!(e.version, "157.0.7000.0");
        assert_eq!(
            (e.date.as_str(), e.commit.as_str()),
            ("2026-11-12", "242db28")
        );
        assert_eq!(e.raise, "");
        let medians: Vec<f64> = e.runs.iter().map(|r| r.chunk_median).collect();
        assert_eq!(medians, [5.0, 5.3, 4.9, 5.1, 5.0]);
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
    /// * each entry has a tolerance of at least 10 %;
    /// * no value holds `/Users/`, `/home/` or `~/`.
    #[test]
    fn the_committed_baselines_hold_tolerances_and_no_machine_path() {
        let text = std::fs::read_to_string(baselines_path()).unwrap();
        let entries = read_baselines(&text);
        assert!(entries.is_ok(), "{entries:?}");
        for e in entries.iter().flatten() {
            assert!(e.tolerance_pct >= MIN_TOLERANCE_PCT, "{:?}", e.key);
        }
        for path in ["/Users/", "/home/", "~/"] {
            assert!(!text.contains(path), "{path}");
        }
    }

    /// Asserts, with an old baseline of 5.0 s and a tolerance of 10 %:
    /// * a lower baseline (4.0 s) passes;
    /// * a baseline of 5.5 s (the bound) passes;
    /// * a baseline of 5.6 s is refused, and the reason names `--raise`;
    /// * the same 5.6 s passes with a new browser version, or with a
    ///   reason;
    /// * any baseline passes with no old entry.
    #[test]
    fn a_raise_over_the_tolerance_needs_a_reason() {
        let old = entry(5.0, 10.0);
        assert_eq!(update_refusal(Some(&old), &entry(4.0, 10.0), ""), None);
        assert_eq!(update_refusal(Some(&old), &entry(5.5, 10.0), ""), None);
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
        assert_eq!(update_refusal(None, &entry(9.0, 10.0), ""), None);
    }
}
