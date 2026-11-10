
export function page_url() { return location.href; }
// The open document may be an older build than the server's, so the files
// to refresh come from the page as fetched now: its `<base href>` (if any)
// and its preloads, resolved against that page, not this one.
export async function fresh_page(page) {
  const r = await fetch(page, { cache: "reload" });
  if (!r.ok) throw new Error(`HTTP ${r.status} from ${page}`);
  const doc = new DOMParser().parseFromString(await r.text(), "text/html");
  const href = doc.querySelector("base[href]")?.getAttribute("href");
  const base = href ? new URL(href, page).href : page;
  const preloads = Array.from(
    doc.querySelectorAll('link[rel="modulepreload"][href], link[rel="preload"][href]'),
    (l) => new URL(l.getAttribute("href"), base).href
  );
  return { base, preloads };
}
export function absolute(path, base) { return new URL(path, base).href; }
export async function refetch(urls) {
  const results = await Promise.allSettled(
    urls.map((u) => fetch(u, { cache: "reload" }).then((r) => {
      if (!r.ok) throw new Error(`HTTP ${r.status} from ${u}`);
      return r.arrayBuffer();
    }))
  );
  return urls.filter((u, i) => results[i].status === "rejected");
}
// A terminated worker's open files may take a moment to be released, so
// entries that could not be deleted are tried twice more.
export async function wipe_opfs() {
  const root = await navigator.storage.getDirectory();
  let failed = [];
  for (let attempt = 0; attempt < 3; attempt++) {
    if (attempt > 0) await new Promise((r) => setTimeout(r, 300));
    const names = [];
    for await (const name of root.keys()) names.push(name);
    failed = [];
    for (const name of names) {
      try { await root.removeEntry(name, { recursive: true }); }
      catch (e) { if (e.name !== "NotFoundError") { console.warn("reset: could not delete", name, e); failed.push(name); } }
    }
    if (failed.length === 0) break;
  }
  return failed;
}
