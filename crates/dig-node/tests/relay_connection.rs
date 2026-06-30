//! Integration test: the node's persistent relay connection registers with a relay and reports
//! connected.
//!
//! A mock relay (a tiny WebSocket server) accepts the node's `Register` (RLY-001) and replies with
//! a `RegisterAck { success: true }`. We run `dig_node::relay::run_relay_connection` against it and
//! assert the shared `RelayStatus` flips to connected — proving the node holds a reservation so a
//! NAT'd peer stays reachable. This exercises the real connect → register → ack path end-to-end.

use std::time::Duration;

use dig_node::relay::{run_relay_connection, RelayMessage, RelayStatus, DEFAULT_NETWORK_ID};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// Start a mock relay that, for each connection, reads the first `Register` and replies
/// `register_ack{success:true}`. Returns the `ws://` URL it listens on.
async fn start_mock_relay() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let (mut write, mut read) = ws.split();
                while let Some(Ok(frame)) = read.next().await {
                    let bytes = match frame {
                        Message::Text(t) => t.into_bytes(),
                        Message::Binary(b) => b,
                        Message::Close(_) => break,
                        _ => continue,
                    };
                    if let Ok(RelayMessage::Register { .. }) =
                        serde_json::from_slice::<RelayMessage>(&bytes)
                    {
                        let ack = RelayMessage::RegisterAck {
                            success: true,
                            message: "registered".into(),
                            connected_peers: 1,
                        };
                        let _ = write
                            .send(Message::Text(serde_json::to_string(&ack).unwrap()))
                            .await;
                    }
                    // Ignore subsequent pings/etc.; keep the connection open.
                }
            });
        }
    });
    format!("ws://{addr}")
}

#[tokio::test]
async fn node_registers_with_relay_and_reports_connected() {
    let url = start_mock_relay().await;
    let status = RelayStatus::new();
    let status2 = status.clone();

    // The relay loop runs forever (reconnecting); run it in the background and poll the status.
    let task = tokio::spawn(run_relay_connection(
        url,
        "test-peer-id".to_string(),
        DEFAULT_NETWORK_ID.to_string(),
        status2,
    ));

    // Wait up to ~3s for the reservation to establish.
    let mut connected = false;
    for _ in 0..30 {
        if status.is_connected() {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    task.abort();
    assert!(
        connected,
        "node should establish a relay reservation (RegisterAck → connected)"
    );

    let snap = status.snapshot_json("ws://relay", "test-peer-id");
    assert_eq!(snap["connected"], true);
    assert_eq!(snap["connected_peers"], 1);
}

#[tokio::test]
async fn unreachable_relay_records_error_and_keeps_retrying() {
    // A TCP server that accepts then IMMEDIATELY drops every connection → the WebSocket handshake
    // fails fast and deterministically (no reliance on a refused/unroutable port's OS timing). The
    // loop must record the error + bump the attempt counter and keep retrying (not panic/exit).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        // Accept then immediately drop every connection → the WebSocket handshake fails.
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });

    let status = RelayStatus::new();
    let status2 = status.clone();
    let url = format!("ws://{addr}");
    let task = tokio::spawn(run_relay_connection(
        url.clone(),
        "p".to_string(),
        DEFAULT_NETWORK_ID.to_string(),
        status2,
    ));

    // Wait for at least one recorded failure (the handshake against a dropped socket fails quickly).
    let mut failed = false;
    for _ in 0..30 {
        if status.snapshot_json(&url, "p")["reconnect_attempts"]
            .as_u64()
            .unwrap_or(0)
            >= 1
        {
            failed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    task.abort();

    assert!(failed, "a failed connect bumps the attempt counter");
    let snap = status.snapshot_json(&url, "p");
    assert_eq!(snap["connected"], false);
    assert!(snap["last_error"].is_string(), "the failure is recorded");
}
