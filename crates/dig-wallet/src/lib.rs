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

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
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
    approvals: Mutex<Approvals>,
}

/// Per-origin dapp connection state. `approved` is the user's allow-list (which
/// web origins may use the wallet), persisted to disk so it survives restarts.
/// `pending` holds origins that called `connect` and are awaiting the user's
/// approval in the wallet UI (in-memory — a pending request doesn't outlive the
/// session).
struct Approvals {
    approved: BTreeSet<String>,
    pending: BTreeSet<String>,
}

impl Default for Approvals {
    fn default() -> Self {
        Approvals {
            approved: load_approved(),
            pending: BTreeSet::new(),
        }
    }
}

impl Approvals {
    /// The wallet's own loopback origin is implicitly trusted (the wallet UI
    /// itself), so it never needs a connect handshake.
    fn is_approved(&self, origin: &str) -> bool {
        is_self_origin(origin) || self.approved.contains(origin)
    }
}

/// The loopback port the wallet serves on (default 9777; `DIG_WALLET_PORT`).
fn wallet_port() -> u16 {
    std::env::var("DIG_WALLET_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9777)
}

/// True only for the wallet's OWN page origin (exact host + port) — the UI is
/// trusted. Deliberately NOT all of 127.0.0.1: another local app on a different
/// port must still go through the approval gate, or any localhost process could
/// spend the wallet unprompted.
fn is_self_origin(origin: &str) -> bool {
    let port = wallet_port();
    origin == format!("http://127.0.0.1:{port}") || origin == format!("http://localhost:{port}")
}

/// Path to the encrypted seed file (per-user, off the profile dir).
fn seed_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join("DigWallet").join("seed.bin")
}

/// Path to the persisted dapp allow-list (next to the seed file).
fn connections_path() -> PathBuf {
    seed_path()
        .parent()
        .map(|p| p.join("connections.json"))
        .unwrap_or_else(|| PathBuf::from("connections.json"))
}

/// Load the approved-origins allow-list from disk (empty if absent/corrupt).
fn load_approved() -> BTreeSet<String> {
    std::fs::read(connections_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

/// Persist the approved-origins allow-list.
fn save_approved(approved: &BTreeSet<String>) {
    let path = connections_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(&approved.iter().collect::<Vec<_>>()) {
        let _ = std::fs::write(path, json);
    }
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

/// The bundled WalletConnect responder (esbuild IIFE exposing `window.DigWC`).
/// Served as a static asset the wallet page loads with `<script src>`. Checked in
/// (regenerated via `wc/build.mjs`) so the crate builds offline — no npm at build
/// time. Loopback only, same as the rest of the wallet.
async fn wc_bundle_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        WC_BUNDLE_JS,
    )
}

/// The DIG protocol settings page (loopback). The browser opens it at
/// `dig://settings`, which the dig:// loader redirects here.
async fn settings_page() -> Html<&'static str> {
    Html(SETTINGS_HTML)
}

/// Current local-cache configuration: the LRU capacity ceiling and the bytes
/// currently on disk. Both come from `dig-node`, the single source of truth for
/// the native cache (so the CLI, the loader, and this UI agree).
#[derive(serde::Serialize)]
struct DigConfig {
    cache_cap_bytes: u64,
    cache_used_bytes: u64,
}

async fn dig_config_get() -> Json<DigConfig> {
    Json(DigConfig {
        cache_cap_bytes: dig_node::cache_cap_bytes(),
        cache_used_bytes: dig_node::cache_used_bytes(),
    })
}

#[derive(serde::Deserialize)]
struct SetDigConfig {
    cache_cap_bytes: u64,
}

/// Clamp a requested cache cap to a sane minimum. A fat-fingered "0" must not
/// disable caching entirely (that would defeat local-first and hammer
/// rpc.dig.net), so the cap floors at 64 MiB.
fn floored_cache_cap(requested: u64) -> u64 {
    const MIN_CAP: u64 = 64 * 1024 * 1024;
    requested.max(MIN_CAP)
}

