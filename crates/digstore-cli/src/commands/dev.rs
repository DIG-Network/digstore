//! `digstore dev` — the free local preview loop.
//!
//! This is the core inner loop the platform was missing: watch the project,
//! rebuild on save, and serve the site over the REAL dig:// read path locally —
//! compile the content into a genuine module, then for each request drive that
//! module through the host runtime, verify the merkle proof against the root, and
//! AES-256-GCM-decrypt the bytes, exactly as a visitor's browser does. It is
//! FREE: no wallet, no chain, no spend, no singleton.
//!
//! Faithfulness matters: `dev` reuses the SAME plumbing as `compile`/`commit`
//! (`store_ops::init_store` + `add_files` + `commit`) to build the module, and
//! the SAME read path as `cat` (`serve::read_resource_plaintext`) to serve it. So
//! if it renders under `dev`, it renders after `deploy`.
//!
//! Two developer conveniences are layered ON TOP of the served bytes (never baked
//! into the deployed capsule): an injected dev `window.chia` shim so wallet flows
//! can be built without a real wallet, and a live-reload poller that refreshes the
//! page when a rebuild lands. Both are injected only into `text/html` responses at
//! request time.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use digstore_core::Bytes32;

use crate::cli::DevArgs;
use crate::context::CliContext;
use crate::dig_toml::DigToml;
use crate::error::CliError;
use crate::ops::{discovery, serve, store_ops};
use crate::ui::Ui;

/// The dev `window.chia` shim + live-reload script injected into served HTML.
/// Kept in a sibling file so it is easy to read/maintain as plain JS.
const DEV_SHIM_JS: &str = include_str!("../../assets/dev-shim.js");

/// One successful build of the project: an ephemeral store context plus the root
/// and resource list to serve from it.
struct Build {
    ctx: CliContext,
    root: Bytes32,
    cfg: digstore_core::StoreConfig,
    /// Committed resource keys (e.g. `index.html`, `app.js`).
    keys: Vec<String>,
}

/// Server state shared between the HTTP handlers and the watch loop. The `Build`
/// is swapped wholesale on each rebuild; `version` increments so the in-page
/// live-reload poller can detect a new build and refresh.
struct DevState {
    build: Mutex<Build>,
    version: std::sync::atomic::AtomicU64,
    /// A monotonically-unique dir under which each rebuild gets its own `.dig`, so
    /// a new build never collides with the old store's on-disk layout.
    work_root: PathBuf,
    counter: std::sync::atomic::AtomicU64,
}

pub fn run(ctx: &CliContext, ui: &Ui, args: DevArgs) -> Result<(), CliError> {
    // 1. Resolve the content dir + build command (precedence: flags > env > dig.toml).
    let file = DigToml::read_with_env(&ctx.op_dir)?;
    let content_rel = args
        .dir
        .clone()
        .or(file.output_dir)
        .unwrap_or_else(|| ".".to_string());
    let build_command = args.build_command.clone().or(file.build_command);

    let content_dir = if Path::new(&content_rel).is_absolute() {
        PathBuf::from(&content_rel)
    } else {
        ctx.op_dir.join(&content_rel)
    };

    // 2. A private scratch area for the ephemeral build stores (cleaned on exit).
    let work_root = std::env::temp_dir().join(format!("digstore-dev-{}", std::process::id()));
    std::fs::create_dir_all(&work_root).map_err(|e| CliError::Other(e.into()))?;

    let counter = std::sync::atomic::AtomicU64::new(0);

    // 3. First build (fails fast if there is nothing to serve).
    let first = build_once(
        ui,
        &content_dir,
        build_command.as_deref(),
        &work_root,
        &counter,
    )?;

    let state = Arc::new(DevState {
        build: Mutex::new(first),
        version: std::sync::atomic::AtomicU64::new(1),
        work_root: work_root.clone(),
        counter,
    });

    let bind = format!("127.0.0.1:{}", args.port);

    // 4. Serve. Build the router and block on the async server.
    let app = Router::new()
        .route("/__dig/reload", get(reload_handler))
        .route("/", get(asset_handler))
        .route("/*path", get(asset_handler))
        .with_state(state.clone());

    // A multi-thread runtime so the preview can handle a page's burst of asset
    // requests concurrently (a current-thread runtime serializes connections and
    // makes a real browser's parallel fetches stall).
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(anyhow::anyhow!("tokio runtime: {e}")))?;

    // BIND FIRST, then announce. Binding before printing the URL means:
    //   - a port already in use fails immediately (a clear error, not a silent
    //     "serving" line that never answers), and
    //   - `--port 0` works: the OS assigns a free port and we report the REAL one,
    //     so callers/tests never race a fixed port or a slow startup — the URL is
    //     only emitted once the listener is actually accepting connections.
    let result = rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .map_err(|e| CliError::Other(anyhow::anyhow!("bind {bind}: {e}")))?;
        // The actual bound address (resolves `:0` to the OS-assigned port).
        let local = listener
            .local_addr()
            .map_err(|e| CliError::Other(anyhow::anyhow!("local_addr: {e}")))?;
        let url = format!("http://{local}/");

        // Announce now that we are bound (stdout so a parent process/test can read
        // the real URL even when the rest is quiet).
        if ui.json() {
            ui.emit_json(&serde_json::json!({
                "serving": true,
                "url": url,
                "port": local.port(),
                "content_dir": content_dir.display().to_string(),
                "spent": false,
                "mocked": false,
            }));
        } else {
            ui.success(format!("Preview serving at {url}"));
            ui.line(format!("  content: {}", content_dir.display()));
            ui.line("  live reload on save · dev window.chia shim injected · no spend");
            ui.line("  Press Ctrl-C to stop.");
        }
        if args.open {
            let _ = open_in_browser(&url);
        }

        // Spawn the watch loop only AFTER we are serving: poll the content dir,
        // rebuild + bump version on change. A failed rebuild keeps the last good
        // build serving.
        {
            let state = state.clone();
            let content_dir = content_dir.clone();
            let build_command = build_command.clone();
            let poll = args.poll.max(1);
            let ui = ui.clone();
            std::thread::spawn(move || {
                watch_loop(&ui, &state, &content_dir, build_command.as_deref(), poll);
            });
        }

        axum::serve(listener, app)
            .await
            .map_err(|e| CliError::Other(anyhow::anyhow!("server error: {e}")))
    });

    // Best-effort cleanup of the scratch area. Use the outer `work_root` (the same
    // path `state.work_root` holds) since `state` was moved into the server block.
    let _ = std::fs::remove_dir_all(&work_root);
    result
}

