//! `xtask setup [--check] [tool...]`: installs each dev tool at its pin.
//!
//! The pins of Trunk and cargo-mutants are here.  wasm-bindgen-cli takes
//! the `wasm-bindgen` of `Cargo.lock`, and the nightly is
//! `check::DOC_NIGHTLY`.  `README.md` and CI point to this command for the
//! installs.  `--check` installs nothing, and fails when a tool is missing
//! or at another version.  CI keeps one copy of the Trunk pin, beside the
//! SHA-256 of its release binary, and a test keeps it equal to [`TRUNK`].

use crate::check::DOC_NIGHTLY;
use crate::web_test::locked_version;
use anyhow::{Context, Result, bail};
use std::process::Command;

/// The Trunk that builds the app.
pub const TRUNK: &str = "0.22.1";

/// The cargo-mutants of `xtask review mutants`.
pub const CARGO_MUTANTS: &str = "27.1.0";

/// The target of the web build and of the browser tests.
pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// How setup reads what is installed.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Probe {
    /// A command whose first line is `<name> <version>`.
    Version(Vec<String>),
    /// A command that lists one installed item on each line, such as
    /// `rustup target list --installed`.  The item is installed when a
    /// line starts with the pin.
    Listed(Vec<String>),
}

/// A dev tool, its pin, and how to read and install it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub name: &'static str,
    pub pin: String,
    probe: Probe,
    install: Vec<String>,
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// Each tool, with the pins of this checkout.
pub fn tools(lock: &str) -> Result<Vec<Tool>> {
    let bindgen = locked_version(lock, "wasm-bindgen").context("no wasm-bindgen in Cargo.lock")?;
    let cargo_tool = |name: &'static str, krate: &str, pin: &str, probe: &str| Tool {
        name,
        pin: pin.to_string(),
        probe: Probe::Version(words(probe)),
        install: words(&format!(
            "cargo install --locked {krate} --version {pin} --force"
        )),
    };
    Ok(vec![
        Tool {
            name: "wasm32",
            pin: WASM_TARGET.to_string(),
            probe: Probe::Listed(words("rustup target list --installed")),
            install: words(&format!("rustup target add {WASM_TARGET}")),
        },
        Tool {
            name: "nightly",
            pin: DOC_NIGHTLY.to_string(),
            probe: Probe::Listed(words("rustup toolchain list")),
            install: words(&format!(
                "rustup toolchain install {DOC_NIGHTLY} --profile minimal"
            )),
        },
        cargo_tool("trunk", "trunk", TRUNK, "trunk --version"),
        cargo_tool(
            "wasm-bindgen-cli",
            "wasm-bindgen-cli",
            &bindgen,
            "wasm-bindgen-test-runner --version",
        ),
        cargo_tool(
            "cargo-mutants",
            "cargo-mutants",
            CARGO_MUTANTS,
            "cargo mutants --version",
        ),
    ])
}

/// The version that a probe's output gives, or `None` for no such tool.
fn installed(probe: &Probe, pin: &str, stdout: Option<&str>) -> Option<String> {
    let stdout = stdout?;
    match probe {
        Probe::Version(_) => stdout
            .lines()
            .next()?
            .split_whitespace()
            .nth(1)
            .map(str::to_string),
        Probe::Listed(_) => stdout
            .lines()
            .any(|line| line.trim().starts_with(pin))
            .then(|| pin.to_string()),
    }
}

/// What setup does with one tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Keep,
    Install { have: Option<String> },
}

pub fn action(tool: &Tool, have: Option<String>) -> Action {
    if have.as_deref() == Some(tool.pin.as_str()) {
        Action::Keep
    } else {
        Action::Install { have }
    }
}

pub fn line(tool: &Tool, action: &Action) -> String {
    match action {
        Action::Keep => format!("ok: {} {}", tool.name, tool.pin),
        Action::Install { have } => format!(
            "install: {} {} (installed: {}): {}",
            tool.name,
            tool.pin,
            have.as_deref().unwrap_or("none"),
            tool.install.join(" ")
        ),
    }
}

/// The tools that the arguments name, or all of them for no name.
pub fn chosen(all: Vec<Tool>, names: &[String]) -> Result<Vec<Tool>> {
    if let Some(unknown) = names
        .iter()
        .find(|name| !all.iter().any(|tool| tool.name == name.as_str()))
    {
        let known: Vec<_> = all.iter().map(|tool| tool.name).collect();
        bail!("unknown tool {unknown}; the tools are {}", known.join(", "));
    }
    Ok(all
        .into_iter()
        .filter(|tool| names.is_empty() || names.iter().any(|name| name == tool.name))
        .collect())
}

