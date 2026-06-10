//! DigStore Installer — Tauri backend.
//!
//! Exposes the commands the frontend (src/bridge.js) calls:
//!   - installer_meta        → { version, compiler }
//!   - default_install_path  → per-OS default dir
//!   - run_install(opts)     → runs the real pipeline, streams events
//!   - cancel_install()      → cooperatively cancels an in-flight install
//!   - launch_terminal(path) → opens a terminal at the install dir
//!
//! The install runs on a background thread so the UI stays responsive while it
//! streams `install://progress` / `install://error` / `install://done`.

mod install;

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Serialize)]
struct Meta {
    version: String,
    compiler: String,
}

struct InstallState {
    cancelled: Arc<AtomicBool>,
}

#[tauri::command]
fn installer_meta(app: AppHandle) -> Meta {
    // Best-effort: ask the bundled binary for its version so the UI shows the
    // truth. Falls back to the spec's 1.0.0 if the binary can't be queried yet.
    let version = bundled_version(&app).unwrap_or_else(|| "1.0.0".to_string());
    Meta { version, compiler: "1.0.0".to_string() }
}

/// Returns the version of the **bundled `digstore` CLI** that this installer
/// will install — i.e. the semver printed by `digstore --version` from the
/// app's resources. This is the version the badge should display (distinct
/// from the installer app's own version). Falls back to "0.3.0" if the binary
/// can't be queried (e.g. missing in a dev run) so the UI never blanks out.
#[tauri::command]
fn bundled_digstore_version(app: AppHandle) -> String {
    bundled_version(&app).unwrap_or_else(|| "0.3.0".to_string())
}

fn bundled_version(app: &AppHandle) -> Option<String> {
    // Embedded single-file build: the version was captured at build time from
    // the binary that was compiled into this installer.
    if let Some(v) = option_env!("DIGSTORE_BUNDLED_VERSION") {
        return Some(v.to_string());
    }
    // Dev fallback: query the staged resource binary directly.
    let res = app.path().resource_dir().ok()?;
    let bin = res.join("bin").join(install::bin_name());
    let bin = if bin.exists() {
        bin
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("bin")
            .join(install::bin_name())
    };
    let out = Command::new(&bin).arg("--version").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    // "digstore 0.1.0" → "0.1.0"
    s.split_whitespace().nth(1).map(|v| v.to_string())
}

#[tauri::command]
fn default_install_path() -> String {
    install::default_install_path()
}

#[tauri::command]
fn run_install(app: AppHandle, state: State<'_, InstallState>, opts: install::InstallOpts) -> Result<(), String> {
    state.cancelled.store(false, Ordering::SeqCst);
    let cancelled = state.cancelled.clone();
    // Run on a worker thread; the pipeline emits its own events.
    std::thread::spawn(move || {
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        // ALWAYS surface a failure: an early `?` in the pipeline (e.g. a missing
        // payload, a write/permission error) returns before its own error emit,
        // which would otherwise leave the UI hung with no message.
        if let Err(e) = install::run(&app, opts) {
            let _ = app.emit("install://error", install::InstallError { message: e });
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_install(state: State<'_, InstallState>) {
    state.cancelled.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn launch_terminal(install_path: String) -> Result<(), String> {
    let bin_dir = std::path::PathBuf::from(&install_path).join("bin");
    let cwd = if bin_dir.exists() { bin_dir } else { std::path::PathBuf::from(&install_path) };

    #[cfg(windows)]
    {
        // Open a new Command Prompt in the install dir.
        Command::new("cmd")
            .args(["/C", "start", "cmd", "/K", "echo digstore installed. Try: digstore --version"])
            .current_dir(&cwd)
            .spawn()
            .map_err(|e| format!("launch terminal: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-a", "Terminal"])
            .arg(&cwd)
            .spawn()
            .map_err(|e| format!("launch terminal: {e}"))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Try common terminals in order.
        let term = ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"]
            .into_iter()
            .find(|t| which(t));
        match term {
            Some(t) => {
                Command::new(t).current_dir(&cwd).spawn().map_err(|e| format!("launch terminal: {e}"))?;
            }
            None => return Err("no terminal emulator found".into()),
        }
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn which(bin: &str) -> bool {
    Command::new("which").arg(bin).output().map(|o| o.status.success()).unwrap_or(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(InstallState { cancelled: Arc::new(AtomicBool::new(false)) })
        .invoke_handler(tauri::generate_handler![
            installer_meta,
            bundled_digstore_version,
            default_install_path,
            run_install,
            cancel_install,
            launch_terminal
        ])
        .run(tauri::generate_context!())
        .expect("error while running DigStore Installer");
}
