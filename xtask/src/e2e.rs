//! `xtask e2e <scenario> [options]`: drives the built web app (`app/dist`)
//! in Safari or Chrome through WebDriver and checks what it does.
//!
//! Scenarios (the first seven need no model and run in CI):
//! * `smoke`: drop generated stems, Play (every lane from browser storage),
//!   change the tempo.
//! * `decode`: the decode worker writes the session files of a 44.1 kHz
//!   WAV, a 48 kHz WAV and the generated stems, equal to the native decode;
//!   a reload opens a complete session as stored lanes with no decode, and decodes a
//!   session of the M4 layout (`e2e/decode.rs`).
//! * `playback`: a song restored with six stems plays every lane, "original"
//!   too, from browser storage through the read-ahead buffer, with no
//!   underrun, after a seek too (M6, `e2e/playback.rs`).
//! * `old-session`: a session of M4's layout, of 1 minute and then of 5
//!   minutes, restores through the decode worker and the stem overviews,
//!   plays its 7 lanes from storage, and the page's WebAssembly memory
//!   stays under a fixed bound and does not grow with the song (M7,
//!   `e2e/old_session.rs`).
//! * `audio-resume`: a song restored at page load plays after one click,
//!   with Web Audio behaving as in Safari; stems stored with it come back
//!   with it.
//! * `storage`: the worker's model storage across messages and workers:
//!   queue order, the cross-tab lock, stalled downloads, `?model_url=`;
//!   with `--weights`, also the paths that load a real model (GPU).
//! * `flac`: the release worker encodes a 5-minute stem as a FLAC; logs
//!   the time.
//! * `bench`: `?bench`, every matmul kernel on every model shape (WebGPU).
//! * `selftest`: `?selftest`, the GPU against the CPU reference files
//!   (`--weights`, and `xtask roformer-selftest` output).
//! * `separate`: drop a clip and the model, Separate, lanes appear
//!   (`--weights`).  The clip is kept in browser storage, so it separates
//!   on the storage path; a fallback to the memory path fails it.  The clip
//!   is a 48 kHz WAV, or with `--flac` a 96 kHz FLAC.
//! * `worker-check`: the worker's separation against PyTorch's
//!   (`--weights`, `--reference` from `tools/roformer_export.py`).
//! * `memory-budget`: separate songs of 1 and 4 minutes, each in a new
//!   browser, and print the peak memory of each and the difference
//!   (`--weights`).  M2 of `docs/plans/active/stream-from-storage.md`.
//! * `memory-budget` also prints how much the page's and the worker's
//!   WebAssembly memory grew during each separation, and fails if either
//!   is more than 4 MB (D9 of M3 in
//!   `docs/plans/active/stream-from-storage-m3.md`, step 4 of M4).  The
//!   page's growth includes its write of the mix.  A fallback to the
//!   memory path fails it.  It also checks that the page and the decode
//!   worker grow by at most 4 MB while the song opens, and that Separate
//!   writes no second mix (M5); `separate` checks that too.  Since M7, the
//!   page's growth from the separation's end to the stems shown must be at
//!   most 4 MB more for the 4-minute song than for the 1-minute song: the
//!   stems show as stored lanes, with no read-back.
//! * `stored-stems`: one worker separates 3 seconds of the synthetic mix on the
//!   memory path, then on the storage path into a session folder, from a mix
//!   file there (`--weights`).  Each stem file equals the memory path's stem as
//!   a 16-bit WAV, byte for byte.  It prints the SHA-256 of each stem, and
//!   checks them against `--hashes`.
//!
//! Options: `--browser safari|chrome`, `--dist DIR` (default `app/dist`),
//! `--out DIR` (logs and screenshots; default `target/e2e/<scenario>`),
//! `--headed` (Chrome with a window), `--swiftshader` (Chrome's software
//! WebGPU, for machines without a GPU), `--query Q` (page URL query for
//! the GPU scenarios), `--weights FILE`, `--selftest-dir DIR`,
//! `--reference DIR`, `--download`, `--flac`, `--hashes H1,...,H6` (the
//! six SHA-256s of `stored-stems`, in the worker's stem order), and click
//! positions `--play-x`,
//! `--tempo-x`, `--download-x`, `--separate-x` (CSS pixels in the top bar).
//!
//! Classic WebDriver has no console events and no init scripts, so the
//! server adds `xtask/e2e/*.js` to the page (`xtask serve --inject`): one
//! keeps the console in `window.__e2e_log`, which this runner reads.

use crate::drivers::Browser;
use crate::serve;
use crate::webdriver::{Driver, Session};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod decode;
mod old_session;
mod playback;

/// The viewport the click positions are measured in.
const VIEWPORT: (u32, u32) = (1100, 700);
/// The top bar's buttons sit on this line.
const BAR_Y: f64 = 15.0;

const SWIFTSHADER: [&str; 7] = [
    "--enable-unsafe-webgpu",
    "--enable-features=Vulkan",
    "--use-vulkan=swiftshader",
    "--use-webgpu-adapter=swiftshader",
    "--use-angle=swiftshader",
    "--enable-unsafe-swiftshader",
    "--ignore-gpu-blocklist",
];

pub struct Opts {
    pub scenario: String,
    pub browser: Browser,
    pub dist: PathBuf,
    pub out: PathBuf,
    pub headed: bool,
    pub swiftshader: bool,
    pub query: Option<String>,
    pub weights: Option<PathBuf>,
    pub selftest_dir: PathBuf,
    pub reference: Option<PathBuf>,
    pub download: bool,
    /// `separate` drops a 16-bit 96 kHz FLAC clip, not a 48 kHz WAV.
    pub flac: bool,
    /// `stored-stems` checks the stems' SHA-256s against these.
    pub hashes: Option<String>,
    pub play_x: f64,
    pub tempo_x: f64,
    pub download_x: f64,
    pub separate_x: f64,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Self> {
        let scenario = args
            .first()
            .filter(|s| !s.starts_with("--"))
            .context(
                "usage: xtask e2e <smoke|decode|playback|old-session|audio-resume|storage|flac|bench|selftest|separate|worker-check|memory-budget|stored-stems> [options]",
            )?
            .clone();
        // Defaults are in the workspace, wherever this runs from.
        let root = crate::workspace_root();
        let mut o = Self {
            out: root.join("target/e2e").join(&scenario),
            scenario,
            browser: Browser::default_here(),
            dist: root.join("app/dist"),
            headed: false,
            swiftshader: false,
            query: None,
            weights: None,
            selftest_dir: root.join("third_party/roformer/selftest"),
            reference: None,
            download: false,
            flac: false,
            hashes: None,
            // Positions in the 1100-pixel top bar with no model loaded.
            play_x: 600.0,
            // 75% on the 50..150% tempo slider while playing ("Pause" is
            // wider than "Play" and pushes the slider right).
            tempo_x: 737.0,
            download_x: 263.0,
            // "Separate 6 stems" takes the place of the model buttons.
            separate_x: 245.0,
        };
        let mut i = 1;
        while i < args.len() {
            let flag = args[i].as_str();
            let mut value = || -> Result<String> {
                i += 1;
                args.get(i)
                    .cloned()
                    .with_context(|| format!("{flag} needs a value"))
            };
            match flag {
                "--browser" => o.browser = Browser::parse(&value()?)?,
                "--dist" => o.dist = value()?.into(),
                "--out" => o.out = value()?.into(),
                "--headed" => o.headed = true,
                "--swiftshader" => o.swiftshader = true,
                "--query" => o.query = Some(value()?),
                "--weights" => o.weights = Some(value()?.into()),
                "--selftest-dir" => o.selftest_dir = value()?.into(),
                "--reference" => o.reference = Some(value()?.into()),
                "--download" => o.download = true,
                "--flac" => o.flac = true,
                "--hashes" => o.hashes = Some(value()?),
                "--play-x" => o.play_x = value()?.parse()?,
                "--tempo-x" => o.tempo_x = value()?.parse()?,
                "--download-x" => o.download_x = value()?.parse()?,
                "--separate-x" => o.separate_x = value()?.parse()?,
                other => bail!("unknown option {other}"),
            }
            i += 1;
        }
        Ok(o)
    }

    fn weights(&self) -> Result<&Path> {
        let w = self
            .weights
            .as_deref()
            .context("this scenario needs the model: --weights <bs_roformer_sw.*.safetensors>")?;
        if !w.is_file() {
            bail!("no model file at {}", w.display());
        }
        Ok(w)
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let o = Opts::parse(args)?;
    if !o.dist.join("index.html").is_file() {
        bail!(
            "{} has no index.html (build the app first: cd app && trunk build --release)",
            o.dist.display()
        );
    }
    std::fs::create_dir_all(&o.out)?;
    match o.scenario.as_str() {
        "smoke" => smoke(&o),
        "decode" => decode::run(&o),
        "playback" => playback::run(&o),
        "old-session" => old_session::run(&o),
        "audio-resume" => audio_resume(&o),
        "storage" => storage(&o),
        "flac" => flac(&o),
        "bench" => bench(&o),
        "selftest" => selftest(&o),
        "separate" => separate(&o),
        "worker-check" => worker_check(&o),
        "memory-budget" => memory_budget(&o),
        "stored-stems" => stored_stems(&o),
        other => bail!("unknown scenario {other}"),
    }
}

/// The app's static files plus `mounts`, on a free port, for the rest of
/// the process; returns its base URL.
fn start_server(dist: &Path, mut mounts: serve::Mounts, inject: &[&str]) -> Result<String> {
    let helpers = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/e2e"));
    mounts.push(("/__e2e/".into(), helpers));
    mounts.push(("/".into(), dist.to_path_buf()));
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let base = format!("http://{}", listener.local_addr()?);
    let inject = inject.iter().map(|s| format!("/__e2e/{s}")).collect();
    std::thread::spawn(move || serve::serve_with(listener, mounts, inject));
    Ok(base)
}

/// One browser on one page, with the page's console as it arrives.
/// Its log is written to `<out>/<scenario>.log` when it is dropped, so a
/// scenario that stops early still leaves one.
struct Page {
    session: Session,
    _driver: Driver,
    base: String,
    t0: Instant,
    log: Vec<String>,
    out: PathBuf,
    scenario: String,
    failures: Vec<String>,
}

impl Page {
    /// Starts the browser and opens `path` (relative to the server).
    fn open(o: &Opts, base: String, chrome_args: &[&str], path: &str) -> Result<Self> {
        let driver = Driver::start(o.browser)?;
        let mut args: Vec<String> = chrome_args.iter().map(|s| s.to_string()).collect();
        if o.swiftshader {
            args.extend(SWIFTSHADER.iter().map(|s| s.to_string()));
        }
        if !o.headed {
            args.push("--headless=new".into());
        }
        // Without a usable GPU (CI), Chrome refuses WebGL, which the app
        // draws with, unless it may fall back to SwiftShader.
        args.push("--enable-unsafe-swiftshader".into());
        args.push(format!("--window-size={},{}", VIEWPORT.0, VIEWPORT.1));
        let session = Session::new(&driver, o.browser, &args)?;
        let tries = session.set_viewport(VIEWPORT.0, VIEWPORT.1)?;
        let mut page = Self {
            session,
            _driver: driver,
            base,
            t0: Instant::now(),
            log: Vec::new(),
            out: o.out.clone(),
            scenario: o.scenario.clone(),
            failures: Vec::new(),
        };
        page.note(&format!("viewport: {}", tries.join("; ")));
        page.session.goto(&page.url(path))?;
        Ok(page)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base, path.trim_start_matches('/'))
    }

    fn note(&mut self, text: &str) {
        let line = format!("[{:.1}s] {text}", self.t0.elapsed().as_secs_f64());
        println!("{line}");
        self.log.push(line);
    }

    /// Takes the page's new console lines into the log.
    fn poll(&mut self) -> Result<()> {
        let lines = self.session.execute(
            "return window.__e2e_log ? window.__e2e_log.splice(0) : [];",
            json!([]),
        )?;
        for l in lines.as_array().into_iter().flatten() {
            let text = l.as_str().unwrap_or_default().to_string();
            // Chromium's software GL notes, not the page's.
            if !text.contains("GL Driver") {
                self.note(&text);
            }
        }
        Ok(())
    }