fn run_probe(probe: &Probe) -> Option<String> {
    let (Probe::Version(command) | Probe::Listed(command)) = probe;
    crate::stdout_on_success(
        Command::new(&command[0])
            .args(&command[1..])
            .current_dir(crate::workspace_root()),
    )
}

fn have(tool: &Tool) -> Option<String> {
    installed(&tool.probe, &tool.pin, run_probe(&tool.probe).as_deref())
}

/// Runs the install command of `tool` in `root`, and gives whether it
/// exited with success.
fn install(tool: &Tool, root: &std::path::Path) -> Result<bool> {
    let status = Command::new(&tool.install[0])
        .args(&tool.install[1..])
        .current_dir(root)
        .status()
        .with_context(|| format!("running {}", tool.install.join(" ")))?;
    Ok(status.success())
}

/// The arguments of `setup`: whether `--check` is given, and the names
/// of the tools.
fn parse_args(args: &[String]) -> Result<(bool, Vec<String>)> {
    let check_only = args.iter().any(|arg| arg == "--check");
    if let Some(option) = args
        .iter()
        .find(|arg| arg.starts_with("--") && arg.as_str() != "--check")
    {
        bail!("unknown option {option}; usage: xtask setup [--check] [tool...]");
    }
    let names = args
        .iter()
        .filter(|arg| !arg.starts_with("--"))
        .cloned()
        .collect();
    Ok((check_only, names))
}

/// `xtask setup`: [`run_with`] with the real probe and install.  No test
/// calls it, since a test of it could install a tool.  So cargo-mutants
/// makes no mutant of it.
#[cfg_attr(mutants, mutants::skip)]
pub fn run(args: &[String]) -> Result<()> {
    let root = crate::workspace_root();
    run_with(args, &root, have, |tool| install(tool, &root))
}

/// Runs `setup` with `args` for the checkout in `root`.  `have` reads the
/// installed version of a tool, and `install` installs a tool (see
/// [`set_up`]).
fn run_with(
    args: &[String],
    root: &std::path::Path,
    have: impl FnMut(&Tool) -> Option<String>,
    install: impl FnMut(&Tool) -> Result<bool>,
) -> Result<()> {
    let (check_only, names) = parse_args(args)?;
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).context("reading Cargo.lock")?;
    let tools = chosen(tools(&lock)?, &names)?;
    set_up(&tools, check_only, have, install)
}

/// Keeps each of `tools` that is at its pin, and installs each other one
/// with `install`, or only lists it with `check_only`.  `have` reads the
/// installed version of a tool.  It fails when a tool is not at its pin
/// at the end.
fn set_up(
    tools: &[Tool],
    check_only: bool,
    mut have: impl FnMut(&Tool) -> Option<String>,
    mut install: impl FnMut(&Tool) -> Result<bool>,
) -> Result<()> {
    let mut missing = Vec::new();
    for tool in tools {
        let action = action(tool, have(tool));
        println!("{}", line(tool, &action));
        if action == Action::Keep {
            continue;
        }
        if check_only {
            missing.push(tool.name);
            continue;
        }
        let succeeded = install(tool)?;
        if let Some(problem) = action_after(tool, succeeded, have(tool)) {
            bail!(problem);
        }
        println!("ok: {} {}", tool.name, tool.pin);
    }
    if !missing.is_empty() {
        bail!(
            "{} tool(s) not at the pin: {}; run `cargo run -p xtask -- setup`",
            missing.len(),
            missing.join(", ")
        );
    }
    Ok(())
}

/// The problem after an install, if the tool is still not at its pin.
fn action_after(tool: &Tool, succeeded: bool, have: Option<String>) -> Option<String> {
    if !succeeded {
        return Some(format!("`{}` failed", tool.install.join(" ")));
    }
    match action(tool, have) {
        Action::Keep => None,
        Action::Install { have } => Some(format!(
            "after `{}`, {} is {}, not {}; check that PATH finds the new one first",
            tool.install.join(" "),
            tool.name,
            have.as_deref().unwrap_or("not found"),
            tool.pin
        )),
    }
}

