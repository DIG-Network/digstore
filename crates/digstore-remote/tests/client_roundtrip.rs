mod test_helpers;
use test_helpers::*;

use digstore_remote::{DigClient, InMemoryBackend, PullResult, PushResult, RemoteServer};
use std::sync::Arc;

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
        .clone_store(&id, |b, r| {
            if b.len() == 64 && *r == b32(0x10) {
                Ok(())
            } else {
                Err("size mismatch".into())
            }
        })
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
    let res = client.pull(&id, Some(b32(0x10)), false).await.unwrap();
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
    let res = client.pull(&id, Some(b32(0x10)), false).await.unwrap();
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
    let res = client.pull(&id, Some(b32(0x10)), true).await.unwrap();
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
        )
        .await;
    assert!(matches!(
        res,
        Err(digstore_remote::ClientError::NonFastForward)
    ));
}
