//! `.cargo/mutants.toml` excludes the functions of `app/src/lib.rs` that
//! compile for wasm32 only, because no native test can catch a mutant in
//! them (finding 3-3 of review round 3 of M7).

use std::path::Path;

/// The names of the functions directly under `#[cfg(target_arch =
/// "wasm32")]` in `source`, other attributes between them allowed.
fn wasm32_only_functions(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut names = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim() != r#"#[cfg(target_arch = "wasm32")]"# {
            continue;
        }
        let Some(item) = lines[i + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.starts_with("#["))
        else {
            continue;
        };
        let item = item
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub ")
            .trim_start_matches("async ");
        if let Some(rest) = item.strip_prefix("fn ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            names.push(name);
        }
    }
    names
}

/// Whether `name` is one of the alternatives of a group in `config`, such
/// as `(a|name|b)`.
fn excluded(config: &str, name: &str) -> bool {
    ["(", "|"].iter().any(|before| {
        [")", "|"]
            .iter()
            .any(|after| config.contains(&format!("{before}{name}{after}")))
    })
}

#[test]
fn the_wasm32_only_functions_of_the_app_are_excluded_from_mutants() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let source = std::fs::read_to_string(root.join("app/src/lib.rs")).unwrap();
    let config = std::fs::read_to_string(root.join(".cargo/mutants.toml")).unwrap();
    let names = wasm32_only_functions(&source);
    assert!(names.len() > 40, "the scan finds the functions: {names:?}");
    let missing: Vec<&String> = names.iter().filter(|n| !excluded(&config, n)).collect();
    assert!(
        missing.is_empty(),
        "add these wasm32-only functions to exclude_re: {missing:?}"
    );
}

#[test]
fn the_scan_finds_functions_under_the_attribute_only() {
    let source = r#"
#[cfg(target_arch = "wasm32")]
fn acme() {}
#[cfg(target_arch = "wasm32")]
#[expect(dead_code)]
pub async fn lloyd() {}
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn beachape(&self) {}
#[cfg(target_arch = "wasm32")]
struct Acme;
fn native() {}
"#;
    assert_eq!(wasm32_only_functions(source), ["acme", "lloyd", "beachape"]);
    let config = "exclude_re = ['(replace|in) (acme|lloyd)\\b', '(replace|in) beachape\\b']";
    assert!(excluded(config, "acme") && excluded(config, "lloyd"));
    assert!(!excluded(config, "beachape"), "not in a group");
    assert!(!excluded(config, "acm"), "a whole name only");
}
