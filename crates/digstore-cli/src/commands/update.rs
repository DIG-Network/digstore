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

/// Pick the best installer asset for the current platform from a release's
/// asset list.
///
/// On Windows we prefer an NSIS `*-setup.exe` and fall back to an `.msi`. On
/// other platforms there is no bundled installer yet, so this returns `None`
/// and the caller prints manual-download instructions.
#[cfg(target_os = "windows")]
pub fn select_installer_asset(assets: &[Asset]) -> Option<&Asset> {
    select_windows_installer(assets)
}

#[cfg(not(target_os = "windows"))]
pub fn select_installer_asset(_assets: &[Asset]) -> Option<&Asset> {
    None
}

/// Windows installer selection, factored out so it is unit-testable on any host.
pub fn select_windows_installer(assets: &[Asset]) -> Option<&Asset> {
    // Prefer the setup installer (it can update an existing install in place).
    // The release asset is named `DigStore-Setup-<ver>-windows-x64.exe`, so we
    // match any `.exe` whose name carries the "setup" marker rather than a fixed
    // `-setup.exe` suffix (the version/arch tail comes after "Setup").
    let nsis = assets.iter().find(|a| {
        let n = a.name.to_ascii_lowercase();
        n.ends_with(".exe") && n.contains("setup")
    });
    if nsis.is_some() {
        return nsis;
    }
    // Fall back to any `.msi`.
    assets
        .iter()
        .find(|a| a.name.to_ascii_lowercase().ends_with(".msi"))
}

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

/// Carry out the platform-specific update once we know a newer release exists.
fn perform_update(
    _ctx: &CliContext,
    ui: &crate::ui::Ui,
    release: &Release,
    current: &str,
    latest: &str,
    yes: bool,
) -> Result<(), CliError> {
    #[cfg(target_os = "windows")]
    {
        let asset = select_installer_asset(&release.assets).ok_or_else(|| {
            CliError::NotFound(format!(
                "no Windows installer (*-setup.exe / .msi) in release {latest}"
            ))
        })?;

        ui.line(format!(
            "update available: {} -> {}",
            display_version(current),
            latest
        ));
        ui.line(format!("installer: {}", asset.name));

        if !yes && !confirm("Download and run the installer now?") {
            ui.line("aborted; run `digstore update --yes` to skip this prompt");
            return Ok(());
        }

        let dest = download_asset(asset, ui)?;
        ui.verb("Launching", asset.name.clone());
        launch_installer(&dest)?;
        ui.success("installer launched; it will update your DigStore install");
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = yes;
        // No bundled installer on macOS/Linux yet: point the user at the release.
        ui.line(format!(
            "update available: {} -> {}",
            display_version(current),
            latest
        ));
        if !release.html_url.is_empty() {
            ui.line(format!("release: {}", release.html_url));
        }
        match suggest_manual_asset(&release.assets) {
            Some(a) => ui.line(format!("download: {}", a.browser_download_url)),
            None => ui.line("download the asset for your platform from the release page"),
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
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

/// Download `asset` into a temp directory and return the on-disk path.
#[cfg(target_os = "windows")]
fn download_asset(asset: &Asset, ui: &crate::ui::Ui) -> Result<std::path::PathBuf, CliError> {
    ui.verb("Downloading", asset.name.clone());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    let bytes = rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(UPDATE_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| CliError::Network(format!("http client: {e}")))?;
        let resp = client
            .get(&asset.browser_download_url)
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
            .map_err(|e| CliError::Network(format!("read body: {e}")))
    })?;

    let dir = std::env::temp_dir().join("digstore-update");
    std::fs::create_dir_all(&dir).map_err(|e| CliError::Other(e.into()))?;
    let dest = dir.join(&asset.name);
    std::fs::write(&dest, &bytes).map_err(|e| CliError::Other(e.into()))?;
    Ok(dest)
}

/// Launch the downloaded installer and return immediately (the installer takes
/// over the actual update). `.msi` files are run via `msiexec /i`.
#[cfg(target_os = "windows")]
fn launch_installer(path: &std::path::Path) -> Result<(), CliError> {
    let is_msi = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("msi"))
        .unwrap_or(false);
    let result = if is_msi {
        std::process::Command::new("msiexec")
            .arg("/i")
            .arg(path)
            .spawn()
    } else {
        std::process::Command::new(path).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| CliError::Other(anyhow::anyhow!("launch installer: {e}")))
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
    fn windows_installer_prefers_setup_exe_over_msi() {
        let assets = vec![
            asset("digstore-0.4.0-x86_64.msi"),
            asset("digstore-0.4.0-x86_64-setup.exe"),
            asset("digstore-0.4.0-linux.tar.gz"),
        ];
        let picked = select_windows_installer(&assets).unwrap();
        assert_eq!(picked.name, "digstore-0.4.0-x86_64-setup.exe");
    }

    #[test]
    fn windows_installer_matches_release_asset_name() {
        // The actual release asset (Setup mid-name, version/arch tail after).
        let assets = vec![
            asset("DigStore-Setup-0.4.4-linux-x86_64.AppImage"),
            asset("DigStore-Setup-0.4.4-macos.dmg"),
            asset("DigStore-Setup-0.4.4-windows-x64.exe"),
        ];
        let picked = select_windows_installer(&assets).unwrap();
        assert_eq!(picked.name, "DigStore-Setup-0.4.4-windows-x64.exe");
    }

    #[test]
    fn windows_installer_falls_back_to_msi() {
        let assets = vec![
            asset("digstore-0.4.0-x86_64.msi"),
            asset("digstore-0.4.0-linux.tar.gz"),
        ];
        let picked = select_windows_installer(&assets).unwrap();
        assert_eq!(picked.name, "digstore-0.4.0-x86_64.msi");
    }

    #[test]
    fn windows_installer_none_when_absent() {
        let assets = vec![
            asset("digstore-0.4.0-linux.tar.gz"),
            asset("digstore-0.4.0-darwin.tar.gz"),
        ];
        assert!(select_windows_installer(&assets).is_none());
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

    /// Live-network smoke test, gated so CI/unit runs never hit GitHub.
    #[test]
    #[ignore]
    fn live_fetch_latest_release() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let rel = rt.block_on(fetch_latest_release(UPDATE_TIMEOUT)).unwrap();
        assert!(parse_version(&rel.tag_name).is_some());
    }
}