/// Each version that `text` gives for `name`: the first token like
/// `1.2.3` among the four after each mention of the name, or in the
/// mention, as in `name@1.2.3`.  The match ignores case.
pub fn pin_mentions(text: &str, name: &str) -> Vec<String> {
    let name = name.to_lowercase();
    let tokens: Vec<String> = text.split_whitespace().map(clean).collect();
    let mut found = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        let after_at = token
            .strip_prefix(&name)
            .and_then(|rest| rest.strip_prefix('@'));
        if let Some(version) = after_at.filter(|version| is_version(version)) {
            found.push(version.to_string());
        } else if *token == name
            && let Some(version) = tokens[at + 1..]
                .iter()
                .take(4)
                .find(|token| is_version(token))
        {
            found.push(version.clone());
        }
    }
    found
}

/// A token without its punctuation, and a Markdown link as its text.
fn clean(token: &str) -> String {
    let token = token
        .strip_prefix('[')
        .and_then(|link| link.split_once("]("))
        .map_or(token, |(text, _)| text);
    token
        .trim_matches(|c: char| "`'\"*,.;:()[]|".contains(c))
        .to_lowercase()
}

fn is_version(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = "[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.129\"\n";

    fn tool(name: &str) -> Tool {
        tools(LOCK)
            .unwrap()
            .into_iter()
            .find(|tool| tool.name == name)
            .unwrap()
    }

    #[test]
    fn wasm_bindgen_cli_takes_the_locked_version() {
        let cli = tool("wasm-bindgen-cli");
        assert_eq!(cli.pin, "0.2.129");
        assert_eq!(
            cli.install.join(" "),
            "cargo install --locked wasm-bindgen-cli --version 0.2.129 --force"
        );
        assert!(tools("").is_err(), "a lock without wasm-bindgen fails");
    }

    #[test]
    fn a_version_probe_reads_the_second_word_of_the_first_line() {
        let trunk = tool("trunk");
        assert_eq!(
            installed(
                &trunk.probe,
                &trunk.pin,
                Some("trunk 0.21.14\nmore 9.9.9\n")
            ),
            Some("0.21.14".to_string())
        );
        assert_eq!(installed(&trunk.probe, &trunk.pin, Some("")), None);
        assert_eq!(installed(&trunk.probe, &trunk.pin, None), None);
    }

    #[test]
    fn a_listed_probe_finds_a_line_that_starts_with_the_pin() {
        let nightly = tool("nightly");
        let list =
            format!("stable-aarch64-apple-darwin (default)\n{DOC_NIGHTLY}-aarch64-apple-darwin\n");
        assert_eq!(
            installed(&nightly.probe, &nightly.pin, Some(&list)),
            Some(DOC_NIGHTLY.to_string())
        );
        assert_eq!(
            installed(
                &nightly.probe,
                &nightly.pin,
                Some("stable-aarch64-apple-darwin\n")
            ),
            None
        );
        let other = format!("x-{DOC_NIGHTLY}\n");
        assert_eq!(
            installed(&nightly.probe, &nightly.pin, Some(&other)),
            None,
            "the pin starts the line"
        );
    }

    #[test]
    fn setup_installs_a_tool_that_is_missing_or_at_another_version() {
        let trunk = tool("trunk");
        assert_eq!(action(&trunk, Some(TRUNK.to_string())), Action::Keep);
        assert_eq!(action(&trunk, None), Action::Install { have: None });
        assert_eq!(
            action(&trunk, Some("0.20.0".to_string())),
            Action::Install {
                have: Some("0.20.0".to_string())
            }
        );
        assert_eq!(
            line(&trunk, &Action::Install { have: None }),
            format!(
                "install: trunk {TRUNK} (installed: none): cargo install --locked trunk --version {TRUNK} --force"
            )
        );
        assert_eq!(line(&trunk, &Action::Keep), format!("ok: trunk {TRUNK}"));
    }

    #[test]
    fn an_install_that_leaves_another_version_fails() {
        let trunk = tool("trunk");
        assert_eq!(action_after(&trunk, true, Some(TRUNK.to_string())), None);
        let failed = action_after(&trunk, false, Some(TRUNK.to_string())).unwrap();
        assert!(failed.contains("failed"), "{failed}");
        let shadowed = action_after(&trunk, true, Some("0.20.0".to_string())).unwrap();
        assert!(shadowed.contains("is 0.20.0, not"), "{shadowed}");
        let gone = action_after(&trunk, true, None).unwrap();
        assert!(gone.contains("is not found"), "{gone}");
    }

    #[test]
    fn the_arguments_choose_tools_by_name() {
        let all = tools(LOCK).unwrap();
        assert_eq!(chosen(all.clone(), &[]).unwrap(), all);
        let names: Vec<_> = chosen(all.clone(), &["trunk".to_string()])
            .unwrap()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(names, ["trunk"]);
        let error = chosen(all, &["trunkk".to_string()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown tool trunkk"), "{error}");
    }

    #[test]
    fn a_mention_gives_the_version_after_the_name() {
        assert_eq!(
            pin_mentions("[Trunk](https://x) 0.21.14, the version", "trunk"),
            ["0.21.14"],
            "a link counts as its text"
        );
        assert_eq!(
            pin_mentions("Trunk 0.21.14, the version", "trunk"),
            ["0.21.14"]
        );
        assert_eq!(
            pin_mentions("cargo install --locked trunk --version 0.21.14", "trunk"),
            ["0.21.14"]
        );
        assert_eq!(
            pin_mentions(
                "`cargo install --locked cargo-mutants@27.1.0`",
                "cargo-mutants"
            ),
            ["27.1.0"]
        );
        assert_eq!(
            pin_mentions(
                "Trunk downloads `wasm-bindgen` and `wasm-opt` into its cache",
                "trunk"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            pin_mentions("trunk a b c d 1.2.3", "trunk"),
            Vec::<String>::new(),
            "only four tokens after the name"
        );
        assert_eq!(pin_mentions("trunk 1.2", "trunk"), Vec::<String>::new());
    }

    /// A tool whose probe and install are harmless shell commands.
    fn shell_tool(probe: &str, install: &str) -> Tool {
        Tool {
            name: "trunk",
            pin: "1.2.3".to_string(),
            probe: Probe::Version(vec!["sh".into(), "-c".into(), probe.into()]),
            install: vec!["sh".into(), "-c".into(), install.into()],
        }
    }

    #[test]
    fn a_probe_gives_the_stdout_of_a_command_that_succeeds() {
        let trunk = shell_tool("echo trunk 1.2.3", "exit 0");
        assert_eq!(run_probe(&trunk.probe), Some("trunk 1.2.3\n".to_string()));
        assert_eq!(have(&trunk), Some("1.2.3".to_string()));
        let failed = shell_tool("echo trunk 1.2.3; exit 1", "exit 0");
        assert_eq!(run_probe(&failed.probe), None);
        assert_eq!(have(&failed), None);
    }

    #[test]
    fn an_install_gives_whether_its_command_succeeded() {
        let root = crate::workspace_root();
        assert!(install(&shell_tool("", "exit 0"), &root).unwrap());
        assert!(!install(&shell_tool("", "exit 1"), &root).unwrap());
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn the_arguments_give_check_and_the_tool_names() {
        assert_eq!(
            parse_args(&args(&["trunk", "--check", "wasm32"])).unwrap(),
            (true, args(&["trunk", "wasm32"]))
        );
        assert_eq!(
            parse_args(&args(&["trunk"])).unwrap(),
            (false, args(&["trunk"]))
        );
        assert_eq!(parse_args(&[]).unwrap(), (false, Vec::new()));
    }

    #[test]
    fn an_unknown_option_fails() {
        let error = parse_args(&args(&["--check", "--lloyd"]))
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("unknown option --lloyd;"), "{error}");
    }

    /// An install that a test must not reach.
    fn no_install(tool: &Tool) -> Result<bool> {
        panic!("{} installed", tool.name)
    }

    /// Review round 3 of M2 (3-1): `run` gives its work to `run_with`.
    #[test]
    fn setup_with_an_unknown_option_fails_and_installs_nothing() {
        let root = crate::workspace_root();
        let result = run_with(&args(&["--lloyd"]), &root, |_| None, no_install);
        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.to_string().starts_with("unknown option --lloyd;")),
            "{result:?}"
        );
        let result = run_with(&args(&["--check", "lloyd"]), &root, |_| None, no_install);
        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.to_string().starts_with("unknown tool lloyd;")),
            "{result:?}"
        );
    }

    #[test]
    fn setup_check_with_a_tool_not_at_its_pin_fails_and_installs_nothing() {
        let root = crate::workspace_root();
        let result = run_with(
            &args(&["--check", "trunk", "nightly"]),
            &root,
            |tool| (tool.name == "nightly").then(|| tool.pin.clone()),
            no_install,
        );
        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("1 tool(s) not at the pin: trunk; run `cargo run -p xtask -- setup`".to_string())
        );
    }

    #[test]
    fn setup_with_each_tool_at_its_pin_passes() {
        let root = crate::workspace_root();
        let result = run_with(&[], &root, |tool| Some(tool.pin.clone()), no_install);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn setup_installs_the_named_tool_with_the_locked_pin() {
        let root = crate::workspace_root();
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        let bindgen = locked_version(&lock, "wasm-bindgen").unwrap();
        let installed = std::cell::RefCell::new(Vec::new());
        let result = run_with(
            &args(&["wasm-bindgen-cli"]),
            &root,
            |tool| {
                installed
                    .borrow()
                    .iter()
                    .any(|(name, _)| *name == tool.name)
                    .then(|| tool.pin.clone())
            },
            |tool| {
                installed.borrow_mut().push((tool.name, tool.pin.clone()));
                Ok(true)
            },
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(installed.into_inner(), [("wasm-bindgen-cli", bindgen)]);
    }

    #[test]
    fn check_lists_each_tool_not_at_its_pin_and_installs_nothing() {
        let tools = [tool("trunk"), tool("wasm-bindgen-cli"), tool("nightly")];
        let error = set_up(
            &tools,
            /* check_only= */ true,
            |tool| (tool.name == "wasm-bindgen-cli").then(|| tool.pin.clone()),
            |tool| panic!("{} installed", tool.name),
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error,
            "2 tool(s) not at the pin: trunk, nightly; run `cargo run -p xtask -- setup`"
        );
        assert!(
            set_up(
                &tools,
                /* check_only= */ true,
                |tool| Some(tool.pin.clone()),
                |tool| panic!("{} installed", tool.name),
            )
            .is_ok()
        );
    }

    #[test]
    fn setup_installs_only_a_tool_not_at_its_pin() {
        let tools = [tool("trunk"), tool("wasm-bindgen-cli")];
        let installed = std::cell::RefCell::new(Vec::new());
        let result = set_up(
            &tools,
            /* check_only= */ false,
            |tool| {
                (tool.name == "wasm-bindgen-cli" || installed.borrow().contains(&tool.name))
                    .then(|| tool.pin.clone())
            },
            |tool| {
                installed.borrow_mut().push(tool.name);
                Ok(true)
            },
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(installed.into_inner(), ["trunk"]);
        let error = set_up(
            &tools,
            /* check_only= */ false,
            |_| None,
            |_| Ok(false),
        )
        .unwrap_err()
        .to_string();
        assert!(error.ends_with("--force` failed"), "{error}");
    }

    #[test]
    fn a_version_can_be_the_fourth_word_after_the_name() {
        assert_eq!(pin_mentions("trunk a b c 1.2.3", "trunk"), ["1.2.3"]);
    }

    #[test]
    fn a_version_has_three_parts_of_digits_only() {
        assert!(is_version("0.22.1"));
        assert!(!is_version("1.2.x"));
        assert!(!is_version("1..3"));
        assert!(!is_version("1.2.3.4"));
        assert_eq!(pin_mentions("trunk 1.2.x", "trunk"), Vec::<String>::new());
    }

    /// The pins are only here: a doc or CI that names another version of a
    /// tool fails (DOC-1).  Plans and review logs are history, so they are
    /// not read.
    #[test]
    fn the_docs_and_ci_name_only_the_pins() {
        let root = crate::workspace_root();
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        let tools = tools(&lock).unwrap();
        let mut files = vec![
            root.join("README.md"),
            root.join("CLAUDE.md"),
            root.join(".github/workflows/ci.yml"),
        ];
        for entry in std::fs::read_dir(root.join("docs")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "md") {
                files.push(path);
            }
        }
        files.push(root.join("docs/references/local-browser-testing.md"));
        let mut wrong = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            for tool in &tools {
                for version in pin_mentions(&text, tool.name) {
                    if version != tool.pin {
                        wrong.push(format!("{}: {} {version}", file.display(), tool.name));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "not the pins of xtask setup: {wrong:?}");
        let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
        assert_eq!(
            ci.matches(&format!("  TRUNK_VERSION: \"{TRUNK}\"\n"))
                .count(),
            1,
            "ci.yml's TRUNK_VERSION is not {TRUNK}; change it with TRUNK_SHA256"
        );
    }
}
