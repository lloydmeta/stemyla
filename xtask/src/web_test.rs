//! `xtask web-test [--browser safari|chrome] [--chrome-arg <arg>]... [filter]`:
//! the browser tests
//! of `app` (song sessions in browser storage), `stem-worker` (model
//! storage, message handling), `decode-worker`, `stem-protocol` and
//! `engine` (the resampler with a 32-bit `usize`), compiled for wasm32 and
//! run in a real browser by `wasm-bindgen-test-runner`.  It keeps the
//! output of each crate run while it prints it, and fails a crate run
//! whose test binary has no wasm test, unless a filter is given.
//!
//! In Safari, the crate run of `stem-worker` gets `WASM_BINDGEN_NO_DEBUG=1`,
//! and `web-test` prints why (TD-93, [`run_vars`]).
//!
//! In Safari, it ends each stale `safaridriver` before each crate run.  If
//! the session of a test binary did not start before a test ran, it runs
//! that crate once more (`session_did_not_start`).  It runs no test again.
//!
//! It runs each crate, also after a crate fails.  Then it prints one line
//! for each crate, and fails with the name of each crate that failed.
//!
//! It sets Cargo's runner and the driver through the environment, so no
//! `.cargo/config.toml` is needed, and it checks that the installed runner
//! matches the `wasm-bindgen` in `Cargo.lock` (the two must be equal).

use crate::drivers::Browser;
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

/// Environment variables `wasm-bindgen-test-runner` reads to pick a driver
/// (`<DRIVER>` and `<DRIVER>_REMOTE`); all are cleared, then one is set.
const DRIVER_VARS: [&str; 4] = [
    "GECKODRIVER",
    "SAFARIDRIVER",
    "CHROMEDRIVER",
    "MSEDGEDRIVER",
];

/// Seconds that the runner gives each test binary, unless the caller sets
/// `WASM_BINDGEN_TEST_TIMEOUT`.
const WEB_TEST_TIMEOUT_S: &str = "600";

pub fn run(args: &[String]) -> Result<()> {
    let mut browser = Browser::default_here();
    let mut filter = None;
    let mut chrome_args = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--browser" => {
                browser = Browser::parse(args.get(i + 1).context("--browser needs a name")?)?;
                i += 1;
            }
            "--chrome-arg" => {
                let arg = args.get(i + 1).context("--chrome-arg needs an argument")?;
                chrome_args.push(arg.clone());
                i += 1;
            }
            other if other.starts_with("--") => bail!("unknown option {other}"),
            f => filter = Some(f.to_string()),
        }
        i += 1;
    }
    let root = crate::workspace_root();
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).context("reading Cargo.lock")?;
    let wanted = locked_version(&lock, "wasm-bindgen").context("no wasm-bindgen in Cargo.lock")?;
    check_runner(&wanted)?;
    if browser != Browser::Chrome && !chrome_args.is_empty() {
        bail!("--chrome-arg needs --browser chrome");
    }
    crate::drivers::refuse_a_locked_screen(browser)?;
    let _active = crate::drivers::keep_user_active(browser);
    let driver = browser.driver()?;
    // One run per crate: `--test web` names targets only `app` has.
    let runs: [(&str, &[&str]); 5] = [
        (
            "app",
            &[
                "--lib",
                "--test",
                "web",
                "--test",
                "web_close",
                "--test",
                "web_output",
            ],
        ),
        ("stem-worker", &["--lib"]),
        ("decode-worker", &["--lib"]),
        ("stem-protocol", &["--test", "web"]),
        ("engine", &["--lib"]),
    ];
    let mut results = Vec::with_capacity(runs.len());
    for (package, targets) in runs {
        let mut cmd = Command::new(env!("CARGO"));
        cmd.current_dir(&root);
        cmd.args(["test", "--locked", "--target", "wasm32-unknown-unknown"])
            .args(["-p", package])
            .args(targets)
            .env(
                "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER",
                "wasm-bindgen-test-runner",
            );
        for var in DRIVER_VARS {
            cmd.env_remove(var).env_remove(format!("{var}_REMOTE"));
        }
        // It would skip every browser test and report success.
        cmd.env_remove("WASM_BINDGEN_TEST_ONLY_NODE");
        // The runner gives each test binary 20 s by default.  The app's
        // lib tests run the `?streamtest` runs, the stored lanes of M6,
        // which play in real time, and the saves of M8.  They took 197 s in
        // CI run 40788465048, and 219 s to 231 s on the maintainer's Mac.
        // One run there passed 300 s (review round speed 1 of M8).  600 s
        // is about 3 times the CI time.
        if std::env::var_os("WASM_BINDGEN_TEST_TIMEOUT").is_none() {
            cmd.env("WASM_BINDGEN_TEST_TIMEOUT", WEB_TEST_TIMEOUT_S);
        }
        let var = match browser {
            Browser::Chrome => "CHROMEDRIVER",
            Browser::Safari => "SAFARIDRIVER",
        };
        cmd.env(var, &driver);
        if browser == Browser::Chrome {
            cmd.env(
                "WASM_BINDGEN_TEST_WEBDRIVER_JSON",
                chrome_capabilities(&root, &chrome_args)?,
            );
        }
        for var in run_vars(browser, package) {
            println!(
                "web-test: {package}: {}={}, since {}",
                var.name, var.value, var.why
            );
            cmd.env(var.name, var.value);
        }
        if let Some(f) = &filter {
            cmd.args(["--", f]);
        }
        if browser == Browser::Safari {
            crate::drivers::end_stale_safari_sessions();
        }
        let (mut success, mut output) =
            run_and_keep(&mut cmd).with_context(|| format!("running cargo test for {package}"))?;
        // Q8 of the tech debt rungs: only a session start is tried again.
        if browser == Browser::Safari && !success && session_did_not_start(&output) {
            println!(
                "web-test: {package}: the Safari session did not start before a test ran; ending the stale sessions, and running {package} once more"
            );
            crate::drivers::end_stale_safari_sessions();
            (success, output) = run_and_keep(&mut cmd)
                .with_context(|| format!("running cargo test for {package} again"))?;
        }
        if filter.is_none() && found_no_test(&output) {
            println!(
                "web-test: {package}: a test binary has no wasm test (\"{NO_TESTS}\"); mark each test with wasm_bindgen_test"
            );
        }
        results.push((
            package,
            crate_run_passed(success, &output, filter.as_deref()),
        ));
    }
    match summary(&results) {
        Ok(lines) => {
            println!("{lines}");
            Ok(())
        }
        Err(lines) => bail!(
            "{lines}\nbrowser tests failed in {browser:?}; if no test ran, check the driver: {}",
            browser.setup_hint()
        ),
    }
}

