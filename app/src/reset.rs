//! "Reset…": recovery for a page that a reload does not fix.
//!
//! Trunk writes fixed file names (`filehash = false` in `Trunk.toml`), and
//! the site's CDN may let the browser keep `.js` files much longer than
//! `.wasm` files.  After a new version is published, a browser can then hold
//! the old wasm-bindgen glue next to the new wasm, which fails to start on
//! every reload.  Refreshing the files fetches each one again with
//! `cache: "reload"`, which bypasses the HTTP cache and stores the fresh
//! copy.  Deleting stored data also clears this site's private file system
//! (OPFS): the model, the kernel choice and the kept songs.

use eframe::egui;

/// Trunk's names for the worker's files: the loader shim the app starts,
/// the wasm-bindgen glue it imports and the wasm the glue fetches.
pub const WORKER_LOADER: &str = "./stem-worker_loader.js";
pub const WORKER_FILES: [&str; 3] = [WORKER_LOADER, "./stem-worker.js", "./stem-worker_bg.wasm"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reset {
    /// Fetch the app's files again, then reload.  Songs and the model stay.
    RefreshFiles,
    /// Also delete everything this site keeps in browser storage.
    DeleteStored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogOutcome {
    Open,
    Cancelled,
    Chosen(Reset),
}

/// The URLs to fetch again after the page itself: the files the freshly
/// fetched page preloads (the app's glue, wasm and snippets, as Trunk lists
/// them) and the worker's files, each once and without a fragment.  All are
/// absolute.  They come from the new page, not the open one, so a file a
/// later build dropped is never asked for.
pub fn urls_to_refresh(preloaded: &[String], worker: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let candidates = preloaded
        .iter()
        .map(String::as_str)
        .chain(worker.iter().map(String::as_str));
    for url in candidates {
        let url = url.split('#').next().unwrap_or(url);
        if !url.is_empty() && !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
    }
    out
}

/// The reset dialog.  `reason` is why it opened (the worker's error), if
/// it did not open from the Storage menu; `songs` and `stored_mb` describe
/// what deleting would remove.
pub fn dialog(
    ctx: &egui::Context,
    reason: Option<&str>,
    songs: usize,
    stored_mb: f64,
) -> DialogOutcome {
    let mut outcome = DialogOutcome::Open;
    let modal = egui::Modal::new(egui::Id::new("reset_dialog")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.heading("Reset");
        if let Some(reason) = reason {
            ui.label(format!("The separation worker stopped: {reason}"));
        }
        ui.label(
            "If reloading the page does not help, the browser may be holding outdated copies of \
             this app's files, or stored data that no longer loads.",
        );
        ui.separator();
        if ui.button("Refresh app files and reload").clicked() {
            outcome = DialogOutcome::Chosen(Reset::RefreshFiles);
        }
        ui.small("Keeps the model and the kept songs.");
        ui.add_space(6.0);
        let songs = match songs {
            1 => "1 kept song".to_string(),
            n => format!("{n} kept songs"),
        };
        let delete = egui::Button::new(
            egui::RichText::new("Delete stored data, refresh and reload").color(ui.visuals().error_fg_color),
        );
        if ui.add(delete).clicked() {
            outcome = DialogOutcome::Chosen(Reset::DeleteStored);
        }
        ui.small(format!(
            "Deletes the model (350 MB to download again) and {songs} with their stems, {stored_mb:.0} MB in all."
        ));
        ui.separator();
        if ui.button("Cancel").clicked() {
            outcome = DialogOutcome::Cancelled;
        }
    });
    if outcome == DialogOutcome::Open && modal.should_close() {
        DialogOutcome::Cancelled
    } else {
        outcome
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressOutcome {
    Waiting,
    TryAgain,
    Reload,
}

/// Shown from the choice until the page reloads, so nothing new is opened
/// or saved meanwhile.  `failure` says what could not be deleted or
/// fetched again; the reset then waits for the user instead of reloading.
pub fn progress(ctx: &egui::Context, failure: Option<&str>) -> ProgressOutcome {
    let mut outcome = ProgressOutcome::Waiting;
    egui::Modal::new(egui::Id::new("reset_progress")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        let Some(failure) = failure else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Resetting, then reloading the page…");
            });
            return;
        };
        ui.heading("Reset incomplete");
        ui.colored_label(ui.visuals().error_fg_color, failure);
        ui.horizontal(|ui| {
            if ui.button("Try again").clicked() {
                outcome = ProgressOutcome::TryAgain;
            }
            if ui.button("Reload").clicked() {
                outcome = ProgressOutcome::Reload;
            }
        });
    });
    outcome
}

/// What a failed deletion tells the user.
pub fn not_deleted(names: &[String]) -> String {
    format!(
        "Could not delete from browser storage: {}.  Another tab of this page may be using \
         them: close it and try again, or clear this site's data in the browser's settings.",
        names.join(", ")
    )
}

