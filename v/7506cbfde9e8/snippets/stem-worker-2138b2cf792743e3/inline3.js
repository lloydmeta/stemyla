
export async function mix_file(path) {
  const parts = path.split("/").filter((s) => s);
  const name = parts.pop();
  let d = await navigator.storage.getDirectory();
  for (const p of parts) d = await d.getDirectoryHandle(p);
  return await (await d.getFileHandle(name)).getFile();
}
