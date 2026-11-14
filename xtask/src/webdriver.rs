//! A small blocking WebDriver client (W3C WebDriver, HTTP/1.1 + JSON to a
//! local driver), with only the commands the end-to-end scenarios need.
//! Spec: https://www.w3.org/TR/webdriver2/
//!
//! Every request opens its own connection.  A response ends after its
//! `Content-Length` or its last chunk (drivers may keep the connection
//! open whatever the request asked), or else where the stream does.
//!
//! On macOS, each Chrome driver process gets a run's Chrome folder
//! ([`ChromeFolder`]) in `/tmp/stemyla-chrome/<pid>-<n>`, through
//! `MAC_CHROMIUM_TMPDIR`.  `chromedriver` makes each session's profile in
//! it.  A drop of the [`Driver`] removes the folder after it stops the
//! driver.  A run that a signal ends leaves its folder.  So the next
//! folder's start removes each folder whose run has ended, if no process
//! names it.  `web-test` gives each crate run such a folder too.

use crate::drivers::Browser;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A port that was free a moment ago, for a child process to bind.
pub fn free_port() -> Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

/// How long a WebDriver command may take, unless it says otherwise.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// One HTTP request to `127.0.0.1:port`; the status and the JSON body.  A
/// driver that stays silent for `timeout` is an error, not a hang.
pub fn http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<(u16, Value)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .with_context(|| format!("connecting to the driver on port {port}"))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let body = body.map(Value::to_string).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 1 << 16];
    while !is_complete(&raw)? {
        let n = stream
            .read(&mut buf)
            .with_context(|| format!("{method} {path}: no answer within {timeout:?}"))?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
    }
    parse_response(&raw)
}

/// Whether `raw` holds a whole response: headers and the body they
/// announce.  A response with neither a length nor chunks ends at EOF.
fn is_complete(raw: &[u8]) -> Result<bool> {
    let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Ok(false);
    };
    let (chunked, length) = framing(std::str::from_utf8(&raw[..end])?);
    let body = &raw[end + 4..];
    Ok(if chunked {
        dechunk(body).is_ok()
    } else {
        length.is_some_and(|n| body.len() >= n)
    })
}

/// Whether the body is chunked, and its length if given.
fn framing(head: &str) -> (bool, Option<usize>) {
    let (mut chunked, mut length) = (false, None);
    for line in head.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        if name == "transfer-encoding" && value.eq_ignore_ascii_case("chunked") {
            chunked = true;
        } else if name == "content-length" {
            length = value.parse::<usize>().ok();
        }
    }
    (chunked, length)
}

/// Splits a whole HTTP/1.1 response into its status and JSON body.
fn parse_response(raw: &[u8]) -> Result<(u16, Value)> {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("response without a header end")?;
    let head = std::str::from_utf8(&raw[..end]).context("response header is not UTF-8")?;
    let status: u16 = head
        .split("\r\n")
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .context("response without a status")?;
    let (chunked, length) = framing(head);
    let rest = &raw[end + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else {
        rest[..length.unwrap_or(rest.len()).min(rest.len())].to_vec()
    };
    let value = if body.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(&body).context("response body is not JSON")?
    };
    Ok((status, value))
}

fn dechunk(mut b: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b
            .windows(2)
            .position(|w| w == b"\r\n")
            .context("truncated chunk size")?;
        let size_text = std::str::from_utf8(&b[..eol])?;
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).context("bad chunk size")?;
        b = &b[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if b.len() < size + 2 {
            bail!("truncated chunk");
        }
        out.extend_from_slice(&b[..size]);
        b = &b[size + 2..];
    }
}

/// The `value` of a successful WebDriver response, or its error.
fn value_of(status: u16, body: Value) -> Result<Value> {
    let value = body.get("value").cloned().unwrap_or(Value::Null);
    if status >= 400 {
        let error = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("error");
        let message = value.get("message").and_then(Value::as_str).unwrap_or("");
        bail!("WebDriver {status} {error}: {message}");
    }
    Ok(value)
}

/// The parent of the runs' Chrome folders.  Only `xtask` uses it.  The
/// path is short, since Chrome makes a Unix socket in its temporary
/// folder, and a socket path on macOS has at most 104 bytes.
pub const CHROME_FOLDER_PARENT: &str = "/tmp/stemyla-chrome";

/// The variable that gives Chromium on macOS its temporary folder.
/// `base::GetTempDir` reads it before `NSTemporaryDirectory()`.
pub const CHROME_TMP_VAR: &str = "MAC_CHROMIUM_TMPDIR";

/// How many times a removal of a run's Chrome folder tries, and the pause
/// between two tries.  Chrome's processes can still close files.
const REMOVE_TRIES: u32 = 10;
const REMOVE_PAUSE: Duration = Duration::from_millis(200);

/// The number of the next Chrome folder of this process.
static NEXT_CHROME_FOLDER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Whether a driver process of `browser` gets a run's Chrome folder.  It
/// is Chrome on macOS only, where the leaked profiles were seen.
pub fn wants_chrome_folder(browser: Browser, on_macos: bool) -> bool {
    browser == Browser::Chrome && on_macos
}

/// The path of the `n`th Chrome folder of the process `pid` in `parent`.
fn chrome_folder_path(parent: &Path, pid: u32, n: u32) -> PathBuf {
    parent.join(format!("{pid}-{n}"))
}

/// The folder for the temporary files of one Chrome driver process and
/// its browsers (a run's Chrome folder).  A drop removes it, with all in
/// it.  `chromedriver` makes each session's profile in it.
pub struct ChromeFolder {
    path: PathBuf,
}