async fn dig_config_set(Json(req): Json<SetDigConfig>) -> impl IntoResponse {
    let cap = floored_cache_cap(req.cache_cap_bytes);
    match dig_node::set_cache_cap_bytes(cap) {
        Ok(()) => (
            StatusCode::OK,
            Json(DigConfig {
                cache_cap_bytes: cap,
                cache_used_bytes: dig_node::cache_used_bytes(),
            }),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Purge the entire local DIG cache. Content stays available — it just falls
/// back to rpc.dig.net on next visit and re-warms the cache.
async fn dig_cache_clear() -> impl IntoResponse {
    dig_node::clear_cache();
    StatusCode::NO_CONTENT
}

// ---- DIG settings: WalletConnect projectId, public key, key export ----------
//
// These endpoints back the DIG settings page (`dig://settings`). Two of them
// touch secrets and so are restricted to the wallet's OWN loopback origin
// (`is_self_origin`): the master mnemonic export and the projectId setter. They
// are deliberately NOT routed through `/api/wc/request`, so no dapp / injected
// `window.chia` / WC session can reach them — see `wc_dispatch`, whose method
// set has no export/projectId/key-material path.

/// The effective WalletConnect projectId surfaced to the settings page.
#[derive(Serialize)]
struct WcProjectIdResp {
    /// The effective projectId (persisted config > `DIG_WALLET_WC_PROJECT_ID`),
    /// or `null` when none is configured.
    project_id: Option<String>,
    /// `true` iff a projectId is configured (relay can pair); drives the
    /// "WalletConnect not configured" UI state when `false`.
    configured: bool,
}

/// Current effective WalletConnect projectId (config value, else env default).
/// Readable by the wallet UI so the in-page WC responder can boot the relay with
/// it (or show the "not configured" state).
async fn wc_project_id_get() -> Json<WcProjectIdResp> {
    let id = dig_node::wc_project_id();
    Json(WcProjectIdResp {
        configured: id.is_some(),
        project_id: id,
    })
}

#[derive(Deserialize)]
struct SetWcProjectId {
    project_id: String,
}

/// Persist the WalletConnect projectId (DIG settings). Restricted to the wallet's
/// own origin — only the settings UI may change it, never a dapp. A blank value
/// clears the override (falls back to the env default).
async fn wc_project_id_set(headers: HeaderMap, Json(req): Json<SetWcProjectId>) -> Response {
    if !is_self_origin(&origin_of(&headers)) {
        return (StatusCode::FORBIDDEN, "settings are wallet-local only").into_response();
    }
    match dig_node::set_wc_project_id(&req.project_id) {
        Ok(()) => {
            let id = dig_node::wc_project_id();
            (
                StatusCode::OK,
                Json(WcProjectIdResp {
                    configured: id.is_some(),
                    project_id: id,
                }),
            )
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// The wallet's public identity, safe to display in plain text.
#[derive(Serialize)]
struct PubKeyResp {
    /// The owner (first synthetic) public key, hex (no `0x`) — the same value
    /// `chip0002_getPublicKeys`/`derive_wallet_keys` exposes.
    public_key: String,
    /// The owner mainnet receive address (`xch1…`).
    address: String,
}

/// The wallet's public key + address (read-only, plain text in DIG settings).
/// Requires an unlocked session; public keys are not secret, but we still gate
/// to the wallet's own origin so it is never served to a dapp page.
async fn wallet_pubkey(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_self_origin(&origin_of(&headers)) {
        return (StatusCode::FORBIDDEN, "wallet-local only").into_response();
    }
    let mnemonic = {
        let s = st.session.lock().await;
        match s.as_ref() {
            Some(w) => w.mnemonic.clone(),
            None => return (StatusCode::UNAUTHORIZED, "wallet is locked").into_response(),
        }
    };
    match derive_wallet_keys(&mnemonic) {
        Ok(keys) => (
            StatusCode::OK,
            Json(PubKeyResp {
                public_key: hex::encode(keys.synthetic_pk.to_bytes()),
                address: owner_address(&keys),
            }),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct ExportReq {
    /// The wallet password — re-entered to authorize revealing the master secret.
    password: String,
}

#[derive(Serialize)]
struct ExportResp {
    /// The 24-word master mnemonic. THE master secret — anyone with it controls
    /// the funds. Served only to the wallet's own origin, only on a correct
    /// password, and never to any dapp-facing path.
    mnemonic: String,
}

/// Reveal the wallet's recovery phrase for backup. The most sensitive endpoint:
///
/// * **Self-origin only** — restricted to the wallet's own loopback origin via
///   the unspoofable `Origin` header, so a dapp page can never call it.
/// * **Password-gated** — the on-disk encrypted seed is decrypted with the
///   password supplied *in this request* (not the live session), so revealing
///   the master secret always requires re-proving the password.
/// * **Unreachable from dapps** — it is its own route, NOT a `wc_dispatch`
///   method, so no `/api/wc/request`, injected provider, or WC session can hit it.
///
/// The mnemonic is never logged. The UI reveals it transiently (reveal-then-hide).
async fn export(headers: HeaderMap, Json(req): Json<ExportReq>) -> Response {
    // Gate 1: only the wallet's own UI origin may ever ask to export.
    if !is_self_origin(&origin_of(&headers)) {
        return (StatusCode::FORBIDDEN, "export is wallet-local only").into_response();
    }
    // Gate 2: re-decrypt the on-disk seed with the supplied password. A wrong
    // password fails decryption (AEAD) — there is no other way to recover it.
    let bytes = match std::fs::read(seed_path()) {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "no wallet on this device").into_response(),
    };
    let enc = match EncryptedSeed::from_bytes(&bytes) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("corrupt seed file: {e}"),
            )
                .into_response()
        }
    };
    match decrypt_seed(&enc, &req.password) {
        Ok(m) => (
            StatusCode::OK,
            Json(ExportResp {
                mnemonic: m.to_string(),
            }),
        )
            .into_response(),
        Err(_) => (StatusCode::UNAUTHORIZED, "wrong password").into_response(),
    }
}

// ---- WalletConnect / CHIP-0002 dapp signer ----------------------------------
//
// The in-page WalletConnect client (loopback UI) pairs with dapps over the WC
// relay and forwards each CHIP-0002 / chia request here. The cryptographic core
// lives in `digstore_chain::chip0002` (byte-exact to Sage); this layer is just
// routing + the unlocked-session gate.

/// A single WC request forwarded from the in-page WC client.
#[derive(Deserialize)]
struct WcRequest {
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// Sage's default `getPublicKeys` window, and the most keys we will derive for
/// one request (a dapp must not be able to force unbounded derivation).
const DEFAULT_PUBKEYS: u32 = 10;
const MAX_PUBKEYS: u32 = 1000;
/// Wallet indices searched when matching a `publicKey` / covering coin spends.
const KEY_SEARCH_WINDOW: u32 = 100;

/// Resolve `{offset?, limit?}` for `chip0002_getPublicKeys`: Sage's defaults,
/// with the limit clamped so a dapp can't make us derive unboundedly.
fn pubkey_window(params: &serde_json::Value) -> (u32, u32) {
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n.min(MAX_PUBKEYS as u64) as u32)
        .unwrap_or(DEFAULT_PUBKEYS);
    (offset, limit)
}

/// Whether a WC method requires an unlocked wallet. The handshake methods
/// (`chainId`, `connect`) are answered without one; anything that reads keys or
/// signs requires an unlocked session.
fn wc_method_needs_wallet(method: &str) -> bool {
    !matches!(method, "chip0002_chainId" | "chip0002_connect")
}

/// The per-origin permission decision for a WC request. A dapp's web origin
/// (from the unspoofable HTTP `Origin` header) must be explicitly approved by the
/// user before it can read keys or request signatures.
#[derive(Debug, PartialEq, Eq)]
enum Gate {
    /// No origin approval needed (e.g. `chainId`).
    Public,
    /// Origin is approved — proceed.
    Allowed,
    /// `connect` from an unapproved origin — record it as pending and ask the user.
    NeedsApproval,
    /// A key/sign method from an unapproved origin — refuse; it must `connect` first.
    Forbidden,
}

/// Decide what to do with `method` from an origin that is (or isn't) approved.
/// Pure so the consent policy is unit-tested independently of HTTP/state.
fn wc_gate(method: &str, origin_approved: bool) -> Gate {
    match method {
        "chip0002_chainId" => Gate::Public,
        "chip0002_connect" => {
            if origin_approved {
                Gate::Allowed
            } else {
                Gate::NeedsApproval
            }
        }
        _ => {
            if origin_approved {
                Gate::Allowed
            } else {
                Gate::Forbidden
            }
        }
    }
}

fn wc_err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, String) {
    (code, msg.into())
}

/// Dispatch one WalletConnect / CHIP-0002 request to the native signer, returning
/// the bare result value Sage would return (the in-page WC client wraps it into
/// the WC response). Signing methods require an unlocked wallet.
async fn wc_dispatch(
    st: &AppState,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, (StatusCode, String)> {
    use serde_json::json;

    if !wc_method_needs_wallet(method) {
        return Ok(match method {
            "chip0002_chainId" => json!("mainnet"),
            "chip0002_connect" => json!(true),
            _ => unreachable!("non-wallet method list is exhaustive"),
        });
    }

    let mnemonic = {
        let s = st.session.lock().await;
        match s.as_ref() {
            Some(w) => w.mnemonic.clone(),
            None => return Err(wc_err(StatusCode::UNAUTHORIZED, "wallet is locked")),
        }
    };
    let bad = |e: digstore_chain::error::ChainError| wc_err(StatusCode::BAD_REQUEST, e.to_string());

    match method {
        "chip0002_getPublicKeys" => {
            let (offset, limit) = pubkey_window(&params);
            let keys = digstore_chain::chip0002::wallet_public_keys(&mnemonic, offset, limit)
                .map_err(bad)?;
            Ok(json!(keys))
        }
        "chip0002_signMessage" => {
            let message = params
                .get("message")
                .and_then(|m| m.as_str())
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'message'"))?;
            let public_key = params
                .get("publicKey")
                .and_then(|m| m.as_str())
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'publicKey'"))?;
            let sig = digstore_chain::chip0002::sign_message_by_public_key(
                &mnemonic,
                public_key,
                message.as_bytes(),
                KEY_SEARCH_WINDOW,
            )
            .map_err(bad)?;
            Ok(json!(sig))
        }
        "chip0002_signCoinSpends" => {
            let spends_val = params
                .get("coinSpends")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let spends: Vec<digstore_chain::chip0002::WcCoinSpend> =
                serde_json::from_value(spends_val)
                    .map_err(|e| wc_err(StatusCode::BAD_REQUEST, format!("bad coinSpends: {e}")))?;
            let sig = digstore_chain::chip0002::sign_wc_coin_spends(
                &mnemonic,
                &spends,
                KEY_SEARCH_WINDOW,
            )
            .map_err(bad)?;
            Ok(json!(sig))
        }
        "chia_getAddress" => {
            let keys = digstore_chain::keys::derive_wallet_keys(&mnemonic).map_err(bad)?;
            Ok(json!({ "address": digstore_chain::keys::owner_address(&keys) }))
        }
        "chia_signMessageByAddress" => {
            let message = params
                .get("message")
                .and_then(|m| m.as_str())
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'message'"))?;
            let address = params
                .get("address")
                .and_then(|m| m.as_str())
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'address'"))?;
            let signed = digstore_chain::chip0002::sign_message_by_address(
                &mnemonic,
                address,
                message.as_bytes(),
            )
            .map_err(bad)?;
            Ok(json!(signed))
        }
        "chip0002_getAssetBalance" => {
            // The hub reads `resp.confirmed`; type null => XCH, type "cat" => a CAT
            // identified by `assetId` (DIG when omitted). Generic over the TAIL, so
            // any CAT the wallet holds is reported — not just DIG. Drives the
            // account-menu XCH balance and every token balance widget.
            let is_cat = params.get("type").and_then(|t| t.as_str()) == Some("cat");
            let chain = Coinset::mainnet();
            let confirmed = if is_cat {
                let asset_id = cat_asset_id(&params)?;
                // Spendable base units of this CAT at the wallet's per-asset CAT puzzle
                // hash, summed across every scanned HD address (Sage-style).
                let owner_phs = wallet_owner_phs(&mnemonic).await?;
                let mut total = 0u64;
                for ph in owner_phs {
                    total = total.saturating_add(
                        digstore_chain::cat::cat_balance(&chain, ph, asset_id)
                            .await
                            .map_err(|e| {
                                wc_err(
                                    StatusCode::BAD_GATEWAY,
                                    format!("coinset CAT balance failed: {e}"),
                                )
                            })?,
                    );
                }
                total
            } else {
                let scanned = scan_wallet(&chain, &mnemonic).await.map_err(|e| {
                    wc_err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}"))
                })?;
                scanned.xch_balance()
            };
            Ok(json!({ "confirmed": confirmed, "spendable": confirmed }))
        }
        "chip0002_getAssetCoins" => {
            // The hub's spend path. Returns Sage's SpendableCoin shape: each entry is
            // { coin{parent_coin_info,puzzle_hash,amount}, locked, spent_block_index }
            // plus, for XCH, `puzzle` = the standard p2 reveal curried with that
            // coin's synthetic key (the hub uncurries it to recover the key per coin).
            // DIG CAT entries omit `puzzle` — the hub rebuilds the CAT lineage proof
            // from the parent spend. type null => XCH; type "cat" => the DIG CAT.
            let is_cat = params.get("type").and_then(|t| t.as_str()) == Some("cat");
            let offset = params.get("offset").and_then(|o| o.as_u64()).unwrap_or(0) as usize;
            let limit = params.get("limit").and_then(|l| l.as_u64()).unwrap_or(100) as usize;
            let chain = Coinset::mainnet();
            let scanned = scan_wallet(&chain, &mnemonic).await.map_err(|e| {
                wc_err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}"))
            })?;
            let mut coins = Vec::new();
            if is_cat {
                // Generic over the TAIL: the unspent CAT coins of `assetId` (DIG when
                // omitted) at each address's per-asset CAT puzzle hash. The DIG scan
                // shortcut still applies for DIG; any other CAT is read directly.
                let asset_id = cat_asset_id(&params)?;
                let is_dig = asset_id == digstore_chain::dig::DIG_ASSET_ID;
                for a in &scanned.addrs {
                    if is_dig {
                        for c in &a.dig {
                            coins.push(coin_entry_json(c, None));
                        }
                    } else {
                        let ph = digstore_chain::cat::cat_puzzle_hash(
                            a.keys.owner_puzzle_hash,
                            asset_id,
                        );
                        let cat_coins = chain.unspent_coins(ph).await.map_err(|e| {
                            wc_err(
                                StatusCode::BAD_GATEWAY,
                                format!("coinset CAT coins failed: {e}"),
                            )
                        })?;
                        for c in &cat_coins {
                            coins.push(coin_entry_json(c, None));
                        }
                    }
                }
            } else {
                for a in &scanned.addrs {
                    let puzzle =
                        digstore_chain::send::standard_puzzle_reveal_hex(&a.keys.synthetic_pk)
                            .map_err(bad)?;
                    for c in &a.xch {
                        coins.push(coin_entry_json(c, Some(&puzzle)));
                    }
                }
            }
            let page: Vec<_> = coins.into_iter().skip(offset).take(limit).collect();
            Ok(json!({ "coins": page }))
        }
        "chia_takeOffer" => {
            // The hub's badge-minting path: accept a MintGarden offer (pay the
            // requested DIG, receive the badge NFT). Build + BLS-sign the taker side
            // over the wallet's scanned DIG/XCH; the maker's half is already signed
            // inside the offer. Like /api/send, this is gated TWICE before it spends
            // real funds: it dry-runs (signs but pushes nothing) unless the env opts
            // in with DIG_WALLET_ALLOW_BROADCAST=1.
            let offer = params
                .get("offer")
                .and_then(|o| o.as_str())
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'offer'"))?;
            // Fee tolerated as number or decimal string (dapps send both); default 0.
            let fee = params
                .get("fee")
                .map(|f| {
                    f.as_u64()
                        .or_else(|| f.as_str().and_then(|s| s.trim().parse().ok()))
                        .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "invalid 'fee'"))
                })
                .transpose()?
                .unwrap_or(0);

            let chain = Coinset::mainnet();
            // mainnet agg_sig (for_testnet = false) — same as the canonical send path.
            let taken =
                digstore_chain::offer::build_take_offer(&chain, &mnemonic, offer, fee, false)
                    .await
                    .map_err(bad)?;

            let broadcast_enabled =
                std::env::var("DIG_WALLET_ALLOW_BROADCAST").ok().as_deref() == Some("1");
            // A dapp cannot itself force a broadcast; taking an offer is treated as a
            // broadcast-intent request, so it pushes only when the env also opts in.
            let status = match send_action(true, broadcast_enabled) {
                SendAction::Broadcast => {
                    chain
                        .push(taken.bundle.clone())
                        .await
                        .map_err(|e| wc_err(StatusCode::BAD_GATEWAY, format!("push_tx: {e}")))?;
                    "broadcast"
                }
                // Disabled → dry run (signed, not pushed). The hub still gets a
                // success-shaped result proving the take was built + signed.
                SendAction::RefusedDisabled | SendAction::DryRun => "signed",
            };

            // Sage's chia_takeOffer returns a transaction-ish object; the hub tolerates
            // any shape, so report status + the signed bundle + what was paid.
            Ok(json!({
                "status": status,
                "success": true,
                "spendBundle": {
                    "coinSpends": taken.bundle.coin_spends.len(),
                    "aggregatedSignature": hex::encode(taken.bundle.aggregated_signature.to_bytes()),
                },
                "paid": {
                    "xch": taken.cost.xch.to_string(),
                    "cats": taken
                        .cost
                        .cats
                        .iter()
                        .map(|(id, amt)| json!({
                            "assetId": format!("0x{}", hex::encode(id)),
                            "amount": amt.to_string(),
                        }))
                        .collect::<Vec<_>>(),
                },
            }))
        }
        "chia_getOfferSummary" => {
            // Inspect an offer WITHOUT taking it: offered/requested assets, the net
            // the taker must fund (arbitrage), and any NFT royalties. Read-only; needs
            // an unlocked wallet only to match the other methods' gate.
            let offer = params
                .get("offer")
                .and_then(|o| o.as_str())
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'offer'"))?;
            let summary = digstore_chain::offer::decode_offer_summary(offer).map_err(bad)?;
            Ok(offer_summary_json(&summary))
        }
        "chia_createOffer" => {
            // Build + sign a make-offer: OFFER `offered` assets, REQUEST `requested`
            // assets (paid to the maker), optional `fee`. Returns the bech32 offer1…
            // string (+ a decoded summary). Building an offer SPENDS nothing on its own
            // (settlement happens when someone takes it), so it returns the string
            // regardless of the broadcast gate.
            let offered = parse_offer_legs(&params, "offered")?;
            let requested = parse_offer_legs(&params, "requested")?;
            let fee = json_u64(&params, "fee").unwrap_or(0);

            let chain = Coinset::mainnet();
            let maker = digstore_chain::keys::derive_indexed_keys(&mnemonic, 0..1)
                .map_err(bad)?
                .into_iter()
                .next()
                .ok_or_else(|| wc_err(StatusCode::INTERNAL_SERVER_ERROR, "no wallet key"))?;
            let scanned = scan_wallet(&chain, &mnemonic).await.map_err(|e| {
                wc_err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}"))
            })?;

            // XCH funding coins tagged with each address's keys.
            let mut xch: Vec<(chia_protocol::Coin, &digstore_chain::keys::IndexedKeys)> =
                Vec::new();
            for a in &scanned.addrs {
                for c in &a.xch {
                    xch.push((*c, &a.keys));
                }
            }
            // CAT funding coins (with lineage proofs) for each DISTINCT offered CAT.
            let offered_cats: BTreeSet<chia_protocol::Bytes32> = offered
                .iter()
                .filter_map(|l| match l {
                    digstore_chain::offer::OfferAsset::Cat { asset_id, .. } => Some(*asset_id),
                    _ => None,
                })
                .collect();
            let mut cats: Vec<(
                chia_wallet_sdk::driver::Cat,
                &digstore_chain::keys::IndexedKeys,
            )> = Vec::new();
            for asset_id in offered_cats {
                for a in &scanned.addrs {
                    let reconstructed = digstore_chain::cat::reconstruct_cat_coins(
                        &chain,
                        a.keys.owner_puzzle_hash,
                        asset_id,
                    )
                    .await
                    .map_err(bad)?;
                    for cat in reconstructed {
                        cats.push((cat, &a.keys));
                    }
                }
            }

            let funds = digstore_chain::offer::MakerFunds { xch, cats };
            let offer_str = digstore_chain::offer::build_make_offer(
                &maker, funds, &offered, &requested, fee, false,
            )
            .map_err(bad)?;
            let summary = digstore_chain::offer::decode_offer_summary(&offer_str).map_err(bad)?;
            Ok(json!({ "offer": offer_str, "summary": offer_summary_json(&summary) }))
        }
        "chia_cancelOffer" => {
            // Cancel an offer the wallet MADE: re-spend the offered coins back to the
            // maker, invalidating the outstanding offer1… string. State-changing, so
            // gated by the broadcast/dry-run env gate (signed-only unless opted in).
            let offer = params
                .get("offer")
                .and_then(|o| o.as_str())
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'offer'"))?;
            let fee = json_u64(&params, "fee").unwrap_or(0);
            let maker = digstore_chain::keys::derive_indexed_keys(&mnemonic, 0..1)
                .map_err(bad)?
                .into_iter()
                .next()
                .ok_or_else(|| wc_err(StatusCode::INTERNAL_SERVER_ERROR, "no wallet key"))?;
            let bundle =
                digstore_chain::offer::cancel_offer(offer, &maker, fee, false).map_err(bad)?;
            let chain = Coinset::mainnet();
            let n = bundle.coin_spends.len();
            let status = broadcast_or_dry_run(&chain, bundle.clone()).await?;
            Ok(json!({
                "status": status,
                "success": true,
                "spendBundle": {
                    "coinSpends": n,
                    "aggregatedSignature": hex::encode(bundle.aggregated_signature.to_bytes()),
                },
            }))
        }
        "chia_send" => {
            // Sage-parity send: pay XCH (type null) or a generic CAT (type "cat",
            // identified by `assetId`) to `address`, with optional `memos` and `fee`.
            // Like /api/send, this spends real funds only when the env opts in; a
            // dapp-originated send is treated as broadcast-intent and pushes only when
            // DIG_WALLET_ALLOW_BROADCAST=1 (otherwise dry-run / "signed").
            let to = params
                .get("address")
                .or_else(|| params.get("to"))
                .and_then(|a| a.as_str())
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'address'"))?;
            let recipient_ph = digstore_chain::send::decode_xch_address(to).map_err(bad)?;
            let amount = json_u64(&params, "amount")
                .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "missing 'amount'"))?;
            let fee = json_u64(&params, "fee").unwrap_or(0);
            let is_cat = params.get("type").and_then(|t| t.as_str()) == Some("cat");
            let chain = Coinset::mainnet();

            let (signed_bundle, coin_spends, status_value) = if is_cat {
                let asset_id = cat_asset_id(&params)?;
                // Reconstruct this asset's CAT coins (with lineage proofs) across every
                // scanned address; the recipient/change use the primary owner key.
                let keys = digstore_chain::keys::derive_wallet_keys(&mnemonic).map_err(bad)?;
                let scanned = scan_wallet(&chain, &mnemonic).await.map_err(|e| {
                    wc_err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}"))
                })?;
                let mut cats = Vec::new();
                for a in &scanned.addrs {
                    cats.extend(
                        digstore_chain::cat::reconstruct_cat_coins(
                            &chain,
                            a.keys.owner_puzzle_hash,
                            asset_id,
                        )
                        .await
                        .map_err(bad)?,
                    );
                }
                let memos = parse_memo_hashes(&params)?;
                let (bundle, plan) = digstore_chain::cat::build_cat_send(
                    &keys,
                    &cats,
                    asset_id,
                    recipient_ph,
                    amount,
                    &memos,
                    fee,
                    false,
                )
                .map_err(bad)?;
                let n = bundle.coin_spends.len();
                (
                    bundle,
                    n,
                    json!({ "amount": plan.amount.to_string(), "change": plan.change.to_string() }),
                )
            } else {
                let scanned = scan_wallet(&chain, &mnemonic).await.map_err(|e| {
                    wc_err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}"))
                })?;
                let (bundle, plan) =
                    digstore_chain::send::build_xch_send(&scanned, recipient_ph, amount, fee)
                        .map_err(bad)?;
                let n = bundle.coin_spends.len();
                (
                    bundle,
                    n,
                    json!({ "amount": amount.to_string(), "change": plan.change.to_string() }),
                )
            };

            let status = broadcast_or_dry_run(&chain, signed_bundle.clone()).await?;
            Ok(json!({
                "status": status,
                "success": true,
                "spendBundle": {
                    "coinSpends": coin_spends,
                    "aggregatedSignature": hex::encode(signed_bundle.aggregated_signature.to_bytes()),
                },
                "sent": status_value,
            }))
        }
        other => Err(wc_err(
            StatusCode::NOT_IMPLEMENTED,
            format!("unsupported WC method: {other}"),
        )),
    }
}

