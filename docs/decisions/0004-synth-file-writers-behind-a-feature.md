# 0004: synth's file writers are behind a feature, so the web app takes only song generation

Status: proposed

## Context

`app`, one of the two WebAssembly modules of the web app (`app` and `stem-worker`), takes `synth` for the demo song.  So it compiles `hound` for writers that it never calls, and `THIRD-PARTY.md` credits `hound`.  `app_bg.wasm` is only 242 bytes smaller without them, because the linker already drops the unused writers.

## Options

* (a) A `files` feature in `synth`, not a default.  It adds no crate.  The cost: `--workspace` builds merge the feature, so CI needs `-p` steps.
* (b) A separate crate for the writers.  It is an eleventh crate, with a row in the layers table and the crate lists.
* (c) The writers move into `xtask`.  `app`'s test `dropping_several_files_loads_them_as_stems` then writes its WAV files with `engine::wav::encode_pcm16`, as 16-bit files, not `synth`'s 32-bit float.  So `synth` is no longer the one source of the song's files.

## Decision

Option (a), with `app`'s dev-dependency on `synth` with `files` for its native tests only.

## Consequences

* LAYER-1 gains one sentence: in the web app's normal dependency tree, `synth` brings no other crate.
* `synth_brings_no_crate_into_the_web_app` checks the web app's trees.
* Three CI `clippy` steps check `synth`, `app`'s lib and `app`'s tests without the workspace's merged features.
