//! Digstore HTTPS remote protocol (paper §21): axum server + reqwest client.
//!
//! Deviation note: the wire codec for store *content* is the Chia big-endian
//! streamable framing used everywhere in digstore-core; only the REST envelope
//! (descriptor/roots/delta metadata) is JSON for transport ergonomics. Content,
//! proof, and key-table blobs remain Chia custom-codec encoded and are merely
//! base64-wrapped for JSON transport.
//!
//! Push-signing deviation (CONVENTIONS C7): this crate does NOT define its own
//! push-signing message. It delegates to
//! `digstore_crypto::{push_signing_message, verify_push}` with argument order
//! `(root, store_id)` (message = `SHA-256(root || store_id)`), the single source
//! of truth shared with `digstore-cli`.
pub mod auth;
pub mod backend;
pub mod backend_inmem;
pub mod error;
pub mod etag;
pub mod handlers;
pub mod ratelimit;
pub mod server;
pub mod wire;

pub use auth::{push_signing_message, verify_push_signature, PushAuth};
pub use ratelimit::RateLimiter;
pub use server::{AppState, RemoteServer};

pub use backend::{
    DeltaSet, HeadState, PushMode, PushOutcome, RemoteBackend, RootRecord,
};
pub use backend_inmem::InMemoryBackend;
pub use error::{ClientError, RemoteError};
pub use etag::{etag_for_root, matches_current, parse_if_none_match};
