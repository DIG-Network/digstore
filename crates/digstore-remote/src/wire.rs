use serde::{Deserialize, Serialize};

/// The DIG §21 remote-protocol wire version this crate implements, reported in the
/// `protocol` field of the [`RpcWellKnown`] discovery document.
pub const DIG_RPC_PROTOCOL_VERSION: &str = "1";

/// `GET /stores/{id}` — store descriptor (§21.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreDescriptor {
    /// Current served (confirmed) root, hex.
    pub current_root: String,
    /// Total served module size in bytes.
    pub size: u64,
    /// Store BLS G1 public key, 48-byte hex.
    pub public_key: String,
    /// Publisher push signature over `SHA-256(current_root || store_id)`, 96-byte
    /// hex (§21.6). Empty string if the served head was never push-signed. A
    /// client verifies this against the store-id-bound public key before trusting
    /// the served root (the "authenticated head" guarantee). `#[serde(default)]`
    /// keeps older servers' descriptors decodable.
    #[serde(default)]
    pub push_sig: String,
    /// Active signed revocation tombstones for this store (SECURITY.md residual
    /// #1 Layer 1). A client verifies each entry's signature against the
    /// store-id-bound module key and refuses to install/advance to a `Root`-revoked
    /// root (or refuses the whole store on a `Store` tombstone). `#[serde(default)]`
    /// keeps older servers' descriptors (which omit this field) decodable; an empty
    /// list means nothing is revoked.
    #[serde(default)]
    pub tombstones: Vec<TombstoneEntry>,
}

/// One signed revocation tombstone on the wire (SECURITY.md residual #1 Layer 1).
/// `record` is hex of the canonical `Tombstone` bytes
/// (`digstore_core::Tombstone::canonical`); `signature` is 96-byte hex of the
/// publisher's BLS signature over `tombstone_signing_message(record)`. Carrying
/// the canonical record (rather than exploded fields) keeps the signed preimage
/// unambiguous across client/server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TombstoneEntry {
    /// Hex of `Tombstone::canonical()`.
    pub record: String,
    /// 96-byte hex BLS signature over the canonical tombstone message.
    pub signature: String,
}

/// `POST /stores/{id}/tombstone` request body: a signed revocation tombstone the
/// remote verifies (against the store's published key) before persisting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TombstoneRequest {
    /// Hex of `Tombstone::canonical()`.
    pub record: String,
    /// 96-byte hex BLS signature over the canonical tombstone message.
    pub signature: String,
}

/// `GET /stores/{id}/roots` — linear root history, oldest→newest (§21.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootHistory {
    pub roots: Vec<RootEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootEntry {
    pub generation: u64,
    pub root: String,
    pub timestamp: u64,
}

/// `POST /stores/{id}/content` request body (§21.2): retrieval key + root + range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRequest {
    /// Retrieval key (SHA-256 of canonical URN), 32-byte hex.
    pub retrieval_key: String,
    /// Generation root to read against, 32-byte hex.
    pub root: String,
    /// Optional byte range [start,end) into the resource.
    pub range: Option<ByteRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

/// `POST /stores/{id}/content` response (§14.x shape; decoy identical on wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentEnvelope {
    /// base64(ciphertext bytes).
    pub ciphertext_b64: String,
    /// base64(custom-codec-encoded MerkleProof).
    pub merkle_proof_b64: String,
    /// 32-byte hex roothash the proof commits to.
    pub roothash: String,
}

/// `POST /stores/{id}/proof` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofRequest {
    pub retrieval_key: String,
    pub root: String,
}

/// `POST /stores/{id}/proof` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofEnvelope {
    /// base64(custom-codec-encoded ExecutionProof).
    pub proof_b64: String,
    pub roothash: String,
}

/// `GET /delta?from=&to=` / `POST /delta` response (§21.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaResponse {
    pub from: String,
    pub to: String,
    /// New chunks present in `to` and absent from `from`: hex hash -> base64 bytes.
    pub chunks: Vec<DeltaChunk>,
    /// Key-table entries changed/added between `from` and `to`.
    pub key_table_changes: Vec<KeyTableChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaChunk {
    pub hash: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTableChange {
    /// base64(custom-codec-encoded KeyTableEntry).
    pub entry_b64: String,
}

/// `POST /delta` request: client have-summary (§21.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaNegotiateRequest {
    pub to: String,
    /// Hex hashes of chunks the client already holds.
    pub have: Vec<String>,
}

