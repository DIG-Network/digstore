//! dig-wallet — the DIG Browser's built-in Chia wallet sidecar.
//!
//! A local axum server (loopback only) that reuses `digstore-chain` (BIP-39
//! seed, standard Chia key derivation, the Sage-style HD wallet scan, DIG-CAT
//! support, AND encrypted seed storage) over coinset.org, and serves a
//! Sage-mirroring web UI the browser opens at 127.0.0.1. Native Rust, so BLS
//! (blst) signing works — unlike a WASM wallet.
//!
//! This build: create/restore a 24-word wallet protected by a password; the
//! seed is stored **encrypted at rest** (Argon2 + AES-GCM via
//! `digstore_chain::seed`) so the wallet persists across restarts and unlocks
//! with the password. Shows the live XCH + DIG balance scanned from coinset.org.
//!
//! Send/sign (`POST /api/send`): builds + BLS-signs a standard XCH payment via
//! `digstore_chain::send` (AugScheme, §11.3) drawing coins across the HD wallet.
//! Because a broadcast spends REAL mainnet funds, it is **gated twice**: the
//! request must set `broadcast: true` AND the process must run with
//! `DIG_WALLET_ALLOW_BROADCAST=1`. Otherwise the endpoint performs a **dry run** —
//! it returns the fully signed bundle (proof the signing path works) and pushes
//! NOTHING. The default is dry-run, so the flow can be exercised unattended safely.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use digstore_chain::coinset::{ChainReads, Coinset};
use digstore_chain::keys::{derive_wallet_keys, owner_address};
use digstore_chain::seed::{
    decrypt_seed, encrypt_seed, generate_mnemonic, validate_mnemonic, EncryptedSeed,
};
use digstore_chain::send::{build_xch_send, decode_xch_address};
use digstore_chain::wallet::scan_wallet;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

/// In-memory unlocked wallet for the session.
struct Session {
    mnemonic: Zeroizing<String>,
    address: String,
}

#[derive(Default)]
struct AppState {
    session: Mutex<Option<Session>>,
}

/// Path to the encrypted seed file (per-user, off the profile dir).
fn seed_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join("DigWallet").join("seed.bin")
}

fn wallet_exists() -> bool {
    seed_path().exists()
}

#[derive(Serialize)]
struct StatusResp {
    /// "none" (no wallet yet), "locked" (encrypted seed on disk, needs unlock),
    /// or "unlocked" (a session is active).
    state: &'static str,
    address: Option<String>,
}

#[derive(Serialize)]
struct GenerateResp {
    mnemonic: String,
}

#[derive(Deserialize)]
struct ImportReq {
    mnemonic: String,
    /// Password that encrypts the seed at rest (also required to unlock later).
    password: String,
}

#[derive(Deserialize)]
struct UnlockReq {
    password: String,
}

#[derive(Serialize)]
struct AddressResp {
    address: String,
}

#[derive(Serialize)]
struct BalanceResp {
    address: String,
    /// XCH balance in mojos (1 XCH = 1e12 mojos).
    xch_mojos: u64,
    /// DIG balance in base units (1 DIG = 1e3 base units).
    dig_units: u64,
}

#[derive(Serialize)]
struct ErrResp {
    error: String,
}

fn err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrResp>) {
    (code, Json(ErrResp { error: msg.into() }))
}

/// Derive the receive address + store an unlocked session for `mnemonic`.
async fn open_session(st: &AppState, mnemonic: Zeroizing<String>) -> Result<String, String> {
    let keys = derive_wallet_keys(&mnemonic).map_err(|e| e.to_string())?;
    let address = owner_address(&keys);
    *st.session.lock().await = Some(Session {
        mnemonic,
        address: address.clone(),
    });
    Ok(address)
}

async fn status(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let s = st.session.lock().await;
    if let Some(w) = s.as_ref() {
        return Json(StatusResp {
            state: "unlocked",
            address: Some(w.address.clone()),
        });
    }
    Json(StatusResp {
        state: if wallet_exists() { "locked" } else { "none" },
        address: None,
    })
}

/// Generate a fresh 24-word mnemonic (not stored until imported with a password).
async fn generate() -> Result<Json<GenerateResp>, (StatusCode, Json<ErrResp>)> {
    let m =
        generate_mnemonic(24).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(GenerateResp {
        mnemonic: m.to_string(),
    }))
}

