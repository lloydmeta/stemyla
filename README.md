# Stemyla

Stemyla splits a song into its instruments, in the browser, on your own GPU.  Drop an MP3 or WAV, press "Separate 6 stems", and get bass, drums, other, vocals, guitar and piano as separate lanes you can mute, solo, slow down (without changing pitch), loop, and save as WAV or MP3.  Nothing leaves the computer.

It is written in Rust and compiled to WebAssembly, including the GPU code: the separation model runs on WebGPU through hand-written WGSL kernels.  It started as a winter-holiday project for learning to build software with AI, and to hear the guitar part of a song on its own.

![The demo song in Stemyla: a mix lane and five stem lanes](docs/screenshot.png)

## Using it

Open the published page (for example `https://beachape.com/stemyla/`) in a browser with WebGPU: Chrome or Edge on macOS, Windows or ChromeOS, Safari 26, or Firefox 141 and later on Windows ([status by platform](https://github.com/gpuweb/gpuweb/wiki/Implementation-Status)).  It has been tested in Chrome on macOS (a Radeon Pro 5500M and an Intel UHD 630); other browsers should work but are untested.

1. Press **Download model (350 MB)** once.  The page keeps the model in the browser's own storage.
2. Press **Open song…** and choose a song (44.1 kHz MP3 or WAV), drop one on the page, or press **Demo song**.  On a phone or tablet, **Open song…** is the way in.
3. Press **Separate 6 stems**.  On a laptop GPU a 3.5-minute song takes about 7 minutes.
4. Use **Mute** and **Solo** per lane, the tempo slider, and drag across the bar above the lanes to loop a section.  **Save** on a lane downloads it as WAV or MP3.

Each song you open, and its stems once separated, stays in the browser, so a reload brings it back.  The **Storage** menu lists the songs kept, opens or forgets one, shows the space used, and can remove the model.  **Reset…** (also offered when the separation worker stops) fetches the app's files again, bypassing the browser cache, or also deletes everything the page keeps, then reloads: use it when a reload does not help.  **About** links to the model, its code and its paper.

Page options, added to the address: `?selftest` checks the GPU against reference results (when served with the reference files, see below); `?bench` times the GPU kernels (`?bench=i64` only those listed); `?profile=1` logs GPU time per kernel after the self-test or a separation; `?mm=i64` and `?attn=split` (or `fused`, `time`, `freq`) override this GPU's measured kernel choice; `?gpu=integrated` asks for the low-power GPU.

## The model

The separation model is BS-RoFormer SW, a six-stem [BS-RoFormer](https://arxiv.org/abs/2309.02612) whose architecture is open source (MIT, [lucidrains/BS-RoFormer](https://github.com/lucidrains/BS-RoFormer)).  The trained weights are a different matter: their trainer is unknown and no licence was ever stated for them (see [this open question](https://github.com/openmirlab/bs-roformer-infer/issues/3)).  So this repository and the published page do not contain them.  "Download model" fetches a copy that already exists on Hugging Face ([lumabeat/bs-roformer-sw](https://huggingface.co/lumabeat/bs-roformer-sw)) and checks its SHA-256 before use; "Load model file…" accepts a local `.safetensors` file instead.  Whether you may use the weights is for you to judge.

A PyTorch checkpoint (`BS-Rofo-SW-Fixed.ckpt`) converts to the file the app reads with:

```bash
cargo run -p xtask --release -- roformer-convert BS-Rofo-SW-Fixed.ckpt bs_roformer_sw.f16.safetensors
```

## Building and running locally

Everything is built and served with Rust tools.

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked trunk                                   # --locked matters: without it the build of Trunk itself fails
(cd app && trunk build --release)                              # into app/dist
cargo run -p xtask --release -- serve app/dist --port 8080     # http://localhost:8080/
```

To use `?selftest`, make the reference files once and serve them too:

```bash
cargo run -p xtask --release -- roformer-selftest bs_roformer_sw.f16.safetensors third_party/roformer/selftest
cargo run -p xtask --release -- serve app/dist --port 8080 --mount /selftest=third_party/roformer/selftest
```

The native app (`cargo run -p app --release`) plays songs and stems but cannot separate; separation is web only.

## Publishing on GitHub Pages

The page is static, so a `gh-pages` branch serves it.  One command builds it for the site's path, commits the build to `gh-pages` and pushes that branch:

```bash
cargo run -p xtask --release -- publish        # for https://<user>.github.io/stemyla/ or <custom domain>/stemyla/
```

It refuses to run with uncommitted changes, so every published version names the commit it was built from.  It commits through a temporary git index, so the checkout you work in is never switched or touched, and it builds on the remote's `gh-pages` if another clone published last.  Options: `--public-url /other-path/` for a different path, `--no-push` to commit locally only, `--remote <name>`, `--branch <name>`, `--allow-dirty`, `--skip-if-behind <branch>`.

Each build goes in its own folder, `v/<id>/`, named after its commit (`<12-character hash>`, or `<hash>-dirty-<unix seconds>` with `--allow-dirty`), and a published folder is never changed.  Only the root `index.html` changes in place: it is a copy of the newest build's page, whose `<base href>` points into that build's folder.  So a browser that keeps old JavaScript in its cache can never pair it with a newer build's wasm.  The branch keeps the newest 3 builds, listed newest first in `v/VERSIONS`, because a browser may still hold the previous root page for its 10-minute cache lifetime.  Publishing a build id again keeps the copy published first, even if it has been pruned since.  CI passes `--skip-if-behind main`, which publishes nothing unless the run's commit is still the tip of `main`, so re-running an old run cannot roll the site back.

The first time, choose the `gh-pages` branch under the repository's Settings → Pages.  A project site on an account whose user site has a custom domain (`beachape.com` here) appears under that domain at `/stemyla/`.  The site is about 18 MB with 3 builds of about 6 MB each; the model is not part of it.

The CDN in front of `beachape.com` (Cloudflare) lets browsers keep `.js` files for a month.  That is safe for the `v/` folders, but the root `index.html` must stay short-lived: a "Cache Everything" rule, or a long cache lifetime on HTML, would keep browsers on an old build, and once that build is pruned its files return 404.

## Checks

```bash
cargo test --workspace                       # unit, integration and UI snapshot tests (Linux needs ALSA headers)
cargo clippy --workspace --all-targets
```

* `roformer` and `roformer-gpu` compare the network with PyTorch when the fixtures exist (`tools/roformer_export.py`, a Python script that only generates test data; tests skip without them).
* The GPU kernel tests run on any Vulkan, Metal or DX12 GPU.  Without one, SwiftShader's Vulkan driver works: `VK_ICD_FILENAMES=<path>/vk_swiftshader_icd.json WGPU_BACKEND=vulkan cargo test -p roformer-gpu --release`.
* `tools/*.mjs` check the built page in headless Chromium (Node and Playwright): `web_selftest.mjs`, `worker_check.mjs`, `web_session.mjs`, `web_audio_resume.mjs`, `web_bench.mjs`, `web_separate.mjs`, `web_smoke.mjs`.

## Layout

| Path | What |
| - | - |
| `app/` | the egui app (web and native); `index.html` is Trunk's entry point |
| `worker/` | the Web Worker that runs separations |
| `roformer-gpu/` | the network on WebGPU: WGSL kernels and their driver |
| `roformer/` | STFT, chunking, weights, and the CPU reference network |
| `engine/` | decoding, playback, time-stretch, classic drum separation |
| `dsp/` | small audio functions in plain Rust |
| `synth/` | the demo song |
| `xtask/` | the static server and other tools |
| `docs/ARCHITECTURE.md` | how the pieces fit together |

## Licence

MIT or Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`).  Third-party crates and their licences are listed in `THIRD-PARTY.md`.  The model weights are not covered; see "The model".
