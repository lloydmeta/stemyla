//! `xtask e2e weights`: the evidence for M9's default (D10 and D11 of
//! `plans/completed/stream-from-storage-m9.md`).  f16 weights must give the
//! stems of f32 weights bit for bit; the speed is reported as a measure.
//!
//! * Part 1, the pinned kernels: the model loads as f32 with its bytes and
//!   separates the 3-second clip of `stored-stems`, then loads from browser
//!   storage as f16 and separates it again, both with M3's settings.  The
//!   f32 stems must have M3's hashes.
//! * Part 2, this GPU's kernels at the default chunk: an f32 load chooses
//!   the kernels, and an f16 load takes that choice from the URL settings.
//! * Part 3, the speed, as a measure only: one worker separates about 60 s
//!   of the synthetic mix after a warm-up of each format, then four times
//!   with each format, in the order of [`TIMED`].  Each format takes its
//!   own kernel choice.  It reports the times, the medians, their ratio,
//!   the f32 spread and D11's bar.  It fails no speed check: D12 passed on
//!   the maintainer's rule (the stems and `?bench`), and on this Mac a
//!   separation's spread was over 5% twice.  It fails a load or a
//!   separation that does not finish, and a missing `kernels:` line.
//!
//! Each `e2e.ask` starts a new worker.  The model is posted once, in part 1;
//! the later loads are `init` messages, which read it from browser storage.

