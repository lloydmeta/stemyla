# GPU speed, M5.1: the leaks of browser data

## Purpose

This is the milestone plan for M5.1 of `plans/active/gpu-speed.md`.  The parent's M5.1 section, with its goal and its 6 cases, stays the criteria.

Goal (the parent): no test run and no use of the app leaves browser data behind that grows with the number of runs.  On 2026-11-14, test runs had left about 352 GB in `$TMPDIR`: Chrome profiles, and Safari's write-stream swap files and blob folders.

M5.1 has three parts:

1. Chrome: each `e2e` scenario and each `web-test` run leaves no Chrome profile behind (case 1).
2. Safari: find, by counted runs, why Safari keeps `FileSystemWritableStream*` files and `BlobRegistryFiles-*` folders (cases 2 and 5).  Then fix the cause, or record a WebKit bug and take the cheapest workaround that the data supports (cases 3 and 4).
3. `xtask` tells the person how much browser data a run left (case 6).

Readers: the implementer, the team leader, the orchestrator, and the team leader of M6.

## Context and orientation

The state at the start, on 2026-11-14, at commit `8b16df7` on branch `gpu-speed/2026-11-12`.  The review base is `8b16df7`.  The machine is the maintainer's Apple M2 Pro, with Safari 27.2 and Chrome 157.

### The decisions that M5.1 follows

* The maintainer, 2026-11-14: fix both leaks before M6.  The orchestrator deleted the old files and quit Safari.  So the two folders below start empty.
* The orchestrator, 2026-11-14:
  * Count and size only the two named paths below.  Never search or `du` all of `$TMPDIR`, or a wider path.  Other apps' folders in `$TMPDIR` are private.
  * Never delete a folder or file that a run of yours did not make.
  * Never quit or kill the maintainer's own Safari or Chrome.  The automation window of `safaridriver` is yours.
  * Each claim about a cause comes from a run that counts the files in the folder before and after the run.
  * Proportion (`CLAUDE.md`): the simplest fix that the data supports comes first.  A new module, crate or product change needs the maintainer's choice.  The team leader asks the orchestrator.
* Lessons of M1 to M5: a build in a git worktree uses its own `--target-dir` in that worktree.  One browser run or perf gate at a time.  A Safari run that loses its session runs once more; then stop and report.  Put each choice of an I/O function in a pure function with a test.

### The paths of this run

* `$TMPDIR` is `/var/folders/qr/1phzsw110tjdbfjhxbx6qmrh0000gn/T` on this Mac.  `getconf DARWIN_USER_TEMP_DIR` gives the same folder.
* Chrome's leaked profiles: `$TMPDIR/org.chromium.Chromium.scoped_dir.*`.  Count them with `ls -d "$TMPDIR"/org.chromium.Chromium.scoped_dir.* 2>/dev/null | wc -l`.  Do not use another glob.
* Safari's folder: `$TMPDIR/com.apple.WebKit.Networking+com.apple.Safari/`.  Count its `FileSystemWritableStream*` files and its `BlobRegistryFiles-*` folders, and give their bytes (`du -sk` of that folder only).
* Weights: `/Users/lloyd/Documents/rustt/stemyla/third_party/models/bs_roformer_sw.f16.safetensors` (349,521,256 bytes; never commit it, ASSET-1).  `STEMYLA_PERF_WEIGHTS` names it.  `CHROMEDRIVER` is `/Users/lloyd/.local/chromedriver-157/chromedriver-mac-arm64/chromedriver`.
* Scratch files and logs: `/private/tmp/claude-501/-Users-lloyd-Documents-rustt-stemyla/dfc742e1-85d0-46cd-9553-21e0d5ad2fcf/scratchpad/m51/`.

### What is known

Chrome:

