mod test_helpers;
use test_helpers::*;

use digstore_core::Bytes32;
use digstore_remote::{ClientError, DigClient, InMemoryBackend, PullResult, PushResult, RemoteServer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

async fn spawn_server(be: Arc<InMemoryBackend>) -> String {
    let app = RemoteServer::new(be).allow_anonymous().router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn fetch_returns_descriptor_and_roots() {
    let (be, id, _hex) = one_store();
    be.add_generation(
        &id,
        b32(0x10),
        b32(0x11),
        vec![0u8; 8],
        vec![],
        vec![],
        true,
    );
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let info = client.fetch(&id).await.unwrap();
    assert_eq!(info.descriptor.current_root, "11".repeat(32));
    assert_eq!(info.roots.roots.len(), 2);
}

#[tokio::test]
async fn clone_downloads_and_verifies_module() {
    let (be, id, _hex) = one_store();
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let (root, bytes) = client
        .clone_store(
            &id,
            |b, r| {
                if b.len() == 64 && *r == b32(0x10) {
                    Ok(())
                } else {
                    Err("size mismatch".into())
                }
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(root, b32(0x10));
    assert_eq!(bytes.len(), 64);
}

#[tokio::test]
async fn pull_up_to_date_when_local_equals_head() {
    let (be, id, _hex) = one_store();
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let res = client
        .pull(&id, Some(b32(0x10)), false, None)
        .await
        .unwrap();
    assert!(matches!(res, PullResult::UpToDate));
}

#[tokio::test]
async fn pull_downloads_module_when_behind() {
    let (be, id, _hex) = one_store();
    be.add_generation(
        &id,
        b32(0x10),
        b32(0x12),
        vec![0u8; 32],
        vec![],
        vec![],
        true,
    );
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let res = client
        .pull(&id, Some(b32(0x10)), false, None)
        .await
        .unwrap();
    match res {
        PullResult::Module { root, bytes } => {
            assert_eq!(root, b32(0x12));
            assert_eq!(bytes.len(), 32);
        }
        other => panic!("expected Module, got {other:?}"),
    }
}

#[tokio::test]
async fn pull_delta_path_returns_new_chunks() {
    let (be, id, _hex) = one_store();
    // Chunks are content-addressed: the client verifies SHA-256(data) == hash, so
    // the server's delta chunks must carry their real content ids.
    let c1 = vec![1u8];
    let c2 = vec![2u8];
    be.add_generation(
        &id,
        b32(0x10),
        b32(0x13),
        vec![0u8; 16],
        vec![
            (digstore_crypto::sha256(&c1), c1.clone()),
            (digstore_crypto::sha256(&c2), c2.clone()),
        ],
        vec![vec![5, 5]],
        true,
    );
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let res = client.pull(&id, Some(b32(0x10)), true, None).await.unwrap();
    match res {
        PullResult::Delta { root, delta } => {
            assert_eq!(root, b32(0x13));
            assert_eq!(delta.chunks.len(), 2);
        }
        other => panic!("expected Delta, got {other:?}"),
    }
}

#[tokio::test]
async fn push_signs_and_advances_head() {
    let (sk, pk) = digstore_crypto::bls_keygen(&[99u8; 32]);
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(7);
    be.add_store(id, pk, b32(0x10), vec![0u8; 8], None);
    let base = spawn_server(be.clone()).await;
    let client = DigClient::new(base);
    let new_root = b32(0x20);
    let res = client
        .push(
            &id,
            &b32(0x10),
            &new_root,
            &[1u8; 40],
            false,
            None,
            &pk.to_hex(),
            |msg| digstore_crypto::bls_sign(&sk, msg),
            None,
        )
        .await
        .unwrap();
    assert_eq!(res, PushResult::Advanced);
}

#[tokio::test]
async fn push_pending_returns_pending_and_pull_sees_confirmed_not_pending() {
    let (sk, pk) = digstore_crypto::bls_keygen(&[55u8; 32]);
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(8);
    be.add_store(id, pk, b32(0x10), vec![0u8; 8], None);
    let base = spawn_server(be.clone()).await;
    let client = DigClient::new(base);
    let pending_root = b32(0x20);
    let res = client
        .push(
            &id,
            &b32(0x10),
            &pending_root,
            &[1u8; 40],
            true,
            None,
            &pk.to_hex(),
            |msg| digstore_crypto::bls_sign(&sk, msg),
            None,
        )
        .await
        .unwrap();
    assert_eq!(res, PushResult::Pending, "(§21.4 202)");
    // pull must still see the confirmed (genesis) head, NOT the pending root.
    let info = client.fetch(&id).await.unwrap();
    assert_eq!(
        info.descriptor.current_root,
        "10".repeat(32),
        "served head still confirmed (§21.4)"
    );
}

#[tokio::test]
async fn push_non_fast_forward_is_client_error() {
    let (sk, pk) = digstore_crypto::bls_keygen(&[33u8; 32]);
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(9);
    be.add_store(id, pk, b32(0x10), vec![0u8; 8], None);
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let res = client
        .push(
            &id,
            &b32(0xEE),
            &b32(0x20),
            &[1u8; 8],
            false,
            None,
            &pk.to_hex(),
            |msg| digstore_crypto::bls_sign(&sk, msg),
            None,
        )
        .await;
    assert!(matches!(
        res,
        Err(digstore_remote::ClientError::NonFastForward)
    ));
}

// ---------------------------------------------------------------------------
// SPEC §4 — whole-module read root pinning (#1903)
// ---------------------------------------------------------------------------

/// A deliberately non-conforming `/stores/:id/module` route that always
/// answers `200` with `served_root`'s `ETag`, ignoring whatever `?root=` it was
/// asked for. No conforming `digstore serve`/gateway can produce this shape
/// (§4.4.6 forbids serving a rooted request under any ETag but the one it
/// named); this exists to prove the CLIENT's own pin check (§4.2.5), not
/// merely a well-behaved server's cooperation.
async fn spawn_wrong_etag_module_server(served_root: Bytes32, body: Vec<u8>) -> String {
    use axum::{body::Body, response::IntoResponse, routing::get, Router};
    let etag = format!("\"{}\"", served_root.to_hex());
    let app = Router::new().route(
        "/stores/:id/module",
        get(move || {
            let etag = etag.clone();
            let body = body.clone();
            async move {
                (
                    axum::http::StatusCode::OK,
                    [(axum::http::header::ETAG, etag)],
                    Body::from(body),
                )
                    .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A hand-rolled remote for `pull`-focused tests: `/stores/:id` and
/// `/stores/:id/roots` answer like a normal descriptor (so `pull`'s internal
/// `fetch` succeeds), but `/stores/:id/module` is deliberately non-conforming —
/// it always serves `served_etag_root`'s ETag regardless of the requested
/// `?root=`, and records the query string it was asked with into
/// `captured_module_query`. This lets one test assert BOTH that `pull` sends
/// `?root=<remote_root>` (§4.3.1) AND that `pull` refuses a server answering a
/// different root than it claims to (§4.3.3) — a shape no conforming server
/// can produce, so it proves `pull`'s own check rather than a happy path any
/// well-behaved server would also satisfy.
async fn spawn_pull_probe_server(
    descriptor_root: Bytes32,
    served_etag_root: Bytes32,
    module_body: Vec<u8>,
    captured_module_query: Arc<Mutex<Option<String>>>,
) -> String {
    use axum::{
        body::Body,
        extract::{OriginalUri, Path},
        response::IntoResponse,
        routing::get,
        Json, Router,
    };

    let desc_root_hex = descriptor_root.to_hex();
    let descriptor = move |Path(_id): Path<String>| {
        let current_root = desc_root_hex.clone();
        async move {
            Json(serde_json::json!({
                "current_root": current_root,
                "size": 0,
                "public_key": "00".repeat(48),
                "push_sig": "",
                "tombstones": [],
            }))
        }
    };
    let roots =
        |Path(_id): Path<String>| async move { Json(serde_json::json!({ "roots": [] })) };
    let etag = format!("\"{}\"", served_etag_root.to_hex());
    let module = move |Path(_id): Path<String>, uri: OriginalUri| {
        let etag = etag.clone();
        let body = module_body.clone();
        let captured = captured_module_query.clone();
        async move {
            *captured.lock().unwrap() = uri.0.query().map(|q| q.to_string());
            (
                axum::http::StatusCode::OK,
                [(axum::http::header::ETAG, etag)],
                Body::from(body),
            )
                .into_response()
        }
    };

    let app = Router::new()
        .route("/stores/:id", get(descriptor))
        .route("/stores/:id/roots", get(roots))
        .route("/stores/:id/module", get(module));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn clone_store_at_pins_to_the_served_head() {
    let (be, id, _hex) = one_store();
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let (root, bytes) = client
        .clone_store_at(
            &id,
            Some(&b32(0x10)),
            |b, r| {
                if b.len() == 64 && *r == b32(0x10) {
                    Ok(())
                } else {
                    Err("size mismatch".into())
                }
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(root, b32(0x10));
    assert_eq!(bytes.len(), 64);
}

#[tokio::test]
async fn clone_store_at_of_a_held_non_head_generation_is_404() {
    // The old genesis root is still HELD (InMemoryBackend keeps every
    // generation) but no longer SERVED once 0x12 advances the head — §4.4.4
    // says a `digstore serve` remote refuses even a generation it still holds.
    let (be, id, _hex) = one_store();
    be.add_generation(
        &id,
        b32(0x10),
        b32(0x12),
        vec![0u8; 8],
        vec![],
        vec![],
        true,
    );
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let result = client
        .clone_store_at(&id, Some(&b32(0x10)), |_, _| Ok(()), None)
        .await;
    assert!(
        matches!(result, Err(ClientError::Status(404))),
        "got {result:?}"
    );
}

#[tokio::test]
async fn clone_store_at_of_a_never_existed_root_is_404() {
    let (be, id, _hex) = one_store();
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let result = client
        .clone_store_at(&id, Some(&b32(0x99)), |_, _| Ok(()), None)
        .await;
    assert!(
        matches!(result, Err(ClientError::Status(404))),
        "got {result:?}"
    );
}

#[tokio::test]
async fn clone_store_at_refuses_a_served_root_that_disagrees_with_the_pin() {
    let requested_root = b32(0x10);
    let served_wrong_root = b32(0xAA); // what the misbehaving server actually serves
    let base = spawn_wrong_etag_module_server(served_wrong_root, vec![9u8; 4]).await;
    let client = DigClient::new(base);

    let verify_called = Arc::new(AtomicBool::new(false));
    let vc = verify_called.clone();
    let progress_called = Arc::new(AtomicBool::new(false));
    let pc = progress_called.clone();
    let on_progress = move |_done: u64, _total: u64| {
        pc.store(true, Ordering::SeqCst);
    };

    let result = client
        .clone_store_at(
            &b32(1),
            Some(&requested_root),
            move |_bytes: &[u8], _root: &Bytes32| -> Result<(), String> {
                vc.store(true, Ordering::SeqCst);
                Ok(())
            },
            Some(&on_progress),
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Verification(_))),
        "got {result:?}"
    );
    assert!(
        !verify_called.load(Ordering::SeqCst),
        "verify must not run on a pin mismatch"
    );
    assert!(
        !progress_called.load(Ordering::SeqCst),
        "on_progress must not run on a pin mismatch"
    );
}

#[tokio::test]
async fn pull_full_module_get_is_pinned_to_the_remote_head_and_refuses_a_wrong_etag() {
    let remote_head = b32(0x42);
    let wrong_served_root = b32(0x99); // the stub server's misbehavior
    let captured_query = Arc::new(Mutex::new(None));
    let base = spawn_pull_probe_server(
        remote_head,
        wrong_served_root,
        vec![1u8; 4],
        captured_query.clone(),
    )
    .await;
    let client = DigClient::new(base);

    let result = client.pull(&b32(1), Some(b32(0x01)), false, None).await;

    assert!(
        matches!(result, Err(ClientError::Verification(_))),
        "got {result:?}"
    );
    let query = captured_query.lock().unwrap().clone();
    assert_eq!(
        query,
        Some(format!("root={}", remote_head.to_hex())),
        "pull's full-module GET must carry ?root=<remote head> (SPEC §4.3.1)"
    );
}

#[tokio::test]
async fn rooted_get_of_a_held_non_head_generation_is_404() {
    let (be, id, hex) = one_store();
    be.add_generation(
        &id,
        b32(0x10),
        b32(0x12),
        vec![0u8; 8],
        vec![],
        vec![],
        true,
    );
    let base = spawn_server(be).await;
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{base}/stores/{hex}/module?root={}", b32(0x10).to_hex()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

#[tokio::test]
async fn rooted_get_of_a_never_existed_root_is_404() {
    let (be, _id, hex) = one_store();
    let base = spawn_server(be).await;
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{base}/stores/{hex}/module?root={}", b32(0x99).to_hex()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

#[tokio::test]
async fn malformed_root_is_422_even_for_an_unknown_store() {
    // A store id that was never registered on this backend at all — proves
    // row 2 (malformed root) precedes row 3 (unknown store) in SPEC §4.4.2:
    // a malformed root never reaches the store lookup.
    let unknown_hex = b32(0xEE).to_hex();
    let (be, _id, _hex) = one_store();
    let base = spawn_server(be).await;
    let http = reqwest::Client::new();
    for bad in ["zz", "", "abcd"] {
        let resp = http
            .get(format!("{base}/stores/{unknown_hex}/module?root={bad}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status().as_u16(),
            422,
            "root={bad:?} against an unknown store must still 422"
        );
    }
}

#[tokio::test]
async fn head_rooted_at_the_served_head_is_200_with_etag() {
    let (be, _id, hex) = one_store();
    let base = spawn_server(be).await;
    let http = reqwest::Client::new();
    let resp = http
        .head(format!("{base}/stores/{hex}/module?root={}", b32(0x10).to_hex()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(etag, format!("\"{}\"", b32(0x10).to_hex()));
}

#[tokio::test]
async fn head_rooted_at_a_non_head_root_is_404() {
    let (be, id, hex) = one_store();
    be.add_generation(
        &id,
        b32(0x10),
        b32(0x12),
        vec![0u8; 8],
        vec![],
        vec![],
        true,
    );
    let base = spawn_server(be).await;
    let http = reqwest::Client::new();
    let resp = http
        .head(format!("{base}/stores/{hex}/module?root={}", b32(0x10).to_hex()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}
