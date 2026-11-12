//! `xtask web-test [--browser safari|chrome] [--chrome-arg <arg>]... [filter]`:
//! the browser tests
//! of `app` (song sessions in browser storage), `stem-worker` (model
//! storage, message handling), `decode-worker`, `stem-protocol` and
//! `engine` (the resampler with a 32-bit `usize`), compiled for wasm32 and
//! run in a real browser by `wasm-bindgen-test-runner`.  It keeps the
//! output of each crate run while it prints it, and fails a crate run
//! whose test binary has no wasm test, unless a filter is given.  With a
//! filter, it fails when no crate run ran a test.
//!
//! In Safari, the crate run of `stem-worker` gets `WASM_BINDGEN_NO_DEBUG=1`,
//! and `web-test` prints why (TD-93, [`run_vars`]).
//!
//! In Safari, it ends each stale `safaridriver` before each crate run.  If
//! the session of a test binary did not start before a test ran, it runs
//! that crate once more (`session_did_not_start`).  The second run runs
//! each test binary of the crate again, so a test that passed runs again.
//! No failed test runs again.
//!
//! It runs each crate, also after a crate fails.  Then it prints one line
//! for each crate, and fails with the name of each crate that failed.
//! It exits with [`HANG_EXIT_CODE`] when each failed crate failed only by
//! the runner's hang (TD-98, [`exit_code`]).  Each other failure exits
//! with 1.  CI runs a step again only after the hang exit code.
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

/// The options of `web-test`.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    browser: Browser,
    filter: Option<String>,
    chrome_args: Vec<String>,
}

/// Reads the arguments of `web-test`.  `browser` is the browser when no
/// `--browser` is given.
fn parse_args(args: &[String], browser: Browser) -> Result<Options> {
    let mut options = Options {
        browser,
        filter: None,
        chrome_args: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--browser" => {
                options.browser =
                    Browser::parse(args.get(i + 1).context("--browser needs a name")?)?;
                i += 1;
            }
            "--chrome-arg" => {
                let arg = args.get(i + 1).context("--chrome-arg needs an argument")?;
                options.chrome_args.push(arg.clone());
                i += 1;
            }
            other if other.starts_with("--") => bail!("unknown option {other}"),
            f => options.filter = Some(f.to_string()),
        }
        i += 1;
    }
    if options.browser != Browser::Chrome && !options.chrome_args.is_empty() {
        bail!("--chrome-arg needs --browser chrome");
    }
    Ok(options)
}

/// The exit code of a `web-test` whose each failed crate failed only by
/// the runner's hang (TD-98).  It is `EX_TEMPFAIL` of `sysexits.h`.
pub const HANG_EXIT_CODE: i32 = 75;

/// Runs `web-test`, and gives its exit code: 0 when it passed,
/// [`HANG_EXIT_CODE`] or 1 when it failed ([`exit_code`]).
pub fn main(args: &[String]) -> i32 {
    match run(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    }
}

fn run(args: &[String]) -> Result<i32> {
    let Options {
        browser,
        filter,
        chrome_args,
    } = parse_args(args, Browser::default_here())?;
    let root = crate::workspace_root();
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).context("reading Cargo.lock")?;
    let wanted = locked_version(&lock, "wasm-bindgen").context("no wasm-bindgen in Cargo.lock")?;
    check_runner(&wanted)?;
    let on_macos = cfg!(target_os = "macos");
    crate::drivers::refuse_a_locked_screen(browser, on_macos, crate::drivers::ioreg_root)?;
    let _active = crate::drivers::keep_user_active(browser, on_macos, crate::drivers::caffeinate);
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
    let mut outputs = Vec::with_capacity(runs.len());
    for (package, targets) in runs {
        let mut cmd = Command::new(env!("CARGO"));
        cmd.current_dir(&root);
        cmd.args(crate_run_args(package, targets, filter.as_deref()))
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
        let (success, output) = run_crate(
            browser,
            package,
            || run_and_keep(&mut cmd).with_context(|| format!("running cargo test for {package}")),
            || {
                crate::drivers::end_stale_safari_sessions(
                    crate::drivers::processes().as_deref(),
                    |mut kill| {
                        let _ = kill.status();
                    },
                )
            },
        )?;
        if let Some(line) = no_test_line(package, filter.as_deref(), &output) {
            println!("{line}");
        }
        results.push((
            package,
            crate_run_passed(success, &output, filter.as_deref()),
        ));
        outputs.push(output);
    }
    let End {
        stdout,
        stderr,
        code,
    } = end(browser, &results, &outputs, filter.as_deref());
    if let Some(lines) = stdout {
        println!("{lines}");
    }
    for lines in stderr {
        eprintln!("{lines}");
    }
    Ok(code)
}