/// Run the optional build command, then compile the content dir into a fresh
/// ephemeral store and return the [`Build`]. Reuses the exact `compile`/`commit`
/// local pipeline so the served bytes are byte-identical to a real deploy.
fn build_once(
    ui: &Ui,
    content_dir: &Path,
    build_command: Option<&str>,
    work_root: &Path,
    counter: &std::sync::atomic::AtomicU64,
) -> Result<Build, CliError> {
    if let Some(cmd) = build_command {
        run_build(ui, content_dir, cmd)?;
    }
    if !content_dir.is_dir() {
        return Err(CliError::InvalidArgument(format!(
            "content directory '{}' does not exist (set --dir or `output-dir` in dig.toml)",
            content_dir.display()
        )));
    }

    // Each build gets its own `.dig` so the new module never collides with the
    // previous one (init_store refuses an already-initialized store).
    let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dig_dir = work_root.join(format!("build-{n}"));
    std::fs::create_dir_all(&dig_dir).map_err(|e| CliError::Other(e.into()))?;

    // op_dir == the content dir so resource keys are relative to the site root.
    let ctx = CliContext {
        dig_dir: dig_dir.clone(),
        workspace_dir: dig_dir.clone(),
        op_dir: content_dir.to_path_buf(),
        store_name: Some("default".to_string()),
        json: true,
        verbose: false,
    };

    // Scaffold a public, chainless store with a deterministic-enough random id.
    // No mint, no anchor — this is the headless compile pipeline.
    store_ops::init_store(&ctx, false, None, None, None, None, None, None)?;

    let staged = store_ops::add_files(&ctx, &[], true, false, None)?;
    if staged.staged.is_empty() {
        return Err(CliError::InvalidArgument(format!(
            "no files to serve under {}",
            content_dir.display()
        )));
    }

    // Local commit (NO chain) — computes the root, writes the generation +
    // compiles the self-serving module, exactly as `compile` does.
    let outcome = store_ops::commit(&ctx, None, serve::empty_manifest())?;
    let cfg = ctx.load_config()?;
    let keys = store_ops::list_generation_resources(&ctx, &outcome.roothash)?;

    Ok(Build {
        ctx,
        root: outcome.roothash,
        cfg,
        keys,
    })
}

/// Run a shell build command from the content dir's parent (the project root).
fn run_build(ui: &Ui, content_dir: &Path, cmd: &str) -> Result<(), CliError> {
    if !ui.json() {
        ui.line(format!("building: {cmd}"));
    }
    // Build from the project root (the content dir's parent), matching `deploy`.
    let run_dir = content_dir.parent().unwrap_or(content_dir);
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", cmd]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    let status = command
        .current_dir(run_dir)
        .status()
        .map_err(|e| CliError::Other(anyhow::anyhow!("spawn build command: {e}")))?;
    if !status.success() {
        return Err(CliError::Other(anyhow::anyhow!(
            "build command failed with status {status}"
        )));
    }
    Ok(())
}

