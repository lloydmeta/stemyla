# Stream from storage, M6: playback reads storage through the read-ahead buffer

## Purpose

This is the milestone plan for M6 of `plans/active/stream-from-storage.md` (the parent plan).  It gives more detail than the parent's M6 section, which stays the criteria.  After M6:

* The `Player` reads each lane through a trait in `engine` that gives the frames of a stem from a position.  A lane in memory (`Samples`) implements it, and so does a lane in a session file.
* For a lane in a session file, the audio callback copies frames only from a read-ahead buffer of a fixed size.  An async reader on the main thread keeps that buffer full in front of the play head and after the loop start (decision 0017).  If the buffer does not hold the frames, the lane plays silence and the buffer counts an underrun.
* A song shown from its session files plays from those files: the source lanes, the separated stems and the original lane.  A lane that has no complete session file (classic drums made on the page, the demo song, a song on the memory path) plays from memory, as today.
* No failure removes `mix.f32`, or any other session file, while the shown song plays from it.

M6 does not lower the page's memory yet.  The page still reads the session files back into `Samples` for the lanes, the spectrograms, classic drums and saves, until M7 and M8 (TD-54).  M6 adds the read-ahead buffer, at most 32 MiB.

## Context and orientation

The state at the start of M6, on 2026-10-26, at commit `6be302b` on the branch `iphone-crashes-2026-10-25`.  The parent plan's "The code today" and "Terms", and the plans of M3, M4 and M5 (`stream-from-storage-m3.md`, `-m4.md`, `-m5.md`, with their decision logs), apply.  These parts concern M6:

