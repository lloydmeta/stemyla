# 0008: which crates MEM-1's lint covers

Status: accepted (2026-10-21, by the maintainer)

## Context

Harness phase 2, M12, makes clippy's `disallowed-methods` fail `to_vec` on a slice and on a JS typed array (MEM-1).  Clippy reads one `clippy.toml` for each crate: the first that it finds from the crate's folder up to the root.  The known breaches (TD-13, TD-14) are in `app` and `stem-worker`.  No breach is known in `roformer` or `roformer-gpu`, but they hold the largest buffers in the tab: the model's weights and each chunk's output.

## Options

* (a) `app`, `stem-worker`, `stem-protocol` and `engine`, each with its own `clippy.toml`.
* (b) (a), and `roformer` and `roformer-gpu`, each with its own `clippy.toml`.
* (c) `app`, `stem-worker` and `engine` only.
* (d) One `clippy.toml` at the root, and files that opt out in `roformer`, `roformer-gpu` and `xtask`.  A new crate is covered by default.  The cost: each crate's lint configuration lives in two places, and a root file reaches `dsp`'s lint settings.

## Decision

(b), chosen by the maintainer on 2026-10-21.  MEM-1 covers model-sized buffers, and the lint's value is the next copy, not the breaches known today.  The agent had recommended (a).

## Consequences

* Six crates have a `clippy.toml`.  `slice::to_vec` is listed in all six, and `js_sys::Uint8Array::to_vec` and `js_sys::Float32Array::to_vec` in `app`, `stem-worker` and `stem-protocol`.
* Each path in each file has a canary, an `#[expect]` on a planted copy in a test, since clippy passes a path that does not resolve.
* A new crate gets no lint until it has a `clippy.toml` (TD-26).  `synth` has none.
* A crate's other clippy settings go in the same file, since clippy reads only one.  M13 adds MEM-3's entries to `roformer-gpu/clippy.toml`.
