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

/// Runs the hook as Claude Code does: `sh -c`, the tool call as JSON on
/// stdin, and `CLAUDE_PROJECT_DIR` set to the project root.  `path`, when
/// given, replaces `PATH`.
fn run_hook(
    command: &str,
    project: &std::path::Path,
    input: &serde_json::Value,
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
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input.to_string().as_bytes())
        .expect("the hook reads stdin");
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
        let output = run_hook(&command, &project, &input, None);
        let text = std::fs::read_to_string(path).expect("the test file can be read");
        assert!(
            output.status.success() && text == *expected,
            "{}",
            left(&path.display().to_string(), &text, &output)
        );
    }

    let input = serde_json::json!({ "tool_name": "Write", "tool_input": {} });
    let output = run_hook(&command, &project, &input, None);
    assert!(output.status.success(), "{}", left("no file", "", &output));

    let lib = project.join("lib.rs");
    let input = serde_json::json!({ "tool_name": "Write", "tool_input": { "file_path": lib } });
    let output = run_hook(&command, &project, &input, Some(&no_tools));
    assert!(
        !output.status.success(),
        "GIT-2: the rustfmt hook in .claude/settings.json exits 0 without jq on PATH, so a missing jq is silent.  Fix: keep `|| exit 1` after the jq command substitution."
    );
}