/// Validate a mnemonic, encrypt + persist it under `password`, and unlock.
async fn import(
    State(st): State<Arc<AppState>>,
    Json(req): Json<ImportReq>,
) -> Result<Json<AddressResp>, (StatusCode, Json<ErrResp>)> {
    if req.password.len() < 8 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "password must be at least 8 characters",
        ));
    }
    let m = validate_mnemonic(&req.mnemonic).map_err(|e| {
        err(
            StatusCode::BAD_REQUEST,
            format!("invalid recovery phrase: {e}"),
        )
    })?;
    // Encrypt at rest (Argon2 + AES-GCM) and persist.
    let enc = encrypt_seed(&m, &req.password)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let path = seed_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create dir: {e}"),
            )
        })?;
    }
    std::fs::write(&path, enc.to_bytes()).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write seed: {e}"),
        )
    })?;
    let address = open_session(&st, Zeroizing::new(m.to_string()))
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(AddressResp { address }))
}

/// Decrypt the on-disk seed with `password` and unlock the session.
async fn unlock(
    State(st): State<Arc<AppState>>,
    Json(req): Json<UnlockReq>,
) -> Result<Json<AddressResp>, (StatusCode, Json<ErrResp>)> {
    let bytes = std::fs::read(seed_path())
        .map_err(|_| err(StatusCode::NOT_FOUND, "no wallet on this device"))?;
    let enc = EncryptedSeed::from_bytes(&bytes).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("corrupt seed file: {e}"),
        )
    })?;
    let m = decrypt_seed(&enc, &req.password)
        .map_err(|_| err(StatusCode::UNAUTHORIZED, "wrong password"))?;
    let address = open_session(&st, m)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(AddressResp { address }))
}

/// Live XCH + DIG balance, scanned from coinset.org (Sage-style whole-wallet scan).
async fn balance(
    State(st): State<Arc<AppState>>,
) -> Result<Json<BalanceResp>, (StatusCode, Json<ErrResp>)> {
    let (mnemonic, address) = {
        let s = st.session.lock().await;
        match s.as_ref() {
            Some(w) => (w.mnemonic.clone(), w.address.clone()),
            None => return Err(err(StatusCode::UNAUTHORIZED, "wallet is locked")),
        }
    };
    let chain = Coinset::mainnet();
    let scanned = scan_wallet(&chain, &mnemonic)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}")))?;
    Ok(Json(BalanceResp {
        address,
        xch_mojos: scanned.xch_balance(),
        dig_units: scanned.dig_balance(),
    }))
}

