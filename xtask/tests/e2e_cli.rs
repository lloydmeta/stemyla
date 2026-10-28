//! `xtask e2e` as a person runs it, with no browser: a scenario that cannot
//! start its driver fails before any browser starts.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A new, empty folder for one test, with a `dist` that has an
/// `index.html`, so `e2e` gets as far as the scenario.
fn test_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xtask-e2e-cli-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dist = dir.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), "<!doctype html>").unwrap();
    dir
}

/// Runs `xtask e2e <scenario>` in Chrome with a `CHROMEDRIVER` that is not
/// a file, plus `extra`, and removes `dir` after.
fn run_without_a_driver(dir: &Path, scenario: &str, extra: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["e2e", scenario, "--browser", "chrome", "--dist"])
        .arg(dir.join("dist"))
        .arg("--out")
        .arg(dir.join("out"))
        .args(extra)
        .env("CHROMEDRIVER", dir.join("no-such-chromedriver"))
        .env_remove("CHROMEWEBDRIVER")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    out
}

/// Asserts that the run failed on the driver.
fn assert_fails_on_the_driver(scenario: &str, out: &Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{scenario} fails: {stderr}");
    assert!(
        stderr.contains("CHROMEDRIVER is") && stderr.contains("which is not a file"),
        "{scenario} fails on the driver: {stderr}"
    );
}

/// Runs `scenario` with no driver.  A scenario that needs the model gets
/// a file that is not a model, since the driver starts before the model
/// loads.
fn fails_before_a_browser_starts(test: &str, scenario: &str, needs_the_model: bool) {
    let dir = test_dir(test);
    let weights = dir.join("model.safetensors");
    std::fs::write(&weights, b"not a model").unwrap();
    let weights = weights.to_str().unwrap().to_string();
    let extra: &[&str] = if needs_the_model {
        &["--weights", &weights]
    } else {
        &[]
    };
    let out = run_without_a_driver(&dir, scenario, extra);
    assert_fails_on_the_driver(scenario, &out);
}

/// Finding 3-9 of review round 3 of M7: `e2e old-session` with a
/// `CHROMEDRIVER` that is not a file ends with an error, not a pass.  It
/// needs no browser and no internet: the server binds a port on this
/// machine, then the driver fails to start.
#[test]
fn old_session_with_no_driver_fails_before_a_browser_starts() {
    fails_before_a_browser_starts("old-session", "old-session", false);
}

/// Finding 2-14 of review round 2 of M6: a `smoke` that does nothing
/// fails this test.
#[test]
fn smoke_with_no_driver_fails_before_a_browser_starts() {
    fails_before_a_browser_starts("smoke", "smoke", false);
}

/// Findings 1-80 of M3 and 1-41 of M4: a `separate` that does nothing
/// fails this test.
#[test]
fn separate_with_no_driver_fails_before_a_browser_starts() {
    fails_before_a_browser_starts("separate", "separate", true);
}

/// Findings 2-49 of M3, 1-30 of M4 and 1-68 of M5: a `memory_budget` that
/// does nothing fails this test.
#[test]
fn memory_budget_with_no_driver_fails_before_a_browser_starts() {
    fails_before_a_browser_starts("memory-budget", "memory-budget", true);
}

/// Findings 1-139 and 2-41 of M3 and 1-53 of M4: a `stored_stems` that
/// does nothing fails this test.
#[test]
fn stored_stems_with_no_driver_fails_before_a_browser_starts() {
    fails_before_a_browser_starts("stored-stems", "stored-stems", true);
}

/// Review round 1 of the catch-up plan: an `audio_resume` that does
/// nothing fails this test.
#[test]
fn audio_resume_with_no_driver_fails_before_a_browser_starts() {
    fails_before_a_browser_starts("audio-resume", "audio-resume", false);
}

/// Review round 1 of the catch-up plan: a `bench` that does nothing fails
/// this test.
#[test]
fn bench_with_no_driver_fails_before_a_browser_starts() {
    fails_before_a_browser_starts("bench", "bench", false);
}

/// Review round 1 of the catch-up plan: a `run` of `e2e playback` that
/// does nothing fails this test.
#[test]
fn playback_with_no_driver_fails_before_a_browser_starts() {
    fails_before_a_browser_starts("playback", "playback", false);
}
