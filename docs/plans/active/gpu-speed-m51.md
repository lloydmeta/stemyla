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
| `e2e weights` in Safari, with `--download` | the model save through `download_to_opfs`, the users' path: the same |
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
* Acceptance: counted `e2e weights` runs in Safari, with `--weights` and with `--download`, leave no swap file of 349,521,256 bytes.  `e2e weights` passes in Chrome and Safari.  `check --tier push` passes, with both perf gates (the worker's paths change).

### Step 5: the page's writes (decision point)

The page's writes run on the main thread, where a sync access handle is not available.  If Step 3 shows that they leave swap files, the team leader gives the orchestrator Step 3's bytes for each use and these options:

1. A WebKit bug and a TD only.  The page's writes keep leaking in Safari until WebKit fixes it.  Cost: Step 6 only.
2. The page's song-sized writes (the source files, the mix, the stems of the memory path, the save files) go through the decode worker, which writes with sync access handles.  That is a change of the messages (PROTO-1) and of the page's storage code.
3. A small dedicated write worker on the page, from an inline script (as `app/src/streamtest.rs` makes its worker).  It is a new module.

The team leader recommends from Step 3's figures, and waits for the answer.  The answer goes in "Decisions for the maintainer".  If the answer is 2 or 3, the implementer designs it in this plan before the code, as a new step after Step 5.

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
* Case 4, if the cause is WebKit: Step 4's tests fail first and pass after.  Counted Safari `e2e weights` runs (`--weights` and `--download`) leave no model-sized swap file.  The quirk entry, the draft bug and the new TD exist.
* Case 6: each Safari `e2e` and `web-test` run prints the line of Step 2, and its tests pass.  Chrome meets case 6 by Step 1: each run removes its own folder, and a failed removal prints its path.
* `check --tier push` passes, with both perf gates where the paths need them.

## Idempotence and recovery

Each step is one or more signed commits.  A session that stops reads Progress and goes on from the next step.  A counted run changes no file in the repo.  If a run leaves a run's Chrome folder, the next run's start sweep removes it.  A swap file that a run leaves stays in Safari's folder.  Record it, and do not delete it.

## Decisions for the maintainer

1. The page's writes in Safari (Step 5).  Options and recommendation: from Step 3's figures.  Answer: open.

## Progress

* [x] 2026-11-14: the plan written, at `8b16df7`.
* [x] 2026-11-14: Step 1: each Chrome run removes its own profile (`71e0e5b`).  The 10 new tests failed first on an assert (`m51/step1-fail-first.log`, `m51/step1-fail-first-2.log`).  The counted Chrome runs left 0 new profiles and no run's Chrome folder (Surprises).
* [x] 2026-11-14: Step 2: `xtask` reports what a run left (`2cb5de4`).  The 4 new tests failed first on an assert (`m51/step2-fail-first.log`, `m51/step2-fail-first-b.log`).  TD-103 records the leak.
* [x] 2026-11-14: Step 3: counted runs in Safari (Surprises).  WebKit keeps the swap file of each closed write stream; an abort removes it.  The code check found no stream of the app with no `close()` and no `abort()`, and no object URL with no `revokeObjectURL`.  So the cause is in WebKit (case 4).
* [x] 2026-11-14: Step 3a: not needed.  Step 3 ties no leak to a path of the app.
* [ ] Step 4: the model saves use a sync access handle.
* [ ] Step 5: the page's writes (decision point).
* [ ] Step 6: the record.
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

  * c2 failed on the stem hashes of part 1, which Step 1 does not change.  The probable cause: this Mac has only the f16 model, and M3's hashes are of the f32 model.  Not checked on the base commit.  It is also a run that fails after its session starts, after 8 minutes.
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

  * The swap files: each `close()` of a `createWritable` stream keeps one swap file of the bytes written, and an `abort()` keeps none.  s1 to s7 show it for the page (`st_write`, `st_open` and `st_close`), the stem worker (`opfs_write`, `download_to_opfs`, `cache_put`) and the test helpers.  This is what WebKit's source says (Context).  Safari did not remove them in 5 minutes, and the 176 GB of 2026-11-14 shows that they stay longer.  Not checked: whether a quit of Safari removes them.  The implementer did not quit Safari (the orchestrator's rule).
  * The bytes for each use of the app in Safari: one model save, 349.5 MB (s3, s4); one song open, the bytes of the dropped files (52.9 MB for 5 stems of 30 s, s6); one save, the bytes of the save file (5.3 MB for a 30 s WAV, s7).  A separation also writes the mix (about 85 MB for 4 minutes) and the stems; no run here measured them.
  * The blob folders: each WebDriver session made one new `BlobRegistryFiles-*` folder at its start, and Safari's launch made 3 (s1).  Each stayed empty, also after s7's object URL of a save file.  So a blob folder comes from a new network session, not from a blob or an object URL of the app.  It holds no bytes, but it stays: the 767 folders of 2026-11-14 are about 767 sessions.  In these runs, only a session start or the launch of Safari made one.
  * The code check: each `createWritable` of the production code ends with `close()` or `abort()` on each path (`st_write`, `write_file` in `worker/src/web.rs`, `write_mix_now`, `save_open` to `save_close` and `save_abort` with `StoredWav`'s drop, and `app/src/streamtest.rs`).  The test helpers `plant`, `plant_at`, `store_plant_tone` and `store_plant_wav` close, but do not abort on a failed write.  `save_blob` revokes its URL after 60 s, `app/src/streamtest.rs` revokes its worker URL, and each test that calls `blob_url` calls `revoke`.

## Decision log

* 2026-11-14, agent (team leader): Chrome's temporary files go to a folder that `xtask` makes and removes, through `MAC_CHROMIUM_TMPDIR`, not `--user-data-dir`.  Why: `web-test`'s one capabilities file serves each test binary of a crate run, so one `--user-data-dir` would share browser storage between them.
* 2026-11-14, agent (team leader), plan review round 1: the folders are in `/tmp/stemyla-chrome/`, not `$TMPDIR`, so the start sweep reads only a folder that `xtask` owns, and the socket path fits.  The `Driver` owns its folder, so a failed session start removes it.  Step 2 reports Safari only.  Step 3 adds an abort test that reaches WebKit and the `--download` path of the model.  Step 3a covers a cause in the app.
* 2026-11-14, agent (team leader), plan review round 2: `MAC_CHROMIUM_TMPDIR` on macOS only, so cargo and rustc are not affected.  The start sweep keeps a folder that a live process names, and a folder of another user.  The code goes in existing files, with no new module.  The MEM-5 spies of the model writes move to the sync access handle.
* 2026-11-14, agent (implementer), Step 1: the start sweep reads the current user from `/usr/bin/id -u`, since the standard library gives no user ID and `xtask` has no `libc`.  It reads each process's command line from `ps -A -ww -o pid=,command=` (`drivers::command_lines`), since `drivers::processes()` gives the executable only, and an orphan Chrome names the folder in its arguments.  A process of another user counts as live too, which keeps more.  `/tmp/stemyla-chrome` must be a folder of the current user, not a link, or the run stops with an error that tells the person to remove it.  The folders get mode 700.
* 2026-11-14, agent (implementer), Step 2: the listing is of `getconf DARWIN_USER_TEMP_DIR`, not `TMPDIR`, since a tool can set `TMPDIR` to another folder, and WebKit uses the user's temporary folder.  The line also gives the growth of the swap files and blob folders that were there before, since a blob folder of a Safari that runs can grow in place.  TD-103 is added in Step 2, since the line names it; Step 6 completes it.
* 2026-11-14, agent (implementer), Step 3: `e2e weights` has no download path: it reads no `--download` (s3b shows it).  The download path of the model runs in `e2e worker-check --download`, so s4 uses it.  Step 4's acceptance names `e2e weights --download`; it needs `worker-check --download` too, or a `--download` in `weights`.  The abort row uses the existing test `a_dropped_wav_save_leaves_no_file_and_no_stream`: its two save streams reach WebKit's `createWritable`, and one has a write in flight before its abort.  So no new test.  Rows s6 and s7 give the bytes of one song open and one save.

## Outcomes and retrospective

Not yet.
