# 0032: a page-side worker crate writes the page's song-sized files

Status: accepted (2026-11-14, by the maintainer, through the orchestrator)

The maintainer chose option c) of M5.1 Step 5, a write worker for this one purpose, and its form, a Rust crate built by Trunk (parent plan `plans/active/gpu-speed.md`, Decision log, commits `bd40598` and `00668a1`).  This record states that choice.

## Context

Safari keeps the swap file of each OPFS write stream that closes (`references/browser-quirks.md`, TD-103).  On 2026-11-14, test runs had left 176 GB of them.  Each song open left the bytes of the dropped files, and each save the bytes of its save file.  A `FileSystemSyncAccessHandle` writes a file in place and makes no swap file, but it exists only in a dedicated worker ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemFileHandle/createSyncAccessHandle)).  The page writes its files on the main thread.

## Options

* a) Keep the page's writes on write streams, with a WebKit bug and a TD.  They keep leaking in Safari until WebKit changes.
* b) The decode worker (decision 0019) writes the page's song-sized files.  Its messages change, and it gets a second duty.
* c) A new page-side worker for this one purpose writes them.  It can be a JavaScript inline script in `app`, or a Rust crate built by Trunk, as `decode-worker` is.

## Decision

Option c), as a Rust crate: `write-worker`, in `writer/`.  It writes the files as dropped, the mix, the stems of the memory path and the save files through sync access handles.  Its messages are in `stem_protocol::write`, so PROTO-1 holds as it is.  The small files (`meta`, `current`) stay on write streams: a `close()` replaces such a file in one step, and the page reads `meta` with no lock.

## Consequences

* The layers table in `docs/ARCHITECTURE.md` gets a row for the crate.  LAYER-1's wording and its tests cover a fourth WebAssembly module, on the page side.
* The page and the write worker exchange messages.  Their encoders and decoders follow PROTO-1, and each typed array is transferred (MEM-2).
* The write worker logs no memory.  `e2e memory-budget` counts a fixed 1.25 MB for its WebAssembly memory (TD-11).
* The app's browser tests run with no worker build, so they use a page-side fake behind the `fake-writer` feature, which `xtask web-test` turns on.
* Trunk builds a fourth WebAssembly file of about 70 KB, so the page downloads more code once.
