# Tech debt

Known gaps in what enforces the rules, and known breaches of them, by ID.  Each entry names the rule it concerns, the evidence, and the fix in view.  An entry is closed by deleting it in the commit that fixes it (git keeps the history); its ID is never reused.

Entries found on 2026-10-19 by a survey of the code for the harness plan (`plans/active/harness-engineering.md`); the plan's phase 2 closes most enforcement gaps.

## Enforcement gaps

### TD-1: CI never runs roformer-gpu's GPU-free tests

`.github/workflows/ci.yml` runs `cargo test --workspace --exclude roformer-gpu`, which also drops the crate's unit tests that need no GPU, among them `workgroup_memory_within_defaults` and `sources_are_complete` (GPU-1) and the `net.rs` and `bench.rs` unit tests.

* Fix: add `cargo test -p roformer-gpu --lib` to the `test` job; add a naga-based test that parses every generated shader and asserts the limits in `references/webgpu-limits.md` (workgroup z is 64, not 256).

### TD-2: dsp switches off every clippy lint

`dsp/src/lib.rs` has `#![allow(clippy::all)]`, which turns off the correctness and performance groups as well as style, and silences `disallowed_methods` (DSP-1).

* Fix: allow only the style lints DSP-1 relaxes, by name.

### TD-3: DSP-1 is not checked

Nothing fails when `dsp/` gains an iterator, a closure or a dependency.

* Fix: a structural test in `xtask/tests/` that parses `dsp/src` and allows only the listed constructs (an allowlist, not a denylist), and asserts `dsp/Cargo.toml` has no dependencies.

### TD-4: LAYER-1 is not checked, and app depends on synth

No test checks the edges between workspace crates.  `app` depends on `synth`, so the demo song generator and its dependencies (hound, serde_json) ship in the page.

* Fix: a test over `cargo metadata` with an allowlist of edges per crate; decide whether `synth` is a pure crate the page may use or a fixture behind a feature.

### TD-5: BUILD-1 is not checked

Nothing fails when a Python file or a `python`/`pip` step appears.

* Fix: a test that fails on any tracked `*.py` other than `tools/roformer_export.py`, and on Python in workflows, `app/Trunk.toml` or a `build.rs`.

### TD-6: ASSET-1 rests on .gitignore alone

`git add -f` gets past it, and `.flac`, `.ogg`, `.m4a`, `.pt`, `.pth`, `.onnx`, `.npy` are not listed.

* Fix: list the missing extensions; a CI check over `git ls-files` for audio and weight extensions and a size cap.

### TD-7: DEPS-1 is not checked, and young crates float

`engine/Cargo.toml` takes `wsola = "0.1.0"`, `cute-dsp = "0.0.41"` and `timestretch = "0.15.0"` as caret ranges; only `rusty_mp3` (in `worker/Cargo.toml`) is pinned exactly.  Nothing compares `THIRD-PARTY.md` with `cargo tree`, and there is no licence policy.

* Fix: pin with `=`; a golden test that reruns the command at the top of `THIRD-PARTY.md` and diffs; `cargo deny` for licences and sources.

### TD-8: tests that cannot run pass silently

GPU tests (`roformer-gpu/tests/kernels.rs`, `memory.rs`) and the PyTorch reference tests print "skipping" and pass when there is no adapter or no fixture, so a run without them looks like a pass (VERIFY-1, GPU-2).  The fixtures are not on the maintainer's Mac either.

* Fix: an environment variable (for example `STEMYLA_REQUIRE=gpu,fixtures`) that turns a skip into a failure, set wherever the evidence is claimed.

### TD-9: UI snapshots are never compared in CI

The `test` job sets `UPDATE_SNAPSHOTS: "1"`, so egui_kittest rewrites the snapshots instead of comparing them (VERIFY-1).

* Fix: compare in CI with a tolerance that lavapipe meets, or generate the snapshots on the CI renderer.

### TD-10: GPU-3 is not checked

Nothing records which adapter a kernel passed on, and any new `TILES` entry becomes a calibration candidate on every user's GPU.