/// One `getAssetCoins` entry in Sage's SpendableCoin shape. `amount` is a decimal
/// string (BigInt-safe); `puzzle` (the standard p2 reveal) is present for XCH coins
/// and omitted for CAT coins (whose lineage the hub rebuilds from the parent spend).
fn coin_entry_json(coin: &chia_protocol::Coin, puzzle: Option<&str>) -> serde_json::Value {
    use serde_json::json;
    let mut e = json!({
        "coin": {
            "parent_coin_info": format!("0x{}", hex::encode(coin.parent_coin_info)),
            "puzzle_hash": format!("0x{}", hex::encode(coin.puzzle_hash)),
            "amount": coin.amount.to_string(),
        },
        "locked": false,
        "spent_block_index": 0,
    });
    if let Some(p) = puzzle {
        e["puzzle"] = json!(p);
    }
    e
}

/// Parse a 32-byte CAT asset id (TAIL hash) from a hex string (`0x`-prefixed or
/// bare). Pure so the parse/validation is unit-tested independently of HTTP.
fn parse_asset_id_hex(s: &str) -> Result<chia_protocol::Bytes32, String> {
    let hex = s.trim().trim_start_matches("0x");
    let bytes = hex::decode(hex).map_err(|_| format!("invalid asset id hex: {s}"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| format!("asset id must be 32 bytes: {s}"))?;
    Ok(chia_protocol::Bytes32::new(arr))
}

/// The CAT `assetId` for a request, defaulting to the DIG TAIL when omitted/empty
/// (so the common DIG path needs no asset id). Any 32-byte TAIL is accepted —
/// generic over the asset, no allow-list.
fn cat_asset_id(
    params: &serde_json::Value,
) -> Result<chia_protocol::Bytes32, (StatusCode, String)> {
    let raw = params
        .get("assetId")
        .and_then(|a| a.as_str())
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        return Ok(digstore_chain::dig::DIG_ASSET_ID);
    }
    parse_asset_id_hex(raw).map_err(|e| wc_err(StatusCode::BAD_REQUEST, e))
}

