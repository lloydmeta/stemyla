//! `xtask e2e <scenario> [options]`: drives the built web app (`app/dist`)
//! in Safari or Chrome through WebDriver and checks what it does.
//!
//! Scenarios (the first three need no model and run in CI):
//! * `smoke`: drop generated stems, Play, change the tempo.
//! * `audio-resume`: a song restored at page load plays after one click,
//!   with Web Audio behaving as in Safari.
//! * `storage`: the worker's model storage across messages and workers:
//!   queue order, the cross-tab lock, stalled downloads, `?model_url=`;
//!   with `--weights`, also the paths that load a real model (GPU).
//! * `bench`: `?bench`, every matmul kernel on every model shape (WebGPU).
//! * `selftest`: `?selftest`, the GPU against the CPU reference files
//!   (`--weights`, and `xtask roformer-selftest` output).
//! * `separate`: drop a clip and the model, Separate, lanes appear
//!   (`--weights`).
//! * `worker-check`: the worker's separation against PyTorch's
//!   (`--weights`, `--reference` from `tools/roformer_export.py`).
//!
//! Options: `--browser safari|chrome`, `--dist DIR` (default `app/dist`),
//! `--out DIR` (logs and screenshots; default `target/e2e/<scenario>`),
//! `--headed` (Chrome with a window), `--swiftshader` (Chrome's software
//! WebGPU, for machines without a GPU), `--query Q` (page URL query for
//! the GPU scenarios), `--weights FILE`, `--selftest-dir DIR`,
//! `--reference DIR`, `--download`, and click positions `--play-x`,
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
                "usage: xtask e2e <smoke|audio-resume|storage|bench|selftest|separate|worker-check> [options]",
            )?
            .clone();
        let mut o = Self {
            out: PathBuf::from("target/e2e").join(&scenario),
            scenario,
            browser: Browser::default_here(),
            dist: PathBuf::from("app/dist"),
            headed: false,
            swiftshader: false,
            query: None,
            weights: None,
            selftest_dir: PathBuf::from("third_party/roformer/selftest"),
            reference: None,
            download: false,
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
        "audio-resume" => audio_resume(&o),
        "storage" => storage(&o),
        "bench" => bench(&o),
        "selftest" => selftest(&o),
        "separate" => separate(&o),
        "worker-check" => worker_check(&o),
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
struct Page {
    session: Session,
    _driver: Driver,
    base: String,
    t0: Instant,
    log: Vec<String>,
    out: PathBuf,
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
        args.push(format!("--window-size={},{}", VIEWPORT.0, VIEWPORT.1));
        let session = Session::new(&driver, o.browser, &args)?;
        session.set_viewport(VIEWPORT.0, VIEWPORT.1)?;
        let page = Self {
            session,
            _driver: driver,
            base,
            t0: Instant::now(),
            log: Vec::new(),
            out: o.out.clone(),
            failures: Vec::new(),
        };
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

    /// Lines that report a page error.
    fn errors(&self) -> Vec<String> {
        self.log
            .iter()
            .filter(|l| l.contains("PAGE ERROR") || l.contains("console.error:"))
            .cloned()
            .collect()
    }

    /// Writes the log; an error naming every failed check.
    fn finish(mut self, name: &str) -> Result<()> {
        let _ = self.poll();
        let path = self.out.join(format!("{name}.log"));
        let mut f = std::fs::File::create(&path)?;
        for l in &self.log {
            writeln!(f, "{l}")?;
        }
        println!("log: {}", path.display());
        if self.failures.is_empty() {
            println!("{name}: passed");
            Ok(())
        } else {
            bail!("{name} FAILED: {}", self.failures.join("; "))
        }
    }
}

/// The number after `key` in `line` ("playhead 1234" -> 1234).
fn number_after(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
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
    p.idle(5.0)?;
    let contexts = p.session.execute(
        "return (window.__e2e_ctxs || []).map((c) => ({ state: c.state, time: c.currentTime }));",
        json!([]),
    )?;
    p.note(&format!("audio contexts: {contexts}"));
    let running = contexts.as_array().is_some_and(|cs| {
        cs.iter()
            .any(|c| c["state"] == "running" && c["time"].as_f64() > Some(0.5))
    });
    p.check(running, "Play starts a running AudioContext");
    let heads = audio_stats(&p.log, from);
    p.check(
        heads.last().is_some_and(|(_, head)| *head >= 44_100.0),
        "the playhead moves after Play",
    );

    let from = p.mark();
    p.click(o.tempo_x, "the tempo slider at 75%")?;
    p.wait_for(from, 10, |l| {
        l.contains("audio stats: ") && l.contains(" callbacks of ")
    })?;
    let tempo = audio_stats(&p.log, from).last().map(|(t, _)| *t);
    p.note(&format!("tempo after the click: {tempo:?}"));
    p.check(
        tempo.is_some_and(|t| (0.70..=0.80).contains(&t)),
        "the tempo slider sets about 75%",
    );
    p.idle(3.0)?;
    p.screenshot("smoke_playing")?;
    let errors = p.errors();
    p.check(
        errors.is_empty(),
        &format!("no page errors ({})", errors.len()),
    );
    p.finish("smoke")
}

/// A song stored, then restored when the page
/// loads, which creates its audio output before any user gesture; one
/// click on Play must then start playback.  Chrome is made to behave like
/// Safari (contexts start suspended; `resume()` only during a click);
/// Safari is tested as it is.
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
    p.idle(5.0)?;
    let head = audio_stats(&p.log, from).last().map_or(0.0, |(_, h)| *h);
    p.note(&format!("after the click: playhead {head}"));
    p.check(head >= 44_100.0, "one click on Play starts playback");
    p.screenshot("audio_resume")?;
    p.finish("audio-resume")
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
    p.finish("storage")
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

    p.note("case: a picked file is committed");
    let r = in_page(
        p,
        "await e2e.wipe();
        const bytes = new Uint8Array(await (await fetch(args[0])).arrayBuffer());
        return e2e.ask([{ type: 'model', bytes, ...args[1] }]);",
        json!([model, opts]),
        1200,
    )?;
    p.check(
        r["replies"][0]["ok"] == json!(true) && r["replies"][0]["stored"] == json!(true),
        "the picked file loads and is stored",
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
    let verdict = p.wait_for(0, 4 * 3600, |l| {
        l.contains("bench finished")
            || l.contains("WebGPU error")
            || (l.contains("separator: ") && (l.contains("failed") || l.contains("stopped")))
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
    p.finish("bench")
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
    let verdict = p.wait_for(0, 4 * 3600, |l| {
        l.contains("selftest passed")
            || l.contains("selftest FAILED")
            || l.contains("Separation failed")
            || l.contains("separator: init:")
            || l.contains("separator: model:")
    })?;
    p.note(&format!("verdict: {verdict:?}"));
    p.check(
        verdict
            .as_deref()
            .is_some_and(|v| v.contains("selftest passed")),
        "the GPU self-test passes",
    );
    p.finish("selftest")
}

/// A two-second clip of the synthetic mix as a WAV, for `separate`.
fn clip(out: &Path) -> Result<PathBuf> {
    let song = crate::synth::generate();
    let n = 2 * 44_100;
    let dir = out.join("clip");
    std::fs::create_dir_all(&dir)?;
    let wav = engine::wav::encode_pcm16(&song.mix.left[..n], &song.mix.right[..n], 44_100);
    std::fs::write(dir.join("clip.wav"), wav)?;
    Ok(dir)
}

/// Drop a clip and the model, press Separate, and the
/// six lanes appear; screenshots along the way.
fn separate(o: &Opts) -> Result<()> {
    let mut mounts = vec![("/test/".to_string(), clip(&o.out)?)];
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
    p.drop_files(&[("/test/clip.wav", "clip.wav", "audio/wav")])?;
    let kept = p.wait_for(from, 60, |l| l.contains("song kept in browser storage"))?;
    p.check(kept.is_some(), "the clip opens");
    p.screenshot("separate_song")?;
    let from = p.mark();
    drop_model(&mut p, &model)?;
    let loaded = p.wait_for(from, 1200, |l| {
        l.contains("separator: model loaded")
            || l.contains("separator: model:")
            || l.contains("could not be loaded")
            || l.contains("not a BS-RoFormer")
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
        (l.contains("separator: ") && l.contains(" stems in "))
            || l.contains("Separation failed")
            || l.contains("separator: separate:")
    })?;
    p.note(&format!("separated: {done:?}"));
    p.check(
        done.as_deref()
            .is_some_and(|l| l.contains("separator: 6 stems in")),
        "six stems arrive",
    );
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
    p.finish("separate")
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
          const m = await e2e.ask([msg, sep], { stem: (d) => { stems[d.name] = [d.left, d.right]; } });
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
    p.finish("worker-check")
}

#[cfg(test)]
mod tests {
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
        assert_eq!(d.out, PathBuf::from("target/e2e/bench"));
        assert!(Opts::parse(&args("--browser chrome")).is_err());
        assert!(Opts::parse(&args("smoke --play-x")).is_err());
        assert!(Opts::parse(&args("smoke --frobnicate")).is_err());
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
