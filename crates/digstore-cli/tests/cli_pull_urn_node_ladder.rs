//! End-to-end proof that `digstore pull urn:…` (the NETWORK content-read path,
//! `commands::pull::pull_urn_resource`) actually reaches the `CLAUDE.md` §5.3
//! client->node ladder — via `--node` — rather than the old hard-coded
//! `rpc.dig.net` fallback, driven through the REAL installed-shape binary.
//!
//! This stands up a MINIMAL `dig.getContent` JSON-RPC responder (the wire
//! contract `digstore_remote::DigClient::get_content` speaks — see
//! `digstore-remote/src/client.rs`), independent of `digstore serve`'s own
//! `RemoteServer` (which does not implement this RPC — only `dig-node` and
//! `rpc.dig.net` do in production). It proves the WIRING end-to-end: `--node`
//! flows from the CLI flag through `ops::node::resolve_node` into the actual
//! HTTP request `pull_urn_resource` issues, and a real round trip decrypts to
//! the original plaintext.

mod common;
use common::{dig, tmp_dig};

use axum::{routing::post, Json, Router};
use base64::Engine;
use digstore_core::{Bytes32, Encode, MerkleTree};

async fn spawn_fake_dig_node(ciphertext: Vec<u8>, proof_b64: String, root_hex: String) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let app = Router::new()
        .route(
            "/health",
            axum::routing::get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        )
        .route(
            "/",
            post(move |Json(_req): Json<serde_json::Value>| {
                let ciphertext = ciphertext.clone();
                let proof_b64 = proof_b64.clone();
                let root_hex = root_hex.clone();
                async move {
                    let ct_b64 = base64::engine::general_purpose::STANDARD.encode(&ciphertext);
                    Json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "result": {
                            "ciphertext": ct_b64,
                            "inclusion_proof": proof_b64,
                            "chunk_lens": [ciphertext.len()],
                            "root": root_hex,
                            "complete": true,
                        }
                    }))
                }
            }),
        );

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// `digstore pull urn:dig:…/<resource> --node <fake-node>` fetches from the
/// EXPLICITLY-OVERRIDDEN node (never `rpc.dig.net`), verifies the merkle proof,
/// decrypts, and writes the original plaintext — proving `--node` really is
/// wired into the URN network-read path end-to-end.
#[test]
fn pull_urn_resource_uses_explicit_node_override() {
    let dir = tmp_dig();
    let store_id = Bytes32([0x42u8; 32]);
    let plaintext = b"hello from the resolved node";

    // The SAME root-independent canonical URN `client_crypto::derive_decryption_key`
    // reconstructs (root dropped) — the single source of truth for this shape is
    // `digstore_stage::canonical_resource_urn`.
    let rootless = digstore_stage::canonical_resource_urn(store_id, "doc");
    let key = digstore_crypto::derive_decryption_key(&rootless.canonical(), None);
    let ciphertext = digstore_crypto::encrypt_chunk(&key, plaintext);

    // Single-leaf merkle tree: its root becomes the "pinned" root the URN names,
    // so `client_crypto::verify_chunk_inclusion`'s trusted-root check passes.
    let leaf = digstore_crypto::sha256(&ciphertext);
    let tree = MerkleTree::from_leaves(vec![leaf]);
    let proof = tree.prove(0).expect("single-leaf proof");
    let root = proof.root;

    let mut urn = rootless;
    urn.root_hash = Some(root);

    let proof_b64 = base64::engine::general_purpose::STANDARD.encode(proof.to_bytes());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(spawn_fake_dig_node(ciphertext, proof_b64, root.to_hex()));

    // A URN network-read is store-independent in principle, but `pull` is
    // dispatched as a store-scoped command (`commands::mod::dispatch` resolves
    // the active store before routing), so an initialized workspace must exist
    // — same precondition `clone_then_cat_round_trips_from_remote` satisfies via
    // `clone`. `init` here is a throwaway local store; the resource actually
    // read comes entirely from the fake remote node via `--node`.
    dig(&dir).arg("init").assert().success();

    let out_path = dir.path().join("out.txt");
    dig(&dir)
        .args(["--node", &base])
        .args(["pull", &urn.canonical()])
        .args(["--out"])
        .arg(&out_path)
        .assert()
        .success();

    let written = std::fs::read(&out_path).unwrap();
    assert_eq!(written, plaintext);
}
