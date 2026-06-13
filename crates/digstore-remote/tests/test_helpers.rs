#![allow(dead_code)]
use axum::Router;
use digstore_core::{Bytes32, Bytes48, Bytes96};
use digstore_remote::{InMemoryBackend, RemoteServer};
use std::sync::Arc;

pub fn b32(x: u8) -> Bytes32 {
    Bytes32([x; 32])
}
pub fn b48(x: u8) -> Bytes48 {
    Bytes48([x; 48])
}
pub fn b96(x: u8) -> Bytes96 {
    Bytes96([x; 96])
}

/// A backend with one store registered at genesis root 0x10, pk 0x02,
/// 64-byte module. Returns (backend Arc, store id, store id hex).
pub fn one_store() -> (Arc<InMemoryBackend>, Bytes32, String) {
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(1);
    be.add_store(id, b48(2), b32(0x10), vec![0u8; 64], None);
    (be, id, id.to_hex())
}

pub fn router_for(be: Arc<InMemoryBackend>) -> Router {
    // These tests exercise the handler/protocol logic, not the §21.9 auth layer, so
    // they run an anonymous (open) server. Auth enforcement has its own tests.
    RemoteServer::new(be).allow_anonymous().router()
}