/// A `u64` from `params[key]`, tolerating both a JSON number and a decimal string
/// (dapps send amounts/fees as either; BigInt-safe values arrive as strings).
fn json_u64(params: &serde_json::Value, key: &str) -> Option<u64> {
    let v = params.get(key)?;
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Parse optional `memos` (array of hex strings) into 32-byte memo hashes for a CAT
/// send. Memos that are not 32-byte hex are rejected (the CAT memo slot is a hash).
fn parse_memo_hashes(
    params: &serde_json::Value,
) -> Result<Vec<chia_protocol::Bytes32>, (StatusCode, String)> {
    let Some(arr) = params.get("memos").and_then(|m| m.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(arr.len());
    for m in arr {
        let s = m
            .as_str()
            .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, "each memo must be a hex string"))?;
        out.push(parse_asset_id_hex(s).map_err(|e| wc_err(StatusCode::BAD_REQUEST, e))?);
    }
    Ok(out)
}

/// The wallet's HD owner puzzle hashes (one per scanned address), used to find a
/// wallet's coins/NFTs/DIDs across every derived address. Scans over coinset.
async fn wallet_owner_phs(
    mnemonic: &str,
) -> Result<Vec<chia_protocol::Bytes32>, (StatusCode, String)> {
    let chain = Coinset::mainnet();
    let scanned = scan_wallet(&chain, mnemonic)
        .await
        .map_err(|e| wc_err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}")))?;
    Ok(scanned
        .addrs
        .iter()
        .map(|a| a.keys.owner_puzzle_hash)
        .collect())
}