    /// The position in the log from which [`wait_for`](Self::wait_for)
    /// should look.
    fn mark(&self) -> usize {
        self.log.len()
    }

    /// The first log line after `from` that `matches`, waiting up to `secs`.
    fn wait_for(
        &mut self,
        from: usize,
        secs: u64,
        matches: impl Fn(&str) -> bool,
    ) -> Result<Option<String>> {
        let until = Instant::now() + Duration::from_secs(secs);
        loop {
            self.poll()?;
            if let Some(l) = self.log[from.min(self.log.len())..]
                .iter()
                .find(|l| matches(l))
            {
                return Ok(Some(l.clone()));
            }
            if Instant::now() > until {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Lets the page run for `secs`, collecting its console.
    fn idle(&mut self, secs: f64) -> Result<()> {
        let until = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < until {
            self.poll()?;
            std::thread::sleep(Duration::from_millis(250));
        }
        Ok(())
    }

    fn check(&mut self, ok: bool, what: &str) {
        if ok {
            self.note(&format!("ok: {what}"));
        } else {
            self.note(&format!("FAIL: {what}"));
            self.failures.push(what.to_string());
        }
    }

    fn click(&mut self, x: f64, what: &str) -> Result<()> {
        self.note(&format!("click {what} at ({x}, {BAR_Y})"));
        self.session.click(x, BAR_Y)
    }

    fn reload(&mut self) -> Result<()> {
        self.poll()?;
        self.note("reload");
        self.session.refresh()
    }

    fn screenshot(&mut self, name: &str) -> Result<()> {
        let path = self.out.join(format!("{name}.png"));
        self.session.screenshot(&path)?;
        self.note(&format!("screenshot {}", path.display()));
        Ok(())
    }

    /// Drops files (URLs on the server, with names and types) on the app's
    /// canvas the way a user's drop arrives.
    fn drop_files(&mut self, files: &[(&str, &str, &str)]) -> Result<()> {
        let files: Vec<Value> = files
            .iter()
            .map(|(url, name, kind)| json!({ "url": url, "name": name, "type": kind }))
            .collect();
        let dropped = self.session.execute_async(
            r#"const [files, done] = arguments;
            (async () => {
              const dt = new DataTransfer();
              for (const f of files) {
                const b = await (await fetch(f.url)).blob();
                dt.items.add(new File([b], f.name, { type: f.type }));
              }
              const c = document.querySelector("canvas");
              const r = c.getBoundingClientRect();
              const opts = { bubbles: true, cancelable: true, dataTransfer: dt, clientX: r.width / 2, clientY: r.height / 2 };
              for (const t of ["dragenter", "dragover", "drop"]) c.dispatchEvent(new DragEvent(t, opts));
              return dt.files.length;
            })().then(done, (e) => done(`error: ${e}`));"#,
            json!([files]),
            Duration::from_secs(300),
        )?;
        self.note(&format!("dropped {dropped} files"));
        if dropped.as_u64() != Some(files.len() as u64) {
            bail!("the drop failed: {dropped}");
        }
        Ok(())
    }

    fn errors(&self) -> Vec<String> {
        page_errors(&self.log)
    }

    /// An error naming every failed check.
    fn finish(mut self) -> Result<()> {
        let _ = self.poll();
        let name = self.scenario.clone();
        if self.failures.is_empty() {
            println!("{name}: passed");
            Ok(())
        } else {
            bail!("{name} FAILED: {}", self.failures.join("; "))
        }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        let path = self.out.join(format!("{}.log", self.scenario));
        let written = std::fs::File::create(&path)
            .and_then(|mut f| self.log.iter().try_for_each(|l| writeln!(f, "{l}")));
        match written {
            Ok(()) => println!("log: {}", path.display()),
            Err(e) => eprintln!("could not write {}: {e}", path.display()),
        }
    }
}

/// Log lines that report a page error: uncaught ones, `console.error`, and
/// the app's `log::error!`, which eframe's logger sends to `console.warn`
/// as "ERROR: ...".
fn page_errors(log: &[String]) -> Vec<String> {
    log.iter().filter(|l| is_page_error(l)).cloned().collect()
}

fn is_page_error(line: &str) -> bool {
    line.contains("PAGE ERROR")
        || line.contains("console.error:")
        || line.contains("console.warn: ERROR:")
}

/// The number after `key` in `line` ("playhead 1234" -> 1234).
fn number_after(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Whether `line` reports a playhead at least one second (44 100 frames)
/// into the song.
fn played_a_second(line: &str) -> bool {
    audio_stats(&[line.to_string()], 0)
        .first()
        .is_some_and(|(_, head)| *head >= 44_100.0)
}

/// (tempo, playhead) of every "audio stats" line after `from`.
fn audio_stats(log: &[String], from: usize) -> Vec<(f64, f64)> {
    log[from.min(log.len())..]
        .iter()
        .filter(|l| l.contains("audio stats: ") && l.contains(" callbacks of "))
        .filter_map(|l| Some((number_after(l, "tempo ")?, number_after(l, "playhead ")?)))
        .collect()
}

/// Writes the synthetic song's stems (`xtask gen-song`) under `out`.
fn synth_stems(out: &Path) -> Result<PathBuf> {
    let dir = out.join("stems");
    if !dir.join("drums.wav").is_file() {
        crate::synth::write_song(&crate::synth::generate(), &dir, false)?;
    }
    Ok(dir)
}

/// The page loads with WebGL2, a multi-file drop opens
/// the stems, Play starts Web Audio and the playhead, and the tempo slider
/// changes the tempo; no page errors.
fn smoke(o: &Opts) -> Result<()> {
    let stems = synth_stems(&o.out)?;
    let base = start_server(
        &o.dist,
        vec![("/stems/".into(), stems)],
        &["console.js", "audio_contexts.js"],
    )?;
    let mut p = Page::open(
        o,
        base,
        &["--autoplay-policy=no-user-gesture-required"],
        "index.html",
    )?;
    let ready = p.wait_for(0, 60, |l| l.contains("separator: "))?;
    p.check(ready.is_some(), "the app started");
    p.screenshot("smoke_empty")?;
    let webgl2 = p.session.execute(
        "return !!document.createElement('canvas').getContext('webgl2');",
        json!([]),
    )?;
    p.check(webgl2 == json!(true), "WebGL2 is available");

    let names = ["drums", "bass", "guitar", "keys", "lead"];
    let urls: Vec<String> = names.iter().map(|n| format!("/stems/{n}.wav")).collect();
    let files: Vec<String> = names.iter().map(|n| format!("{n}.wav")).collect();
    let drop: Vec<(&str, &str, &str)> = urls
        .iter()
        .zip(&files)
        .map(|(u, f)| (u.as_str(), f.as_str(), "audio/wav"))
        .collect();
    let from = p.mark();
    p.drop_files(&drop)?;
    let kept = p.wait_for(from, 60, |l| {
        l.contains("song kept in browser storage as session")
    })?;
    p.check(kept.is_some(), "the dropped stems open and are kept");
    p.idle(1.0)?;
    p.screenshot("smoke_loaded")?;

    let from = p.mark();
    p.click(o.play_x, "Play")?;
    let played = p.wait_for(from, 60, played_a_second)?;
    p.check(played.is_some(), "the playhead moves after Play");
    // M6: every lane of the dropped stems plays from browser storage.
    let stored = p.wait_for(from, 60, |l| playback::playback_line(l).is_some())?;
    let line = stored.as_deref().and_then(playback::playback_line);
    p.check(
        playback::all_from_storage(line.as_ref(), 5),
        &format!("every lane plays from storage, with no underrun: {line:?}"),
    );
    let contexts = p.session.execute(
        "return (window.__e2e_ctxs || []).map((c) => ({ state: c.state, time: c.currentTime }));",
        json!([]),
    )?;
    p.note(&format!("audio contexts: {contexts}"));
    let running = contexts.as_array().is_some_and(|cs| {
        cs.iter()
            .any(|c| c["state"] == "running" && c["time"].as_f64() > Some(0.5))
    });
    p.check(running, "Web Audio runs (an AudioContext is running)");

    let from = p.mark();
    p.click(o.tempo_x, "the tempo slider at 75%")?;
    // A stats line from just before the click may still be on its way.
    let near_75 = |l: &str| {
        audio_stats(&[l.to_string()], 0)
            .first()
            .is_some_and(|(t, _)| (0.70..=0.80).contains(t))
    };
    let tempo = p.wait_for(from, 60, near_75)?;
    p.check(tempo.is_some(), "the tempo slider sets about 75%");
    p.idle(3.0)?;
    p.screenshot("smoke_playing")?;
    let errors = p.errors();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish()
}

/// A song stored, then restored when the page
/// loads, which creates its audio output before any user gesture; one
/// click on Play must then start playback.  Chrome is made to behave like
/// Safari (contexts start suspended; `resume()` only during a click);
/// Safari is tested as it is.  Then stems added to the stored session
/// must come back with the song at the next load.
fn audio_resume(o: &Opts) -> Result<()> {
    let stems = synth_stems(&o.out)?;
    // One file that is not called "mix" (the app skips a mix).
    let song = o.out.join("song");
    std::fs::create_dir_all(&song)?;
    std::fs::copy(stems.join("mix.wav"), song.join("song.wav"))?;
    let mut inject = vec!["console.js"];
    if o.browser == Browser::Chrome {
        inject.push("audio_like_safari.js");
    }
    let base = start_server(&o.dist, vec![("/input/".into(), song)], &inject)?;
    let mut p = Page::open(
        o,
        base,
        &["--autoplay-policy=user-gesture-required"],
        "index.html",
    )?;
    p.wait_for(0, 60, |l| l.contains("separator: "))?;
    let from = p.mark();
    p.drop_files(&[("/input/song.wav", "song.wav", "audio/wav")])?;
    let kept = p.wait_for(from, 60, |l| l.contains("song kept in browser storage"))?;
    p.check(kept.is_some(), "the song is kept in browser storage");

    p.reload()?;
    let from = p.mark();
    let restored = p.wait_for(from, 60, |l| {
        l.contains("Restored \"song\" from this browser")
    })?;
    p.check(restored.is_some(), "the song is restored at page load");
    p.idle(3.0)?;
    let before = audio_stats(&p.log, from);
    p.note(&format!(
        "before the click: {} audio stats lines",
        before.len()
    ));
    p.check(
        before.iter().all(|(_, head)| *head == 0.0),
        "nothing plays before the click",
    );

    let from = p.mark();
    p.click(o.play_x, "Play")?;
    // A busy machine plays slower than real time, so this waits for the
    // playhead rather than for a fixed time.
    let played = p.wait_for(from, 60, played_a_second)?;
    p.check(played.is_some(), "one click on Play starts playback");
    p.screenshot("audio_resume")?;

    // Separated stems in the stored session (written as `store::save_stems`
    // writes them) come back with the song at the next page load.
    in_page(
        &mut p,
        "const root = await navigator.storage.getDirectory();
        const sessions = await root.getDirectoryHandle('sessions');
        const id = (await (await (await sessions.getFileHandle('current')).getFile()).text()).trim();
        const dir = await sessions.getDirectoryHandle(id);
        const metaFile = await dir.getFileHandle('meta');
        const meta = (await (await metaFile.getFile()).text()).trim();
        const rate = 44100, frames = 1000;
        const wav = (f) => {
          const b = new DataView(new ArrayBuffer(44 + frames * 4));
          const s = (o, t) => [...t].forEach((c, i) => b.setUint8(o + i, c.charCodeAt(0)));
          s(0, 'RIFF'); b.setUint32(4, 36 + frames * 4, true); s(8, 'WAVEfmt ');
          b.setUint32(16, 16, true); b.setUint16(20, 1, true); b.setUint16(22, 2, true);
          b.setUint32(24, rate, true); b.setUint32(28, rate * 4, true); b.setUint16(32, 4, true); b.setUint16(34, 16, true);
          s(36, 'data'); b.setUint32(40, frames * 4, true);
          for (let i = 0; i < frames * 2; i++) b.setInt16(44 + 2 * i, Math.round(8000 * Math.sin(i * f)), true);
          return b.buffer;
        };
        const stems = await dir.getDirectoryHandle('stems', { create: true });
        for (const [n, f] of [['guitar', 0.01], ['drums', 0.03]]) {
          const w = await (await stems.getFileHandle(`${n}.wav`, { create: true })).createWritable();
          await w.write(wav(f)); await w.close();
        }
        const w = await metaFile.createWritable();
        await w.write(meta + '\\nstem\\tguitar\\nstem\\tdrums\\n'); await w.close();",
        json!([]),
        30,
    )?;
    p.reload()?;
    let from = p.mark();
    let with_stems = p.wait_for(from, 60, |l| {
        l.contains("Restored \"song\" and its separated stems from this browser")
    })?;
    p.check(
        with_stems.is_some(),
        "a stored song with stems comes back with them at page load",
    );
    p.finish()
}

/// A local server for the storage cases that need a misbehaving one:
/// `/slow` sends 1000 bytes over about 3 s; `/stall` sends 10 of 1000000
/// bytes and then nothing for 90 s.  Returns its base URL.
fn drip_server() -> Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let base = format!("http://{}", listener.local_addr()?);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = drip(stream);
            });
        }
    });
    Ok(base)
}

