//! `digstore update` — self-update against the latest GitHub release, plus the
//! shared release-resolution / version-compare / asset-selection logic reused by
//! the throttled startup beacon (see [`crate::beacon`]).
//!
//! Design goals: BEST-EFFORT and FAIL-SAFE. The command itself reports errors
//! (it is what the user asked for), but the underlying helpers are written so the
//! beacon can call them and silently swallow any failure — a broken network or a
//! GitHub outage must never break, slow, or fail a normal `digstore` command.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cli::UpdateArgs;
use crate::context::CliContext;
use crate::error::CliError;

/// Upstream repository whose releases drive `digstore update`.
pub const RELEASES_API: &str = "https://api.github.com/repos/DIG-Network/digstore/releases/latest";

/// GitHub requires a non-empty User-Agent on every API request.
pub const USER_AGENT: &str = concat!("digstore-cli/", env!("CARGO_PKG_VERSION"));

/// Short timeout for the network calls. The beacon must never block a command
/// meaningfully; the explicit `update` command can afford a longer budget.
const BEACON_TIMEOUT: Duration = Duration::from_secs(2);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// GitHub Releases API (only the fields we consume).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    /// The git tag, e.g. `v0.4.0`.
    pub tag_name: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

// ---------------------------------------------------------------------------
// Version comparison.
// ---------------------------------------------------------------------------

/// Parse a semantic-ish version string into a comparable `(major, minor, patch)`
/// triple, tolerating a leading `v` and pre-release/build suffixes (which are
/// ignored for the comparison — a conservative "is the release line newer?"
/// check is all the update beacon needs).
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    let s = s
        .strip_prefix('v')
        .or_else(|| s.strip_prefix('V'))
        .unwrap_or(s);
    // Drop any pre-release (`-rc1`) or build (`+meta`) suffix.
    let core = s.split(['-', '+']).next().unwrap_or(s);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// True if `latest` is strictly newer than `current`. Returns `false` whenever
/// either side fails to parse (fail-safe: never claim an update on garbage).
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// The version of this running binary.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ---------------------------------------------------------------------------
// Asset selection.
// ---------------------------------------------------------------------------

/// A human-friendly hint for the asset a non-Windows user should download.
/// Picks an asset whose name mentions the OS/arch when possible.
pub fn suggest_manual_asset(assets: &[Asset]) -> Option<&Asset> {
    let os = std::env::consts::OS; // "linux", "macos", …
    let alt = if os == "macos" { "darwin" } else { os };
    assets.iter().find(|a| {
        let n = a.name.to_ascii_lowercase();
        n.contains(os) || n.contains(alt)
    })
}

// ---------------------------------------------------------------------------
// Platform binary-asset selection (self-install path, every OS including
// Windows — #303: there is no separate GUI-installer asset/path anymore).
// ---------------------------------------------------------------------------

/// The release asset that carries the `digstore` binary for a platform, plus whether
/// it is a compressed archive that must be extracted (vs a raw executable usable as-is).
#[derive(Debug, Clone, PartialEq)]
pub struct PlatformAsset {
    /// The chosen asset's file name.
    pub name: String,
    /// The download URL, used verbatim (never reconstructed — the reported "doubled
    /// URL" was terminal wrapping; digstore passes GitHub's `browser_download_url`
    /// through unchanged).
    pub url: String,
    /// True when the asset is a `.tar.gz`/`.tgz`/`.zip`/`.tar` needing extraction.
    pub is_tarball: bool,
}

/// OS-name tokens that appear in release asset names for a `std::env::consts::OS`.
fn os_tokens(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &["macos", "darwin"],
        "linux" => &["linux"],
        "windows" => &["windows"],
        _ => &[],
    }
}

/// Arch tokens for a `std::env::consts::ARCH` (release names use short + triple forms).
fn arch_tokens(arch: &str) -> &'static [&'static str] {
    match arch {
        "x86_64" => &["x86_64", "x64", "amd64"],
        "aarch64" => &["aarch64", "arm64"],
        _ => &[],
    }
}

/// True if `name` is a compressed archive (needs extraction before use).
fn is_archive_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".tar.gz") || n.ends_with(".tgz") || n.ends_with(".zip") || n.ends_with(".tar")
}