impl ChromeFolder {
    /// Makes the next Chrome folder of this process in `parent`.  First it
    /// removes each folder there that an ended run left ([`sweep`]).
    pub fn create(parent: &Path) -> Result<Self> {
        let uid = current_uid().context("cannot tell the current user (/usr/bin/id -u)")?;
        make_private_folder(parent, true)?;
        let meta = parent
            .symlink_metadata()
            .with_context(|| format!("reading {}", parent.display()))?;
        if !meta.is_dir() || owner(&meta) != Some(uid) {
            bail!(
                "{} is not a folder of the current user; remove it, and run again",
                parent.display()
            );
        }
        match sweep(parent, uid, crate::drivers::command_lines) {
            Some(removed) => {
                for removed in removed {
                    println!(
                        "chrome folder: removed {}, which an ended run left",
                        removed.display()
                    );
                }
            }
            None => eprintln!(
                "chrome folder: no process list, so the folders that ended runs left in {} stay",
                parent.display()
            ),
        }
        let n = NEXT_CHROME_FOLDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = chrome_folder_path(parent, std::process::id(), n);
        if path.symlink_metadata().is_ok() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing the old {}", path.display()))?;
        }
        make_private_folder(&path, false)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ChromeFolder {
    fn drop(&mut self) {
        let removed = remove_retrying(&self.path, REMOVE_TRIES, REMOVE_PAUSE, |path| {
            std::fs::remove_dir_all(path)
        });
        if let Err(error) = removed {
            eprintln!("{}", removal_failed_line(&self.path, &error));
        }
    }
}

/// Makes `path` with access for its owner only.
fn make_private_folder(path: &Path, recursive: bool) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(path)
        .with_context(|| format!("making {}", path.display()))
}

/// The user ID of this process, from `/usr/bin/id -u`.
fn current_uid() -> Option<u32> {
    crate::stdout_on_success(Command::new("/usr/bin/id").arg("-u"))?
        .trim()
        .parse()
        .ok()
}

/// The user ID of the owner of a file, if the system has one.
#[cfg(unix)]
fn owner(meta: &std::fs::Metadata) -> Option<u32> {
    Some(std::os::unix::fs::MetadataExt::uid(meta))
}

#[cfg(not(unix))]
fn owner(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Calls `remove` on `path` up to `tries` times, with `pause` between two
/// tries, until it succeeds.  A path that is not there is removed.
fn remove_retrying(
    path: &Path,
    tries: u32,
    pause: Duration,
    remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    remove_retrying_with(path, tries, pause, remove, std::thread::sleep)
}

/// [`remove_retrying`], with `wait` for each pause: a test records them.
fn remove_retrying_with(
    path: &Path,
    tries: u32,
    pause: Duration,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
    mut wait: impl FnMut(Duration),
) -> std::io::Result<()> {
    let mut last = Ok(());
    for attempt in 1..=tries {
        last = match remove(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        };
        if last.is_ok() {
            return last;
        }
        if attempt < tries {
            wait(pause);
        }
    }
    last
}

/// The line of a removal of a run's Chrome folder that failed.
fn removal_failed_line(path: &Path, error: &std::io::Error) -> String {
    format!(
        "chrome folder: could not remove {} ({error}); remove it by hand",
        path.display()
    )
}

/// One entry of the parent of the runs' Chrome folders, as the start
/// sweep sees it.
struct SweptEntry<'a> {
    name: &'a str,
    path: &'a str,
    /// A folder, not a symbolic link.
    is_dir: bool,
    owner: Option<u32>,
}

/// Whether the start sweep removes `entry`.  It removes a folder named
/// `<pid>-<n>` of the user `uid`, when no process of `ps` has the ID `pid`,
/// and no command line of `ps` names the folder's path.  `ps` is the text
/// of [`crate::drivers::command_lines`].
fn sweep_removes(entry: &SweptEntry, uid: u32, ps: &str) -> bool {
    let is_number = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let Some((pid, n)) = entry.name.split_once('-') else {
        return false;
    };
    if !is_number(pid) || !is_number(n) || !entry.is_dir || entry.owner != Some(uid) {
        return false;
    }
    !ps.lines().any(|line| {
        let line = line.trim_start();
        let (line_pid, command) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        line_pid == pid || names_path(command, entry.path)
    })
}

/// Whether `command` names `path`: `path` is in it, and its next character
/// is none, `/`, a space or a quote.
fn names_path(command: &str, path: &str) -> bool {
    command.match_indices(path).any(|(at, _)| {
        command[at + path.len()..]
            .chars()
            .next()
            .is_none_or(|next| matches!(next, '/' | ' ' | '"' | '\''))
    })
}

/// Removes each folder in `parent` that [`sweep_removes`] chooses, and
/// gives their paths, or `None` when `ps` gives no process list.  It reads
/// `parent` first, and only then the process list, so a run that starts
/// meanwhile has no folder in the listing yet, or is in the list.  It
/// reads `parent` only, and follows no symbolic link.
fn sweep(parent: &Path, uid: u32, ps: impl FnOnce() -> Option<String>) -> Option<Vec<PathBuf>> {
    let entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(parent) {
        Ok(entries) => entries.flatten().collect(),
        Err(_) => Vec::new(),
    };
    let ps = ps()?;
    let mut removed = Vec::new();
    for entry in entries {
        let path = entry.path();
        // `DirEntry::metadata` does not follow a symbolic link.
        let (Some(name), Some(text), Ok(meta)) = (
            entry.file_name().to_str().map(str::to_string),
            path.to_str(),
            entry.metadata(),
        ) else {
            continue;
        };
        let swept = SweptEntry {
            name: &name,
            path: text,
            is_dir: meta.is_dir(),
            owner: owner(&meta),
        };
        if !sweep_removes(&swept, uid, &ps) {
            continue;
        }
        match remove_retrying(&path, REMOVE_TRIES, REMOVE_PAUSE, |path| {
            std::fs::remove_dir_all(path)
        }) {
            Ok(()) => removed.push(path),
            Err(error) => eprintln!("{}", removal_failed_line(&path, &error)),
        }
    }
    Some(removed)
}

/// The command of a driver process of `browser` at `path` on `port`.  A
/// Chrome driver with a run's Chrome folder gets [`CHROME_TMP_VAR`].
fn driver_command(
    browser: Browser,
    path: &Path,
    port: u16,
    chrome_folder: Option<&Path>,
) -> Command {
    let mut cmd = Command::new(path);
    match browser {
        Browser::Safari => cmd.args(["-p", &port.to_string()]),
        Browser::Chrome => cmd.arg(format!("--port={port}")),
    };
    if let (Browser::Chrome, Some(folder)) = (browser, chrome_folder) {
        cmd.env(CHROME_TMP_VAR, folder);
    }
    cmd
}

