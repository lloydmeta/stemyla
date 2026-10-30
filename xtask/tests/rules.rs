//! The structural rules hold on the real tree (`xtask::rules`).

fn rust_files() -> Vec<(String, String)> {
    let files = xtask::rules::rust_files(&xtask::workspace_root()).expect("the tree can be read");
    for known in [
        "roformer/tests/reference.rs",
        "roformer-gpu/tests/kernels.rs",
    ] {
        assert!(
            files.iter().any(|(file, _)| file == known),
            "the walk did not reach {known}"
        );
    }
    files
}

#[test]
fn every_skip_can_be_required() {
    let files = rust_files();
    let problems = xtask::rules::skips_use_the_helper(&files);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} skip problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn synth_brings_no_crate_into_the_web_app() {
    let root = xtask::workspace_root();
    let trees: Vec<(&str, String)> = xtask::rules::WEB_APP
        .into_iter()
        .map(|package| {
            let tree = xtask::rules::web_app_tree(&root, package).expect("cargo tree runs");
            (package, tree)
        })
        .collect();
    let pairs: Vec<(&str, &str)> = trees
        .iter()
        .map(|(package, tree)| (*package, tree.as_str()))
        .collect();
    let problems = xtask::rules::synth_brings_no_crate(&pairs);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} problem(s) with synth in the web app:\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn the_skip_allowlist_is_current() {
    let problems = xtask::rules::allowlisted_skips_exist(&rust_files());
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(problems.is_empty(), "{}", report.join("\n"));
}

/// One `cargo metadata` run for the layer tests and the pin test.
fn metadata() -> &'static str {
    static METADATA: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    METADATA.get_or_init(|| {
        xtask::rules::workspace_metadata(&xtask::workspace_root()).expect("cargo metadata runs")
    })
}

fn workspace() -> &'static xtask::rules::Workspace {
    static WORKSPACE: std::sync::OnceLock<xtask::rules::Workspace> = std::sync::OnceLock::new();
    WORKSPACE.get_or_init(|| {
        xtask::rules::workspace(metadata()).expect("cargo metadata's JSON gives the workspace")
    })
}

#[test]
fn young_crates_are_pinned() {
    let problems =
        xtask::rules::young_crates_are_pinned_in(metadata(), &xtask::rules::SETTLED_CRATES)
            .expect("cargo metadata's JSON can be read");
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} DEPS-1 problem(s) with pins:\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn third_party_lists_what_ships() {
    let root = xtask::workspace_root();
    let doc =
        std::fs::read_to_string(root.join("THIRD-PARTY.md")).expect("THIRD-PARTY.md can be read");
    let [normal, normal_and_dev] = xtask::rules::THIRD_PARTY_EDGES
        .map(|edges| xtask::rules::third_party_tree(&root, edges).expect("cargo tree runs"));
    let members =
        xtask::rules::member_folders(metadata()).expect("cargo metadata's JSON can be read");
    let problems = xtask::rules::third_party_matches(&doc, &normal, &normal_and_dev, &members);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} DEPS-1 problem(s) with THIRD-PARTY.md:\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn crates_depend_only_downwards() {
    let architecture =
        std::fs::read_to_string(xtask::workspace_root().join("docs/ARCHITECTURE.md"))
            .expect("docs/ARCHITECTURE.md can be read");
    let problems = xtask::rules::crates_follow_the_layers(workspace(), &architecture);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} problem(s) with the crate layers:\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn the_page_and_the_worker_meet_only_through_stem_protocol() {
    let problems = xtask::rules::shells_meet_only_through_the_protocol(&workspace().crates);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} problem(s) with the page side and the worker side:\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn dsp_is_beginner_style() {
    let files = xtask::rules::dsp_files(&xtask::workspace_root()).expect("dsp/ can be read");
    for known in ["dsp/Cargo.toml", "dsp/src/lib.rs", "dsp/src/mix.rs"] {
        assert!(
            files.iter().any(|(file, _)| file == known),
            "the walk did not reach {known}"
        );
    }
    let problems = xtask::rules::dsp_keeps_beginner_style(&files);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} DSP-1 problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}

/// One `git ls-files` run for both index tests.
fn tracked_files() -> &'static [xtask::rules::TrackedFile] {
    static TRACKED: std::sync::OnceLock<Vec<xtask::rules::TrackedFile>> =
        std::sync::OnceLock::new();
    TRACKED.get_or_init(|| {
        let tracked =
            xtask::rules::tracked_files(&xtask::workspace_root()).expect("git ls-files runs");
        for known in [
            "Cargo.toml",
            "tools/roformer_export.py",
            "app/tests/snapshots/six_lanes.png",
        ] {
            assert!(
                tracked.iter().any(|file| file.path == known),
                "git ls-files did not list {known}"
            );
        }
        tracked
    })
}

#[test]
fn no_python_in_the_build() {
    let tracked = tracked_files();
    let texts = xtask::rules::tracked_texts(&xtask::workspace_root(), tracked)
        .expect("the tracked files can be read");
    for known in [".github/workflows/ci.yml", "README.md"] {
        assert!(
            texts.iter().any(|(file, _)| file == known),
            "the texts do not hold {known}"
        );
    }
    let problems = xtask::rules::python_in_the_build(tracked, &texts);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} BUILD-1 problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn no_build_file_updates_snapshots() {
    let texts = xtask::rules::tracked_texts(&xtask::workspace_root(), tracked_files())
        .expect("the tracked files can be read");
    assert!(
        texts
            .iter()
            .any(|(file, _)| file == ".github/workflows/ci.yml"),
        "the texts do not hold .github/workflows/ci.yml"
    );
    let problems = xtask::rules::snapshot_updates(&texts);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} VERIFY-1 problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn no_audio_or_weights_are_tracked() {
    let problems = xtask::rules::assets_in_the_index(tracked_files());
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} ASSET-1 problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}

/// PROTO-1: the check reads the scenarios in `xtask/src/e2e/` too, each
/// under its own path.
#[test]
fn the_e2e_check_reads_every_rust_file_of_the_scenarios() {
    let rust = xtask::rules::e2e_rust_sources(&xtask::workspace_root()).expect("readable");
    let text_of = |name: &str| {
        rust.iter()
            .find(|(file, _)| file == name)
            .map(|(_, text)| text.as_str())
            .unwrap_or_default()
    };
    assert!(
        text_of("xtask/src/e2e/decode.rs").contains("pub(super) fn run(o: &Opts)"),
        "xtask/src/e2e/decode.rs is read"
    );
    assert!(
        text_of("xtask/src/e2e.rs").contains("fn memory_budget(o: &Opts)"),
        "xtask/src/e2e.rs is read"
    );
}