/// True if `name` is a GUI installer / disk image (the installer path, NOT the raw
/// self-replace path): NSIS `*setup*.exe`, `.msi`, `.dmg`, `.AppImage`.
fn is_installer_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".msi")
        || n.ends_with(".dmg")
        || n.ends_with(".appimage")
        || (n.ends_with(".exe") && n.contains("setup"))
}

/// Select the release asset carrying the `digstore` binary for `os`/`arch` (from
/// `std::env::consts`). Matches an asset whose name mentions BOTH an OS token and an
/// arch token, excluding GUI installers/disk-images. Prefers a raw executable; falls
/// back to a tarball when that is the only match (e.g. Linux aarch64 ships only a
/// `.tar.gz`). Returns `None` when nothing matches (caller fails loud with manual steps).
pub fn select_binary_asset(assets: &[Asset], os: &str, arch: &str) -> Option<PlatformAsset> {
    let oss = os_tokens(os);
    let ars = arch_tokens(arch);
    let matches: Vec<&Asset> = assets
        .iter()
        .filter(|a| {
            let n = a.name.to_ascii_lowercase();
            !is_installer_name(&n)
                && oss.iter().any(|t| n.contains(t))
                && ars.iter().any(|t| n.contains(t))
        })
        .collect();
    // Prefer a raw (non-archive) binary; otherwise take the (tarball) match.
    let chosen = matches
        .iter()
        .find(|a| !is_archive_name(&a.name))
        .or_else(|| matches.first())?;
    Some(PlatformAsset {
        name: chosen.name.clone(),
        url: chosen.browser_download_url.clone(),
        is_tarball: is_archive_name(&chosen.name),
    })
}

/// Sanity-check that `bytes` looks like a native executable for `os` (no checksums are
/// published, so this guards against downloading an HTML error page / truncated file).
/// Checks the leading magic: ELF (Linux), Mach-O / fat (macOS), `MZ` (Windows).
pub fn looks_like_native_binary(bytes: &[u8], os: &str) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    match os {
        "linux" => &bytes[..4] == b"\x7fELF",
        "macos" => {
            // Mach-O thin (BE/LE, 32/64) + fat/universal (BE/LE), any order of the 4 bytes.
            const MACHO: [[u8; 4]; 6] = [
                [0xFE, 0xED, 0xFA, 0xCE], // MH_MAGIC (32, BE)
                [0xCE, 0xFA, 0xED, 0xFE], // MH_CIGAM (32, LE)
                [0xFE, 0xED, 0xFA, 0xCF], // MH_MAGIC_64 (BE)
                [0xCF, 0xFA, 0xED, 0xFE], // MH_CIGAM_64 (LE)
                [0xCA, 0xFE, 0xBA, 0xBE], // FAT_MAGIC (BE)
                [0xBE, 0xBA, 0xFE, 0xCA], // FAT_CIGAM (LE)
            ];
            MACHO.iter().any(|m| &bytes[..4] == m)
        }
        "windows" => &bytes[..2] == b"MZ",
        _ => true,
    }
}

/// Extract the `digstore` executable from a gzip-compressed tarball's raw bytes.
/// Returns the binary's bytes, or an error if the archive holds no `digstore` entry.
pub fn extract_digstore_from_targz(bytes: &[u8]) -> Result<Vec<u8>, CliError> {
    use std::io::Read;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let entries = archive
        .entries()
        .map_err(|e| CliError::UpdateFailed(format!("open release archive: {e}")))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|e| CliError::UpdateFailed(format!("read release archive: {e}")))?;
        let is_digstore = entry
            .path()
            .ok()
            .and_then(|p| p.file_name().and_then(|s| s.to_str()).map(str::to_owned))
            .map(|n| n == "digstore" || n == "digstore.exe")
            .unwrap_or(false);
        if is_digstore {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| CliError::UpdateFailed(format!("extract digstore: {e}")))?;
            return Ok(buf);
        }
    }
    Err(CliError::UpdateFailed(
        "the release archive contains no `digstore` executable".to_string(),
    ))
}

// ---------------------------------------------------------------------------
// Network: fetch the latest release.
// ---------------------------------------------------------------------------