* `xtask/src/webdriver.rs`: `Session`'s drop sends `DELETE /session`.  Then `Driver`'s drop kills `chromedriver` at once (`Child::kill`, SIGKILL on Unix).  `e2e` starts one `Driver` and one `Session` for each page (`Page::open` in `xtask/src/e2e.rs`).
* `web-test` does not start the driver.  `wasm-bindgen-test-runner` starts `chromedriver` from the `CHROMEDRIVER` variable, and reads Chrome's arguments from `WASM_BINDGEN_TEST_WEBDRIVER_JSON` (`chrome_capabilities` in `xtask/src/web_test.rs`).  One crate run runs several test binaries, each with its own session.
* `chromedriver` makes a temporary profile (`scoped_dir`) only when no `--user-data-dir` is given ([Chromium issue 42321600](https://issues.chromium.org/issues/42321600)).  Leftover `scoped_dir` folders are a known, long-lived problem ([Selenium issue 1934](https://groups.google.com/g/selenium-developer-activity/c/5hnhRZQPFaU)).  A process that is killed runs no cleanup.
* On macOS, Chromium's `base::GetTempDir` reads `MAC_CHROMIUM_TMPDIR` first, and else uses `NSTemporaryDirectory()`, not `TMPDIR` ([`file_util_apple.mm`, Chromium 139](https://sources.debian.org/src/chromium/139.0.7258.127-1/base/files/file_util_apple.mm); [the review that made the change](https://codereview.chromium.org/2770753006/diff/1/base/files/file_util_mac.mm)).  On Linux it reads `TMPDIR`.  The comment there says that a long temp path breaks the names of Unix sockets.  A socket path on macOS has at most 104 bytes (`sun_path`).

Safari (WebKit source at `c526785513`, the last change of the file on 2026-10-13):

* `FileSystemStorageHandle::createWritable` (`Source/WebKit/NetworkProcess/storage/FileSystemStorageHandle.cpp`) makes a swap file with `FileSystem::createTemporaryFile("FileSystemWritableStream")`.  It is in the temp folder of the network process: the Safari folder above.
* `closeWritable` with the reason `Aborted` deletes the swap file.  With the reason `Completed` (a `close()`), it copies the swap file onto the stored file (`FileSystem::copyFile`), and does not delete the swap file.  `FileHandleWithPath` (`FileSystemStorageHandle.h`) is a plain struct of a handle and a path, with no destructor that deletes the file.
* `FileSystemStorageHandle::close()` closes each open writable with the reason `Aborted`, so a stream that is open when its handle closes loses its swap file.
* So, by the source, each `close()` of a `createWritable` stream leaves a swap file of the written size.  The 349,521,256-byte files are the model saves of the stem worker: `opfs_write` (a model that the page gives, as in `e2e weights --weights`) and `download_to_opfs` (a model that the worker downloads to `download.part`, the users' path, as in `e2e weights --download`), both in `worker/src/web.rs`.  A counted run must confirm this (case 2).
* `NetworkSessionCocoa`'s constructor (`Source/WebKit/NetworkProcess/cocoa/NetworkSessionCocoa.mm`) makes one `BlobRegistryFiles` temporary folder for each network session that is not ephemeral.  No code found deletes it.  A counted run must confirm what makes a new folder, and what fills it (case 5).
* `createSyncAccessHandle` writes the stored file in place, with no swap file in the source above.  It works only in a dedicated worker.

The app's writes to browser storage:

* The stem worker: `opfs_write` and `download_to_opfs` (the model cache, through `createWritable`), and sync access handles for stems and encodes (`worker/src/web/stems.rs`, `worker/src/web/encode.rs`).
* The decode worker: sync access handles (`decoder/src/web.rs`).
* The page (`app/src/store.rs`): `st_write` (`write_within`: the source files of `save_sources`, `meta`, the stems of the memory path in `save_stems`, overviews) and `st_open`, `st_put`, `st_close` (`write_mix`, and the save files of `save_open`).  All use `createWritable`, on the main thread, where a sync access handle is not available.  Some of these are song-sized: the mix file is 32-bit float stereo, about 85 MB for 4 minutes.
* Test code: `xtask/e2e/worker.js`, the scripts of `xtask/src/e2e.rs` and `xtask/src/e2e/`, and the test helpers of `app`, `worker` and `decoder` also use `createWritable`.
* Object URLs: `save_blob` in `app/src/store.rs` revokes its URL after 60 s.  `app/src/streamtest.rs` revokes its worker URL.  The test helpers `blob_url` and `revoke` are in `worker/src/web/tests.rs`.

### Terms

* Swap file: a `FileSystemWritableStream*` file in the Safari folder.
* Blob folder: a `BlobRegistryFiles-*` folder in the Safari folder.
* A counted run: a run with the count and the bytes of the swap files and blob folders (or of the Chrome profiles) just before it starts and just after it ends.  It also gives the count after the driver has ended, and again after 5 minutes.  Each claim about a cause cites the counted runs that show it, with their logs in the scratch folder.
* The run's Chrome folder: the folder that `xtask` makes for the temporary files of one driver process (Step 1).

## Plan of work

The simplest fix comes first.  Step 1 and Step 2 need no choice of the maintainer.  Step 4 is the cheapest workaround for the largest leak, in an existing function.  Step 5 is the decision point for the page's writes.

### Not done, and why

* `xtask` never deletes a swap file or a blob folder.  Safari makes them, and the maintainer's own Safari can make files in the same folder at the same time.  So `xtask` cannot prove which files are its own (the orchestrator's rule).
* `xtask` does not quit Safari to make it clean up.  It is the maintainer's browser.
* No sweep of `$TMPDIR/org.chromium.Chromium.scoped_dir.*` by name: a folder of that name can belong to the maintainer's own Chrome or another tool.
* No graceful stop of `chromedriver` (a SIGTERM and a wait, not a SIGKILL) as the fix.  In `web-test`, `wasm-bindgen-test-runner` starts and stops `chromedriver`, so `xtask` cannot change how it stops.  Also, a graceful stop does not cover a run that a signal ends.  The folder of Step 1 covers both tools.
* No `xtask` listing of `$TMPDIR` for Chrome profiles.  After Step 1, each run cleans its own, as case 6 allows.  The counted runs count the profiles by hand.

### Step 1: each Chrome run removes its own profile (case 1)

The design, simplest first:

* `xtask` makes the run's Chrome folder for each driver process: in `e2e`, in `Driver::start` for Chrome; in `web-test`, for each crate run.  On macOS only, it sets `MAC_CHROMIUM_TMPDIR` to that folder in the environment of the driver process.  Cargo and rustc do not read that variable.  In `web-test`, the variable goes on the `cargo test` command, and the runner and `chromedriver` inherit it.  So `chromedriver` makes each session's `scoped_dir` profile in the run's Chrome folder, and each session still starts with a new profile.  On other systems (CI's Linux runners) nothing changes: the leak was seen on macOS, and a CI runner's disk does not stay.
* The folders are in one fixed parent that only `xtask` uses: `/tmp/stemyla-chrome/`, with one folder `<pid>-<n>` for each driver process.  The path is short for Chrome's sockets.  Chrome makes its `SingletonSocket` in a folder in its temp folder, and a socket path on macOS has at most 104 bytes.  `/tmp/stemyla-chrome/<pid>-<n>/` is about 27 bytes.  A folder of the bundle ID, such as `.com.google.Chrome.XXXXXX/SingletonSocket` (42 bytes) or `.com.google.chrome.for.testing.XXXXXX/SingletonSocket` (54 bytes), keeps the path under 104.  A folder in `$TMPDIR` (49 bytes) would leave too little.  A pure function gives the path, with a test of its length.  The acceptance run measures the real socket path, and Surprises records it.
* The folder belongs to the driver.  In `e2e`, `Driver::start` makes it for Chrome, and `Drop for Driver` removes it after the kill and the wait.  So a failed session start in `start_session`, which drops the `Driver` alone, also removes it.  `Page` drops its session before its driver, so the order is `DELETE /session`, the kill, the wait, then the removal.  In `web-test`, a guard of the crate run removes it after the run.  The removal tries again for a short, bounded time, since Chrome's processes can still close files.  It also runs on an error, and on the scenario's time limit, which returns an error through `?` (`Page` in `xtask/src/e2e.rs`).  A removal that still fails prints its path and the error.  It does not fail the run.
* A run that a signal kills runs no drop.  So at the start, `xtask` lists `/tmp/stemyla-chrome/` only, and removes each folder there whose `xtask` process has ended.  It reads no other folder.  It keeps a folder while the command line of any process names the folder's path (`drivers::processes()`), since an orphan Chrome can still write to it.  It removes only a folder that the current user owns, and does not follow a symbolic link.  The parent folder is a parameter, so the unit tests use a folder of their own, never the real `/tmp/stemyla-chrome/`.
* Where the code goes, in existing files: the folder, `Drop for Driver` and the start sweep in `xtask/src/webdriver.rs`; the crate run's guard in `xtask/src/web_test.rs`.  No new module.
* Alternative, not taken first: `--user-data-dir` in Chrome's arguments.  In `e2e` it works for each session.  In `web-test`, one capabilities file serves each test binary of a crate run, so the binaries would share one profile and its browser storage.  That changes what the tests see.

Tests that fail first, on an assert (`xtask` unit tests):

* The Chrome driver's command gets the variable with the run's Chrome folder.  The Safari driver's command does not get it.
* `web-test`'s `cargo test` command for Chrome gets the variable.
* The guard removes its folder on drop, with a file in it, also when the code returns an error.
* A failed session start (a fake driver that refuses `POST /session`) removes the folder.
* A removal that fails gives a line with the path.
* The start sweep removes a folder of an ended process.  It keeps a folder of a live process, a folder that a process's command line names, a folder of another user, a symbolic link, and each name that is not `<pid>-<n>`.
* The path function gives a path whose socket path fits in 104 bytes.

Acceptance (counted runs in Chrome):

* `e2e storage`, `e2e weights` (with the weights) and one run that fails after its session starts each leave 0 new `org.chromium.Chromium.scoped_dir.*` in `$TMPDIR`, and no run's Chrome folder.  `e2e` has no option for a short time limit, so the implementer names how the failed run fails, and records it.
* A run that a signal ends (`kill -INT` of the implementer's own `xtask` process) leaves its folder.  Its `chromedriver` and Chrome can stay as orphans.  End only the processes whose arguments name that folder, by PID, never by name.  Then the next run's start sweep removes the folder.
* `web-test --browser chrome` leaves 0 new profiles and no run's Chrome folder.
* During one run, a `scoped_dir` profile is in the run's Chrome folder.  This shows that Chrome reads the variable.  The run also records the real path of `SingletonSocket` and its length.

### Step 2: `xtask` reports what a run left (case 6)

* Safari only.  Chrome cleans its own after Step 1 (see "Not done, and why").
* Where the code goes: the listing and the diff in `xtask/src/drivers.rs`, which holds the other Safari code.  No new module.
* One pure function takes two listings of the Safari folder (the names, the kinds and the sizes, before and after).  It gives the new swap files and blob folders, with their count and bytes.
* `e2e` and `web-test` in Safari list the folder at the start and at the end.  They print one line, such as `browser data: Safari left 12 swap files (350.1 MB) and 1 blob folder (0.0 MB) in <path>; xtask does not remove them (TD-<n>)`.
* The line does not fail the run.  The maintainer's own browser can add files in the same time.
* The listing reads the named folder only, with no recursion outside it, and gives the size of each blob folder.  If the folder does not exist, the count is 0.
* Tests that fail first: the diff of two listings, with a new file, a removed file, a grown file and a new folder.  The text of the line.
* Acceptance: the counted runs of Step 3 give the same counts as the line.

### Step 3: counted runs in Safari (cases 2 and 5)

Run each, one at a time, with the counts of Step 2 and a manual count of the same folder, before and after.  Before each run, record whether the maintainer's own Safari runs (`pgrep -x Safari`): its windows share the network process and its folder.  Record the table in Surprises, with each log.

| Run | What it shows |
| - | - |
| `web-test --browser safari` with a filter for one test that writes with `st_write` and closes | swap files after a `close()`: expected one for each closed stream, of the written size |
| A write that fails mid-way.  `st_write` (`app/src/store.rs`) and `write_file` (`worker/src/web.rs`) call `abort()` on an error, so the abort row below covers it.  If Step 3's code check finds a path that does not, add a row for it | the same as the abort row |
| The same, for one test that opens a writable, writes, then calls `abort()`.  `refuse_next_writable` does not serve: it refuses in JavaScript before WebKit makes a swap file.  If no such test exists, the implementer adds one | an abort leaves no swap file |
| `e2e weights` in Safari, with `--weights` | the model save through `opfs_write`: expected one swap file of 349,521,256 bytes |
| `e2e worker-check --download` in Safari | the model save through `download_to_opfs`, the users' path: the same |
| `e2e storage` in Safari | the page's song writes: the count and the bytes |
| The counts after the driver ends, and 5 minutes after | if Safari deletes swap files later |
| Blob folders in each run above | which run makes a new blob folder, and what is in it |

Also check the app's code for a stream with no `close()` and no `abort()`, and for each object URL with no `revokeObjectURL`.  The parent's case 3 applies if a counted run ties a leak to such a path (Step 3a).  A leak that the WebKit source explains is case 4 (Steps 4 to 6).

### Step 3a: a cause in the app (case 3, only if Step 3 finds one)

* Fix each path of the app that Step 3 ties to a leak, such as a stream with no `close()` or `abort()`, or an object URL with no `revokeObjectURL`.
* A test fails first, on an assert: a spy on the stream or on `URL` counts the opens and the ends.
* Acceptance (the parent's case 3): a counted Safari run (`e2e weights` or `perf`) leaves no new swap file and no new blob folder from that path.

Write the result in Surprises: the cause, with the runs that show it, and the bytes for each use of the app in Safari: one model save, one song open, one save.

### Step 4: the model saves use a sync access handle (case 4)

The cheapest workaround for the largest leak, if Step 3 confirms that `close()` leaves the swap file.  Its acceptance is narrower than the parent's case 3, since WebKit, not the app, makes the leak:

* `opfs_write` and `download_to_opfs` in `worker/src/web.rs` write with `createSyncAccessHandle`, as `worker/src/web/stems.rs` and `worker/src/web/encode.rs` do.  It reads a `Blob` source in slices of 8 MiB, and writes whole arrays (MEM-5).  No copy of the whole model (MEM-1).  It keeps the `.part` file and the move of `cache_put`, so a failed write leaves no cache entry.  It closes the handle on each path, also on a failure.
* Each other `createWritable` in the production code of `worker/` and `decoder/` moves to a sync access handle too.
* Tests that fail first: `worker` web tests spy on `FileSystemFileHandle.prototype.createWritable`, call `opfs_write` and `download_to_opfs`, and assert no call.
* The tests of MEM-5 for these writes move with them.  `opfs_write_writes_more_than_one_slice` and `a_download_holds_only_the_bytes_of_its_chunks` (`worker/src/web/tests.rs`) spy on `FileSystemWritableFileStream.prototype.write` (`start_spy`).  They spy on `FileSystemSyncAccessHandle.prototype.write` instead, as the tests of a refused `createSyncAccessHandle` do.  A `Blob` source then expects only whole arrays.  MEM-5's "Enforced by" line in `docs/invariants.md` names the changed tests.  The other tests of `opfs_write` and `cache_put` still pass, with a read-back of the written bytes.
* Acceptance: a counted `e2e weights --weights` run (`opfs_write`) and a counted `e2e worker-check --download` run (`download_to_opfs`; `e2e weights` does not read `--download`) in Safari leave no swap file of 349,521,256 bytes.  Both scenarios run in Chrome and Safari.  On this Mac, `e2e weights` fails one check, "part 1: the f32 stems have M3's six hashes", also before Step 4 (Surprises, c2 and s3).  Each of its other checks passes.  `check --tier push` passes, with both perf gates (the worker's paths change).

### Step 5: the page's writes (decided: c))

The maintainer chose option c) on 2026-11-14 ("Decisions for the maintainer", item 1): a write worker on the page, for this one purpose, writes the page's song-sized files with sync access handles.  The maintainer then chose its form: a Rust crate built by Trunk (parent Decision log, `00668a1`).  The decode worker keeps its duties (decision 0019).  Steps 5a to 5e do it.  The small writes stay on `createWritable`, with TD-103: `createWritable` replaces a file in one step on `close()`, and the page reads `meta` with no lock.

The routed writes (the "song-sized writes"):

| Call site | What it writes | Through today |
| - | - | - |
| `save_sources` (`app/src/store.rs`) | each source file of a song (a dropped `File` or `Blob`) | `write_within`, `st_write` |
| `write_mix` (`app/src/store.rs`) | the session's 32-bit float mix, in blocks, to `mix.f32.part`, then a move | `st_open`, `st_put`, `st_close` |
| `save_stems` (`app/src/store.rs`) | the stems of the memory path, to `stems/<name>.wav` | `write_within`, `st_write` |
| `save_open`, `save_put`, `save_close`, `save_abort` (`app/src/store.rs`), through `StoredWav` (`app/src/export.rs`) | a save file, in blocks of 2 MiB | `st_open`, `st_put`, `st_close`, `st_abort` |

Each other write of the page stays on `createWritable`: `meta`, `current`, overviews, and each other `st_write`.

### Step 5a: the facts before the code

The implementer writes these in Surprises before any code of Step 5b:

* For each call site of the table: its callers, and what each caller does after a failed write.  Known: `save_sources` removes the session folder, `write_mix` removes its `.part` file, and `save_open` and `save_abort` remove the save file.
* The readers of each routed file, and whether one can read the file while it is written.  A sync access handle is exclusive, and a read during a write can fail: the `?streamtest` "concurrent" run accepts failed reads, and Chrome failed 2 of 29 reads with `NotReadableError` (`plans/completed/stream-from-storage.md`).  If a reader can overlap a write, the plan gets a fix before Step 5c.
* The arrays: no routed call site passes a view into WebAssembly memory today (`Uint8Array::from` copies).  The one reused array is `StoredWav`'s `self.array`, which carries each 2 MiB block of a save (`a_wav_save_of_a_stored_mix_writes_whole_blocks_through_one_array`, D3 of M8).  The implementer confirms both.
* The writes that stay, with their sizes for a 4-minute song.
* The cost of opening and closing a sync access handle for each block, measured on the mix write of a 4-minute song in Chrome and Safari.  It chooses the form of `put` in Step 5b.

### Step 5b: the write worker crate

The maintainer chose the form on 2026-11-14 (parent Decision log, `00668a1`): a Rust crate built by Trunk, as `decode-worker` is.  So the messages go in `stem-protocol`, and PROTO-1's text does not change.  No decision record is needed beyond the parent's Decision log.

The crate:

* `write-worker`, in a new folder `writer/`, a page-side Web Worker.  Its normal dependencies: `stem-protocol`, and `wasm-bindgen`, `js-sys`, `web-sys` and `wasm-bindgen-futures` at the versions that `decode-worker` takes (DEPS-1: no new crate from a registry, so `THIRD-PARTY.md` gets only the new package in its `cargo tree` commands).  The calls to browser storage are inline JavaScript in Rust strings, as in `worker/src/web.rs`.
* Its layout is that of `decoder/`: a thin `lib.rs` and `main.rs`, and all browser code in `writer/src/web.rs` and `writer/src/web/**` behind `#[cfg(target_arch = "wasm32")]` (`decoder/src/lib.rs`).
* `writer/clippy.toml` holds the MEM-1 lints, as `decoder/clippy.toml` does.  The crate goes in `each_shipped_crate_lints_mem1_copies` if that test lists the crates.
* `app/index.html` gets a Trunk link for it, as for `decode-worker`.  The page starts the worker at the first routed write (lazily), not at load: `Worker::new` with the loader's URL, as `app/src/decoder.rs` does.  If that is not simple, the implementer measures the start-up cost and records it.
* After it starts, the worker sends a ready message, as the decode worker sends `DecodeReply::Alive` (`decoder/src/web.rs`).  The client holds each request until that message comes, as `app/src/decoder/client.rs` does (`post`).  Else a request posted before the worker's handler is set is lost, and `WRITES` never goes back to 0.
* An explicit queue in the worker runs one request at a time, in the order of arrival.  It catches the error of each operation and sends its reply, so no request waits for ever.

The operations, each with one reply (`ok`, the error's name, or `gone`):

* `write`: `within`, `rel` and a body.  It keeps `st_write`'s rule: if the folder `within` is gone, the reply is `gone`, and no folder is made again.  It makes the file, opens a sync access handle, truncates the file to 0, writes the body, flushes and closes.  A `Blob` body is read in slices of 8 MiB (`slice` and `arrayBuffer`), so the worker holds at most one slice of it.  A typed array body arrives by transfer, whole.
* The block writes of the mix and the save files.  The simpler form comes first: a `write` with an offset `at`, which opens the handle, truncates the file to 0 when `at` is 0, writes, flushes and closes, for each block.  It needs no stream ID, and leaves no handle open.  If Step 5a's cost is more than 5 % of the mix write's time in either browser, the form is `open` (which truncates to 0), `put`, `close` and `abort`, with a stream ID.  Record the choice in the Decision log.
* The reply of each block write, `ok` or an error, gives its array back, by transfer.  The array moves from `StoredWav` into its `Shared` state, so the task of `send` (`app/src/export.rs`) can keep the array that comes back.  If the worker fails, the detached array is dropped, and the save fails.  D3 of M8 becomes: "one buffer for all full blocks; the page makes no new one".  A transfer makes a new JavaScript object on each side, so the tests assert this with a count of new arrays or the buffer's `byteLength`, not with object identity.
* On a failed write, the worker closes the handle.  The visible state after a failure is the same as with `createWritable` today for each call site, as the callers clean up (Step 5a).
* `save_stems` writes each stem to `stems/<name>.wav.page.part`, then moves it to `stems/<name>.wav` (as `keep_stored_now` does), since a sync access handle truncates the old file first.  Not `.wav.part`: the module doc of `app/src/store.rs` gives that name to the stem worker's storage path, and a second separation can overlap the first one's save (`SAVING`).  The module doc names the new suffix, as it names `.decode.part`.
* Each array that the worker writes is whole (MEM-5): a transferred whole array, or a slice that `arrayBuffer` gives.

The messages (PROTO-1, MEM-2): a module `stem_protocol::write`, as `stem_protocol::decode`, with one encoder and one decoder for each request and reply (the ready message included), and the transfer list of each.  Each kind starts with `write-`, so the test that keeps the workers' messages apart can tell them from the others.  Each typed array body goes in the transfer list.  `xtask e2e` writes no message of the write worker.

The client on the page: `app/src/writer.rs` (and a `client` module if it grows, as `app/src/decoder/client.rs`).

* If the worker fails (`onerror`, `onmessageerror`), the client ends it (`terminate()`), so it holds no sync access handle, and fails each request that waits, with an error.  So `WRITES` goes down, a reset does not wait for ever, and a caller's `st_remove` and `wipe_opfs` can remove the file.  The next routed write starts a new worker.
* The app's web tests run with no build of the workers (the test runner serves only the test module).  Both the unit tests and the integration tests make routed writes: `app/tests/web.rs` and `app/tests/web_close.rs` call `store::save_sources` and `store::save_stems`, and an integration test links the library built without `cfg(test)`.  So the app's tests use a fake write worker, behind a new non-default feature of `app`, `fake-writer`:
  * The client is one per thread (a `thread_local`), which the routed functions of `app/src/store.rs` use.  It posts to a port: a `web_sys::Worker` in the shipped page, or any JavaScript object with `postMessage` and `onmessage`.
  * With `cfg(any(test, feature = "fake-writer"))`, the default port is the fake.  `xtask web-test` turns the feature on for the crate run of `app` (`xtask/src/web_test.rs`, with a test).  The shipped build (Trunk, `xtask publish`) does not turn it on, and a test pins that.
  * The fake is a page-side object.  It sends the ready message, takes each request through a structured clone with its transfers, serves it with `createWritable` as the page does today, and replies the same way, with the block's array given back.  For a whole `write` it calls `st_write`.  For the per-block form it calls, for each block, `createWritable({ keepExistingData: at > 0 })`, `seek(at)`, `write` and `close`; for the stream form, `st_open`, `st_put`, `st_close` and `st_abort`.  So the app's existing tests, their spies (`start_spy`) and their refusals (`refuse_next_writable`) keep their meaning.  The tests whose count of writable streams or whose refusal timing changes with the per-block form are named in the commit, with their new expected records.
  * The fake's JavaScript, and the functions that only it calls after Step 5c, are compiled only with `cfg(any(test, feature = "fake-writer"))`, so the shipped page carries no dead JavaScript.
  * A test that swaps the port (the routing tests below use a real `Worker` with no script, and turn off its error handler, as `Fake` in `app/src/decode_tests.rs` does) holds a guard that puts the fake back on drop.  So a later test in the same thread does not wait for a ready message that never comes.

Tests, each failing first on an assert:

* `protocol/tests/web.rs`: each write request and reply arrives intact through a structured clone, with its typed array in the transfer list.  `a_message_for_one_worker_is_not_read_as_one_for_the_other` covers the three workers.
* `write-worker`'s own web tests, in a dedicated worker (`run_in_dedicated_worker`, as `decoder/src/web/tests.rs`):
  * a read-back of a `write`, from a typed array and from a `Blob` of more than 8 MiB;
  * a spy on `FileSystemSyncAccessHandle.prototype.write` shows that each written array is whole (MEM-5), also for the `Blob` slices;
  * a block write at an offset, and a first block that truncates an old, longer file;
  * `write` into a gone folder gives `gone` and makes no folder;
  * a refused `createSyncAccessHandle` (as `worker/src/web/tests.rs` plants one) gives an error reply and leaves no open handle;
  * two requests to one file sent at once are done in order.  It runs first with a plain `async` handler, where the second fails with `NoModificationAllowedError`.
* `protocol/tests/web.rs` also: the ready message arrives intact.
* `app` web tests:
  * a request posted before the ready message is held, then sent after it;
  * a client with a worker that has no script and a spy on its posts (as `Fake` in `app/src/decode_tests.rs`): each routed call site posts a write request, and calls no `createWritable`; a small write still calls `createWritable`;
  * each posted typed array is detached after the post, and a block write's reply gives the array back whole;
  * a worker that fails (a planted `onerror`) fails each waiting request, `WRITES` goes back to 0, and the next write starts a new worker;
  * D3: a save of several blocks makes one buffer.
* The tests that change are named in the commit, with why: `a_wav_save_of_a_stored_mix_writes_whole_blocks_through_one_array`, `sources_are_saved_from_their_files`, and each other.

The docs and the harness that the new crate touches, in the commit that makes each one wrong (DOC-1):

* `CLAUDE.md`: the layout gets `writer/`; `protocol/` holds the messages between the page and its three workers; the clippy line of the web code gets `-p write-worker`.  `.claude/rules/memory.md` and `.claude/rules/protocol.md`: their `paths:` get `writer/**`, `writer/src/**` and `app/src/writer*`, and their lists of crates that clippy checks get `write-worker` (also `.claude/rules/gpu.md`).  The team leader gives these to the orchestrator as a step checkpoint diff before that commit, and the commit adds them by name.
* The root `Cargo.toml` (`members`) and `Cargo.lock`.
* `README.md`: the layout table and the clippy lines.  `THIRD-PARTY.md`: the `cargo tree` commands.  `.github/workflows/ci.yml`: the clippy line of the web code, and the comment on the build ("and the decode-worker, the same build xtask publish").  `app/Trunk.toml`: the comment that lists the loader files.
* MEM-1: `mem1_lint_problems` (`xtask/src/rules.rs`) checks each workspace member for `slice::to_vec` with no change.  The lints on JavaScript arrays apply only to the crates of `MEM1_JS_CRATES`: add `writer`, and the canary function `mem1_lint_is_on_for_js_arrays` to the new crate.  Update the two crate lists of MEM-1's "Enforced by" text in `docs/invariants.md`, TD-26 in `docs/tech-debt.md`, and "the first four" in `.claude/rules/memory.md`, `.claude/rules/protocol.md` and `.claude/rules/gpu.md` (in the step checkpoint diff).
* `xtask/src/rules.rs`: `WEB_APP`, `PAGE_SIDE` and `manifest_of`, with their tests `the_two_sides_are_the_web_app` and `each_package_names_its_manifest`; `third_party_args` and its test, since `third_party_lists_what_ships` compares it with `THIRD-PARTY.md`.
* `.cargo/mutants.toml`: `exclude_globs` gets `writer/src/web.rs` and `writer/src/web/**`, and each wasm32-only file of the client in `app/src/`, each by its whole name, as for the decode worker's files.
* `docs/ARCHITECTURE.md`: the first section (the page's parts), the crate list, the layers table (a Shells row: `write-worker` takes `stem-protocol`), the web app's crates, the diagram of the page's parts, and the description of `web-test`.
* `docs/invariants.md`: DEPS-1's "the wasm dependency tree of `app`, `stem-worker` and `decode-worker`" gets `write-worker`.  The first line of `THIRD-PARTY.md` names the four WebAssembly modules.
* `docs/invariants.md`: LAYER-1 names the page side (`app`, `decode-worker` and `write-worker`) and the web app (`app`, `decode-worker`, `write-worker` and `stem-worker`), in its text and its "Enforced by" sentence, and `the_page_and_the_worker_meet_only_through_stem_protocol` follows the new crate too.  This names a new crate in a rule's text, and does not change its meaning; the team leader names it at the checkpoint.  The new tests go on the "Enforced by" lines of PROTO-1, MEM-2 and MEM-5.
* `xtask`: the crate lists and the path routes of `xtask/src/check.rs`, the routes of `xtask/src/review.rs`, the crate runs of `xtask/src/web_test.rs`, the files of `xtask/src/publish.rs`, and the worker files of `app/src/reset.rs`.  The tests of each change with them.

### Step 5c: the page uses the write worker

* The call sites of the table use the write worker.  The others do not change.
* `write_within`, `write_mix` and `save_open` to `save_close` keep their counts in `WRITES`, so a storage reset waits for a routed write as today, and `CLOSED` refuses a new one.  A test: a reset during a routed write waits for it, and a routed write after the reset starts is refused.
* The existing web tests of `app` pass in Chrome and Safari, with the fake write worker.
* `docs/ARCHITECTURE.md` says "The page writes its files through write streams, on the main thread".  The commit of Step 5c changes it: which writes go through the write worker, and which stay on `createWritable`, and why (TD-103).

### Step 5d: the acceptance runs

* A counted Safari `e2e save` (it opens a song and saves it) leaves no new swap file from a routed write.  Each new swap file is a small write that stays on `createWritable` (`meta`, `current` and the like), and their bytes are under 1 MiB.  Also a counted `e2e smoke`: no swap file of a source file.  This is narrower than "no new swap file", since the small writes stay.  The orchestrator accepted this wording on 2026-11-14 (Decision log).
* These pass in Chrome and Safari: `web-test`, `e2e storage`, `smoke`, `decode`, `playback`, `old-session`, `save`, `separate`.
* `e2e stored-stems`: the hashes do not change, in Chrome and Safari.
* `e2e memory-budget` passes in Chrome and Safari.  It reads the WebAssembly memory and the GPU bytes (MEM-4), not a worker's JavaScript heap.  The write worker has a WebAssembly memory of its own.  The implementer checks whether the tab total counts it.  If not, a run records its size, and the total counts it if it is more than 1 MB.  The write worker's JavaScript bound is the largest body that it holds times the depth of its queue: an 8 MiB slice of a `Blob`, a 2 MiB block, or a whole memory-path stem (about 17.6 MB for 100 s), for each waiting request.  The transferred arrays move from the page, so they do not add to the tab.  The implementer records the bound and how it was checked, or that it was not.
* `check --tier push` passes, with both perf gates where the paths need them.

### Step 5e: the docs

* TD-103, the quirk entry and the draft WebKit bug: what still leaks after Step 5c (the small writes, the blob folders, the test helpers).

### Step 6: the record (case 4)

If the cause is in WebKit:

* An entry in `docs/references/browser-quirks.md`: Safari keeps each write stream's swap file after `close()`, with the WebKit source lines and the counted runs.
* A draft WebKit bug in `docs/references/webkit-bug-writable-swap-file.md`, as `references/webkit-bug-workgroup-zero-init.md` is, for the maintainer to file.  A line for it in `docs/README.md`.
* A new TD in `docs/tech-debt.md` (the next free number): what still leaks after Steps 4 and 5 (page writes, test helpers, blob folders), its bytes for each use, and that `xtask` only reports it.
* A note in `docs/references/local-browser-testing.md`: the line of Step 2, and how to delete the files by hand after Safari quits.

### Step 7: the checks before review round 1

* `cargo run -p xtask -- check --tier push`, with `STEMYLA_PERF_WEIGHTS` and `CHROMEDRIVER` set.  Both perf gates run if the paths need them.  A skip line is not a pass.
* `cargo build -p xtask`.
* A last counted run in each browser: `e2e storage` and `e2e weights`.  Chrome: 0 new profiles.  Safari: the numbers of Step 3 less the fixed leaks.
* The Progress and Surprises of this plan are current.

## Validation and acceptance

* Case 1: Step 1's tests fail before the change and pass after it.  The counted Chrome runs of Steps 1 and 7 leave 0 new profiles in `$TMPDIR`, and no run's Chrome folder.
* Cases 2 and 5: Surprises gives the cause of the swap files and of the blob folders, each with counted runs.
* Case 3, if Step 3 ties a leak to the app: Step 3a's test fails first and passes after.  A counted Safari run (`e2e weights` or `perf`) leaves no new swap file and no new blob folder from that path.
* Case 4, if the cause is WebKit: Step 4's tests fail first and pass after.  Counted Safari runs of `e2e weights --weights` and `e2e worker-check --download` leave no model-sized swap file.  The quirk entry, the draft bug and the new TD exist.
* Case 6: each Safari `e2e` and `web-test` run prints the line of Step 2, and its tests pass.  Chrome meets case 6 by Step 1: each run removes its own folder, and a failed removal prints its path.
* The page's writes (option c), Steps 5a to 5e): each new test fails first and passes after.  A counted Safari `e2e save` leaves no new swap file from a routed write, and each new swap file is a small write, under 1 MiB in all.  The scenarios of Step 5d pass in Chrome and Safari, `stored-stems` hashes do not change, and `memory-budget` passes.
* `check --tier push` passes, with both perf gates where the paths need them.

## Idempotence and recovery

Each step is one or more signed commits.  A session that stops reads Progress and goes on from the next step.  A counted run changes no file in the repo.  If a run leaves a run's Chrome folder, the next run's start sweep removes it.  A swap file that a run leaves stays in Safari's folder.  Record it, and do not delete it.

## Decisions for the maintainer

1. The page's writes in Safari (Step 5).  Options and recommendation: from Step 3's figures.  Answer (2026-11-14, the maintainer, through the orchestrator): c), a write worker for this one purpose, which keeps the decode worker's duties clean (decision 0019).  The maintainer first said b), then changed to c).
2. The form of the write worker.  Options: a JavaScript inline script in `app`, or a Rust crate built by Trunk, as `decode-worker` is.  Recommendation: either; JavaScript is less code and fewer docs, and a Rust crate keeps PROTO-1's text and the Rust tooling.  Answer (2026-11-14, the maintainer, through the orchestrator, parent `00668a1`): a Rust crate.

## Progress

* [x] 2026-11-14: the plan written, at `8b16df7`.
* [x] 2026-11-14: Step 1: each Chrome run removes its own profile (`71e0e5b`).  The 10 new tests failed first on an assert (`m51/step1-fail-first.log`, `m51/step1-fail-first-2.log`).  The counted Chrome runs left 0 new profiles and no run's Chrome folder (Surprises).
* [x] 2026-11-14: Step 2: `xtask` reports what a run left (`2cb5de4`).  The 4 new tests failed first on an assert (`m51/step2-fail-first.log`, `m51/step2-fail-first-b.log`).  TD-103 records the leak.
* [x] 2026-11-14: Step 3: counted runs in Safari (Surprises).  WebKit keeps the swap file of each closed write stream; an abort removes it.  The code check found no stream of the app with no `close()` and no `abort()`, and no object URL with no `revokeObjectURL`.  So the cause is in WebKit (case 4).
* [x] 2026-11-14: Step 3a: not needed.  Step 3 ties no leak to a path of the app.
* [x] 2026-11-14: Step 4: the model saves use a sync access handle.  `write_file` in `worker/src/web.rs` writes through `createSyncAccessHandle`, for `opfs_write`, `cache_put` and `download_to_opfs`; no other production `createWritable` was in `worker/` or `decoder/`.  `the_model_writes_open_no_write_stream` and the two moved MEM-5 tests failed first on an assert (`m51/step4-fail-first-*.log`).  Counted Safari runs s9 and s10 left no swap file (Surprises).  `check --tier push` passed at `6199418`, 15 lines, with both perf gates: Chrome 3.65 s against a bound of 3.95 s, Safari 5.64 s against 6.13 s (`m51/check-push-1.log`).
* [x] 2026-11-14: Step 5: the page's writes (decision point): decided: c), a write worker on the page for the song-sized writes (Decisions for the maintainer, item 1).  The team leader adds its steps.
* [x] 2026-11-14: Step 5a: the facts before the code (Surprises).  No reader overlaps a routed write.  The per-block form costs 3.7 % (Chrome) and 1.4 % (Safari) of a 4-minute mix write, so Step 5b takes it.
* [x] 2026-11-14: Step 5b: the write worker crate (`write-worker`), its messages (`stem_protocol::write`), the client (`app::writer`) and the fake behind `fake-writer`, with the harness and the docs.  Each new test failed first on an assert (`m51/step5b-*fail*.log`).  `web-test` passed in Chrome and in Safari (s13).  The orchestrator applied the diff of `CLAUDE.md` and `.claude/rules/**` (`m51/step5b-claude.diff`) at the step checkpoint.
* [x] 2026-11-14: Step 5c: the page uses the write worker: `save_sources` (each source file), `save_stems` (through `.wav.page.part` and a move), `write_mix` (1 MiB blocks at their offsets, then the move) and the save files (`save_open`, `save_put`, `save_close`, `save_abort`, with `StoredWav`'s array given back).  `each_routed_call_site_posts_its_file_and_opens_no_write_stream` and `a_reset_waits_for_a_routed_write_and_refuses_a_later_one` failed first on an assert (`m51/step5c-fail-1.log`); D3's one array (`arrays_made`) failed first at 2 arrays with the array not kept (`m51/step5c-run-through_one_array.log`).  `web-test` passed in Chrome and in Safari.
* [x] 2026-11-14: Step 5d: the acceptance runs (Surprises).  In Chrome and in Safari: `web-test`, `e2e save`, `smoke`, `storage`, `decode`, `playback`, `old-session`, `separate`, `stored-stems --hashes <M1's six>` and `memory-budget` pass.  The counted Safari `e2e save` and `e2e smoke` leave no swap file of a routed write.  `memory-budget` counts the write worker's WebAssembly memory (`WRITE_WORKER_WASM_MB`); `the_tab_total_must_fit_the_budget` failed first on an assert (`m51/step5d-tab-fail.log`).  `check --tier push`: Step 7.
* [x] 2026-11-14: Step 5e: TD-103, the quirk entry, the draft WebKit bug and `references/local-browser-testing.md` give what still leaks after Step 5c: the small writes, the blob folders and the test runs' writes.
* [x] 2026-11-14: Step 6: the record: the quirk entry in `references/browser-quirks.md`, the draft bug `references/webkit-bug-writable-swap-file.md` with its line in `docs/README.md`, TD-103 for the state after Step 4, and the notes and the known local failure in `references/local-browser-testing.md`.
* [ ] Step 7: the checks before review round 1.

## Surprises and discoveries

* 2026-11-14: WebKit's source, at `c526785513`, deletes a write stream's swap file only on an abort.  A `close()` copies the swap file onto the stored file and keeps the swap file (`FileSystemStorageHandle::closeWritable`).  Not yet confirmed by a counted run.

* 2026-11-14, Step 1, the counted Chrome runs (logs in `m51/`, each with `<run>.counts`, `<run>.log` and `<run>.watch`).  `$TMPDIR` held 2 `org.chromium.Chromium.scoped_dir.*` before each run.  Two orphan Chromes of earlier runs (pids 85971 and 86183, parent 1, started about 30 hours before) use them as `--user-data-dir`.  The orchestrator removed the folders, and those Chromes made them again at 14:42.  They are not of this milestone's runs, so they stay; the orchestrator decides.

  | Run | Result | New profiles in `$TMPDIR` | Run's Chrome folder after |
  | - | - | - | - |
  | c1 `e2e storage` | passed | 0 (2 to 2) | none |
  | c2 `e2e weights --weights <f16>` | failed at its end: "part 1: the f32 stems have M3's six hashes" | 0 (2 to 2) | none |
  | c4 `e2e storage`, `kill -INT` of its `xtask` at about 6 s | ended by the signal (exit 130) | 0 (2 to 2) | kept, as planned |
  | c3 `e2e smoke --play-x 5000` | failed after its session started: WebDriver "move target out of bounds" | 0 (2 to 2) | its own: none; c4's: kept, since the orphan Chrome of c4 named it |
  | c5 `web-test --browser chrome` (5 crates) | passed | 0 (2 to 2) | none; at its start it printed "chrome folder: removed /tmp/stemyla-chrome/12977-0, which an ended run left" |

  * c2 failed on the stem hashes of part 1, which Step 1 does not change.  The probable cause: this Mac has only the f16 model, and M3's hashes are of the f32 model.  c2 ran at `71e0e5b`, whose diff from the base `8b16df7` changes only docs and `xtask/src/{drivers,web_test,webdriver}.rs`.  So the page, the workers and `xtask/src/e2e/weights.rs` were those of the base, and the check fails at the base too.  s3 gave the same six hashes in Safari: vocals and piano equal `M3_HASHES`, and the other four differ.  It does not matter for Step 4: Step 4's acceptance uses the scenario's other checks, which compare the f16 and f32 stems bit for bit, and the read-back tests of the writes.  It is also a run that fails after its session starts, after 8 minutes.
  * After c4's signal, its `chromedriver` (12984) and Chrome (12985) stayed as orphans, and Chrome's `--user-data-dir` named the folder.  The implementer ended those two PIDs, which c4's own `ps` tied to its `xtask`.  A first try of c4 sent SIGINT to a background job of a non-interactive bash, which ignores SIGINT, so that run passed (`m51/c4a-sigint-ignored.*`); the second try used `set -m`.
  * Chrome reads `MAC_CHROMIUM_TMPDIR`: during each run, the `scoped_dir` profiles were in the run's Chrome folder (22 in c5's watch).  The real socket path was `/tmp/stemyla-chrome/32776-0/com.google.Chrome.GkbIUr/SingletonSocket`, 57 bytes.  The bundle folder has no leading dot.  The longest form in the test (`/tmp/stemyla-chrome/4294967295-4294967295` with Chrome for Testing's folder) gives 95 bytes, under 104.
  * Chrome also made `com.google.Chrome.chrome_chrome_Unpacker_BeginUnzipping.*` and `com.google.Chrome.chrome_chrome_url_fetcher_.*` folders there.  The removal of the run's folder removes them too.
* 2026-11-14, Step 3, the counted Safari runs (logs in `m51/`, `s<n>-*.counts`, `.log` and `.watch`).  No Safari ran before s1.  s1's session start launched Safari (pid 14461, parent 1).  It stayed open after each session, and each later run used it.  The implementer did not quit it.  Each `browser data:` line gave the same counts as the manual count.  A count 5 minutes after s1 to s5 gave the same figures as the count at the end: Safari removed nothing while it ran.

  | Run | New swap files | Their bytes | New blob folders | What made the swap files |
  | - | - | - | - | - |
  | s1 `web-test` `a_view_is_written_whole` (3 `st_write`, each closed) | 3 | 15 (3 of 5) | 3, empty | each closed stream |
  | s2 `web-test` `a_dropped_wav_save_leaves_no_file_and_no_stream` | 2 | 10,584,088 (2 of 5,292,044) | 1, empty | the 2 planted stems, closed; the 2 aborted save streams (one dropped at its open, one with a write in flight) left none |
  | s3 `e2e weights --weights <f16>` (`opfs_write`) | 3 | 349,521,434 | 1, empty | the model, 349,521,256; the kernel choice of `cache_put`, 60 and 118 |
  | s3b the same run again | 3 | 349,521,430 | 1, empty | the model; the kernel choice, 58 and 116 |
  | s4 `e2e worker-check --download --weights <f16>` (`download_to_opfs`) | 2 | 349,521,316 | 1, empty | the model, 349,521,256; the kernel choice, 60 |
  | s5 `e2e storage` | 5 | 4,217 | 1, empty | 4 downloads of 1,000 bytes and the 217-byte non-model of `?model_url=`; the stalled download, aborted, left none |
  | s6 `e2e smoke` (one song open: 5 dropped 30 s stems) | 8 | 52,920,667 | 1, empty | the 5 source files, 10,584,068 each; 3 small files (125, 13, 189) |
  | s7 `web-test` `a_wav_save_of_a_stored_song_offers_its_save_file` (one save) | 4 | 21,168,176 (4 of 5,292,044) | 1, empty | the 3 planted stems and the WAV save file |

  * The swap files: each `close()` of a `createWritable` stream keeps one swap file of the bytes written, and an `abort()` keeps none.  s1 to s7 show it for the page (`st_write`, `st_open` and `st_close`), the stem worker (`opfs_write`, `download_to_opfs`, `cache_put`) and the test helpers.  This is what WebKit's source says (Context).  Safari did not remove them in 5 minutes, and the 176 GB of 2026-11-14 shows that they stay longer.  A quit of Safari does not remove them: the orchestrator saw the 35,841 files stay after Safari quit and its network process ended (Decision log).  The implementer did not quit Safari.
  * The bytes for each use of the app in Safari: one model save, 349.5 MB (s3, s4); one song open, the bytes of the dropped files (52.9 MB for 5 stems of 30 s, s6); one save, the bytes of the save file (5.3 MB for a 30 s WAV, s7).  A separation also writes the mix (about 85 MB for 4 minutes) and the stems; no run here measured them.
  * The blob folders: each WebDriver session made one new `BlobRegistryFiles-*` folder at its start, and Safari's launch made 3 (s1).  Each stayed empty, also after s7's object URL of a save file.  So a blob folder comes from a new network session, not from a blob or an object URL of the app.  It holds no bytes, but it stays: the 767 folders of 2026-11-14 are about 767 sessions.  In these runs, only a session start or the launch of Safari made one.
  * The code check: each `createWritable` of the production code ends with `close()` or `abort()` on each path (`st_write`, `write_file` in `worker/src/web.rs`, `write_mix_now`, `save_open` to `save_close` and `save_abort` with `StoredWav`'s drop, and `app/src/streamtest.rs`).  The test helpers `plant`, `plant_at`, `store_plant_tone` and `store_plant_wav` close, but do not abort on a failed write.  `save_blob` revokes its URL after 60 s, `app/src/streamtest.rs` revokes its worker URL, and each test that calls `blob_url` calls `revoke`.

* 2026-11-14, Step 4, the counted runs after the change (logs in `m51/`).  The page was rebuilt with the change (`app/dist`).

  | Run | Result | New swap files | New blob folders |
  | - | - | - | - |
  | s8 `web-test --browser safari` (5 crates, before the model runs) | passed | 1,311, 1,691.2 MB | 8, empty |
  | s9 `e2e weights --weights <f16>` (`opfs_write`) | the known hash check failed; each other check passed | 0 (s3 and s3b: 3, with the 349,521,256-byte model) | 1, empty |
  | s10 `e2e worker-check --download --weights <f16>` (`download_to_opfs`) | passed | 0 (s4: 2, with the model) | 1, empty |
  | c6 `e2e worker-check --download` in Chrome | passed | none in Safari's folder | none |
  | c7 `e2e weights` in Chrome | the known hash check failed; each other check passed | none in Safari's folder | none |

  * The Safari perf gate of `check --tier push` (`e2e perf`, with the model) left 7 swap files, 10.6 MB, and 1 blob folder: the page's writes of its song, not the model.
  * s8 is the leak of a whole Safari `web-test`: the page's writes in its tests and the test helpers, which Step 4 does not change (TD-103).
  * `$TMPDIR` held 0 `org.chromium.Chromium.scoped_dir.*` from s9 on.  The two orphan Chromes' folders went; this milestone's runs did not remove them.
* 2026-11-14, Step 6: the Step 2 commit (`2cb5de4`) had dropped the heading "## Legibility for agents" of `docs/tech-debt.md`, so TD-18, TD-19 and TD-20 were under "Known breaches".  `docs-check` did not see it.  Step 6 puts the heading back.

* 2026-11-14, Step 5a, the facts before the code (from the code at `48faa1d`, with file and line).
  * The callers, and what each does after a failed write:
    * `save_sources` (`app/src/store.rs:418`) removes its own session folder on a failure (`:422`).  Its one caller, `open_chosen_now` (`app/src/lib.rs:761`), logs and opens the song on the page from the chosen files; during a reset it gives the error.
    * `write_mix` removes `mix.f32.part` after a failed open, put, close or move, and `mix.f32` after a failed last `meta` write (`app/src/store.rs:543-569`).  Its one caller, `write_mix_first` (`app/src/lib.rs:3583`), gives the result to `mix_written`, which separates on the memory path with no storage, or refuses with a status line.
    * `save_stems` does no clean-up: `meta` names no stems from its first write to its last, so a failure leaves stray `stems/*.wav` and no `stem` lines.  Its one caller, `apply_separated` (`app/src/lib.rs:2267`), sets a status line.  This call is not reached in production: it needs `job.session` and `keeps_stems`, and each memory-path post of a song with a session clears both (`app/src/lib.rs:3536-3556`, `3821-3824`).  Only the tests call `save_stems` (`app/tests/web.rs`, `app/tests/web_close.rs`, `app/src/decode_tests.rs`, `app/src/store.rs`).  Step 5b routes it all the same, as the plan says.
    * `save_open` to `save_abort` serve only `StoredWav` (`app/src/export.rs:701-923`).  A failed open or close removes the file, and a failed put aborts and removes it.  The save then ends `Finish::Failed`, and the page saves again on the memory path (`app/src/lib.rs:4783-4789`).
  * The readers: no reader of the page, the decode worker or the stem worker can read a routed file while it is written.
    * `src/<i>.<ext>`: the decode worker (`getFile`, `decoder/src/web.rs:43-55`) and the page's `rebuild` and `head_of` read it only after `save_sources` returns.  Each save has a new session ID, and the session is not listed before its `meta`.
    * `mix.f32.part`: no reader.  `mix.f32` is read (the stem worker, `worker/src/web/mix.rs:24-31`; the playback and the decode worker when it is usable) only after the move and the `mix` line.  `MIX_WRITING` defers `forget_mix` and `keep_decoded`, and Separate is off while a separation runs, so two mix writes of one session cannot run at once.
    * `stems/<name>.wav` (only from tests, above): the playback reader, `OpfsStore`, `head_of`, `read_stem` and the decode worker read the `.wav` with `getFile`.  An old stored lane of the same session can read the old `.wav` while a new save of the same name runs.  With `.wav.page.part` and a move (Step 5b), that read sees the old file, as with `createWritable` today.
    * `saves/*.wav`: the download offer (`st_file`, `getFile`) and the stem worker's encodes read it only after `save_close` succeeds (`Finish::Done`).  `clean_saves` at the page's start reads `getFile` of each file in `saves/`, and keeps a file whose read fails (`st_remove_older` catches it), so another tab's open handle loses nothing.
  * The arrays: no routed call site passes a view into WebAssembly memory.  `save_sources` passes the dropped `Blob`.  `put_mix_blocks` copies each 1 MiB block into a new array (`Uint8Array::from`, `app/src/store.rs:607`).  `save_stems` passes `Uint8Array::from` of a whole 16-bit WAV (17.64 MB for a stem of 100 s).  `StoredWav`'s `self.array` is the one reused array: 2 MiB, made once, each full block copied in with `copy_from` (`app/src/export.rs:46`, `785-791`); the last block is a new array (`:770`).
  * The writes that stay on `createWritable` (a 4-minute song): `meta`, about 0.3 to 0.4 KB a write, written about 11 places in `app/src/store.rs`; `sessions/current`, 13 bytes; the `?streamtest` scratch files, a diagnostic only (36 bytes a frame).  The overviews, `classic.wav`, `decoded/<i>.f32` and the stems of the storage path are written by the decode worker and the stem worker through sync access handles already.
  * The cost of a sync access handle for each block (a worker of the M2 Pro; logs `m51/step5a-measure-chrome.log`, `m51/s11-safari-measure.log`, `m51/step5a-mixwrite-chrome.log`, `m51/s12-safari-mixwrite.log`):

    | | Chrome 157 | Safari 27.2 |
    | - | - | - |
    | 84,672,000 bytes (a 4-minute mix) in 81 blocks of 1 MiB, one handle | 15 to 19 ms | 13 to 22 ms |
    | the same, a handle opened, flushed and closed for each block | 77 to 88 ms | 36 to 42 ms |
    | the same, one `createWritable` stream | 81 ms | 40 ms |
    | the page's `write_mix` of a 4-minute song of 2 stored lanes (3 runs) | 1,710 to 1,732 ms | 1,772 to 1,839 ms |
    | the cost of the handle for each block, over the mix write | 63 ms, 3.7 % | 25 ms, 1.4 % |

    Under 5 % in both browsers, so Step 5b takes the per-block form (Decision log).  The measures were temporary tests, not committed.  s11's stream run left one swap file of 84,672,000 bytes, and s12 left 5 of 84,672,000 (its 2 planted lanes and its 3 mix writes) and 7 small ones (`meta`).

* 2026-11-14, Step 5c: a whole Safari `web-test` after Step 5c (s14) left 1,556 swap files, 8,898.9 MB, against 1,316 files, 1,697.2 MB before it (s13).  The cause is the fake write worker of the tests: it writes each block through a new write stream that keeps the file's bytes (`keepExistingData`), so each block's swap file holds the whole file so far.  The shipped page writes no such stream.  CI's macOS runner is new for each run; on this Mac, a person deletes the files (TD-103).
* 2026-11-14, Step 5d, the counted runs after Step 5c (logs `m51/d-<browser>-<scenario>.*`; Safari's folder has a new count from the maintainer's clean-up).
  * Safari `e2e save` (WAV, FLAC, M4A, MP3 and AAC saves of 1 and 10 minutes): 40 new swap files, 349.3 MB.  Of them, 6 are the scenario's own plant (`old_session`'s plant, through `createWritable`), 3 of 10,584,044 bytes and 3 of 105,840,044, made at its start (18:59), before the first save.  The other 34 are small writes: 28 of `current` (13 bytes) and 6 of `meta` (64 to 148 bytes), about 1 KB in all.  No save file left one.  Before Step 5c, a 30 s WAV save left its 5.3 MB (s7).
  * Safari `e2e smoke` (one song open of 5 dropped stems): 3 new swap files, 327 bytes (`current` and `meta`).  Before Step 5c, s6 left the 5 source files, 52.9 MB.
  * Safari `e2e storage` left none; `decode` 0.4 MB, `playback` 31.8 MB, `old-session` 444.5 MB and `stored-stems` 1.1 MB, each from the scenario's own plants through `createWritable`.
  * `e2e save` failed first in Chrome, on its heap readings: `save_spy.js` read the heap at each write of a page's write stream, and the save's blocks now go to the write worker.  It now reads at each post with a transfer list too.  Then it passed in both browsers.
  * `stored-stems --hashes` gave M1's six hashes in both browsers (`plans/completed/gpu-speed-m1.md`).  They equal the hashes that `e2e weights` part 1 printed for its f32 load of the f16 file (c2), which supports the probable cause of that known failure.
  * `memory-budget`: the tab total, with the write worker's 1.25 MB, was 756 MB in Chrome and 758 MB in Safari (margin 187 and 185 MB).  The write worker's module starts with 17 pages (1.11 MB, the memory section of `write-worker_bg.wasm`), over the plan's 1 MB, so the total counts it.  Its JavaScript bound, as the plan gives it: its queue holds each request's body, a 2 MiB block, an 8 MiB slice of a `Blob`, or a whole memory-path stem (17.64 MB for 100 s); each of a save, a mix write and a source save sends one request at a time, so the bound is one body for each that runs.  Not measured.

## Decision log

* 2026-11-14, agent (team leader): Chrome's temporary files go to a folder that `xtask` makes and removes, through `MAC_CHROMIUM_TMPDIR`, not `--user-data-dir`.  Why: `web-test`'s one capabilities file serves each test binary of a crate run, so one `--user-data-dir` would share browser storage between them.
* 2026-11-14, agent (team leader), plan review round 1: the folders are in `/tmp/stemyla-chrome/`, not `$TMPDIR`, so the start sweep reads only a folder that `xtask` owns, and the socket path fits.  The `Driver` owns its folder, so a failed session start removes it.  Step 2 reports Safari only.  Step 3 adds an abort test that reaches WebKit and the `--download` path of the model.  Step 3a covers a cause in the app.
* 2026-11-14, agent (team leader), plan review round 2: `MAC_CHROMIUM_TMPDIR` on macOS only, so cargo and rustc are not affected.  The start sweep keeps a folder that a live process names, and a folder of another user.  The code goes in existing files, with no new module.  The MEM-5 spies of the model writes move to the sync access handle.
* 2026-11-14, agent (implementer), Step 1: the start sweep reads the current user from `/usr/bin/id -u`, since the standard library gives no user ID and `xtask` has no `libc`.  It reads each process's command line from `ps -A -ww -o pid=,command=` (`drivers::command_lines`), since `drivers::processes()` gives the executable only, and an orphan Chrome names the folder in its arguments.  A process of another user counts as live too, which keeps more.  `/tmp/stemyla-chrome` must be a folder of the current user, not a link, or the run stops with an error that tells the person to remove it.  The folders get mode 700.
* 2026-11-14, agent (implementer), Step 2: the listing is of `getconf DARWIN_USER_TEMP_DIR`, not `TMPDIR`, since a tool can set `TMPDIR` to another folder, and WebKit uses the user's temporary folder.  The line also gives the growth of the swap files and blob folders that were there before, since a blob folder of a Safari that runs can grow in place.  TD-103 is added in Step 2, since the line names it; Step 6 completes it.
* 2026-11-14, agent (implementer), Step 3: `e2e weights` has no download path: it reads no `--download` (s3b shows it).  The download path of the model runs in `e2e worker-check --download`, so s4 uses it.  Step 4's acceptance names `e2e weights --download`; it needs `worker-check --download` too, or a `--download` in `weights`.  The abort row uses the existing test `a_dropped_wav_save_leaves_no_file_and_no_stream`: its two save streams reach WebKit's `createWritable`, and one has a write in flight before its abort.  So no new test.  Rows s6 and s7 give the bytes of one song open and one save.

* 2026-11-14, agent (team leader), after Step 3: Step 3a is not needed; the cause is WebKit (Surprises, s1 to s7).  The download path is checked with `e2e worker-check --download`, since `e2e weights` does not read `--download`.  The failed hash check of `e2e weights` on this Mac is not a check of Step 4.  The implementer records it under "Known local failures" in `references/local-browser-testing.md`, with the evidence.
* 2026-11-14, orchestrator: a quit of Safari does not remove the swap files.  The orchestrator saw it at about 13:30 on 2026-11-14: Safari quit, its network process ended, and the 35,841 swap files (176 GB) were still there.  No agent quits Safari or deletes a swap file, a blob folder or another file; the deletes go to the orchestrator.

* 2026-11-14, maintainer (through the orchestrator, parent `00668a1`): the write worker is a Rust crate built by Trunk, as `decode-worker` is, not a JavaScript inline script.  So its messages go in `stem-protocol`, and PROTO-1's text does not change.
* 2026-11-14, orchestrator: the acceptance of Step 5d is "no new swap file from a routed write; each new swap file is a small write that stays on `createWritable`, under 1 MiB in all".  Why: `close()` replaces a small file in one step, and the page reads `meta` with no lock; a sync access handle writes in place and is exclusive.
* 2026-11-14, agent (team leader), plan reviews of Steps 5 to 5e (4 rounds over the two forms): the worker sends a ready message, and the client holds requests until it comes; the reused array of a save comes back by transfer; `save_stems` writes through `.wav.page.part`; a failed worker is ended and its requests fail; the app's tests use a fake write worker behind the non-default feature `fake-writer`, which `xtask web-test` turns on, since the integration tests link the library without `cfg(test)`.
* 2026-11-14, agent (implementer), Step 5a: the block writes take the per-block form (a `write` with an offset, which opens, writes, flushes and closes a handle for each block, with no stream ID).  Why: the cost of a handle for each block was 3.7 % of the mix write in Chrome and 1.4 % in Safari, under the plan's 5 % (Surprises).  No fact of Step 5a breaks the design: no reader overlaps a routed write.  `save_stems` is reached only by tests; it is routed all the same.

* 2026-11-14, agent (implementer), Step 5b: the messages are two requests, `write-file` (a whole file from a `Blob` or a `Uint8Array`) and `write-block` (a block at an offset; at 0 the file is cut first), and two replies, `write-ready` and `write-done` (`result`: `written`, `gone` or `failed` with `error`; a block's `bytes` given back).  Requests carry no ID: the worker ends them in the order sent, as the decode worker does.  The worker opens, writes, flushes and closes a handle for each request (Step 5a).  It has no `console_error_panic_hook`, since the plan's list of its dependencies has none.  The fake serves a whole file through `st_write` (`store::fake_write_whole`) and a block through a write stream with `keepExistingData` after the first block; it is a JS object whose `postMessage` calls a Rust closure, after a structured clone with the transfers.  Step 5b changes no call site of `app/src/store.rs`, so the tests of the routed call sites (each posts, no `createWritable`, `WRITES`, D3's one buffer) go with the routing in Step 5c.
* 2026-11-14, orchestrator, Step 5b: accepted the `fake-writer` feature of `app` (an agent decision of the plan reviews): the app's browser tests write through a page-side fake of the write worker, which `xtask web-test` turns on, and the shipped page never does.
* 2026-11-14, agent (implementer), Step 5c: no open block: a save and the mix start with their first block at 0, which cuts the file and finds a gone session, so the fake's spies see the same writes as before.  `save_close` has no write: the last block ends the file.  The page's write streams of the stream form (`st_open`, `st_put`, `st_close`, `st_abort`) are gone, with no caller left.  A stem whose move fails after a reset gives "browser storage is being reset", as the next write did before.  The tests that change: `a_wav_save_of_a_stored_mix_writes_whole_blocks_through_one_array` counts the arrays made (`arrays_made`, 1), not the identity of each written object, since a transfer makes a new object on each side; `sources_are_saved_from_their_files` records `blob` for the source, a structured clone of the dropped `Blob`, not `same`.

## Outcomes and retrospective

Not yet.
