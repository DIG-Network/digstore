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
            // The hub reads `resp.confirmed`; type null => XCH, type "cat" => the
            // DIG CAT (the only asset the native wallet tracks). Drives the
            // account-menu XCH balance and the DIG balance widget.
            let is_cat = params.get("type").and_then(|t| t.as_str()) == Some("cat");
            let chain = Coinset::mainnet();
            let scanned = scan_wallet(&chain, &mnemonic).await.map_err(|e| {
                wc_err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}"))
            })?;
            let confirmed = if is_cat {
                let asset = params
                    .get("assetId")
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .trim_start_matches("0x")
                    .to_ascii_lowercase();
                let dig = hex::encode(digstore_chain::dig::DIG_ASSET_ID);
                if asset.is_empty() || asset == dig {
                    scanned.dig_balance()
                } else {
                    return Err(wc_err(
                        StatusCode::BAD_REQUEST,
                        format!("unsupported asset id: {asset}"),
                    ));
                }
            } else {
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
            if is_cat {
                let asset = params
                    .get("assetId")
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .trim_start_matches("0x")
                    .to_ascii_lowercase();
                let dig = hex::encode(digstore_chain::dig::DIG_ASSET_ID);
                if !asset.is_empty() && asset != dig {
                    return Err(wc_err(
                        StatusCode::BAD_REQUEST,
                        format!("unsupported asset id: {asset}"),
                    ));
                }
            }
            let chain = Coinset::mainnet();
            let scanned = scan_wallet(&chain, &mnemonic).await.map_err(|e| {
                wc_err(StatusCode::BAD_GATEWAY, format!("coinset scan failed: {e}"))
            })?;
            let mut coins = Vec::new();
            for a in &scanned.addrs {
                if is_cat {
                    for c in &a.dig {
                        coins.push(coin_entry_json(c, None));
                    }
                } else {
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
    }

    #[test]
    fn settings_page_wires_the_cache_config_api() {
        // The served settings page must talk to the same config endpoints the
        // handlers expose, or the UI silently no-ops.
        assert!(SETTINGS_HTML.contains("/api/dig-config"));
        assert!(SETTINGS_HTML.contains("/api/dig-cache/clear"));
    }
}