fn drip(mut stream: TcpStream) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? <= 2 {
            break;
        }
    }
    let stall = request.contains("/stall");
    let length = if stall { 1_000_000 } else { 1000 };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    )?;
    if stall {
        stream.write_all(&[0; 10])?;
        stream.flush()?;
        std::thread::sleep(Duration::from_secs(90));
        return Ok(());
    }
    for _ in 0..10 {
        stream.write_all(&[0; 100])?;
        stream.flush()?;
        std::thread::sleep(Duration::from_millis(300));
    }
    Ok(())
}

/// Runs `script` (an async function body using the page's `e2e` helpers;
/// its result is returned) in the worker page.
fn in_page(p: &mut Page, script: &str, args: Value, secs: u64) -> Result<Value> {
    let wrapped = format!(
        "const done = arguments[arguments.length - 1];
        (async (...args) => {{ {script} }})(...arguments).then(
          (r) => done({{ ok: r }}),
          (e) => done({{ error: String(e && e.stack || e) }}));"
    );
    let r = p
        .session
        .execute_async(&wrapped, args, Duration::from_secs(secs))?;
    p.poll()?;
    if let Some(e) = r.get("error") {
        bail!("in the page: {}", e.as_str().unwrap_or_default());
    }
    Ok(r.get("ok").cloned().unwrap_or(Value::Null))
}

fn texts(replies: &Value) -> Vec<String> {
    replies
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            format!(
                "{}{}: {}",
                r["type"].as_str().unwrap_or(""),
                r["of"]
                    .as_str()
                    .map(|o| format!(" of {o}"))
                    .unwrap_or_default(),
                r["text"].as_str().unwrap_or("")
            )
        })
        .collect()
}

