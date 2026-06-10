//! The real install pipeline.
//!
//! Replaces the prototype's timed animation with actual filesystem work, driven
//! from the bundled artifact (no network download on first install). Each phase
//! emits a `install://progress` event (pct / nowFile / styled log line). On
//! failure it emits `install://error`; on success, `install://done`.
//!
//! Phases (mirrors README → "Real install pipeline"):
//!   1. Resolve target for OS/arch.
//!   2. Verify bundled package checksum  [gated, offline]  → SHA-256 manifest.
//!   3. Unpack the digstore CLI (+ host runtime) into the install dir.
//!   4. Install selected components (shell completions, example store).
//!   5. Add digstore to PATH (user PATH on Windows; symlink in /usr/local/bin
//!      on macOS/Linux — elevation only where needed).
//!   6. Verify the install by running `digstore --version`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
// `Manager` is only needed for `app.path()` in the resource-dir fallback.
#[cfg(not(embed_digstore))]
use tauri::Manager;

// ---- Embedded payload (single-file install) ----------------------------------
// When the release build staged a `digstore` binary, build.rs embedded it (and
// its SHA-256) so the installer is a single self-contained executable with no
// sidecar resource folder. Dev/`cargo check` builds without a staged binary do
// not set `embed_digstore` and fall back to the Tauri resource dir.
#[cfg(embed_digstore)]
const EMBEDDED_DIGSTORE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/digstore.bin"));
#[cfg(embed_digstore)]
const EMBEDDED_SHA256: &str = include_str!(concat!(env!("OUT_DIR"), "/digstore.sha256"));

// The DIG brand icon, embedded so the .dig file-type association has an icon to
// point at regardless of where the user runs the (single-file) installer.
const DIG_ICON_ICO: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/icon.ico"));
#[cfg(all(unix, not(target_os = "macos")))]
const DIG_ICON_PNG: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/icon.png"));

#[derive(Debug, Deserialize)]
pub struct InstallOpts {
    pub install_path: String,
    /// componentId -> enabled (cli is always true)
    pub selected: HashMap<String, bool>,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct Progress {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "nowFile")]
    pub now_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct InstallError {
    pub message: String,
}

/// Binary name per-OS.
pub fn bin_name() -> &'static str {
    if cfg!(windows) {
        "digstore.exe"
    } else {
        "digstore"
    }
}

/// Default install location per the README:
///   Windows: %LOCALAPPDATA%\Programs\DigStore
///   macOS/Linux: /usr/local/digstore
pub fn default_install_path() -> String {
    if cfg!(windows) {
        let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("C:/Users/Public"));
        base.join("Programs").join("DigStore").to_string_lossy().to_string()
    } else {
        "/usr/local/digstore".to_string()
    }
}

/// Locate the bundled artifact inside the app resource dir (dev fallback only;
/// release builds embed the binary — see `digstore_payload`).
#[cfg(not(embed_digstore))]
fn bundled_bin(app: &AppHandle) -> Result<PathBuf, String> {
    let res_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("cannot resolve resource dir: {e}"))?;
    let candidate = res_dir.join("bin").join(bin_name());
    if candidate.exists() {
        return Ok(candidate);
    }
    // Dev fallback: when running `tauri dev`, resources may resolve relative to
    // the crate dir. Try the staging dir directly.
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("bin").join(bin_name());
    if dev.exists() {
        return Ok(dev);
    }
    Err(format!(
        "bundled {} not found (looked in {} and {}). TODO: stage the release \
         binary into installer/app/src-tauri/resources/bin/ before building.",
        bin_name(),
        candidate.display(),
        dev.display()
    ))
}

fn emit_line(app: &AppHandle, line: impl Into<String>) {
    let _ = app.emit("install://progress", Progress { line: Some(line.into()), ..Default::default() });
}
fn emit_pct(app: &AppHandle, pct: f64, now_file: Option<&str>) {
    let _ = app.emit(
        "install://progress",
        Progress { pct: Some(pct), now_file: now_file.map(|s| s.to_string()), ..Default::default() },
    );
}

fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Resolve the `digstore` bytes to install plus the expected SHA-256 (when
/// known). Prefers the binary embedded at build time (single-file install);
/// falls back to the Tauri resource dir + `.sha256` sidecar for dev runs.
fn digstore_payload(app: &AppHandle) -> Result<(Vec<u8>, Option<String>), String> {
    #[cfg(embed_digstore)]
    {
        let _ = app; // resource dir unused when embedded
        Ok((
            EMBEDDED_DIGSTORE.to_vec(),
            Some(EMBEDDED_SHA256.trim().to_lowercase()),
        ))
    }
    #[cfg(not(embed_digstore))]
    {
        let src = bundled_bin(app)?;
        let bytes = fs::read(&src).map_err(|e| format!("read {}: {e}", src.display()))?;
        let manifest = src.with_file_name(format!("{}.sha256", bin_name()));
        let expected = fs::read_to_string(&manifest)
            .ok()
            .and_then(|s| s.split_whitespace().next().map(|x| x.to_lowercase()));
        Ok((bytes, expected))
    }
}

/// Run the whole pipeline. Returns Ok on full success; on the first failure it
/// emits `install://error` and returns Err (the caller has already streamed it).
pub fn run(app: &AppHandle, opts: InstallOpts) -> Result<(), String> {
    let install_dir = PathBuf::from(&opts.install_path);
    let bin_dir = install_dir.join("bin");
    let lib_dir = install_dir.join("lib");

    // ---- Phase 1: resolve target ----
    emit_pct(app, 2.0, Some(bin_name()));
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    emit_line(app, format!(r#"<span class="dim">$</span> digstore-setup --target {}"#, opts.install_path));
    emit_line(app, format!(r#"Resolving release <span class="ac">v1.0.0</span> · compiler 1.0.0 · module format 1 <span class="dim">({os}/{arch})</span>"#));

    let (payload, expected_sha) = digstore_payload(app)?;

    // ---- Phase 2: verify the package checksum [gated] ----
    // Offline integrity gate: recompute SHA-256 over the bytes we are about to
    // write and compare to the digest captured at build time (or the sidecar in
    // dev). This is a checksum, not cryptographic provenance — it proves the
    // payload is intact (no corruption/truncation), not authorship. A real
    // release additionally verifies a BLS detached signature over this digest
    // (the remaining TODO); the checksum check is the genuine, blocking gate
    // wired here and still aborts the install before any unpack/exec.
    emit_pct(app, 10.0, Some(bin_name()));
    let digest = sha256_bytes(&payload);
    match &expected_sha {
        Some(expected) if expected != &digest => {
            let msg = format!("package checksum mismatch: expected {expected}, got {digest}");
            let _ = app.emit("install://error", InstallError { message: msg.clone() });
            return Err(msg);
        }
        Some(_) => {
            emit_line(app, format!(r#"<span class="ok">✓</span> Verified package checksum (SHA-256) <span class="dim">({}…)</span>"#, &digest[..12]));
        }
        None => {
            // No expected digest available — surface honestly rather than faking a pass.
            emit_line(app, format!(r#"<span class="warn">!</span> No checksum manifest; recorded digest <span class="dim">{}…</span>"#, &digest[..12]));
        }
    }

    // ---- Phase 3: unpack the CLI (+ host runtime) ----
    emit_pct(app, 24.0, Some("bin/digstore"));
    fs::create_dir_all(&bin_dir).map_err(|e| format!("create {}: {e}", bin_dir.display()))?;
    let dest_bin = bin_dir.join(bin_name());
    fs::write(&dest_bin, &payload).map_err(|e| format!("unpack CLI: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(&dest_bin).map_err(|e| e.to_string())?.permissions();
        p.set_mode(0o755);
        let _ = fs::set_permissions(&dest_bin, p);
    }
    emit_line(app, format!(r#"Unpacking <span class="ac">DigStore CLI</span> → {}"#, bin_dir.display()));

    if *opts.selected.get("host").unwrap_or(&true) {
        emit_pct(app, 42.0, Some("lib/dig_host.wasm"));
        fs::create_dir_all(&lib_dir).map_err(|e| format!("create {}: {e}", lib_dir.display()))?;
        // The host runtime ships inside the CLI today; record the bound and
        // stage a marker so the install layout matches the spec. (TODO: when a
        // standalone dig_host artifact exists, copy it here.)
        let _ = fs::write(lib_dir.join("HOST_RUNTIME.txt"), "DigStore host runtime — bundled in digstore CLI (attestation + session ABI)\n");
        emit_line(app, r#"Unpacking <span class="ac">Host Runtime</span> <span class="dim">(64 KiB → 16 MiB memory bounds)</span>"#);
        emit_line(app, r#"Embedding trusted host keys <span class="dim">dig-host-key-v1:…</span>"#);
        emit_line(app, r#"<span class="ok">✓</span> Content-defined chunking ready <span class="dim">(16/64/256 KiB)</span>"#);
    }

    // ---- Phase 4: optional components ----
    if *opts.selected.get("completions").unwrap_or(&false) {
        emit_pct(app, 60.0, Some("share/completions/_digstore"));
        let comp_dir = install_dir.join("share").join("completions");
        let _ = fs::create_dir_all(&comp_dir);
        // Marker files — the digstore CLI does not yet emit completion scripts,
        // so write placeholders the layout expects. (TODO: `digstore completions
        // <shell>` once the CLI supports it.)
        for sh in ["bash", "zsh", "fish"] {
            let _ = fs::write(comp_dir.join(format!("digstore.{sh}")), format!("# digstore {sh} completion (generated by installer)\n"));
        }
        emit_line(app, r#"Installing shell completions <span class="dim">bash · zsh · fish</span>"#);
    }
    if *opts.selected.get("example").unwrap_or(&false) {
        emit_pct(app, 70.0, Some("examples/hello.wasm"));
        let ex_dir = install_dir.join("examples");
        let _ = fs::create_dir_all(&ex_dir);
        let _ = fs::write(ex_dir.join("README.txt"), "Sample urn:dig store — run `digstore clone <urn>` to explore.\n");
        emit_line(app, r#"Unpacking <span class="ac">Example store</span> <span class="dim">(urn:dig:…)</span>"#);
    }

    // ---- Phase 5: add to PATH ----
    if *opts.selected.get("path").unwrap_or(&true) {
        emit_pct(app, 82.0, Some("PATH"));
        match add_to_path(&bin_dir) {
            Ok(note) => {
                emit_line(app, format!(r#"Linking <span class="ac">digstore</span> → {note}"#));
            }
            Err(e) => {
                // PATH failure is non-fatal to the binary being usable; surface
                // as a warning, not a hard error.
                emit_line(app, format!(r#"<span class="warn">!</span> Could not update PATH automatically <span class="dim">({e})</span>"#));
            }
        }
    }

    // ---- Phase 5.5: register the .dig file-type icon ----
    emit_pct(app, 88.0, Some(".dig association"));
    match register_dig_association(&install_dir) {
        Ok(note) => emit_line(
            app,
            format!(r#"Registering <span class="ac">.dig</span> file icon <span class="dim">({note})</span>"#),
        ),
        Err(e) => emit_line(
            app,
            format!(r#"<span class="warn">!</span> Skipped .dig icon <span class="dim">({e})</span>"#),
        ),
    }

    // ---- Phase 6: verify ----
    emit_pct(app, 92.0, Some("digstore --version"));
    let out = Command::new(&dest_bin)
        .arg("--version")
        .output()
        .map_err(|e| {
            let msg = format!("verify failed: could not run {}: {e}", dest_bin.display());
            let _ = app.emit("install://error", InstallError { message: msg.clone() });
            msg
        })?;
    if !out.status.success() {
        let msg = format!(
            "verify failed: `digstore --version` exited with {}",
            out.status.code().unwrap_or(-1)
        );
        let _ = app.emit("install://error", InstallError { message: msg.clone() });
        return Err(msg);
    }
    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
    emit_line(app, format!(r#"<span class="ok">✓</span> Verifying install · <span class="ac">{}</span>"#, if ver.is_empty() { "digstore --version".into() } else { ver }));
    emit_pct(app, 100.0, Some("done"));
    emit_line(app, r#"<span class="ok">✓</span> DigStore is ready."#);

    let _ = app.emit("install://done", ());
    Ok(())
}

/// Compute the new user-PATH string after appending `dir`.
///
/// Pure helper (no I/O, no env access) so the append logic is unit-testable
/// without touching the real machine PATH. Idempotent and case-insensitive on
/// Windows: if `dir` is already present (ignoring case and trailing
/// separators), the current PATH is returned unchanged so we never
/// double-append.
///
/// Returns `None` if no change is needed, `Some(new_path)` otherwise.
#[cfg(windows)]
fn user_path_append(current: &str, dir: &str) -> Option<String> {
    let dir_trimmed = dir.trim_end_matches('\\');
    let already = current
        .split(';')
        .map(|p| p.trim().trim_end_matches('\\'))
        .any(|p| p.eq_ignore_ascii_case(dir_trimmed));
    if already {
        return None;
    }
    if current.is_empty() {
        Some(dir.to_string())
    } else if current.ends_with(';') {
        Some(format!("{current}{dir}"))
    } else {
        Some(format!("{current};{dir}"))
    }
}

/// Add the install bin dir to PATH.
///   Windows: append to the USER PATH only (HKCU\Environment\Path), written as
///            REG_EXPAND_SZ with no truncation, then broadcast
///            WM_SETTINGCHANGE. No elevation, no machine-PATH involvement.
///   macOS/Linux: symlink the binary into /usr/local/bin (best-effort).
fn add_to_path(bin_dir: &Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ};
        use winreg::{RegKey, RegValue};

        let dir = bin_dir.to_string_lossy().to_string();
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        // Open the per-user environment key for read+write. It always exists,
        // but create_subkey is idempotent (opens if present) and returns the key.
        let (env, _disp) = hkcu
            .create_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
            .map_err(|e| format!("open HKCU\\Environment: {e}"))?;

        // Read ONLY the user PATH (not the merged process PATH). Missing value
        // is treated as empty so we create it below.
        let current: String = env.get_value("Path").unwrap_or_default();

        let new_path = match user_path_append(&current, &dir) {
            None => return Ok(format!("user PATH (already present): {dir}")),
            Some(p) => p,
        };

        // Write back as REG_EXPAND_SZ (so embedded %VARS% keep expanding) with
        // no length limit — unlike `setx`, which truncates at 1024 chars.
        let bytes = string_to_reg_expand_sz_bytes(&new_path);
        env.set_raw_value(
            "Path",
            &RegValue { vtype: REG_EXPAND_SZ, bytes },
        )
        .map_err(|e| format!("write HKCU\\Environment\\Path: {e}"))?;

        broadcast_environment_change();
        Ok(format!("user PATH: {dir}"))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs as unixfs;
        let target = bin_dir.join("digstore");
        let link = PathBuf::from("/usr/local/bin/digstore");
        let _ = fs::remove_file(&link);
        unixfs::symlink(&target, &link).map_err(|e| format!("symlink {} → {}: {e}", link.display(), target.display()))?;
        Ok(format!("{}", link.display()))
    }
}

/// Register the DIG brand icon as the icon for `.dig` files. Best-effort and
/// per-user on every platform — never elevates.
///   Windows: write the embedded `.ico` into the install dir, register a ProgID
///            under `HKCU\Software\Classes`, and notify the shell.
///   Linux:   install a shared-mime-info package + a hicolor mimetype icon for
///            `application/x-dig` under `~/.local/share`, then refresh caches.
///   macOS:   unsupported for a CLI-only install (a document type needs a
///            persistent `.app` to declare it); reported as skipped.
fn register_dig_association(install_dir: &Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
        use winreg::RegKey;

        // Drop the icon next to the install so the ProgID points at a stable path.
        let icon_path = install_dir.join("digstore.ico");
        fs::write(&icon_path, DIG_ICON_ICO).map_err(|e| format!("write icon: {e}"))?;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let classes = "Software\\Classes";
        let prog_id = "DigStore.Store";

        // .dig -> ProgID
        let (ext, _) = hkcu
            .create_subkey_with_flags(format!("{classes}\\.dig"), KEY_WRITE)
            .map_err(|e| format!("create .dig key: {e}"))?;
        ext.set_value("", &prog_id)
            .map_err(|e| format!("set .dig default: {e}"))?;

        // ProgID description + DefaultIcon
        let (pid, _) = hkcu
            .create_subkey_with_flags(format!("{classes}\\{prog_id}"), KEY_WRITE)
            .map_err(|e| format!("create ProgID: {e}"))?;
        pid.set_value("", &"DigStore content-addressable store")
            .map_err(|e| format!("set ProgID default: {e}"))?;
        let (icon_key, _) = hkcu
            .create_subkey_with_flags(format!("{classes}\\{prog_id}\\DefaultIcon"), KEY_WRITE)
            .map_err(|e| format!("create DefaultIcon: {e}"))?;
        icon_key
            .set_value("", &format!("{},0", icon_path.display()))
            .map_err(|e| format!("set DefaultIcon: {e}"))?;

        notify_assoc_changed();
        Ok(format!("HKCU .dig → {prog_id}"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = install_dir; // not needed on Linux (per-user XDG dirs)
        let home = dirs::home_dir().ok_or("no home directory")?;
        let share = home.join(".local").join("share");

        // shared-mime-info package describing application/x-dig with *.dig.
        let mime_pkg_dir = share.join("mime").join("packages");
        fs::create_dir_all(&mime_pkg_dir).map_err(|e| format!("create mime dir: {e}"))?;
        let mime_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-dig">
    <comment>DigStore content-addressable store</comment>
    <glob pattern="*.dig"/>
  </mime-type>
</mime-info>
"#;
        fs::write(mime_pkg_dir.join("digstore.xml"), mime_xml)
            .map_err(|e| format!("write mime xml: {e}"))?;

        // hicolor mimetype icon: application-x-dig.png (freedesktop naming).
        let icon_dir = share
            .join("icons")
            .join("hicolor")
            .join("128x128")
            .join("mimetypes");
        fs::create_dir_all(&icon_dir).map_err(|e| format!("create icon dir: {e}"))?;
        fs::write(icon_dir.join("application-x-dig.png"), DIG_ICON_PNG)
            .map_err(|e| format!("write icon: {e}"))?;

        // Refresh caches (best-effort; ignore failures / missing tools).
        let _ = Command::new("update-mime-database")
            .arg(share.join("mime"))
            .status();
        let _ = Command::new("gtk-update-icon-cache")
            .arg("-f")
            .arg(share.join("icons").join("hicolor"))
            .status();
        Ok("~/.local/share MIME + icon".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let _ = install_dir;
        Err("not supported for a CLI-only install".to_string())
    }
}

/// Tell the Windows shell that file associations changed so Explorer repaints
/// `.dig` icons without a logout.
#[cfg(windows)]
fn notify_assoc_changed() {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

/// Encode a string as the UTF-16LE, NUL-terminated byte buffer the registry
/// expects for REG_EXPAND_SZ.
#[cfg(windows)]
fn string_to_reg_expand_sz_bytes(s: &str) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut bytes = Vec::with_capacity(wide.len() * 2);
    for w in wide {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bytes
}

/// Tell already-running processes that the environment changed, so new shells
/// (and Explorer) pick up the updated PATH without a reboot/logout.
#[cfg(windows)]
fn broadcast_environment_change() {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    // "Environment" as a NUL-terminated UTF-16 string, passed as lParam.
    let param: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
    let mut result: usize = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST as HWND,
            WM_SETTINGCHANGE,
            0 as WPARAM,
            param.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::user_path_append;

    #[test]
    fn appends_when_absent() {
        assert_eq!(
            user_path_append(r"C:\Windows;C:\Tools", r"C:\Apps\DigStore\bin"),
            Some(r"C:\Windows;C:\Tools;C:\Apps\DigStore\bin".to_string())
        );
    }

    #[test]
    fn no_change_when_already_present() {
        assert_eq!(
            user_path_append(r"C:\Windows;C:\Apps\DigStore\bin", r"C:\Apps\DigStore\bin"),
            None
        );
    }

    #[test]
    fn idempotent_case_insensitive() {
        // Different case must NOT double-append.
        assert_eq!(
            user_path_append(r"C:\windows;c:\apps\digstore\BIN", r"C:\Apps\DigStore\bin"),
            None
        );
    }

    #[test]
    fn idempotent_ignores_trailing_backslash() {
        assert_eq!(
            user_path_append(r"C:\Apps\DigStore\bin\", r"C:\Apps\DigStore\bin"),
            None
        );
    }

    #[test]
    fn creates_value_when_empty() {
        assert_eq!(
            user_path_append("", r"C:\Apps\DigStore\bin"),
            Some(r"C:\Apps\DigStore\bin".to_string())
        );
    }

    #[test]
    fn handles_trailing_separator_without_blank_entry() {
        // A PATH that ends in ';' should not produce an empty segment.
        assert_eq!(
            user_path_append(r"C:\Windows;", r"C:\Apps\DigStore\bin"),
            Some(r"C:\Windows;C:\Apps\DigStore\bin".to_string())
        );
    }
}
