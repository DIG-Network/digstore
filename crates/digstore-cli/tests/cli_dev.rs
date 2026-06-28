//! `digstore dev` — the FREE local preview loop (roadmap #6).
//!
//! `dev` serves the project over the REAL dig:// read path (compile → verify →
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
/// body as a (lossy) string. Reads raw bytes until EOF (the server sends
/// `Connection: close`), which is robust against partial reads / non-UTF8 — unlike
/// `read_to_string`. Returns None if the connection fails (server not up).
fn http_get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(8))).ok()?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).ok()?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break, // EOF: server closed the connection.
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(_) => break, // timeout / reset: use whatever we have.
        }
    }
    if bytes.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    // Split headers/body on the blank line.
    text.split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
}

/// GET a path, retrying briefly to ride out a transient connection refusal.
fn http_get_retry(port: u16, path: &str) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
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
