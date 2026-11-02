//! `xtask serve`: files, MIME types, mounts and path-traversal refusal,
//! over a real socket.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use xtask::serve::{inject_into, resolve, serve, serve_with};

fn get(port: u16, path: &str) -> (String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (
        String::from_utf8(buf[..split].to_vec()).unwrap(),
        buf[split + 4..].to_vec(),
    )
}

#[test]
fn serves_files_with_types_and_refuses_escapes() {
    let dir = std::env::temp_dir().join(format!("xtask-serve-{}", std::process::id()));
    let site = dir.join("site");
    let extra = dir.join("extra");
    std::fs::create_dir_all(&site).unwrap();
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::write(site.join("index.html"), "<p>hi</p>").unwrap();
    std::fs::write(site.join("a b.wasm"), [0u8, 97, 115, 109]).unwrap();
    std::fs::write(dir.join("secret.txt"), "no").unwrap();
    std::fs::write(extra.join("clip.wav"), "RIFF").unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mounts = vec![
        ("/".to_string(), site.clone()),
        ("/extra/".to_string(), extra.clone()),
    ];
    std::thread::spawn(move || serve(listener, mounts));

    let (h, body) = get(port, "/?x=1");
    assert!(h.starts_with("HTTP/1.1 200"), "{h}");
    assert!(h.contains("text/html"));
    assert_eq!(body, b"<p>hi</p>");

    let (h, body) = get(port, "/a%20b.wasm");
    assert!(h.contains("Content-Type: application/wasm"), "{h}");
    assert_eq!(body, [0u8, 97, 115, 109]);

    let (h, body) = get(port, "/extra/clip.wav");
    assert!(h.contains("audio/wav"), "{h}");
    assert_eq!(body, b"RIFF");

    assert!(get(port, "/missing.js").0.starts_with("HTTP/1.1 404"));
    assert!(get(port, "/../secret.txt").0.starts_with("HTTP/1.1 403"));
    assert!(
        get(port, "/%2e%2e/secret.txt")
            .0
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        get(port, "/extra/../../secret.txt")
            .0
            .starts_with("HTTP/1.1 403")
    );

    let m = vec![("/".to_string(), PathBuf::from("/srv"))];
    assert_eq!(
        resolve(&m, "/js/app.js"),
        Some(PathBuf::from("/srv/js/app.js"))
    );
    assert_eq!(resolve(&m, "//etc/passwd"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn injects_scripts_into_html_pages_only() {
    let dir = std::env::temp_dir().join(format!("xtask-serve-inject-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!DOCTYPE html><html><HEAD><script type=module src=app.js></script></HEAD></html>",
    )
    .unwrap();
    std::fs::write(dir.join("app.js"), "<head>").unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let inject = vec!["/__e2e/log.js".to_string(), "/x.js\"><b>".to_string()];
    std::thread::spawn(move || serve_with(listener, vec![("/".to_string(), dir)], inject));

    let (h, body) = get(port, "/");
    assert!(
        h.contains(&format!("Content-Length: {}", body.len())),
        "{h}"
    );
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "<!DOCTYPE html><html><HEAD><script src=\"/__e2e/log.js\"></script><script src=\"/x.jsb\"></script><script type=module src=app.js></script></HEAD></html>",
        "first in the head, quotes and angle brackets dropped from URLs"
    );
    assert_eq!(
        get(port, "/app.js").1,
        b"<head>",
        "scripts are served as they are"
    );
}

#[test]
fn injects_at_the_start_without_a_head() {
    assert_eq!(
        inject_into("<p>hi</p>", "<script></script>"),
        "<script></script><p>hi</p>"
    );
}

/// The `xtask serve` binary, stopped and reaped when the test ends, also
/// when an assert fails.
struct Server(std::process::Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts `xtask serve <site> --port 0` in `dir`, and gives the server and
/// its first line on stdout.
fn start_serve(dir: &std::path::Path, site: &std::path::Path) -> (Server, String) {
    use std::io::BufRead;
    let mut server = Server(
        std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["serve", site.to_str().unwrap(), "--port", "0"])
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    std::io::BufReader::new(server.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    (server, line)
}

/// The port of a line `serving <dir> at http://127.0.0.1:<port>/`.
fn port_of(line: &str) -> u16 {
    line.trim()
        .rsplit(':')
        .next()
        .and_then(|p| p.strip_suffix('/'))
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("no port in {line:?}"))
}

#[test]
fn serve_writes_its_pid_file_in_the_repo() {
    let dir = std::env::temp_dir().join(format!("xtask-serve-pid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    let site = dir.join("site");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::create_dir_all(&site).unwrap();
    std::fs::write(site.join("index.html"), "<p>hi</p>").unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(init.success());

    let (server, line) = start_serve(&repo.join("sub"), &site);
    assert!(line.starts_with("serving "), "{line:?}");
    let port = port_of(&line);
    let root = repo.canonicalize().unwrap();
    let pid_file = xtask::serve::pid_file(&root, port);
    assert_eq!(
        pid_file,
        root.join(format!("target/xtask-serve/{port}.pid"))
    );
    let text = std::fs::read_to_string(&pid_file)
        .unwrap_or_else(|e| panic!("{}: {e}", pid_file.display()));
    assert_eq!(text, format!("{}\n", server.0.id()));
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn serve_outside_a_repo_writes_no_pid_file_and_says_so() {
    use std::io::Read;
    let dir = std::env::temp_dir().join(format!("xtask-serve-norepo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<p>hi</p>").unwrap();
    let (mut server, line) = start_serve(&dir, &dir);
    assert!(line.starts_with("serving "), "{line:?}");
    assert!(!dir.join("target").exists(), "a pid file outside a repo");
    let _ = server.0.kill();
    let _ = server.0.wait();
    let mut stderr = String::new();
    server
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.contains("no pid file"), "{stderr:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