/// What a failed refetch tells the user.  A reload now could keep the
/// outdated copies, so it asks for a retry first.
pub fn not_refreshed(urls: &[String]) -> String {
    let names: Vec<&str> = urls.iter().map(|u| file_name(u)).collect();
    format!(
        "Could not fetch again: {}.  Check the connection and try again; reloading now may \
         keep the outdated copies.",
        names.join(", ")
    )
}

/// The last part of a URL's path, or "the page" for the page itself.
fn file_name(url: &str) -> &str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    match path.rsplit('/').next() {
        Some(name) if !name.is_empty() => name,
        _ => "the page",
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{Reset, WORKER_FILES, not_deleted, not_refreshed, urls_to_refresh};
    use js_sys::{Array, Reflect};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
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
"#)]
    extern "C" {
        fn page_url() -> String;
        #[wasm_bindgen(catch)]
        async fn fresh_page(page: &str) -> Result<JsValue, JsValue>;
        fn absolute(path: &str, base: &str) -> String;
        async fn refetch(urls: Array) -> JsValue;
        #[wasm_bindgen(catch)]
        async fn wipe_opfs() -> Result<JsValue, JsValue>;
    }

    /// Does the reset, then reloads the page.  The caller stops the worker
    /// first.  Deleting closes the page's own storage and waits for writes
    /// in progress.  If anything is left undeleted, or a file could not be
    /// fetched again, it returns what instead of reloading.
    pub async fn run(reset: Reset) -> Result<(), String> {
        if reset == Reset::DeleteStored {
            crate::store::close().await;
            let left: Vec<String> = match wipe_opfs().await {
                Ok(names) => Array::from(&names)
                    .iter()
                    .filter_map(|v| v.as_string())
                    .collect(),
                Err(e) => return Err(format!("Could not open browser storage: {e:?}")),
            };
            if !left.is_empty() {
                return Err(not_deleted(&left));
            }
            log::info!("reset: browser storage cleared");
        }
        let page = page_url();
        let fresh = fresh_page(&page).await.map_err(|e| {
            log::warn!("reset: could not fetch the page again: {e:?}");
            not_refreshed(std::slice::from_ref(&page))
        })?;
        let field =
            |k: &str| Reflect::get(&fresh, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED);
        let base = field("base").as_string().unwrap_or_else(|| page.clone());
        let preloaded: Vec<String> = Array::from(&field("preloads"))
            .iter()
            .filter_map(|v| v.as_string())
            .collect();
        let worker: Vec<String> = WORKER_FILES.iter().map(|f| absolute(f, &base)).collect();
        let urls = urls_to_refresh(&preloaded, &worker);
        let list: Array = urls.iter().map(|u| JsValue::from_str(u)).collect();
        let failed: Vec<String> = Array::from(&refetch(list).await)
            .iter()
            .filter_map(|v| v.as_string())
            .collect();
        if !failed.is_empty() {
            return Err(not_refreshed(&failed));
        }
        log::info!("reset: fetched the page and {} files again", urls.len());
        reload();
        Ok(())
    }

    pub fn reload() {
        if let Some(w) = web_sys::window() {
            let _ = w.location().reload();
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{reload, run};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_list_has_preloads_and_worker_once_each() {
        let preloaded = vec![
            "https://beachape.com/stemyla/app.js".to_string(),
            "https://beachape.com/stemyla/app_bg.wasm".to_string(),
            "https://beachape.com/stemyla/app.js".to_string(),
        ];
        let worker = vec![
            "https://beachape.com/stemyla/stem-worker_loader.js".to_string(),
            "https://beachape.com/stemyla/stem-worker.js".to_string(),
            "https://beachape.com/stemyla/stem-worker_bg.wasm".to_string(),
        ];
        assert_eq!(
            urls_to_refresh(&preloaded, &worker),
            [
                "https://beachape.com/stemyla/app.js",
                "https://beachape.com/stemyla/app_bg.wasm",
                "https://beachape.com/stemyla/stem-worker_loader.js",
                "https://beachape.com/stemyla/stem-worker.js",
                "https://beachape.com/stemyla/stem-worker_bg.wasm",
            ]
        );
    }

    /// The worker's file names are Trunk's for an unhashed worker named
    /// `stem-worker` with a loader shim; if either changes, so must
    /// `WORKER_FILES`.
    #[test]
    fn worker_file_names_match_the_trunk_build() {
        let trunk = include_str!("../Trunk.toml");
        assert!(trunk.lines().any(|l| l.trim() == "filehash = false"));
        let index = include_str!("../index.html");
        let worker_link = index
            .lines()
            .find(|l| l.contains("data-type=\"worker\""))
            .expect("worker link in index.html");
        assert!(worker_link.contains("data-bin=\"stem-worker\""));
        assert!(worker_link.contains("data-loader-shim"));
        assert_eq!(
            WORKER_FILES,
            [
                "./stem-worker_loader.js",
                "./stem-worker.js",
                "./stem-worker_bg.wasm"
            ]
        );
    }
}