/// Apply the SAME broadcast gate the XCH `send`/`chia_takeOffer` paths use to a
/// built+signed bundle: push only when the env opts in (`DIG_WALLET_ALLOW_BROADCAST=1`),
/// otherwise dry-run (the bundle is signed but nothing is pushed). A dapp can never
/// itself force a broadcast — taking/sending is broadcast-INTENT, gated by the env.
/// Returns `"broadcast"` or `"signed"`.
async fn broadcast_or_dry_run(
    chain: &Coinset,
    bundle: chia_protocol::SpendBundle,
) -> Result<&'static str, (StatusCode, String)> {
    let broadcast_enabled =
        std::env::var("DIG_WALLET_ALLOW_BROADCAST").ok().as_deref() == Some("1");
    Ok(match send_action(true, broadcast_enabled) {
        SendAction::Broadcast => {
            chain
                .push(bundle)
                .await
                .map_err(|e| wc_err(StatusCode::BAD_GATEWAY, format!("push_tx: {e}")))?;
            "broadcast"
        }
        SendAction::RefusedDisabled | SendAction::DryRun => "signed",
    })
}

/// Parse an offer's `offered`/`requested` legs from `params[key]` — an array of
/// `{ assetId?, amount }` where a missing/empty `assetId` means XCH and any 32-byte
/// TAIL means that CAT — into `OfferAsset`s. Generic over the asset (no allow-list).
fn parse_offer_legs(
    params: &serde_json::Value,
    key: &str,
) -> Result<Vec<digstore_chain::offer::OfferAsset>, (StatusCode, String)> {
    use digstore_chain::offer::OfferAsset;
    let arr = params
        .get(key)
        .and_then(|v| v.as_array())
        .ok_or_else(|| wc_err(StatusCode::BAD_REQUEST, format!("missing '{key}' (array)")))?;
    let mut out = Vec::with_capacity(arr.len());
    for leg in arr {
        let amount = json_u64(leg, "amount").ok_or_else(|| {
            wc_err(
                StatusCode::BAD_REQUEST,
                format!("{key} leg missing 'amount'"),
            )
        })?;
        let asset_raw = leg
            .get("assetId")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .trim();
        if asset_raw.is_empty() {
            out.push(OfferAsset::Xch(amount));
        } else {
            let asset_id =
                parse_asset_id_hex(asset_raw).map_err(|e| wc_err(StatusCode::BAD_REQUEST, e))?;
            out.push(OfferAsset::Cat { asset_id, amount });
        }
    }
    Ok(out)
}

/// Render an [`OfferAsset`] as `{ type, assetId?, amount }` (amount as a BigInt-safe
/// decimal string). XCH carries no `assetId`.
fn offer_asset_json(a: &digstore_chain::offer::OfferAsset) -> serde_json::Value {
    use digstore_chain::offer::OfferAsset;
    match a {
        OfferAsset::Xch(amount) => {
            serde_json::json!({ "type": "xch", "amount": amount.to_string() })
        }
        OfferAsset::Cat { asset_id, amount } => serde_json::json!({
            "type": "cat",
            "assetId": format!("0x{}", hex::encode(asset_id)),
            "amount": amount.to_string(),
        }),
    }
}

/// Render an [`OfferSummary`] (offered/requested assets, the taker's funding cost,
/// and any NFT royalties) as the JSON the wallet UI / dapps consume.
fn offer_summary_json(s: &digstore_chain::offer::OfferSummary) -> serde_json::Value {
    serde_json::json!({
        "offered": s.offered.iter().map(offer_asset_json).collect::<Vec<_>>(),
        "requested": s.requested.iter().map(offer_asset_json).collect::<Vec<_>>(),
        "arbitrage": {
            "xch": s.arbitrage.xch.to_string(),
            "cats": s.arbitrage.cats.iter().map(|(id, amt)| serde_json::json!({
                "assetId": format!("0x{}", hex::encode(id)),
                "amount": amt.to_string(),
            })).collect::<Vec<_>>(),
        },
        "royalties": s.royalties.iter().map(|(launcher, bp)| serde_json::json!({
            "launcherId": format!("0x{}", hex::encode(launcher)),
            "basisPoints": bp,
        })).collect::<Vec<_>>(),
    })
}

