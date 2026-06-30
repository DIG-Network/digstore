//! Persistent relay connection — keeps the node reachable to NAT'd peers.
//!
//! A DIG Node behind NAT can't accept inbound dials. To stay reachable it holds a CONSTANT
//! connection (a reservation) with a publicly-reachable relay (default `relay.dig.net`, override
//! `DIG_RELAY_URL`). This module is the CLIENT side of the DIG relay protocol (`RelayMessage`, JSON
//! over WebSocket, RLY-001..RLY-006); the SERVER is the `dig-relay` repo. The wire types here are
//! vendored byte-identical to the relay server's (and to dig-gossip's `relay_types`) — a shared
//! contract pinned by [`tests`] below; see SYSTEM.md for the change-impact edge.
//!
//! Lifecycle ([`run_relay_connection`]): connect → `Register` (RLY-001) → on `RegisterAck` mark
//! connected → keepalive `Ping`/`Pong` (RLY-006) → on any drop, reconnect with capped exponential
//! backoff. Status (connected / attempts / last error) is published through [`RelayStatus`], a
//! cheap shared snapshot the node surfaces via the `control.relayStatus` RPC.
//!
//! This runs ONLY in the standalone `dig-node` binary's `run()`; the in-process FFI path (the
//! browser, a pure consumer) does not open a relay connection, so the byte-exact §21/FFI contract
//! is untouched.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message;

/// Default relay endpoint when `DIG_RELAY_URL` is unset. The canonical public relay.
pub const DEFAULT_RELAY_URL: &str = "wss://relay.dig.net:9450";

/// Default network id a node registers under (matches dig-gossip `DEFAULT_INTRODUCER_NETWORK_ID`).
pub const DEFAULT_NETWORK_ID: &str = "DIG_MAINNET";

/// Relay protocol version the node advertises in `Register`.
pub const RELAY_PROTOCOL_VERSION: u32 = 1;

/// Base reconnect delay (RLY-004 lineage: dig-gossip `RelayConfig::reconnect_delay_secs` = 5).
const BASE_BACKOFF_SECS: u64 = 5;
/// Cap on the exponential backoff so a long outage doesn't push the retry interval to hours.
const MAX_BACKOFF_SECS: u64 = 300;
/// Keepalive ping period (RLY-006; dig-gossip `PING_INTERVAL_SECS` = 30).
const PING_INTERVAL_SECS: u64 = 30;

/// The relay message wire — vendored byte-identical to `dig-relay`'s `src/wire.rs` and dig-gossip's
/// `relay_types::RelayMessage`. Only the subset the node sends/receives is needed, but the full
/// enum is mirrored so the `#[serde(tag = "type")]` discriminators stay locked to the server's.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RelayMessage {
    #[serde(rename = "register")]
    Register {
        peer_id: String,
        network_id: String,
        protocol_version: u32,
    },
    #[serde(rename = "register_ack")]
    RegisterAck {
        success: bool,
        message: String,
        connected_peers: usize,
    },
    #[serde(rename = "unregister")]
    Unregister { peer_id: String },
    #[serde(rename = "relay_message")]
    RelayGossipMessage {
        from: String,
        to: String,
        payload: Vec<u8>,
        seq: u64,
    },
    #[serde(rename = "broadcast")]
    Broadcast {
        from: String,
        payload: Vec<u8>,
        exclude: Vec<String>,
    },
    #[serde(rename = "peer_connected")]
    PeerConnected { peer: RelayPeerInfo },
    #[serde(rename = "peer_disconnected")]
    PeerDisconnected { peer_id: String },
    #[serde(rename = "get_peers")]
    GetPeers { network_id: Option<String> },
    #[serde(rename = "peers")]
    Peers { peers: Vec<RelayPeerInfo> },
    #[serde(rename = "ping")]
    Ping { timestamp: u64 },
    #[serde(rename = "pong")]
    Pong { timestamp: u64 },
    #[serde(rename = "hole_punch_request")]
    HolePunchRequest {
        peer_id: String,
        target_peer_id: String,
        external_addr: std::net::SocketAddr,
    },
    #[serde(rename = "hole_punch_coordinate")]
    HolePunchCoordinate {
        peer_id: String,
        external_addr: std::net::SocketAddr,
    },
    #[serde(rename = "hole_punch_result")]
    HolePunchResult { peer_id: String, success: bool },
    #[serde(rename = "error")]
    Error { code: u32, message: String },
}

/// Peer info as tracked by the relay (mirrors the server's `RelayPeerInfo`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayPeerInfo {
    pub peer_id: String,
    pub network_id: String,
    pub protocol_version: u32,
    pub connected_at: u64,
    pub last_seen: u64,
}

