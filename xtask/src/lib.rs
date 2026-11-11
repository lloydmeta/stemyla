//! Offline tooling: synthetic song, metrics, benches, model conversion, the static server.

pub mod attention;
pub mod bench;
pub mod check;
pub mod clean;
pub mod docs;
pub mod drivers;
pub mod e2e;
pub mod metrics;
pub mod pr_body;
pub mod publish;
pub mod review;
pub mod review_log;
pub mod roformer_cli;
pub mod rules;
pub mod separate;
pub mod serve;
pub mod setup;
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

/// The stdout of `cmd`, or `None` when it does not start or exits with a
/// failure.
pub fn stdout_on_success(cmd: &mut std::process::Command) -> Option<String> {
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[test]
    fn a_command_that_succeeds_gives_its_stdout() {
        assert_eq!(
            stdout_on_success(&mut sh("echo lloyd; echo beachape >&2")),
            Some("lloyd\n".to_string())
        );
    }

    #[test]
    fn a_command_that_fails_or_does_not_start_gives_none() {
        assert_eq!(stdout_on_success(&mut sh("echo lloyd; exit 1")), None);
        assert_eq!(
            stdout_on_success(&mut Command::new("/no/such/program-lloyd")),
            None
        );
    }
}