/// The dapp's web origin, from the unspoofable HTTP `Origin` header (page JS
/// cannot forge it on a cross-origin fetch). Empty if absent.
fn origin_of(headers: &HeaderMap) -> String {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// A JSON response carrying the CORS header a dapp page needs to read it. Security
/// is the per-origin approval gate, not CORS, so the origin is reflected.
fn cors_json(origin: &str, status: StatusCode, body: serde_json::Value) -> Response {
    let mut resp = (status, Json(body)).into_response();
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(if origin.is_empty() { "null" } else { origin }) {
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    h.insert(header::VARY, HeaderValue::from_static("Origin"));
    resp
}

/// CORS preflight for the dapp-facing `/api/wc/request` endpoint.
async fn wc_preflight(headers: HeaderMap) -> Response {
    let origin = origin_of(&headers);
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(if origin.is_empty() { "null" } else { &origin }) {
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    h.insert(header::VARY, HeaderValue::from_static("Origin"));
    resp
}

/// The dapp-facing WalletConnect endpoint. Applies the per-origin consent gate,
/// then dispatches to the native signer, always with CORS so the dapp can read
/// the reply.
async fn wc_request(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<WcRequest>,
) -> Response {
    let origin = origin_of(&headers);
    let approved = st.approvals.lock().await.is_approved(&origin);

    match wc_gate(&req.method, approved) {
        Gate::NeedsApproval => {
            if !origin.is_empty() {
                st.approvals.lock().await.pending.insert(origin.clone());
            }
            return cors_json(
                &origin,
                StatusCode::ACCEPTED,
                serde_json::json!({ "status": "pending" }),
            );
        }
        Gate::Forbidden => {
            return cors_json(
                &origin,
                StatusCode::FORBIDDEN,
                serde_json::json!({
                    "error": "origin not connected — call chip0002_connect and approve it in the DIG wallet"
                }),
            );
        }
        Gate::Public | Gate::Allowed => {}
    }

    match wc_dispatch(&st, &req.method, req.params).await {
        Ok(data) => cors_json(&origin, StatusCode::OK, serde_json::json!({ "data": data })),
        Err((code, msg)) => cors_json(&origin, code, serde_json::json!({ "error": msg })),
    }
}

/// The dapp connections the wallet UI shows: approved allow-list + pending requests.
#[derive(Serialize)]
struct ConnectionsResp {
    approved: Vec<String>,
    pending: Vec<String>,
}

async fn wc_connections(State(st): State<Arc<AppState>>) -> Json<ConnectionsResp> {
    let a = st.approvals.lock().await;
    Json(ConnectionsResp {
        approved: a.approved.iter().cloned().collect(),
        pending: a.pending.iter().cloned().collect(),
    })
}

#[derive(Deserialize)]
struct OriginReq {
    origin: String,
}

/// Approve a pending dapp origin (user action in the wallet) — persists it.
async fn wc_approve(
    State(st): State<Arc<AppState>>,
    Json(req): Json<OriginReq>,
) -> impl IntoResponse {
    let mut a = st.approvals.lock().await;
    a.pending.remove(&req.origin);
    a.approved.insert(req.origin.clone());
    save_approved(&a.approved);
    StatusCode::NO_CONTENT
}

/// Reject a pending dapp origin (drop it without approving).
async fn wc_reject(
    State(st): State<Arc<AppState>>,
    Json(req): Json<OriginReq>,
) -> impl IntoResponse {
    st.approvals.lock().await.pending.remove(&req.origin);
    StatusCode::NO_CONTENT
}

/// Revoke a previously-approved dapp origin — persists the removal.
async fn wc_revoke(
    State(st): State<Arc<AppState>>,
    Json(req): Json<OriginReq>,
) -> impl IntoResponse {
    let mut a = st.approvals.lock().await;
    a.approved.remove(&req.origin);
    save_approved(&a.approved);
    StatusCode::NO_CONTENT
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
        .route("/wc-bundle.js", get(wc_bundle_js))
        .route("/settings", get(settings_page))
        .route("/api/status", get(status))
        .route("/api/generate", post(generate))
        .route("/api/import", post(import))
        .route("/api/unlock", post(unlock))
        .route("/api/balance", get(balance))
        .route("/api/send", post(send))
        .route("/api/lock", post(lock))
        .route("/api/dig-config", get(dig_config_get).post(dig_config_set))
        .route("/api/dig-cache/clear", post(dig_cache_clear))
        .route(
            "/api/wc/project-id",
            get(wc_project_id_get).post(wc_project_id_set),
        )
        .route("/api/wallet/pubkey", get(wallet_pubkey))
        .route("/api/export", post(export))
        .route("/api/wc/request", post(wc_request).options(wc_preflight))
        .route("/api/wc/connections", get(wc_connections))
        .route("/api/wc/approve", post(wc_approve))
        .route("/api/wc/reject", post(wc_reject))
        .route("/api/wc/revoke", post(wc_revoke))
        .with_state(state);

    // Bind loopback only — the wallet must never be reachable off-host.
    let addr = format!("127.0.0.1:{}", wallet_port());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("dig-wallet: cannot bind {addr}: {e}"));
    println!("dig-wallet listening on http://{addr}");
    axum::serve(listener, app).await.expect("dig-wallet server");
}

/// The Sage-mirroring wallet UI (single self-contained page). Dark, luxury,
/// DIG-purple / Chia-green accents.
const UI_HTML: &str = include_str!("ui.html");

/// The DIG protocol settings page (single self-contained page). Same dark luxury
/// DIG aesthetic as the wallet; first setting is the native local-cache threshold.
const SETTINGS_HTML: &str = include_str!("settings.html");

/// The bundled WalletConnect responder client (`window.DigWC`), generated by
/// `wc/build.mjs` (esbuild). Checked in so the crate builds offline; served at
/// `/wc-bundle.js`.
const WC_BUNDLE_JS: &str = include_str!("wc-bundle.js");

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

    #[test]
    fn coin_entry_json_matches_sage_spendable_coin_shape() {
        let coin = chia_protocol::Coin::new([1u8; 32].into(), [2u8; 32].into(), 12345);
        // XCH entry carries the standard puzzle reveal the hub uncurries per coin.
        let xch = coin_entry_json(&coin, Some("0xdeadbeef"));
        assert_eq!(
            xch["coin"]["parent_coin_info"],
            format!("0x{}", "01".repeat(32))
        );
        assert_eq!(xch["coin"]["puzzle_hash"], format!("0x{}", "02".repeat(32)));
        assert_eq!(xch["coin"]["amount"], "12345"); // decimal string (BigInt-safe)
        assert_eq!(xch["puzzle"], "0xdeadbeef");
        assert_eq!(xch["locked"], false);
        assert_eq!(xch["spent_block_index"], 0);
        // CAT entry omits `puzzle` (the hub rebuilds the lineage from the parent spend).
        let cat = coin_entry_json(&coin, None);
        assert!(cat.get("puzzle").is_none());
        assert_eq!(cat["coin"]["amount"], "12345");
    }

    #[test]
    fn cache_cap_is_floored_so_caching_cant_be_disabled() {
        // A 0 / tiny request must not disable the cache (which would defeat
        // local-first and hammer rpc.dig.net) — it floors to the 64 MiB minimum.
        assert_eq!(floored_cache_cap(0), 64 * 1024 * 1024);
        assert_eq!(floored_cache_cap(1), 64 * 1024 * 1024);
        // A request above the floor is honoured verbatim.
        assert_eq!(
            floored_cache_cap(5 * 1024 * 1024 * 1024),
            5 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn pubkey_window_defaults_and_clamps() {
        // No params → Sage's default first 10 keys at offset 0.
        assert_eq!(pubkey_window(&serde_json::Value::Null), (0, 10));
        // Explicit offset/limit honoured.
        assert_eq!(
            pubkey_window(&serde_json::json!({"offset": 5, "limit": 3})),
            (5, 3)
        );
        // An absurd limit is clamped so a dapp can't make us derive forever.
        let (off, lim) = pubkey_window(&serde_json::json!({"limit": 100000}));
        assert_eq!(off, 0);
        assert!(lim <= MAX_PUBKEYS, "limit clamped to {MAX_PUBKEYS}");
    }

    #[test]
    fn only_the_exact_wallet_origin_is_self_trusted() {
        // The wallet's own page origin is trusted (it serves the UI)…
        assert!(is_self_origin("http://127.0.0.1:9777"));
        assert!(is_self_origin("http://localhost:9777"));
        // …but NOT some other local server on a different port (that would let any
        // localhost app spend the wallet without approval).
        assert!(!is_self_origin("http://127.0.0.1:8099"));
        assert!(!is_self_origin("http://127.0.0.1"));
        assert!(!is_self_origin("https://example.com"));
        assert!(!is_self_origin(""));
    }

    #[test]
    fn wc_origin_gate() {
        // chainId is public — no origin approval needed.
        assert_eq!(wc_gate("chip0002_chainId", false), Gate::Public);
        // connect from an unapproved origin must ask the user; from an approved
        // origin it just succeeds.
        assert_eq!(wc_gate("chip0002_connect", false), Gate::NeedsApproval);
        assert_eq!(wc_gate("chip0002_connect", true), Gate::Allowed);
        // Any key/sign method is forbidden until the origin is approved.
        assert_eq!(wc_gate("chip0002_signMessage", false), Gate::Forbidden);
        assert_eq!(wc_gate("chip0002_signCoinSpends", false), Gate::Forbidden);
        assert_eq!(wc_gate("chip0002_getPublicKeys", false), Gate::Forbidden);
        assert_eq!(wc_gate("chia_getAddress", false), Gate::Forbidden);
        // …and allowed once approved.
        assert_eq!(wc_gate("chip0002_signMessage", true), Gate::Allowed);
    }

    #[test]
    fn wc_methods_that_need_a_wallet() {
        // Public handshake methods never need an unlocked wallet…
        assert!(!wc_method_needs_wallet("chip0002_chainId"));
        assert!(!wc_method_needs_wallet("chip0002_connect"));
        // …but anything that reads keys or signs does.
        assert!(wc_method_needs_wallet("chip0002_getPublicKeys"));
        assert!(wc_method_needs_wallet("chip0002_signMessage"));
        assert!(wc_method_needs_wallet("chip0002_signCoinSpends"));
        assert!(wc_method_needs_wallet("chia_getAddress"));
        // Taking an offer builds + signs a spend, so it needs an unlocked wallet…
        assert!(wc_method_needs_wallet("chia_takeOffer"));
    }

    #[test]
    fn take_offer_is_gated_behind_the_origin_consent_and_wallet() {
        // chia_takeOffer is a spend method: forbidden until the origin is approved,
        // allowed once approved (same gate as the other signing methods). This guards
        // the badge-minting path from an unapproved dapp triggering a take.
        assert_eq!(wc_gate("chia_takeOffer", false), Gate::Forbidden);
        assert_eq!(wc_gate("chia_takeOffer", true), Gate::Allowed);
    }

    #[test]
    fn asset_id_parses_with_or_without_0x_and_rejects_bad_len() {
        // A 32-byte hex TAIL parses identically with or without the 0x prefix.
        let bare = "ab".repeat(32);
        let prefixed = format!("0x{bare}");
        let a = parse_asset_id_hex(&bare).unwrap();
        let b = parse_asset_id_hex(&prefixed).unwrap();
        assert_eq!(a, b);
        assert_eq!(hex::encode(a), bare);
        // Wrong length / bad hex are rejected (not panics).
        assert!(parse_asset_id_hex("dead").is_err()); // too short
        assert!(parse_asset_id_hex(&"zz".repeat(32)).is_err()); // not hex
    }

    #[test]
    fn cat_asset_id_defaults_to_dig_and_accepts_any_tail() {
        // No / empty assetId → DIG (the common token path needs no id)…
        assert_eq!(
            cat_asset_id(&serde_json::Value::Null).unwrap(),
            digstore_chain::dig::DIG_ASSET_ID
        );
        assert_eq!(
            cat_asset_id(&serde_json::json!({ "assetId": "" })).unwrap(),
            digstore_chain::dig::DIG_ASSET_ID
        );
        // …and ANY 32-byte TAIL is accepted — no allow-list, generic over the asset.
        let tail = "cd".repeat(32);
        let got = cat_asset_id(&serde_json::json!({ "assetId": format!("0x{tail}") })).unwrap();
        assert_eq!(hex::encode(got), tail);
        // A malformed assetId is a 400.
        let (code, _) = cat_asset_id(&serde_json::json!({ "assetId": "nope" })).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn json_u64_tolerates_number_and_decimal_string() {
        // Amounts/fees arrive as numbers OR decimal strings (BigInt-safe) — both work.
        assert_eq!(
            json_u64(&serde_json::json!({"amount": 42}), "amount"),
            Some(42)
        );
        assert_eq!(
            json_u64(&serde_json::json!({"amount": "1000000000000"}), "amount"),
            Some(1_000_000_000_000)
        );
        assert_eq!(json_u64(&serde_json::json!({}), "amount"), None);
    }

    #[test]
    fn memo_hashes_parse_and_reject_non_hash() {
        // No memos → empty (a memo-less send is fine).
        assert!(parse_memo_hashes(&serde_json::Value::Null)
            .unwrap()
            .is_empty());
        // 32-byte hex memos parse in order.
        let m = "11".repeat(32);
        let got = parse_memo_hashes(&serde_json::json!({ "memos": [format!("0x{m}")] })).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(hex::encode(got[0]), m);
        // A non-32-byte memo is rejected (the CAT memo slot is a hash).
        let (code, _) = parse_memo_hashes(&serde_json::json!({ "memos": ["dead"] })).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn offer_legs_parse_xch_and_cat() {
        use digstore_chain::offer::OfferAsset;
        // A missing/empty assetId means XCH; a 32-byte TAIL means that CAT.
        let tail = "ab".repeat(32);
        let legs = parse_offer_legs(
            &serde_json::json!({
                "offered": [
                    { "amount": 1000 },
                    { "assetId": format!("0x{tail}"), "amount": "250" }
                ]
            }),
            "offered",
        )
        .unwrap();
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[0], OfferAsset::Xch(1000));
        match legs[1] {
            OfferAsset::Cat { asset_id, amount } => {
                assert_eq!(hex::encode(asset_id), tail);
                assert_eq!(amount, 250);
            }
            _ => panic!("expected a CAT leg"),
        }
        // A missing array is a 400 (so the builder never sees a half-formed offer).
        let (code, _) = parse_offer_legs(&serde_json::json!({}), "offered").unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        // A leg without an amount is a 400.
        let (code, _) =
            parse_offer_legs(&serde_json::json!({ "offered": [{}] }), "offered").unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn offer_summary_json_shape() {
        use digstore_chain::offer::{OfferAsset, OfferCost, OfferSummary};
        let tail = chia_protocol::Bytes32::new([0x42u8; 32]);
        let nft = chia_protocol::Bytes32::new([0x07u8; 32]);
        let s = OfferSummary {
            offered: vec![OfferAsset::Xch(5)],
            requested: vec![OfferAsset::Cat {
                asset_id: tail,
                amount: 9,
            }],
            arbitrage: OfferCost {
                xch: 0,
                cats: vec![(tail, 9)],
            },
            royalties: vec![(nft, 300)],
        };
        let j = offer_summary_json(&s);
        assert_eq!(j["offered"][0]["type"], "xch");
        assert_eq!(j["offered"][0]["amount"], "5"); // decimal string (BigInt-safe)
        assert_eq!(j["requested"][0]["type"], "cat");
        assert_eq!(
            j["requested"][0]["assetId"],
            format!("0x{}", "42".repeat(32))
        );
        assert_eq!(j["arbitrage"]["cats"][0]["amount"], "9");
        assert_eq!(j["royalties"][0]["basisPoints"], 300);
    }

    #[test]
    fn offer_methods_are_gated() {
        // Summary is read-only but still requires origin approval; create/cancel are
        // state-changing and need an unlocked wallet.
        assert_eq!(wc_gate("chia_getOfferSummary", false), Gate::Forbidden);
        assert_eq!(wc_gate("chia_createOffer", false), Gate::Forbidden);
        assert_eq!(wc_gate("chia_cancelOffer", true), Gate::Allowed);
        assert!(wc_method_needs_wallet("chia_createOffer"));
        assert!(wc_method_needs_wallet("chia_cancelOffer"));
        assert!(wc_method_needs_wallet("chia_getOfferSummary"));
    }

    #[test]
    fn token_methods_are_gated_and_need_a_wallet() {
        // chia_send is a spend method: forbidden until the origin is approved, allowed
        // once approved, and always needs an unlocked wallet.
        assert_eq!(wc_gate("chia_send", false), Gate::Forbidden);
        assert_eq!(wc_gate("chia_send", true), Gate::Allowed);
        assert!(wc_method_needs_wallet("chia_send"));
        // Generic CAT balance/coins still need a wallet (they read keys + scan).
        assert!(wc_method_needs_wallet("chip0002_getAssetBalance"));
        assert!(wc_method_needs_wallet("chip0002_getAssetCoins"));
    }

    #[test]
    fn settings_page_wires_the_cache_config_api() {
        // The served settings page must talk to the same config endpoints the
        // handlers expose, or the UI silently no-ops.
        assert!(SETTINGS_HTML.contains("/api/dig-config"));
        assert!(SETTINGS_HTML.contains("/api/dig-cache/clear"));
    }

    #[test]
    fn wallet_page_hosts_the_walletconnect_responder() {
        // The wallet page must load the bundled responder and expose the
        // "Connect a dapp" pairing surface; the bundle must be the real client
        // (exposes window.DigWC), and the page must read the effective projectId
        // from DIG settings so the relay boots with it.
        assert!(
            UI_HTML.contains("/wc-bundle.js"),
            "page loads the WC bundle"
        );
        assert!(UI_HTML.contains("DigWC"), "page uses the responder API");
        assert!(
            UI_HTML.contains("/api/wc/project-id"),
            "page reads the effective projectId"
        );
        // The bundle is the actual esbuild output, not a stub.
        assert!(
            WC_BUNDLE_JS.contains("var DigWC") && WC_BUNDLE_JS.len() > 100_000,
            "wc-bundle.js is the real bundled SignClient"
        );
    }

    #[test]
    fn settings_page_wires_the_new_settings_apis() {
        // The settings page must talk to the projectId, export, import, and
        // public-key endpoints, or those features silently no-op.
        assert!(SETTINGS_HTML.contains("/api/wc/project-id"));
        assert!(SETTINGS_HTML.contains("/api/export"));
        assert!(SETTINGS_HTML.contains("/api/import"));
        assert!(SETTINGS_HTML.contains("/api/wallet/pubkey"));
    }

    // -- Key export is unreachable from every dapp-facing path -----------------

    /// The master mnemonic must NEVER be reachable through the WC / injected
    /// `window.chia` dispatch. `wc_dispatch` is the single dapp-facing signer
    /// surface; with the wallet UNLOCKED (so the locked-gate isn't what stops it),
    /// every export-flavoured method name is rejected as unsupported (501) — never
    /// served. (Locked, it 401s first; either way no key material comes back.)
    #[tokio::test]
    async fn export_is_not_a_dispatchable_wc_method() {
        const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
        let st = AppState::default();
        // Unlock a session so the unlocked-gate is satisfied; now an unsupported
        // method can only fall through to the explicit 501 arm.
        *st.session.lock().await = Some(Session {
            mnemonic: Zeroizing::new(ABANDON.to_string()),
            address: "xch1test".to_string(),
        });
        for method in [
            "export",
            "exportMnemonic",
            "chip0002_export",
            "getMnemonic",
            "getSecretKeys",
            "chia_export",
            "revealSeed",
        ] {
            let r = wc_dispatch(&st, method, serde_json::Value::Null).await;
            match r {
                Err((code, _)) => assert_eq!(
                    code,
                    StatusCode::NOT_IMPLEMENTED,
                    "dapp-facing dispatch must reject {method} as unsupported"
                ),
                Ok(v) => panic!("{method} must not be dispatchable, got {v:?}"),
            }
        }
    }

    #[test]
    fn wc_dispatch_method_set_has_no_export_path() {
        // Guard the source of truth: the dapp-facing dispatcher must never RETURN
        // the recovery phrase or reach the export/decrypt path. (`mnemonic` as a
        // local signing secret is fine; what must not appear is returning it or
        // decrypting the seed.) If someone wires export into the dapp surface,
        // this fails.
        let src = include_str!("lib.rs");
        let dispatch = src
            .split("async fn wc_dispatch")
            .nth(1)
            .expect("wc_dispatch present")
            .split("\nasync fn ")
            .next()
            .unwrap();
        for forbidden in [
            "ExportResp",
            "/api/export",
            "decrypt_seed",
            "mnemonic.to_string()",
            "fn export",
        ] {
            assert!(
                !dispatch.contains(forbidden),
                "wc_dispatch must not reference {forbidden} (would leak the recovery phrase to dapps)"
            );
        }
    }

    // -- Export endpoint: self-origin + password gates -------------------------

    /// Build a HeaderMap carrying an Origin (the unspoofable dapp/page origin).
    fn origin_headers(origin: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        h
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    /// End-to-end of the export gate against a real on-disk encrypted seed:
    /// * a non-self origin is refused (403) even with the right password;
    /// * the self origin with a WRONG password is refused (401);
    /// * the self origin with the CORRECT password yields the exact mnemonic.
    ///
    /// Points the seed file at a throwaway tempdir via LOCALAPPDATA — no other
    /// dig-wallet test reads that env, so the process-global set is safe here.
    #[tokio::test]
    async fn export_requires_self_origin_and_correct_password() {
        // Public BIP-39 test vector (NOT a real wallet).
        const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
        const PW: &str = "correct horse battery";

        let td = tempfile::tempdir().unwrap();
        std::env::set_var("LOCALAPPDATA", td.path());
        // Encrypt + persist the seed exactly as `import` does.
        let enc = encrypt_seed(ABANDON, PW).unwrap();
        let path = seed_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, enc.to_bytes()).unwrap();

        let dapp = "https://evil.example.com";
        let self_origin = format!("http://127.0.0.1:{}", wallet_port());

        // A dapp origin is refused even with the correct password.
        let r = export(
            origin_headers(dapp),
            Json(ExportReq {
                password: PW.to_string(),
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);

        // Self origin + wrong password → 401, no mnemonic.
        let r = export(
            origin_headers(&self_origin),
            Json(ExportReq {
                password: "wrong".to_string(),
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

        // Self origin + correct password → the exact mnemonic.
        let r = export(
            origin_headers(&self_origin),
            Json(ExportReq {
                password: PW.to_string(),
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let body = body_json(r).await;
        assert_eq!(body["mnemonic"], ABANDON);

        std::env::remove_var("LOCALAPPDATA");
    }

    /// The projectId setter is wallet-local only: a dapp origin cannot change it.
    #[tokio::test]
    async fn wc_project_id_set_is_self_origin_only() {
        let r = wc_project_id_set(
            origin_headers("https://evil.example.com"),
            Json(SetWcProjectId {
                project_id: "hijacked".to_string(),
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }

    /// The public-key endpoint is wallet-local only and needs an unlocked wallet:
    /// a dapp origin is refused; the self origin with no session is unauthorized.
    #[tokio::test]
    async fn wallet_pubkey_is_self_origin_and_needs_unlock() {
        let st = Arc::new(AppState::default());
        // Dapp origin → forbidden.
        let r = wallet_pubkey(
            State(st.clone()),
            origin_headers("https://evil.example.com"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        // Self origin but locked → unauthorized.
        let self_origin = format!("http://localhost:{}", wallet_port());
        let r = wallet_pubkey(State(st), origin_headers(&self_origin)).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
}
