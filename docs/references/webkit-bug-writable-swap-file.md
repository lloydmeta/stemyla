# WebKit bug text: a write stream keeps its swap file after close()

This is a bug text for the maintainer to file at [bugs.webkit.org](https://bugs.webkit.org), component Website Storage.  It is not filed yet (2026-11-14).  M5.1 of `plans/active/gpu-speed.md` measured it (`plans/active/gpu-speed-m51.md`, Step 3).  `browser-quirks.md` gives the quirk and what the code here does about it.  TD-103 records what still leaks.

Title: OPFS: FileSystemWritableFileStream.close() leaves its FileSystemWritableStream temporary file in the network process's temporary folder

## Summary

Each `FileSystemFileHandle.createWritable()` makes a temporary file in the temporary folder of the network process.  `close()` copies it onto the stored file and does not delete it.  `abort()` deletes it.  So each closed stream leaves a file of the bytes it wrote, and the files stay after Safari quits.  A page that saves a 350 MB file to the origin private file system leaves 350 MB in the user's temporary folder each time.

## Environment

Safari 27.2 on macOS 27.2.1, Apple M2 Pro.  Checked 2026-11-14.  The folder is `$DARWIN_USER_TEMP_DIR/com.apple.WebKit.Networking+com.apple.Safari/`.

## Source

WebKit at `c526785513`, the last change of `FileSystemStorageHandle.cpp` (2026-10-13), read on 2026-11-14.  Safari 27.2 ships from a release branch, which can differ from `main`.

* FileSystemStorageHandle.cpp in WebKit's Source/WebKit/NetworkProcess/storage, lines 262 to 295 (`createWritable`): `FileSystem::createTemporaryFile("FileSystemWritableStream"_s)` makes the swap file ([source](https://github.com/WebKit/WebKit/blob/c526785513/Source/WebKit/NetworkProcess/storage/FileSystemStorageHandle.cpp#L262-L295)).
* The same file, lines 297 to 321 (`closeWritable`): with the reason `Aborted`, it deletes the swap file.  Else it copies the swap file onto the stored file with `FileSystem::copyFile`, and the swap file stays ([source](https://github.com/WebKit/WebKit/blob/c526785513/Source/WebKit/NetworkProcess/storage/FileSystemStorageHandle.cpp#L297-L321)).
* The same file, lines 85 to 100 (`close`): it closes each open stream with the reason `Aborted`.  So only an open stream's swap file goes ([source](https://github.com/WebKit/WebKit/blob/c526785513/Source/WebKit/NetworkProcess/storage/FileSystemStorageHandle.cpp#L85-L100)).

## Test case

Run in a page of any origin, then list the folder above:

```js
const root = await navigator.storage.getDirectory();
const fh = await root.getFileHandle("lloyd", { create: true });
for (let i = 0; i < 3; i++) {
  const w = await fh.createWritable();
  await w.write(new Uint8Array(5));
  await w.close();
}
```

Expected: no new `FileSystemWritableStream*` file.  Actual: three new files of 5 bytes each.  (The counted run did the same through the app's own code, with three files of 5 bytes.)  With `abort()` in place of `close()`, there is no new file.

## Results

Counted runs on 2026-11-14 (the folder was listed before and after each run):

* 3 streams that closed, of 5 bytes each: 3 new files, 15 bytes.
* 2 streams that closed and 2 that aborted, one of them after a write: 2 new files, of the closed streams' sizes.
* A save of a 349,521,256-byte file: one new file of 349,521,256 bytes.
* The files were still there 5 minutes after each run, while Safari ran.
* After a quit of Safari, with its network process ended, 35,841 such files (176 GB) of earlier runs were still there.

## Workaround

In a dedicated worker, `createSyncAccessHandle()` writes the file in place, with no temporary file.  The main thread of a page has no such handle, so a page sends its large writes to a dedicated worker.  This app does that for the model, the songs, the mix, the stems and the save files (M5.1).  Its small files stay on write streams, since `close()` replaces such a file in one step.  They still leave a few hundred bytes for each song.