/// What a `web-test` prints and gives at its end.
#[derive(Debug, PartialEq, Eq)]
struct End {
    /// The text for stdout, or `None`.
    stdout: Option<String>,
    /// The texts for stderr, in order.
    stderr: Vec<String>,
    /// The exit code.
    code: i32,
}

/// The [`End`] of a `web-test` from its [`verdict`].  A pass gives 0 and
/// the summary on stdout.  A failure gives the code of [`failure_end`],
/// and the error and the note on stderr.
fn end(
    browser: Browser,
    results: &[(&str, bool)],
    outputs: &[String],
    filter: Option<&str>,
) -> End {
    match verdict(results, outputs, filter) {
        Ok(lines) => End {
            stdout: Some(lines),
            stderr: Vec::new(),
            code: 0,
        },
        Err(lines) => {
            let error = format!(
                "Error: {lines}\nbrowser tests failed in {browser:?}; if no test ran, check the driver: {}",
                browser.setup_hint()
            );
            let (note, code) = failure_end(results, outputs);
            End {
                stdout: None,
                stderr: std::iter::once(error).chain(note).collect(),
                code,
            }
        }
    }
}

/// The line that `wasm-bindgen-test-runner` 0.2.130 prints when a test
/// page gives no `test result: ` line in the time limit (`headless.rs`).
const HANG: &str = "Failed to detect test as having been run. It might have timed out.";

/// The parts of lines that show a failure other than the hang and a
/// failed test.  They are a panic and a failed WebDriver request.
const NOT_A_HANG: [&str; 2] = ["panicked at ", "request failed"];

/// Whether `line` shows a failed test.  wasm-bindgen-test 0.3.80 prints
/// `test <name> ... FAIL` for it (`log_test` and `TestResult` in
/// `src/rt/mod.rs`).
fn a_failed_test(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("test ") && line.ends_with(" ... FAIL")
}

/// Whether the output of a failed crate run shows only the runner's hang.
/// It has the [`HANG`] line.  It has no line of [`a_failed_test`], and no
/// line that holds a part of [`NOT_A_HANG`].
fn failed_only_by_the_hang(output: &str) -> bool {
    let output = plain(output);
    output.lines().any(|line| line.trim() == HANG)
        && !output
            .lines()
            .any(|line| a_failed_test(line) || NOT_A_HANG.iter().any(|part| line.contains(part)))
}

/// The exit code of a failed `web-test`.  It is [`HANG_EXIT_CODE`] when at
/// least one crate failed, and [`failed_only_by_the_hang`] holds for each
/// failed crate.  Else it is 1.
fn exit_code(results: &[(&str, bool)], outputs: &[String]) -> i32 {
    let failed: Vec<&str> = results
        .iter()
        .zip(outputs)
        .filter(|((_, passed), _)| !passed)
        .map(|(_, output)| output.as_str())
        .collect();
    if !failed.is_empty() && failed.iter().all(|output| failed_only_by_the_hang(output)) {
        HANG_EXIT_CODE
    } else {
        1
    }
}

/// The note and the exit code of a failed `web-test`.  The code is
/// [`exit_code`].  The note is `Some` only for [`HANG_EXIT_CODE`].
fn failure_end(results: &[(&str, bool)], outputs: &[String]) -> (Option<String>, i32) {
    let code = exit_code(results, outputs);
    let note = (code == HANG_EXIT_CODE).then(|| {
        format!(
            "web-test: each failed crate failed only by the runner's hang (\"{HANG}\"); exit code {HANG_EXIT_CODE} (TD-98)"
        )
    });
    (note, code)
}

