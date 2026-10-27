//! `xtask e2e` as a person runs it, with no browser: a scenario that cannot
//! start its driver fails before any browser starts.

use std::process::Command;

/// Finding 3-9 of review round 3 of M7: `e2e old-session` with a
/// `CHROMEDRIVER` that is not a file ends with an error, not a pass.  It
/// needs no browser and no internet: the server binds a port on this
/// machine, then the driver fails to start.
#[test]
fn old_session_with_no_driver_fails_before_a_browser_starts() {
    let dir = std::env::temp_dir().join(format!("xtask-e2e-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dist = dir.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), "<!doctype html>").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["e2e", "old-session", "--browser", "chrome", "--dist"])
        .arg(&dist)
        .arg("--out")
        .arg(dir.join("out"))
        .env("CHROMEDRIVER", dir.join("no-such-chromedriver"))
        .env_remove("CHROMEWEBDRIVER")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "it fails: {stderr}");
    assert!(
        stderr.contains("CHROMEDRIVER is") && stderr.contains("which is not a file"),
        "it fails on the driver: {stderr}"
    );
}