/// The worker's model storage seen from outside: the cases that need a
/// whole worker, two workers, or the app.
fn storage(o: &Opts) -> Result<()> {
    let drip = drip_server()?;
    let mut mounts = Vec::new();
    if let Some(w) = &o.weights {
        let dir = w.parent().context("model file without a folder")?;
        mounts.push(("/weights/".to_string(), dir.to_path_buf()));
    }
    let base = start_server(&o.dist, mounts, &["console.js"])?;
    let mut p = Page::open(o, base, &[], "__e2e/blank.html")?;
    let not_a_model = "the downloaded file is not a usable BS-RoFormer SW model";
    let slow = format!("{drip}/slow");

    p.note("case: messages are answered in the order sent (a removal waits for a download)");
    let r = in_page(
        &mut p,
        "await e2e.wipe(); return e2e.ask([{ type: 'download', model_url: args[0] }, { type: 'forget_model' }]);",
        json!([slow]),
        60,
    )?;
    let t = texts(&r["replies"]);
    p.note(&format!("replies: {t:?}"));
    p.check(
        t.len() == 2 && t[0].contains(not_a_model) && t[1].contains("model removed"),
        "the download is answered before the removal sent after it",
    );
    // One worker's own messages queue for their turn; they never wait for
    // the storage lock as if another tab held it.
    p.check(
        r["statuses"] == json!([]),
        "no wait for another tab is reported",
    );
    p.check(r["files"] == json!({}), "nothing is left in storage");

    p.note("case: a second download waits for the first");
    let r = in_page(
        &mut p,
        "await e2e.wipe(); const m = { type: 'download', model_url: args[0] }; return e2e.ask([m, m]);",
        json!([slow]),
        60,
    )?;
    let t = texts(&r["replies"]);
    let at: Vec<u64> = r["replies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x["at"].as_u64())
        .collect();
    p.note(&format!("replies: {t:?} at {at:?} ms"));
    p.check(
        t.len() == 2 && t.iter().all(|x| x.contains(not_a_model)),
        "both downloads finish cleanly (no shared partial file)",
    );
    p.check(
        at.len() == 2 && at[1] >= at[0] + 2_000,
        "the second download runs after the first (each takes about 3 s)",
    );
    p.check(
        r["statuses"] == json!([]),
        "no wait for another tab is reported",
    );

    p.note("case: another worker waits for the storage lock, and says so");
    let r = in_page(
        &mut p,
        "await e2e.wipe();
        const t0 = performance.now();
        const mine = e2e.ask([{ type: 'download', model_url: args[0] }]);
        await new Promise((r) => setTimeout(r, 1000));
        const other = e2e.ask([{ type: 'init' }]);
        const [m, x] = await Promise.all([mine, other]);
        const end = (a) => a.t0 + a.replies[0].at;
        return { mine: m.replies, other: x.replies, statuses: x.statuses, other_after: end(x) >= end(m) };",
        json!([slow]),
        60,
    )?;
    p.note(&format!("result: {r}"));
    let waited = r["statuses"].as_array().is_some_and(|s| {
        s.iter().any(|x| {
            x["text"]
                .as_str()
                .is_some_and(|t| t.starts_with("Waiting for another tab"))
        })
    });
    p.check(waited, "the second worker reports that it is waiting");
    let other_text = r["other"][0]["text"].as_str().unwrap_or_default();
    p.check(
        other_text == "no model cached in this browser" && r["other_after"] == json!(true),
        "the second worker answers once the first is done",
    );

    p.note("case: a stalled download gives up after 60 s and leaves nothing");
    let r = in_page(
        &mut p,
        "await e2e.wipe(); return e2e.ask([{ type: 'download', model_url: args[0] }]);",
        json!([format!("{drip}/stall")]),
        120,
    )?;
    let t = texts(&r["replies"]);
    p.note(&format!("replies: {t:?}"));
    p.check(
        t.len() == 1
            && t[0].contains("model download failed")
            && t[0].contains("no data from the server for 60 s"),
        "the stalled download times out",
    );
    p.check(r["files"] == json!({}), "nothing is left in storage");

    if let Some(w) = &o.weights {
        let name = w
            .file_name()
            .and_then(|n| n.to_str())
            .context("model file name")?;
        storage_with_model(&mut p, &format!("/weights/{name}"))?;
    } else {
        p.note("no --weights: skipping the cases that load a real model");
    }

    // `?model_url=`: the app's Download model button fetches that file.
    p.note("case: ?model_url= points Download model at another file");
    in_page(&mut p, "await e2e.wipe();", json!([]), 30)?;
    p.session
        .goto(&p.url("index.html?model_url=/__e2e/blank.html"))?;
    let from = p.mark();
    p.wait_for(from, 60, |l| l.contains("separator: "))?;
    p.click(o.download_x, "Download model")?;
    let got = p.wait_for(from, 60, |l| l.contains(not_a_model))?;
    p.check(
        got.is_some(),
        "the app downloads from ?model_url= and refuses a non-model",
    );
    p.finish()
}

/// Storage cases that load the real model (a GPU, and `--weights`).
fn storage_with_model(p: &mut Page, model: &str) -> Result<()> {
    let opts = json!({ "mm": "s64" });
    p.note("case: a download is committed, and the next start loads it");
    let r = in_page(
        p,
        "await e2e.wipe();
        const d = await e2e.ask([{ type: 'download', model_url: args[0], ...args[1] }]);
        const i = await e2e.ask([{ type: 'init', ...args[1] }]);
        return { download: d.replies, files: d.files, init: i.replies };",
        json!([model, opts]),
        1200,
    )?;
    p.note(&format!("result: {r}"));
    p.check(
        r["download"][0]["ok"] == json!(true) && r["download"][0]["stored"] == json!(true),
        "the download loads and is stored",
    );
    p.check(
        r["files"].as_object().is_some_and(|f| {
            f.contains_key("bs_roformer_sw.safetensors") && !f.contains_key("download.part")
        }),
        "only the committed model is left",
    );
    p.check(
        r["init"][0]["ok"] == json!(true),
        "a new worker loads the stored model",
    );

    p.note("case: a download over a stored model replaces it");
    let r = in_page(
        p,
        "return e2e.ask([{ type: 'download', model_url: args[0], ...args[1] }]);",
        json!([model, opts]),
        1200,
    )?;
    p.check(
        r["replies"][0]["ok"] == json!(true)
            && r["replies"][0]["stored"] == json!(true)
            && r["replies"][0]["text"].as_str().is_some_and(|t| {
                t.contains("saved in browser storage") && !t.contains("could not")
            })
            && r["files"]
                .as_object()
                .is_some_and(|f| f.len() <= 2 && !f.contains_key("download.part")),
        "the stored model is replaced, with no partial file left",
    );

    p.note("case: a failed load keeps the model loaded before it");
    let r = in_page(
        p,
        "await e2e.wipe();
        return e2e.ask([{ type: 'download', model_url: args[0], ...args[1] }, { type: 'model', bytes: new Uint8Array([1, 2, 3]) }]);",
        json!([model, opts]),
        1200,
    )?;
    p.note(&format!("replies: {:?}", texts(&r["replies"])));
    p.check(
        r["replies"][1]["ok"] == json!(false)
            && r["replies"][1]["loaded"] == json!(true)
            && r["replies"][1]["stored"] == json!(true),
        "the earlier model stays loaded and stored",
    );

    p.note("case: a download that is not safetensors at all keeps the model loaded");
    let r = in_page(
        p,
        "await e2e.wipe();
        return e2e.ask([{ type: 'download', model_url: args[0], ...args[1] }, { type: 'download', model_url: '/__e2e/blank.html', ...args[1] }]);",
        json!([model, opts]),
        1200,
    )?;
    p.note(&format!("replies: {:?}", texts(&r["replies"])));
    p.check(
        r["replies"][1]["ok"] == json!(false)
            && r["replies"][1]["loaded"] == json!(true)
            && r["replies"][1]["stored"] == json!(true),
        "the file is refused before the model loaded before it is freed",
    );

    p.note("case: a safetensors file that is not the model unloads the one before it");
    let r = in_page(
        p,
        "await e2e.wipe();
        const empty = new Uint8Array([2, 0, 0, 0, 0, 0, 0, 0, 123, 125]);
        return e2e.ask([{ type: 'download', model_url: args[0], ...args[1] }, { type: 'model', bytes: empty, ...args[1] }]);",
        json!([model, opts]),
        1200,
    )?;
    p.note(&format!("replies: {:?}", texts(&r["replies"])));
    p.check(
        r["replies"][1]["ok"] == json!(false)
            && r["replies"][1]["loaded"] == json!(false)
            && r["replies"][1]["stored"] == json!(true)
            && r["files"]
                .as_object()
                .is_some_and(|f| !f.contains_key("download.part")),
        "two models are never on the GPU at once, and the stored one is kept",
    );

    p.note("case: a picked file is committed, over a stored model");
    let r = in_page(
        p,
        "await e2e.plant('bs_roformer_sw.safetensors', [7]);
        const bytes = new Uint8Array(await (await fetch(args[0])).arrayBuffer());
        return e2e.ask([{ type: 'model', bytes, ...args[1] }]);",
        json!([model, opts]),
        1200,
    )?;
    p.check(
        r["replies"][0]["ok"] == json!(true)
            && r["replies"][0]["stored"] == json!(true)
            && r["replies"][0]["text"].as_str().is_some_and(|t| {
                t.contains("saved in browser storage") && !t.contains("could not")
            })
            && r["files"].as_object().is_some_and(|f| {
                !f.contains_key("download.part")
                    && f["bs_roformer_sw.safetensors"].as_u64() > Some(1)
            }),
        "the picked file loads and replaces the stored one",
    );
    Ok(())
}

/// The page URL's query for a GPU scenario.
fn query(o: &Opts, default: &str) -> String {
    o.query.clone().unwrap_or_else(|| default.to_string())
}

/// `?bench` compiles and runs every matmul kernel on every
/// model shape, with no WebGPU error.  No model needed.
fn bench(o: &Opts) -> Result<()> {
    let base = start_server(&o.dist, Vec::new(), &["console.js"])?;
    let q = query(o, "bench&bench_quick=1&mm=s64");
    let mut p = Page::open(
        o,
        base,
        &["--enable-unsafe-webgpu"],
        &format!("index.html?{q}"),
    )?;
    // Any error ends the wait (no WebGPU adapter, say), not only the verdict.
    let verdict = p.wait_for(0, 4 * 3600, |l| {
        l.contains("bench finished") || l.contains("WebGPU error") || is_page_error(l)
    })?;
    p.note(&format!("verdict: {verdict:?}"));
    p.check(
        verdict
            .as_deref()
            .is_some_and(|v| v.contains("bench finished")),
        "the bench finishes",
    );
    let gpu_errors = p.log.iter().filter(|l| l.contains("WebGPU error")).count();
    p.check(gpu_errors == 0, "no WebGPU errors");
    p.finish()
}

/// Mounts the model's folder at `/weights/`; returns the model's URL.
fn weights_mount(o: &Opts, mounts: &mut serve::Mounts) -> Result<String> {
    let w = o.weights()?;
    let dir = w.parent().context("model file without a folder")?;
    mounts.push(("/weights/".into(), dir.to_path_buf()));
    let name = w
        .file_name()
        .and_then(|n| n.to_str())
        .context("model file name")?;
    Ok(format!("/weights/{name}"))
}

/// Drops the model file on the app, which hands it to the worker.
fn drop_model(p: &mut Page, url: &str) -> Result<()> {
    let name = url
        .rsplit('/')
        .next()
        .unwrap_or("model.safetensors")
        .to_string();
    p.drop_files(&[(url, &name, "application/octet-stream")])
}

/// The in-app GPU self-test's verdict, with the model
/// dropped on the page (the file picker cannot be driven over WebDriver).
fn selftest(o: &Opts) -> Result<()> {
    let mut mounts = vec![("/selftest/".to_string(), o.selftest_dir.clone())];
    if !o.selftest_dir.join("stages.safetensors").is_file() {
        bail!(
            "no reference files in {} (xtask roformer-selftest makes them; --selftest-dir)",
            o.selftest_dir.display()
        );
    }
    let model = weights_mount(o, &mut mounts)?;
    let base = start_server(&o.dist, mounts, &["console.js"])?;
    let q = query(o, "selftest=stages&step_limit=600&mm=s64");
    let mut p = Page::open(
        o,
        base,
        &["--enable-unsafe-webgpu"],
        &format!("index.html?{q}"),
    )?;
    p.wait_for(0, 60, |l| l.contains("separator: "))?;
    drop_model(&mut p, &model)?;
    // A model that does not load never starts the self-test, so its
    // message ends the wait too.
    let verdict = p.wait_for(0, 4 * 3600, |l| {
        l.contains("selftest passed")
            || l.contains("selftest FAILED")
            || l.contains("could not be loaded")
            || l.contains("not a BS-RoFormer")
            || is_page_error(l)
    })?;
    p.note(&format!("verdict: {verdict:?}"));
    p.check(
        verdict
            .as_deref()
            .is_some_and(|v| v.contains("selftest passed")),
        "the GPU self-test passes",
    );
    p.finish()
}

/// The `separate` scenario's clip: its file, its media type and its rate.
/// Neither rate is the model's, so the page must convert the clip before
/// it separates (decision 0012).
struct Clip {
    file: &'static str,
    media_type: &'static str,
    rate: u32,
}

const WAV_CLIP: Clip = Clip {
    file: "clip.wav",
    media_type: "audio/wav",
    rate: 48_000,
};

const FLAC_CLIP: Clip = Clip {
    file: "clip.flac",
    media_type: "audio/flac",
    rate: 96_000,
};

/// A two-second clip of the synthetic mix, for `separate`, in the folder it
/// returns.
fn clip(out: &Path, clip: &Clip) -> Result<PathBuf> {
    let song = crate::synth::generate();
    let n = 2 * crate::synth::SAMPLE_RATE as usize;
    let dir = out.join("clip");
    std::fs::create_dir_all(&dir)?;
    let at_clip_rate =
        |x: &[f32]| engine::resample::to_rate(x.to_vec(), crate::synth::SAMPLE_RATE, clip.rate);
    let (left, right) = (
        at_clip_rate(&song.mix.left[..n])?,
        at_clip_rate(&song.mix.right[..n])?,
    );
    let bytes = if clip.file.ends_with(".flac") {
        flac_16(&left, &right, clip.rate)?
    } else {
        engine::wav::encode_pcm16(&left, &right, clip.rate)
    };
    std::fs::write(dir.join(clip.file), bytes)?;
    Ok(dir)
}

/// A 16-bit stereo FLAC, from `flacenc`.
fn flac_16(left: &[f32], right: &[f32], rate: u32) -> Result<Vec<u8>> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    const BLOCK: usize = 4096;
    let q = |x: f32| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i32;
    let samples: Vec<i32> = left
        .iter()
        .zip(right)
        .flat_map(|(l, r)| [q(*l), q(*r)])
        .collect();
    let source = flacenc::source::MemSource::from_samples(&samples, 2, 16, rate as usize);
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| anyhow::anyhow!("FLAC config: {e:?}"))?;
    let mut stream = flacenc::encode_with_fixed_block_size(&config, source, BLOCK)
        .map_err(|e| anyhow::anyhow!("FLAC encoder: {e:?}"))?;
    // flacenc gives the short last block as the minimum, which RFC 9639
    // (8.2) excludes, and Symphonia then rejects every frame.
    stream
        .stream_info_mut()
        .set_block_sizes(BLOCK, BLOCK)
        .map_err(|e| anyhow::anyhow!("FLAC block sizes: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| anyhow::anyhow!("FLAC file: {e:?}"))?;
    Ok(sink.into_inner())
}

/// Drop a clip (a 48 kHz WAV, or with `--flac` a 96 kHz FLAC), which opens
/// at 44.1 kHz, and the model, press Separate, and the six lanes appear;
/// screenshots along the way.
fn separate(o: &Opts) -> Result<()> {
    let c = if o.flac { FLAC_CLIP } else { WAV_CLIP };
    let mut mounts = vec![("/test/".to_string(), clip(&o.out, &c)?)];
    let model = weights_mount(o, &mut mounts)?;
    let base = start_server(&o.dist, mounts, &["console.js"])?;
    let q = query(o, "chunk=16384&overlap=1&mm=s64");
    let mut p = Page::open(
        o,
        base,
        &[
            "--enable-unsafe-webgpu",
            "--autoplay-policy=no-user-gesture-required",
        ],
        &format!("index.html?{q}"),
    )?;
    p.wait_for(0, 60, |l| l.contains("separator: "))?;
    p.screenshot("separate_empty")?;
    let from = p.mark();
    p.drop_files(&[(&format!("/test/{}", c.file), c.file, c.media_type)])?;
    // Two seconds at 44.1 kHz are 88,200 frames.
    let opened = p.wait_for(from, 60, |l| l.contains("song: opened"))?;
    p.note(&format!("opened: {opened:?}"));
    p.check(
        opened
            .as_deref()
            .is_some_and(|l| l.contains("at 44100 Hz") && l.contains("(88200 frames)")),
        &format!("the {} Hz clip opens at 44.1 kHz", c.rate),
    );
    let kept = p.wait_for(from, 60, |l| l.contains("song kept in browser storage"))?;
    p.check(kept.is_some(), "the clip opens");
    p.screenshot("separate_song")?;
    let from = p.mark();
    drop_model(&mut p, &model)?;
    let loaded = p.wait_for(from, 1200, |l| {
        l.contains("separator: model loaded")
            || l.contains("could not be loaded")
            || l.contains("not a BS-RoFormer")
            || is_page_error(l)
    })?;
    p.note(&format!("model: {loaded:?}"));
    p.check(
        loaded
            .as_deref()
            .is_some_and(|l| l.contains("model loaded")),
        "the model loads",
    );
    p.idle(1.0)?;
    p.screenshot("separate_model")?;
    let from = p.mark();
    p.click(o.separate_x, "Separate 6 stems")?;
    let done = p.wait_for(from, 4 * 3600, |l| {
        (l.contains("separator: ") && l.contains(" stems in ")) || is_page_error(l)
    })?;
    p.note(&format!("separated: {done:?}"));
    p.check(
        done.as_deref()
            .is_some_and(|l| l.contains("separator: 6 stems in")),
        "six stems arrive",
    );
    p.check(
        p.log[from..]
            .iter()
            .any(|l| l.contains("separator: separating on the storage path")),
        "the stored clip separates on the storage path",
    );
    let (ok, what) = no_fallback_check(&p.log[from..]);
    p.check(ok, &what);
    let (ok, what) = no_mix_write_check(&p.log[from..]);
    p.check(ok, &what);
    p.idle(2.0)?;
    p.screenshot("separated")?;
    let errors: Vec<String> = p
        .errors()
        .into_iter()
        .filter(|l| !l.contains("favicon") && !l.contains("404"))
        .collect();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish()
}

/// Minutes of the songs that `memory-budget` separates.
const BUDGET_MINUTES: [u32; 2] = [1, 4];

/// The peak of each part of the tab's memory in one separation, in MB,
/// from the `memory: ...` lines that the worker and the page log.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MemoryPeak {
    page_wasm: f64,
    worker_wasm: f64,
    decoder_wasm: f64,
    gpu_live: f64,
}

impl MemoryPeak {
    /// The sum of the parts' peaks: at most the peak of their sum.
    fn total(&self) -> f64 {
        self.page_wasm + self.worker_wasm + self.decoder_wasm + self.gpu_live
    }
}

/// The largest value of each part in the `memory: ...` lines of `log`, or
/// `None` if a part never appears.
fn memory_peak(log: &[String]) -> Option<MemoryPeak> {
    let largest = |key: &str| {
        log.iter()
            .filter(|l| l.contains("memory: "))
            .filter_map(|l| number_after(l, key))
            .reduce(f64::max)
    };
    Some(MemoryPeak {
        page_wasm: largest("page wasm ")?,
        worker_wasm: largest("worker wasm ")?,
        decoder_wasm: largest("decoder wasm ")?,
        gpu_live: largest("GPU buffers live ")?,
    })
}

/// A song of `minutes` at 44.1 kHz: the synthetic mix, repeated, as a
/// 16-bit WAV in the folder it returns.
fn long_song(out: &Path, minutes: u32) -> Result<PathBuf> {
    let song = crate::synth::generate();
    let frames = minutes as usize * 60 * crate::synth::SAMPLE_RATE as usize;
    let repeat = |x: &[f32]| x.iter().copied().cycle().take(frames).collect::<Vec<f32>>();
    let dir = out.join(format!("song-{minutes}"));
    std::fs::create_dir_all(&dir)?;
    let bytes = engine::wav::encode_pcm16(
        &repeat(&song.mix.left),
        &repeat(&song.mix.right),
        crate::synth::SAMPLE_RATE,
    );
    std::fs::write(dir.join("song.wav"), bytes)?;
    Ok(dir)
}

/// The tolerance of M10 for memory that must not grow with the song, in
/// MB.  The checks that use it:
/// * the page's and the worker's growth during a separation (D9 of M3,
///   step 4 of M4: [`growth_check`]);
/// * the page's and the decode worker's growth during an open (D12 of M5:
///   [`open_growth_check`]);
/// * the page's growth to show the stems, the 4-minute song against the
///   1-minute song (M7: [`shown_growth_check`]);
/// * the page's memory after the restore of the 5-minute session against
///   the 1-minute session (M7: `old_session::old_session_checks`).
pub(super) const MAX_GROWTH_MB: f64 = 4.0;

