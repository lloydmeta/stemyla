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

/// The URLs to fetch again: the page, the files it preloads (the app's
/// glue, wasm and snippets, as Trunk lists them) and the worker's files,
/// each once and without a fragment.  All are absolute.
pub fn urls_to_refresh(page: &str, preloaded: &[String], worker: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let candidates = std::iter::once(page)
        .chain(preloaded.iter().map(String::as_str))
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

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{Reset, WORKER_FILES, urls_to_refresh};
    use js_sys::Array;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
export function page_url() { return location.href; }
export function preloaded() {
  return Array.from(document.querySelectorAll('link[rel="modulepreload"], link[rel="preload"]'), (l) => l.href);
}
export function absolute(path) { return new URL(path, document.baseURI).href; }
export async function refetch(urls) {
  const results = await Promise.allSettled(
    urls.map((u) => fetch(u, { cache: "reload" }).then((r) => {
      if (!r.ok) throw new Error(`HTTP ${r.status} from ${u}`);
      return r.arrayBuffer();
    }))
  );
  return results.filter((r) => r.status === "rejected").length;
}
export async function wipe_opfs() {
  const root = await navigator.storage.getDirectory();
  const names = [];
  for await (const name of root.keys()) names.push(name);
  let failed = 0;
  for (const name of names) {
    try { await root.removeEntry(name, { recursive: true }); } catch (e) { console.warn("reset: could not delete", name, e); failed++; }
  }
  return failed;
}
export function reload() { location.reload(); }
"#)]
    extern "C" {
        fn page_url() -> String;
        fn preloaded() -> Array;
        fn absolute(path: &str) -> String;
        async fn refetch(urls: Array) -> JsValue;
        #[wasm_bindgen(catch)]
        async fn wipe_opfs() -> Result<JsValue, JsValue>;
        fn reload();
    }

    /// Does the reset, then reloads the page.  The caller stops the worker
    /// first, so nothing holds a stored file open.
    pub async fn run(reset: Reset) {
        let worker: Vec<String> = WORKER_FILES.iter().map(|f| absolute(f)).collect();
        let preloaded: Vec<String> = preloaded().iter().filter_map(|v| v.as_string()).collect();
        let urls = urls_to_refresh(&page_url(), &preloaded, &worker);
        let list: Array = urls.iter().map(|u| JsValue::from_str(u)).collect();
        let failed = refetch(list).await.as_f64().unwrap_or(0.0);
        log::info!("reset: fetched {} files again, {failed} failed", urls.len());
        if reset == Reset::DeleteStored {
            match wipe_opfs().await {
                Ok(n) => log::info!(
                    "reset: browser storage cleared, {} entries not deleted",
                    n.as_f64().unwrap_or(0.0)
                ),
                Err(e) => log::warn!("reset: could not open browser storage: {e:?}"),
            }
        }
        reload();
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::run;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_list_has_page_preloads_and_worker_once_each() {
        let page = "https://beachape.com/stemyla/?selftest#top";
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
            urls_to_refresh(page, &preloaded, &worker),
            [
                "https://beachape.com/stemyla/?selftest",
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
