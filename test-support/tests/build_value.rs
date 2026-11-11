//! The value of `STEMYLA_REQUIRE` at build time reaches the code of a
//! wasm32 build, and not the code of a native build (VERIFY-1).  A wasm
//! test has no environment at run time, so it needs the value of its
//! build.  A native test reads the value of its run, so a value of an
//! earlier build must not count there.
//!
//! Each test compiles `src/lib.rs` with `rustc --emit=mir`, with a marker
//! as the build value, and looks for the marker in the MIR.

use std::path::Path;
use std::process::Command;

use test_support::VAR;

/// The wasm target of the browser tests.
const WASM: &str = "wasm32-unknown-unknown";

/// Whether the MIR of `src/lib.rs`, built for `target` (the host for
/// `None`), holds the value of `STEMYLA_REQUIRE` at build time.
fn the_build_value_reaches_the_code(target: Option<&str>) -> bool {
    let marker = format!("lloyd-{}", std::process::id());
    let dir = std::env::temp_dir().join(format!(
        "test-support-mir-{}-{}",
        target.unwrap_or("host"),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("a temporary folder");
    let mir = dir.join("lib.mir");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let mut rustc = Command::new("rustc");
    rustc
        .args(["--edition", "2024", "--crate-type", "lib"])
        .args(["--crate-name", "test_support"])
        .arg(format!("--emit=mir={}", mir.display()))
        .arg(&source)
        .env(VAR, &marker);
    if let Some(target) = target {
        rustc.args(["--target", target]);
    }
    let output = rustc.output().expect("rustc runs");
    assert!(
        output.status.success(),
        "rustc failed.  For {WASM}, install the target with `cargo run -p xtask -- setup wasm32`.\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&mir).expect("the MIR file");
    std::fs::remove_dir_all(&dir).expect("the temporary folder goes");
    text.contains(&marker)
}

#[test]
fn a_wasm_build_reads_the_build_value() {
    assert!(the_build_value_reaches_the_code(Some(WASM)));
}

#[test]
fn a_native_build_ignores_the_build_value() {
    assert!(!the_build_value_reaches_the_code(None));
}