/// Fetch + parse the latest release with the given timeout. Used by both the
/// command (long timeout) and the beacon (short timeout). Any failure surfaces
/// as a `CliError::Network` so the command can report it; the beacon discards it.
pub async fn fetch_latest_release(timeout: Duration) -> Result<Release, CliError> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| CliError::Network(format!("http client: {e}")))?;
    let resp = client
        .get(RELEASES_API)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| CliError::Network(format!("github releases: {e}")))?;
    if !resp.status().is_success() {
        return Err(CliError::Network(format!(
            "github releases returned status {}",
            resp.status().as_u16()
        )));
    }
    resp.json::<Release>()
        .await
        .map_err(|e| CliError::Network(format!("decode release json: {e}")))
}

/// Best-effort variant for the beacon: short timeout, never errors.
pub async fn fetch_latest_release_quiet() -> Option<Release> {
    fetch_latest_release(BEACON_TIMEOUT).await.ok()
}

// ---------------------------------------------------------------------------
// Throttle cache (shared with the beacon).
// ---------------------------------------------------------------------------

/// On-disk record of the last update check, used to throttle the beacon to at
/// most once per [`CHECK_INTERVAL_SECS`].
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckCache {
    /// Unix seconds of the last successful (or attempted) check.
    pub last_check_unix: u64,
    /// The latest tag observed at that check (empty if unknown).
    #[serde(default)]
    pub latest_tag: String,
}

/// How often the beacon is allowed to hit the network: once per 24h.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Path of the throttle cache: `<config_dir>/digstore/update-check.json`.
pub fn cache_path() -> Option<std::path::PathBuf> {
    Some(
        dirs::config_dir()?
            .join("digstore")
            .join("update-check.json"),
    )
}

/// Decide whether enough time has elapsed since `last_check_unix` to check again.
/// Pure for testability.
pub fn should_check(now_unix: u64, last_check_unix: u64) -> bool {
    now_unix.saturating_sub(last_check_unix) >= CHECK_INTERVAL_SECS
}

/// Current wall-clock time in unix seconds (0 on the impossible clock-error).
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Load the cache, returning the default (never-checked) record on any error.
pub fn load_cache() -> CheckCache {
    let Some(p) = cache_path() else {
        return CheckCache::default();
    };
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the cache best-effort (creating the parent dir); errors are ignored.
pub fn save_cache(cache: &CheckCache) {
    let Some(p) = cache_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(&p, json);
    }
}

// ---------------------------------------------------------------------------
// Command.
// ---------------------------------------------------------------------------

/// `digstore update [--check] [--yes]`.
pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: UpdateArgs) -> Result<(), CliError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    let release = rt.block_on(fetch_latest_release(UPDATE_TIMEOUT))?;
    let current = current_version();
    let latest = release.tag_name.clone();

    // Record the result so the beacon doesn't re-check right after a manual run.
    save_cache(&CheckCache {
        last_check_unix: now_unix(),
        latest_tag: latest.clone(),
    });

    if !is_newer(current, &latest) {
        if ui.json() {
            ui.emit_json(&serde_json::json!({
                "update_available": false,
                "current": current,
                "latest": latest,
            }));
        } else {
            ui.success(format!("already up to date ({})", display_version(current)));
        }
        return Ok(());
    }

    // An update is available.
    if args.check {
        if ui.json() {
            ui.emit_json(&serde_json::json!({
                "update_available": true,
                "current": current,
                "latest": latest,
                "release_url": release.html_url,
            }));
        } else {
            ui.line(format!(
                "update available: {} -> {}",
                display_version(current),
                latest
            ));
            ui.hint("digstore update");
        }
        return Ok(());
    }

    perform_update(ctx, ui, &release, current, &latest, ui.assume_yes())
}