/// The watch loop: poll the content dir's modification fingerprint and rebuild on
/// change. A rebuild failure is reported but keeps the last good build serving, so
/// a broken save never takes the preview down.
fn watch_loop(
    ui: &Ui,
    state: &Arc<DevState>,
    content_dir: &Path,
    build_command: Option<&str>,
    poll_secs: u64,
) {
    let mut last = fingerprint(content_dir);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(poll_secs));
        let now = fingerprint(content_dir);
        if now == last {
            continue;
        }
        last = now;
        match build_once(
            ui,
            content_dir,
            build_command,
            &state.work_root,
            &state.counter,
        ) {
            Ok(build) => {
                // Swap in the new build, drop the old ctx's on-disk dir afterwards.
                let old_dir = {
                    let mut guard = state
                        .build
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let old = std::mem::replace(&mut *guard, build);
                    old.ctx.dig_dir
                };
                state
                    .version
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = std::fs::remove_dir_all(&old_dir);
                if !ui.json() {
                    ui.success("rebuilt");
                }
            }
            Err(e) => {
                if !ui.json() {
                    ui.line(format!("rebuild failed (keeping last good build): {e}"));
                }
            }
        }
    }
}

/// A cheap change fingerprint: the count + the newest mtime across the content
/// tree. Good enough to detect saves without a filesystem-notify dependency.
fn fingerprint(dir: &Path) -> (u64, u128) {
    let mut count = 0u64;
    let mut newest = 0u128;
    for entry in ignore::WalkBuilder::new(dir)
        .hidden(false)
        .git_ignore(true)
        .build()
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        // Skip our own ephemeral `.dig` dirs if they live under the tree.
        if path.components().any(|c| c.as_os_str() == ".dig") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            count += 1;
            if let Ok(modified) = meta.modified() {
                if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                    newest = newest.max(dur.as_millis());
                }
            }
        }
    }
    (count, newest)
}

/// `GET /__dig/reload` → the current build version as plain text. The injected
/// live-reload poller compares it and refreshes the page when it changes.
async fn reload_handler(State(state): State<Arc<DevState>>) -> Response {
    let v = state.version.load(std::sync::atomic::Ordering::SeqCst);
    Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(v.to_string()))
        .unwrap()
}

/// Serve a resource through the real dig:// read path. Resolves the request path
/// to a resource key (defaulting `/` and directory paths to `index.html`), reads
/// + verifies + decrypts it, and — for HTML — injects the dev shims.
async fn asset_handler(State(state): State<Arc<DevState>>, uri: axum::http::Uri) -> Response {
    let raw = uri.path().trim_start_matches('/');
    // Decode %20 etc.; fall back to the raw path on a decode failure.
    let decoded = percent_decode(raw);

    let result = {
        // Recover from a poisoned lock (a prior request/rebuild panicked) rather
        // than cascading the panic into a connection reset — the `Build` data is
        // still valid to read, so the preview keeps serving instead of one bad
        // request silently taking the whole server down for later fetches.
        let build = state
            .build
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = resolve_key(&build, &decoded);
        key.and_then(|k| {
            serve::read_resource_plaintext(&build.ctx, &build.cfg, &build.root, &k)
                .map(|bytes| (k, bytes))
                .map_err(|e| e.to_string())
        })
    };

    match result {
        Ok((key, bytes)) => {
            let ctype = discovery::infer_content_type(&key);
            let body = if ctype.starts_with("text/html") {
                inject_shims(&bytes)
            } else {
                bytes
            };
            Response::builder()
                .header(header::CONTENT_TYPE, ctype)
                .header(header::CACHE_CONTROL, "no-store")
                .body(Body::from(body))
                .unwrap()
        }
        Err(_) => not_found(&decoded),
    }
}

/// Resolve a request path to a committed resource key. `/` and any path that is
/// not itself a key fall back to `<path>/index.html` then `index.html` (the §8.5
/// default view), so client-side routers and directory URLs work in preview.
fn resolve_key(build: &Build, path: &str) -> Result<String, String> {
    let has = |k: &str| build.keys.iter().any(|x| x == k);

    if path.is_empty() {
        return if has(digstore_core::DEFAULT_RESOURCE_KEY) {
            Ok(digstore_core::DEFAULT_RESOURCE_KEY.to_string())
        } else {
            Err("no index.html".to_string())
        };
    }
    if has(path) {
        return Ok(path.to_string());
    }
    // Directory-style fallback: `/about` → `about/index.html`.
    let nested = format!("{}/index.html", path.trim_end_matches('/'));
    if has(&nested) {
        return Ok(nested);
    }
    // SPA fallback: serve the root index for unknown ROUTES (extension-less paths)
    // so client-side routing works. A missing ASSET (a path with a file extension,
    // e.g. `logo.png`/`app.js`) is a genuine 404 — never masked with HTML.
    let looks_like_asset = path
        .rsplit('/')
        .next()
        .map(|seg| seg.contains('.'))
        .unwrap_or(false);
    if !looks_like_asset && has(digstore_core::DEFAULT_RESOURCE_KEY) {
        return Ok(digstore_core::DEFAULT_RESOURCE_KEY.to_string());
    }
    Err(format!("no resource for '{path}'"))
}

