# Docs index

What each doc is for and when to read it.  `CLAUDE.md` at the root is the map loaded into every session; everything here is read on demand.  `cargo run -p xtask -- docs-check` fails when a doc under `docs/` is missing from this list.  One line, `reviews/`, lists each log in the reviews folder.  These are the branch logs, and `reviews/replay.md`, the record of harness phase 3's replay of old reviews (M5).

## Always relevant

* `invariants.md`: every rule by ID (MEM-1, GPU-1, ...), why it holds, what enforces it and its known gaps.  Read before any change.
* `tech-debt.md`: known gaps in enforcement and known breaches of the rules, by ID (`TD-n`).  Read before relying on a rule being checked, and add to it when leaving a gap.

## How it works

* `ARCHITECTURE.md`: the pipeline from a song to six stems, the crates and their layers, the model, memory on phones, the GPU kernels, browser storage, the tests, and a vocabulary of terms.

## How work is done

* `working-with-agents.md`: for the person who directs the agent: recipes for common tasks, what a person must still do, and the tools.  Read it first when you start to work here.
* `harness.md`: the parts of the agent harness (map, rules, checks, reviews), the failure each one addresses, and the baseline it is measured against, and phase 2's numbers.
* `plans/PLANS.md`: how to write and keep an execution plan.  Plans in progress are in `plans/active/`, finished ones in `plans/completed/`.
* `plans/active/harness-engineering.md`: the phased adoption of harness engineering in this repo.
* `plans/active/stream-from-storage.md`: a song's audio moves to browser storage, so the tab's memory does not grow with the song (MEM-4, TD-54).
* `plans/active/stream-from-storage-m3.md`: M3 of that plan: the worker writes the stems to their session files, and the page falls back to memory when storage is unavailable or full.
* `plans/active/stream-from-storage-m4.md`: M4 of that plan: the page keeps the mix in a session file, and the worker reads it in blocks.
* `plans/active/stream-from-storage-m5.md`: M5 of that plan: a new decode worker reads a stored song in blocks and writes its session files and overviews.
* `plans/active/stream-from-storage-m6.md`: M6 of that plan: playback reads stored lanes through a read-ahead buffer that the main thread fills.
* `plans/active/stream-from-storage-m7.md`: M7 of that plan: the page shows a stored song from its files and overviews, with no read-back; spectrograms, classic drums and saves read blocks.
* `plans/active/stream-from-storage-catchup.md`: the catch-up after M7: round 4 of M3 to M6, their deferred should-fix, the page-decode limit, `mutants::skip`, the Safari worker memory, and a 3,072-frame output buffer.
* `plans/active/stream-from-storage-m8.md`: M8 of that plan: a save writes its file to browser storage in blocks, and the worker encodes FLAC and MP3 from that file.
* `plans/active/stream-from-storage-m9.md`: M9 of that plan: the GPU keeps the weight matrices of an f16 model as f16, and computes in f32.
* `plans/completed/mix-download.md`: done on 2026-10-25: the mix lane saves the mix as heard (WAV, FLAC, MP3), mixed on the page in blocks; its waveform and spectrogram follow the mix.
* `plans/completed/flac.md`: done on 2026-10-25: FLAC files open as songs and stems, a stem saves as a 16-bit FLAC, and a decoded file holds at most 1 GiB of samples.
* `plans/completed/48k-audio.md`: done on 2026-10-24: songs at 48 kHz and other standard rates convert to 44.1 kHz when they open, so they separate.
* `plans/completed/harness-phase-2.md`: phase 2 of that plan, done on 2026-10-22: each rule a machine can check became a test, a lint or a hook, one PR per milestone.  Its retrospective gives what phase 2 achieved and what it did not.
* `plans/completed/harness-phase-3.md`: phase 3 of that plan, done on 2026-10-24: review agents chosen by the diff, a validator for each serious finding, cargo-mutants and a findings log, in one PR.  Its gate is still open.  Its retrospective gives what phase 3 achieved and what it did not.
* `plans/completed/harness-phase-3-m1.md`: M1 of phase 3, done: `xtask review` routes a diff to reviewers, guards a round, and runs cargo-mutants.
* `plans/completed/harness-phase-3-m2.md`: M2 of phase 3, done: eight read-only review agents, and a guard on their Bash commands.
* `plans/completed/harness-phase-3-m3.md`: M3 of phase 3, done: the `review-rounds` skill, which runs one review round, and a test harness that runs it in `claude -p` on a clone.
* `plans/completed/harness-phase-3-m4.md`: M4 of phase 3, done: the findings log, one file per branch in the reviews folder, which `docs-check` accepts, and the count of the skill's late review commands apart from interventions.
* `plans/completed/harness-phase-3-m5.md`: M5 of phase 3, done: the replay of six old review rounds in the test harness, against a ground truth written before the runs.
* `plans/completed/harness-phase-3-m6.md`: M6 of phase 3, done: the commits before the review rounds of the PR: decision 0011, approvals as cost in the review log, and tech debt that names the agent that looks.
* `decisions/0001-harness-in-the-repo.md`: agent knowledge is versioned in the repo, with `CLAUDE.md` as a map; the harness is Claude Code only.
* `decisions/0002-staged-background-autonomy.md`: agents are to work in the background, up to merging their own PRs, reached in stages with gates.
* `decisions/0003-required-skips-in-a-test-support-crate.md`: `STEMYLA_REQUIRE` turns a skipped test into a failure, through one helper in the `test-support` crate.
* `decisions/0004-synth-file-writers-behind-a-feature.md`: `synth`'s file writers are behind its `files` feature, so the web app (`app` and `stem-worker`) takes only song generation.
* `decisions/0005-how-strictly-the-crate-checks-read-layer-1.md`: the layers table lists each dependency between workspace crates, and the page side and the worker side share only `stem-protocol` (LAYER-1).
* `decisions/0006-clippy-exceptions-in-dsp.md`: `dsp` keeps its beginner forms (DSP-1), and `dsp/src/lib.rs` expects each clippy lint that they trigger, by name and with a reason.
* `decisions/0007-no-allocation-in-the-audio-callback.md`: after warm-up, the audio callback never allocates or frees memory, for the stretchers the app ships (RT-1).
* `decisions/0008-mem-1-lint-in-each-crate.md`: six crates, each with its own `clippy.toml`, fail `to_vec` on a slice or a JS typed array (MEM-1).
* `decisions/0009-mem-3-lint-and-private-device.md`: clippy fails five ways to make a GPU buffer in `roformer-gpu` outside `GpuBuffer`, and `Gpu`'s device is private (MEM-3).
* `decisions/0010-git-1-deny-rules.md`: Claude Code deny rules refuse an agent's push that names `main` or `gh-pages`, `xtask publish` and `gh pr merge`; it pushes other branches when asked (GIT-1).
* `decisions/0011-review-agents.md`: review agents chosen by `xtask review` from the diff, a validator for each serious finding, cargo-mutants in the skill, and a log of each branch's rounds (harness phase 3).
* `decisions/0012-one-song-rate.md`: every song in the app is at 44.1 kHz; `Song::from_audio` converts other rates on the page thread.
* `decisions/0013-decode-limit.md`: a decoded file holds at most 1 GiB of `f32` samples, in every format, so a small FLAC cannot stop the tab.
* `decisions/0014-save-the-mix-as-heard.md`: the mix lane saves the mix as heard (Mute, Solo, volume), mixed on the page in blocks; the worker encodes as before.
* `decisions/0015-storage-holds-the-song.md`: browser storage holds a song's audio, and memory holds blocks, so a long song fits on an iPhone.
* `decisions/0016-stem-files-are-16-bit.md`: stem files in browser storage hold 16-bit PCM, and the mix and source files 32-bit float.
* `decisions/0017-the-main-thread-reads-ahead.md`: async reads on the main thread keep a read-ahead buffer in front of the play head.
* `decisions/0018-f16-weights-on-the-gpu.md`: the GPU keeps the f16 weight matrices of an f16 model file as f16, and computes in f32.
* `decisions/0019-a-decode-worker-crate.md`: a page-side worker crate that takes `engine` decodes a song in blocks into browser storage.
* `decisions/0020-memory-path-limit-shares-lanes.md`: the memory path's limit shrinks with the lanes that the page holds in memory, and an open that decodes on the page keeps it (accepted).
* `decisions/0021-saves-go-through-browser-storage.md`: every save writes a 16-bit WAV save file in browser storage, the worker encodes FLAC and MP3 from it into a save file of its own, and the page offers the file (accepted).
* `reviews/`: the log of the review rounds of each PR branch, one file for each branch, which step 11 of the `review-rounds` skill writes.  Read it to see what a review found and what it cost.  Beside the branch logs, `reviews/replay.md` records harness phase 3's replay of six old review rounds (M5), with its ground truth.

