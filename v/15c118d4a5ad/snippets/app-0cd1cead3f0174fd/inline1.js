
// Reads `ranges[2i]..ranges[2i + 1]` of each `File` of `files` (null: no
// read), each on its own promise, so one bad file does not lose the
// others'.  Gives a Uint8Array, null, or the error's text for each.
export async function ls_read(files, ranges) {
  const got = await Promise.allSettled(files.map((f, i) =>
    f ? f.slice(ranges[2 * i], ranges[2 * i + 1]).arrayBuffer() : Promise.resolve(null)));
  return got.map((r) => r.status === "fulfilled"
    ? (r.value ? new Uint8Array(r.value) : null)
    : String((r.reason && r.reason.name) || r.reason));
}