/// An extra environment variable of one crate run, and why `web-test`
/// sets it.
#[derive(Debug, PartialEq, Eq)]
struct RunVar {
    name: &'static str,
    value: &'static str,
    why: &'static str,
}

/// TD-93: Safari 27.2 gives no `GPUAdapterInfo.subgroupMinSize`, and the
/// debug glue of the runner asserts that wgpu reads a number.
const NO_DEBUG: RunVar = RunVar {
    name: "WASM_BINDGEN_NO_DEBUG",
    value: "1",
    why: "Safari 27.2 gives no GPUAdapterInfo.subgroupMinSize, and the runner's debug glue asserts a number (TD-93)",
};

/// The extra variables of the crate run of `package` in `browser`.
fn run_vars(browser: Browser, package: &str) -> Vec<RunVar> {
    if browser == Browser::Safari && package == "stem-worker" {
        vec![NO_DEBUG]
    } else {
        Vec::new()
    }
}

/// One line for each crate run, with `passed` or `failed`.  It is an
/// error when a crate run failed, and the last line then names each
/// failed crate.
fn summary(results: &[(&str, bool)]) -> Result<String, String> {
    let lines: Vec<String> = results
        .iter()
        .map(|(package, passed)| {
            let verdict = if *passed { "passed" } else { "failed" };
            format!("web-test: {package}: {verdict}")
        })
        .collect();
    let failed: Vec<&str> = results
        .iter()
        .filter(|(_, passed)| !passed)
        .map(|(package, _)| *package)
        .collect();
    if failed.is_empty() {
        Ok(lines.join("\n"))
    } else {
        Err(format!(
            "{}\nweb-test: failed crates: {}",
            lines.join("\n"),
            failed.join(", ")
        ))
    }
}

/// The line that `wasm-bindgen-test-runner` prints when its WebDriver
/// session did not start.
const NO_SESSION: &str = "webdriver POST /session request failed";

/// Whether a crate run failed because the WebDriver session of a test
/// binary did not start, before any test of that binary ran.  Cargo prints
/// one `Running ... .wasm)` line for each test binary, and the runner
/// prints `running N tests` once its session started.  Cargo stops at the
/// first test binary that fails, so a later binary did not run.
fn session_did_not_start(output: &str) -> bool {
    let binaries = output
        .lines()
        .filter(|line| {
            let line = line.trim();
            line.starts_with("Running ") && line.ends_with(".wasm)")
        })
        .count();
    let started = output
        .lines()
        .filter(|line| {
            line.starts_with("running ") && (line.ends_with(" tests") || line.ends_with(" test"))
        })
        .count();
    output.contains(NO_SESSION) && started < binaries
}

