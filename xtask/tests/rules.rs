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
fn claude_settings_deny_publish_and_merge() {
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
