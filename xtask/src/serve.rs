//! `xtask serve [dir] [--port N] [--mount /prefix=dir]...`: a static file
//! server for the web app, in plain Rust (standard library only).
//!
//! Browsers will not run the app's wasm module worker from a `file://` URL,
//! so the built `app/dist` has to come from a web server; `localhost` counts
//! as a secure context, which WebGPU and the worker need.  The server binds
//! to 127.0.0.1 only, answers GET and HEAD, refuses paths that climb out of
//! a served directory, and sends `.wasm` as `application/wasm`.
//!
//! `--mount /prefix=dir` serves another directory under a URL prefix (the
//! end-to-end tests use it for weights and test clips).
//!
//! `--inject /url.js` adds `<script src="/url.js"></script>` at the start of
//! every HTML page's `<head>`, so it runs before the page's own scripts
//! (`xtask e2e` uses it for its console capture and audio stand-ins).

use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// URL prefix (always starting and ending with `/`) and the directory it maps to.
pub type Mounts = Vec<(String, PathBuf)>;

/// The folder of the pid files, relative to the repo root.
pub const PID_FOLDER: &str = "target/xtask-serve";

/// The pid file of `xtask serve` on `port`, in the repo at `root`.
/// `xtask clean` reads it to find a server that a run left.
pub fn pid_file(root: &Path, port: u16) -> PathBuf {
    root.join(PID_FOLDER).join(format!("{port}.pid"))
}

pub fn run(args: &[String]) -> Result<()> {
    let mut root = PathBuf::from("app/dist");
    let mut port: u16 = 8080;
    let mut mounts: Mounts = Vec::new();
    let mut inject = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args.get(i + 1).context("--port needs a number")?.parse()?;
                i += 1;
            }
            "--mount" => {
                let spec = args.get(i + 1).context("--mount needs /prefix=dir")?;
                let (prefix, dir) = spec.split_once('=').context("--mount needs /prefix=dir")?;
                mounts.push((normalise_prefix(prefix), PathBuf::from(dir)));
                i += 1;
            }
            "--inject" => {
                inject.push(
                    args.get(i + 1)
                        .context("--inject needs a script URL")?
                        .clone(),
                );
                i += 1;
            }
            other if other.starts_with("--") => bail!("unknown option {other}"),
            dir => root = PathBuf::from(dir),
        }
        i += 1;
    }
    if !root.join("index.html").is_file() {
        bail!(
            "{} has no index.html (build the app first: cd app && trunk build --release)",
            root.display()
        );
    }
    mounts.push(("/".to_string(), root.clone()));
    let listener =
        TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("binding port {port}"))?;
    let addr = listener.local_addr()?;
    // Scripts read this line to find the port when started with --port 0.
    println!("serving {} at http://{addr}/", root.display());
    std::io::stdout().flush()?;
    serve_with(listener, mounts, inject)
}

fn normalise_prefix(p: &str) -> String {
    let p = p.trim_matches('/');
    if p.is_empty() {
        "/".to_string()
    } else {
        format!("/{p}/")
    }
}

/// Accepts connections forever, one thread per connection.
pub fn serve(listener: TcpListener, mounts: Mounts) -> Result<()> {
    serve_with(listener, mounts, Vec::new())
}

/// [`serve`], adding a script tag for each of `inject` to HTML pages.
pub fn serve_with(listener: TcpListener, mut mounts: Mounts, inject: Vec<String>) -> Result<()> {
    // Longest prefix first, so /weights/ wins over /.
    mounts.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
    let site = Arc::new((mounts, script_tags(&inject)));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let site = Arc::clone(&site);
        std::thread::spawn(move || {
            let (mounts, tags) = &*site;
            if let Err(e) = handle(stream, mounts, tags) {
                eprintln!("serve: {e:#}");
            }
        });
    }
    Ok(())
}

fn script_tags(urls: &[String]) -> String {
    urls.iter()
        .map(|u| {
            format!(
                "<script src=\"{}\"></script>",
                u.replace(['"', '<', '>'], "")
            )
        })
        .collect()
}

/// `html` with `tags` at the start of its `<head>` (or of the document,
/// without one).
pub fn inject_into(html: &str, tags: &str) -> String {
    let at = html
        .to_ascii_lowercase()
        .find("<head>")
        .map_or(0, |i| i + "<head>".len());
    format!("{}{tags}{}", &html[..at], &html[at..])
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

/// `%xx` escapes decoded; `None` if the result is not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The file a URL path maps to, or `None` if it would leave the mount.
pub fn resolve(mounts: &[(String, PathBuf)], url_path: &str) -> Option<PathBuf> {
    let path = url_path.split(['?', '#']).next().unwrap_or("/");
    let path = percent_decode(path)?;
    let path = if path.ends_with('/') {
        format!("{path}index.html")
    } else {
        path
    };
    for (prefix, dir) in mounts {
        let rest = if prefix == "/" {
            path.strip_prefix('/')
        } else {
            path.strip_prefix(prefix.as_str())
        };
        let Some(rest) = rest else { continue };
        let rel = Path::new(rest);
        // Only plain names: no "..", no absolute parts, no drive prefixes.
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return None;
        }
        return Some(dir.join(rel));
    }
    None
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}

fn handle(mut stream: TcpStream, mounts: &[(String, PathBuf)], tags: &str) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Skip the headers (bounded, so a client cannot make us read forever).
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        header_bytes += n;
        if n == 0 || line == "\r\n" || line == "\n" || header_bytes > 64 * 1024 {
            break;
        }
    }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method != "GET" && method != "HEAD" {
        return respond(&mut stream, "405 Method Not Allowed", "only GET and HEAD\n");
    }
    let Some(mut path) = resolve(mounts, target) else {
        return respond(&mut stream, "403 Forbidden", "forbidden\n");
    };
    if path.is_dir() {
        path = path.join("index.html");
    }
    let Ok(mut file) = File::open(&path) else {
        return respond(&mut stream, "404 Not Found", "not found\n");
    };
    if !tags.is_empty() && content_type(&path).starts_with("text/html") {
        let mut html = String::new();
        file.read_to_string(&mut html)?;
        let page = inject_into(&html, tags);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
            page.len()
        )?;
        if method == "GET" {
            stream.write_all(page.as_bytes())?;
        }
        stream.flush()?;
        return Ok(());
    }
    let len = file.metadata()?.len();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {len}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        content_type(&path)
    )?;
    if method == "GET" {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            stream.write_all(&buf[..n])?;
        }
    }
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_files_have_their_media_types() {
        for (file, media_type) in [
            ("a.wav", "audio/wav"),
            ("a.mp3", "audio/mpeg"),
            ("a.flac", "audio/flac"),
            ("a.ogg", "application/octet-stream"),
        ] {
            assert_eq!(content_type(Path::new(file)), media_type, "{file}");
        }
    }
}