/// The line that `wasm-bindgen-test-runner` prints, and exits 0, when a
/// test binary has no `wasm_bindgen_test` left, for example plain
/// `#[test]`s only.  The runner applies the filter first, so a binary
/// whose tests the filter removes prints it too.
const NO_TESTS: &str = "no tests to run!";

/// Whether a crate run passed: `cargo test` exited with success, and, with
/// no filter, each test binary had a wasm test.  With a filter, the marker
/// shows only that the filter matched no test of a binary.
fn crate_run_passed(success: bool, output: &str, filter: Option<&str>) -> bool {
    success && (filter.is_some() || !found_no_test(output))
}

/// Whether the output of a crate run shows a test binary with no wasm test.
fn found_no_test(output: &str) -> bool {
    output.lines().any(|line| line.trim() == NO_TESTS)
}

/// Runs `cmd`, prints its stdout and stderr as they come, and keeps both.
/// Gives whether it exited with success, and the kept text.
fn run_and_keep(cmd: &mut Command) -> Result<(bool, String)> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = child.stdout.take().context("no stdout of cargo test")?;
    let stderr = child.stderr.take().context("no stderr of cargo test")?;
    let errors = std::thread::spawn(move || {
        let mut kept = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            eprintln!("{line}");
            kept.push_str(&line);
            kept.push('\n');
        }
        kept
    });
    let mut kept = String::new();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        println!("{line}");
        kept.push_str(&line);
        kept.push('\n');
    }
    let status = child.wait().context("waiting for cargo test")?;
    let errors = errors
        .join()
        .map_err(|_| anyhow::anyhow!("the stderr reader of cargo test panicked"))?;
    kept.push_str(&errors);
    Ok((status.success(), kept))
}

/// Writes the `webdriver.json` of [`chrome_capabilities_json`] for the
/// runner, and gives its path.
fn chrome_capabilities(root: &std::path::Path, extra: &[String]) -> Result<std::path::PathBuf> {
    let path = root.join("target/web-test-chrome.json");
    std::fs::create_dir_all(root.join("target"))?;
    std::fs::write(&path, chrome_capabilities_json(extra))
        .context("writing the Chrome capabilities")?;
    Ok(path)
}

