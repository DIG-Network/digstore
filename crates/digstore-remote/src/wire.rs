use serde::{Deserialize, Serialize};

/// `GET /stores/{id}` — store descriptor (§21.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreDescriptor {
    /// Current served (confirmed) root, hex.
    pub current_root: String,
    /// Total served module size in bytes.
    pub size: u64,
    /// Store BLS G1 public key, 48-byte hex.
    pub public_key: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_json_round_trips() {
        let d = StoreDescriptor {
            current_root: "ab".repeat(32),
            size: 4096,
            public_key: "cd".repeat(48),
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: StoreDescriptor = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
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