/// Inject the dev `window.chia` shim + live-reload script just before `</head>`
/// (or, lacking one, prepend it). Operates on the decrypted HTML bytes only.
fn inject_shims(html: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(html);
    let snippet = format!("<script>\n{DEV_SHIM_JS}\n</script>");
    let injected = if let Some(idx) = text.to_lowercase().find("</head>") {
        let mut out = String::with_capacity(text.len() + snippet.len());
        out.push_str(&text[..idx]);
        out.push_str(&snippet);
        out.push_str(&text[idx..]);
        out
    } else {
        format!("{snippet}{text}")
    };
    injected.into_bytes()
}

fn not_found(path: &str) -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(format!("404 — no resource for '/{path}'")))
        .unwrap()
}

/// Minimal percent-decoding for request paths (enough for spaces/unicode names).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Best-effort "open this URL in the default browser".
fn open_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ui() -> Ui {
        Ui::resolve(
            crate::ui::ColorChoice::Never,
            true,
            true,
            true,
            false,
            false,
        )
    }

    fn write(dir: &Path, name: &str, body: &str) {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    /// The core promise: `dev` builds via the real pipeline and the resulting
    /// resource decrypts back to the EXACT source bytes through the real read
    /// path — with no chain and no spend.
    #[test]
    fn build_serves_real_read_path_bytes() {
        let content = TempDir::new().unwrap();
        write(
            content.path(),
            "index.html",
            "<html><head></head><body>hi</body></html>",
        );
        write(content.path(), "app.js", "console.log('x')");
        let work = TempDir::new().unwrap();
        let counter = std::sync::atomic::AtomicU64::new(0);

        let build = build_once(&ui(), content.path(), None, work.path(), &counter).unwrap();
        assert!(build.keys.iter().any(|k| k == "index.html"));

        // Read index.html back through the genuine serve→verify→decrypt path.
        let bytes =
            serve::read_resource_plaintext(&build.ctx, &build.cfg, &build.root, "index.html")
                .unwrap();
        assert_eq!(bytes, b"<html><head></head><body>hi</body></html>");
    }

    #[test]
    fn empty_content_dir_errors() {
        let content = TempDir::new().unwrap();
        let work = TempDir::new().unwrap();
        let counter = std::sync::atomic::AtomicU64::new(0);
        match build_once(&ui(), content.path(), None, work.path(), &counter) {
            Err(CliError::InvalidArgument(_)) => {}
            Err(other) => panic!("expected InvalidArgument, got {other:?}"),
            Ok(_) => panic!("expected an error for an empty content dir"),
        }
    }

    #[test]
    fn html_injection_adds_window_chia_and_reload() {
        let injected = inject_shims(b"<html><head></head><body>x</body></html>");
        let s = String::from_utf8(injected).unwrap();
        assert!(s.contains("window.chia"), "injects the dev wallet shim");
        assert!(s.contains("__dig/reload"), "injects the live-reload poller");
        // Injected before </head>.
        assert!(s.find("window.chia").unwrap() < s.find("</head>").unwrap());
    }

    #[test]
    fn resolve_key_defaults_and_spa_fallback() {
        let content = TempDir::new().unwrap();
        write(
            content.path(),
            "index.html",
            "<html><head></head>root</html>",
        );
        write(
            content.path(),
            "about/index.html",
            "<html><head></head>about</html>",
        );
        let work = TempDir::new().unwrap();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let build = build_once(&ui(), content.path(), None, work.path(), &counter).unwrap();

        assert_eq!(resolve_key(&build, "").unwrap(), "index.html");
        assert_eq!(resolve_key(&build, "about").unwrap(), "about/index.html");
        // Unknown path → SPA fallback to root index.
        assert_eq!(resolve_key(&build, "deep/route").unwrap(), "index.html");
    }

    #[test]
    fn percent_decode_handles_spaces() {
        assert_eq!(percent_decode("a%20b.txt"), "a b.txt");
        assert_eq!(percent_decode("plain.txt"), "plain.txt");
    }
}
