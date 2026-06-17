//! dighub account integration: RFC-8628 device-pairing login, session storage,
//! and the pre-command login gate.
//!
//! This is the CLI's "dighub account" — a staging-only scoped token (`aud=dighub-cli`,
//! NO on-chain authority) proving the operator has a dighub account. It is **never**
//! used to sign or anchor anything; the push owner-auth (store key + identity key,
//! §21.9) is entirely separate and unchanged. The gate is a product requirement, not a
//! crypto change.
//!
//! The session lives next to the identity key (the `%APPDATA%/dig` or `~/.dig` dir),
//! in `session.json`, written owner-only where the OS allows.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::CliError;
use crate::ops::store_ops::write_secret_file;

/// Default dighub API base. Override with the `DIG_API_BASE` env var (trailing
/// slash stripped). Put on the dighub edge (`hub.dig.net`), distinct from the
/// content remote (`rpc.dig.net`).
pub const DEFAULT_API_BASE: &str = "https://hub.dig.net/v1";

/// The same WAF-safe User-Agent the §21 remote client sends. The hub/rpc edge WAF
/// 403s requests with no User-Agent before they reach the backend.
const USER_AGENT: &str = concat!("digstore/", env!("CARGO_PKG_VERSION"));

/// Resolve the dighub API base: `DIG_API_BASE` if set (trailing slash stripped),
/// else [`DEFAULT_API_BASE`].
pub fn api_base() -> String {
    match std::env::var("DIG_API_BASE") {
        Ok(v) if !v.trim().is_empty() => v.trim().trim_end_matches('/').to_string(),
        _ => DEFAULT_API_BASE.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Session storage
// ---------------------------------------------------------------------------

/// A persisted dighub login session. `account_ph` is optional (the backend does
/// not return one); `handle` is the user's chosen handle, or `None` if unset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub account_ph: Option<String>,
    pub api_base: String,
    pub obtained_at: u64,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// The dig config directory (mirrors `identity.rs`): `DIG_IDENTITY_DIR` override,
/// else `<config_dir>/dig`.
fn identity_dir() -> Result<PathBuf, CliError> {
    if let Some(d) = std::env::var_os("DIG_IDENTITY_DIR") {
        return Ok(PathBuf::from(d));
    }
    let base = dirs::config_dir().ok_or_else(|| {
        CliError::Other(anyhow::anyhow!(
            "no OS config directory available for the dig session"
        ))
    })?;
    Ok(base.join("dig"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// The `*_in(dir)` variants take the session directory explicitly so the storage
// logic is unit-testable without the process-global `DIG_IDENTITY_DIR` env var
// (which other test modules also mutate concurrently). The public functions just
// resolve the dir and delegate.

fn load_session_in(dir: &std::path::Path) -> Option<Session> {
    let bytes = std::fs::read(dir.join("session.json")).ok()?;
    serde_json::from_slice::<Session>(&bytes).ok()
}

fn save_session_in(dir: &std::path::Path, s: &Session) -> Result<(), CliError> {
    std::fs::create_dir_all(dir).map_err(|e| CliError::Other(e.into()))?;
    let json = serde_json::to_vec_pretty(s).map_err(|e| CliError::Other(e.into()))?;
    // The token is a credential; write it owner-only (0600 on unix) like the
    // identity key.
    write_secret_file(&dir.join("session.json"), &json).map_err(|e| CliError::Other(e.into()))?;
    Ok(())
}

fn clear_session_in(dir: &std::path::Path) -> Result<(), CliError> {
    match std::fs::remove_file(dir.join("session.json")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CliError::Other(e.into())),
    }
}

/// Load the persisted session, if any. Returns `None` when absent or unparseable.
pub fn load_session() -> Option<Session> {
    load_session_in(&identity_dir().ok()?)
}

/// Persist the session to `session.json` (owner-only where the OS allows).
pub fn save_session(s: &Session) -> Result<(), CliError> {
    save_session_in(&identity_dir()?, s)
}

/// Remove the persisted session (idempotent — fine if none exists).
pub fn clear_session() -> Result<(), CliError> {
    clear_session_in(&identity_dir()?)
}

/// The current logged-in handle, if a session exists and has a handle set.
pub fn current_handle() -> Option<String> {
    load_session().and_then(|s| s.handle)
}

impl Session {
    /// Whether the session's token has expired (best-effort; only if `expires_in`
    /// is known). A session with no `expires_in` is treated as non-expiring here.
    pub fn is_expired(&self) -> bool {
        match self.expires_in {
            Some(ttl) => now_unix() >= self.obtained_at.saturating_add(ttl),
            None => false,
        }
    }

    /// A valid (present + unexpired) session.
    pub fn is_valid(&self) -> bool {
        !self.access_token.is_empty() && !self.is_expired()
    }
}

/// A currently-valid persisted session, if any.
pub fn valid_session() -> Option<Session> {
    load_session().filter(|s| s.is_valid())
}

// ---------------------------------------------------------------------------
// Device-pairing protocol (RFC-8628 style)
// ---------------------------------------------------------------------------

/// Response from `POST /v1/auth/cli/pair`.
#[derive(Debug, Clone, Deserialize)]
pub struct PairResponse {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

/// The outcome of a single poll of `POST /v1/auth/cli/poll`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// Authorization still pending — keep polling at the current interval.
    Pending,
    /// Approved: the scoped token (and optional handle).
    Approved {
        access_token: String,
        handle: Option<String>,
    },
    /// Server asked us to back off: increase the interval and keep polling.
    SlowDown,
    /// Terminal failure (expired / denied / forbidden / not-found / unknown).
    Failed(String),
}

/// Classify a poll response (HTTP status + parsed JSON body) into a [`PollOutcome`].
/// Pure + table-driven so it is unit-testable without a live network.
pub fn classify_poll(status: u16, body: &serde_json::Value) -> PollOutcome {
    // Success: a token (200).
    if status == 200 {
        if let Some(tok) = body.get("access_token").and_then(|v| v.as_str()) {
            let handle = body
                .get("handle")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            return PollOutcome::Approved {
                access_token: tok.to_string(),
                handle,
            };
        }
        // 200 with no token is anomalous; treat as still-pending to be safe.
        return PollOutcome::Pending;
    }

    let code = body
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Pending (typically 202).
    if status == 202 || code == "authorization_pending" {
        return PollOutcome::Pending;
    }

    // Transient back-off.
    if code == "slow_down" {
        return PollOutcome::SlowDown;
    }

    // Terminal classes.
    let expired = code.contains("expired") || code == "challenge_expired";
    let denied = code == "forbidden"
        || code.contains("denied")
        || code.contains("locked")
        || code.contains("already used");
    let not_found = code == "not_found" || status == 404;

    if expired {
        PollOutcome::Failed("the device code expired — run `digstore login` again".into())
    } else if denied {
        PollOutcome::Failed("pairing was denied or the device code is no longer valid".into())
    } else if not_found {
        PollOutcome::Failed("device code not found — run `digstore login` again".into())
    } else if !code.is_empty() {
        PollOutcome::Failed(format!("login failed: {code}"))
    } else {
        PollOutcome::Failed(format!("login failed (HTTP {status})"))
    }
}

fn http_client() -> Result<reqwest::Client, CliError> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| CliError::Network(format!("http client: {e}")))
}

/// POST `/v1/auth/cli/pair` (body `{}`, no auth).
pub async fn pair(base: &str) -> Result<PairResponse, CliError> {
    let client = http_client()?;
    let url = format!("{}/auth/cli/pair", base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| CliError::Network(format!("pair request: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(CliError::Network(format!(
            "pair failed (HTTP {}): {}",
            status.as_u16(),
            body.trim()
        )));
    }
    resp.json::<PairResponse>()
        .await
        .map_err(|e| CliError::Network(format!("pair response: {e}")))
}

/// POST `/v1/auth/cli/poll` once and classify the outcome.
pub async fn poll_once(base: &str, device_code: &str) -> Result<PollOutcome, CliError> {
    let client = http_client()?;
    let url = format!("{}/auth/cli/poll", base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "device_code": device_code }))
        .send()
        .await
        .map_err(|e| CliError::Network(format!("poll request: {e}")))?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    Ok(classify_poll(status, &body))
}

