//! A small blocking WebDriver client (W3C WebDriver, HTTP/1.1 + JSON to a
//! local driver), with only the commands the end-to-end scenarios need.
//! Spec: https://www.w3.org/TR/webdriver2/
//!
//! Every request opens its own connection.  A response ends after its
//! `Content-Length` or its last chunk (drivers may keep the connection
//! open whatever the request asked), or else where the stream does.

use crate::drivers::Browser;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
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

/// A WebDriver process on a free local port, stopped when dropped.
pub struct Driver {
    child: Child,
    pub port: u16,
}

impl Driver {
    pub fn start(browser: Browser) -> Result<Self> {
        let path = browser.driver()?;
        let port = free_port()?;
        let mut cmd = Command::new(&path);
        match browser {
            Browser::Safari => cmd.args(["-p", &port.to_string()]),
            Browser::Chrome => cmd.arg(format!("--port={port}")),
        };
        let child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("starting {}", path.display()))?;
        let driver = Self { child, port };
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

/// A browser session, ended when dropped.
pub struct Session {
    port: u16,
    id: String,
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
        })
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

    #[test]
    fn sends_json_and_reads_a_sized_reply() {
        let (port, rx) = fake("200 OK", r#"{"value":"lloyd"}"#);
        let (status, body) = http(
            port,
            "POST",
            "/session/1/url",
            Some(&json!({"url": "x"})),
            COMMAND_TIMEOUT,
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

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_decode("bGxveWQ=").unwrap(), b"lloyd");
        assert_eq!(base64_decode("YWNtZQ==").unwrap(), b"acme");
        assert_eq!(base64_decode("YmVhY2hhcGU=").unwrap(), b"beachape");
        assert!(base64_decode("not*base64").is_err());
    }
}
