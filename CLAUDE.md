# Stemyla

Rust workspace for a browser app that separates a song into six stems (bass, drums, other, vocals, guitar, piano) on the user's GPU and plays them as lanes with mute, solo, tempo and loop.  `README.md` covers using and setting it up; `docs/ARCHITECTURE.md` covers how it fits together.

This file is the map, loaded into every session: what is always true, and where to look for the rest.  Everything else is read on demand from `docs/` (index: `docs/README.md`), and the rules for one part of the tree load from `.claude/rules/` when its files are touched.  `cargo run -p xtask -- docs-check` (also a test) fails when this map or the docs drift from the code.

## Layout

* `dsp/`: beginner-style pure audio functions with zero dependencies (DSP-1).
* `engine/`: decoding, playback, mixing, time-stretch, classic drum separation, WAV writing.
* `synth/`: the demo song, synthesised so its stems are known exactly; used by the app and as test audio.
* `app/`: eframe/egui UI, native and web.  Trunk's entry point is `app/index.html`.
* `roformer/`: STFT, chunking, weights, and the CPU reference network (Burn, ndarray).
* `roformer-gpu/`: the network on WebGPU (WGSL kernels in `src/shaders/`, driver in `src/lib.rs` and `src/net.rs`).
* `worker/`: the Web Worker that runs separations (package `stem-worker`).
* `protocol/`: the page <-> worker messages (package `stem-protocol`).
* `xtask/`: the static server, the browser test runners (`web-test`, `e2e`), `docs-check`, `publish` and other tools.

## Where to look

| Task | Read first |
| - | - |
| Any change | `docs/invariants.md` (the rules below, with why and what enforces each) |
| Find a doc | `docs/README.md` |
| Work of more than a few steps | `docs/plans/PLANS.md`, then `docs/plans/active/` |
| GPU kernels, the model | `docs/ARCHITECTURE.md` (the model; running it on the GPU) |
| Memory, buffers, storage | `docs/ARCHITECTURE.md` (memory on phones; browser storage) |
| Browser tests on this Mac | `docs/references/local-browser-testing.md` |
| Safari and WebGPU quirks | `docs/references/browser-quirks.md`, `docs/references/webgpu-limits.md` |
| Known gaps and breaches | `docs/tech-debt.md` |
| Why the harness is shaped this way | `docs/harness.md`, `docs/decisions/` |

## Commands

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude roformer-gpu   # what CI runs; roformer-gpu's tests need a GPU (see README)
cargo test -p roformer-gpu --release            # every kernel against the CPU, on this machine's GPU
cargo run -p xtask -- docs-check                # the map and docs match the code
(cd app && trunk build --release)
cargo run -p xtask --release -- serve app/dist --port 8080
cargo run -p xtask -- web-test                  # browser tests (needs wasm-bindgen-cli; see README)
cargo run -p xtask -- e2e <scenario>            # the built page over WebDriver (see README)
cargo run -p xtask --release -- publish         # build, commit to gh-pages, push (GIT-1)
```

## Rules

Each rule has a stable ID; `docs/invariants.md` states it in full, says why it holds and names what enforces it.  Cite the ID when a change touches a rule.

* BUILD-1: building and serving is Rust only (Cargo, Trunk, `xtask serve`); Python appears only in `tools/roformer_export.py`.
* ASSET-1: never commit audio, stems or model weights; link to the weights, never host them.
* DEPS-1: before adding a crate, confirm it exists and is maintained, pin young crates exactly, keep `THIRD-PARTY.md` current.
* LAYER-1: crates depend only downwards through the layers in `docs/ARCHITECTURE.md`.
* DSP-1: `dsp/` uses only `&[f32]`, `Vec<f32>`, `usize`, `for`, `while`, `if` and indexing; no iterators, closures or `swap`.
* MEM-1: a song- or model-sized buffer is never copied to share it; stem audio is `engine::Samples`.
* MEM-2: JS arrays that cross between page and worker are transferred, not cloned.
* MEM-3: GPU buffers are freed explicitly when dropped, never left to garbage collection.
* MEM-4: the tab stays under 2 GB on an iPhone, GPU buffers included.
* MEM-5: every browser-storage (OPFS) write passes a whole array, never a view.
* RT-1: the audio callback never waits for the player slot and never frees a player.
* GPU-1: every kernel fits WebGPU's default limits.
* GPU-2: a kernel change is checked against the CPU and in a browser before it becomes a default.
* GPU-3: a kernel never becomes the default on the strength of SwiftShader alone.
* PROTO-1: each message has one encoder and one decoder for both sides; `xtask e2e` writes them by hand too, so the wire format changes only with it.
* PROTO-2: a `?model_url=` is accepted only from the page's own site.
* VERIFY-1: every behaviour is verified by an automated check; "it compiles" is not acceptance, and a skipped test is not a pass.
* GIT-1: never push, or run `xtask publish` (which pushes), without being asked.
* GIT-2: small verified steps: fmt, clippy, tests and `docs-check` pass before every commit, and commits are signed (`git commit -S`).
* GIT-3: never squash or rewrite `gh-pages`.
* DOC-1: a change that makes the map or a doc wrong fixes it in the same commit.

## Working here

* Writing: docs, comments, commit messages and PR descriptions follow [ASD-STE100 Simplified Technical English](https://www.asd-ste100.org): short sentences (at most 20 words for an instruction, 25 for a description), one instruction per sentence, active voice, and the same word for the same thing throughout.  STE's American English spelling is not required.
* Plans: work of more than a few steps gets an execution plan in `docs/plans/active/` that follows `docs/plans/PLANS.md`, kept current at every stopping point and moved to `docs/plans/completed/` when done.
* Decisions: a choice with lasting effect gets a record in `docs/decisions/`; agents write it as `proposed`, and only a person accepts it.
* Review: after implementing, run three rounds of review, each by a new subagent with fresh context; verify each finding before fixing it, and fix the real ones with tests.
* Debt: a workaround, a skipped check or a known breach of a rule gets an entry in `docs/tech-debt.md` in the same commit.