// ---------------------------------------------------------------------------
// Login flow (pair + poll loop) — shared by `login` and the gate
// ---------------------------------------------------------------------------

/// Run the full interactive pairing flow: print the user code + URI, then poll
/// with a spinner respecting `interval` + `slow_down` backoff until approved or
/// `expires_in` elapses. On success, saves the session and returns it.
pub fn login_interactive(ui: &crate::ui::Ui) -> Result<Session, CliError> {
    let base = api_base();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    let pairing = rt.block_on(pair(&base))?;

    // Present the code (clean, color-coded).
    ui.line("");
    ui.line(format!(
        "To authorize this device, visit:  {}",
        bold(ui, &pairing.verification_uri)
    ));
    ui.line(format!(
        "and enter the code:  {}",
        bold(ui, &pairing.user_code)
    ));
    ui.line("");

    let session = rt.block_on(poll_until_approved(ui, &base, &pairing))?;
    save_session(&session)?;
    Ok(session)
}

/// Poll the device code until approval / expiry, showing a spinner.
async fn poll_until_approved(
    ui: &crate::ui::Ui,
    base: &str,
    pairing: &PairResponse,
) -> Result<Session, CliError> {
    let spinner = ui.spinner("Waiting for approval…");
    let result = poll_loop(base, pairing).await;
    spinner.finish_clear();
    result
}

