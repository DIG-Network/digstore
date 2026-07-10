//! Proves `digstore cat urn:dig:…` resolves a full, self-contained URN via the
//! `CLAUDE.md` §5.3 client->node ladder WITHOUT requiring a local store (#227).
//!
//! Bug reproduced live: `digstore cat "chia://urn:dig:…/index.html"` failed
//! `NO_STORE` (exit 3, "run `digstore init` first") even though the URN alone
//! carries everything a network-only read needs (store id + pinned root). This
//! stands up a MINIMAL `dig.getContent` JSON-RPC responder (the same shape
//! `cli_pull_urn_node_ladder.rs` uses for `pull`) and drives the REAL installed-
//! shape binary with `--node` pointed at it — critically, WITHOUT ever running
//! `digstore init`, so no `.dig` workspace exists on disk at all.

mod common;
use common::{dig, tmp_dig};

use axum::{routing::post, Json, Router};
use base64::Engine;
use digstore_core::{Bytes32, Encode, MerkleTree};

/// A fake `dig.getContent` responder. When `gate_key` is set, it 404s
/// (`-32004`, the real "resource not available" code) for any retrieval key
/// other than `gate_key`, so a test can prove the caller retried a SECOND key
/// (the §8.5 default-then-empty fallback) rather than just echoing back
/// whatever the first request happened to carry.
async fn spawn_fake_dig_node(
    ciphertext: Vec<u8>,
    proof_b64: String,
    root_hex: String,
    gate_key: Option<String>,
) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let app = Router::new().route(
        "/",
        post(move |Json(req): Json<serde_json::Value>| {
            let ciphertext = ciphertext.clone();
            let proof_b64 = proof_b64.clone();
            let root_hex = root_hex.clone();
            let gate_key = gate_key.clone();
            async move {
                if let Some(gate_key) = &gate_key {
                    let sent = req["params"]["retrieval_key"].as_str().unwrap_or("");
                    if sent != gate_key {
                        return Json(serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "error": {
                                "code": -32004,
                                "message": "resource not available at the requested root",
                            }
                        }));
                    }
                }
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

/// Builds a single-leaf-merkle-verifiable ciphertext for `plaintext` under the
/// canonical (root-independent) URN for `(store_id, resource_key)`, returning
/// (ciphertext, proof_b64, root, retrieval_key_hex).
fn build_fixture(
    store_id: Bytes32,
    resource_key: &str,
    plaintext: &[u8],
) -> (Vec<u8>, String, Bytes32, String) {
    let rootless = digstore_stage::canonical_resource_urn(store_id, resource_key);
    let key = digstore_crypto::derive_decryption_key(&rootless.canonical(), None);
    let ciphertext = digstore_crypto::encrypt_chunk(&key, plaintext);

    let leaf = digstore_crypto::sha256(&ciphertext);
    let tree = MerkleTree::from_leaves(vec![leaf]);
    let proof = tree.prove(0).expect("single-leaf proof");
    let root = proof.root;
    let proof_b64 = base64::engine::general_purpose::STANDARD.encode(proof.to_bytes());
    let retrieval_key = rootless.retrieval_key().to_hex();
    (ciphertext, proof_b64, root, retrieval_key)
}

/// `digstore cat urn:dig:…/<resource> --node <fake-node>` succeeds with NO
/// local `.dig` store present at all (no `init` ever ran) — proving the
/// full-URN path resolves via the node ladder instead of demanding
/// `digstore init` first (#227).
#[test]
fn cat_full_urn_with_no_local_store_resolves_via_node_ladder() {
    let dir = tmp_dig();
    let store_id = Bytes32([0x77u8; 32]);
    let plaintext = b"hello from the node ladder, no local store needed";

    let (ciphertext, proof_b64, root, _rk) = build_fixture(store_id, "index.html", plaintext);
    let mut urn = digstore_stage::canonical_resource_urn(store_id, "index.html");
    urn.root_hash = Some(root);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(spawn_fake_dig_node(
        ciphertext,
        proof_b64,
        root.to_hex(),
        None,
    ));

    // Deliberately NO `dig(&dir).arg("init")` — the `.dig` workspace never exists on disk.
    let out = dig(&dir)
        .args(["--node", &base])
        .args(["cat", &urn.canonical()])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "cat of a full URN with no local store failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, plaintext);
}

/// Same as above but the URN carries NO resource key (§8.5 keyless URN): the
/// default landing view `index.html` must resolve over the network too, with
/// no local generation manifest available to consult.
#[test]
fn cat_keyless_full_urn_with_no_local_store_defaults_to_index_html() {
    let dir = tmp_dig();
    let store_id = Bytes32([0x78u8; 32]);
    let plaintext = b"<html>landing page, no local store</html>";

    let (ciphertext, proof_b64, root, _rk) = build_fixture(store_id, "index.html", plaintext);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(spawn_fake_dig_node(
        ciphertext,
        proof_b64,
        root.to_hex(),
        None,
    ));

    let keyless_urn = format!("urn:dig:chia:{}:{}", store_id.to_hex(), root.to_hex());
    let out = dig(&dir)
        .args(["--node", &base])
        .args(["cat", &keyless_urn])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "cat of a keyless full URN with no local store failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, plaintext);
}

/// A keyless URN whose store has NO `index.html` falls back to the store-level
/// empty resource key (§8.5), mirroring the local social-convention fallback,
/// even though there is no local manifest to consult — proven here by a fake
/// node that 404s the `index.html` retrieval key and only answers the EMPTY
/// key.
#[test]
fn cat_keyless_full_urn_falls_back_to_empty_key_when_index_html_missing() {
    let dir = tmp_dig();
    let store_id = Bytes32([0x79u8; 32]);
    let plaintext = b"store-level content, no index.html here";

    let (ciphertext, proof_b64, root, empty_rk) = build_fixture(store_id, "", plaintext);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(spawn_fake_dig_node(
        ciphertext,
        proof_b64,
        root.to_hex(),
        Some(empty_rk),
    ));

    let keyless_urn = format!("urn:dig:chia:{}:{}", store_id.to_hex(), root.to_hex());
    let out = dig(&dir)
        .args(["--node", &base])
        .args(["cat", &keyless_urn])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "cat keyless URN did not fall back to the empty resource key: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, plaintext);
}

/// A local store IS initialized, but the URN names a COMPLETELY DIFFERENT
/// store — `cat` must still resolve via the network rather than erroring,
/// proving the local/network routing decision is keyed on a STORE-ID MATCH,
/// not merely "does any local store exist" (#227).
#[test]
fn cat_full_urn_for_a_different_store_than_the_local_one_resolves_via_network() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();

    let other_store_id = Bytes32([0x7au8; 32]);
    let plaintext = b"content from a store I never initialized locally";
    let (ciphertext, proof_b64, root, _rk) = build_fixture(other_store_id, "doc", plaintext);
    let mut urn = digstore_stage::canonical_resource_urn(other_store_id, "doc");
    urn.root_hash = Some(root);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(spawn_fake_dig_node(
        ciphertext,
        proof_b64,
        root.to_hex(),
        None,
    ));

    let out = dig(&dir)
        .args(["--node", &base])
        .args(["cat", &urn.canonical()])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "cat of a foreign-store URN with a DIFFERENT local store failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, plaintext);
}
