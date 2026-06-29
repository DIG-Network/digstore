//! `digstore dev` — the FREE local preview loop (roadmap #6).
//!
//! `dev` serves the project over the REAL chia:// read path (compile → verify →
//! decrypt) locally, with NO chain and NO spend, injecting a dev `window.chia`
//! shim + live reload into HTML. This test drives the INSTALLED binary: it
//! scaffolds a project with `digstore new`, starts `dev` on a port, and asserts
//! the served HTML is the decrypted source WITH the injected shims.

mod common;
use assert_cmd::cargo::cargo_bin;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A minimal HTTP/1.1 GET against 127.0.0.1:<port><path>, returning the response
/// body as a (lossy) string.
///
/// Reads the headers, then exactly `Content-Length` body bytes — it does NOT wait
/// for EOF. hyper/axum keeps HTTP/1.1 connections alive (it does not honor the
/// client's `Connection: close` REQUEST header by closing), so an EOF-based read
/// only ends on the socket read timeout; under parallel CI load that timeout made
/// this fragile ("server did not come up"). Reading by Content-Length returns as
/// soon as the body is complete, so the exchange is deterministic regardless of
/// keep-alive or load. Returns None if the connection/exchange fails.
fn http_get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).ok()?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    // 1. Read until the end of the header block (blank line).
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None, // closed before headers were complete
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    };

    // 2. Parse Content-Length (case-insensitive) from the header block.
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
    let content_len: usize = headers
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    // 3. Read exactly Content-Length body bytes (some may already be buffered).
    let mut body = buf[header_end..].to_vec();
    while body.len() < content_len {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// Index of the first occurrence of `needle` in `haystack` (small, no deps).
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// GET a path, retrying to ride out a transient connection refusal OR a slow first response.
///
/// The deadline is generous (30s): the FIRST fetch of a given asset goes through the real chia://
/// read path (compile → verify → decrypt), which on a cold, parallel-loaded CI Windows runner can
/// take several seconds the first time an asset is touched — long enough that a single attempt's read
/// could time out. Retrying over a 30s window (well under the per-request 10s read timeout's worst
/// case × a few attempts) makes the asset fetch as robust as `wait_for_server`'s `/` poll, killing
/// the intermittent `expect("style.css")` panic seen only under CI load.
fn http_get_retry(port: u16, path: &str) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(body) = http_get(port, path) {
            return Some(body);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Poll until the dev server answers `/` (or time out).
fn wait_for_server(port: u16) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        if let Some(body) = http_get(port, "/") {
            if !body.is_empty() {
                return Some(body);
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    None
}

/// RAII guard that kills the dev child process on drop.
struct DevServer(Child);
impl Drop for DevServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Extract the OS-assigned port from a pretty-printed JSON line (`  "port": N,`).
fn parse_port_line(line: &str) -> Option<u16> {
    let t = line.trim();
    let rest = t.strip_prefix("\"port\":")?;
    let digits: String = rest
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<u16>().ok()
}

/// Drain the dev server's stdout on a background thread, returning a receiver that
/// yields the OS-assigned port (`--port 0`) once announced. CRITICAL: the thread
/// keeps READING for the child's whole life, so the child never blocks (or gets a
/// broken-pipe kill) writing to stdout — if the reader were dropped after the port
/// line, the child could die on its next write and the server would vanish. The
/// `dev` command binds FIRST then prints the real port, so this is a reliable
/// readiness signal that avoids a fixed-port collision (the old fixed-port + poll
/// approach was flaky under CI load).
fn spawn_port_reader(child: &mut Child) -> std::sync::mpsc::Receiver<u16> {
    let stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut sent = false;
        for line in reader.lines().map_while(Result::ok) {
            if !sent {
                if let Some(p) = parse_port_line(&line) {
                    let _ = tx.send(p);
                    sent = true;
                }
            }
            // Keep draining after the port is sent so the child's stdout never fills.
        }
    });
    rx
}

#[test]
fn dev_serves_real_read_path_with_injected_shims() {
    let td = tmp_dir();
    // Scaffold a static site to serve.
    Command::new(cargo_bin("digstore"))
        .args(["new", "static-site"])
        .arg(td.path())
        .arg("--force")
        .status()
        .unwrap();

    // `--port 0` => the OS assigns a free port; `--json` makes `dev` print the
    // real bound port once it is accepting connections. This removes both the
    // fixed-port collision and the startup race that made this test flaky in CI.
    let mut child = Command::new(cargo_bin("digstore"))
        .current_dir(td.path())
        .args(["--json", "dev", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let rx = spawn_port_reader(&mut child);
    let _guard = DevServer(child);
    let port = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("dev server did not announce a port");

    let body = wait_for_server(port).expect("dev server did not come up");

    // The served HTML is the decrypted source (the template's <h1>It works.</h1>).
    assert!(
        body.contains("It works."),
        "served page should be the decrypted source HTML; got: {body}"
    );
    // ...with the dev shims injected (window.chia + live reload).
    assert!(
        body.contains("window.chia"),
        "dev must inject a window.chia shim into HTML"
    );
    assert!(
        body.contains("__dig/reload"),
        "dev must inject the live-reload poller"
    );

    // The live-reload endpoint answers a version number.
    let reload = http_get_retry(port, "/__dig/reload").expect("reload endpoint");
    assert!(
        reload.trim().parse::<u64>().is_ok(),
        "reload endpoint returns a version number; got: {reload}"
    );

    // A non-HTML asset is served verbatim (no shim injection).
    let css = http_get_retry(port, "/style.css").expect("style.css");
    assert!(css.contains("--accent"), "css served through the read path");
    assert!(!css.contains("window.chia"), "no shim in non-HTML assets");
}

fn tmp_dir() -> tempfile::TempDir {
    tempfile::TempDir::new().unwrap()
}