## References

Distilled facts from outside the repo, each with its source and the date it was checked.

* `references/browser-quirks.md`: Safari, Web Audio and WebGPU behaviour the code works around.
* `references/webgpu-limits.md`: the WebGPU default limits every kernel must fit (GPU-1).
* `references/local-browser-testing.md`: running `xtask web-test` and `xtask e2e` on the maintainer's Mac: Safari, Chrome, model files and known local failures; reading an iPhone's memory kills.

## Conventions

These are what `docs-check` holds the docs to:

* Paths in code spans (`engine/src/song.rs`) exist, relative to the repo root, to the doc's own folder or, inside `docs/`, to `docs/`.  Plans, decisions and review logs are exempt: plans name files still to be made, and decisions and review logs are records of their time.  Name files, functions and types rather than linking to lines that move; `file.rs:12` is fine for evidence, and the check ignores the line.
* IDs are defined once, as a `### ID: title` heading in `invariants.md` or `tech-debt.md`, and never reused; every `###` heading there is an ID.  A mention of an ID whose prefix is in use (MEM, TD, ...) must be defined; an unknown prefix cannot be told from text such as `SHA-256`, so it is not checked.  A closed ID is a `* ID: ...` bullet under `## Closed`.  Only plans, decisions and review logs may name it, and it cannot be defined again.
* The tests an invariant names on its `* Enforced by:` line exist.
* Every workspace crate is listed in the layout of `CLAUDE.md` and `README.md` and under the crates of `ARCHITECTURE.md`.
* Decisions are numbered `0001-...` with no gaps and carry a `Status:` line.
* Plans live in `plans/active/` or `plans/completed/` and keep the living sections listed in `plans/PLANS.md`.
* Every rule in `.claude/rules/` has `paths:` frontmatter whose globs match files.
* `CLAUDE.md` stays within 100 lines, and its rules section lists exactly the current rules in `invariants.md`.
* `working-with-agents.md` names every `xtask` command and every project skill, subagent and command in `.claude/`.