/// Render a version the way users expect (`vX.Y.Z`).
fn display_version(v: &str) -> String {
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

/// Carry out the update once we know a newer release exists.
///
/// One unified path on EVERY platform (Windows included, #303): `digstore`
/// updates ITS OWN binary in place — resolve the raw per-OS/arch `digstore`
/// binary asset via [`select_binary_asset`], download it, sanity-check it,
/// then atomically self-replace via [`install_binary`]. There is no GUI
/// installer step here; the bundled installer moved to dig-installer.
fn perform_update(
    _ctx: &CliContext,
    ui: &crate::ui::Ui,
    release: &Release,
    current: &str,
    latest: &str,
    yes: bool,
) -> Result<(), CliError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    ui.line(format!(
        "update available: {} -> {}",
        display_version(current),
        latest
    ));

    // Pick the correct raw-binary (or tarball) asset for this OS+arch.
    let Some(sel) = select_binary_asset(&release.assets, os, arch) else {
        if !release.html_url.is_empty() {
            ui.line(format!("release: {}", release.html_url));
        }
        return Err(CliError::NotFound(format!(
            "no digstore binary asset for {os}/{arch} in release {latest}; \
             download it from the release page above and place it on your PATH"
        )));
    };
    ui.line(format!("asset: {}", sel.name));

    if !yes && !confirm("Download it and replace the current binary now?") {
        ui.line("aborted; run `digstore update --yes` to skip this prompt");
        return Ok(());
    }

    // Download -> (extract if tarball) -> sanity-check -> atomic self-replace.
    ui.verb("Downloading", sel.name.clone());
    let payload = fetch_bytes(&sel.url)?;
    let binary = if sel.is_tarball {
        extract_digstore_from_targz(&payload)?
    } else {
        payload
    };
    if !looks_like_native_binary(&binary, os) {
        return Err(CliError::UpdateFailed(
            "the downloaded asset is not a valid digstore binary (aborting; nothing changed)"
                .to_string(),
        ));
    }

    let target = resolve_self_path()?;
    ui.verb("Installing", target.display().to_string());
    install_binary(&target, &binary)?;

    ui.success(format!("updated to {} ({})", latest, target.display()));
    // Best-effort post-verify: run the freshly installed binary's --version.
    if let Ok(out) = std::process::Command::new(&target)
        .arg("--version")
        .output()
    {
        let v = String::from_utf8_lossy(&out.stdout);
        let v = v.trim();
        if !v.is_empty() {
            ui.line(format!("now: {v}"));
        }
    }
    Ok(())
}

fn confirm(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

// ---------------------------------------------------------------------------
// Self-install (every platform): download -> verify -> (chmod/de-quarantine on
// Unix) -> atomic rename over the running binary.
// ---------------------------------------------------------------------------

/// Download an asset URL into memory. Used verbatim from `browser_download_url`.
/// Shared by every platform's self-replace path (#303: Windows has no separate
/// GUI-installer download anymore — it self-replaces its own binary too).
fn fetch_bytes(url: &str) -> Result<Vec<u8>, CliError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(UPDATE_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| CliError::Network(format!("http client: {e}")))?;
        let resp = client
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| CliError::Network(format!("download: {e}")))?;
        if !resp.status().is_success() {
            return Err(CliError::Network(format!(
                "download returned status {}",
                resp.status().as_u16()
            )));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| CliError::Network(format!("read body: {e}")))
    })
}