* Fix: log `isFallbackAdapter` and the adapter name in selftest and e2e results; a GPU reviewer that requires a named real GPU in the PR.

### TD-11: MEM-4 is never asserted

The worker logs its wasm memory and live GPU bytes and the page its wasm memory, but no test asserts a total, and the page's WebGL textures and the JS heap are not counted.

* Fix: a tab total (page wasm, worker wasm, GPU live bytes) in an e2e scenario that separates a song and fails over a budget.

## Known breaches

### TD-12: dropped files are copied before they are saved

`app/src/lib.rs:224` calls `Song::load_files(files.clone())`, copying every dropped file's bytes in page memory so they can be saved afterwards, with no stated reason (MEM-1).

* Fix: save first, or decode from a shared buffer; a test that pins it.

### TD-13: an MP3 reply is copied into page memory

`app/src/separator.rs:289` turns the transferred MP3 into a `Vec` with `bytes.to_vec()`, and saving it copies it back out to JS (MEM-1, MEM-2).

* Fix: keep the `Uint8Array` in `Event::Mp3`.

### TD-14: MP3 encoding holds four channel-sized buffers

`worker/src/web.rs:1593` copies both channels out of JS (`left.to_vec()`, `right.to_vec()`) and then interleaves them into a third buffer, and all live through the encode: about 170 MB of worker memory for a 4-minute song (MEM-1).

* Fix: interleave from the JS arrays in blocks, or drop the channel copies before encoding.

### TD-15: the default stretcher allocates in the audio callback

`engine/src/stretch/wsola_adapter.rs` calls `wsola` 0.1.0's `pull`, which returns a new `Vec` on every call, and the chipmunk path's `dsp::double_speed` returns a new `Vec` too.  RT-1 does not yet forbid allocation, but an allocation in the callback can miss its deadline.  No test counts allocations.

* Fix: decide whether RT-1 forbids allocation; if so, a test with a counting global allocator over `render_interleaved` for each stretcher, and a pull into a reused buffer.

### TD-16: the wire format has unchecked copies and silent defaults

`xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` repeat message types and field names by hand, unchecked against `stem_protocol::kind`; a reply missing a field decodes as `""`, `0` or `false`; and there is no version handshake between page and worker (PROTO-1).

* Fix: a test that extracts the type strings from the e2e scripts and checks them against `stem_protocol::kind`; fail decoding on a missing required field; a version in `Alive`.

### TD-17: one OPFS write path is safe only by convention

`app/src/store.rs` passes its argument straight to `write`; it is safe only because every caller passes a whole `Uint8Array`, and no test writes a view through it (MEM-5).

* Fix: a wasm-bindgen test that writes a `subarray` through it and reads back the exact bytes, or copy views as the worker's `whole()` does.

## Legibility for agents

### TD-18: large files and JavaScript inside Rust strings

`roformer-gpu/src/lib.rs`, `app/src/lib.rs`, `worker/src/web.rs` and `xtask/src/e2e.rs` are each over 1,200 lines, and storage logic is JavaScript inside Rust strings (`worker/src/web.rs`, `app/src/store.rs`, `app/src/reset.rs`), out of reach of Rust tooling.

* Fix: split by concern when next changed; move inline JS into `.js` modules.

### TD-19: the running app is legible only through pixels and log text

The app draws to one canvas.  `xtask e2e` clicks fixed coordinates (`VIEWPORT`, `BAR_Y` in `xtask/src/e2e.rs`) and asserts on log lines, so a layout change breaks it, and an evaluator agent cannot read the app's state (VERIFY-1).

* Fix: a small `window.__stemyla` state snapshot (scalars only, per MEM-1), structured console events, `?fixture=demo`, and a command API that queues into the same inbox as a click.

### TD-20: process rules rest on local settings

Commit signing is set in the maintainer's own git config only, the `main` ruleset does not require signed commits, `browser tests (Chrome)` is not a required check, and nothing stops an agent from pushing (GIT-1, GIT-2).

* Fix: permission rules and hooks in the project's Claude Code settings (phase 2 of the plan); ruleset changes in phase 5.
