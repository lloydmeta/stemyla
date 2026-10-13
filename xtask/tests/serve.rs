//! `xtask serve`: files, MIME types, mounts and path-traversal refusal,
//! over a real socket.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use xtask::serve::{resolve, serve};

fn get(port: u16, path: &str) -> (String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (String::from_utf8(buf[..split].to_vec()).unwrap(), buf[split + 4..].to_vec())
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
    let mounts = vec![("/".to_string(), site.clone()), ("/extra/".to_string(), extra.clone())];
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
    assert!(get(port, "/%2e%2e/secret.txt").0.starts_with("HTTP/1.1 403"));
    assert!(get(port, "/extra/../../secret.txt").0.starts_with("HTTP/1.1 403"));

    let m = vec![("/".to_string(), PathBuf::from("/srv"))];
    assert_eq!(resolve(&m, "/js/app.js"), Some(PathBuf::from("/srv/js/app.js")));
    assert_eq!(resolve(&m, "//etc/passwd"), None);
    let _ = std::fs::remove_dir_all(&dir);
}