/// Lock the wallet (clear the session; the encrypted seed stays on disk).
async fn lock(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    *st.session.lock().await = None;
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct SendReq {
    /// Recipient mainnet address (`xch1…`).
    to: String,
    /// Amount to send, in mojos (1 XCH = 1e12 mojos).
    amount_mojos: u64,
    /// Network fee in mojos (0 = no fee).
    #[serde(default)]
    fee_mojos: u64,
    /// Request an actual on-chain broadcast. Ignored unless the process also runs
    /// with `DIG_WALLET_ALLOW_BROADCAST=1`; otherwise the send is a dry run.
    #[serde(default)]
    broadcast: bool,
}

#[derive(Serialize)]
struct SendResp {
    /// "signed" — built + signed but NOT broadcast (dry run; nothing was spent); or
    /// "broadcast" — the signed bundle was pushed to mainnet (real funds spent).
    status: &'static str,
    to: String,
    amount_mojos: u64,
    fee_mojos: u64,
    /// Change returned to the wallet, in mojos.
    change_mojos: u64,
    /// Total mojos of the selected input coins.
    inputs_mojos: u64,
    /// Number of input coin spends in the bundle.
    coin_spends: usize,
    /// Hex of the aggregated BLS signature over the spend — proof the bundle is
    /// fully signed and ready to broadcast.
    aggregated_signature: String,
}

/// What to do with a built+signed send bundle. Kept as a pure decision so the
/// safety gate (never broadcast unattended) is unit-tested independently of the
/// network path.
#[derive(Debug, PartialEq, Eq)]
enum SendAction {
    /// Built + signed, push nothing (default / `broadcast:false`).
    DryRun,
    /// Both the request and the env opted in — push to mainnet.
    Broadcast,
    /// `broadcast:true` requested but broadcasting is disabled — refuse (do not push).
    RefusedDisabled,
}

/// Broadcasting requires BOTH an explicit `broadcast:true` request AND the process
/// env opt-in (`DIG_WALLET_ALLOW_BROADCAST=1`). Anything else is a dry run; an
/// explicit request while disabled is refused (never silently downgraded).
fn send_action(req_broadcast: bool, env_enabled: bool) -> SendAction {
    match (req_broadcast, env_enabled) {
        (true, true) => SendAction::Broadcast,
        (true, false) => SendAction::RefusedDisabled,
        (false, _) => SendAction::DryRun,
    }
}

/// Build + BLS-sign a standard XCH payment. **Spends real mainnet funds only when
/// explicitly enabled.** Broadcasting requires BOTH `broadcast: true` in the request
/// AND `DIG_WALLET_ALLOW_BROADCAST=1` in the environment; otherwise the endpoint
/// returns the fully signed bundle as a dry run and pushes nothing. A `broadcast:
/// true` request while broadcasting is disabled is refused (403) — never silently
/// downgraded — so the caller knows the spend did not happen.
async fn send(
    State(st): State<Arc<AppState>>,
    Json(req): Json<SendReq>,
) -> Result<Json<SendResp>, (StatusCode, Json<ErrResp>)> {
    let recipient_ph =
        decode_xch_address(&req.to).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let mnemonic = {
        let s = st.session.lock().await;
        match s.as_ref() {
            Some(w) => w.mnemonic.clone(),
            None => return Err(err(StatusCode::UNAUTHORIZED, "wallet is locked")),
        }
    };
    let chain = Coinset::mainnet();
    let scanned = scan_wallet(&chain, &mnemonic)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}")))?;
    let (bundle, plan) = build_xch_send(&scanned, recipient_ph, req.amount_mojos, req.fee_mojos)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

    let broadcast_enabled =
        std::env::var("DIG_WALLET_ALLOW_BROADCAST").ok().as_deref() == Some("1");
    let status = match send_action(req.broadcast, broadcast_enabled) {
        SendAction::Broadcast => {
            chain
                .push(bundle.clone())
                .await
                .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("push_tx: {e}")))?;
            "broadcast"
        }
        SendAction::RefusedDisabled => {
            // Refuse rather than silently dry-run, so the caller is not misled into
            // thinking funds moved.
            return Err(err(
                StatusCode::FORBIDDEN,
                "broadcasting is disabled — set DIG_WALLET_ALLOW_BROADCAST=1 to spend real mainnet funds",
            ));
        }
        SendAction::DryRun => "signed", // signed, not pushed
    };

    Ok(Json(SendResp {
        status,
        to: req.to,
        amount_mojos: req.amount_mojos,
        fee_mojos: req.fee_mojos,
        change_mojos: plan.change,
        inputs_mojos: plan.inputs,
        coin_spends: bundle.coin_spends.len(),
        aggregated_signature: hex::encode(bundle.aggregated_signature.to_bytes()),
    }))
}

async fn index() -> Html<&'static str> {
    Html(UI_HTML)
}

/// Serve the DIG wallet (loopback only) to completion. Driven either by the
/// standalone `dig-wallet` binary OR in-process by `dig-runtime` on the browser's
/// tokio runtime (no sidecar). The wallet UI is an interactive web page, so it is
/// served over loopback HTTP (never reachable off-host); native BLS signing runs
/// in this same process.
pub async fn run() {
    let state = Arc::new(AppState::default());
    let app = Router::new()
        .route("/", get(index))
        .route("/api/status", get(status))
        .route("/api/generate", post(generate))
        .route("/api/import", post(import))
        .route("/api/unlock", post(unlock))
        .route("/api/balance", get(balance))
        .route("/api/send", post(send))
        .route("/api/lock", post(lock))
        .with_state(state);

    // Bind loopback only — the wallet must never be reachable off-host.
    let port: u16 = std::env::var("DIG_WALLET_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9777);
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("dig-wallet: cannot bind {addr}: {e}"));
    println!("dig-wallet listening on http://{addr}");
    axum::serve(listener, app).await.expect("dig-wallet server");
}

/// The Sage-mirroring wallet UI (single self-contained page). Dark, luxury,
/// DIG-purple / Chia-green accents.
const UI_HTML: &str = include_str!("ui.html");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_send_is_a_dry_run_never_broadcasts() {
        // No broadcast requested → never push, regardless of the env opt-in. This is
        // the safe default that lets the signing path run unattended.
        assert_eq!(send_action(false, false), SendAction::DryRun);
        assert_eq!(send_action(false, true), SendAction::DryRun);
    }

    #[test]
    fn broadcast_requires_both_request_and_env_optin() {
        // Explicit request but env disabled → refused (NOT a silent dry run, NOT a push).
        assert_eq!(send_action(true, false), SendAction::RefusedDisabled);
        // Both opted in → the only path that actually broadcasts.
        assert_eq!(send_action(true, true), SendAction::Broadcast);
    }
}
