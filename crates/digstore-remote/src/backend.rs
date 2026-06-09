use crate::error::RemoteError;
use digstore_core::{Bytes32, Bytes48, Bytes96};

/// The current head state of a store on the remote (§21.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadState {
    /// The confirmed generation the remote currently serves.
    pub served_root: Bytes32,
    /// A pushed-but-not-yet-advanced generation, if any (§21.4 pending).
    pub pending_root: Option<Bytes32>,
    /// Served module size in bytes.
    pub served_size: u64,
    /// Store BLS G1 public key.
    pub public_key: Bytes48,
    /// Publisher BLS signature over `SHA-256(served_root || store_id)` (§21.6),
    /// i.e. the push authorization for the currently served head. A client uses
    /// it to verify the served root was authorized by the store key (whose hash
    /// is the store id), not merely that the module is self-consistent. `None`
    /// for a head that was never push-signed (e.g. an unsigned seeded genesis).
    pub served_sig: Option<Bytes96>,
}

/// One entry in the linear root history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRecord {
    pub generation: u64,
    pub root: Bytes32,
    pub timestamp: u64,
}

/// Whether a push advances the served head or stays pending (§21.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushMode {
    /// Advance the served head immediately (=> 201).
    Advance,
    /// Accept into pending state, do not advance served head (=> 202).
    Pending,
}

/// Result of an accepted push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    /// New generation is now the served head (201).
    Advanced,
    /// Accepted into pending state, head unchanged (202).
    Pending,
}

/// The chunk-set difference between two generations along linear ancestry (§21.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaSet {
    pub from: Bytes32,
    pub to: Bytes32,
    /// (chunk hash, chunk bytes) for chunks in `to` not in `from`.
    pub new_chunks: Vec<(Bytes32, Vec<u8>)>,
    /// Custom-codec-encoded KeyTableEntry blobs changed between from..to.
    pub key_table_changes: Vec<Vec<u8>>,
}

/// Abstracts store storage + host serving for the remote (§18, §21).
/// Implementations: InMemoryBackend (tests/reference) and a production adapter
/// over digstore-store. All methods are synchronous; the server runs them
/// inside spawn_blocking because the host (wasmtime) is sync.
pub trait RemoteBackend: Send + Sync + 'static {
    /// Head state for a store, or UnknownStore.
    fn head_state(&self, store_id: &Bytes32) -> Result<HeadState, RemoteError>;

    /// Linear root history oldest→newest, or UnknownStore.
    fn root_history(&self, store_id: &Bytes32) -> Result<Vec<RootRecord>, RemoteError>;

    /// Raw module bytes for the served head, or UnknownStore/UnknownRoot.
    /// If `root` is Some, it must equal the served head root (else UnknownRoot).
    fn module_bytes(&self, store_id: &Bytes32, root: Option<&Bytes32>)
        -> Result<Vec<u8>, RemoteError>;

    /// Serve content for a retrieval key + root + optional range.
    /// MUST return Ok with a decoy on a retrieval miss (never 404 for content,
    /// §14.2/§21.8). Returns (ciphertext, encoded_merkle_proof, roothash).
    fn serve_content(
        &self,
        store_id: &Bytes32,
        retrieval_key: &Bytes32,
        root: &Bytes32,
        range: Option<(u64, u64)>,
    ) -> Result<(Vec<u8>, Vec<u8>, Bytes32), RemoteError>;

    /// Serve a proof for a retrieval key + root. Returns (encoded_proof, roothash).
    fn serve_proof(
        &self,
        store_id: &Bytes32,
        retrieval_key: &Bytes32,
        root: &Bytes32,
    ) -> Result<(Vec<u8>, Bytes32), RemoteError>;

    /// Accept a pushed module. The caller has ALREADY verified the BLS push
    /// signature and fast-forward eligibility; this only persists state.
    /// `parent` is the root the push claims to fast-forward from. `sig` is the
    /// verified publisher push signature over `SHA-256(new_root || store_id)`;
    /// it is persisted so a later clone/pull can re-verify head authorization
    /// (§21.6). `None` is accepted only for test/seed paths that bypass push auth.
    fn accept_push(
        &self,
        store_id: &Bytes32,
        parent: &Bytes32,
        new_root: &Bytes32,
        module_bytes: &[u8],
        sig: Option<&Bytes96>,
        mode: PushMode,
    ) -> Result<PushOutcome, RemoteError>;

    /// Compute the delta from `from` to `to` along linear ancestry (§21.5).
    fn delta(&self, store_id: &Bytes32, from: &Bytes32, to: &Bytes32)
        -> Result<DeltaSet, RemoteError>;

    /// Negotiated delta from a client have-summary (§21.5 POST /delta).
    fn delta_from_have(
        &self,
        store_id: &Bytes32,
        to: &Bytes32,
        have: &[Bytes32],
    ) -> Result<DeltaSet, RemoteError>;

    /// Maximum accepted module size in bytes (§21.8 413).
    fn max_module_size(&self) -> u64;

    /// Whether a bearer token is required for push (§21.6).
    fn requires_bearer(&self, store_id: &Bytes32) -> bool;

    /// Validate a presented bearer token for a store (transport-level, §21.6).
    fn check_bearer(&self, store_id: &Bytes32, token: Option<&str>) -> bool;
}
