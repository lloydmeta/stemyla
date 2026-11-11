//! The value of `STEMYLA_REQUIRE` at build time reaches the code of a
//! wasm32 build, and not the code of a native build (VERIFY-1).  A wasm
//! test has no environment at run time, so it needs the value of its
//! build.  A native test reads the value of its run, so a value of an
//! earlier build must not count there.
//!
//! Each test compiles `src/lib.rs` with `rustc --emit=mir`, with a marker
//! as the build value.  For wasm32, the marker is in the body of `BUILT`,
//! and `skip_line` gives `BUILT` to `claimed`.  For the host, the MIR has
//! no marker.

use std::path::Path;
use std::process::Command;

use test_support::VAR;

/// The wasm target of the browser tests.
const WASM: &str = "wasm32-unknown-unknown";

/// The MIR of `src/lib.rs`, built for `target` (the host for `None`),
/// with `marker` as the value of `STEMYLA_REQUIRE` at build time.
fn mir(target: Option<&str>, marker: &str) -> String {
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
        .env(VAR, marker);
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
    text
}

/// The MIR body whose first line starts with `head`, to its closing
/// `}` at the start of a line, or an empty text when no line starts with
/// `head`.
fn body<'a>(mir: &'a str, head: &str) -> &'a str {
    let start = if mir.starts_with(head) {
        Some(0)
    } else {
        mir.find(&format!("\n{head}")).map(|at| at + 1)
    };
    let Some(start) = start else {
        return "";
    };
    let rest = &mir[start..];
    rest.find("\n}\n").map_or(rest, |end| &rest[..end + 2])
}

/// A marker that no other text of the MIR holds.
fn marker() -> String {
    format!("lloyd-{}", std::process::id())
}

#[test]
fn a_wasm_build_reads_the_build_value() {
    let marker = marker();
    let mir = mir(Some(WASM), &marker);
    let built = body(&mir, "const BUILT:");
    assert!(built.contains(&marker), "the wasm32 BUILT:\n{built}");
    let skip_line = body(&mir, "fn skip_line(");
    assert!(
        skip_line.contains("claimed(const BUILT,"),
        "skip_line does not give BUILT to claimed:\n{skip_line}"
    );
}

#[test]
fn a_native_build_ignores_the_build_value() {
    let marker = marker();
    assert!(!mir(None, &marker).contains(&marker));
}

#[test]
fn a_body_ends_at_its_closing_brace() {
    let mir = "fn lloyd() {\n    acme\n}\n\nconst BUILT: X = {\n    beachape\n}\n";
    assert_eq!(
        body(mir, "const BUILT:"),
        "const BUILT: X = {\n    beachape\n}"
    );
    assert_eq!(body(mir, "fn lloyd("), "fn lloyd() {\n    acme\n}");
    assert_eq!(body(mir, "fn acme("), "");
    assert_eq!(body(mir, "beachape"), "");
    assert_eq!(
        body("\nfn lloyd() {\n    acme", "fn lloyd("),
        "fn lloyd() {\n    acme"
    );
}