/// The on-disk path of the binary to replace. Resolves `current_exe()` through symlinks
/// (via `canonicalize`) so we overwrite the REAL file — e.g. a Homebrew symlink at
/// `/opt/homebrew/bin/digstore` keeps pointing at the (now-updated) Cellar target.
pub fn resolve_self_path() -> Result<std::path::PathBuf, CliError> {
    let exe = std::env::current_exe()
        .map_err(|e| CliError::UpdateFailed(format!("locate the running binary: {e}")))?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// Atomically replace the executable at `target` with `new_bytes`.
///
/// Writes to a temp file in the SAME directory (so the final move is atomic on one
/// filesystem), sets the exec bit (Unix), clears the macOS Gatekeeper quarantine
/// best-effort, then moves it into place via [`rename_into_place`]. Returns a clear
/// [`CliError::UpdateFailed`] with manual steps if the destination is not writable.
pub fn install_binary(target: &std::path::Path, new_bytes: &[u8]) -> Result<(), CliError> {
    let dir = target.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp = dir.join(format!(".digstore-update-{}.tmp", std::process::id()));

    std::fs::write(&tmp, new_bytes).map_err(|e| not_writable(target, &e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o755);
        if let Err(e) = std::fs::set_permissions(&tmp, perm) {
            let _ = std::fs::remove_file(&tmp);
            return Err(CliError::UpdateFailed(format!("set exec bit: {e}")));
        }
    }

    // Downloaded-then-written bytes carry no quarantine attr, but clear it defensively
    // (harmless if absent) so a self-updated binary is never Gatekeeper-blocked.
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("xattr")
        .args(["-d", "com.apple.quarantine"])
        .arg(&tmp)
        .output();

    rename_into_place(&tmp, target)?;

    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("xattr")
        .args(["-d", "com.apple.quarantine"])
        .arg(target)
        .output();

    Ok(())
}

/// Move `tmp` into `target`'s place, replacing any existing file there (`target`
/// is typically the CURRENTLY RUNNING `digstore` process's own binary).
///
/// Unix: a plain rename over `target` is atomic and safe even while it is
/// executing — the live process keeps its old inode open until it exits.
///
/// Windows (#303): the destination is the image file backing this very
/// process. Windows' loader opens a running image with share-delete
/// permission, so RENAMING it aside succeeds even while it executes, but a
/// direct one-step overwrite does not (verified: it fails with "Access is
/// denied", os error 5). So: rename the current binary aside, rename the new
/// one into place, then best-effort delete the old one — a failure there is
/// harmless, it just leaves a stray `.old` file for a future update to
/// overwrite/ignore (it is never the active `target` path).
fn rename_into_place(tmp: &std::path::Path, target: &std::path::Path) -> Result<(), CliError> {
    #[cfg(target_os = "windows")]
    {
        if target.exists() {
            let dir = target.parent().unwrap_or_else(|| std::path::Path::new("."));
            let aside = dir.join(format!(".digstore-old-{}.tmp", std::process::id()));
            if let Err(e) = std::fs::rename(target, &aside) {
                let _ = std::fs::remove_file(tmp);
                return Err(not_writable(target, &e));
            }
            if let Err(e) = std::fs::rename(tmp, target) {
                // Restore the original binary so a failed update never leaves
                // the user with no `digstore` at all.
                let _ = std::fs::rename(&aside, target);
                return Err(not_writable(target, &e));
            }
            let _ = std::fs::remove_file(&aside);
            return Ok(());
        }
    }
    if let Err(e) = std::fs::rename(tmp, target) {
        let _ = std::fs::remove_file(tmp);
        return Err(not_writable(target, &e));
    }
    Ok(())
}

/// A fail-LOUD error for a non-writable install location, with the manual recovery
/// steps (§ install docs) so the user is never left with a silent no-op.
fn not_writable(target: &std::path::Path, e: &std::io::Error) -> CliError {
    CliError::UpdateFailed(format!(
        "cannot write {} ({e}).\n\
         Manual update:\n  \
         1. Download the digstore binary for your OS/arch from \
         https://github.com/DIG-Network/digstore/releases/latest\n  \
         2. chmod +x ./digstore\n  \
         3. (macOS) xattr -d com.apple.quarantine ./digstore\n  \
         4. mv ./digstore \"{}\"   # or re-run with write permission / the installer",
        target.display(),
        target.display(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example/{name}"),
        }
    }

    #[test]
    fn parse_version_tolerates_v_prefix_and_suffixes() {
        assert_eq!(parse_version("v0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("v1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("v2.0"), Some((2, 0, 0)));
        assert_eq!(parse_version("1"), Some((1, 0, 0)));
        assert_eq!(parse_version("not-a-version"), None);
    }

    #[test]
    fn is_newer_detects_newer_older_equal() {
        // newer
        assert!(is_newer("0.3.0", "0.4.0"));
        assert!(is_newer("v0.3.0", "v0.3.1"));
        assert!(is_newer("1.2.3", "2.0.0"));
        // older
        assert!(!is_newer("0.4.0", "0.3.0"));
        assert!(!is_newer("2.0.0", "1.9.9"));
        // equal
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        // unparsable -> never claims an update
        assert!(!is_newer("0.3.0", "garbage"));
        assert!(!is_newer("garbage", "0.4.0"));
    }

    #[test]
    fn should_check_respects_24h_interval() {
        let day = CHECK_INTERVAL_SECS;
        // Just checked -> do not re-check.
        assert!(!should_check(1000, 1000));
        assert!(!should_check(1000 + day - 1, 1000));
        // 24h elapsed -> check.
        assert!(should_check(1000 + day, 1000));
        assert!(should_check(1000 + day + 1, 1000));
        // Never checked (0) -> check.
        assert!(should_check(day, 0));
    }

    #[test]
    fn cache_round_trips_through_json() {
        let c = CheckCache {
            last_check_unix: 12345,
            latest_tag: "v0.4.0".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: CheckCache = serde_json::from_str(&json).unwrap();
        assert_eq!(back.last_check_unix, 12345);
        assert_eq!(back.latest_tag, "v0.4.0");
    }

    #[test]
    fn display_version_adds_v_prefix() {
        assert_eq!(display_version("0.3.0"), "v0.3.0");
        assert_eq!(display_version("v0.3.0"), "v0.3.0");
    }

    // --- #127: platform binary-asset selection --------------------------------

    /// The ACTUAL release asset set (from `gh release view v0.7.2`), so the selector
    /// is pinned to the real names. Raw binaries for macOS + linux-x64; a `.tar.gz`
    /// for linux aarch64 (the only asset for that target).
    fn real_release_assets() -> Vec<Asset> {
        [
            "digstore-0.7.2-aarch64-unknown-linux-gnu.tar.gz",
            "digstore-0.7.2-linux-x64",
            "digstore-0.7.2-macos-arm64",
            "digstore-0.7.2-macos-x64",
            "digstore-0.7.2-windows-x64.exe",
            "digstore-0.7.2-x86_64-unknown-linux-gnu.tar.gz",
        ]
        .iter()
        .map(|n| asset(n))
        .collect()
    }

    #[test]
    fn selects_macos_arm64_raw_binary() {
        let a = select_binary_asset(&real_release_assets(), "macos", "aarch64").unwrap();
        assert_eq!(a.name, "digstore-0.7.2-macos-arm64");
        assert!(!a.is_tarball);
    }

    #[test]
    fn selects_macos_x64_raw_binary() {
        let a = select_binary_asset(&real_release_assets(), "macos", "x86_64").unwrap();
        assert_eq!(a.name, "digstore-0.7.2-macos-x64");
        assert!(!a.is_tarball);
    }

    #[test]
    fn selects_linux_x64_raw_binary_over_tarball() {
        let a = select_binary_asset(&real_release_assets(), "linux", "x86_64").unwrap();
        assert_eq!(a.name, "digstore-0.7.2-linux-x64");
        assert!(!a.is_tarball, "prefer the raw binary over the .tar.gz");
    }

    #[test]
    fn selects_linux_aarch64_tarball_when_only_option() {
        let a = select_binary_asset(&real_release_assets(), "linux", "aarch64").unwrap();
        assert_eq!(a.name, "digstore-0.7.2-aarch64-unknown-linux-gnu.tar.gz");
        assert!(a.is_tarball);
    }

    #[test]
    fn selects_windows_x64_raw_binary() {
        // #303: the Windows self-update path must resolve the raw
        // `digstore-<ver>-windows-x64.exe` binary — never a `*-setup.exe`/`.msi`
        // GUI installer (that moved to dig-installer; digstore's own releases no
        // longer publish one, so the old installer-only path always found
        // nothing and `digstore update` returned NotFound on Windows).
        let a = select_binary_asset(&real_release_assets(), "windows", "x86_64").unwrap();
        assert_eq!(a.name, "digstore-0.7.2-windows-x64.exe");
        assert!(!a.is_tarball);
    }

    #[test]
    fn selector_uses_browser_download_url_verbatim() {
        // Guards the reported "doubled URL": digstore passes GitHub's URL through
        // unchanged (the doubling was terminal line-wrapping, not construction).
        let url = "https://github.com/DIG-Network/digstore/releases/download/v0.7.2/digstore-0.7.2-macos-arm64";
        let assets = vec![Asset {
            name: "digstore-0.7.2-macos-arm64".into(),
            browser_download_url: url.into(),
        }];
        let a = select_binary_asset(&assets, "macos", "aarch64").unwrap();
        assert_eq!(a.url, url);
    }

    #[test]
    fn selector_skips_installers_and_disk_images() {
        // A .dmg / setup.exe is the installer path, never a raw self-replace asset.
        let assets = vec![
            asset("DigStore-Setup-0.7.2-macos.dmg"),
            asset("DigStore-Setup-0.7.2-windows-x64.exe"),
        ];
        assert!(select_binary_asset(&assets, "macos", "aarch64").is_none());
    }

    // --- #127: downloaded-binary sanity check ---------------------------------

    #[test]
    fn native_binary_magic_recognized_per_os() {
        assert!(looks_like_native_binary(b"\x7fELF....", "linux"));
        assert!(!looks_like_native_binary(b"<!DOCTYPE html>", "linux"));
        assert!(looks_like_native_binary(
            &[0xCF, 0xFA, 0xED, 0xFE, 0, 0],
            "macos"
        )); // Mach-O 64 LE
        assert!(looks_like_native_binary(
            &[0xCA, 0xFE, 0xBA, 0xBE, 0, 0],
            "macos"
        )); // fat/universal
        assert!(!looks_like_native_binary(b"nope-not-macho", "macos"));
        assert!(looks_like_native_binary(b"MZ\x90\x00", "windows"));
        assert!(!looks_like_native_binary(b"", "linux"));
    }

    // --- #127: tarball extraction ---------------------------------------------

    fn targz_with(entry_name: &str, data: &[u8]) -> Vec<u8> {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tb = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tb.append_data(&mut header, entry_name, data).unwrap();
        tb.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn extracts_digstore_from_a_targz() {
        let payload = b"\x7fELF this-is-the-digstore-binary";
        let gz = targz_with("digstore", payload);
        assert_eq!(extract_digstore_from_targz(&gz).unwrap(), payload);
    }

    #[test]
    fn extract_errors_when_no_digstore_entry() {
        let gz = targz_with("README.txt", b"not the binary");
        assert!(extract_digstore_from_targz(&gz).is_err());
    }

    // --- #127: atomic self-replace (against a TEMP file, NEVER the live binary) --

    #[test]
    fn install_binary_replaces_contents_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("digstore");
        std::fs::write(&target, b"OLD-BINARY").unwrap();

        install_binary(&target, b"\x7fELF NEW-BINARY").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"\x7fELF NEW-BINARY");
        // No temp file left behind (it was renamed over the target).
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("digstore-update-"))
            .count();
        assert_eq!(leftovers, 0, "temp file must be renamed away");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "exec bits must be set");
        }
    }

    #[test]
    fn install_binary_fails_loud_on_unwritable_target() {
        // A target under a non-existent directory can't be written -> clear error + steps.
        let bad = std::path::Path::new("digstore-update-nope")
            .join("deeper")
            .join("digstore");
        let err = install_binary(&bad, b"\x7fELFx").unwrap_err();
        assert!(matches!(err, CliError::UpdateFailed(_)));
        assert!(
            err.to_string().contains("Manual update"),
            "gives manual steps"
        );
    }

    /// #303 regression: `digstore update` self-replaces the binary of the
    /// CURRENTLY RUNNING `digstore` process. On Windows, a plain one-step
    /// `rename(tmp, target)` fails while `target` is the image of a running
    /// process (the OS denies the overwrite even though the running image was
    /// opened with share-delete). Prove `install_binary` handles this by
    /// literally spawning a copy of a system executable and replacing it
    /// while the child process is executing it — the exact scenario `digstore
    /// update` faces in the field.
    #[test]
    #[cfg(target_os = "windows")]
    fn install_binary_replaces_a_currently_executing_file_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("locked-target.exe");
        std::fs::copy(r"C:\Windows\System32\ping.exe", &target)
            .expect("copy a real system exe to act as the locked target");

        // Keep the copied exe running for a few seconds so Windows has its file
        // open/mapped for execution, exactly like the running `digstore` process.
        let mut child = std::process::Command::new(&target)
            .args(["-n", "5", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn the locked target");

        // Give it a moment to actually start executing (file becomes locked).
        std::thread::sleep(std::time::Duration::from_millis(300));

        let new_bytes = b"\x7fELF fake-updated-binary-contents".to_vec();
        let result = install_binary(&target, &new_bytes);

        let _ = child.kill();
        let _ = child.wait();
        // Let Windows release its handle on the (renamed-aside) old file before
        // the tempdir is cleaned up.
        std::thread::sleep(std::time::Duration::from_millis(200));

        result.expect("install_binary must replace a currently-executing exe on Windows");
        assert_eq!(std::fs::read(&target).unwrap(), new_bytes);
    }

    /// Live-network smoke test, gated so CI/unit runs never hit GitHub.
    #[test]
    #[ignore]
    fn live_fetch_latest_release() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let rel = rt.block_on(fetch_latest_release(UPDATE_TIMEOUT)).unwrap();
        assert!(parse_version(&rel.tag_name).is_some());
    }
}
