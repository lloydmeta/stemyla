//! `xtask setup [--check] [tool...]`: installs each dev tool at its pin.
//!
//! The pins are here, and nowhere else: `README.md` and CI point to this
//! command.  wasm-bindgen-cli takes the `wasm-bindgen` of `Cargo.lock`, and
//! the nightly is `check::DOC_NIGHTLY`.  `--check` installs nothing, and
//! fails when a tool is missing or at another version.  CI keeps one copy
//! of the Trunk pin, beside the SHA-256 of its release binary, and a test
//! keeps it equal to [`TRUNK`].

use crate::check::DOC_NIGHTLY;
use crate::web_test::locked_version;
use anyhow::{Context, Result, bail};
use std::process::Command;

/// The Trunk that builds the app.
pub const TRUNK: &str = "0.21.14";

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
    let out = Command::new(&command[0])
        .args(&command[1..])
        .current_dir(crate::workspace_root())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn have(tool: &Tool) -> Option<String> {
    installed(&tool.probe, &tool.pin, run_probe(&tool.probe).as_deref())
}

pub fn run(args: &[String]) -> Result<()> {
    let check_only = args.iter().any(|arg| arg == "--check");
    if let Some(option) = args
        .iter()
        .find(|arg| arg.starts_with("--") && arg.as_str() != "--check")
    {
        bail!("unknown option {option}; usage: xtask setup [--check] [tool...]");
    }
    let names: Vec<String> = args
        .iter()
        .filter(|arg| !arg.starts_with("--"))
        .cloned()
        .collect();
    let root = crate::workspace_root();
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).context("reading Cargo.lock")?;
    let tools = chosen(tools(&lock)?, &names)?;
    let mut missing = Vec::new();
    for tool in &tools {
        let action = action(tool, have(tool));
        println!("{}", line(tool, &action));
        if action == Action::Keep {
            continue;
        }
        if check_only {
            missing.push(tool.name);
            continue;
        }
        let status = Command::new(&tool.install[0])
            .args(&tool.install[1..])
            .current_dir(&root)
            .status()
            .with_context(|| format!("running {}", tool.install.join(" ")))?;
        let after = action_after(tool, status.success(), have(tool));
        if let Some(problem) = after {
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
