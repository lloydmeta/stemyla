# Stemyla

Rust workspace for a browser app that separates a song into six stems (bass, drums, other, vocals, guitar, piano) on the user's GPU and plays them as lanes with mute, solo, tempo and loop.  See `README.md` for use and `docs/ARCHITECTURE.md` for how it fits together.

## Layout

* `dsp/`: beginner-style pure functions with zero dependencies: only `&[f32]`, `Vec<f32>`, `usize`, `for`, `while`, `if` and indexing.  No iterators, closures or `swap`.  Clippy style lints are relaxed there on purpose.
* `engine/`: decoding, playback, mixing, time-stretch, classic drum separation, WAV writing.
* `app/`: eframe/egui UI, native and web.  Trunk's entry point is `app/index.html`.
* `roformer/`: STFT, chunking, weights, and the CPU reference network (Burn, ndarray).
* `roformer-gpu/`: the network on WebGPU (WGSL kernels in `src/shaders/`, driver in `src/lib.rs` and `src/net.rs`).
* `worker/`: the Web Worker that runs separations.
* `xtask/`: the static server and other tools.

## Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
(cd app && trunk build --release)
cargo run -p xtask --release -- serve app/dist --port 8080
cargo run -p xtask --release -- publish        # build, commit to gh-pages, push
```

## Rules

* Building and serving the app is Rust only (Cargo, Trunk, `xtask serve`).  Never suggest or add Python or other non-Rust servers or build steps for it.  Python appears only in `tools/roformer_export.py`, which generates test fixtures from the PyTorch reference.
* Correctness first: GPU code must stay within WebGPU's default limits, every change to a kernel is checked against the CPU (`cargo test -p roformer-gpu`) and in a browser (`?selftest`) before it becomes a default, and a new kernel is never made the default on the strength of SwiftShader alone.
* Verify every behaviour with an automated check; "it compiles" is not acceptance.
* Never commit audio, stems or model weights (see `.gitignore`).  The model weights have no stated licence: link to them, never host them.
* Before adding a crate, confirm it exists and is maintained, pin young crates exactly, and keep `THIRD-PARTY.md` current.
* Small verified steps; run clippy and tests before every commit.  Never push, or run `xtask publish` (which pushes), without being asked.
