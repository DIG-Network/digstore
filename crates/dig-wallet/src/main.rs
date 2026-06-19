//! dig-wallet — the DIG Browser's built-in Chia wallet sidecar.
//!
//! A local axum server that reuses `digstore-chain` (BIP-39 seed, standard Chia
//! key derivation, the Sage-style HD wallet scan, DIG-CAT support) over
//! coinset.org, and serves a Sage-mirroring web UI the browser opens at
//! 127.0.0.1. Native Rust, so BLS (blst) signing works — unlike a WASM wallet.
//!
//! v0 (this build): create/import a 24-word wallet, derive the standard receive
//! address, and show the live XCH + DIG balance scanned from coinset.org, plus a
//! receive view. The wallet lives in-memory for the session (encrypted on-disk
//! persistence + the send/sign flow are the next increments — send spends real
//! mainnet funds, so it is deliberately not exercised unattended).

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
use digstore_chain::seed::{generate_mnemonic, validate_mnemonic};
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

#[derive(Serialize)]
struct StatusResp {
    has_wallet: bool,
    address: Option<String>,
}

#[derive(Serialize)]
struct GenerateResp {
    mnemonic: String,
}

#[derive(Deserialize)]
struct ImportReq {
    mnemonic: String,
}

#[derive(Serialize)]
struct ImportResp {
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

async fn status(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let s = st.session.lock().await;
    Json(StatusResp {
        has_wallet: s.is_some(),
        address: s.as_ref().map(|w| w.address.clone()),
    })
}

/// Generate a fresh 24-word mnemonic. Not stored until the user imports it
/// (so the UI can show + confirm the seed words first).
async fn generate() -> Result<Json<GenerateResp>, (StatusCode, Json<ErrResp>)> {
    let m =
        generate_mnemonic(24).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(GenerateResp {
        mnemonic: m.to_string(),
    }))
}

/// Validate + unlock a wallet from a mnemonic (create or restore).
async fn import(
    State(st): State<Arc<AppState>>,
    Json(req): Json<ImportReq>,
) -> Result<Json<ImportResp>, (StatusCode, Json<ErrResp>)> {
    let m = validate_mnemonic(&req.mnemonic).map_err(|e| {
        err(
            StatusCode::BAD_REQUEST,
            format!("invalid recovery phrase: {e}"),
        )
    })?;
    let keys = derive_wallet_keys(&m).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let address = owner_address(&keys);
    let mut s = st.session.lock().await;
    *s = Some(Session {
        mnemonic: Zeroizing::new(m.to_string()),
        address: address.clone(),
    });
    Ok(Json(ImportResp { address }))
}

/// Live XCH + DIG balance, scanned from coinset.org (Sage-style whole-wallet scan).
async fn balance(
    State(st): State<Arc<AppState>>,
) -> Result<Json<BalanceResp>, (StatusCode, Json<ErrResp>)> {
    let (mnemonic, address) = {
        let s = st.session.lock().await;
        match s.as_ref() {
            Some(w) => (w.mnemonic.clone(), w.address.clone()),
            None => return Err(err(StatusCode::UNAUTHORIZED, "no wallet unlocked")),
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
