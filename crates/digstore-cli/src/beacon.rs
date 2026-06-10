//! Throttled, best-effort startup/exit beacon: prints a one-line notice to
//! stderr when a newer DigStore release exists.
//!
//! Contract (all enforced here):
//!   * NEVER blocks or slows a command meaningfully — short 2s network timeout,
//!     and only when the 24h throttle window has elapsed.
//!   * NEVER fails a command — every error path is silent; this function returns
//!     `()` and is called for its side effect only.
//!   * Stays out of the way — disabled for non-TTY runs, `--json`/`--quiet`
//!     output, and when `DIGSTORE_NO_UPDATE_CHECK=1` is set.
//!
//! It reuses the release-resolution / version-compare / throttle logic in
//! [`crate::commands::update`] so the beacon and the explicit command agree on
//! what "newer" means and share one on-disk cache.

use std::io::{IsTerminal, Write};

use crate::commands::update::{
    current_version, fetch_latest_release_quiet, is_newer, load_cache, now_unix, save_cache,
    should_check, CheckCache,
};

/// Environment variable that disables the beacon entirely.
const DISABLE_ENV: &str = "DIGSTORE_NO_UPDATE_CHECK";

/// Decide, from the runtime gates, whether the beacon is allowed to run at all.
/// Pure for testability — callers pass in the observed environment/TTY state.
///
/// * `json`/`quiet` → off (machine output or explicitly silenced).
/// * not a TTY → off (scripts, pipes, CI).
/// * `DIGSTORE_NO_UPDATE_CHECK=1` → off.
pub fn beacon_enabled(json: bool, quiet: bool, is_tty: bool, disable_env: Option<&str>) -> bool {
    if json || quiet || !is_tty {
        return false;
    }
    !matches!(disable_env, Some("1"))
}

/// Run the beacon for its side effect. Cheap and silent on every failure.
///
/// `json`/`quiet` come from the resolved CLI flags. Everything else (TTY, env,
/// network, cache) is read here. Returns immediately without touching the
/// network when the throttle window has not elapsed.
pub fn maybe_notify(json: bool, quiet: bool) {
    let is_tty = std::io::stderr().is_terminal();
    let disable = std::env::var(DISABLE_ENV).ok();
    if !beacon_enabled(json, quiet, is_tty, disable.as_deref()) {
        return;
    }

    let cache = load_cache();
    let now = now_unix();

    // Within the 24h window: reuse the cached result instead of hitting the
    // network, so repeated interactive commands stay instant.
    if !should_check(now, cache.last_check_unix) {
        if is_newer(current_version(), &cache.latest_tag) {
            print_notice(&cache.latest_tag);
        }
        return;
    }

    // Throttle window elapsed: do a short, best-effort network check on a
    // throwaway runtime. Any failure is swallowed (we still bump the timestamp
    // so a persistent outage doesn't retry on every command).
    let release = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
        .and_then(|rt| rt.block_on(fetch_latest_release_quiet()));

    let latest_tag = release.map(|r| r.tag_name).unwrap_or_default();
    save_cache(&CheckCache {
        last_check_unix: now,
        latest_tag: latest_tag.clone(),
    });

    if !latest_tag.is_empty() && is_newer(current_version(), &latest_tag) {
        print_notice(&latest_tag);
    }
}

/// Print the one-line stderr notice. Best-effort; ignores write errors.
fn print_notice(latest_tag: &str) {
    let tag = if latest_tag.starts_with('v') {
        latest_tag.to_string()
    } else {
        format!("v{latest_tag}")
    };
    let mut err = std::io::stderr();
    let _ = writeln!(
        err,
        "a newer DigStore ({tag}) is available — run `digstore update`"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_for_json_quiet_and_non_tty() {
        // Baseline: TTY, no env override, human output -> enabled.
        assert!(beacon_enabled(false, false, true, None));
        // json output -> off.
        assert!(!beacon_enabled(true, false, true, None));
        // quiet -> off.
        assert!(!beacon_enabled(false, true, true, None));
        // not a TTY -> off.
        assert!(!beacon_enabled(false, false, false, None));
    }

    #[test]
    fn disabled_by_env_var() {
        assert!(!beacon_enabled(false, false, true, Some("1")));
        // Any other value does not disable it.
        assert!(beacon_enabled(false, false, true, Some("0")));
        assert!(beacon_enabled(false, false, true, Some("")));
    }
}
