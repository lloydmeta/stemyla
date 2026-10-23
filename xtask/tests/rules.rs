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
        &read("protocol/src/lib.rs"),
        &scripts,
        &read("xtask/src/e2e.rs"),
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
