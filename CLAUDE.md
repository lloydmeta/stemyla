# Stemyla

Stemyla is a Rust workspace for a browser app.  The app separates a song into six stems (bass, drums, other, vocals, guitar, piano) on the user's GPU.  It plays the stems as lanes with mute, solo, tempo and loop.  `README.md` tells how to use and set up the app.  `docs/ARCHITECTURE.md` tells how the parts fit together.

This file is the map.  Claude Code loads it into every session.  It gives what is always true and where to find the rest.  Read the rest from `docs/` when you need it (index: `docs/README.md`).  Claude Code loads a rule from `.claude/rules/` when you touch a file in that rule's paths.  `cargo run -p xtask -- docs-check` (also a test) fails when the map or the docs drift from the code.

## Layout

* `dsp/`: beginner-style pure audio functions with zero dependencies (DSP-1).
* `engine/`: decoding, playback, mixing, time-stretch, classic drum separation, WAV writing.
* `synth/`: the demo song, synthesised so its stems are known exactly; used by the app and as test audio.
* `app/`: eframe/egui UI, native and web.  Trunk's entry point is `app/index.html`.
* `roformer/`: STFT, chunking, weights, and the CPU reference network (Burn, ndarray).
* `roformer-gpu/`: the network on WebGPU (WGSL kernels in `src/shaders/`, driver in `src/lib.rs` and `src/net.rs`).
* `worker/`: the Web Worker that runs separations (package `stem-worker`).
* `protocol/`: the page <-> worker messages (package `stem-protocol`).
* `test-support/`: `skip`, which fails a test that cannot run when `STEMYLA_REQUIRE` asks for evidence; a dev-dependency only.
* `xtask/`: the static server, the browser test runners (`web-test`, `e2e`), `docs-check`, `publish` and other tools.

## Where to look

| Task | Read first |
| - | - |
| Any change | `docs/invariants.md` (the rules below, with why and what enforces each) |
| Find a doc | `docs/README.md` |
| How the person directing you works | `docs/working-with-agents.md` |
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
cargo clippy -p synth --all-targets -- -D warnings  # synth without its files feature
cargo clippy -p app -- -D warnings              # app's lib and binary without any files feature
cargo clippy -p app --all-targets -- -D warnings  # app's tests without the workspace's merged features
cargo clippy --target wasm32-unknown-unknown -p app -p stem-worker -p stem-protocol -p engine --all-targets -- -D warnings  # the web code, which compiles only for wasm32
cargo test --workspace --exclude roformer-gpu   # what CI runs; most roformer-gpu tests need a GPU
cargo test -p roformer-gpu --lib                # CI runs these too, and --doc; they need no GPU
STEMYLA_REQUIRE=gpu cargo test -p roformer-gpu --release  # every kernel against the CPU; fails without a GPU
cargo run -p xtask -- docs-check                # the map and docs match the code
(cd app && trunk build --release)
cargo run -p xtask --release -- serve app/dist --port 8080
cargo run -p xtask -- web-test                  # browser tests (needs wasm-bindgen-cli; see README)
cargo run -p xtask -- e2e <scenario>            # the built page over WebDriver (see README)
cargo run -p xtask --release -- publish         # build, commit to gh-pages, push: a person only (GIT-1)
```

## Rules

Each rule has a stable ID; `docs/invariants.md` states it in full, says why it holds and names what enforces it.  Cite the ID when a change touches a rule.

* BUILD-1: building and serving is Rust only (Cargo, Trunk, `xtask serve`); Python appears only in `tools/roformer_export.py`.
* ASSET-1: never commit audio, stems or model weights; link to the weights, never host them.
* DEPS-1: before adding a crate, confirm it exists and is maintained, pin young crates exactly, keep `THIRD-PARTY.md` current.
* LAYER-1: crates depend only downwards through the layers in `docs/ARCHITECTURE.md`.
* DSP-1: `dsp/` is beginner style: plain types, `while`, `for` over a range, `if` and indexing; no iterators, closures or `swap`.
* MEM-1: a song- or model-sized buffer is never copied to share it; stem audio is `engine::Samples`.
* MEM-2: JS arrays that cross between page and worker are transferred, not cloned.
* MEM-3: GPU buffers are freed explicitly when dropped, never left to garbage collection.
* MEM-4: the tab stays under 2 GB on an iPhone, GPU buffers included.
* MEM-5: every browser-storage (OPFS) write passes a whole array, never a view.
* RT-1: the audio callback never waits for the player slot, never frees a player, and never allocates after warm-up.
* GPU-1: every kernel fits WebGPU's default limits.
* GPU-2: a kernel change is checked against the CPU and in a browser before it becomes a default.
* GPU-3: a kernel never becomes the default on the strength of SwiftShader alone.
* PROTO-1: each message has one encoder and one decoder for both sides; `xtask e2e` writes them by hand too, so the wire format changes only with it.
* PROTO-2: a `?model_url=` is accepted only from the page's own site.
* VERIFY-1: every behaviour is verified by an automated check; "it compiles" is not acceptance, and a skipped test is not a pass.
* GIT-1: push only when asked, never to `main` or `gh-pages`; never merge a PR or run `xtask publish`.  Deny rules refuse the usual commands.
* GIT-2: make small verified steps.  Before each commit, run fmt, clippy, the tests and `docs-check`.  Sign every commit (`git commit -S`).
* GIT-3: never squash or rewrite `gh-pages`.
* DOC-1: a change that makes the map or a doc wrong fixes it in the same commit.

## Working here

* Writing: write docs, comments, commit messages and PR descriptions in [ASD-STE100 Simplified Technical English](https://www.asd-ste100.org).  Keep an instruction to 20 words or fewer, and a description to 25 or fewer.  Write one instruction per sentence, in the active voice.  Use the same word for the same thing.  You need not use STE's American English spelling.
* Plans: work of more than a few steps gets an execution plan in `docs/plans/active/` that follows `docs/plans/PLANS.md`, kept current at every stopping point and moved to `docs/plans/completed/` when done.
* Decisions: a choice with lasting effect gets a record in `docs/decisions/`; agents write it as `proposed`, and only a person accepts it.
* Review: after implementing, run three rounds of review, each by a new subagent with fresh context; verify each finding before fixing it, and fix the real ones with tests.
* Debt: a workaround, a skipped check or a known breach of a rule gets an entry in `docs/tech-debt.md` in the same commit.