* `engine/src/player.rs`: `Player::with_config(song, params, stretcher, cfg)` makes all scratch buffers and warms the stretcher (RT-1).  `process` takes a pending seek (`SharedParams::take_seek`): while playing, it fades out over `crossfade` frames, then `jump_to`, then fades in.  When paused, it jumps at once.  `read_mixed(n)` finds the song frame of each of `n` input frames (`pos_buf`, with loop wraps through `dsp::wrap_loop`, and `old_buf` for the loop crossfade), then reads `stem.left[p]` and `stem.right[p]` frame by frame for each stem whose gain is not zero, and sums them into `in_l` and `in_r`.  Chipmunk mode reads two song frames for each output frame.  `playhead` maps the read position back through the stretcher's latency.
* `engine/src/song.rs`: `Samples` (`Arc<Vec<f32>>`), `Stem { name, left, right }`, `Song { sample_rate, frames, stems }`.
* `engine/src/slot.rs`: `PlayerSlot`, an `Arc<Mutex<Option<Player>>>`; the callback `render_interleaved` takes it with `try_lock` and renders silence when it is busy.  `engine/src/live.rs`: cpal's output; on the web, cpal calls the callback on the main thread (two buffers of 2,048 frames queued).
* `engine/src/params.rs`: `SharedParams` (atomics): `playing`, tempo, `chipmunk`, the loop (`loop_region`), `request_seek` and `take_seek`, `playhead`, and the callback's cost counters.
* `engine/tests/realtime.rs`: `the_callback_does_not_allocate_after_warm_up` runs a `Rig` (six stems of noise in a `PlayerSlot`) under a counting allocator through the phases start, tempo sweep, fast, chipmunk, seek, loop, short loop, mix and end, for each shipped stretcher, at 44.1 and 48 kHz, with callbacks of 128 to 10,000 frames.
* `app/src/lib.rs`: `load_song(song)` builds the lanes (`lane_columns` of each stem, and of `mix_of_sources` for the mix lane), the `SharedParams`, and a `Player` that `play_on_output` puts in the live output's slot (or `Output::Silent`, which `pump_silent` renders on the UI thread).  `open_song` is `load_song` for a new song.  `with_original(source, stems)` adds the "original" lane: the source's only lane in the mix (`original_lane`), or `mix_of_sources`.  `show_separated` loads the stems onto the shown song; `add_stem` adds classic drums; `load_demo` opens the synthetic song.  `log_audio_stats` logs an `audio stats:` line every 2 s while playing, which `xtask e2e` reads (`audio_stats`, `played_a_second`).
* The open flows of M5 (`open_chosen_now`, `decode_session`, `restore_stored`, the Storage menu's open) end in an `Opened` in the `Inbox`, which `apply_inbox` shows with `open_song`.  A complete session reads back with `store::read_back` (D10 of M5).  Stems come from `store::stems_of` and `store::read_stem` (each stem WAV read whole and decoded with `engine::decode::decode_bytes`).  A storage-path separation reads its stems with `read_stem` in `separation_stored`, and shows them through `stored_stems_arrived` and `show_separated`.
* `app/src/store.rs`: the session layout (its module doc), `forget_mix`, `mix_in_use`, `set_separating`, `forget`, `usable_mix_frames`, `session_folder`, `st_file` and `st_slice` (a `Blob` slice read as an `ArrayBuffer`).  `app/src/store_meta.rs`: `Meta`, `PlannedSource`, `one_source`, `source_file`, `planned_sources`.
* `forget_mix` removes the `mix` line and `mix.f32`.  It runs on `unstored` from the stem worker (`separation_unstored`, D6 of M4) and on `decode-unstored` with a kept mix (`decode_session`, D8 of M5, decision 1-23).  For a song of one source, `mix.f32` is the only session file of the source (D1 of M5).  The Storage menu's "Forget" removes a session; for the shown song it says "it stays open until another is loaded".  "Open" is not offered for the shown session.
* Session files (D1 of M5, D1 of M4, M3): `mix.f32` and `decoded/<i>.f32` are raw f32 little-endian, interleaved stereo, 44.1 kHz, padded to the song's frames, with both sides written for a mono source.  `stems/<name>.wav` is a 16-bit stereo PCM WAV at 44.1 kHz, written by the worker (M3) or by `save_stems` (memory path).  A stem file can be one frame shorter than the song (`read_stem` pads it).
* `app/src/streamtest.rs` (`?streamtest`, M1's spike): the `Ahead` class reads one second of all eight files at a time with one `Promise.all` of `Blob.slice(...).arrayBuffer()`.  M1 on the iPhone 17 Pro: reads of one second of eight files p50 22 to 23 ms, p99 26 ms (52 to 59 ms with a 30 ms block every 3 s), seeks p99 35 ms, no underruns.  Chrome on the Mac failed 2 of 29 reads of a file while a worker wrote it (`NotReadableError`), so a lane reads a file only once it is complete.
* `xtask/src/e2e.rs`: `audio-resume` drops a song, writes stem WAVs and `stem` lines into the session in the page (as `save_stems` does), reloads, and plays after one click, in CI's `browser-tests` job, with no GPU.

Facts from outside the repo, checked on 2026-10-26:

* A `File` from `FileSystemFileHandle.getFile()` is a snapshot.  In Chrome, a read of it after the file changes fails with `NotReadableError`; a new `getFile()` gives the new contents ([MDN getFile](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemFileHandle/getFile), and M1's evidence above).
* `std::sync::Mutex::try_lock` never blocks ([docs.rs](https://doc.rust-lang.org/std/sync/struct.Mutex.html#method.try_lock)).  On `wasm32-unknown-unknown` with no threads, it always succeeds when nothing holds the lock.
* Symphonia converts a 16-bit sample to `f32` as `s / 32768` (`symphonia-core` 0.6.1, `src/audio/conv.rs`: `impl_convert!(i16, f32, s, s as f32 / 32_768.0)`).  Division by a power of two is exact, so a conversion written the same way gives the same bits.  Step 2 proves it with a test over all 65,536 values, not by this note.

Terms, besides the parent's and those of M3 to M5:

* Lane: one stem of the shown `Song`, as the `Player` plays it.  (The mix lane is drawn only; the player does not play it.)
* Stored lane: a lane that the player reads from a session file through the read-ahead buffer.  Memory lane: a lane that it reads from `Samples`.
* Lane file: the session file of a stored lane, with its format: f32 stereo (`mix.f32`, `decoded/<i>.f32`) or 16-bit stereo WAV (`stems/<name>.wav`), its data offset and its frames.
* Block: `BLOCK_FRAMES` (32,768) frames of every stored lane, read together.  Block `b` holds frames `b x BLOCK_FRAMES` up to the next block.
* Slot: one block's room in the read-ahead buffer.  Reader: the async task on the main thread that fills slots.
* Wait: the player holds still in silence because the block at a position that it has not played from yet is not in the buffer (D6).  A wait is not an underrun.

## Plan of work

### How the team works

The team leader wrote this plan, has it reviewed, runs the review rounds and commits Progress.  One implementer (a `general-purpose` subagent) does steps 1 to 4 and the review fixes.  It starts no other agent.  Before each commit it runs the checks of GIT-2 (`CLAUDE.md`, Commands), commits signed (`git commit -S`) with the attribution lines that the team leader gives, and does not push.

Each commit leaves the docs true (DOC-1).  Write each test first and see it fail for the right reason, then write the code.  If code comes before its test, show with a mutation that the test fails, and record it in Progress.  A workaround or a known gap gets a `docs/tech-debt.md` entry in the same commit.  The implementer does not edit `CLAUDE.md` or `.claude/`; if a change there is needed, it sends the exact text to the team leader.

### Decisions of this milestone

* D1, the trait.  `engine::source::StemSource: Send + Sync` gives the frames of one lane:
  * `read(&self, from: u64, left: &mut [f32], right: &mut [f32])` fills both slices (the same length) with the frames from `from`.  Frames at or after the lane's end are 0.0.  Frames that the source does not hold now are 0.0, and the source counts an underrun (D5).  It never blocks, allocates or frees (RT-1).
  * `ready(&self, at: u64) -> bool`: whether `read` from `at` gives real frames now (D6).
  * `hint(&self, hint: Hint)` with `Hint::Reading(pos)` (the player's read position, once in each `process`) and `Hint::Seek(Option<u64>)` (a seek that waits, or none).  It stores atomics only.  The reader reads them (D4).
  * `MemorySource` holds two `Samples` (clones share the buffer, MEM-1).  `read` copies, `ready` is always true, `hint` does nothing.  `Song::memory_sources()` (or a function of that kind) makes one for each stem, with no copy; a test pins the sharing (`Samples::shares`).
  * Names may change if the implementer finds better ones; the Decision log records it.
* D2, the player.  `Player::with_sources(song, params, stretcher, cfg, sources: Vec<Arc<dyn StemSource>>)` takes one source for each stem of `song`, in order (it panics on another count, as `with_config` does for `params`).  `with_config` keeps its signature and uses the memory sources.  `read_mixed` keeps its arithmetic, bit for bit: it finds the song frame of each input frame as today, then, for each stem with a gain, reads each run of consecutive frames of `pos_buf` (and of `old_buf` for the loop crossfade) with one `read` into two scratch pairs made in `with_sources`, and sums `vl x fin[k] + ol x fout[k]` and `v x g` in today's order.  A loop shorter than a run makes several runs; that is correct, only slower.
  * Before the change, a test records the SHA-256 of the output of today's player through tempo, chipmunk, a loop with a crossfade, a short loop, a seek, mute and solo changes, at 44.1 kHz, with Varispeed and with wsola (`the_player_output_is_unchanged`, in its own commit).  The restructured player must give the same hashes.
* D3, the read-ahead buffer.  `engine::readahead::ReadAhead` is shared (`Arc`) by the stored sources of one song and the reader.
  * It holds `slots` slots.  Each slot is a `Mutex` around the block number it holds (or none) and one byte buffer for each stored lane, `BLOCK_FRAMES x bytes per frame` long: 8 for an f32 lane, 4 for a 16-bit lane.  The bytes are the file's bytes; the callback converts them (`f32::from_le_bytes`, or `i16::from_le_bytes` as `f32` divided by 32768.0).
  * `ReadAhead::new(lanes, frames)` makes every buffer at once, before the player is made, so the callback never allocates (RT-1).  Its size does not depend on the song's frames.  The slot counts: 16 slots ahead of the play head and 8 after the loop start (about 11.9 s and 5.9 s), fewer if the total would pass `MAX_READ_AHEAD_BYTES` (32 MiB), but at least 4 ahead and 2 for the loop.  Six 16-bit stems and an f32 original take 1 MiB a slot, so 24 MiB.  A song whose stored lanes need more than 32 MiB for the minimum (more than about 16 f32 lanes) plays those lanes from memory, and the Decision log records it if a test or e2e scenario meets it.
  * The callback finds a block by its number among the slots (at most 24 compares), takes the slot with `try_lock` only, and checks the number again under the lock.  A slot that it cannot take counts as not held.  It never waits.
  * `StoredSource` is one lane of a `ReadAhead`: `read` copies the lane's frames from the slots that hold them, block by block.
  * The reader side, in `engine`, so native tests can drive it: `next_read(&self) -> Option<BlockRead>` gives the most wanted block that no slot holds and a slot to put it in (D4), or none.  `fill(&self, read: BlockRead, put: impl FnOnce(&mut [&mut [u8]]))` locks that slot, lets the caller write the lane bytes, and publishes the block number last.  A lane's bytes after the end of its file are zero.  `fail_lane(lane)` makes a lane silent for good (D7).  No lock is held across an `await`: the app reads the bytes first, then calls `fill`.
* D4, which blocks the reader wants, in order (a pure function, `wanted_blocks`, with a native test):
  1. If a seek waits (`Hint::Seek(Some(t))`): the block of `t` and the next one.
  2. Ahead: from the block of the read position (`Hint::Reading`), up to the ahead count.  If a loop is set and the read position is in it, the run follows the loop: to the loop's end, then from its start.
  3. The loop: from the block of the loop's start, up to the loop count, not past the loop's end.
  * Blocks past the song's end are not wanted.  A block is listed once.  The reader puts a new block in an empty slot, or in a slot whose block is not wanted.  If every slot holds a wanted block, it reads nothing.  The loop comes from `SharedParams::loop_region`, which `ReadAhead` takes at `new`.
* D5, underruns.  A `read` of a run that the buffer does not hold, in full or in part, gives 0.0 for the missing frames, adds 1 to `underruns` and the missing frames to `underrun_frames` (atomics on `ReadAhead`).  A memory lane never underruns.  A muted lane is not read, so it does not count.
* D6, seeks and waits.
  * While playing, a seek to a position whose block is not `ready` does not fade or jump yet.  The player keeps playing from its position, and gives `Hint::Seek(Some(target))`.  Each `process` checks again.  When the block is ready, the seek fades out and jumps as today, and the hint is cleared.  A newer seek replaces the waiting one.  So the old position plays until the new block arrives (the parent's acceptance).
  * When paused, a seek jumps at once, as today.
  * After any jump (a seek, a chipmunk change, a new player), the player has not yet played from its position.  If it plays and that position is not ready, it waits: it renders silence, does not move, and adds the frames to `waits`.  It starts as soon as the block is ready.  A memory lane is always ready, so a song of memory lanes never waits.
  * A loop wrap is not a jump.  If the loop's start is not held at the wrap, the lane underruns (D5).
* D7, read failures.  If a read of a lane file fails, the reader gets the file again (`getFile`) and reads the block once more.  If that fails too, it calls `fail_lane`: the lane plays silence from then on, `failed_lanes` goes up, the page logs `playback: lane <name> failed: <error>` once, and the status line says "Could not read the <name> lane from browser storage; it plays silent."  The other lanes go on.  A new `getFile` after a move of the file (for example `keep_stored_stems` during a new separation of the shown song) reads the new file, and the song with the new stems is shown soon after; the Decision log records this.
* D8, which lanes are stored.  The page keeps, for the shown song, a map from lane name to lane file (`StemApp::lane_files`), from the session that the song opened from.  A lane is stored only if its file is complete when the song is shown: its size is the size of the song's frames (f32), or a WAV header of 16-bit stereo PCM at 44,100 Hz whose data chunk holds at least the song's frames less one (frames past the song's end are not read, and a missing last frame reads as 0.0, as `read_stem` pads).  Else it is a memory lane.  The map:
  * A source lane of a complete session (D2 of M5): its `source_file` (`mix.f32` for the source in the mix of a song of one source, else `decoded/<i>.f32`).
  * A stem in the `stem` lines of `meta`: `stems/<stem_file(name)>`.
  * The "original" lane: `mix.f32`, if the session has a usable mix of the song's frames (`usable_mix_frames`).  For a song of several sources, `mix.f32` is `mix_of_sources` bit for bit (D1 of M5), which `with_original` gives.
  * Classic drums made on the page, a lane added with `add_stem`, the demo song, a song decoded on the page and the stems of a memory-path separation are memory lanes.  M7 adds the classic-drums file to the map.
  * The names and formats come from a pure function in `app/src/store_meta.rs` with a native test.  The size checks and the WAV header read are in `app/src/store.rs` (the header parse is a pure function with a native test).
  * The open flows put the map in `Opened`; `stored_stems_arrived` adds the stems and "original" of a storage-path separation; `open_song` replaces the map, `load_demo` and the memory path clear it.  `load_song` makes the sources from it.
* D9, the reader on the page (`app/src/playback.rs`, web only).
  * `load_song` first drops the old player and stops the old reader (as it frees `mix_follower`), so two read-ahead buffers never live at once.  Then, if the song has a stored lane, it makes the `ReadAhead` and the sources, the player, and starts the reader with `spawn_local`.
  * The reader holds a `Weak<ReadAhead>`, and stops when it cannot upgrade it or when its song's serial is no longer shown.  It holds no strong reference across an `await`.  It gets each lane's `File` once (`getFile`) at its start.
  * Each turn: `next_read`; if there is a block, it reads that block's byte range of every stored lane with one `Promise.all` of `Blob.slice(...).arrayBuffer()` (as M1's spike does), then copies each `ArrayBuffer` into the slot with `Uint8Array::copy_to` inside `fill`.  It never reads more than one block of a lane at a time.  If there is no block to read, it waits 10 ms (`setTimeout`) and looks again.  So a seek starts a read within about 10 ms.
  * It records the time of each block read, and the page logs a `playback:` line every 2 s while playing, after the `audio stats:` line: `playback: lanes from storage S of T, read-ahead R MB, underruns U (F frames), waits W frames, failed lanes L, reads N, read p50 A ms, p99 B ms`.  `xtask e2e` parses it with a pure function.
* D10, no failure removes a file that playback reads.  `store::set_playing(Some(id))` (as `set_separating`) names the session whose files the shown song reads; `load_song` sets it, and clears it when no lane is stored.
  * `forget_mix` removes nothing for that session.  It checks inside its `meta` lock, logs `store: kept the mix of session <id>: playback reads it`, and returns `Ok`.  Both callers (`separation_unstored`, `decode_session`) keep their other behaviour.  So after `unstored` for the shown song, the mix stays usable, and the fallback of D4 of M3 goes on as today.
  * The Storage menu offers no "Forget" for that session, with the hover text "This song plays from browser storage; open another song first".
  * The parent's M6 bullet says "before playback reads `mix.f32`, no failure removes it".  This plan reads it as: a failure never removes a session file that the shown song plays from, and the rest of D8 of M5 and D6 of M4 holds.  The Decision log records it.

### Step 1: the trait and the player (engine)

* First commit: `the_player_output_is_unchanged` (D2), on today's code.
* D1 and D2: `engine/src/source.rs`, `Player::with_sources`, the run reads in `read_mixed`, the hints, the waiting seek and the wait of D6.  Module docs of `player.rs` and `source.rs`.
* Tests, native (`cargo test -p engine`):
  * `the_player_output_is_unchanged` passes with the new player.
  * `memory_sources_share_the_stems` (MEM-1).
  * `a_seek_waits_for_its_block_and_the_old_position_plays` and `a_paused_seek_jumps_at_once`, with a test source whose `ready` the test sets.
  * `a_jump_to_a_position_not_ready_waits_and_counts_no_underrun`.

### Step 2: the read-ahead buffer (engine)

* D3, D4 and D5: `engine/src/readahead.rs` with `ReadAhead`, `StoredSource`, `wanted_blocks`, the counters and the conversion of D3.
* RT-1: the callback path takes slots with `try_lock` only, and allocates nothing.  Update the module doc of `slot.rs` if it names what the callback touches.
* Tests, native.  A fake reader, in the test, fills slots from in-memory lanes between callbacks (or on another thread), with no allocation in the counted part:
  * `pcm16_converts_as_the_decoder_does`: a WAV of all 65,536 values, decoded by `engine::decode::decode_bytes`, equals the conversion of D3 bit for bit.
  * `a_stored_song_plays_as_the_memory_song`: six 16-bit lanes and one f32 lane, with a reader that keeps up, through tempo 0.5 to 1.5, chipmunk, a loop with a crossfade, a loop shorter than a block and one longer than the loop slots, mute and solo, and seeks to held and not-held positions.  The output equals a memory player's bit for bit, with Varispeed and with wsola, and `underruns` is 0.  (A seek in the stored run lands later than in the memory run; compare each part between jumps, or seek only to held positions in the equal part.)
  * `a_slow_reader_underruns_into_silence`: a reader that fills one block for each N callbacks.  Each output frame equals the memory output, or a missing lane gives 0.0 there, and `underruns` and `underrun_frames` equal what the test counts.
  * `a_stalled_reader_plays_silence_then_resumes`: no fills for 2 s, then fills.
  * `a_failed_lane_is_silent_and_the_others_play`: `fail_lane` on one lane; that lane gives 0.0, the others equal memory, `failed_lanes` is 1.
  * `after_a_seek_the_old_position_plays_until_the_new_block_arrives`: the reader holds back the target's block for 300 ms of callbacks.  Until then the output follows the old position (equal to memory from there); then it fades and plays the target.  `underruns` is 0.
  * `the_wanted_blocks_follow_the_head_the_loop_and_a_seek`: D4's order, with a loop that holds the head, one that does not, a seek, the song's end, and blocks listed once.
  * `the_read_ahead_has_a_fixed_size`: the bytes of a 1-minute and a 30-minute song are equal, at most `MAX_READ_AHEAD_BYTES`, with the slot counts of D3.
  * `a_reader_on_another_thread_never_blocks_the_callback`: a thread fills slots while the callback runs; the callback never waits (it finishes within a bound), and every frame equals memory or is counted silence.
* `engine/tests/realtime.rs`: the `Rig` also runs with the six stems as stored lanes over a `ReadAhead`.  The fake reader fills slots between callbacks, outside the counted part.  Every phase runs, and a seek to a position not yet read waits for its block.  `the_callback_does_not_allocate_after_warm_up` fails on any allocation or free in a callback, for each stretcher, rate and callback size.  The test makes one `ReadAhead` for each `Rig` before the first callback.

### Step 3: the page plays stored lanes

* D7 to D10: `app/src/playback.rs`, the lane map (`store_meta`, `store`), `StemApp::lane_files`, `load_song`, the open flows, `stored_stems_arrived`, `forget_mix`, the Storage menu, `set_playing` and the `playback:` line.  `.cargo/mutants.toml` excludes the new wasm32-only code, as M5 did.
* Tests, native: `the_lane_files_of_a_session` (D8's map, for a song of one source, of several, with a derived source, with stems, with "original"); `a_wav_header_gives_its_data_and_frames` (16-bit stereo at 44.1 kHz; refuses other formats; a stem one frame short); `the_playback_line_reads_back` (if the e2e parser is in `xtask`, there).
* Tests, browser (`cargo run -p xtask -- web-test`, Chrome), with planted session files in OPFS:
  * `a_stored_song_plays_as_the_memory_song_in_the_browser`: a song of 30 s of six 16-bit stem WAVs and an f32 `mix.f32` (the original).  The real reader fills from OPFS while a timer renders the `PlayerSlot` at real time (2,048 frames each 46 ms) for 8 s, through tempo 1 and 1.5, chipmunk and a loop.  The output equals a memory player's bit for bit, and `underruns` and `waits` after the first block are 0.
  * `a_seek_in_the_browser_plays_the_old_position_until_its_block_arrives`: a spy delays each `Blob.prototype.arrayBuffer` by 200 ms.  After a seek, the old position plays, then the target; `underruns` is 0.  The test logs the seek-to-sound time.
  * `a_lane_file_is_read_one_block_at_a_time`: a spy on `Blob.prototype.slice` and `arrayBuffer` sees no read larger than one block of one lane (256 KiB), and the read-ahead's bytes are the same for a 1-minute and a 10-minute session.
  * `a_removed_lane_file_goes_silent_and_says_so` (D7): one stem file removed during play; that lane gives silence, the others play, the status line says so, `failed_lanes` is 1.
  * `a_restored_session_plays_its_lanes_from_storage`: a complete session with stems restores with every lane stored ("original" too), and `set_playing` names it.  A song decoded on the page, the demo and classic drums are memory lanes.
  * `unstored_keeps_the_mix_that_playback_reads` (D10): after `unstored` for the shown song that plays from `mix.f32`, `mix.f32` and its `mix` line stay.  `decode_unstored_keeps_the_mix_that_playback_reads` likewise.  The tests of M4 and M5 that expect a removal (`unstored_removes_the_mix_line`, `unstored_with_a_kept_mix_removes_it`) keep a song that does not play from that session, or change with a reason in the Decision log.
  * `forget_is_not_offered_for_the_song_that_plays_from_storage` (the pure `forget_enabled` gets the playing session, with a native test).
  * `a_new_song_frees_the_old_read_ahead`: after a second `load_song`, the first `ReadAhead` has no strong reference (a `Weak` that does not upgrade), and its reader has stopped.

### Step 4: the e2e check and the docs

* A new scenario `e2e playback` (`xtask/src/e2e/playback.rs`), in the `browser-tests` job of `.github/workflows/ci.yml` (no GPU), with a pure function and a native test for each check of the log:
  * Drop the synthetic stems of `e2e smoke`, then write six stem WAVs and their `stem` lines into the session as `audio-resume` does, and reload.  The song restores with `playback: lanes from storage 7 of 7`.
  * Click Play, play at least 8 s, click a lane to seek, and play on.  The `playback:` lines say `underruns 0`, `failed lanes 0`, and a read-ahead of at most 32 MB.  The playhead passes the seek target.  The log has no page error.
  * `e2e smoke` also checks a `playback:` line with every lane from storage and `underruns 0`.
* Run in Chrome on the Mac, with `CHROMEDRIVER` and the weights of the team leader's prompt, after `(cd app && trunk build --release)`: `e2e playback`, `smoke`, `decode`, `audio-resume`, `storage`, `flac`, `separate`, `memory-budget`, and `web-test`.  Record the numbers in Progress: the `playback:` line of `e2e playback`, the seek-to-sound times of the browser test, and the peaks of `memory-budget` (the read-ahead adds at most 32 MB to the page's peak; the growth checks of M3 to M5 must still pass).
* Docs (in the commit that makes each one wrong, at the latest here): `docs/ARCHITECTURE.md` (playback: the trait, the read-ahead buffer, the reader, waits and underruns; memory on phones: the 32 MiB), RT-1's "Enforced by" in `docs/invariants.md` (the new tests; the rule's text does not change), TD-54 (playback reads storage; the read-back for the lanes stays until M7), the README's scenario list and the doc comment of `xtask/src/e2e.rs`, and `docs/references/browser-quirks.md` if a browser surprises.

## Validation and acceptance

The parent's M6 acceptance, check by check:

* "Engine tests play a storage-backed source with a fake reader that is slow, stalls or fails.  The output equals the memory source, or is silence with the underrun counted": `a_stored_song_plays_as_the_memory_song`, `a_slow_reader_underruns_into_silence`, `a_stalled_reader_plays_silence_then_resumes`, `a_failed_lane_is_silent_and_the_others_play` (CI `test`).
* "After a seek, the old position plays until the new block arrives": `after_a_seek_the_old_position_plays_until_the_new_block_arrives`, `a_seek_waits_for_its_block_and_the_old_position_plays` (CI `test`), and its browser test (CI `browser-tests`).
* "The allocation tests in `engine/tests/realtime.rs` cover the new source (RT-1)": the stored `Rig` in `the_callback_does_not_allocate_after_warm_up` (CI `test`).
* "A browser test plays a stored song and counts no underruns": `a_stored_song_plays_as_the_memory_song_in_the_browser` and `e2e playback` (CI `browser-tests`).
* "Before playback reads `mix.f32`, no failure removes it": `unstored_keeps_the_mix_that_playback_reads`, `decode_unstored_keeps_the_mix_that_playback_reads` and `forget_is_not_offered_for_the_song_that_plays_from_storage`.
* The memory path does not change: `the_player_output_is_unchanged`, the UI snapshot tests, and `e2e smoke`.
* Every check of GIT-2 passes, and `cargo run -p xtask -- web-test --browser chrome`.

## Idempotence and recovery

* Each step is one or more commits that pass every check.  Steps 1 and 2 change only `engine`; the app still plays memory lanes until step 3.  A step can be reverted alone, from the newest back.
* M6 writes no file and changes no session layout.  A session of M3, M4 or M5 plays as before; its lanes are stored where D8 finds a complete file.
* If the reader fails or storage goes away, the lanes play silence and the page says so (D7); a reload opens the song again.  To go back to memory playback, revert step 3: `load_song` then uses `with_config`.

## Progress

* [x] 2026-10-26: milestone plan written.
* [ ] Plan reviews.
* [ ] Step 1: the trait and the player.
* [ ] Step 2: the read-ahead buffer.
* [ ] Step 3: the page plays stored lanes.
* [ ] Step 4: the e2e check and the docs.
* [ ] Review rounds 1 to 3, and the review log.

## Surprises and discoveries

None yet.

## Decision log

* 2026-10-26, team leader: D3, the read-ahead buffer keeps each lane's file bytes (16-bit for a stem), not `f32`, so its size matches decision 0017's estimate (about 1 MiB a slot for six stems and the original).  The callback converts them, the same way as the decoder.
* 2026-10-26, team leader: D3 and D4, a set of slots with block numbers, not a ring.  So the frames after the loop start stay while the play head runs ahead, and a seek's block can go in any free slot.
* 2026-10-26, team leader: D6, a wait at a jump is not an underrun.  After a paused seek or a new song, the play head has not played the position yet, so silence there loses no audio.  An underrun is a gap in audio that plays on.
* 2026-10-26, team leader: D8, a lane plays from storage only if its file is complete when the song is shown.  Chrome failed reads of a file while a worker wrote it (M1).
* 2026-10-26, team leader: D10, the parent's "before playback reads `mix.f32`, no failure removes it" is read as: no failure removes a file that the shown song plays from.  A mix that no player reads still goes on `unstored`, as D6 of M4 and decision 1-23 of M5 say, so the next open decodes again.  "Forget" is not offered for the session of the shown song, so a user action does not remove it either.
* 2026-10-26, team leader: MEM-1's text does not change in M6.  The song's stems stay `Samples` in memory until M7 removes the read-back.  M7 rewords MEM-1 then (the maintainer allows it in the milestone that needs it).
* 2026-10-26, team leader: the trait reads with `&self` and atomics, so the stored sources of one song share one `ReadAhead` with the reader, and M8's save job can read through the same trait with a reader of its own.

## Outcomes and retrospective

Not started.