/// Whether `line` ends the wait of `memory-budget` for a separation: its
/// stems are shown, or the page reports an error.
fn separation_shown(line: &str) -> bool {
    line.contains("with the stems shown") || is_page_error(line)
}

/// The check of D9 of M3 on the growth of `part` (the page or the
/// worker) during a separation.
fn growth_check(part: &str, growth: Option<f64>) -> (bool, String) {
    (
        growth.is_some_and(|g| g <= MAX_GROWTH_MB),
        format!(
            "the {part} grew by {growth:?} MB during the separation, at most {MAX_GROWTH_MB} MB"
        ),
    )
}

/// How much the worker's WebAssembly memory grew from the start of a
/// separation to its end, from the worker's log lines.
fn worker_growth(log: &[String]) -> Option<f64> {
    let at = |when: &str| {
        log.iter()
            .find(|l| l.contains(&format!("at the separation's {when}")))
            .and_then(|l| number_after(l, "worker wasm "))
    };
    Some(at("end")? - at("start")?)
}

/// How much the page's WebAssembly memory grew from the start of an open
/// to the end of its decode (D12 of M5), from the page's log lines.
fn open_page_growth(log: &[String]) -> Option<f64> {
    let at = |when: &str| {
        log.iter()
            .find(|l| l.contains(when))
            .and_then(|l| number_after(l, "page wasm "))
    };
    Some(at("as the decode ends")? - at("as the open starts")?)
}

/// How much the decode worker's WebAssembly memory grew from the start of
/// a decode to its end (D12 of M5).
fn decoder_growth(log: &[String]) -> Option<f64> {
    let at = |when: &str| {
        log.iter()
            .find(|l| l.contains(&format!("at the {when} of the decode request")))
            .and_then(|l| number_after(l, "decoder wasm "))
    };
    Some(at("end")? - at("start")?)
}

/// The check of M5 on the growth of `part` (the page or the decode worker)
/// during an open.
fn open_growth_check(part: &str, growth: Option<f64>) -> (bool, String) {
    (
        growth.is_some_and(|g| g <= MAX_GROWTH_MB),
        format!(
            "the {part} grew by {growth:?} MB from the open's start to the decode's end, at most {MAX_GROWTH_MB} MB"
        ),
    )
}

/// The check of M5 that the decode worker decoded the song of an open, and
/// the page did not (D9, finding 3-12): no line of `log` says that the page
/// decodes on its main thread.
fn open_on_the_worker_check(log: &[String]) -> (bool, String) {
    (
        !log.iter()
            .any(|l| l.contains("decoder: decoding on the page")),
        "the decode worker decoded the song: no \"decoder: decoding on the page\" line".into(),
    )
}

/// The page's line before it writes a mix: a song of one source has its
/// mix from the decode (D1 of M5), so Separate writes no second mix.
const MIX_WRITE_LINE: &str = "before the mix is written";

/// The check that Separate wrote no mix: no [`MIX_WRITE_LINE`] in `log`.
fn no_mix_write_check(log: &[String]) -> (bool, String) {
    let found: Vec<&String> = log.iter().filter(|l| l.contains(MIX_WRITE_LINE)).collect();
    (
        found.is_empty(),
        format!("Separate writes no second mix (lines {MIX_WRITE_LINE:?}: {found:?})"),
    )
}

/// The page's lines that show a fallback to the memory path.
const FALLBACK_LINES: [&str; 2] = [
    "separator: separating on the memory path",
    "separator: browser storage failed",
];

/// The check that a separation stayed on the storage path: no line of
/// [`FALLBACK_LINES`] in `log`.
fn no_fallback_check(log: &[String]) -> (bool, String) {
    let found: Vec<&String> = log
        .iter()
        .filter(|l| FALLBACK_LINES.iter().any(|f| l.contains(f)))
        .collect();
    (
        found.is_empty(),
        format!("the separation stays on the storage path (fallback lines: {found:?})"),
    )
}

/// How much the page's WebAssembly memory grew from the start of a
/// separation to its end, from the page's log lines.  The start is the
/// line before the page writes the mix (step 3 of M4), if it wrote one.
fn page_growth(log: &[String]) -> Option<f64> {
    let at = |when: &str| {
        log.iter()
            .find(|l| l.contains(when))
            .and_then(|l| number_after(l, "page wasm "))
    };
    let start = at("before the mix is written").or_else(|| at("as the separation starts"))?;
    Some(at("as the separation ends")? - start)
}

/// How much the page's WebAssembly memory grew from the end of a
/// separation to the stems shown, from the page's log lines (step 9 of
/// the M7 plan).  The stems show as stored lanes, so the growth is the
/// read-ahead buffer and the overviews, not the song.
fn shown_growth(log: &[String]) -> Option<f64> {
    let at = |when: &str| {
        log.iter()
            .find(|l| l.contains(when))
            .and_then(|l| number_after(l, "page wasm "))
    };
    Some(at("with the stems shown")? - at("as the separation ends")?)
}

/// The check of M7 on the growths of [`shown_growth`]: the long song's is
/// at most [`MAX_GROWTH_MB`] above the short song's.  Each holds about 18
/// MiB of a larger read-ahead buffer, seven lanes after one, whatever the
/// song's length.
fn shown_growth_check(short: Option<f64>, long: Option<f64>) -> (bool, String) {
    (
        short.zip(long).is_some_and(|(s, l)| l <= s + MAX_GROWTH_MB),
        format!(
            "the page grows by {long:?} MB to show the stems of the long song, and by {short:?} MB for the short one: at most {MAX_GROWTH_MB} MB more"
        ),
    )
}

/// Separates songs of 1 and 4 minutes, each in a new browser, so that each
/// starts with empty WebAssembly memory.  Prints the peak of page wasm,
/// worker wasm and GPU live bytes for each, and their difference.  It also
/// prints how much the page and the worker grew during each separation.
/// It fails:
/// * if the page or the worker grows by more than [`MAX_GROWTH_MB`] during
///   a separation ([`growth_check`]);
/// * if the page or the decode worker grows by more than that during the
///   open ([`open_growth_check`]);
/// * if the page decodes the song on the main thread
///   ([`open_on_the_worker_check`]);
/// * if Separate writes a second mix ([`no_mix_write_check`]);
/// * if the 4-minute song's growth to show the stems is more than that
///   above the 1-minute song's ([`shown_growth_check`]);
/// * if a separation falls back to the memory path ([`no_fallback_check`]).
///
/// It asserts no budget for the peak yet; M10 of the plan does.
fn memory_budget(o: &Opts) -> Result<()> {
    let mut mounts = Vec::new();
    for minutes in BUDGET_MINUTES {
        mounts.push((format!("/test/{minutes}/"), long_song(&o.out, minutes)?));
    }
    let model = weights_mount(o, &mut mounts)?;
    let base = start_server(&o.dist, mounts, &["console.js"])?;
    let q = query(o, "chunk=16384&overlap=1&mm=s64");
    let mut peaks = Vec::new();
    let mut shown_growths = Vec::new();
    for minutes in BUDGET_MINUTES {
        let mut p = Page::open(
            o,
            base.clone(),
            &[
                "--enable-unsafe-webgpu",
                "--autoplay-policy=no-user-gesture-required",
            ],
            &format!("index.html?{q}"),
        )?;
        p.scenario = format!("memory-budget-{minutes}");
        p.wait_for(0, 60, |l| l.contains("separator: "))?;
        let from = p.mark();
        p.drop_files(&[(
            &format!("/test/{minutes}/song.wav"),
            "song.wav",
            "audio/wav",
        )])?;
        let kept = p.wait_for(from, 300, |l| l.contains("song kept in browser storage"))?;
        p.check(kept.is_some(), &format!("the {minutes}-minute song opens"));
        let (ok, what) = open_on_the_worker_check(&p.log[from..]);
        p.check(ok, &what);
        // D12 of M5: neither the page nor the decode worker grows with the
        // song while it opens.
        for (part, growth) in [
            ("page", open_page_growth(&p.log[from..])),
            ("decode worker", decoder_growth(&p.log[from..])),
        ] {
            let (ok, what) = open_growth_check(part, growth);
            p.check(ok, &what);
            if let Some(g) = growth {
                p.note(&format!(
                    "memory-budget: the {minutes}-minute song: {part} wasm grew by {g:.0} MB during the open"
                ));
            }
        }
        let from = p.mark();
        drop_model(&mut p, &model)?;
        let loaded = p.wait_for(from, 1200, |l| {
            l.contains("separator: model loaded") || is_page_error(l)
        })?;
        p.check(
            loaded
                .as_deref()
                .is_some_and(|l| l.contains("model loaded")),
            "the model loads",
        );
        let from = p.mark();
        p.click(o.separate_x, "Separate 6 stems")?;
        let shown = p.wait_for(from, 4 * 3600, separation_shown)?;
        p.check(
            shown
                .as_deref()
                .is_some_and(|l| l.contains("with the stems shown")),
            &format!("the {minutes}-minute song separates"),
        );
        let (ok, what) = no_fallback_check(&p.log[from..]);
        p.check(ok, &what);
        let (ok, what) = no_mix_write_check(&p.log[from..]);
        p.check(ok, &what);
        for (part, growth) in [
            ("page", page_growth(&p.log[from..])),
            ("worker", worker_growth(&p.log[from..])),
        ] {
            let (ok, what) = growth_check(part, growth);
            p.check(ok, &what);
            if let Some(g) = growth {
                p.note(&format!(
                    "memory-budget: the {minutes}-minute song: {part} wasm grew by {g:.0} MB during the separation"
                ));
            }
        }
        let growth = shown_growth(&p.log[from..]);
        p.note(&format!(
            "memory-budget: the {minutes}-minute song: page wasm grew by {growth:?} MB to show the stems"
        ));
        shown_growths.push(growth);
        let peak = memory_peak(&p.log);
        p.check(peak.is_some(), "the memory lines");
        if let Some(peak) = peak {
            p.note(&format!(
                "memory-budget: the {minutes}-minute song: page wasm {:.0} MB, worker wasm {:.0} MB, decoder wasm {:.0} MB, GPU live {:.0} MB, peak {:.0} MB",
                peak.page_wasm,
                peak.worker_wasm,
                peak.decoder_wasm,
                peak.gpu_live,
                peak.total()
            ));
            peaks.push((minutes, peak));
        }
        p.finish()?;
    }
    if let [(short, a), (long, b)] = peaks[..] {
        println!(
            "memory-budget: the {long}-minute peak is {:.0} MB above the {short}-minute peak",
            b.total() - a.total()
        );
    }
    // M7: the stems show with no read-back, so showing them does not grow
    // with the song.
    let (ok, what) = shown_growth_check(
        shown_growths.first().copied().flatten(),
        shown_growths.last().copied().flatten(),
    );
    println!("memory-budget: {} {what}", if ok { "ok:" } else { "FAIL:" });
    if !ok {
        bail!("memory-budget FAILED: {what}");
    }
    Ok(())
}

/// Seconds of the synthetic mix that `stored-stems` separates.
const STORED_STEMS_SECONDS: usize = 3;

/// The clip of `stored-stems` as 32-bit floats, the left channel and then
/// the right, in the folder it returns.
fn stored_stems_clip(out: &Path) -> Result<(PathBuf, usize)> {
    let song = crate::synth::generate();
    let n = STORED_STEMS_SECONDS * crate::synth::SAMPLE_RATE as usize;
    let dir = out.join("clip");
    std::fs::create_dir_all(&dir)?;
    let bytes: Vec<u8> = song.mix.left[..n]
        .iter()
        .chain(&song.mix.right[..n])
        .flat_map(|x| x.to_le_bytes())
        .collect();
    std::fs::write(dir.join("clip.f32"), bytes)?;
    Ok((dir, n))
}