/// Poll the device code until approval / expiry with NO spinner (for `--json` /
/// non-interactive). Honors `expires_in` so a non-TTY never hangs forever.
pub async fn poll_until_approved_quiet(
    base: &str,
    pairing: &PairResponse,
) -> Result<Session, CliError> {
    poll_loop(base, pairing).await
}

/// The shared poll loop: sleep `interval`, poll, handle pending/slow_down/approved/
/// failed, until `expires_in` elapses.
async fn poll_loop(base: &str, pairing: &PairResponse) -> Result<Session, CliError> {
    let deadline = Instant::now() + Duration::from_secs(pairing.expires_in.max(1));
    let mut interval = pairing.interval.max(1);

    loop {
        if Instant::now() >= deadline {
            return Err(CliError::Network(
                "login timed out waiting for approval — run `digstore login` again".into(),
            ));
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
        match poll_once(base, &pairing.device_code).await? {
            PollOutcome::Pending => continue,
            PollOutcome::SlowDown => {
                interval = interval.saturating_add(5);
                continue;
            }
            PollOutcome::Approved {
                access_token,
                handle,
            } => {
                return Ok(Session {
                    access_token,
                    handle,
                    account_ph: None,
                    api_base: base.trim_end_matches('/').to_string(),
                    obtained_at: now_unix(),
                    expires_in: None,
                });
            }
            PollOutcome::Failed(msg) => return Err(CliError::Unauthorized(msg)),
        }
    }
}

fn bold(ui: &crate::ui::Ui, s: &str) -> String {
    crate::ui::theme::paint(
        ui.color(),
        anstyle::Style::new()
            .fg_color(Some(anstyle::AnsiColor::Cyan.into()))
            .bold(),
        s,
    )
}

// ---------------------------------------------------------------------------
// Pre-command login gate
// ---------------------------------------------------------------------------

/// Ensure a valid dighub session exists before running a network/account command
/// (push, pull, clone, revoke). This is a PRODUCT gate (an account requirement),
/// NOT a change to the push/pull/§21.9 owner-auth crypto.
///
/// * Valid session present → return it.
/// * Interactive TTY → prompt to log in; if accepted, run the login flow inline
///   and continue; if declined, abort with a hint.
/// * Non-interactive (`--json`/`--yes`/non-TTY) → error out (never prompt, never
///   hang).
pub fn ensure_logged_in(ui: &crate::ui::Ui) -> Result<Session, CliError> {
    if let Some(s) = valid_session() {
        return Ok(s);
    }
    // Either no session or an expired one — re-auth is required.
    if ui.can_prompt() {
        if ui.confirm("Not logged in. Log in now?", true) {
            return login_interactive(ui);
        }
        return Err(CliError::Unauthorized(
            "not logged in; run `digstore login`".into(),
        ));
    }
    Err(CliError::Unauthorized(
        "not logged in; run `digstore login`".into(),
    ))
}

// ---------------------------------------------------------------------------
// Origin URL handling
// ---------------------------------------------------------------------------

/// The default origin URL for `digstore remote add origin` with no URL, given a
/// session handle: `https://<handle>@rpc.dig.net`.
pub fn default_origin_url(handle: &str) -> String {
    format!("https://{handle}@rpc.dig.net")
}

/// Inject a session handle as `userinfo@` into a remote URL when it has none.
/// Cosmetic — returns the URL unchanged if it already has userinfo, has no
/// recognizable `scheme://host`, or anything is off. `https://rpc.dig.net` ->
/// `https://<handle>@rpc.dig.net`.
pub fn inject_handle(url: &str, handle: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        // Already has userinfo (`user@host…`) before the first `/`?
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        if authority.contains('@') {
            return url.to_string();
        }
        if authority.is_empty() {
            return url.to_string();
        }
        return format!("{scheme}://{handle}@{rest}");
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `DIG_IDENTITY_DIR` / `DIG_API_BASE` are process-global; serialize tests that
    // mutate them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn sample_session() -> Session {
        Session {
            access_token: "tok-abc".into(),
            handle: Some("alice".into()),
            account_ph: None,
            api_base: DEFAULT_API_BASE.into(),
            obtained_at: now_unix(),
            expires_in: None,
        }
    }

    #[test]
    fn session_round_trip_save_load_clear() {
        // Uses the explicit-dir helpers so it is free of the process-global
        // `DIG_IDENTITY_DIR` env var (which other test modules also mutate in
        // parallel) — no lock needed, no cross-test interference.
        let td = tempfile::tempdir().unwrap();
        let dir = td.path();

        assert!(load_session_in(dir).is_none());
        let s = sample_session();
        save_session_in(dir, &s).unwrap();

        let loaded = load_session_in(dir).expect("session present after save");
        assert_eq!(loaded.access_token, "tok-abc");
        assert_eq!(loaded.handle.as_deref(), Some("alice"));
        assert_eq!(loaded.handle.clone(), s.handle);
        assert!(loaded.is_valid());

        clear_session_in(dir).unwrap();
        assert!(load_session_in(dir).is_none());
        // Idempotent: clearing again is fine.
        clear_session_in(dir).unwrap();
    }

    #[test]
    fn api_base_env_override() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("DIG_API_BASE");
        assert_eq!(api_base(), DEFAULT_API_BASE);

        std::env::set_var("DIG_API_BASE", "https://staging.example.com/v1/");
        assert_eq!(api_base(), "https://staging.example.com/v1");

        // Empty/whitespace falls back to the default.
        std::env::set_var("DIG_API_BASE", "   ");
        assert_eq!(api_base(), DEFAULT_API_BASE);

        std::env::remove_var("DIG_API_BASE");
    }

    #[test]
    fn expired_session_is_not_valid() {
        let mut s = sample_session();
        s.obtained_at = 0;
        s.expires_in = Some(1);
        assert!(s.is_expired());
        assert!(!s.is_valid());
    }

    #[test]
    fn classify_poll_pending() {
        let v = serde_json::json!({ "error": "authorization_pending" });
        assert_eq!(classify_poll(202, &v), PollOutcome::Pending);
        // 200 with no token is also treated as pending (anomalous).
        assert_eq!(
            classify_poll(200, &serde_json::json!({})),
            PollOutcome::Pending
        );
    }

    #[test]
    fn classify_poll_approved() {
        let v = serde_json::json!({ "access_token": "jwt-xyz", "handle": "bob" });
        match classify_poll(200, &v) {
            PollOutcome::Approved {
                access_token,
                handle,
            } => {
                assert_eq!(access_token, "jwt-xyz");
                assert_eq!(handle.as_deref(), Some("bob"));
            }
            other => panic!("expected approved, got {other:?}"),
        }
        // Null handle is fine.
        let v = serde_json::json!({ "access_token": "jwt", "handle": null });
        match classify_poll(200, &v) {
            PollOutcome::Approved { handle, .. } => assert!(handle.is_none()),
            other => panic!("expected approved, got {other:?}"),
        }
    }

    #[test]
    fn classify_poll_slow_down_is_transient() {
        let v = serde_json::json!({ "error": "slow_down" });
        assert_eq!(classify_poll(429, &v), PollOutcome::SlowDown);
    }

    #[test]
    fn classify_poll_terminal_errors() {
        for (status, code) in [
            (400, "challenge_expired"),
            (400, "device code expired"),
            (403, "forbidden"),
            (403, "pairing denied"),
            (403, "device code locked"),
            (403, "device code already used"),
            (404, "not_found"),
        ] {
            let v = serde_json::json!({ "error": code });
            assert!(
                matches!(classify_poll(status, &v), PollOutcome::Failed(_)),
                "expected terminal failure for {code}"
            );
        }
    }

    #[test]
    fn default_origin_url_uses_handle() {
        assert_eq!(default_origin_url("alice"), "https://alice@rpc.dig.net");
    }

    #[test]
    fn inject_handle_when_no_userinfo() {
        assert_eq!(
            inject_handle("https://rpc.dig.net", "alice"),
            "https://alice@rpc.dig.net"
        );
        assert_eq!(
            inject_handle("https://rpc.dig.net/path", "bob"),
            "https://bob@rpc.dig.net/path"
        );
    }

    #[test]
    fn inject_handle_leaves_existing_userinfo() {
        assert_eq!(
            inject_handle("https://carol@rpc.dig.net", "alice"),
            "https://carol@rpc.dig.net"
        );
    }

    #[test]
    fn inject_handle_passthrough_when_unparseable() {
        assert_eq!(inject_handle("rpc.dig.net", "alice"), "rpc.dig.net");
    }
}