/// Runs one crate with `run`.  In Safari, it ends the stale sessions with
/// `end_stale` first.  If a run fails and [`session_did_not_start`] holds,
/// it ends them again and runs the crate once more.  Q8 of the tech debt
/// rungs allows a second try of a session start only.
fn run_crate(
    browser: Browser,
    package: &str,
    mut run: impl FnMut() -> Result<(bool, String)>,
    mut end_stale: impl FnMut(),
) -> Result<(bool, String)> {
    if browser != Browser::Safari {
        return run();
    }
    end_stale();
    let (success, output) = run()?;
    if success || !session_did_not_start(&output) {
        return Ok((success, output));
    }
    println!(
        "web-test: {package}: the Safari session did not start before a test ran; ending the stale sessions, and running {package} once more"
    );
    end_stale();
    run()
}

/// The line that tells why a crate run with no wasm test failed, or `None`.
fn no_test_line(package: &str, filter: Option<&str>, output: &str) -> Option<String> {
    (filter.is_none() && found_no_test(output)).then(|| {
        format!(
            "web-test: {package}: a test binary has no wasm test (\"{NO_TESTS}\"); mark each test with wasm_bindgen_test"
        )
    })
}

/// The [`summary`] of a `web-test`.  With a filter, it is also an error
/// when no crate run ran a test, since the runner passes a binary whose
/// tests the filter removes.
fn verdict(
    results: &[(&str, bool)],
    outputs: &[String],
    filter: Option<&str>,
) -> Result<String, String> {
    let lines = summary(results)?;
    match filter {
        Some(filter) if !outputs.iter().any(|output| a_test_ran(output)) => Err(format!(
            "{lines}\nweb-test: the filter \"{filter}\" matched no test in any crate"
        )),
        _ => Ok(lines),
    }
}

/// Whether the output of a crate run shows a test binary that ran a test:
/// the runner prints `running N tests` (or `running 1 test`) with N > 0.
fn a_test_ran(output: &str) -> bool {
    output.lines().any(|line| {
        let mut words = line.trim().split(' ');
        words.next() == Some("running")
            && words
                .next()
                .and_then(|n| n.parse::<u64>().ok())
                .is_some_and(|n| n > 0)
            && matches!(words.next(), Some("test" | "tests"))
            && words.next().is_none()
    })
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
    let output = plain(output);
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

/// `text` with no ANSI colour codes.  A code is `ESC [`, then parameters,
/// then one final character from `@` to `~`.  A code with no end goes to
/// the end of `text`.  An `ESC` with another next character removes both.
fn plain(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            kept.push(c);
            continue;
        }
        if chars.next() == Some('[') {
            chars.find(|c| ('@'..='~').contains(c));
        }
    }
    kept
}