/// Little-endian 32-bit floats from base64, as `e2e.base64` sends them.
fn floats_from_base64(text: &str) -> Result<Vec<f32>> {
    let bytes = crate::webdriver::base64_decode(text)?;
    if bytes.len() % 4 != 0 {
        bail!("{} bytes are not whole floats", bytes.len());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The SHA-256 of each stem in `stems` as a 16-bit WAV (see
/// [`stem_wavs`]), in the order of `names`.
fn stem_hashes(stems: &Value, names: &[&str]) -> Result<Vec<String>> {
    Ok(stem_wavs(stems, names)?
        .iter()
        .map(|wav| sha256_hex(wav))
        .collect())
}

/// The file name of the stem `name`, as `stem_protocol::stem_file` gives
/// it (`xtask` does not take `stem-protocol`): `bass.wav`.
fn stem_protocol_file(name: &str) -> String {
    format!("{name}.wav")
}

/// The session folder that `stored-stems` plants for the storage path.
const STORED_STEMS_SESSION: &str = "sessions/1792640000000";

/// The bytes of each stem in `stems` (`{name: [left, right]}`, each
/// channel as base64 floats) as a 16-bit WAV, in the order of `names`.
fn stem_wavs(stems: &Value, names: &[&str]) -> Result<Vec<Vec<u8>>> {
    names
        .iter()
        .map(|name| {
            let channel = |c: usize| {
                let text = stems[name][c]
                    .as_str()
                    .with_context(|| format!("no stem {name}"))?;
                floats_from_base64(text)
            };
            let (left, right) = (channel(0)?, channel(1)?);
            Ok(engine::wav::encode_pcm16(
                &left,
                &right,
                crate::synth::SAMPLE_RATE,
            ))
        })
        .collect()
}

/// Checks, each with whether it passed and what it checks.
type Checks = Vec<(bool, String)>;

/// The checks of `stored-stems` on what its page script returns (`r`),
/// for a clip of `frames`, and the memory path's SHA-256s.  With `hashes`
/// (`--hashes`), the SHA-256s must equal them.
fn stored_stems_checks(
    r: &Value,
    frames: usize,
    hashes: Option<&str>,
) -> Result<(Checks, Vec<String>)> {
    let mut checks = vec![
        (
            r["replies"][0]["ok"] == json!(true),
            "the worker loads the model".to_string(),
        ),
        (
            r["replies"][1]["type"] == json!("done"),
            "the memory path finishes".into(),
        ),
        (
            r["len"] == json!(frames),
            format!("the song is {frames} frames"),
        ),
    ];
    let segments = r["segments"].as_u64().unwrap_or(0);
    checks.push((segments > 1, format!("several segments ({segments})")));
    checks.push((
        r["replies"][2]["type"] == json!("done"),
        format!(
            "the storage path finishes ({})",
            texts(&json!([r["replies"][2]])).join("")
        ),
    ));
    checks.push((
        r["storage_segments"] == json!(0),
        "the storage path sends no segment reply".into(),
    ));
    let stored = r["stored"].as_array().cloned().unwrap_or_default();
    checks.push((
        stored.len() > 1,
        format!("a stored reply after each segment ({})", stored.len()),
    ));
    let last = stored.last().cloned().unwrap_or(Value::Null);
    checks.push((
        last["frames"] == json!(frames) && last["len"] == json!(frames),
        format!("the last stored reply covers the song ({last})"),
    ));
    checks.push((
        last["stems"] == json!(roformer::STEMS),
        "the stored replies name the stems in the worker's order".into(),
    ));
    let wavs = stem_wavs(&r["memory"], &roformer::STEMS)?;
    for (name, want) in roformer::STEMS.iter().zip(&wavs) {
        let got = r["files"][name]
            .as_str()
            .map(crate::webdriver::base64_decode)
            .transpose()?;
        let first_wrong = got
            .as_deref()
            .and_then(|g| g.iter().zip(want).position(|(a, b)| a != b));
        checks.push((
            got.as_deref() == Some(want.as_slice()),
            format!(
                "the stored {name} equals the memory path's as a 16-bit WAV ({} of {} bytes, first difference at {first_wrong:?})",
                got.as_ref().map_or(0, Vec::len),
                want.len()
            ),
        ));
    }
    let got = stem_hashes(&r["memory"], &roformer::STEMS)?;
    if let Some(want) = hashes {
        let want: Vec<&str> = want.split(',').map(str::trim).collect();
        checks.push((
            want == got,
            "the memory path's stems equal the hashes given".into(),
        ));
    }
    Ok((checks, got))
}

/// One worker, from a blank page, separates 3 seconds of the synthetic mix with
/// `chunk: 16384, overlap: 2`, so that it sends several segments: first on the
/// memory path, from two arrays, then on the storage path into a planted
/// session folder, from the clip planted there as its mix file (`mix.f32`,
/// interleaved).  The kernels are fixed (`mm: s64, attn: split`), so the two
/// runs do the same arithmetic.  Each stem file must equal
/// `engine::wav::encode_pcm16` of the memory path's stem, byte for byte.  The
/// storage path must send no `segment` reply, and its last `stored` reply must
/// cover the song.  It prints the SHA-256 of each memory-path stem as a 16-bit
/// WAV, and checks them against `--hashes`.
fn stored_stems(o: &Opts) -> Result<()> {
    let (clip_dir, frames) = stored_stems_clip(&o.out)?;
    let mut mounts = vec![("/clip/".to_string(), clip_dir)];
    let model = weights_mount(o, &mut mounts)?;
    let base = start_server(&o.dist, mounts, &["console.js"])?;
    let mut p = Page::open(o, base, &["--enable-unsafe-webgpu"], "__e2e/blank.html")?;
    let r = in_page(
        &mut p,
        "const [model, session, names] = args;
        await e2e.wipe();
        const clip = new Float32Array(await (await fetch('/clip/clip.f32')).arrayBuffer());
        const n = clip.length / 2;
        const bytes = new Uint8Array(await (await fetch(model)).arrayBuffer());
        const opts = { mm: 's64', attn: 'split', chunk: 16384, overlap: 2, step_limit: 600 };
        const stems = {};
        // Each separation starts with a progress reply of 0 chunks done.
        const out = { segments: 0, storage_segments: 0, len: 0, stored: [], separations: 0 };
        const started = (d) => { if (d.done === 0) out.separations++; };
        const place = (d) => {
          if (out.separations > 1) { out.storage_segments++; return; }
          out.segments++;
          out.len = d.len;
          for (const [name, l, r] of d.stems) {
            stems[name] ??= [new Float32Array(d.len), new Float32Array(d.len)];
            stems[name][0].set(l, d.start);
            stems[name][1].set(r, d.start);
          }
        };
        const stored = (d) => out.stored.push({ frames: d.frames, len: d.len, stems: d.stems });
        // The mix file: interleaved 32-bit little-endian floats.
        const mix = new DataView(new ArrayBuffer(8 * n));
        for (let i = 0; i < n; i++) {
          mix.setFloat32(8 * i, clip[i], true);
          mix.setFloat32(8 * i + 4, clip[n + i], true);
        }
        await e2e.plant_at(`${session}/mix.f32`, new Uint8Array(mix.buffer));
        const m = await e2e.ask([
          { type: 'model', bytes, ...opts },
          { type: 'separate', left: clip.slice(0, n), right: clip.slice(n), ...opts },
          { type: 'separate', mix: `${session}/mix.f32`, frames: n, session, ...opts },
        ], { progress: started, segment: place, stored });
        out.replies = m.replies;
        out.memory = {};
        for (const [name, [l, r]] of Object.entries(stems)) out.memory[name] = [e2e.base64(l), e2e.base64(r)];
        out.files = {};
        for (const [name, file] of names) {
          const bytes = await e2e.read(`${session}/stems/${file}.part`);
          out.files[name] = bytes && e2e.base64(bytes);
        }
        return out;",
        json!([
            model,
            STORED_STEMS_SESSION,
            roformer::STEMS
                .iter()
                .map(|n| [n.to_string(), stem_protocol_file(n)])
                .collect::<Vec<_>>()
        ]),
        3600,
    )?;
    p.note(&format!("replies: {:?}", texts(&r["replies"])));
    let (checks, hashes) = stored_stems_checks(&r, frames, o.hashes.as_deref())?;
    for (name, hash) in roformer::STEMS.iter().zip(&hashes) {
        p.note(&format!("stored-stems: memory path: {name} sha256 {hash}"));
    }
    p.note(&format!("stored-stems: --hashes {}", hashes.join(",")));
    if o.hashes.is_none() {
        p.note("no --hashes: the hashes are not checked");
    }
    for (ok, what) in checks {
        p.check(ok, &what);
    }
    p.finish()
}

/// Seconds of the stem that `flac` encodes.
const FLAC_SECONDS: u32 = 300;

/// The release worker encodes a 5-minute stereo stem as a FLAC, from a blank
/// page that posts the request by hand.  The reply is a `Blob` that starts
/// with `fLaC` and is smaller than the stem's 16-bit PCM.  Logs the time.
fn flac(o: &Opts) -> Result<()> {
    let base = start_server(&o.dist, Vec::new(), &["console.js"])?;
    let mut p = Page::open(o, base, &[], "__e2e/blank.html")?;
    let r = in_page(
        &mut p,
        "const rate = 44100, n = args[0] * rate;
        const left = new Float32Array(n), right = new Float32Array(n);
        for (let i = 0; i < n; i++) {
          left[i] = 0.5 * Math.sin(2 * Math.PI * 220 * i / rate);
          right[i] = 0.5 * Math.sin(2 * Math.PI * 330 * i / rate);
        }
        const w = await e2e.start();
        const t0 = performance.now();
        const d = await new Promise((resolve) => {
          w.onmessage = (e) => {
            if (e.data.type === 'flac' || e.data.type === 'error') resolve(e.data);
          };
          w.postMessage({ type: 'flac', name: 'tone.flac', left, right, rate }, [left.buffer, right.buffer]);
        });
        const ms = Math.round(performance.now() - t0);
        w.terminate();
        if (d.type !== 'flac') return { ms, error: d.text };
        const head = new Uint8Array(await d.file.slice(0, 4).arrayBuffer());
        return { ms, name: d.name, size: d.file.size, media: d.file.type,
          head: String.fromCharCode(...head) };",
        json!([FLAC_SECONDS]),
        300,
    )?;
    p.note(&format!("reply: {r}"));
    for (ok, what) in flac_checks(&r, FLAC_SECONDS) {
        p.check(ok, &what);
    }
    p.note(&format!(
        "flac: {FLAC_SECONDS} s of stereo encoded in {} ms",
        r["ms"]
    ));
    p.finish()
}

/// The checks of the `flac` scenario on the page's summary of the reply,
/// for a stem of `seconds` of 44.1 kHz stereo: each with what it checks.
fn flac_checks(r: &Value, seconds: u32) -> [(bool, String); 5] {
    let pcm_bytes = u64::from(seconds) * 44_100 * 4;
    [
        (r["error"].is_null(), "the worker encodes the stem".into()),
        (r["name"] == "tone.flac", "the reply keeps the name".into()),
        (r["media"] == "audio/flac", "the file is audio/flac".into()),
        (r["head"] == "fLaC", "the file starts with fLaC".into()),
        (
            r["size"].as_u64().is_some_and(|s| s < pcm_bytes),
            format!("the file is smaller than its {pcm_bytes} bytes of 16-bit PCM"),
        ),
    ]
}

/// The worker loads the model (from bytes, or with
/// `--download` through its download path from this server), separates
/// PyTorch's 0.5 s clip and compares every stem with PyTorch's output
/// (`--reference`: `clip.f32` and `clip_ref.f32` from
/// `tools/roformer_export.py`), then a fresh worker finds the model in
/// browser storage.  Without `--reference` only the load and storage parts
/// run.  Writes `worker_check.json`.
fn worker_check(o: &Opts) -> Result<()> {
    let mut mounts = Vec::new();
    let model = weights_mount(o, &mut mounts)?;
    if let Some(r) = &o.reference {
        mounts.push(("/reference/".into(), r.clone()));
    }
    let base = start_server(&o.dist, mounts, &["console.js"])?;
    let mut p = Page::open(o, base, &["--enable-unsafe-webgpu"], "__e2e/blank.html")?;
    let r = in_page(
        &mut p,
        "const [model, download, reference] = args;
        await e2e.wipe();
        const r = {};
        const gpu = navigator.gpu && await navigator.gpu.requestAdapter();
        r.adapter = gpu ? `${gpu.info.vendor} ${gpu.info.architecture}` : 'none';
        const msg = download
          ? { type: 'download', mm: 's64', model_url: model }
          : { type: 'model', mm: 's64', bytes: new Uint8Array(await (await fetch(model)).arrayBuffer()) };
        let t = performance.now();
        if (!reference) {
          const m = await e2e.ask([msg]);
          r.model = m.replies[0]; r.model_ms = performance.now() - t;
        } else {
          const clip = new Float32Array(await (await fetch('/reference/clip.f32')).arrayBuffer());
          const ref = new Float32Array(await (await fetch('/reference/clip_ref.f32')).arrayBuffer());
          const n = clip.length / 2;
          const stems = {};
          const sep = { type: 'separate', left: clip.slice(0, n), right: clip.slice(n), chunk: 32 * 512, overlap: 2, step_limit: 600 };
          const place = (d) => {
            for (const [name, l, r] of d.stems) {
              stems[name] ??= [new Float32Array(d.len), new Float32Array(d.len)];
              stems[name][0].set(l, d.start);
              stems[name][1].set(r, d.start);
            }
          };
          const m = await e2e.ask([msg, sep], { segment: place });
          r.model = m.replies[0]; r.end = m.replies[1]; r.ms = performance.now() - t;
          let mixE = 0;
          for (const v of clip) mixE += v * v;
          r.stems = {};
          ['bass', 'drums', 'other', 'vocals', 'guitar', 'piano'].forEach((name, s) => {
            if (!stems[name]) { r.stems[name] = 'missing'; return; }
            let e = 0, err = 0;
            for (let c = 0; c < 2; c++) {
              const want = ref.subarray((s * 2 + c) * n, (s * 2 + c + 1) * n);
              const got = stems[name][c];
              for (let i = 0; i < n; i++) { e += want[i] * want[i]; err += (got[i] - want[i]) ** 2; }
            }
            r.stems[name] = { vs_stem_db: 10 * Math.log10(e / err), vs_mix_db: 10 * Math.log10(mixE / err), share_pct: 100 * e / mixE };
          });
        }
        t = performance.now();
        const c = await e2e.ask([{ type: 'init', mm: 's64' }]);
        r.cached = c.replies[0]; r.cached_ms = performance.now() - t;
        r.files = c.files;
        return r;",
        json!([model, o.download, o.reference.is_some()]),
        4 * 3600,
    )?;
    let text = serde_json::to_string_pretty(&r)?;
    p.note(&text);
    std::fs::write(o.out.join("worker_check.json"), &text)?;
    p.check(
        r["model"]["ok"] == json!(true),
        "the worker loads the model",
    );
    p.check(
        r["cached"]["ok"] == json!(true),
        "a fresh worker loads it from browser storage",
    );
    if o.reference.is_some() {
        p.check(r["end"]["type"] == json!("done"), "the separation finishes");
        // PyTorch's own f16 run agrees with f32 to about 40 dB; well under
        // that means the GPU path is wrong.
        let worst = r["stems"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, v)| v["vs_mix_db"].as_f64().unwrap_or(f64::NEG_INFINITY))
            .fold(f64::INFINITY, f64::min);
        p.note(&format!(
            "worst stem vs PyTorch, relative to the mix: {worst:.1} dB"
        ));
        p.check(
            worst > 30.0,
            "every stem matches PyTorch (over 30 dB against the mix)",
        );
    } else {
        p.note("no --reference: the comparison with PyTorch did not run");
    }
    p.finish()
}

#[cfg(test)]
mod tests {

    /// Base64 of `bytes`, as the page's `e2e.base64` gives it.
    fn base64(bytes: &[u8]) -> String {
        const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn floats(v: &[f32]) -> String {
        base64(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
    }

    /// Stem `k`'s two channels in the sample result.
    fn channels(k: usize) -> (Vec<f32>, Vec<f32>) {
        let left: Vec<f32> = (0..4).map(|i| 0.1 * (k + i) as f32).collect();
        let right = left.iter().map(|x| -x).collect();
        (left, right)
    }

    /// What the page script of `stored-stems` returns when all goes well,
    /// for a clip of 4 frames.
    fn stored_stems_reply() -> Value {
        let mut memory = serde_json::Map::new();
        let mut files = serde_json::Map::new();
        for (k, name) in roformer::STEMS.iter().enumerate() {
            let (l, r) = channels(k);
            memory.insert(name.to_string(), json!([floats(&l), floats(&r)]));
            files.insert(
                name.to_string(),
                json!(base64(&engine::wav::encode_pcm16(&l, &r, 44_100))),
            );
        }
        let stored =
            |frames: usize| json!({ "frames": frames, "len": 4, "stems": roformer::STEMS });
        json!({
            "replies": [
                { "type": "model", "ok": true },
                { "type": "done" },
                { "type": "done" },
            ],
            "len": 4,
            "segments": 2,
            "storage_segments": 0,
            "stored": [stored(2), stored(4)],
            "memory": memory,
            "files": files,
        })
    }

    /// The names of the checks that fail.
    fn failing(r: &Value, hashes: Option<&str>) -> Vec<String> {
        let (checks, _) = stored_stems_checks(r, 4, hashes).unwrap();
        checks
            .into_iter()
            .filter(|(ok, _)| !ok)
            .map(|(_, what)| what)
            .collect()
    }

    #[test]
    fn the_stored_stems_checks_pass_a_good_reply_and_fail_wrong_ones() {
        let good = stored_stems_reply();
        let (checks, hashes) = stored_stems_checks(&good, 4, None).unwrap();
        assert_eq!(checks.len(), 15, "nine checks, and one for each stem");
        assert_eq!(failing(&good, None), Vec::<String>::new());
        let all = hashes.join(",");
        assert_eq!(failing(&good, Some(&all)), Vec::<String>::new());
        let mut wrong_hashes = hashes.clone();
        wrong_hashes[5] = "0".repeat(64);
        assert_eq!(
            failing(&good, Some(&wrong_hashes.join(","))),
            ["the memory path's stems equal the hashes given"]
        );
        let cases: [(&str, Value, &str); 10] = [
            ("/replies/0/ok", json!(false), "the worker loads the model"),
            (
                "/replies/1/type",
                json!("error"),
                "the memory path finishes",
            ),
            ("/len", json!(5), "the song is 4 frames"),
            ("/segments", json!(1), "several segments (1)"),
            (
                "/replies/2/type",
                json!("unstored"),
                "the storage path finishes",
            ),
            (
                "/storage_segments",
                json!(1),
                "the storage path sends no segment reply",
            ),
            (
                "/stored",
                json!([{ "frames": 4, "len": 4, "stems": roformer::STEMS }]),
                "a stored reply after each segment (1)",
            ),
            (
                "/stored/1/frames",
                json!(3),
                "the last stored reply covers the song",
            ),
            (
                "/stored/1/len",
                json!(5),
                "the last stored reply covers the song",
            ),
            (
                "/stored/1/stems",
                json!(["drums"]),
                "the stored replies name the stems",
            ),
        ];
        for (at, value, check) in cases {
            let mut r = good.clone();
            *r.pointer_mut(at).unwrap() = value;
            let failed = failing(&r, None);
            assert!(
                failed.len() == 1 && failed[0].starts_with(check),
                "{at}: {failed:?}"
            );
        }
        let mut r = good.clone();
        let wav = engine::wav::encode_pcm16(&[0.5; 4], &[0.5; 4], 44_100);
        r["files"]["piano"] = json!(base64(&wav));
        let failed = failing(&r, None);
        // The header is the same; the first sample of the right channel is
        // 0.5, not -0.5, so its high byte (offset 47) differs first.
        assert!(
            failed.len() == 1
                && failed[0].starts_with("the stored piano equals")
                && failed[0].ends_with("(60 of 60 bytes, first difference at Some(47))"),
            "{failed:?}"
        );
    }

    #[test]
    fn stem_wavs_and_hashes_are_of_16_bit_wavs() {
        let stems = json!({ "bass": [floats(&[0.5]), floats(&[-0.25])] });
        let wavs = stem_wavs(&stems, &["bass"]).unwrap();
        assert_eq!(wavs, [engine::wav::encode_pcm16(&[0.5], &[-0.25], 44_100)]);
        assert_eq!(wavs[0].len(), 48);
        // The WAV of one frame (16384, -8192), from Python's hashlib.
        assert_eq!(
            stem_hashes(&stems, &["bass"]).unwrap(),
            ["eee3c63df7f6248ef5893dfca72b1f3a92f1669294c50384ba94567001a9cd12"]
        );
        assert!(stem_wavs(&stems, &["drums"]).is_err(), "no such stem");
    }

    #[test]
    fn a_stem_file_name_is_the_name_and_wav() {
        assert_eq!(stem_protocol_file("bass"), "bass.wav");
        // `stem_protocol::stem_file` keeps letters as they are, so it
        // gives the same names for the worker's stems.
        assert!(
            roformer::STEMS
                .iter()
                .all(|n| n.chars().all(|c| c.is_ascii_lowercase())),
            "{:?}",
            roformer::STEMS
        );
    }

    #[test]
    fn the_stored_stems_clip_is_3_seconds_of_the_mix() {
        let out = std::env::temp_dir().join(format!("stemyla-clip-{}", std::process::id()));
        let (dir, n) = stored_stems_clip(&out).unwrap();
        let bytes = std::fs::read(dir.join("clip.f32")).unwrap();
        std::fs::remove_dir_all(&out).unwrap();
        assert_eq!(n, 3 * 44_100);
        assert_eq!(bytes.len(), 2 * n * 4, "two channels of f32");
        let song = crate::synth::generate();
        let first = f32::from_le_bytes(bytes[..4].try_into().unwrap());
        let right = f32::from_le_bytes(bytes[4 * n..4 * n + 4].try_into().unwrap());
        assert_eq!((first, right), (song.mix.left[0], song.mix.right[0]));
        let last = f32::from_le_bytes(bytes[4 * n - 4..4 * n].try_into().unwrap());
        assert_eq!(last, song.mix.left[n - 1]);
    }

    #[test]
    fn showing_the_stems_does_not_grow_with_the_song() {
        let log = |ends: f64, shown: f64| {
            vec![
                format!(
                    "[9.0s] console.info: separator: memory: page wasm {ends} MB as the separation ends"
                ),
                format!(
                    "[9.5s] console.info: separator: memory: page wasm {shown} MB with the stems shown"
                ),
            ]
        };
        let (short, long) = (
            shown_growth(&log(300.0, 320.0)),
            shown_growth(&log(302.0, 324.0)),
        );
        assert_eq!((short, long), (Some(20.0), Some(22.0)));
        assert!(shown_growth_check(short, long).0);
        // The read-back of the stems of 4 minutes: about 600 MB more.
        let read_back = shown_growth(&log(302.0, 922.0));
        assert!(!shown_growth_check(short, read_back).0);
        assert!(!shown_growth_check(short, Some(24.1)).0, "over 4 MB more");
        assert!(!shown_growth_check(None, long).0);
        assert!(!shown_growth_check(short, None).0);
        assert_eq!(shown_growth(&log(300.0, 320.0)[..1]), None, "no shown line");
    }

    #[test]
    fn memory_budget_waits_for_the_stems_shown_and_checks_the_growth() {
        assert!(separation_shown(
            "[9.5s] console.info: separator: memory: page wasm 900 MB with the stems shown"
        ));
        assert!(separation_shown("[9.5s] console.error: something broke"));
        assert!(!separation_shown(
            "[9.0s] console.info: separator: memory: page wasm 302 MB as the separation ends"
        ));
        assert!(growth_check("page", Some(0.0)).0);
        assert!(growth_check("page", Some(4.0)).0, "at the limit");
        assert!(!growth_check("page", Some(4.5)).0);
        assert!(!growth_check("worker", None).0, "no lines");
        assert_eq!(
            growth_check("worker", Some(2.0)).1,
            "the worker grew by Some(2.0) MB during the separation, at most 4 MB"
        );
    }

    /// The page's and the worker's lines of one separation on the storage
    /// path, in their real order: the worker's end line comes before the
    /// page's.
    fn separation_log() -> Vec<String> {
        [
            "[1.0s] console.info: separator: memory: page wasm 290 MB before the mix is written",
            "[2.0s] console.info: separator: separating on the storage path",
            "[2.0s] console.info: separator: memory: page wasm 300 MB as the separation starts",
            "[2.1s] console.info: separator: memory: worker wasm 57 MB at the separation's start",
            "[3.0s] console.info: separator: chunk 1/6: 0.3 s; memory: worker wasm 58 MB, model on the GPU 699 MB",
            "[8.9s] console.info: separator: memory: worker wasm 60 MB at the separation's end",
            "[9.0s] console.info: separator: memory: page wasm 302 MB as the separation ends",
            "[9.5s] console.info: separator: memory: page wasm 900 MB with the stems shown",
        ]
        .map(String::from)
        .to_vec()
    }

    #[test]
    fn the_worker_growth_is_its_end_less_its_start() {
        let log = separation_log();
        assert_eq!(worker_growth(&log), Some(3.0));
        assert_eq!(page_growth(&log), Some(12.0), "from before the mix write");
        assert_eq!(worker_growth(&log[4..]), None, "no start");
        assert_eq!(worker_growth(&log[..5]), None, "no end");
        let pages: Vec<String> = log
            .iter()
            .filter(|l| l.contains("page wasm"))
            .cloned()
            .collect();
        assert_eq!(
            worker_growth(&pages),
            None,
            "the page's lines are not the worker's"
        );
        let workers: Vec<String> = log
            .iter()
            .filter(|l| l.contains("worker wasm"))
            .cloned()
            .collect();
        assert_eq!(
            page_growth(&workers),
            None,
            "the worker's lines are not the page's"
        );
    }

    #[test]
    fn a_fallback_to_the_memory_path_fails_the_checks() {
        let log = separation_log();
        let (ok, what) = no_fallback_check(&log);
        assert!(ok, "{what}");
        for line in [
            "[2.0s] console.info: separator: separating on the memory path",
            "[5.0s] console.warn: separator: browser storage failed: NotReadableError: the mix file could not be read",
        ] {
            let mut with = log.clone();
            with.insert(5, line.to_string());
            let (ok, what) = no_fallback_check(&with);
            assert!(!ok, "{line}");
            assert!(what.contains(line), "{what}");
        }
    }

    /// The page's and the decode worker's lines of one open of a song
    /// (D12 of M5), in their real order.
    fn open_log() -> Vec<String> {
        [
            "[1.0s] console.info: [app] decoder: memory: page wasm 8 MB as the open starts",
            "[1.1s] console.info: [app::decoder::client] decoder: memory: decoder wasm 6 MB at the start of the decode request",
            "[2.0s] console.info: [app::decoder::client] decoder: memory: decoder wasm 7 MB at the end of the decode request",
            "[2.1s] console.info: [app] decoder: memory: page wasm 9 MB as the decode ends",
            "[2.2s] console.info: [app] song kept in browser storage as session 1",
        ]
        .map(String::from)
        .to_vec()
    }

    #[test]
    fn the_open_growths_are_the_end_less_the_start() {
        let log = open_log();
        assert_eq!(open_page_growth(&log), Some(1.0));
        assert_eq!(decoder_growth(&log), Some(1.0));
        assert_eq!(open_page_growth(&log[1..]), None, "no start");
        assert_eq!(open_page_growth(&log[..3]), None, "no end");
        assert_eq!(decoder_growth(&log[2..]), None, "no start");
        assert_eq!(decoder_growth(&log[..2]), None, "no end");
        let separation = separation_log();
        assert_eq!(
            open_page_growth(&separation),
            None,
            "a separation is no open"
        );
        assert_eq!(
            decoder_growth(&separation),
            None,
            "the stem worker is not the decode worker"
        );
        assert!(open_on_the_worker_check(&log).0, "the worker decoded it");
        let mut on_the_page = log.clone();
        on_the_page.insert(
            3,
            "[2.05s] console.info: [app] decoder: decoding on the page".into(),
        );
        assert!(
            !open_on_the_worker_check(&on_the_page).0,
            "the page decoded it"
        );
        assert!(open_growth_check("page", Some(4.0)).0, "at the limit");
        assert!(!open_growth_check("page", Some(4.5)).0);
        assert!(!open_growth_check("decode worker", None).0, "no lines");
        assert_eq!(
            open_growth_check("decode worker", Some(1.0)).1,
            "the decode worker grew by Some(1.0) MB from the open's start to the decode's end, at most 4 MB"
        );
    }

    #[test]
    fn a_mix_write_fails_the_check() {
        let (ok, what) = no_mix_write_check(&open_log());
        assert!(ok, "{what}");
        let (ok, what) = no_mix_write_check(&separation_log());
        assert!(!ok && what.contains("before the mix is written"), "{what}");
    }

    #[test]
    fn the_page_growth_is_the_end_less_the_start() {
        let log: Vec<String> = [
            "[1.0s] console.info: separator: memory: page wasm 300 MB as the separation starts",
            "[9.0s] console.info: separator: memory: page wasm 302 MB as the separation ends",
            "[9.5s] console.info: separator: memory: page wasm 900 MB with the stems shown",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(page_growth(&log), Some(2.0), "no mix write: from the start");
        assert_eq!(page_growth(&log[1..]), None, "no start");
        assert_eq!(page_growth(&log[..1]), None, "no end");
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn floats_come_back_from_base64_as_sent() {
        // 1.0 and -0.5 as little-endian f32: 00 00 80 3f, 00 00 00 bf.
        assert_eq!(floats_from_base64("AACAPwAAAL8=").unwrap(), [1.0, -0.5]);
        assert!(floats_from_base64("AACA").is_err(), "3 bytes");
    }

    #[test]
    fn the_flac_clip_decodes_to_its_samples() {
        let left: Vec<f32> = (0..9_000).map(|i| 0.5 * (0.01 * i as f32).sin()).collect();
        let right: Vec<f32> = left.iter().map(|x| -x).collect();
        let bytes = flac_16(&left, &right, 96_000).unwrap();
        let audio = engine::decode_bytes(bytes, Some("flac")).unwrap();
        assert_eq!(audio.sample_rate, 96_000);
        for (got, want) in audio.channels.iter().zip([&left, &right]) {
            assert_eq!(got.len(), want.len());
            for (g, w) in got.iter().zip(want.iter()) {
                assert!((g - w).abs() <= 0.5 / 32_768.0 + 1e-7, "{g} for {w}");
            }
        }
    }

    /// The reply of a real run (`target/e2e/flac/flac.log`, 2026-10-25).
    fn real_reply() -> serde_json::Value {
        json!({"head": "fLaC", "media": "audio/flac", "ms": 11293, "name": "tone.flac", "size": 8460848})
    }

    fn passes(r: &serde_json::Value) -> Vec<bool> {
        flac_checks(r, 300).map(|(ok, _)| ok).to_vec()
    }

    #[test]
    fn the_flac_checks_pass_a_real_reply_and_fail_wrong_ones() {
        assert_eq!(passes(&real_reply()), [true; 5]);
        let wrong = |field: &str, value: serde_json::Value, check: usize| {
            let mut r = real_reply();
            r[field] = value;
            let mut want = [true; 5];
            want[check] = false;
            assert_eq!(passes(&r), want, "{field}");
        };
        wrong("error", json!("FLAC encoder: no"), 0);
        wrong("name", json!("stem.flac"), 1);
        wrong("media", json!(""), 2);
        wrong("head", json!("RIFF"), 3);
        // 300 s of 44.1 kHz 16-bit stereo is 52,920,000 bytes.
        wrong("size", json!(52_920_000), 4);
        wrong("size", json!(60_000_000), 4);
        let mut r = real_reply();
        r["size"] = json!(52_919_999);
        assert_eq!(passes(&r), [true; 5]);
        assert!(
            flac_checks(&real_reply(), 300)[4]
                .1
                .contains("52920000 bytes")
        );
    }

    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn parses_scenario_and_options() {
        let o = Opts::parse(&args(
            "smoke --browser chrome --play-x 512.5 --out /tmp/x --download",
        ))
        .unwrap();
        assert_eq!(o.scenario, "smoke");
        assert_eq!(o.browser, Browser::Chrome);
        assert_eq!(o.play_x, 512.5);
        assert_eq!(o.out, PathBuf::from("/tmp/x"));
        assert!(o.download);
        let d = Opts::parse(&args("bench")).unwrap();
        assert_eq!(d.out, crate::workspace_root().join("target/e2e/bench"));
        assert_eq!(d.dist, crate::workspace_root().join("app/dist"));
        assert!(Opts::parse(&args("--browser chrome")).is_err());
        assert!(Opts::parse(&args("smoke --play-x")).is_err());
        assert!(Opts::parse(&args("smoke --frobnicate")).is_err());
    }

    #[test]
    fn page_errors_include_the_apps_log_errors() {
        let log: Vec<String> = [
            "console.info: fine",
            "console.warn: [eframe] SetTheme not implemented",
            "console.warn: ERROR: [app] failed to start eframe",
            "console.error: boom",
            "PAGE ERROR: x is not defined",
        ]
        .map(str::to_string)
        .to_vec();
        assert_eq!(page_errors(&log), log[2..]);
    }

    /// The peak of each part, from the worker's lines after the model
    /// loads and after each chunk, and the page's line at the end.
    #[test]
    fn the_memory_peak_takes_each_part_at_its_largest() {
        let log: Vec<String> = [
            "[1.0s] separator: memory: worker wasm 57 MB, model on the GPU 699 MB, GPU buffers live 699 MB",
            "[2.0s] separator: chunk 1/48: 9.9 s; memory: worker wasm 231 MB, model on the GPU 699 MB, chunk buffers 280 MB, GPU buffers live 979 MB",
            "[3.0s] separator: chunk 2/48: 9.8 s; memory: worker wasm 246 MB, model on the GPU 699 MB, chunk buffers 280 MB, GPU buffers live 979 MB",
            "[4.0s] song: opened at 44100 Hz (88200 frames)",
            "[5.0s] separator: memory: page wasm 325 MB with the stems shown",
            "[0.5s] decoder: memory: decoder wasm 6 MB at the start of the decode request",
            "[0.9s] decoder: memory: decoder wasm 9 MB at the end of the decode request",
        ]
        .map(String::from)
        .to_vec();
        let peak = memory_peak(&log).expect("every part");
        assert_eq!(
            peak,
            MemoryPeak {
                page_wasm: 325.0,
                worker_wasm: 246.0,
                decoder_wasm: 9.0,
                gpu_live: 979.0
            }
        );
        assert_eq!(peak.total(), 1559.0);
        assert_eq!(memory_peak(&log[..5]), None, "no decode worker line");
        assert_eq!(memory_peak(&log[..3]), None, "no page line yet");
    }

    #[test]
    fn reads_numbers_from_log_lines() {
        let l = "[3.0s] console.info: [app] app/src/lib.rs:789: audio stats: 12 callbacks of 512 frames (11.61 ms budget), engine mean 0.100 ms (0.9%), max so far 0.3 ms (2.6%), tempo 0.75, playhead 88200";
        assert_eq!(number_after(l, "tempo "), Some(0.75));
        assert_eq!(number_after(l, "playhead "), Some(88_200.0));
        assert_eq!(number_after(l, "nothing "), None);
        let log = vec!["audio stats: no callbacks yet".to_string(), l.to_string()];
        assert_eq!(audio_stats(&log, 0), [(0.75, 88_200.0)]);
        assert!(audio_stats(&log, 2).is_empty());
    }

    #[test]
    fn the_drip_server_sends_slowly_with_cors() {
        let base = drip_server().unwrap();
        let port: u16 = base.rsplit(':').next().unwrap().parse().unwrap();
        let t0 = Instant::now();
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET /slow HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut s, &mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).to_string();
        assert!(text.contains("Access-Control-Allow-Origin: *"), "{text}");
        assert!(text.contains("Content-Length: 1000"), "{text}");
        let head = text.find("\r\n\r\n").unwrap() + 4;
        assert_eq!(raw.len() - head, 1000);
        assert!(t0.elapsed() >= Duration::from_millis(2_500));
    }
}