/// Finding 2-16 of review round 2: a Rust file in a folder under
/// `xtask/src/e2e/` is read too.
#[test]
fn the_e2e_check_reads_the_folders_under_the_scenarios() {
    let root = std::env::temp_dir().join(format!("xtask-e2e-sources-{}", std::process::id()));
    let nested = root.join("xtask/src/e2e/deep");
    std::fs::create_dir_all(&nested).expect("made");
    std::fs::write(root.join("xtask/src/e2e.rs"), "fn a() {}").expect("written");
    std::fs::write(root.join("xtask/src/e2e/b.rs"), "fn b() {}").expect("written");
    std::fs::write(nested.join("c.rs"), "fn c() {}").expect("written");
    std::fs::write(nested.join("notes.txt"), "not Rust").expect("written");
    let read = xtask::rules::e2e_rust_sources(&root);
    std::fs::remove_dir_all(&root).expect("removed");
    let names: Vec<String> = read
        .expect("readable")
        .into_iter()
        .map(|(file, _)| file)
        .collect();
    assert_eq!(
        names,
        [
            "xtask/src/e2e.rs",
            "xtask/src/e2e/b.rs",
            "xtask/src/e2e/deep/c.rs"
        ]
    );
}

#[test]
fn e2e_messages_use_protocol_types() {
    let root = xtask::workspace_root();
    let read = |file: &str| {
        std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("reading {file}: {e}"))
    };
    let scripts = xtask::rules::e2e_scripts(&root).expect("xtask/e2e can be read");
    assert!(
        scripts
            .iter()
            .any(|(file, _)| file == "xtask/e2e/worker.js"),
        "the scripts do not include xtask/e2e/worker.js"
    );
    let problems = xtask::rules::e2e_types_match_the_protocol(
        // The stem worker's kinds and the decode worker's, each file's own
        // `mod kind`.
        &format!(
            "{}\n{}",
            read("protocol/src/lib.rs"),
            read("protocol/src/decode.rs")
        ),
        &scripts,
        &xtask::rules::e2e_rust_sources(&root).expect("the scenarios can be read"),
        &xtask::rules::SCRIPT_ONLY_TYPES,
    );
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} PROTO-1 problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}

#[test]
fn claude_settings_hold_the_deny_rules() {
    let file = xtask::workspace_root().join(".claude/settings.json");
    let settings = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
    let problems = xtask::rules::claude_settings_problems(&settings);
    let report: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        problems.is_empty(),
        "{} GIT-1 problem(s):\n{}",
        problems.len(),
        report.join("\n")
    );
}

/// The one PostToolUse hook on `Edit|Write` in `.claude/settings.json`.  It
/// must be a plain shell-form command, which is what the test runs.
fn rustfmt_hook_command(settings: &serde_json::Value) -> String {
    let hooks: Vec<&serde_json::Value> = settings["hooks"]["PostToolUse"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["matcher"] == "Edit|Write")
        .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
        .collect();
    assert!(
        hooks.len() == 1,
        "GIT-2: .claude/settings.json has {} PostToolUse hooks on `Edit|Write`, not 1.  Fix: keep one hook that runs rustfmt on the edited .rs file (harness phase 2, M15).",
        hooks.len()
    );
    let shape = "GIT-2: the rustfmt hook in .claude/settings.json is not `{\"type\": \"command\", \"command\": ...}`.  Fix: keep only `type` and `command`; the test runs the command in shell form.";
    let hook = hooks[0].as_object().unwrap_or_else(|| panic!("{shape}"));
    if let Some(key) = hook
        .keys()
        .find(|key| !["type", "command"].contains(&key.as_str()))
    {
        panic!(
            "GIT-2: the rustfmt hook in .claude/settings.json has the key `{key}`.  Fix: keep only `type` and `command`; the test runs the command in shell form."
        );
    }
    assert!(
        hook.get("type").is_some_and(|kind| kind == "command"),
        "{shape}"
    );
    hook.get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{shape}"))
        .to_string()
}

/// Runs the hook as Claude Code does: `sh -c`, the tool call on stdin, and
/// `CLAUDE_PROJECT_DIR` set to the project root.  `input` is the JSON of the
/// tool call, or other text for a test of bad input.  `path`, when given,
/// replaces `PATH`.
fn run_hook(
    command: &str,
    project: &std::path::Path,
    input: &str,
    path: Option<&std::path::Path>,
) -> std::process::Output {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut sh = Command::new("/bin/sh");
    if let Some(path) = path {
        sh.env("PATH", path);
    }
    let mut child = sh
        .arg("-c")
        .arg(command)
        .env("CLAUDE_PROJECT_DIR", project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sh starts");
    // A hook that stops before it reads stdin, such as one without jq,
    // closes the pipe; its exit status, not the write, decides the test.
    let written = child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input.as_bytes());
    if let Err(e) = written
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        panic!("writing the tool call to the hook: {e}");
    }
    child.wait_with_output().expect("the hook finishes")
}