use super::{
    Opts, Page, in_page, start_server, stem_hashes, stored_stems_clip, texts, weights_mount,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// The six SHA-256s of `stored-stems` with M3's settings and f32 weights
/// (`plans/completed/stream-from-storage-m3.md`, Progress), in the worker's
/// stem order.
pub const M3_HASHES: [&str; 6] = [
    "e41500168c92024ecc32778654fa77e22a630f31200e99965274f6cdd931b028",
    "9e916ca00e639c59f9eb31e2b6b11d4bdb04c43ebe9a38f7e71564ba79f3a024",
    "619875b264b286dba4c222bbe8e73d9b63909283905522a7c906f718e1d67f69",
    "f1b79d872435cdb931306a0c33228905c2d6b3ddfd33e9f1cb55720f52d0b624",
    "7e5eb334e7dead42ef2f7902c1590b2561347aa7b7f17b817aced81c295127bb",
    "3166e0de310c930f2b40f75c2631e4524b7115c64f90d4ef540102af6da68f90",
];

/// Seconds of the synthetic mix that part 3 separates: more than 4 chunks
/// of the default 588,800 samples.
const LONG_SECONDS: usize = 60;

/// The formats of part 3's separations after the warm-up of each, so a GPU
/// that heats up favours neither (D11).
pub const TIMED: [&str; 8] = ["f32", "f16", "f16", "f32", "f16", "f32", "f32", "f16"];

/// The bar of D11 for a spread of the f32 times: 2%, or the spread up to
/// 5%.  `None` when the spread is over 5%: then the measure gives no
/// verdict.
pub fn bar(spread: f64) -> Option<f64> {
    (spread <= 0.05).then_some(spread.max(0.02))
}

/// The median of `v` (the mean of the middle two for an even count).
pub fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        f64::NAN
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// Part 3's measure, from the four times of each format.
#[derive(Debug, PartialEq)]
pub struct Speed {
    pub f32_median: f64,
    pub f16_median: f64,
    /// (largest - smallest) / median of the f32 times.
    pub spread: f64,
    /// The bar used, or `None` when the spread is over 5%.
    pub bar: Option<f64>,
}

impl Speed {
    pub fn new(f32_times: &[f64], f16_times: &[f64]) -> Self {
        let f32_median = median(f32_times);
        let lo = f32_times.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = f32_times.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let spread = (hi - lo) / f32_median;
        Self {
            f32_median,
            f16_median: median(f16_times),
            spread,
            bar: bar(spread),
        }
    }

    /// Whether f16 is not slower: its median at most the bar above f32's.
    /// False when the spread was too large to judge.
    pub fn f16_not_slower(&self) -> bool {
        self.bar
            .is_some_and(|bar| self.f16_median <= self.f32_median * (1.0 + bar))
    }

    /// The measure as one line: the medians, their ratio, the f32 spread,
    /// and D11's bar and verdict where a bar applies.
    pub fn report(&self) -> String {
        let verdict = match self.bar {
            Some(bar) => format!(
                "bar {:.2}%: f16 {} slower",
                100.0 * bar,
                if self.f16_not_slower() { "not" } else { "is" }
            ),
            None => "no bar (spread over 5%): no verdict".to_string(),
        };
        format!(
            "part 3 (a measure only): f32 median {:.2} s, f16 median {:.2} s, f16 / f32 {:.3}; f32 spread {:.2}%; {verdict}",
            self.f32_median,
            self.f16_median,
            self.f16_median / self.f32_median,
            100.0 * self.spread
        )
    }
}

/// How two stems differ: how many values, how many of them only in the
/// sign of a zero, and the largest difference.
#[derive(Debug, PartialEq)]
pub struct Differences {
    pub count: usize,
    pub zero_sign_only: usize,
    pub largest: f32,
}

/// Compares `a` and `b` value by value, by their bits.
pub fn differences(a: &[f32], b: &[f32]) -> Differences {
    let mut d = Differences {
        count: a.len().abs_diff(b.len()),
        zero_sign_only: 0,
        largest: 0.0,
    };
    for (x, y) in a.iter().zip(b) {
        if x.to_bits() != y.to_bits() {
            d.count += 1;
            if *x == 0.0 && *y == 0.0 {
                d.zero_sign_only += 1;
            }
            let diff = (x - y).abs();
            d.largest = d.largest.max(if diff.is_finite() {
                diff
            } else {
                f32::INFINITY
            });
        }
    }
    d
}

/// The kernel choice in a worker's `kernels:` line for weights in
/// `format` (the last such line): matmul policy, attention policy and
/// where it came from.  The line is `kernels: matmul <m>, attention <a>
/// (<source>; <format> weights)` (`roformer_gpu::bench::kernels_line`).
pub fn kernels(logs: &[String], format: &str) -> Option<(String, String, String)> {
    let tail = format!("; {format} weights)");
    logs.iter().rev().find_map(|l| {
        let rest = &l[l.find("kernels: matmul ")? + "kernels: matmul ".len()..];
        let rest = rest.strip_suffix(&tail)?;
        let (mm, rest) = rest.split_once(", attention ")?;
        let (attn, source) = rest.split_once(" (")?;
        Some((mm.to_string(), attn.to_string(), source.to_string()))
    })
}

/// From a worker's `memory:` line: the parameters on the GPU in MB, the
/// weights it names (`f32 weights` or `f16 weight matrices`), and the GPU
/// buffers live in MB, if it gives them.
pub fn memory(line: &str) -> Option<(f64, String, Option<f64>)> {
    let rest = &line[line.find("model on the GPU ")? + "model on the GPU ".len()..];
    let (mb, rest) = rest.split_once(" MB (")?;
    let (weights, rest) = rest.split_once(')')?;
    let live = rest
        .split_once("GPU buffers live ")
        .and_then(|(_, r)| r.split_once(" MB"))
        .and_then(|(n, _)| n.parse().ok());
    Some((mb.parse().ok()?, weights.to_string(), live))
}

/// The `memory:` lines of `logs`, parsed.
fn memory_lines(logs: &[String]) -> Vec<(f64, String, Option<f64>)> {
    logs.iter().filter_map(|l| memory(l)).collect()
}

/// Checks that each `memory:` line names `weights`, and that the
/// parameters are in `range` (MB).
pub fn memory_checks(logs: &[String], weights: &str, range: (f64, f64)) -> (bool, String) {
    let lines = memory_lines(logs);
    let ok = !lines.is_empty()
        && lines
            .iter()
            .all(|(mb, w, _)| w == weights && *mb > range.0 && *mb < range.1);
    (
        ok,
        format!(
            "every memory line names {weights}, with parameters between {} and {} MB ({:?})",
            range.0, range.1, lines
        ),
    )
}

/// The parameters of an f16 load: under 360 MB (D10).
pub const F16_MB: (f64, f64) = (0.0, 360.0);
/// The parameters of an f32 load: over 690 MB (D10).
pub const F32_MB: (f64, f64) = (690.0, 800.0);

/// The page script of parts 1 and 2: each load and its separation of the
/// clip, as `[load, separation settings]`, in one `e2e.ask` each, and
/// every stem as base64 floats.
const SEPARATE: &str = "const [model, runs, wipe] = args;
  if (wipe) await e2e.wipe();
  const clip = new Float32Array(await (await fetch('/clip/clip.f32')).arrayBuffer());
  const n = clip.length / 2;
  const out = [];
  for (const [load, settings] of runs) {
    if (load.type === 'model') load.bytes = new Uint8Array(await (await fetch(model)).arrayBuffer());
    const stems = {};
    let len = 0;
    const place = (d) => {
      len = d.len;
      for (const [name, l, r] of d.stems) {
        stems[name] ??= [new Float32Array(d.len), new Float32Array(d.len)];
        stems[name][0].set(l, d.start);
        stems[name][1].set(r, d.start);
      }
    };
    const m = await e2e.ask([load, { type: 'separate', left: clip.slice(0, n), right: clip.slice(n), ...settings }], { segment: place });
    const run = { replies: m.replies, logs: m.logs, len, stems: {} };
    for (const [name, [l, r]] of Object.entries(stems)) run.stems[name] = [e2e.base64(l), e2e.base64(r)];
    out.push(run);
  }
  return out;";

/// The stems of one run as floats, in the worker's stem order: each
/// stem's left channel, then its right.
fn stem_floats(run: &Value) -> Result<Vec<Vec<f32>>> {
    roformer::STEMS
        .iter()
        .flat_map(|name| [(name, 0), (name, 1)])
        .map(|(name, c)| {
            let text = run["stems"][name][c]
                .as_str()
                .with_context(|| format!("no stem {name}"))?;
            super::floats_from_base64(text)
        })
        .collect()
}

/// Notes the replies, hashes and memory lines of one run, and checks that
/// it loaded and finished.
fn note_run(p: &mut Page, label: &str, run: &Value) -> Result<Vec<String>> {
    p.note(&format!("{label}: replies {:?}", texts(&run["replies"])));
    p.check(
        run["replies"][0]["ok"] == json!(true) && run["replies"][1]["type"] == json!("done"),
        &format!("{label}: the model loads and the separation finishes"),
    );
    let hashes = stem_hashes(&run["stems"], &roformer::STEMS)?;
    for (name, hash) in roformer::STEMS.iter().zip(&hashes) {
        p.note(&format!("{label}: {name} sha256 {hash} (16-bit WAV)"));
    }
    for (name, floats) in roformer::STEMS.iter().zip(stem_floats(run)?.chunks(2)) {
        let bytes: Vec<u8> = floats
            .iter()
            .flatten()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        p.note(&format!(
            "{label}: {name} sha256 {} (f32 samples, left then right)",
            super::sha256_hex(&bytes)
        ));
    }
    for line in run["logs"].as_array().into_iter().flatten() {
        let line = line.as_str().unwrap_or_default();
        if line.contains("memory: ")
            || line.contains("kernels: ")
            || line.contains("f16 weights")
            || line.contains("weights: ")
        {
            p.note(&format!("{label}: log: {line}"));
        }
    }
    Ok(hashes)
}

fn logs(run: &Value) -> Vec<String> {
    run["logs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(str::to_string))
        .collect()
}

/// Checks that the stems of an f16 run equal the f32 run's bit for bit,
/// and notes the counts of each stem that differs.
fn compare(p: &mut Page, part: &str, f32_run: &Value, f16_run: &Value) -> Result<()> {
    let (a, b) = (stem_floats(f32_run)?, stem_floats(f16_run)?);
    let mut equal = true;
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        let d = differences(y, x);
        if d.count > 0 {
            equal = false;
            p.note(&format!(
                "{part}: {} {}: {} values differ, {} only in the sign of a zero, the largest by {:e}",
                roformer::STEMS[i / 2],
                ["left", "right"][i % 2],
                d.count,
                d.zero_sign_only,
                d.largest
            ));
        }
    }
    p.check(
        equal,
        &format!("{part}: the f16 stems equal the f32 stems bit for bit"),
    );
    Ok(())
}

/// Part 3: the page script.  One worker loads each format from browser
/// storage and separates the long clip, warm-ups first; returns the
/// replies and the log.
const TIMES: &str = "const [order, settings] = args;
  const long = new Float32Array(await (await fetch('/clip/long.f32')).arrayBuffer());
  const n = long.length / 2;
  const msgs = order.flatMap((w) => [
    { type: 'init', weights: w, ...settings },
    { type: 'separate', left: long.slice(0, n), right: long.slice(n), weights: w, ...settings },
  ]);
  const m = await e2e.ask(msgs);
  return { replies: m.replies, logs: m.logs, n };";

/// The long clip of part 3 as 32-bit floats, left then right: the
/// synthetic mix repeated to [`LONG_SECONDS`].
fn long_clip(dir: &std::path::Path) -> Result<usize> {
    let song = crate::synth::generate();
    let n = LONG_SECONDS * crate::synth::SAMPLE_RATE as usize;
    let len = song.mix.left.len();
    let (left, right) = (&song.mix.left, &song.mix.right);
    let bytes: Vec<u8> = (0..n)
        .map(|i| left[i % len])
        .chain((0..n).map(|i| right[i % len]))
        .flat_map(|x| x.to_le_bytes())
        .collect();
    std::fs::write(dir.join("long.f32"), bytes)?;
    Ok(n)
}

/// Part 3's times: the `secs` of each `done` reply after the two
/// warm-ups, with the format of each.
pub fn timed(replies: &Value) -> Vec<(String, f64)> {
    let done: Vec<f64> = replies
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["type"] == json!("done"))
        .filter_map(|r| r["secs"].as_f64())
        .collect();
    TIMED
        .iter()
        .zip(done.iter().skip(2))
        .map(|(w, s)| (w.to_string(), *s))
        .collect()
}