/// A WebDriver process on a free local port, stopped when dropped.  A drop
/// then removes its run's Chrome folder, if it has one.
pub struct Driver {
    child: Child,
    pub port: u16,
    /// Dropped after the kill and the wait of [`Drop for Driver`].
    _chrome_folder: Option<ChromeFolder>,
}

impl Driver {
    pub fn start(browser: Browser) -> Result<Self> {
        let path = browser.driver()?;
        let port = free_port()?;
        let chrome_folder = if wants_chrome_folder(browser, cfg!(target_os = "macos")) {
            Some(ChromeFolder::create(Path::new(CHROME_FOLDER_PARENT))?)
        } else {
            None
        };
        let child = driver_command(
            browser,
            &path,
            port,
            chrome_folder.as_ref().map(ChromeFolder::path),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting {}", path.display()))?;
        let driver = Self {
            child,
            port,
            _chrome_folder: chrome_folder,
        };
        let t0 = Instant::now();
        loop {
            if let Ok((200, body)) = http(port, "GET", "/status", None, Duration::from_secs(5))
                && body["value"]["ready"] != json!(false)
            {
                return Ok(driver);
            }
            if t0.elapsed() > Duration::from_secs(20) {
                bail!("{} did not answer on port {port}", path.display());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How [`start_session`] starts a session in one browser.  Q8 of the tech
/// debt rungs allows a second try of a session start only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartPlan {
    /// End the stale Safari sessions before each try (TD-96).
    pub end_stale_sessions: bool,
    /// How many times to try.
    pub tries: u32,
}

/// Safari ends the stale sessions before each try, and tries twice, since
/// a stale session can hold Safari (TD-96).  Chrome tries once.
pub fn start_plan(browser: Browser) -> StartPlan {
    match browser {
        Browser::Safari => StartPlan {
            end_stale_sessions: true,
            tries: 2,
        },
        Browser::Chrome => StartPlan {
            end_stale_sessions: false,
            tries: 1,
        },
    }
}

/// Calls `start` as `plan` tells, and gives its first success.  Before
/// each try, it calls `end_stale` if the plan tells it to.  The error
/// names the error of each try.
pub fn start_with<T>(
    plan: StartPlan,
    mut end_stale: impl FnMut(),
    mut start: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut errors = Vec::new();
    for attempt in 1..=plan.tries {
        if plan.end_stale_sessions {
            end_stale();
        }
        match start() {
            Ok(started) => return Ok(started),
            Err(e) => {
                eprintln!(
                    "try {attempt} of {}: the session did not start ({e:#})",
                    plan.tries
                );
                errors.push(format!("try {attempt}: {e:#}"));
            }
        }
    }
    bail!("the session did not start; {}", errors.join("; "))
}

/// Starts a driver and a browser session, as [`start_plan`] tells.  A
/// failed try stops its driver before the next try starts a new one.  It
/// tries nothing else again (Q8 of the tech debt rungs).
pub fn start_session(browser: Browser, args: &[String]) -> Result<(Driver, Session)> {
    start_with(
        start_plan(browser),
        || {
            crate::drivers::end_stale_safari_sessions(
                crate::drivers::processes().as_deref(),
                |mut kill| {
                    let _ = kill.status();
                },
            )
        },
        || session_on(Driver::start(browser)?, browser, args),
    )
}

/// Starts a session on `driver`.  If it fails, it drops the driver, which
/// stops it and removes its run's Chrome folder.
fn session_on(driver: Driver, browser: Browser, args: &[String]) -> Result<(Driver, Session)> {
    let session = Session::new(&driver, browser, args)?;
    Ok((driver, session))
}

/// A browser session, ended when dropped.
pub struct Session {
    port: u16,
    id: String,
    /// The browser's version, from the new session's capabilities.
    pub version: String,
}

/// The browser version of a new session's reply: its
/// `capabilities.browserVersion`, or `unknown`.
fn browser_version(reply: &Value) -> String {
    reply["capabilities"]["browserVersion"]
        .as_str()
        .unwrap_or("unknown")
        .to_string()
}

impl Session {
    /// Starts a browser; `args` are Chrome command-line flags (Safari
    /// takes none).
    pub fn new(driver: &Driver, browser: Browser, args: &[String]) -> Result<Self> {
        let always = match browser {
            Browser::Chrome => json!({
                "browserName": "chrome",
                "goog:chromeOptions": { "args": args },
            }),
            Browser::Safari => json!({ "browserName": "safari" }),
        };
        let caps = json!({ "capabilities": { "alwaysMatch": always } });
        let (status, body) = http(
            driver.port,
            "POST",
            "/session",
            Some(&caps),
            COMMAND_TIMEOUT,
        )?;
        let value =
            value_of(status, body).map_err(|e| anyhow!("{e:#}\n{}", browser.setup_hint()))?;
        let id = value["sessionId"]
            .as_str()
            .context("new session without an id")?
            .to_string();
        Ok(Self {
            port: driver.port,
            id,
            version: browser_version(&value),
        })
    }

    /// The session `id` of a fake driver on `port`, for a test.
    #[cfg(test)]
    pub(crate) fn at(port: u16, id: &str) -> Self {
        Self {
            port,
            id: id.to_string(),
            version: "lloyd".to_string(),
        }
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        self.call_within(method, path, body, COMMAND_TIMEOUT)
    }

    fn call_within(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value> {
        let path = format!("/session/{}{path}", self.id);
        let (status, reply) = http(self.port, method, &path, body.as_ref(), timeout)?;
        value_of(status, reply).with_context(|| format!("{method} {path}"))
    }

    pub fn goto(&self, url: &str) -> Result<()> {
        self.call("POST", "/url", Some(json!({ "url": url })))
            .map(drop)
    }

    pub fn refresh(&self) -> Result<()> {
        self.call("POST", "/refresh", Some(json!({}))).map(drop)
    }

    /// Runs `script` as a function body (`arguments[i]` are `args`) and
    /// returns what it returns.
    pub fn execute(&self, script: &str, args: Value) -> Result<Value> {
        self.call(
            "POST",
            "/execute/sync",
            Some(json!({ "script": script, "args": args })),
        )
    }

    /// Like [`execute`](Self::execute), but the script ends by calling its
    /// last argument with the result; it may take up to `timeout`.
    pub fn execute_async(&self, script: &str, args: Value, timeout: Duration) -> Result<Value> {
        self.call(
            "POST",
            "/timeouts",
            Some(json!({ "script": timeout.as_millis() as u64 })),
        )?;
        // The driver ends the script at `timeout`; a minute more for it
        // to say so.
        self.call_within(
            "POST",
            "/execute/async",
            Some(json!({ "script": script, "args": args })),
            timeout + Duration::from_secs(60),
        )
    }

    /// A trusted mouse click at viewport coordinates.  egui acts on a
    /// click only once it has drawn a frame with the pointer over the
    /// widget, so the pointer moves first and the page draws two frames
    /// (however long a busy page takes).  Press and release then go
    /// together: egui takes them for a long press, not a click, when its
    /// frames see them more than 0.8 s apart (`max_click_duration`).
    pub fn click(&self, x: f64, y: f64) -> Result<()> {
        let pointer = |steps: Value| {
            json!({ "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": steps,
            }]})
        };
        let (x, y) = (x.round() as i64, y.round() as i64);
        self.call(
            "POST",
            "/actions",
            Some(pointer(json!([
                { "type": "pointerMove", "duration": 0, "origin": "viewport", "x": x, "y": y },
            ]))),
        )?;
        self.frames(2)?;
        self.call(
            "POST",
            "/actions",
            Some(pointer(json!([
                { "type": "pointerDown", "button": 0 },
                { "type": "pointerUp", "button": 0 },
            ]))),
        )?;
        self.call("DELETE", "/actions", None).map(drop)
    }

    /// Waits until the page has drawn `n` more frames.
    fn frames(&self, n: u32) -> Result<()> {
        self.execute_async(
            "const [n, done] = arguments;
            const next = (left) => (left === 0 ? done(true) : requestAnimationFrame(() => next(left - 1)));
            next(n);",
            json!([n]),
            Duration::from_secs(60),
        )
        .map(drop)
    }

    /// Sizes the window so the page's viewport is `width` by `height` CSS
    /// pixels, and returns what each try saw, for the log.
    pub fn set_viewport(&self, width: u32, height: u32) -> Result<Vec<String>> {
        fit_viewport(
            (width, height),
            |(w, h)| {
                self.call(
                    "POST",
                    "/window/rect",
                    Some(json!({ "width": w, "height": h })),
                )
                .map(drop)
            },
            || Ok(self.sizes()?.0),
            |window| self.settled_viewport(window),
        )
    }

    /// The viewport once the page knows the window's new size; see `settle`.
    /// Each read waits two frames, which can block for up to 60 s.
    fn settled_viewport(&self, window: Size) -> Result<Size> {
        settle(window, Instant::now() + SETTLE_TIME, Instant::now, || {
            self.frames(2)?;
            self.sizes()
        })
    }

    /// The page's inner (viewport) and outer (window) sizes.
    fn sizes(&self) -> Result<(Size, Size)> {
        let got = self.execute(
            "return [window.innerWidth, window.innerHeight, window.outerWidth, window.outerHeight];",
            json!([]),
        )?;
        let at = |index: usize| got[index].as_u64().unwrap_or(0) as u32;
        Ok(((at(0), at(1)), (at(2), at(3))))
    }

    pub fn screenshot(&self, path: &Path) -> Result<()> {
        let b64 = self.call("GET", "/screenshot", None)?;
        let png = base64_decode(b64.as_str().context("screenshot is not text")?)?;
        std::fs::write(path, png).with_context(|| format!("writing {}", path.display()))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = http(
            self.port,
            "DELETE",
            &format!("/session/{}", self.id),
            None,
            COMMAND_TIMEOUT,
        );
    }
}

pub(crate) fn base64_decode(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' => continue,
            _ => bail!("not base64"),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// A width and a height in CSS pixels.
type Size = (u32, u32);

/// How long `settle` starts new reads for.
const SETTLE_TIME: Duration = Duration::from_secs(2);

/// Reads the page's inner and outer sizes until the outer size equals
/// `window`, which shows that the page knows the window's new size, and
/// gives that read's inner size.  It starts no new read after `deadline`,
/// and then gives the last inner size: a window that the screen clamps
/// never matches.
fn settle(
    window: Size,
    deadline: Instant,
    mut now: impl FnMut() -> Instant,
    mut read: impl FnMut() -> Result<(Size, Size)>,
) -> Result<Size> {
    loop {
        let (inner, outer) = read()?;
        if outer == window || now() >= deadline {
            return Ok(inner);
        }
    }
}

/// Resizes the window until the viewport is `target`.  The window also
/// holds the browser's own UI (143 pixels high in headless Chrome 155, on
/// 2026-10-22), so it takes a second resize.  A read at once can still give
/// the viewport from before the resize, so each try decides on `settled`,
/// a read once the page knows the window's new size (TD-30).  `at_once` is
/// only logged, as evidence of the race.  A settled read that gives the
/// target is read again at the same window, since a late read can give the
/// target from before the resize.  Five tries leave room for a late settled
/// read, which the next try corrects.
fn fit_viewport(
    target: Size,
    mut resize: impl FnMut(Size) -> Result<()>,
    mut at_once: impl FnMut() -> Result<Size>,
    mut settled: impl FnMut(Size) -> Result<Size>,
) -> Result<Vec<String>> {
    let mut window = target;
    let mut tries = Vec::new();
    for _ in 0..5 {
        resize(window)?;
        let now = at_once()?;
        let first = settled(window)?;
        let line = format!(
            "window {}x{}: viewport {}x{} at once, {}x{} settled",
            window.0, window.1, now.0, now.1, first.0, first.1
        );
        let again = if first == target {
            Some(settled(window)?)
        } else {
            None
        };
        tries.push(match again {
            Some(again) => format!("{line}, {}x{} again", again.0, again.1),
            None => line,
        });
        let after = again.unwrap_or(first);
        if after == target {
            return Ok(tries);
        }
        window = (
            (window.0 + target.0).saturating_sub(after.0),
            (window.1 + target.1).saturating_sub(after.1),
        );
    }
    bail!(
        "could not size the viewport to {}x{}: {}",
        target.0,
        target.1,
        tries.join("; ")
    )
}

#[cfg(test)]
mod viewport_tests {
    use super::*;
    use std::cell::Cell;

    /// A headless Chrome window with 143 pixels of its own UI, whose read at
    /// once still gives the viewport from before the last resize.  The
    /// settled reads with the numbers in `stale_settles` (from 1) give it too.
    struct LaggingWindow {
        window: Cell<Size>,
        asked: Cell<Size>,
        before: Cell<Size>,
        settles: Cell<usize>,
        max_height: u32,
        stale_settles: &'static [usize],
    }

    impl LaggingWindow {
        fn new(max_height: u32, stale_settles: &'static [usize]) -> Self {
            Self {
                window: Cell::new((1100, 700)),
                asked: Cell::new((1100, 700)),
                before: Cell::new((1100, 557)),
                settles: Cell::new(0),
                max_height,
                stale_settles,
            }
        }

        fn viewport(&self) -> Size {
            let (w, h) = self.window.get();
            (w, h.saturating_sub(143))
        }

        fn fit(&self) -> Result<Vec<String>> {
            fit_viewport(
                (1100, 700),
                |(w, h)| {
                    self.before.set(self.viewport());
                    self.asked.set((w, h));
                    self.window.set((w, h.min(self.max_height)));
                    Ok(())
                },
                || Ok(self.before.get()),
                |window| {
                    assert_eq!(
                        window,
                        self.asked.get(),
                        "settle waits for the window it asked for"
                    );
                    self.settles.set(self.settles.get() + 1);
                    Ok(if self.stale_settles.contains(&self.settles.get()) {
                        self.before.get()
                    } else {
                        self.viewport()
                    })
                },
            )
        }
    }

    /// A clock that moves 1 s at each look, and reads of `outers` in order,
    /// each with the inner size 1100x(its number).
    fn settle_with(window: Size, outers: &[Size]) -> (Result<Size>, usize) {
        let start = Instant::now();
        let looks = Cell::new(0);
        let reads = Cell::new(0);
        let inner = settle(
            window,
            start + SETTLE_TIME,
            || {
                looks.set(looks.get() + 1);
                start + Duration::from_secs(looks.get())
            },
            || {
                let index = reads.get();
                assert!(index < 10, "settle read 10 times and did not stop");
                reads.set(index + 1);
                let outer = outers[index.min(outers.len() - 1)];
                Ok(((1100, index as u32), outer))
            },
        );
        (inner, reads.get())
    }

    #[test]
    fn settle_gives_the_first_read_whose_outer_size_matches() {
        let (inner, reads) = settle_with((1100, 843), &[(1100, 843)]);
        assert_eq!((inner.unwrap(), reads), ((1100, 0), 1));
        let (inner, reads) = settle_with((1100, 843), &[(1100, 700), (1100, 843)]);
        assert_eq!((inner.unwrap(), reads), ((1100, 1), 2));
    }

    #[test]
    fn settle_stops_at_the_deadline_with_the_last_read() {
        let (inner, reads) = settle_with((1100, 986), &[(1100, 800)]);
        assert_eq!((inner.unwrap(), reads), ((1100, 1), 2));
    }

    #[test]
    fn the_viewport_fits_despite_a_stale_read_at_once() {
        let chrome = LaggingWindow::new(u32::MAX, &[]);
        let tries = chrome.fit().unwrap();
        assert_eq!(chrome.window.get(), (1100, 843), "{tries:?}");
        assert_eq!(
            tries,
            [
                "window 1100x700: viewport 1100x557 at once, 1100x557 settled",
                "window 1100x843: viewport 1100x557 at once, 1100x700 settled, 1100x700 again",
            ]
        );
    }

    #[test]
    fn a_late_settled_read_is_corrected_by_the_next_try() {
        let chrome = LaggingWindow::new(u32::MAX, &[2]);
        let tries = chrome.fit().unwrap();
        assert_eq!(chrome.window.get(), (1100, 843), "{tries:?}");
        assert_eq!(
            tries,
            [
                "window 1100x700: viewport 1100x557 at once, 1100x557 settled",
                "window 1100x843: viewport 1100x557 at once, 1100x557 settled",
                "window 1100x986: viewport 1100x700 at once, 1100x843 settled",
                "window 1100x843: viewport 1100x843 at once, 1100x700 settled, 1100x700 again",
            ]
        );
    }

    #[test]
    fn two_late_settled_reads_do_not_stop_at_a_wrong_size() {
        let chrome = LaggingWindow::new(u32::MAX, &[2, 3]);
        let tries = chrome.fit().unwrap();
        assert_eq!(chrome.window.get(), (1100, 843), "{tries:?}");
        assert_eq!(
            tries,
            [
                "window 1100x700: viewport 1100x557 at once, 1100x557 settled",
                "window 1100x843: viewport 1100x557 at once, 1100x557 settled",
                "window 1100x986: viewport 1100x700 at once, 1100x700 settled, 1100x843 again",
                "window 1100x843: viewport 1100x843 at once, 1100x700 settled, 1100x700 again",
            ]
        );
    }

    #[test]
    fn a_viewport_that_never_fits_names_each_try() {
        let error = LaggingWindow::new(800, &[]).fit().unwrap_err().to_string();
        assert_eq!(
            error,
            "could not size the viewport to 1100x700: \
             window 1100x700: viewport 1100x557 at once, 1100x557 settled; \
             window 1100x843: viewport 1100x557 at once, 1100x657 settled; \
             window 1100x886: viewport 1100x657 at once, 1100x657 settled; \
             window 1100x929: viewport 1100x657 at once, 1100x657 settled; \
             window 1100x972: viewport 1100x657 at once, 1100x657 settled"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;

    /// Asserts:
    /// * a reply with `capabilities.browserVersion` gives that version;
    /// * a reply without it gives `unknown`.
    #[test]
    fn the_browser_version_comes_from_the_capabilities() {
        let reply = json!({
            "sessionId": "1",
            "capabilities": { "browserName": "chrome", "browserVersion": "157.0.7000.0" },
        });
        assert_eq!(browser_version(&reply), "157.0.7000.0");
        assert_eq!(browser_version(&json!({ "sessionId": "1" })), "unknown");
    }

    #[test]
    fn safari_ends_the_stale_sessions_and_tries_twice() {
        assert_eq!(
            start_plan(Browser::Safari),
            StartPlan {
                end_stale_sessions: true,
                tries: 2
            }
        );
    }

    #[test]
    fn chrome_tries_once_and_ends_no_session() {
        assert_eq!(
            start_plan(Browser::Chrome),
            StartPlan {
                end_stale_sessions: false,
                tries: 1
            }
        );
    }

    /// The order of the calls of `start_with`: `end` for `end_stale`, and
    /// `start` for `start`, which fails while `failures` lasts.
    fn calls_of(plan: StartPlan, failures: usize) -> (Vec<&'static str>, Result<&'static str>) {
        let calls = std::cell::RefCell::new(Vec::new());
        let mut left = failures;
        let result = start_with(
            plan,
            || calls.borrow_mut().push("end"),
            || {
                calls.borrow_mut().push("start");
                if left > 0 {
                    left -= 1;
                    bail!("no session for lloyd {left}")
                }
                Ok("session")
            },
        );
        (calls.into_inner(), result)
    }

    #[test]
    fn safari_ends_the_stale_sessions_before_the_first_try() {
        let (calls, result) = calls_of(start_plan(Browser::Safari), 0);
        assert_eq!(calls, ["end", "start"]);
        assert_eq!(result.unwrap(), "session");
    }

    #[test]
    fn a_failed_safari_start_is_tried_once_more() {
        let (calls, result) = calls_of(start_plan(Browser::Safari), 1);
        assert_eq!(calls, ["end", "start", "end", "start"]);
        assert_eq!(result.unwrap(), "session");
        let (calls, result) = calls_of(start_plan(Browser::Safari), 2);
        assert_eq!(calls, ["end", "start", "end", "start"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "the session did not start; try 1: no session for lloyd 1; try 2: no session for lloyd 0"
        );
    }

    #[test]
    fn a_failed_chrome_start_is_not_tried_again() {
        let (calls, result) = calls_of(start_plan(Browser::Chrome), 1);
        assert_eq!(calls, ["start"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "the session did not start; try 1: no session for lloyd 0"
        );
    }

    /// A one-shot fake driver: answers one request with `reply` and sends
    /// back what it received (request line, then body).
    fn fake(status: &'static str, body: &'static str) -> (u16, mpsc::Receiver<(String, String)>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
            }
            let mut received = vec![0; length];
            reader.read_exact(&mut received).unwrap();
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
            // Held open, as chromedriver does: the client must stop at the
            // end of the body, not wait for the connection to close.
            let _held = stream;
            tx.send((
                request_line.trim().to_string(),
                String::from_utf8(received).unwrap(),
            ))
            .unwrap();
            std::thread::sleep(Duration::from_secs(30));
        });
        (port, rx)
    }

    /// The fake holds the socket open for 30 s, and the timeout is 5 s, so
    /// a client that reads to the end of the stream fails this test.
    #[test]
    fn sends_json_and_reads_a_sized_reply() {
        let (port, rx) = fake("200 OK", r#"{"value":"lloyd"}"#);
        let (status, body) = http(
            port,
            "POST",
            "/session/1/url",
            Some(&json!({"url": "x"})),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!((status, body), (200, json!({"value": "lloyd"})));
        let (line, sent) = rx.recv().unwrap();
        assert_eq!(line, "POST /session/1/url HTTP/1.1");
        assert_eq!(sent, r#"{"url":"x"}"#);
    }

    #[test]
    fn reads_a_chunked_reply() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"val\r\na;ext=1\r\nue\":[1,2]}\r\n0\r\n\r\n";
        assert_eq!(
            parse_response(raw).unwrap(),
            (200, json!({"value": [1, 2]}))
        );
    }

    #[test]
    fn a_silent_driver_times_out() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (_held, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(30));
        });
        let t0 = Instant::now();
        let e = http(port, "GET", "/status", None, Duration::from_millis(300)).unwrap_err();
        assert!(t0.elapsed() < Duration::from_secs(5));
        assert!(format!("{e:#}").contains("no answer within"), "{e:#}");
    }

    #[test]
    fn knows_when_a_response_is_complete() {
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n").unwrap());
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{").unwrap());
        assert!(is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap());
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n";
        assert!(!is_complete(chunked).unwrap());
        assert!(is_complete(&[&chunked[..], b"0\r\n\r\n"].concat()).unwrap());
        assert!(
            !is_complete(b"HTTP/1.1 200 OK\r\n\r\n{}").unwrap(),
            "unframed: until EOF"
        );
    }

    #[test]
    fn obeys_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}trailing";
        assert_eq!(parse_response(raw).unwrap(), (200, json!({})));
    }

    #[test]
    fn reads_an_empty_reply_as_null() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(parse_response(raw).unwrap(), (200, Value::Null));
    }

    #[test]
    fn a_webdriver_error_carries_its_name_and_message() {
        let (port, _rx) = fake(
            "404 Not Found",
            r#"{"value":{"error":"no such window","message":"gone"}}"#,
        );
        let (status, body) = http(port, "GET", "/session/1/url", None, COMMAND_TIMEOUT).unwrap();
        let e = value_of(status, body).unwrap_err().to_string();
        assert_eq!(e, "WebDriver 404 no such window: gone");
    }

    #[test]
    fn rejects_a_reply_without_headers_end() {
        assert!(parse_response(b"HTTP/1.1 200 OK\r\n").is_err());
        assert!(
            parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n").is_err()
        );
    }

    /// The most bytes of a Unix socket path on macOS (`sun_path`), with its
    /// end NUL.
    const SOCKET_PATH_MAX: usize = 104;

    /// The longest part of Chrome's socket path after its temporary
    /// folder: the folder of Chrome for Testing's bundle ID, then the
    /// socket.
    const LONGEST_SOCKET_TAIL: &str = "/.com.google.chrome.for.testing.XXXXXX/SingletonSocket";

    /// An empty folder of its own for one test, never the real
    /// [`CHROME_FOLDER_PARENT`].
    fn test_parent(name: &str) -> PathBuf {
        let parent = std::env::temp_dir().join(format!(
            "xtask-chrome-folders-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        parent
    }

    #[test]
    fn only_chrome_on_macos_gets_a_chrome_folder() {
        assert!(wants_chrome_folder(Browser::Chrome, true));
        assert!(!wants_chrome_folder(Browser::Chrome, false));
        assert!(!wants_chrome_folder(Browser::Safari, true));
        assert!(!wants_chrome_folder(Browser::Safari, false));
    }

    /// Asserts:
    /// * the path is `<parent>/<pid>-<n>`;
    /// * with the longest `pid` and `n`, Chrome's longest socket path fits
    ///   in `sun_path`, with its end NUL.
    #[test]
    fn a_chrome_folder_leaves_room_for_chromes_socket() {
        let parent = Path::new(CHROME_FOLDER_PARENT);
        assert_eq!(
            chrome_folder_path(parent, 4242, 7),
            Path::new("/tmp/stemyla-chrome/4242-7")
        );
        let longest = chrome_folder_path(parent, u32::MAX, u32::MAX);
        assert_eq!(
            longest,
            Path::new("/tmp/stemyla-chrome/4294967295-4294967295")
        );
        let socket = format!("{}{LONGEST_SOCKET_TAIL}", longest.display());
        assert!(
            socket.len() < SOCKET_PATH_MAX,
            "{} bytes: {socket}",
            socket.len()
        );
    }

    fn env_of(cmd: &Command, name: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
    }

    #[test]
    fn the_chrome_driver_gets_its_chrome_folder() {
        let folder = Path::new("/tmp/stemyla-chrome/4242-0");
        let chrome = driver_command(
            Browser::Chrome,
            Path::new("chromedriver"),
            9515,
            Some(folder),
        );
        assert_eq!(
            env_of(&chrome, CHROME_TMP_VAR).as_deref(),
            Some("/tmp/stemyla-chrome/4242-0")
        );
        let args: Vec<_> = chrome.get_args().collect();
        assert_eq!(args, ["--port=9515"]);
        let without = driver_command(Browser::Chrome, Path::new("chromedriver"), 9515, None);
        assert_eq!(env_of(&without, CHROME_TMP_VAR), None);
        let safari = driver_command(
            Browser::Safari,
            Path::new("/usr/bin/safaridriver"),
            9515,
            Some(folder),
        );
        assert_eq!(env_of(&safari, CHROME_TMP_VAR), None);
    }

    /// A Chrome folder in `parent`, with one file in it.
    fn folder_with_a_file(parent: &Path) -> PathBuf {
        let folder = ChromeFolder::create(parent).unwrap();
        let path = folder.path().to_path_buf();
        std::fs::create_dir(path.join(".org.chromium.Chromium.lloyd")).unwrap();
        std::fs::write(path.join(".org.chromium.Chromium.lloyd/Cookies"), "acme").unwrap();
        // Ends the guard's scope with an error, through `?`.
        let failed = (|| -> Result<()> {
            let _folder = folder;
            bail!("the scenario failed for lloyd")
        })();
        assert!(failed.is_err());
        path
    }

    #[test]
    fn a_chrome_folder_is_removed_on_drop_also_after_an_error() {
        let parent = test_parent("drop");
        let path = folder_with_a_file(&parent);
        assert!(
            path.starts_with(&parent) && path != parent,
            "{} is not in {}",
            path.display(),
            parent.display()
        );
        assert!(!path.exists(), "{} is still there", path.display());
        std::fs::remove_dir_all(parent).unwrap();
    }

    /// A parent that is a symbolic link is refused, so the sweep reads no
    /// folder that a link points to.
    #[test]
    #[cfg(unix)]
    fn a_link_as_the_parent_is_refused() {
        let parent = test_parent("link");
        let target = parent.join("target");
        std::fs::create_dir(&target).unwrap();
        let link = parent.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let made = ChromeFolder::create(&link);
        let error = made.as_ref().err().map(|e| e.to_string());
        drop(made);
        std::fs::remove_dir_all(&parent).unwrap();
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("is not a folder of the current user")),
            "{error:?}"
        );
    }

    #[test]
    fn a_failed_session_start_removes_the_chrome_folder() {
        let parent = test_parent("session");
        let folder = ChromeFolder::create(&parent).unwrap();
        let path = folder.path().to_path_buf();
        let (port, _rx) = fake(
            "500 Internal Server Error",
            r#"{"value":{"error":"session not created","message":"no Chrome for lloyd"}}"#,
        );
        let driver = Driver {
            child: Command::new("/bin/sleep").arg("30").spawn().unwrap(),
            port,
            _chrome_folder: Some(folder),
        };
        let error = session_on(driver, Browser::Chrome, &[])
            .err()
            .expect("no session");
        assert!(
            format!("{error:#}").contains("no Chrome for lloyd"),
            "{error:#}"
        );
        assert!(!path.exists(), "{} is still there", path.display());
        std::fs::remove_dir_all(parent).unwrap();
    }

    /// A removal pauses between two tries, and not after the last try.
    #[test]
    fn a_removal_pauses_between_tries_only() {
        let path = Path::new("/tmp/stemyla-chrome/4242-0");
        let pause = Duration::from_millis(200);
        let mut waits = Vec::new();
        let result = remove_retrying_with(
            path,
            3,
            pause,
            |_| Err(std::io::Error::other("busy for lloyd")),
            |d| waits.push(d),
        );
        assert!(result.is_err());
        assert_eq!(waits, [pause, pause], "between 3 tries");
        let mut waits = Vec::new();
        let mut calls = 0;
        let result = remove_retrying_with(
            path,
            3,
            pause,
            |_| {
                calls += 1;
                if calls == 1 {
                    Err(std::io::Error::other("busy for acme"))
                } else {
                    Ok(())
                }
            },
            |d| waits.push(d),
        );
        assert!(result.is_ok());
        assert_eq!(waits, [pause], "one pause before the try that works");
    }

    /// Asserts:
    /// * a removal that fails each time tries `tries` times, and gives the
    ///   last error;
    /// * a removal that fails, then succeeds, stops at the success;
    /// * a path that is not there counts as removed;
    /// * the line of a failed removal names the path and the error.
    #[test]
    fn a_removal_tries_again_and_a_failure_names_the_path() {
        let path = Path::new("/tmp/stemyla-chrome/4242-0");
        let mut calls = 0;
        let result = remove_retrying(path, 3, Duration::ZERO, |_| {
            calls += 1;
            Err(std::io::Error::other("busy for lloyd"))
        });
        assert_eq!(calls, 3);
        let error = result.unwrap_err();
        assert_eq!(error.to_string(), "busy for lloyd");
        let mut calls = 0;
        let result = remove_retrying(path, 3, Duration::ZERO, |_| {
            calls += 1;
            if calls == 1 {
                Err(std::io::Error::other("busy for lloyd"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_ok());
        assert_eq!(calls, 2);
        let gone = remove_retrying(path, 3, Duration::ZERO, |_| {
            Err(std::io::ErrorKind::NotFound.into())
        });
        assert!(gone.is_ok());
        assert_eq!(
            removal_failed_line(path, &error),
            "chrome folder: could not remove /tmp/stemyla-chrome/4242-0 (busy for lloyd); remove it by hand"
        );
    }

    #[test]
    fn a_command_names_a_path_only_as_a_whole_part() {
        let path = "/tmp/stemyla-chrome/4242-1";
        assert!(names_path(
            "chrome --user-data-dir=/tmp/stemyla-chrome/4242-1/.org.chromium.Chromium.x",
            path
        ));
        assert!(names_path("ls /tmp/stemyla-chrome/4242-1", path));
        assert!(names_path("ls /tmp/stemyla-chrome/4242-1 -l", path));
        assert!(names_path("ls '/tmp/stemyla-chrome/4242-1'", path));
        assert!(!names_path("ls /tmp/stemyla-chrome/4242-10", path));
        assert!(!names_path(
            "ls /tmp/stemyla-chrome/4242-1x /tmp/stemyla-chrome/4242-12",
            path
        ));
        assert!(!names_path("ls /tmp", path));
    }

    /// Asserts, for the start sweep's choice:
    /// * it removes a folder `<pid>-<n>` of this user whose process has
    ///   ended;
    /// * it keeps it while its process lives, or while a command line
    ///   names it;
    /// * it keeps a folder of another user, a symbolic link, and each
    ///   other name.
    #[test]
    fn the_sweep_removes_only_a_folder_of_an_ended_run() {
        let ps = "    1 /sbin/launchd\n 4242 /Users/lloyd/stemyla/target/debug/xtask e2e storage\n 5151 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --user-data-dir=/tmp/stemyla-chrome/777-0/.org.chromium.Chromium.lloyd\n";
        let entry = |name: &'static str, is_dir: bool, owner: u32| SweptEntry {
            name,
            path: Box::leak(format!("/tmp/stemyla-chrome/{name}").into_boxed_str()),
            is_dir,
            owner: Some(owner),
        };
        assert!(sweep_removes(&entry("999-0", true, 501), 501, ps));
        assert!(sweep_removes(&entry("999-12", true, 501), 501, ps));
        assert!(
            !sweep_removes(&entry("4242-0", true, 501), 501, ps),
            "a live run"
        );
        assert!(
            !sweep_removes(&entry("777-0", true, 501), 501, ps),
            "named by Chrome"
        );
        assert!(
            !sweep_removes(&entry("999-0", true, 502), 501, ps),
            "another user"
        );
        assert!(
            !sweep_removes(&entry("999-0", false, 501), 501, ps),
            "a link"
        );
        for name in [
            "999", "999-", "-0", "999-0-1", "lloyd-0", "999-x", "+999-0", "999-+0",
        ] {
            assert!(!sweep_removes(&entry(name, true, 501), 501, ps), "{name}");
        }
        let unknown = SweptEntry {
            owner: None,
            ..entry("999-0", true, 501)
        };
        assert!(!sweep_removes(&unknown, 501, ps), "no owner");
    }

    /// The sweep in a folder of its own: it removes the folder of an ended
    /// run, and keeps the others and the file in a kept folder.
    #[test]
    fn the_sweep_removes_an_ended_runs_folder_in_its_parent() {
        let parent = test_parent("sweep");
        let ended = parent.join("999999-0");
        let live = parent.join(format!("{}-0", std::process::id()));
        let named = parent.join("999998-0");
        let other = parent.join("beachape");
        for folder in [&ended, &live, &named, &other] {
            std::fs::create_dir(folder).unwrap();
            std::fs::write(folder.join("Cookies"), "acme").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, parent.join("999997-0")).unwrap();
        let uid = current_uid().unwrap();
        let ps = format!(
            "{} xtask\n 5151 chrome --user-data-dir={}/.org.chromium.Chromium.lloyd\n",
            std::process::id(),
            named.display()
        );
        let removed = sweep(&parent, uid, || Some(ps));
        assert_eq!(removed.as_deref(), Some(std::slice::from_ref(&ended)));
        assert!(!ended.exists());
        for kept in [&live, &named, &other] {
            assert!(kept.join("Cookies").is_file(), "{}", kept.display());
        }
        #[cfg(unix)]
        assert!(parent.join("999997-0").symlink_metadata().is_ok());
        std::fs::remove_dir_all(parent).unwrap();
    }

    /// Finding 1-5 of review round 1: the sweep lists the parent before it
    /// reads the process list.  A run that starts after the listing makes
    /// its folder then, and the list may not have it yet: the folder stays.
    #[test]
    fn a_run_that_starts_during_the_sweep_keeps_its_folder() {
        let parent = test_parent("sweep-race");
        let uid = current_uid().unwrap();
        let starting = parent.join("424242-0");
        let made = starting.clone();
        let removed = sweep(&parent, uid, move || {
            std::fs::create_dir(&made).unwrap();
            Some(format!("{} xtask\n", std::process::id()))
        });
        let kept = starting.is_dir();
        std::fs::remove_dir_all(parent).unwrap();
        assert_eq!(removed, Some(vec![]), "nothing to remove");
        assert!(kept, "the new run's folder stays");
    }

    #[test]
    fn no_process_list_removes_nothing() {
        let parent = test_parent("sweep-no-ps");
        let ended = parent.join("999999-0");
        std::fs::create_dir(&ended).unwrap();
        let uid = current_uid().unwrap();
        let removed = sweep(&parent, uid, || None);
        let kept = ended.is_dir();
        std::fs::remove_dir_all(parent).unwrap();
        assert_eq!(removed, None);
        assert!(kept);
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_decode("bGxveWQ=").unwrap(), b"lloyd");
        assert_eq!(base64_decode("YWNtZQ==").unwrap(), b"acme");
        assert_eq!(base64_decode("YmVhY2hhcGU=").unwrap(), b"beachape");
        assert!(base64_decode("not*base64").is_err());
    }
}