#[test]
fn the_rustfmt_hook_formats_rust_files() {
    const UNFORMATTED: &str = "use x::{a10,a9};\nfn f(){let x=1;}\n";
    const FORMATTED: &str = "use x::{a9, a10};\nfn f() {\n    let x = 1;\n}\n";
    let file = xtask::workspace_root().join(".claude/settings.json");
    let settings = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
    let settings: serde_json::Value = serde_json::from_str(&settings)
        .unwrap_or_else(|e| panic!("{} is not JSON: {e}", file.display()));
    let command = rustfmt_hook_command(&settings);

    let folder = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("rustfmt hook");
    let project = folder.join("project");
    let sibling = folder.join("project2");
    let no_tools = folder.join("no tools");
    for made in [&project, &sibling, &no_tools] {
        std::fs::create_dir_all(made).expect("the test folder can be made");
    }
    // rustfmt reads the first rustfmt.toml above the file, so an empty one
    // here keeps out a config above the target folder.
    std::fs::write(folder.join("rustfmt.toml"), "").expect("the test config can be written");
    let left = |file: &str, text: &str, output: &std::process::Output| {
        let code = output
            .status
            .code()
            .map_or_else(|| "a signal".to_string(), |code| code.to_string());
        format!(
            "GIT-2: the rustfmt hook in .claude/settings.json left {file} as {text:?} (exit {code}: {}).  Fix: the hook needs jq and rustfmt on PATH (`rustup component add rustfmt`); it formats .rs files under CLAUDE_PROJECT_DIR only, with the workspace edition.",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    };

    let cases = [
        (project.join("lib.rs"), FORMATTED),
        (project.join("notes.md"), UNFORMATTED),
        (folder.join("outside.rs"), UNFORMATTED),
        (sibling.join("lib.rs"), UNFORMATTED),
    ];
    for (path, expected) in &cases {
        std::fs::write(path, UNFORMATTED).expect("the test file can be written");
        let input =
            serde_json::json!({ "tool_name": "Write", "tool_input": { "file_path": path } });
        let output = run_hook(&command, &project, &input.to_string(), None);
        let text = std::fs::read_to_string(path).expect("the test file can be read");
        assert!(
            output.status.success() && text == *expected,
            "{}",
            left(&path.display().to_string(), &text, &output)
        );
    }

    let input = serde_json::json!({ "tool_name": "Write", "tool_input": {} });
    let output = run_hook(&command, &project, &input.to_string(), None);
    assert!(output.status.success(), "{}", left("no file", "", &output));

    let lib = project.join("lib.rs");
    let input = serde_json::json!({ "tool_name": "Write", "tool_input": { "file_path": lib } });
    let output = run_hook(&command, &project, &input.to_string(), Some(&no_tools));
    assert!(
        !output.status.success(),
        "GIT-2: the rustfmt hook in .claude/settings.json exits 0 without jq on PATH, so a missing jq is silent.  Fix: keep `|| exit 1` after the jq command substitution."
    );
}

/// The guard of the review agents' Bash calls, as their hook names it.
const GUARD: &str = ".claude/hooks/read-only-bash.sh";
const GUARD_COMMAND: &str = r#""$CLAUDE_PROJECT_DIR"/.claude/hooks/read-only-bash.sh"#;

/// The labels of the guard's refusals.
const BACKTICK: &str = "backtick";
const UNCLOSED: &str = "unclosed quote";
const VARIABLE: &str = "variable";
const COMMAND: &str = "command";
const CARGO_OPTION: &str = "cargo option";
const CARGO_SUB: &str = "cargo subcommand";
const GIT_OPTION: &str = "git option";
const GIT_OUTPUT: &str = "git output";
const GIT_SUB: &str = "git subcommand";
const UNREADABLE: &str = "cannot read the command";

/// Commands that the guard refuses, each with the label of its refusal.
/// Each rule of the guard has a case that gives another result without it.
const GUARD_REFUSES: &[(&str, &str)] = &[
    // A backtick outside single quotes.
    (BACKTICK, "echo `git commit`"),
    (BACKTICK, "echo \"`git commit`\""),
    (BACKTICK, r"echo \`git commit\`"),
    // The split into parts.
    (GIT_SUB, "echo $(git commit)"),
    (GIT_SUB, "cargo test; git stash"),
    (GIT_SUB, "cargo test && git stash"),
    (GIT_SUB, "cargo test | git stash"),
    (GIT_SUB, "cargo test\ngit stash"),
    // A backslash outside quotes, and inside double quotes.
    (GIT_SUB, r#"echo \"; git commit -m x; echo \""#),
    (GIT_SUB, r#"echo "\"" ; git commit -m "\"""#),
    // A backslash before a new line joins the lines.
    (CARGO_SUB, "cargo pub\\\nlish"),
    // A comment ends at the new line, joins no line, and starts only at
    // the start of a word.
    (GIT_SUB, "git log # x\ngit commit -m x"),
    (GIT_SUB, "git log # x\\\ngit commit -m x"),
    (GIT_SUB, "echo a#; git commit -m x"),
    (GIT_SUB, "echo ''#; git commit -m x"),
    (GIT_SUB, r"echo \a#; git commit -m x"),
    // A quote that is not closed.
    (UNCLOSED, "grep -n \"x docs"),
    // A tab ends a word.
    (GIT_SUB, "git\tcommit -m x"),
    // Quote removal and backslash removal.
    (GIT_SUB, "'git' commit -m x"),
    (GIT_SUB, "g\"it\" commit -m x"),
    (CARGO_SUB, "cargo \"publish\""),
    (GIT_SUB, r"\git commit -m x"),
    (CARGO_SUB, r"cargo pub\lish"),
    // Prefixes.
    (GIT_SUB, "rtk git commit -m x"),
    (COMMAND, "rtk proxy gh pr view 1"),
    (GIT_SUB, "env git commit -m x"),
    (GIT_SUB, "/usr/bin/env git commit -m x"),
    (GIT_SUB, "command git commit -m x"),
    (GIT_SUB, "command -p git commit -m x"),
    (GIT_SUB, "exec git commit -m x"),
    (GIT_SUB, "time git commit -m x"),
    (GIT_SUB, "nohup git commit -m x"),
    (COMMAND, "timeout 60 gh pr view 1"),
    (GIT_SUB, "timeout -v 60 git commit -m x"),
    (GIT_SUB, "timeout --foreground 600 git commit -m x"),
    (GIT_SUB, "timeout -s KILL 60 git commit -m x"),
    (GIT_SUB, "timeout -k 5 60 git commit -m x"),
    (GIT_SUB, "timeout --signal KILL 60 git commit -m x"),
    (GIT_SUB, "timeout --kill-after 5 60 git commit -m x"),
    (GIT_SUB, "rtk -v git commit -m x"),
    (GIT_SUB, "FOO=1 git commit -m x"),
    // Shell words.
    (GIT_SUB, "! git commit -m x"),
    (GIT_SUB, "{ git commit -m x; }"),
    (GIT_SUB, "if git commit -m x; then :; fi"),
    (GIT_SUB, "if true; then git commit -m x; fi"),
    (
        GIT_SUB,
        "if false; then :; elif git commit -m x; then :; fi",
    ),
    (GIT_SUB, "if false; then :; else git commit -m x; fi"),
    (GIT_SUB, "for f in a; do git add \"$f\"; done"),
    (GIT_SUB, "while git commit -m x; do :; done"),
    (GIT_SUB, "until git commit -m x; do :; done"),
    // Variables.
    (VARIABLE, "GIT_PAGER=cat git log"),
    (VARIABLE, "GIT_DIR=/tmp/x git log"),
    (VARIABLE, "UPDATE_SNAPSHOTS=1 cargo test"),
    (VARIABLE, "GIT_DIR+=x git log"),
    (VARIABLE, "export UPDATE_SNAPSHOTS=1; cargo test"),
    (VARIABLE, "export GIT_EXTERNAL_DIFF=x; git diff"),
    (VARIABLE, "declare -x GIT_DIR=x"),
    (VARIABLE, "typeset GIT_DIR=x"),
    (VARIABLE, "readonly GIT_DIR=x"),
    (VARIABLE, "local GIT_DIR=x"),
    // First words.
    (COMMAND, "gh pr view 1"),
    (COMMAND, "curl x"),
    (COMMAND, "wget x"),
    (COMMAND, "bash -c x"),
    (COMMAND, "sh -c x"),
    (COMMAND, "zsh -c x"),
    (COMMAND, "eval x"),
    (COMMAND, "xargs x"),
    (COMMAND, "target/review/target/debug/xtask publish"),
    (COMMAND, "target/debug/xtask review start HEAD 1"),
    (COMMAND, "target/debug/xtask review mutants 1"),
    (COMMAND, "cargo run -p xtask --release -- publish"),
    (COMMAND, "cargo r -p xtask -- publish"),
    (COMMAND, "cargo run --package=xtask -- publish"),
    (COMMAND, "cargo -q run -pxtask -- publish"),
    (
        COMMAND,
        "cargo run --manifest-path xtask/Cargo.toml -- publish",
    ),
    (
        COMMAND,
        "cargo run --manifest-path ./xtask/Cargo.toml -- publish",
    ),
    (
        COMMAND,
        "cargo run --manifest-path=xtask/Cargo.toml -- publish",
    ),
    (
        COMMAND,
        "cargo run --manifest-path=./xtask/Cargo.toml -- review start HEAD 1",
    ),
    (COMMAND, "cargo run -q -p xtask -- review start HEAD 1"),
    (GIT_SUB, "FOO=a/b git commit -m x"),
    (GIT_SUB, "/usr/bin/git commit -m x"),
    // Letter case: the file system of macOS ignores it.
    (GIT_SUB, "GIT commit -m x"),
    (COMMAND, "Gh pr view 1"),
    (GIT_SUB, "ENV git commit -m x"),
    // Cargo.
    (CARGO_SUB, "cargo publish"),
    (CARGO_SUB, "cargo login"),
    (CARGO_SUB, "cargo owner"),
    (CARGO_SUB, "cargo yank"),
    (CARGO_SUB, "cargo install x"),
    (CARGO_SUB, "cargo uninstall x"),
    (CARGO_SUB, "cargo add x"),
    (CARGO_SUB, "cargo remove x"),
    (CARGO_SUB, "cargo rm x"),
    (CARGO_SUB, "cargo logout"),
    (CARGO_SUB, "cargo update"),
    (CARGO_SUB, "cargo clean"),
    (CARGO_SUB, "cargo search x"),
    (CARGO_SUB, "cargo fix"),
    (CARGO_SUB, "cargo fmt"),
    (CARGO_SUB, "cargo clippy --fix"),
    (CARGO_SUB, "cargo -q publish"),
    (CARGO_SUB, "cargo --color never publish"),
    (CARGO_SUB, "cargo -Z x publish"),
    (CARGO_SUB, "cargo -C x publish"),
    (CARGO_SUB, "cargo +nightly publish"),
    (CARGO_SUB, "cargo generate-lockfile"),
    (CARGO_SUB, "cargo vendor"),
    (CARGO_OPTION, "cargo --config x test"),
    (CARGO_OPTION, "cargo test --config x"),
    (CARGO_OPTION, "cargo --config=x test"),
    // Git.
    (GIT_OPTION, "git -ccore.pager=x log"),
    (GIT_OPTION, "git -c core.pager=x log"),
    (GIT_OPTION, "git --config-env=core.pager=X log"),
    (GIT_OUTPUT, "git diff --output=x"),
    (GIT_OUTPUT, "git diff --output x"),
    (GIT_SUB, "git commit -m x"),
    (GIT_SUB, "git --no-pager commit -m x"),
    (GIT_SUB, "git -C target/review/x push"),
    (GIT_SUB, "git --git-dir x commit"),
    (GIT_SUB, "git --work-tree x commit"),
    (GIT_SUB, "git --namespace x commit"),
    (GIT_SUB, "git add x"),
];

/// Commands that the guard passes.
const GUARD_PASSES: &[&str] = &[
    // Git.
    "git diff HEAD~1",
    "rtk git diff",
    "rtk proxy git log",
    "git -C target/review/1-x diff",
    "git --git-dir .git log",
    "git --no-pager log",
    "git --work-tree . log",
    "git --namespace x log",
    "git log -c",
    "git --version",
    "git show HEAD",
    "git status",
    "git blame README.md",
    "git grep x",
    "git ls-files",
    "git ls-tree HEAD",
    "git rev-parse HEAD",
    "git rev-list HEAD",
    "git merge-base HEAD HEAD",
    "git cat-file -t HEAD",
    "git show-ref",
    "git for-each-ref",
    "git describe --always",
    "git lo\\\ng",
    "git worktree add --detach target/review/1-x HEAD",
    "git worktree remove --force target/review/1-x",
    // Quotes, backslashes and comments.
    "grep -rn \"enough \" docs",
    "grep -rn -e curl -e wget .github",
    "grep -rnE \"a|curl b\" xtask",
    "grep -rnE 'a|curl b' xtask",
    "grep -rn '`x`' docs",
    r"grep -rn 'a\' docs",
    r#"grep -rn "a\"b" docs"#,
    "grep -n a#b docs",
    "git log # it's here",
    // Parts, cargo and the rest.
    "(git log)",
    "cargo test -p dsp && git diff",
    "cargo fmt --all --check",
    "cargo test -p dsp --target-dir /tmp/t 2>&1",
    "cargo --color never test -p dsp",
    "target/debug/xtask docs-check",
    "cargo run -q -p xtask -- docs-check",
    "cargo run -p app -- publish",
    "cargo test -p xtask review",
    "for f in a b; do grep -n x \"$f\"; done",
    "export FOO=1",
    "",
];

/// The tool call of a Bash command, as Claude Code sends it to a hook.
fn bash_call(command: &str) -> String {
    serde_json::json!({ "tool_name": "Bash", "tool_input": { "command": command } }).to_string()
}

/// What is wrong with the guard's result for a case, if anything: a refusal
/// exits 2 with "read-only guard: <label>:", and a pass exits 0 and writes
/// nothing.
fn guard_mismatch(
    case: &str,
    expected: Option<&str>,
    output: &std::process::Output,
) -> Option<String> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code();
    let (matches, wanted) = match expected {
        Some(label) => {
            let prefix = if label == UNREADABLE {
                format!("read-only guard: {label}.")
            } else {
                format!("read-only guard: {label}:")
            };
            (
                code == Some(2) && stderr.starts_with(&prefix),
                format!("a refusal with the label `{label}`"),
            )
        }
        None => (code == Some(0) && stderr.is_empty(), "a pass".to_string()),
    };
    let code = code.map_or_else(|| "a signal".to_string(), |code| code.to_string());
    (!matches).then(|| {
        format!(
            "{case:?}: expected {wanted}, got exit {code} with {:?}",
            stderr.trim()
        )
    })
}

#[test]
fn the_bash_guard_refuses_writes() {
    use std::os::unix::fs::PermissionsExt;
    let root = xtask::workspace_root();
    let staged = std::process::Command::new("git")
        .args(["ls-files", "-s", GUARD])
        .current_dir(&root)
        .output()
        .expect("git ls-files runs");
    let staged = String::from_utf8_lossy(&staged.stdout);
    let text = std::fs::read_to_string(root.join(GUARD))
        .unwrap_or_else(|e| panic!("reading {GUARD}: {e}"));
    assert!(
        text.lines().any(|line| line == "export LC_ALL=C"),
        "{GUARD} has no line `export LC_ALL=C`.  Fix: put it back; without it, the parse of a long command takes several times as long."
    );
    assert!(
        staged.starts_with("100755 "),
        "{GUARD} is tracked as {:?}, not as mode 100755.  Fix: run `git add --chmod=+x {GUARD}`; the agents' hook runs it as a program.",
        staged.trim()
    );
    let mode = std::fs::metadata(root.join(GUARD))
        .unwrap_or_else(|e| panic!("reading {GUARD}: {e}"))
        .permissions()
        .mode();
    assert!(
        mode & 0o111 != 0,
        "{GUARD} has no execute bit on disk (mode {mode:o}).  Fix: run `chmod +x {GUARD}`."
    );

    let no_tools = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("guard without jq");
    std::fs::create_dir_all(&no_tools).expect("the test folder can be made");
    let run =
        |input: &str, path: Option<&std::path::Path>| run_hook(GUARD_COMMAND, &root, input, path);

    let refused = GUARD_REFUSES
        .iter()
        .map(|(label, command)| (command.to_string(), bash_call(command), None, Some(*label)));
    let passed = GUARD_PASSES
        .iter()
        .map(|command| (command.to_string(), bash_call(command), None, None));
    let unreadable = [
        ("no jq", bash_call("git log"), Some(no_tools.as_path())),
        ("input that is not JSON", "not json".to_string(), None),
        (
            "no command",
            r#"{"tool_name": "Bash", "tool_input": {}}"#.to_string(),
            None,
        ),
        (
            "a command that is a number",
            r#"{"tool_name": "Bash", "tool_input": {"command": 5}}"#.to_string(),
            None,
        ),
    ]
    .into_iter()
    .map(|(case, input, path)| (case.to_string(), input, path, Some(UNREADABLE)));
    let cases: Vec<_> = refused.chain(passed).chain(unreadable).collect();
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(case, input, path, expected)| {
            guard_mismatch(case, *expected, &run(input, *path))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} guard case(s) failed:\n{}\nFix: {GUARD} must refuse each case with its label, and pass the others (harness phase 3, M2).",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );

    let shape = "grep -rn '' docs";
    let long = shape.replace("''", &format!("'{}'", "x".repeat(10_000 - shape.len())));
    assert_eq!(long.len(), 10_000);
    let started = std::time::Instant::now();
    let output = run(&bash_call(&long), None);
    let took = started.elapsed();
    let mismatch = guard_mismatch("a search of 10,000 characters", None, &output);
    assert!(mismatch.is_none(), "{}", mismatch.unwrap_or_default());
    assert!(
        took < std::time::Duration::from_secs(5),
        "{GUARD} took {took:?} on a command of 10,000 characters, not less than 5 s.  Fix: keep the parse to one pass, with LC_ALL=C."
    );
}

/// The advice of a refusal by the guard's parse, and of each other refusal.
const PARSE_ADVICE: &str = "This is a limit of the guard's parse.  Write the command again without a backtick or an unmatched quote, for example with `sed`.";
const READ_ONLY_ADVICE: &str = "This agent is read only.  Do not try another form of the command.";

#[test]
fn the_bash_guard_explains_each_refusal() {
    let root = xtask::workspace_root();
    let cases = [
        (
            bash_call("echo `git commit`"),
            format!("read-only guard: backtick: echo `git commit`.  {PARSE_ADVICE}\n"),
        ),
        (
            bash_call("grep -n \"x docs\ngit log"),
            format!("read-only guard: unclosed quote: grep -n \"x docs git log.  {PARSE_ADVICE}\n"),
        ),
        (
            bash_call("cargo test; git stash"),
            format!("read-only guard: git subcommand: git stash.  {READ_ONLY_ADVICE}\n"),
        ),
        (
            bash_call("git log\n  git commit -m \"a\nb\" \t"),
            format!(
                "read-only guard: git subcommand: git commit -m \"a b\".  {READ_ONLY_ADVICE}\n"
            ),
        ),
        (
            bash_call("cargo run -p xtask --release -- publish"),
            format!(
                "read-only guard: command: cargo run -p xtask --release -- publish.  {READ_ONLY_ADVICE}\n"
            ),
        ),
        (
            bash_call("cargo pub\\\nlish"),
            format!("read-only guard: cargo subcommand: cargo pub\\ lish.  {READ_ONLY_ADVICE}\n"),
        ),
        (
            "not json".to_string(),
            format!("read-only guard: cannot read the command.  {READ_ONLY_ADVICE}\n"),
        ),
    ];
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(input, expected)| {
            let output = run_hook(GUARD_COMMAND, &root, input, None);
            let stderr = String::from_utf8_lossy(&output.stderr);
            (output.status.code() != Some(2) || stderr != *expected).then(|| {
                format!(
                    "{input}: expected exit 2 with {expected:?}, got {:?} with {stderr:?}",
                    output.status.code()
                )
            })
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} refusal line(s) of {GUARD} are wrong:\n{}\nFix: write one line, \"read-only guard: <label>: <text>.  <advice>\", with the refused part as <text>, each new line as a space (harness phase 3, M2).",
        failures.len(),
        failures.join("\n")
    );

    // An error of the guard's own exits 2 with its own line, so that the
    // call is refused even without `onFailure: block`.
    let guard = std::fs::read_to_string(root.join(GUARD))
        .unwrap_or_else(|e| panic!("reading {GUARD}: {e}"));
    let broken = guard
        .strip_suffix("\nexit 0\n")
        .map(|start| format!("{start}\n: \"${{an_unset_name_of_the_test:?}}\"\nexit 0\n"))
        .unwrap_or_else(|| {
            panic!("{GUARD} does not end with `exit 0`, where the test puts an error")
        });
    let folder = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("broken-guard");
    std::fs::create_dir_all(&folder).expect("the test folder can be made");
    let script = folder.join("read-only-bash.sh");
    std::fs::write(&script, broken).expect("the broken guard can be written");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("the broken guard can be made executable");
    }
    let output = run_hook(
        &format!("'{}'", script.display()),
        &root,
        &bash_call("git log"),
        None,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.code() == Some(2)
            && stderr.lines().any(|line| {
                line == format!(
                    "read-only guard: the guard failed with status 1.  {READ_ONLY_ADVICE}"
                )
            }),
        "{GUARD} with an error of its own gave exit {:?} with {stderr:?}, not exit 2 with \"read-only guard: the guard failed with status 1.\".  Fix: keep the EXIT trap.",
        output.status.code()
    );
}