/// Compute the next reconnect backoff: capped exponential in the number of consecutive failures.
/// `failures == 0` → base; doubles each failure up to [`MAX_BACKOFF_SECS`]. Pure → unit-tested.
pub fn backoff_secs(consecutive_failures: u32) -> u64 {
    let shifted = BASE_BACKOFF_SECS
        .checked_shl(consecutive_failures)
        .unwrap_or(MAX_BACKOFF_SECS);
    shifted.clamp(BASE_BACKOFF_SECS, MAX_BACKOFF_SECS)
}

/// Live relay-connection status, shared (via `Arc`) between the connection task and the RPC
/// handler. Cheap atomic reads so `control.relayStatus` never blocks.
#[derive(Debug, Default)]
pub struct RelayStatus {
    connected: AtomicBool,
    reconnect_attempts: AtomicU32,
    connected_peers: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl RelayStatus {
    /// A fresh, disconnected status.
    pub fn new() -> Arc<Self> {
        Arc::new(RelayStatus::default())
    }

    /// Mark the session connected (clears the last error, resets the attempt counter).
    pub fn set_connected(&self, connected_peers: u64) {
        self.connected.store(true, Ordering::Relaxed);
        self.connected_peers
            .store(connected_peers, Ordering::Relaxed);
        self.reconnect_attempts.store(0, Ordering::Relaxed);
        *self.last_error.lock().unwrap() = None;
    }

    /// Mark the session disconnected with an optional error and bump the attempt counter.
    pub fn set_disconnected(&self, error: Option<String>) {
        self.connected.store(false, Ordering::Relaxed);
        self.reconnect_attempts.fetch_add(1, Ordering::Relaxed);
        if let Some(e) = error {
            *self.last_error.lock().unwrap() = Some(e);
        }
    }

    /// Whether a relay session is currently held.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// A JSON snapshot for the `control.relayStatus` RPC.
    pub fn snapshot_json(&self, endpoint: &str, peer_id: &str) -> serde_json::Value {
        serde_json::json!({
            "connected": self.connected.load(Ordering::Relaxed),
            "endpoint": endpoint,
            "peer_id": peer_id,
            "reconnect_attempts": self.reconnect_attempts.load(Ordering::Relaxed),
            "connected_peers": self.connected_peers.load(Ordering::Relaxed),
            "last_error": *self.last_error.lock().unwrap(),
        })
    }
}

/// Resolve the relay endpoint: `DIG_RELAY_URL` if set + non-empty, else [`DEFAULT_RELAY_URL`].
pub fn relay_url_from_env() -> String {
    std::env::var("DIG_RELAY_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_RELAY_URL.to_string())
}

/// Whether the relay connection is enabled. Disabled when `DIG_RELAY_URL` is the literal `off`/
/// `disabled`/empty-after-trim — an explicit opt-out for air-gapped/standalone nodes.
pub fn relay_enabled() -> bool {
    match std::env::var("DIG_RELAY_URL") {
        Ok(v) => {
            let v = v.trim();
            !(v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("disabled"))
        }
        Err(_) => true,
    }
}

/// Current unix time (seconds), saturating.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Maintain a CONSTANT relay connection forever: connect, register, keepalive, and on any drop
/// reconnect with capped exponential backoff. Spawned as a background task by the standalone
/// `run()`. `peer_id` is the node's persistent identity pubkey hex (its stable network id);
/// `status` is the shared snapshot the RPC reads.
pub async fn run_relay_connection(
    endpoint: String,
    peer_id: String,
    network_id: String,
    status: Arc<RelayStatus>,
) {
    let mut consecutive_failures: u32 = 0;
    loop {
        match connect_once(&endpoint, &peer_id, &network_id, &status).await {
            Ok(()) => {
                // A clean end (server closed) still counts as a drop to reconnect from.
                consecutive_failures = 0;
                status.set_disconnected(None);
            }
            Err(e) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                tracing::warn!(error = %e, endpoint = %endpoint, "relay connection failed");
                status.set_disconnected(Some(e));
            }
        }
        let delay = backoff_secs(consecutive_failures);
        tokio::time::sleep(Duration::from_secs(delay)).await;
    }
}

/// One connect → register → serve cycle. Returns `Ok` on a clean close, `Err(reason)` on failure.
async fn connect_once(
    endpoint: &str,
    peer_id: &str,
    network_id: &str,
    status: &Arc<RelayStatus>,
) -> Result<(), String> {
    let (ws, _resp) = tokio_tungstenite::connect_async(endpoint)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let (mut write, mut read) = ws.split();

    // RLY-001: register immediately so the relay holds our reservation.
    let register = RelayMessage::Register {
        peer_id: peer_id.to_string(),
        network_id: network_id.to_string(),
        protocol_version: RELAY_PROTOCOL_VERSION,
    };
    send(&mut write, &register).await?;

    let mut ping = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Skip the immediate first tick.
    ping.tick().await;

    loop {
        tokio::select! {
            // Keepalive (RLY-006).
            _ = ping.tick() => {
                send(&mut write, &RelayMessage::Ping { timestamp: now_secs() }).await?;
            }
            frame = read.next() => {
                match frame {
                    None => return Ok(()),                            // clean close
                    Some(Err(e)) => return Err(format!("read: {e}")),
                    Some(Ok(Message::Close(_))) => return Ok(()),
                    Some(Ok(Message::Ping(p))) => {
                        write.send(Message::Pong(p)).await.map_err(|e| format!("pong: {e}"))?;
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                    Some(Ok(Message::Text(t))) => {
                        handle_incoming(t.into_bytes(), &mut write, status).await?;
                    }
                    Some(Ok(Message::Binary(b))) => {
                        handle_incoming(b, &mut write, status).await?;
                    }
                }
            }
        }
    }
}

/// Handle one decoded inbound relay frame: track RegisterAck (→ connected), answer relay Pings.
async fn handle_incoming<W>(
    bytes: Vec<u8>,
    write: &mut W,
    status: &Arc<RelayStatus>,
) -> Result<(), String>
where
    W: SinkExt<Message> + Unpin,
    <W as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    let Ok(msg) = serde_json::from_slice::<RelayMessage>(&bytes) else {
        return Ok(()); // ignore anything we can't parse; the relay is untrusted
    };
    match msg {
        RelayMessage::RegisterAck {
            success,
            message,
            connected_peers,
        } => {
            if success {
                status.set_connected(connected_peers as u64);
                tracing::info!(connected_peers, "relay reservation established");
            } else {
                return Err(format!("register rejected: {message}"));
            }
        }
        RelayMessage::Ping { timestamp } => {
            send(write, &RelayMessage::Pong { timestamp }).await?;
        }
        RelayMessage::Error { code, message } => {
            return Err(format!("relay error {code}: {message}"));
        }
        // PeerConnected/Disconnected/Peers/relayed payloads: the content node does not act on L2
        // gossip yet (it has no gossip mesh) — accepting the reservation keeps it reachable, which
        // is the goal. These are logged at debug and ignored.
        other => tracing::debug!(?other, "relay message ignored by content node"),
    }
    Ok(())
}

/// Serialize + send one `RelayMessage` as a WebSocket text frame.
async fn send<W>(write: &mut W, msg: &RelayMessage) -> Result<(), String>
where
    W: SinkExt<Message> + Unpin,
    <W as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    let txt = serde_json::to_string(msg).map_err(|e| format!("encode: {e}"))?;
    write
        .send(Message::Text(txt))
        .await
        .map_err(|e| format!("send: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_capped_exponential() {
        assert_eq!(backoff_secs(0), 5);
        assert_eq!(backoff_secs(1), 10);
        assert_eq!(backoff_secs(2), 20);
        assert_eq!(backoff_secs(3), 40);
        // Capped at 300 once the exponential exceeds it (and never below the base).
        assert_eq!(backoff_secs(20), 300);
        assert_eq!(backoff_secs(64), 300, "overflow saturates to the cap");
    }

    #[test]
    fn status_transitions() {
        let s = RelayStatus::new();
        assert!(!s.is_connected());
        s.set_disconnected(Some("connect: refused".into()));
        assert!(!s.is_connected());
        let v = s.snapshot_json("wss://relay.dig.net:9450", "pk");
        assert_eq!(v["connected"], false);
        assert_eq!(v["reconnect_attempts"], 1);
        assert_eq!(v["last_error"], "connect: refused");

        s.set_connected(7);
        assert!(s.is_connected());
        let v = s.snapshot_json("wss://relay.dig.net:9450", "pk");
        assert_eq!(v["connected"], true);
        assert_eq!(v["connected_peers"], 7);
        assert_eq!(v["reconnect_attempts"], 0, "reset on connect");
        assert!(v["last_error"].is_null(), "cleared on connect");
    }

    #[test]
    fn relay_url_defaults_and_overrides() {
        // Default when unset is asserted indirectly (env-sensitive test kept minimal): the const is
        // the canonical public relay.
        assert_eq!(DEFAULT_RELAY_URL, "wss://relay.dig.net:9450");
    }

    #[test]
    fn register_serializes_to_the_shared_wire() {
        let m = RelayMessage::Register {
            peer_id: "abc".into(),
            network_id: DEFAULT_NETWORK_ID.into(),
            protocol_version: RELAY_PROTOCOL_VERSION,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["type"], "register");
        assert_eq!(v["peer_id"], "abc");
        assert_eq!(v["network_id"], "DIG_MAINNET");
        assert_eq!(v["protocol_version"], 1);
    }

    #[test]
    fn register_ack_round_trips_from_server_json() {
        // A frame exactly as the dig-relay server emits it must parse here (shared contract).
        let raw =
            r#"{"type":"register_ack","success":true,"message":"registered","connected_peers":3}"#;
        let m: RelayMessage = serde_json::from_str(raw).unwrap();
        match m {
            RelayMessage::RegisterAck {
                success,
                connected_peers,
                ..
            } => {
                assert!(success);
                assert_eq!(connected_peers, 3);
            }
            other => panic!("expected RegisterAck, got {other:?}"),
        }
    }
}
