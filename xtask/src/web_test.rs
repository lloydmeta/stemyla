//! `xtask web-test [--browser safari|chrome] [filter]`: the browser tests
//! of `app` (song sessions in browser storage) and `stem-worker` (model
//! storage, message handling), compiled for wasm32 and run in a real
//! browser by `wasm-bindgen-test-runner`.
//!
//! It sets Cargo's runner and the driver through the environment, so no
//! `.cargo/config.toml` is needed, and it checks that the installed runner
//! matches the `wasm-bindgen` in `Cargo.lock` (the two must be equal).

use crate::drivers::Browser;
use anyhow::{Context, Result, bail};
use std::process::Command;

/// Environment variables `wasm-bindgen-test-runner` reads to pick a driver
/// (`<DRIVER>` and `<DRIVER>_REMOTE`); all are cleared, then one is set.
const DRIVER_VARS: [&str; 4] = [
    "GECKODRIVER",
    "SAFARIDRIVER",
    "CHROMEDRIVER",
    "MSEDGEDRIVER",
];

pub fn run(args: &[String]) -> Result<()> {
    let mut browser = Browser::default_here();
    let mut filter = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--browser" => {
                browser = Browser::parse(args.get(i + 1).context("--browser needs a name")?)?;
                i += 1;
            }
            other if other.starts_with("--") => bail!("unknown option {other}"),
            f => filter = Some(f.to_string()),
        }
        i += 1;
    }
    let lock = std::fs::read_to_string("Cargo.lock").context("reading Cargo.lock")?;
    let wanted = locked_version(&lock, "wasm-bindgen").context("no wasm-bindgen in Cargo.lock")?;
    check_runner(&wanted)?;
    let driver = browser.driver()?;
    // One run per crate: `--test web` names targets only `app` has.
    let runs: [&[&str]; 2] = [
        &["-p", "app", "--lib", "--test", "web", "--test", "web_close"],
        &["-p", "stem-worker", "--lib"],
    ];
    for targets in runs {
        let mut cmd = Command::new(env!("CARGO"));
        cmd.args(["test", "--locked", "--target", "wasm32-unknown-unknown"])
            .args(targets)
            .env(
                "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER",
                "wasm-bindgen-test-runner",
            );
        for var in DRIVER_VARS {
            cmd.env_remove(var).env_remove(format!("{var}_REMOTE"));
        }
        let var = match browser {
            Browser::Chrome => "CHROMEDRIVER",
            Browser::Safari => "SAFARIDRIVER",
        };
        cmd.env(var, &driver);
        if let Some(f) = &filter {
            cmd.args(["--", f]);
        }
        let status = cmd.status().context("running cargo test")?;
        if !status.success() {
            bail!(
                "browser tests failed in {browser:?}; if no test ran, check the driver: {}",
                browser.setup_hint()
            );
        }
    }
    Ok(())
}

/// The version of `name` in a `Cargo.lock` (the first, if several).
pub fn locked_version(lock: &str, name: &str) -> Option<String> {
    let wanted = format!("name = \"{name}\"");
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == wanted {
            let v = lines.next()?.trim().strip_prefix("version = \"")?;
            return v.strip_suffix('"').map(str::to_string);
        }
    }
    None
}

fn check_runner(wanted: &str) -> Result<()> {
    let install = format!("cargo install wasm-bindgen-cli --version {wanted} --locked");
    let out = Command::new("wasm-bindgen-test-runner")
        .arg("--version")
        .output()
        .with_context(|| format!("no wasm-bindgen-test-runner; install it with: {install}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let have = text.split_whitespace().nth(1).unwrap_or("unknown");
    if have != wanted {
        bail!(
            "wasm-bindgen-test-runner is {have}, but Cargo.lock has wasm-bindgen {wanted}; install the matching one with: {install}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"
[[package]]
name = "wasm-bindgen-futures"
version = "0.4.79"

[[package]]
name = "wasm-bindgen"
version = "0.2.129"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    #[test]
    fn finds_the_exact_package_in_the_lockfile() {
        assert_eq!(
            locked_version(LOCK, "wasm-bindgen").as_deref(),
            Some("0.2.129")
        );
        assert_eq!(
            locked_version(LOCK, "wasm-bindgen-futures").as_deref(),
            Some("0.4.79")
        );
        assert_eq!(locked_version(LOCK, "wasm"), None);
    }

    #[test]
    fn the_workspace_lockfile_has_wasm_bindgen() {
        let lock =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.lock")).unwrap();
        assert!(locked_version(&lock, "wasm-bindgen").is_some());
    }
}