/// The route of `xtask review` names only reviewers that exist, and each
/// path glob matches a file, so a renamed folder cannot drop a reviewer.
#[test]
fn every_reviewer_has_an_agent_file() {
    use xtask::review::{MAY_MATCH_NOTHING, REVIEWERS};
    let paths: Vec<&str> = tracked_files()
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    let missing_files = REVIEWERS
        .iter()
        .map(|reviewer| format!(".claude/agents/{}.md", reviewer.name))
        .filter(|file| !paths.contains(&file.as_str()))
        .map(|file| {
            format!("{file} is missing.  Fix: add the agent (M2), or remove its row from REVIEWERS in xtask/src/review.rs.")
        });
    let missing_rows = paths
        .iter()
        .filter_map(|path| path.strip_prefix(".claude/agents/"))
        .filter_map(|name| name.strip_suffix("reviewer.md").map(|_| name))
        .filter(|name| {
            !REVIEWERS
                .iter()
                .any(|reviewer| format!("{}.md", reviewer.name) == *name)
        })
        .map(|name| {
            format!(".claude/agents/{name} has no row in REVIEWERS.  Fix: add a row in xtask/src/review.rs, or rename the agent.")
        });
    let unmatched_globs = REVIEWERS
        .iter()
        .flat_map(|reviewer| reviewer.include.iter().map(move |glob| (reviewer.name, *glob)))
        .filter(|(_, glob)| !MAY_MATCH_NOTHING.iter().any(|(free, _)| free == glob))
        .filter(|(_, glob)| {
            !paths
                .iter()
                .any(|path| xtask::docs::glob_matches(glob, path))
        })
        .map(|(name, glob)| {
            format!("`{glob}` of {name} matches no tracked file.  Fix: correct the glob, or add it to MAY_MATCH_NOTHING with a reason.")
        });
    let problems: Vec<String> = missing_files
        .chain(missing_rows)
        .chain(unmatched_globs)
        .collect();
    assert!(
        problems.is_empty(),
        "{} review route problem(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// The `git hash-object --no-filters` of `.claude/agents/baseline.md`.
const BASELINE_HASH: &str = "abfa256fe04826fc47165e101c18cbbbf03edac7";

/// Each entry below `folder`, into each subfolder: its path below `folder`,
/// and its text if it is a file (`None` for a folder).  A symbolic link
/// counts as what it points to.
fn walk(folder: &std::path::Path, below: &str) -> Vec<(String, Option<String>)> {
    let here = folder.join(below);
    let mut names: Vec<String> = std::fs::read_dir(&here)
        .unwrap_or_else(|e| panic!("reading {}: {e}", here.display()))
        .map(|entry| {
            entry
                .expect("the folder can be listed")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .flat_map(|name| {
            let path = if below.is_empty() {
                name
            } else {
                format!("{below}/{name}")
            };
            let full = folder.join(&path);
            if full.is_dir() {
                [vec![(path.clone(), None)], walk(folder, &path)].concat()
            } else {
                let bytes = std::fs::read(&full)
                    .unwrap_or_else(|e| panic!("reading {}: {e}", full.display()));
                vec![(path, Some(String::from_utf8_lossy(&bytes).into_owned()))]
            }
        })
        .collect()
}

/// Each agent is read only by its tools and by its hook, and the baseline
/// review stays as it was frozen.
#[test]
fn the_agents_are_read_only_by_tools() {
    let root = xtask::workspace_root();
    let folder = root.join(xtask::rules::AGENTS_FOLDER);
    let entries = walk(&folder, "");
    let missing: Vec<String> = ["validator.md", "baseline.md"]
        .into_iter()
        .filter(|known| !entries.iter().any(|(path, _)| path == known))
        .map(|known| {
            format!(
                "{}{known} is missing.  Fix: add the agent (harness phase 3, M2).",
                xtask::rules::AGENTS_FOLDER
            )
        })
        .collect();
    let problems: Vec<String> = xtask::rules::agents_folder_problems(&entries)
        .iter()
        .map(ToString::to_string)
        .chain(missing)
        .collect();

    assert!(
        problems.is_empty(),
        "{} agent problem(s):\n{}",
        problems.len(),
        problems.join("\n")
    );

    let baseline = format!("{}baseline.md", xtask::rules::AGENTS_FOLDER);
    let hashed = std::process::Command::new("git")
        .args(["hash-object", "--no-filters", &baseline])
        .current_dir(&root)
        .output()
        .expect("git hash-object runs");
    let hash = String::from_utf8_lossy(&hashed.stdout);
    assert!(
        hashed.status.success() && hash.trim() == BASELINE_HASH,
        "{baseline} has the hash {:?}, not BASELINE_HASH {BASELINE_HASH}.  Fix: baseline.md is frozen from the end of harness phase 3, M3, so M5 compares one prompt.  For a change before then, or for a found finding in M6, update BASELINE_HASH in xtask/tests/rules.rs in the same commit.",
        hash.trim()
    );
}

/// The worktree path of a validator holds the ID of its input, so two
/// validators that run at the same time never choose the same path.
const VALIDATOR_WORKTREE_TEXT: &[&str] = &[
    "git worktree add --detach target/review/<round>-validator-<id>-<n> HEAD",
    "`<id>` is the ID of the finding in the input, such as `1-3`, or `mutants` for the missed mutants.",
    "--target-dir target/review/<round>-validator-<id>-<n>/target",
    "git worktree remove --force target/review/<round>-validator-<id>-<n>",
    "If `git worktree add` fails, do not use that path.",
];

#[test]
fn a_validator_worktree_path_holds_the_id_of_its_input() {
    let file = format!("{}validator.md", xtask::rules::AGENTS_FOLDER);
    let text = std::fs::read_to_string(xtask::workspace_root().join(&file))
        .unwrap_or_else(|e| panic!("reading {file}: {e}"));
    let missing: Vec<&&str> = VALIDATOR_WORKTREE_TEXT
        .iter()
        .filter(|wanted| !text.contains(**wanted))
        .collect();
    let shared = text.contains("<round>-<name>-<n>");
    assert!(
        missing.is_empty() && !shared,
        "{file} lacks {missing:?}, or holds the path `<round>-<name>-<n>` ({shared}).  Fix: put the ID of the input in each worktree path.  In review round 1 of harness phase 3, M6, two validators chose the same path, and one wrote in the other's worktree."
    );
}

const REVIEW_SKILL: &str = ".claude/skills/review-rounds/SKILL.md";

/// The frontmatter lines of the review skill that must stay as they are.
const REVIEW_SKILL_FRONTMATTER: &[&str] = &[
    "name: review-rounds",
    "description: Run one review round of CLAUDE.md on this branch",
    "allowed-tools: Bash(target/debug/xtask review *)",
];

/// The keys of the review skill's frontmatter, each exactly once.  Another
/// key, such as `context: fork`, changes how Claude Code runs the skill.
const REVIEW_SKILL_KEYS: &[&str] = &["name", "description", "argument-hint", "allowed-tools"];

/// The line that runs `review start` when the skill loads, so a refusal
/// stops the skill before the model can skip it.
const REVIEW_SKILL_START: &str = "!`target/debug/xtask review start $0 $1`";

/// Text that each step of a round needs: the full reasons from
/// `route.txt`, the mutants run in the background for this round, its time
/// limit, the read of its line after the notice, the branch for a refused
/// mutants run, the `repeat-of` check of the missed mutants, a validator
/// for each baseline finding that cannot share a verdict, the wait for the
/// validators before the wait for the mutants run, the check of the state
/// for this round, the rule for a file that the Write tool refuses to a
/// subagent, and the count of late review commands in the findings file
/// and in the log's rounds table.  The approvals of the round and of the
/// baseline are cost, in their own fields and cells (D4).  Step 9 writes
/// them as "not asked", and the baseline's as "-" without `--baseline`.
/// Step 11 first gets the approval counts.  An orchestrator in `dontAsk`
/// mode, or in auto mode in a `claude -p` run, writes 0 with the mode.  In
/// auto mode in an interactive session, it marks the 0, since it does not
/// see an approval of a fallback prompt (TD-38).
/// A subagent, or a run with no person, does not ask, leaves the fields as
/// step 9 wrote them, and gives the questions to its caller.  In another
/// case, step 11 asks the three questions: the round's count and its window,
/// the baseline's count with `--baseline`, and one total or 0.  Then it
/// waits, and writes each count back to its field of the findings file.
/// With one total, the round's count includes the baseline's, and the
/// baseline's field and cell say so.  An intervention is a message of a
/// person, for the round or the baseline, but not the message that gives
/// the approval counts.  The rounds table's header, its separator line and
/// the end of its block are one entry, so a separator with fewer cells than
/// the header fails.
const REVIEW_SKILL_TEXT: &[&str] = &[
    "in `<run>/$1/route.txt`",
    "target/debug/xtask review mutants $1",
    "target/debug/xtask review check $1",
    "run_in_background",
    "3900000",
    "Read that file only after the notice has come",
    "record its error line as the mutants line",
    "Do the `repeat-of` check of step 4",
    "but not one with `same-as` a finding that has a validator",
    "Wait for each validator before step 6",
    "do not write it in another way",
    "* late review commands:",
    "* Late review commands: the field of the findings file.",
    "So in the default permission mode, `review check` of step 8 asks for approval once in each round.",
    "but not the message that gives the approval counts of step 11",
    "* approvals:",
    "* baseline approvals:",
    "Do not ask yourself.",
    "Use the first of the next three bullets that applies.  Then do the two bullets after them, unless that bullet tells you to stop.",
    "Under the orchestrator bullet, use the first mode sub-bullet that applies, and always the last sub-bullet.",
    "  * In a `claude -p` run where nobody answers permission prompts, write each count as 0 with the mode, in any mode.",
    "A mode that asks for approval gets none in that run, so its count is 0 too",
    "Where a host or a prompt tool answers prompts (`--permission-prompts host`), write the approvals that it gave, if the run records them.",
    "Otherwise write `not counted` with the reason.",
    "  * In `dontAsk` mode in an interactive session, write each count as `0 (dontAsk mode)`.",
    "  * In auto mode in an interactive session, the mode can fall back to a prompt",
    "You do not see an approval of that prompt (TD-38).",
    "In any other case, ask the person:",
    "Do not count the `baseline` agent and its validators.",
    "How many tool calls did you approve for the `baseline` agent and its validators in that time?",
    "Write the first count in the `approvals` field of `<run>/$1/findings.md`.",
    "Write the second count in its `baseline approvals` field, in place of \"not asked\".",
    "the `baseline approvals` field holds \"in approvals\"",
    "  * Without `--baseline`, keep \"-\" in the `baseline approvals` field.  This applies in each mode.",
    "  * In another mode in an interactive session, ask the maintainer the questions below.",
    "If you asked, wait for the answer.  Do not guess the counts.",
    "but not the message that gives the approval counts.  An approval is not an intervention.",
    "* Approvals: the count that the person gave for the round, as cost.  It is the field of the findings file.",
    "First, get the approval counts of the round.  They are cost, not interventions.",
    "How many tool calls did you approve for the round's agents and for me, from `review start` until the findings file?",
    "2. With `--baseline`: ",
    "Give one total if you cannot split them.  In a mode that asks for no approval, each count is 0.",
    "Their interventions are the messages of a person for those agents.",
    "* If you are not the orchestrator, and you are a subagent or no person can answer, as in a `claude -p` run, do not ask.",
    "* approvals: <not asked.",
    "Without --baseline, write \"-\">",
    "Stop here, and give the questions below in your final answer.",
    "The caller asks them, and does the rest of this step.",
    "or \"in approvals\" if the person gave one total",
    "which then includes the baseline's",
    "So write each count as `0 (auto mode; a fallback prompt is not counted)`.",
    "Leave the approvals fields as step 9 wrote them.",
    "Baseline approvals: the count that the person gave for the baseline",
    concat!(
        "| Round | Agents | Tokens | Wall minutes | Interventions | Late review commands | Approvals | Mutants | Baseline agents | Baseline tokens | Baseline wall minutes | Baseline interventions | Baseline approvals | Baseline found |\n",
        "| - | - | - | - | - | - | - | - | - | - | - | - | - | - |\n",
        "```",
    ),
];

/// The problems of the review skill's text: each names the line or the
/// name that it wants.
fn review_skill_problems(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let frontmatter: &[&str] = match lines.split_first() {
        Some((&"---", rest)) => rest
            .iter()
            .position(|line| *line == "---")
            .map_or(&[], |end| &rest[..end]),
        _ => &[],
    };
    let frontmatter_problems = REVIEW_SKILL_FRONTMATTER
        .iter()
        .filter(|wanted| !frontmatter.contains(wanted))
        .map(|wanted| {
            format!("{REVIEW_SKILL}: the frontmatter has no line `{wanted}`.  Fix: put the line back exactly (harness phase 3, M3).")
        });
    let keys: Vec<&str> = frontmatter
        .iter()
        .map(|line| line.split_once(':').map_or(*line, |(key, _)| key).trim())
        .collect();
    let keys_are_exact = keys.len() == REVIEW_SKILL_KEYS.len()
        && REVIEW_SKILL_KEYS
            .iter()
            .all(|wanted| keys.iter().filter(|key| *key == wanted).count() == 1);
    let key_problems = (!keys_are_exact).then(|| {
        format!("{REVIEW_SKILL}: the frontmatter keys are {keys:?}, not each of {REVIEW_SKILL_KEYS:?} once.  Fix: remove the other keys and lines (harness phase 3, M3).")
    });
    let invocation_problems = lines
        .iter()
        .filter(|line| {
            line.split_once(':').is_some_and(|(key, value)| {
                key.trim() == "disable-model-invocation" && value.trim() == "true"
            })
        })
        .map(|_| {
            format!("{REVIEW_SKILL}: a line sets `disable-model-invocation` to `true`.  Fix: remove it, so CLAUDE.md can tell the agent to run the skill.")
        });
    let start_problems = (!lines.contains(&REVIEW_SKILL_START)).then(|| {
        format!("{REVIEW_SKILL}: no line is exactly `{REVIEW_SKILL_START}`.  Fix: put it back, so review start runs when the skill loads, and a refusal stops it.")
    });
    let text_problems = REVIEW_SKILL_TEXT
        .iter()
        .filter(|wanted| !text.contains(*wanted))
        .map(|wanted| {
            format!("{REVIEW_SKILL}: the text does not hold `{wanted}`.  Fix: put back the step that needs it.")
        });
    let spans = xtask::docs::inline_code(text);
    let agent_problems = xtask::review::REVIEWERS
        .iter()
        .map(|reviewer| reviewer.name)
        .chain(["validator", "baseline"])
        .filter(|name| !spans.iter().any(|span| span == name))
        .map(|name| {
            format!("{REVIEW_SKILL}: no code span names the agent `{name}`.  Fix: name it in the step that starts it.")
        });
    frontmatter_problems
        .chain(key_problems)
        .chain(invocation_problems)
        .chain(start_problems)
        .chain(text_problems)
        .chain(agent_problems)
        .collect()
}

/// The review skill runs `xtask review start` when it loads, and keeps the
/// steps of a round that a lost line would drop.
#[test]
fn the_review_skill_starts_with_xtask() {
    let path = xtask::workspace_root().join(REVIEW_SKILL);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("reading {REVIEW_SKILL}: {e}.  Fix: restore the skill (harness phase 3, M3).")
    });
    let problems = review_skill_problems(&text);
    assert!(
        problems.is_empty(),
        "{} review skill problem(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// The tracked files that git's ignore rules match, at the workspace root,
/// with `extra` passed to git before `ls-files`.
fn ignored_tracked_files(extra: &[&str]) -> Vec<String> {
    let out = std::process::Command::new("git")
        .args(extra)
        .args(["ls-files", "-ci", "--exclude-standard"])
        .current_dir(xtask::workspace_root())
        .output()
        .expect("git ls-files runs");
    assert!(
        out.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(ToString::to_string)
        .collect()
}

/// No ignore rule matches a tracked file, here or with a global rule for
/// `Cargo.lock`, so the copy that cargo-mutants makes keeps each one.
#[test]
fn no_tracked_file_is_ignored() {
    let global = std::env::temp_dir().join(format!("xtask-global-ignore-{}", std::process::id()));
    std::fs::write(&global, "Cargo.lock\n").expect("the temporary ignore file can be written");
    let excludes = format!("core.excludesFile={}", global.display());
    let cases = [
        (
            "the ignore rules of this machine",
            ignored_tracked_files(&[]),
        ),
        (
            "a global ignore rule for Cargo.lock",
            ignored_tracked_files(&["-c", &excludes]),
        ),
    ];
    let _ = std::fs::remove_file(&global);
    let problems: Vec<String> = cases
        .iter()
        .flat_map(|(rules, files)| {
            files.iter().map(move |file| {
                format!("{file} is tracked, and is ignored by {rules}.  cargo-mutants copies the tree with the ignore rules (xtask review mutants passes --gitignore=true), so the copy has no {file}.  Fix: add a `!/{file}` line to .gitignore.")
            })
        })
        .collect();
    assert!(
        problems.is_empty(),
        "{} ignored tracked file(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