/// The arguments of the `cargo test` of one crate run.  `--color never`
/// keeps Cargo's `Running` lines plain for [`session_did_not_start`], also
/// with CI's `CARGO_TERM_COLOR: always`.
fn crate_run_args(package: &str, targets: &[&str], filter: Option<&str>) -> Vec<String> {
    let fixed = ["test", "--locked", "--color", "never"];
    let target = ["--target", "wasm32-unknown-unknown", "-p", package];
    let filtered = filter.map(|f| ["--", f]);
    fixed
        .into_iter()
        .chain(target)
        .chain(targets.iter().copied())
        .chain(filtered.into_iter().flatten())
        .map(str::to_string)
        .collect()
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
/// `--disable-gpu`.  The runner adds its own arguments (headless).
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

    /// The output of a crate run that hung, as in CI run 43229036261
    /// (TD-98).
    const HUNG: &str = "     Running tests/web_output.rs (target/wasm32-unknown-unknown/debug/deps/web_output-ae8a3c20e4ab00f7.wasm)\n\
        Running headless tests in Safari on `http://127.0.0.1:49241/`\n\
        Loading Wasm module...\n\
        Failed to detect test as having been run. It might have timed out.\n\
        driver status: signal: 9 (SIGKILL)\n\
        Error: some tests failed\n\
        error: test failed, to rerun pass `-p app --test web_output`\n";

    /// The output of a crate run with a failed test, as in CI run
    /// 43152005940.
    const A_FAILED_TEST: &str = "test web::tests::a_mix_file_changed_after_the_open_gives_no_old_samples ... FAIL\n\
        failures:\n\
        \x20       panicked at worker/src/web/tests.rs:3576:24:\n\
        test result: FAILED. 109 passed; 1 failed; 0 ignored; 0 filtered out; finished in 63.72s\n";

    /// The output of a crate run whose WebDriver request failed, as in CI
    /// run 43223719719.
    const A_FAILED_REQUEST: &str = "Error: webdriver POST /session/ABE2DB61/execute/sync request failed\n\
        error: test failed, to rerun pass `-p engine --lib`\n";

    fn outputs(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| text.to_string()).collect()
    }

    /// Asserts:
    /// * one failed crate that hung gives [`HANG_EXIT_CODE`];
    /// * two failed crates that both hung give [`HANG_EXIT_CODE`];
    /// * a passed crate with any output does not change the code;
    /// * a hang line in colour gives [`HANG_EXIT_CODE`];
    /// * a hang after a passed test gives [`HANG_EXIT_CODE`];
    /// * a hang after a line that ends with " ... FAIL" but does not
    ///   start with "test " gives [`HANG_EXIT_CODE`].
    #[test]
    fn only_hung_crates_give_the_hang_exit_code() {
        let results = [("app", false), ("engine", true)];
        assert_eq!(
            exit_code(&results, &outputs(&[HUNG, A_FAILED_TEST])),
            HANG_EXIT_CODE
        );
        let both = [("app", false), ("stem-worker", false)];
        assert_eq!(exit_code(&both, &outputs(&[HUNG, HUNG])), HANG_EXIT_CODE);
        let coloured = HUNG.replace("Failed to detect", "\u{1b}[31mFailed to detect");
        assert_eq!(
            exit_code(&results[..1], &outputs(&[&coloured])),
            HANG_EXIT_CODE
        );
        let after_a_passed_test = format!("test lloyd ... ok\n{HUNG}");
        assert_eq!(
            exit_code(&results[..1], &outputs(&[&after_a_passed_test])),
            HANG_EXIT_CODE
        );
        let not_a_test_line = format!("lloyd: test ... FAIL\n{HUNG}");
        assert_eq!(
            exit_code(&results[..1], &outputs(&[&not_a_test_line])),
            HANG_EXIT_CODE
        );
    }

    /// Asserts that each of these gives 1:
    /// * a hung crate and a crate with a failed test (CI run 43152005940);
    /// * a failed WebDriver request (CI run 43223719719);
    /// * a hang after a failed test in the same crate run;
    /// * a hang after a failed test with no panic, on a line with spaces
    ///   around it;
    /// * a hang and a panic in the same crate run;
    /// * a hang and a failed WebDriver request in the same crate run;
    /// * a failed crate with no output;
    /// * no failed crate, as when a filter matched no test.
    #[test]
    fn each_other_failure_gives_exit_code_1() {
        let two = [("app", false), ("stem-worker", false)];
        assert_eq!(exit_code(&two, &outputs(&[HUNG, A_FAILED_TEST])), 1);
        let one = [("engine", false)];
        assert_eq!(exit_code(&one, &outputs(&[A_FAILED_REQUEST])), 1);
        let after_a_failed_test = format!("test lloyd ... FAIL\n{HUNG}");
        assert_eq!(exit_code(&one, &outputs(&[&after_a_failed_test])), 1);
        let with_spaces = format!("  test web::tests::lloyd ... FAIL \r\n{HUNG}");
        assert_eq!(exit_code(&one, &outputs(&[&with_spaces])), 1);
        let with_a_panic = format!("    panicked at app/src/lib.rs:1:1:\n{HUNG}");
        assert_eq!(exit_code(&one, &outputs(&[&with_a_panic])), 1);
        let with_a_request = format!("{HUNG}{A_FAILED_REQUEST}");
        assert_eq!(exit_code(&one, &outputs(&[&with_a_request])), 1);
        assert_eq!(exit_code(&one, &outputs(&[""])), 1);
        assert_eq!(exit_code(&[("app", true)], &outputs(&[HUNG])), 1);
    }

    /// Asserts:
    /// * a crate that hung gives [`HANG_EXIT_CODE`] and a note that names
    ///   the hang line, the code and TD-98;
    /// * a crate with a failed test gives 1 and no note.
    #[test]
    fn only_the_hang_exit_code_gets_the_hang_note() {
        let failed = [("app", false)];
        let (note, code) = failure_end(&failed, &outputs(&[HUNG]));
        assert_eq!(code, HANG_EXIT_CODE);
        assert_eq!(
            note.as_deref(),
            Some(
                "web-test: each failed crate failed only by the runner's hang (\"Failed to detect test as having been run. It might have timed out.\"); exit code 75 (TD-98)"
            )
        );
        assert_eq!(failure_end(&failed, &outputs(&[A_FAILED_TEST])), (None, 1));
    }

    /// Finding 3-1 of review round 3 of M3 of `gpu-speed`.  Asserts:
    /// * crates that all passed give 0, the summary on stdout and no
    ///   stderr;
    /// * a failed test gives 1, no stdout, and only the error on stderr;
    /// * a hung crate gives [`HANG_EXIT_CODE`], no stdout, and the error
    ///   and then the hang note on stderr;
    /// * a filter that matched no test gives 1 and no stdout.
    #[test]
    fn only_a_pass_ends_with_exit_code_0() {
        let passed = [("app", true)];
        let ran = outputs(&["running 1 test\n"]);
        assert_eq!(
            end(Browser::Safari, &passed, &ran, None),
            End {
                stdout: Some("web-test: app: passed".to_string()),
                stderr: Vec::new(),
                code: 0,
            }
        );
        let failed = [("stem-worker", false)];
        let a_failed_test = end(Browser::Safari, &failed, &outputs(&[A_FAILED_TEST]), None);
        assert_eq!(a_failed_test.code, 1);
        assert_eq!(a_failed_test.stdout, None);
        assert_eq!(a_failed_test.stderr.len(), 1, "{:?}", a_failed_test.stderr);
        assert!(
            a_failed_test.stderr[0].starts_with("Error: web-test: stem-worker: failed\n"),
            "{:?}",
            a_failed_test.stderr
        );
        assert!(
            a_failed_test.stderr[0].contains("browser tests failed in Safari; "),
            "{:?}",
            a_failed_test.stderr
        );
        let hung = end(Browser::Chrome, &[("app", false)], &outputs(&[HUNG]), None);
        assert_eq!(hung.code, HANG_EXIT_CODE);
        assert_eq!(hung.stdout, None);
        assert_eq!(hung.stderr.len(), 2, "{:?}", hung.stderr);
        assert_eq!(
            Some(&hung.stderr[1]),
            failure_end(&[("app", false)], &outputs(&[HUNG])).0.as_ref()
        );
        let no_match = end(Browser::Chrome, &passed, &outputs(&[""]), Some("lloyd"));
        assert_eq!(no_match.code, 1);
        assert_eq!(no_match.stdout, None);
    }

    /// Asserts that a line that only holds the hang text in other words
    /// is not the hang.
    #[test]
    fn only_the_whole_hang_line_is_the_hang() {
        assert!(failed_only_by_the_hang(HUNG));
        assert!(!failed_only_by_the_hang(
            "Failed to detect test as having been run.\n"
        ));
        assert!(!failed_only_by_the_hang(
            "note: Failed to detect test as having been run. It might have timed out. lloyd\n"
        ));
    }

    /// The lines of each step of the workflow `ci` that uses
    /// `nick-fields/retry`, trimmed.  A step goes from its `- ` line at
    /// the step indent to the next one.
    fn retry_steps(ci: &str) -> Vec<Vec<&str>> {
        ci.split("\n      - ")
            .filter(|step| step.contains("uses: nick-fields/retry@"))
            .map(|step| step.lines().map(str::trim).collect())
            .collect()
    }

    /// Findings 1-5 of review round 1 and 3-10 of review round 3 of M3 of
    /// `gpu-speed`.  Asserts that CI has 2 retry steps, and that each:
    /// * runs `xtask -- web-test`;
    /// * retries on [`HANG_EXIT_CODE`] only;
    /// * has `warning_on_retry: true`;
    /// * has no `retry_on`, which would retry each failure;
    /// * has no `continue_on_error` and no step `continue-on-error`, which
    ///   would pass a failed step;
    /// * has no `new_command_on_retry`, which would run another command.
    #[test]
    fn ci_runs_web_test_again_only_after_the_hang() {
        let steps = retry_steps(include_str!("../../.github/workflows/ci.yml"));
        assert_eq!(steps.len(), 2, "{steps:?}");
        let code = format!("retry_on_exit_code: {HANG_EXIT_CODE}");
        for lines in steps {
            assert!(
                lines
                    .iter()
                    .any(|l| l.starts_with("command: cargo run --locked -p xtask -- web-test ")),
                "{lines:?}"
            );
            assert!(lines.contains(&code.as_str()), "{lines:?}");
            assert!(lines.contains(&"warning_on_retry: true"), "{lines:?}");
            for refused in [
                "retry_on:",
                "continue_on_error:",
                "continue-on-error:",
                "new_command_on_retry:",
            ] {
                assert!(
                    !lines.iter().any(|l| l.starts_with(refused)),
                    "{refused} {lines:?}"
                );
            }
        }
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
            serde_json::from_str(&chrome_capabilities_json(&["--disable-gpu".to_string()]))
                .unwrap();
        assert_eq!(
            json["goog:chromeOptions"]["args"],
            serde_json::json!([
                "--autoplay-policy=no-user-gesture-required",
                "--disable-gpu"
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
        let no_session_text = "     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/app-1.wasm)\n\
                               no tests to run!\n\
                                    Running tests/web.rs (target/wasm32-unknown-unknown/debug/deps/web-2.wasm)\n\
                               running 3 tests\n\
                               test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 filtered out; finished in 1.20s\n";
        assert!(
            !session_did_not_start(no_session_text),
            "a binary with no running line, and no NO_SESSION text"
        );
    }

    /// Review round 2 of M2 (2-1): CI sets `CARGO_TERM_COLOR: always`, so
    /// Cargo's `Running` line starts with colour codes.
    #[test]
    fn a_coloured_running_line_is_a_test_binary() {
        let coloured = "\u{1b}[1m\u{1b}[92m     Running\u{1b}[0m unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/stem_worker-1.wasm)\n\
                        Running headless tests in Safari on `http://127.0.0.1:65028/`\n\
                        Error: webdriver POST /session request failed\n";
        assert!(session_did_not_start(coloured));
        let started = format!("{coloured}\u{1b}[0mrunning 108 tests\u{1b}[0m\n");
        assert!(!session_did_not_start(&started));
    }

    #[test]
    fn plain_text_keeps_the_text_around_the_colour_codes() {
        assert_eq!(
            plain("\u{1b}[1m\u{1b}[92m     Running\u{1b}[0m lloyd.wasm)"),
            "     Running lloyd.wasm)"
        );
        assert_eq!(plain("beachape [acme]"), "beachape [acme]");
        assert_eq!(plain("\u{1b}[38;5;208macme"), "acme");
        assert_eq!(plain("lloyd\u{1b}"), "lloyd");
        assert_eq!(plain("lloyd\u{1b}[31"), "lloyd");
        assert_eq!(plain("lloyd\u{1b}7acme"), "lloydacme");
    }

    #[test]
    fn a_crate_run_asks_cargo_for_no_colour() {
        let args = crate_run_args("app", &["--lib"], Some("a_lloyd_test"));
        assert_eq!(
            args,
            strings(&[
                "test",
                "--locked",
                "--color",
                "never",
                "--target",
                "wasm32-unknown-unknown",
                "-p",
                "app",
                "--lib",
                "--",
                "a_lloyd_test",
            ])
        );
        assert_eq!(
            crate_run_args("engine", &["--lib"], None),
            strings(&[
                "test",
                "--locked",
                "--color",
                "never",
                "--target",
                "wasm32-unknown-unknown",
                "-p",
                "engine",
                "--lib",
            ])
        );
    }

    #[test]
    fn the_headless_line_is_not_a_test_binary() {
        let lost = "     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/app-1.wasm)\n\
                    Running headless tests in Safari on `http://127.0.0.1:65014/`\n\
                    running 1 test\n\
                    Error: webdriver POST /session request failed\n";
        assert!(!session_did_not_start(lost));
    }

    #[test]
    fn only_a_running_line_shows_a_started_session() {
        let one_binary_without_a_session = "     Running unittests src/lib.rs (target/wasm32-unknown-unknown/debug/deps/app-1.wasm)\n\
                                            the runner did not start a test\n\
                                            Error: webdriver POST /session request failed\n";
        assert!(session_did_not_start(one_binary_without_a_session));
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

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn a_chrome_arg_takes_the_next_argument() {
        let options = parse_args(
            &strings(&[
                "--chrome-arg",
                "--disable-gpu",
                "--browser",
                "chrome",
                "--chrome-arg",
                "--mute-audio",
                "a_read_back",
            ]),
            Browser::Safari,
        )
        .unwrap();
        assert_eq!(
            options,
            Options {
                browser: Browser::Chrome,
                filter: Some("a_read_back".to_string()),
                chrome_args: strings(&["--disable-gpu", "--mute-audio"]),
            }
        );
    }

    #[test]
    fn no_argument_gives_the_default_browser() {
        assert_eq!(
            parse_args(&[], Browser::Safari).unwrap(),
            Options {
                browser: Browser::Safari,
                filter: None,
                chrome_args: Vec::new(),
            }
        );
    }

    #[test]
    fn a_chrome_arg_needs_chrome_and_a_value() {
        let error = parse_args(
            &strings(&["--chrome-arg", "--disable-gpu"]),
            Browser::Safari,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "--chrome-arg needs --browser chrome");
        let error = parse_args(&strings(&["--chrome-arg"]), Browser::Chrome)
            .unwrap_err()
            .to_string();
        assert_eq!(error, "--chrome-arg needs an argument");
        assert!(
            parse_args(
                &strings(&["--chrome-arg", "--disable-gpu"]),
                Browser::Chrome
            )
            .is_ok()
        );
        let error = parse_args(
            &strings(&["--browser", "chrome", "--lloyd"]),
            Browser::Safari,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "unknown option --lloyd");
    }

    /// The order of the calls of `run_crate`, with a run that gives each
    /// of `outputs` in turn.
    fn crate_calls(browser: Browser, outputs: &[(bool, &str)]) -> (Vec<String>, (bool, String)) {
        let calls = std::cell::RefCell::new(Vec::new());
        let mut next = outputs.iter();
        let result = run_crate(
            browser,
            "app",
            || {
                calls.borrow_mut().push("run".to_string());
                let (success, output) = next.next().expect("no more runs");
                Ok((*success, output.to_string()))
            },
            || calls.borrow_mut().push("end".to_string()),
        )
        .unwrap();
        (calls.into_inner(), result)
    }

    #[test]
    fn safari_ends_the_stale_sessions_before_a_crate_run() {
        let (calls, result) = crate_calls(Browser::Safari, &[(true, "running 3 tests\n")]);
        assert_eq!(calls, ["end", "run"]);
        assert_eq!(result, (true, "running 3 tests\n".to_string()));
    }

    #[test]
    fn safari_runs_a_crate_again_only_when_its_session_did_not_start() {
        let (calls, result) = crate_calls(
            Browser::Safari,
            &[
                (false, NO_SESSION_IN_THE_SECOND_BINARY),
                (true, "running 3 tests\n"),
            ],
        );
        assert_eq!(calls, ["end", "run", "end", "run"]);
        assert_eq!(result, (true, "running 3 tests\n".to_string()));
        let failed_test = "running 3 tests\ntest result: FAILED. 2 passed; 1 failed\n";
        let (calls, result) = crate_calls(Browser::Safari, &[(false, failed_test)]);
        assert_eq!(calls, ["end", "run"]);
        assert_eq!(result, (false, failed_test.to_string()));
    }

    #[test]
    fn a_passed_safari_run_is_not_run_again() {
        let (calls, _) = crate_calls(Browser::Safari, &[(true, NO_SESSION_IN_THE_SECOND_BINARY)]);
        assert_eq!(calls, ["end", "run"]);
    }

    #[test]
    fn chrome_runs_a_crate_once_and_ends_no_session() {
        let (calls, result) =
            crate_calls(Browser::Chrome, &[(false, NO_SESSION_IN_THE_SECOND_BINARY)]);
        assert_eq!(calls, ["run"]);
        assert!(!result.0);
    }

    #[test]
    fn the_no_test_line_is_only_for_a_run_with_no_filter() {
        let line = no_test_line("engine", None, "no tests to run!\n").unwrap();
        assert!(
            line.starts_with("web-test: engine: a test binary has no wasm test"),
            "{line}"
        );
        assert_eq!(
            no_test_line("engine", Some("the_length"), "no tests to run!\n"),
            None
        );
        assert_eq!(no_test_line("engine", None, "running 3 tests\n"), None);
    }

    #[test]
    fn a_filter_that_matches_no_test_fails() {
        let outputs = [
            "no tests to run!\n".to_string(),
            "no tests to run!\n".to_string(),
        ];
        assert_eq!(
            verdict(
                &[("app", true), ("engine", true)],
                &outputs,
                Some("a_lloyd_test")
            ),
            Err("web-test: app: passed\n\
                 web-test: engine: passed\n\
                 web-test: the filter \"a_lloyd_test\" matched no test in any crate"
                .to_string())
        );
    }

    #[test]
    fn a_filter_that_matches_in_one_crate_passes() {
        let outputs = [
            "no tests to run!\n".to_string(),
            "running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 196 filtered out\n"
                .to_string(),
        ];
        assert_eq!(
            verdict(
                &[("app", true), ("stem-worker", true)],
                &outputs,
                Some("a_real_aac")
            ),
            Ok("web-test: app: passed\nweb-test: stem-worker: passed".to_string())
        );
    }

    #[test]
    fn a_run_with_no_filter_needs_no_running_line() {
        assert_eq!(
            verdict(&[("app", true)], &[String::new()], None),
            Ok("web-test: app: passed".to_string())
        );
        assert!(verdict(&[("app", false)], &["running 3 tests\n".to_string()], None).is_err());
    }

    #[test]
    fn only_a_running_line_with_a_test_shows_a_test_ran() {
        assert!(a_test_ran("running 1 test\n"));
        assert!(a_test_ran("  running 197 tests  \n"));
        assert!(!a_test_ran("running 0 tests\n"));
        assert!(!a_test_ran("no tests to run!\n"));
        assert!(!a_test_ran("running 3 tests of lloyd\n"));
        assert!(!a_test_ran("running three tests\n"));
        assert!(!a_test_ran("running 3 benches\n"));
        assert!(!a_test_ran("Running headless tests in Chrome\n"));
    }

    #[test]
    fn a_run_keeps_its_stdout_then_its_stderr() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo lloyd; echo beachape >&2; echo acme"]);
        assert_eq!(
            run_and_keep(&mut cmd).unwrap(),
            (true, "lloyd\nacme\nbeachape\n".to_string())
        );
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo lloyd; exit 3"]);
        assert_eq!(
            run_and_keep(&mut cmd).unwrap(),
            (false, "lloyd\n".to_string())
        );
    }

    #[test]
    fn a_runner_of_another_version_is_refused() {
        let error = check_runner("0.0.0-lloyd").unwrap_err().to_string();
        assert!(
            error.contains(
                "cargo run -p xtask -- setup wasm-bindgen-cli (wasm-bindgen-cli 0.0.0-lloyd)"
            ),
            "{error}"
        );
    }

    #[test]
    fn the_chrome_capabilities_go_in_the_target_folder() {
        let root = std::env::temp_dir().join(format!("xtask-web-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let extra = ["--disable-gpu".to_string()];
        let path = chrome_capabilities(&root, &extra).unwrap();
        assert_eq!(path, root.join("target/web-test-chrome.json"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            chrome_capabilities_json(&extra)
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