/// Runs part 3 once, and reports its speed as a measure only.
fn speed(p: &mut Page, settings: &Value) -> Result<()> {
    let order: Vec<&str> = ["f32", "f16"].into_iter().chain(TIMED).collect();
    let r = in_page(p, TIMES, json!([order, settings]), 7200)?;
    let replies = r["replies"].as_array().cloned().unwrap_or_default();
    p.check(
        replies.len() == 2 * order.len()
            && replies
                .iter()
                .all(|r| r["type"] != json!("error") && r["ok"] != json!(false)),
        "part 3: every load and separation finishes",
    );
    let logs = logs(&r);
    for line in logs.iter().filter(|l| l.contains("kernels: ")) {
        p.note(&format!("part 3: log: {line}"));
    }
    for w in ["f32", "f16"] {
        let k = kernels(&logs, w);
        p.check(
            k.as_ref()
                .is_some_and(|(_, _, source)| !source.contains("URL")),
            &format!("part 3: a kernels line for {w} weights, chosen for them ({k:?})"),
        );
    }
    let times = timed(&r["replies"]);
    let of = |w: &str| -> Vec<f64> {
        times
            .iter()
            .filter(|(f, _)| f == w)
            .map(|(_, s)| *s)
            .collect()
    };
    p.note(&format!(
        "part 3: {} frames; times in order {times:?}",
        r["n"]
    ));
    p.note(&Speed::new(&of("f32"), &of("f16")).report());
    Ok(())
}