/// `GET /.well-known/dig-rpc` — the RPC's identity discovery document.
///
/// An unauthenticated, unguarded well-known endpoint every DIG RPC (a `digstore
/// serve` node, a local dig-node, `rpc.dig.net`) exposes so a client can discover
/// the RPC's own §21.9 IDENTITY PUBLIC KEY — the pubkey the RPC signs its own §21
/// requests with when it acts as a client of an upstream store (its "origin
/// identity"). A store owner authorizes THIS pubkey as a writer delegate so the RPC
/// can advance the store's root on the owner's behalf (`digstore remote authorize`).
///
/// The document is public metadata (a pubkey is not a secret), so it is served
/// without §21.9 auth — a client must be able to fetch it BEFORE it can authenticate
/// (bootstrapping). It is byte-stable JSON so an agent can parse it deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcWellKnown {
    /// The RPC's identity BLS G1 public key, 48-byte (96-char) hex. This is the
    /// `<user>` a store owner authorizes as a writer delegate for their store, and
    /// the identity the RPC stamps in `X-Dig-Identity` when it signs §21 requests
    /// upstream. A client MUST treat a non-96-hex / non-BLS value as "no discoverable
    /// pubkey" and refuse to build an authorization spend for it.
    pub pubkey: String,
    /// The DIG §21 protocol version this RPC implements (the remote-protocol wire
    /// version, e.g. `"1"`). Advisory; a client may branch behavior on it.
    #[serde(default)]
    pub protocol: String,
    /// A free-form software identifier (e.g. `"digstore/0.9.0"` or `"rpc.dig.net"`).
    /// Advisory + diagnostic; NOT security-relevant. `#[serde(default)]` keeps an
    /// older document (which may omit it) decodable.
    #[serde(default)]
    pub software: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_json_round_trips() {
        let d = StoreDescriptor {
            current_root: "ab".repeat(32),
            size: 4096,
            public_key: "cd".repeat(48),
            push_sig: "ef".repeat(96),
            tombstones: vec![TombstoneEntry {
                record: "11".repeat(74),
                signature: "22".repeat(96),
            }],
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: StoreDescriptor = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn descriptor_without_tombstones_field_decodes_to_empty() {
        // Older servers omit the `tombstones` key; `#[serde(default)]` must keep
        // such a descriptor decodable, yielding an empty (no-revocation) set.
        let json = r#"{"current_root":"00","size":1,"public_key":"aa","push_sig":""}"#;
        let d: StoreDescriptor = serde_json::from_str(json).unwrap();
        assert!(d.tombstones.is_empty());
    }

    #[test]
    fn content_request_range_optional() {
        let no_range = ContentRequest {
            retrieval_key: "00".repeat(32),
            root: "11".repeat(32),
            range: None,
        };
        let s = serde_json::to_string(&no_range).unwrap();
        assert!(s.contains("\"range\":null"));
        let back: ContentRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(no_range, back);
    }

    /// **Proves:** the well-known document round-trips and its `pubkey` field is
    /// carried verbatim. **Catches:** a serde rename/shape drift that would break
    /// cross-impl discovery (rpc.dig.net ⇄ digstore ⇄ dig-node must all read it).
    #[test]
    fn rpc_well_known_round_trips() {
        let w = RpcWellKnown {
            pubkey: "ab".repeat(48),
            protocol: "1".into(),
            software: "digstore/test".into(),
        };
        let s = serde_json::to_string(&w).unwrap();
        assert!(s.contains("\"pubkey\":\"ababab"));
        let back: RpcWellKnown = serde_json::from_str(&s).unwrap();
        assert_eq!(w, back);
    }

    /// **Proves:** an older/minimal well-known document (only `pubkey`) decodes,
    /// with `protocol`/`software` defaulting to empty. **Catches:** a required-field
    /// decoder that would reject a lean rpc.dig.net document.
    #[test]
    fn rpc_well_known_minimal_decodes() {
        let json = r#"{"pubkey":"aa"}"#;
        let w: RpcWellKnown = serde_json::from_str(json).unwrap();
        assert_eq!(w.pubkey, "aa");
        assert!(w.protocol.is_empty());
        assert!(w.software.is_empty());
    }

    #[test]
    fn delta_response_round_trips() {
        let d = DeltaResponse {
            from: "00".repeat(32),
            to: "01".repeat(32),
            chunks: vec![DeltaChunk {
                hash: "aa".repeat(32),
                data_b64: "AAAA".into(),
            }],
            key_table_changes: vec![KeyTableChange {
                entry_b64: "BBBB".into(),
            }],
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: DeltaResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }
}
