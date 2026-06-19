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
//! (The send/sign flow is the next increment — it spends real mainnet funds, so
//! it is deliberately not exercised unattended.)

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use digstore_chain::coinset::Coinset;
use digstore_chain::keys::{derive_wallet_keys, owner_address};
use digstore_chain::seed::{
    decrypt_seed, encrypt_seed, generate_mnemonic, validate_mnemonic, EncryptedSeed,
};
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

async fn index() -> Html<&'static str> {
    Html(UI_HTML)
}

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState::default());
    let app = Router::new()
        .route("/", get(index))
        .route("/api/status", get(status))
        .route("/api/generate", post(generate))
        .route("/api/import", post(import))
        .route("/api/unlock", post(unlock))
        .route("/api/balance", get(balance))
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