pub fn run(o: &Opts) -> Result<()> {
    let (clip_dir, frames) = stored_stems_clip(&o.out)?;
    let long = long_clip(&clip_dir)?;
    let mut mounts = vec![("/clip/".to_string(), clip_dir)];
    let model = weights_mount(o, &mut mounts)?;
    let base = start_server(&o.dist, mounts, &["console.js"])?;
    let mut p = Page::open(o, base, &["--enable-unsafe-webgpu"], "__e2e/blank.html")?;
    p.note(&format!(
        "the clip: {frames} frames; the long clip: {long} frames"
    ));

    // Part 1: M3's settings and kernels.
    let pinned =
        json!({ "mm": "s64", "attn": "split", "chunk": 16384, "overlap": 2, "step_limit": 600 });
    let with = |w: &str, extra: Value| {
        let mut v = pinned.clone();
        v["weights"] = json!(w);
        for (k, x) in extra.as_object().into_iter().flatten() {
            v[k] = x.clone();
        }
        v
    };
    let runs = json!([
        [
            with("f32", json!({ "type": "model" })),
            with("f32", json!({}))
        ],
        [
            with("f16", json!({ "type": "init" })),
            with("f16", json!({}))
        ],
    ]);
    let r = in_page(&mut p, SEPARATE, json!([model, runs, true]), 3600)?;
    let hashes = note_run(&mut p, "part 1, f32", &r[0])?;
    note_run(&mut p, "part 1, f16", &r[1])?;
    let f16_lines = super::f16_problems("weights=f16", &logs(&r[1]), true);
    p.check(
        f16_lines.is_empty(),
        &format!("part 1, f16: the start-up line and f16 weights ({f16_lines:?})"),
    );
    p.check(
        hashes == M3_HASHES,
        "part 1: the f32 stems have M3's six hashes",
    );
    compare(&mut p, "part 1", &r[0], &r[1])?;
    let (ok, what) = memory_checks(&logs(&r[0]), "f32 weights", F32_MB);
    p.check(ok, &format!("part 1, f32: {what}"));
    let (ok, what) = memory_checks(&logs(&r[1]), "f16 weight matrices", F16_MB);
    p.check(ok, &format!("part 1, f16: {what}"));

    // Part 2: this GPU's own kernels at the default chunk.
    let default = json!({ "step_limit": 600 });
    let mut f32_load = default.clone();
    f32_load["type"] = json!("init");
    f32_load["weights"] = json!("f32");
    let mut f32_settings = default.clone();
    f32_settings["weights"] = json!("f32");
    let r32 = in_page(
        &mut p,
        SEPARATE,
        json!([model, [[f32_load, f32_settings]], false]),
        3600,
    )?;
    note_run(&mut p, "part 2, f32", &r32[0])?;
    let Some((mm, attn, source)) = kernels(&logs(&r32[0]), "f32") else {
        bail!("part 2: no kernels line for f32 weights");
    };
    p.note(&format!(
        "part 2: the f32 choice: matmul {mm}, attention {attn} ({source})"
    ));
    let f16_settings = json!({ "step_limit": 600, "weights": "f16", "mm": mm, "attn": attn });
    let mut f16_load = f16_settings.clone();
    f16_load["type"] = json!("init");
    let r16 = in_page(
        &mut p,
        SEPARATE,
        json!([model, [[f16_load, f16_settings]], false]),
        3600,
    )?;
    note_run(&mut p, "part 2, f16", &r16[0])?;
    let f16_lines = super::f16_problems("weights=f16", &logs(&r16[0]), true);
    p.check(
        f16_lines.is_empty(),
        &format!("part 2, f16: the start-up line and f16 weights ({f16_lines:?})"),
    );
    let k16 = kernels(&logs(&r16[0]), "f16");
    p.check(
        k16.as_ref().is_some_and(|(m, a, _)| *m == mm && *a == attn),
        &format!("part 2: the f16 load runs the f32 choice ({k16:?})"),
    );
    compare(&mut p, "part 2", &r32[0], &r16[0])?;
    let (ok, what) = memory_checks(&logs(&r32[0]), "f32 weights", F32_MB);
    p.check(ok, &format!("part 2, f32: {what}"));
    let (ok, what) = memory_checks(&logs(&r16[0]), "f16 weight matrices", F16_MB);
    p.check(ok, &format!("part 2, f16: {what}"));

    // Part 3: the speed of each format with its own kernels, as a measure.
    speed(&mut p, &default)?;
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_is_two_percent_or_the_spread_up_to_five() {
        assert_eq!(bar(0.0), Some(0.02));
        assert_eq!(bar(0.019), Some(0.02));
        assert_eq!(bar(0.03), Some(0.03));
        assert_eq!(bar(0.05), Some(0.05));
        assert_eq!(bar(0.051), None);
    }

    #[test]
    fn medians_of_odd_and_even_counts() {
        assert_eq!(median(&[5.0]), 5.0);
        assert_eq!(median(&[4.0, 1.0]), 2.5);
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert!(median(&[]).is_nan());
    }

    /// f16 passes at the bar and fails above it; a spread over 5% gives no
    /// verdict.
    #[test]
    fn f16_is_not_slower_within_the_bar() {
        let f32_times = [10.0, 10.1, 10.0, 10.1];
        let s = Speed::new(&f32_times, &[10.2, 10.25, 10.2, 10.3]);
        assert_eq!(s.f32_median, 10.05);
        assert_eq!(s.f16_median, 10.225);
        assert!((s.spread - 0.1 / 10.05).abs() < 1e-12);
        assert_eq!(s.bar, Some(0.02));
        assert!(s.f16_not_slower());
        let slow = Speed::new(&f32_times, &[10.3, 10.3, 10.3, 10.3]);
        assert!(!slow.f16_not_slower(), "{slow:?}");
        let wide = Speed::new(&[10.0, 10.4, 10.0, 10.4], &[10.4; 4]);
        assert!((wide.bar.unwrap() - 0.4 / 10.2).abs() < 1e-12, "{wide:?}");
        assert!(wide.f16_not_slower());
        let noisy = Speed::new(&[10.0, 11.0, 10.0, 11.0], &[9.0; 4]);
        assert_eq!(noisy.bar, None);
        assert!(!noisy.f16_not_slower());
    }

    /// Part 3's line gives the measure, with or without a bar.
    #[test]
    fn part_3_reports_the_speed_as_a_measure() {
        let s = Speed::new(&[10.0, 10.1, 10.0, 10.1], &[9.0, 9.1, 9.0, 9.1]);
        assert_eq!(
            s.report(),
            "part 3 (a measure only): f32 median 10.05 s, f16 median 9.05 s, f16 / f32 0.900; f32 spread 1.00%; bar 2.00%: f16 not slower"
        );
        let slow = Speed::new(&[10.0; 4], &[11.0; 4]);
        assert!(
            slow.report().ends_with("bar 2.00%: f16 is slower"),
            "{}",
            slow.report()
        );
        let noisy = Speed::new(&[10.0, 11.0, 10.0, 11.0], &[9.0; 4]);
        assert!(
            noisy
                .report()
                .ends_with("f32 spread 9.52%; no bar (spread over 5%): no verdict"),
            "{}",
            noisy.report()
        );
    }

    /// The stems of a run, in the worker's stem order, each channel as the
    /// page sent it: base64 of little-endian floats.
    #[test]
    fn stem_floats_reads_each_stem_and_channel_in_order() {
        let floats = |stem: usize, channel: usize| -> Vec<f32> {
            (0..5)
                .map(|i| (100 * stem + 10 * channel + i) as f32 - 0.5)
                .collect()
        };
        let mut stems = serde_json::Map::new();
        for (s, name) in roformer::STEMS.iter().enumerate() {
            let channel = |c| {
                let bytes: Vec<u8> = floats(s, c).iter().flat_map(|x| x.to_le_bytes()).collect();
                json!(crate::e2e::tests::base64(&bytes))
            };
            stems.insert(name.to_string(), json!([channel(0), channel(1)]));
        }
        let got = stem_floats(&json!({ "stems": stems })).unwrap();
        assert_eq!(got.len(), 2 * roformer::STEMS.len());
        for (i, channel) in got.iter().enumerate() {
            assert_eq!(
                *channel,
                floats(i / 2, i % 2),
                "stem {} channel {}",
                i / 2,
                i % 2
            );
        }
        let missing = json!({ "stems": { "bass": ["", ""] } });
        assert!(stem_floats(&missing).is_err());
    }

    /// The long clip holds [`LONG_SECONDS`] of the synthetic mix: the left
    /// channel, then the right, as little-endian floats.
    #[test]
    fn the_long_clip_is_the_mix_left_then_right() {
        let dir = std::env::temp_dir().join(format!("stemyla-long-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let n = long_clip(&dir).unwrap();
        assert_eq!(n, LONG_SECONDS * crate::synth::SAMPLE_RATE as usize);
        let bytes = std::fs::read(dir.join("long.f32")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(bytes.len(), 2 * n * 4);
        let song = crate::synth::generate();
        let at = |i: usize| f32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap());
        let len = song.mix.left.len();
        for i in [0, 1, len - 1, len, len + 7, n - 1]
            .into_iter()
            .filter(|&i| i < n)
        {
            assert_eq!(at(i), song.mix.left[i % len], "left {i}");
            assert_eq!(at(n + i), song.mix.right[i % len], "right {i}");
        }
    }

    #[test]
    fn differences_count_values_and_zero_signs() {
        let d = differences(&[0.0, -0.0, 1.0, 2.0, 3.0], &[-0.0, -0.0, 1.0, 2.5, 3.0]);
        assert_eq!(
            d,
            Differences {
                count: 2,
                zero_sign_only: 1,
                largest: 0.5
            }
        );
        assert_eq!(differences(&[1.0], &[1.0, 2.0]).count, 1);
        assert_eq!(differences(&[f32::NAN], &[1.0]).largest, f32::INFINITY);
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|l| l.to_string()).collect()
    }

    /// The last kernels line of the format, as `kernels_line` writes it.
    #[test]
    fn kernels_lines_are_read_by_format() {
        let logs = lines(&[
            "kernels: matmul s64, attention split (calibrated; f32 weights)",
            "kernels: matmul v64,i64, attention fused (stored; f32 weights)",
            "kernels: matmul v64,i128x64, attention time (from the URL; f16 weights)",
        ]);
        assert_eq!(
            kernels(&logs, "f32"),
            Some(("v64,i64".into(), "fused".into(), "stored".into()))
        );
        assert_eq!(
            kernels(&logs, "f16"),
            Some(("v64,i128x64".into(), "time".into(), "from the URL".into()))
        );
        assert_eq!(kernels(&logs[..1], "f16"), None);
    }

    #[test]
    fn memory_lines_give_the_parameters_and_the_live_bytes() {
        let f16 = "memory: worker wasm 57 MB, model on the GPU 350 MB (f16 weight matrices), chunk buffers 280 MB, GPU buffers live 631 MB";
        let f32 = "memory: worker wasm 57 MB, model on the GPU 699 MB (f32 weights), GPU buffers live 699 MB";
        assert_eq!(
            memory(f16),
            Some((350.0, "f16 weight matrices".into(), Some(631.0)))
        );
        assert_eq!(
            memory(f32),
            Some((699.0, "f32 weights".into(), Some(699.0)))
        );
        assert_eq!(
            memory("memory: worker wasm 5 MB, model on the GPU 699 MB (f32 weights)"),
            Some((699.0, "f32 weights".into(), None))
        );
        assert_eq!(memory("kernels: matmul s64"), None);
        assert!(memory_checks(&lines(&[f16]), "f16 weight matrices", F16_MB).0);
        assert!(!memory_checks(&lines(&[f16, f32]), "f16 weight matrices", F16_MB).0);
        assert!(memory_checks(&lines(&[f32]), "f32 weights", F32_MB).0);
        assert!(!memory_checks(&lines(&[f32]), "f16 weight matrices", F16_MB).0);
        assert!(!memory_checks(&[], "f32 weights", F32_MB).0);
        let big_f16 = f16.replace("350 MB (f16", "361 MB (f16");
        assert!(!memory_checks(&lines(&[&big_f16]), "f16 weight matrices", F16_MB).0);
        let small_f32 = f32.replace("699 MB (f32", "689 MB (f32");
        assert!(!memory_checks(&lines(&[&small_f32]), "f32 weights", F32_MB).0);
    }

    /// The times skip the two warm-ups and take the formats of [`TIMED`].
    #[test]
    fn the_times_skip_the_warm_ups() {
        let mut replies = Vec::new();
        for i in 0..10 {
            replies.push(json!({ "type": "model", "ok": true }));
            replies.push(json!({ "type": "done", "secs": f64::from(i) }));
        }
        let t = timed(&json!(replies));
        assert_eq!(t.len(), 8);
        assert_eq!(t[0], ("f32".to_string(), 2.0));
        assert_eq!(t[1], ("f16".to_string(), 3.0));
        assert_eq!(t[7], ("f16".to_string(), 9.0));
        assert_eq!(TIMED.iter().filter(|w| **w == "f32").count(), 4);
    }
}
