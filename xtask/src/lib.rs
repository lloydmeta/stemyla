//! Offline tooling: synthetic song, metrics, benches, model conversion, the static server.

pub mod bench;
pub mod docs;
pub mod drivers;
pub mod e2e;
pub mod metrics;
pub mod publish;
pub mod review;
pub mod roformer_cli;
pub mod rules;
pub mod separate;
pub mod serve;
pub mod web_test;
pub mod webdriver;
/// The workspace's root folder (xtask's parent).
pub fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is inside the workspace")
        .to_path_buf()
}

/// The synthetic song lives in its own crate so the app can use it too.
pub use ::synth;