/// A `webdriver.json` that lets Chrome play audio without a click, so an
/// audio context runs and its clock moves (`app/tests/web_output.rs`).
/// `extra` holds the arguments of `--chrome-arg`, such as
/// `--disable-webgpu`.  The runner adds its own arguments (headless).
fn chrome_capabilities_json(extra: &[String]) -> String {
    let args: Vec<&str> = std::iter::once("--autoplay-policy=no-user-gesture-required")
        .chain(extra.iter().map(String::as_str))
        .collect();
    serde_json::json!({"goog:chromeOptions": {"args": args}}).to_string()
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
    let install =
        format!("cargo run -p xtask -- setup wasm-bindgen-cli (wasm-bindgen-cli {wanted})");
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
    fn a_failure_before_the_last_crate_names_that_crate() {
        let result = summary(&[
            ("app", false),
            ("stem-worker", true),
            ("decode-worker", false),
            ("stem-protocol", true),
        ]);
        assert_eq!(
            result,
            Err("web-test: app: failed\n\
                 web-test: stem-worker: passed\n\
                 web-test: decode-worker: failed\n\
                 web-test: stem-protocol: passed\n\
                 web-test: failed crates: app, decode-worker"
                .to_string())
        );
    }

    #[test]
    fn a_chrome_arg_goes_after_the_autoplay_policy() {
        let json: serde_json::Value =
            serde_json::from_str(&chrome_capabilities_json(&["--disable-webgpu".to_string()]))
                .unwrap();
        assert_eq!(
            json["goog:chromeOptions"]["args"],
            serde_json::json!([
                "--autoplay-policy=no-user-gesture-required",
                "--disable-webgpu"
            ])
        );
        let plain: serde_json::Value =
            serde_json::from_str(&chrome_capabilities_json(&[])).unwrap();
        assert_eq!(
            plain["goog:chromeOptions"]["args"],
            serde_json::json!(["--autoplay-policy=no-user-gesture-required"])
        );
    }

    #[test]
    fn a_test_binary_with_no_wasm_test_is_found() {
        let output = "     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/engine-1.wasm)\n\
                      no tests to run!\n";
        assert!(found_no_test(output));
        assert!(found_no_test("  no tests to run!  \r\n"));
    }

    #[test]
    fn a_line_that_only_holds_the_words_is_not_the_marker() {
        let output = "Running headless tests in Chrome on `http://127.0.0.1:65045/`\n\
                      test result: ok. 1 passed; 0 failed; 0 ignored; 196 filtered out; finished in 0.01s\n";
        assert!(!found_no_test(output));
        assert!(!found_no_test("test says: no tests to run! twice\n"));
    }

    #[test]
    fn the_marker_fails_only_a_run_with_no_filter() {
        let output = "no tests to run!\n";
        assert!(!crate_run_passed(true, output, None));
        assert!(crate_run_passed(true, output, Some("the_length")));
        assert!(crate_run_passed(true, "test result: ok. 3 passed\n", None));
        assert!(!crate_run_passed(
            false,
            "test result: FAILED. 0 passed; 1 failed\n",
            None
        ));
        assert!(!crate_run_passed(false, output, Some("the_length")));
    }

    #[test]
    fn chrome_gets_no_extra_variable_for_the_stem_worker() {
        assert_eq!(run_vars(Browser::Chrome, "stem-worker"), []);
    }

    #[test]
    fn safari_gets_no_extra_variable_for_the_other_crates() {
        for package in ["app", "decode-worker", "stem-protocol", "engine"] {
            assert_eq!(run_vars(Browser::Safari, package), [], "{package}");
        }
    }

    #[test]
    fn safari_turns_off_the_debug_glue_for_the_stem_worker() {
        let vars = run_vars(Browser::Safari, "stem-worker");
        assert_eq!(
            vars.iter().map(|v| (v.name, v.value)).collect::<Vec<_>>(),
            [("WASM_BINDGEN_NO_DEBUG", "1")]
        );
        assert!(vars[0].why.contains("subgroupMinSize"), "{}", vars[0].why);
    }

    /// The output of a crate run of `app` whose second test binary had no
    /// session, as Safari gave it on 2026-11-11 with a locked screen.
    const NO_SESSION_IN_THE_SECOND_BINARY: &str = "     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/app-1.wasm)\n\
         Running headless tests in Safari on `http://127.0.0.1:65014/`\n\
         running 197 tests\n\
         test result: ok. 197 passed; 0 failed; 0 ignored; 0 filtered out; finished in 202.20s\n\
              Running tests/web.rs (target/wasm32-unknown-unknown/debug/deps/web-2.wasm)\n\
         Running headless tests in Safari on `http://127.0.0.1:65028/`\n\
         driver status: signal: 9 (SIGKILL)\n\
         Error: webdriver POST /session request failed\n\
         \n\
         Caused by:\n\
             http status: 500\n\
         error: test failed, to rerun pass `-p app --test web`\n";

    #[test]
    fn a_binary_with_no_session_and_no_test_run_is_found() {
        assert!(session_did_not_start(NO_SESSION_IN_THE_SECOND_BINARY));
        let first = "     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/stem_worker-1.wasm)\n\
                     Running headless tests in Safari on `http://127.0.0.1:65028/`\n\
                     Error: webdriver POST /session request failed\n";
        assert!(session_did_not_start(first));
    }

    #[test]
    fn a_failed_test_is_not_a_session_that_did_not_start() {
        let failed = "     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/stem_worker-1.wasm)\n\
                      Running headless tests in Safari on `http://127.0.0.1:65028/`\n\
                      running 108 tests\n\
                      test web::tests::a_read_back_is_copied_into_wasm_memory_once ... FAIL\n\
                      test result: FAILED. 107 passed; 1 failed; 0 ignored; 0 filtered out; finished in 34.52s\n";
        assert!(!session_did_not_start(failed));
        let session_lost_during_tests = "     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/app-1.wasm)\n\
                                         running 1 test\n\
                                         Error: webdriver POST /session request failed\n";
        assert!(!session_did_not_start(session_lost_during_tests));
    }

    #[test]
    fn every_crate_passed_gives_one_line_for_each() {
        assert_eq!(
            summary(&[("app", true), ("stem-protocol", true)]),
            Ok("web-test: app: passed\nweb-test: stem-protocol: passed".to_string())
        );
    }

    #[test]
    fn the_workspace_lockfile_has_wasm_bindgen() {
        let lock =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.lock")).unwrap();
        assert!(locked_version(&lock, "wasm-bindgen").is_some());
    }
}
