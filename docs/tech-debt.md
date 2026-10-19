# Tech debt

Known gaps in what enforces the rules, and known breaches of them, by ID.  Each entry names the rule it concerns, the evidence, and the fix in view.  To close an entry, delete it in the commit that fixes it.  In the same commit, add its ID under `## Closed`, with what closed it.  An ID is never reused.  Plans and decisions may still name a closed ID; `docs-check` fails it in other docs.  Add new entries above `## Closed`.

Entries found on 2026-10-19 by a survey of the code for the harness plan (`plans/active/harness-engineering.md`); the plan's phase 2 closes most enforcement gaps.

## Enforcement gaps

### TD-2: dsp switches off every clippy lint

`dsp/src/lib.rs` has `#![allow(clippy::all)]`, which turns off the correctness and performance groups as well as style, and silences `disallowed_methods` (DSP-1).

* Fix: allow only the style lints DSP-1 relaxes, by name.

### TD-3: DSP-1 is not checked

Nothing fails when `dsp/` gains an iterator, a closure or a dependency.

* Fix: a structural test in `xtask/tests/` that parses `dsp/src` and allows only the listed constructs (an allowlist, not a denylist), and asserts `dsp/Cargo.toml` has no dependencies.

### TD-4: LAYER-1 is not checked, and app depends on synth

No test checks the edges between workspace crates.  `app` depends on `synth`, so the demo song generator and its dependencies (hound, serde_json) compile into the page build.

* Fix: a test over `cargo metadata` with an allowlist of edges per crate.  Decided 2026-10-19: `synth`'s file writing moves behind a feature, so the page links only song generation (`plans/active/harness-phase-2.md`, M4 and M5).

### TD-5: BUILD-1 is not checked

Nothing fails when a Python file or a `python`/`pip` step appears.

* Fix: a test that fails on any tracked `*.py` other than `tools/roformer_export.py`, and on Python in workflows, `app/Trunk.toml` or a `build.rs`.

### TD-6: ASSET-1 rests on .gitignore alone

`git add -f` gets past it, and `.flac`, `.ogg`, `.m4a`, `.pt`, `.pth`, `.onnx`, `.npy` are not listed.  Its folder patterns (`audio/`, `stems/`, `dist/`, `pages/`) are not anchored, so they also ignore a source folder of that name at any depth.

* Fix: list the missing extensions; a CI check over `git ls-files` for audio and weight extensions and a size cap.

### TD-7: DEPS-1 is not checked, and young crates float

`engine/Cargo.toml` takes `wsola = "0.1.0"` and `timestretch = "0.15.0"` as caret ranges, which accept later 0.1 and 0.15 releases (`cute-dsp = "0.0.41"` already accepts only 0.0.41); only `rusty_mp3` (in `worker/Cargo.toml`) is pinned exactly.  Nothing compares `THIRD-PARTY.md` with `cargo tree`, and there is no licence policy.

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

### TD-21: what docs-check cannot see

`xtask docs-check` (DOC-1) checks the shape of the docs, not whether they are still true:

* A doc that names real files and tests but describes old behaviour passes.
* An ID with a prefix not yet in use (`SEC-1`) is not checked, because it cannot be told from text such as `SHA-256`.
* A path counts as present if the file is on disk, so a file not yet added to git passes locally and fails only in CI.
* `README.md` and crate docs (`//!`) are checked for the crate list only.
* A bare file name (`store.rs`) is not checked, because the check cannot tell which crate it means.

* Fix: the drift job in phase 5a of the plan, which compares each doc with the code it describes.

### TD-22: a compile-fail doctest passes on any compile error

MEM-3's doctest on `Rec` is marked `compile_fail,E0505`, but stable rustdoc does not check the error code.  A `compile_fail,E0505` doctest that fails with E0308 passes on rustc 1.99.0 (checked 2026-10-19).  The `no_run` doctest beside it compiles the same calls, so an API change still fails a test; a different error in the `compile_fail` copy alone does not.

* Fix: run `cargo +nightly test -p roformer-gpu --doc` in CI, since nightly rustdoc checks the error code; or check it with a UI-test harness such as `trybuild` (a new crate, DEPS-1).

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

* Fix: decided 2026-10-19: RT-1 is to forbid allocation after warm-up.  A test with a counting global allocator over `render_interleaved` for each stretcher the app ships, wsola 0.2.0 (whose `pull` does not allocate), and chipmunk decimation in place (`plans/active/harness-phase-2.md`, M11).

### TD-16: the wire format has unchecked copies and silent defaults

`xtask/e2e/worker.js` and the scripts in `xtask/src/e2e.rs` repeat message types and field names by hand, unchecked against `stem_protocol::kind`; a reply missing a field decodes as `""`, `0` or `false`; and there is no version handshake between page and worker (PROTO-1).

* Fix: a test that extracts the type strings from the e2e scripts and checks them against `stem_protocol::kind`; fail decoding on a missing required field; a version in `Alive`.

### TD-17: one OPFS write path is safe only by construction

`app/src/store.rs` passes its argument straight to `write`.  Its one caller passes `Uint8Array::from(bytes)`, a new whole array, so no view reaches it today; but nothing stops a later caller passing one, and no test writes a view through it (MEM-5).

* Fix: a wasm-bindgen test that writes a `subarray` through it and reads back the exact bytes, or copy views as the worker's `whole()` does.

## Legibility for agents

### TD-18: large files and JavaScript inside Rust strings

`roformer-gpu/src/lib.rs`, `app/src/lib.rs`, `worker/src/web.rs` and `xtask/src/e2e.rs` are each over 1,200 lines, and storage logic is JavaScript inside Rust strings (`worker/src/web.rs`, `app/src/store.rs`, `app/src/reset.rs`), out of reach of Rust tooling.

* Fix: split by concern when next changed; move inline JS into `.js` modules.

### TD-19: the running app is legible only through pixels and log text

The app draws to one canvas.  `xtask e2e` clicks fixed coordinates (`VIEWPORT`, `BAR_Y` in `xtask/src/e2e.rs`) and asserts on log lines, so a layout change breaks it, and an evaluator agent cannot read the app's state (VERIFY-1).

* Fix: a small `window.__stemyla` state snapshot (scalars only, per MEM-1), structured console events, `?fixture=demo`, and a command API that queues into the same inbox as a click.

### TD-20: process rules rest on local settings

Commit signing is set in the maintainer's own git config only, and nothing stops an agent from pushing (GIT-1, GIT-2).  The `main` ruleset (checked 2026-10-19 with `gh api repos/lloydmeta/stemyla/rulesets/24980616`):

* does not require signed commits;
* lets the admin role bypass it always, so agents running with the maintainer's credentials can skip the required checks and the approval;
* does not require a branch to be up to date before merging (`strict_required_status_checks_policy: false`), so a PR can merge untested against the current `main` (the failure behind PR #12).

* Fix: in phase 2 of the plan, permission rules and hooks in the project's Claude Code settings; in phase 5, the other ruleset changes.

## Closed

* TD-1: closed by `every_kernel_fits_the_default_limits` (harness phase 2, M2).  It checks every shader against the default limits with naga (GPU-1).
