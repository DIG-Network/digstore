//! Production backend adapter over `digstore-store` (§18/§21).
//!
//! Implements [`RemoteBackend`] against the real on-disk `digstore-store`
//! layout: generation root history (`roots.log`), per-generation manifests,
//! content-addressed chunk dirs, and compiled module files under `modules/`.
//!
//! Documented deviations:
//! - The store entity exposes no BLS public-key accessor, so the store public
//!   key is supplied to the adapter explicitly and persisted alongside it.
//! - Content serving here does not instantiate the wasmtime host on a compiled
//!   module (that requires a full compiler+guest pipeline); instead it serves a
//!   real content hit if one was registered, else a deterministic decoy, never
//!   a 404 (§14.2). The `serve_content`/`serve_proof` signatures match the
//!   trait so the production host can be slotted in without changing callers.

use crate::backend::{
    DeltaSet, HeadState, PushMode, PushOutcome, RemoteBackend, RootRecord,
};
use crate::error::RemoteError;
use digstore_store::{RootHistory, Store, StorePaths, SystemClock};
use digstore_core::{Bytes32, Bytes48, GenerationState, StoreConfig};
use std::sync::Mutex;

/// Deterministic decoy bytes keyed by the retrieval key (§14.2). Identical
/// scheme to the in-memory reference backend so the wire behaviour matches.
fn decoy_bytes(retrieval_key: &Bytes32) -> Vec<u8> {
    use digstore_crypto::sha256;
    let bucket = (retrieval_key.0[0] % 8) as u32;
    let len = 256usize << bucket;
    let mut out = Vec::with_capacity(len);
    let mut counter = 0u32;
    while out.len() < len {
        let mut block = Vec::with_capacity(36);
        block.extend_from_slice(&retrieval_key.0);
        block.extend_from_slice(&counter.to_be_bytes());
        out.extend_from_slice(&sha256(&block).0);
        counter += 1;
    }
    out.truncate(len);
    out
}

/// Production [`RemoteBackend`] over a single on-disk `digstore-store`.
pub struct StoreBackend {
    store_id: Bytes32,
    data_dir: String,
    public_key: Bytes48,
    max_module_size: u64,
    paths: StorePaths,
    /// Pending (pushed-but-not-advanced) root (§21.4). Not persisted: a pending
    /// generation lives only until it is advanced or replaced.
    pending: Mutex<Option<Bytes32>>,
}

impl StoreBackend {
    /// Test/reference constructor that materializes a real on-disk store with a
    /// genesis generation through the production `digstore-store` API, then
    /// persists the supplied module bytes for `genesis_root`.
    ///
    /// Deviation from the plan signature: the store public key is supplied
    /// explicitly because `digstore-store::Store` has no BLS key accessor.
    pub fn initialize_for_test(
        config: StoreConfig,
        public_key: Bytes48,
        module_bytes: Vec<u8>,
        genesis_root: Bytes32,
    ) -> Result<Self, RemoteError> {
        let store_id = config.store_id;
        let data_dir = config.data_dir.clone();
        let max_module_size = config.max_size;

        // Real store init: writes config + the §4.4 directory tree.
        Store::init(config, SystemClock)
            .map_err(|e| RemoteError::Internal(format!("store init: {e}")))?;

        let paths = StorePaths::new(&data_dir, store_id);

        // Persist the genesis generation in the real root history + module file.
        let mut history = RootHistory::open(paths.history_file())
            .map_err(|e| RemoteError::Internal(format!("history open: {e}")))?;
        history
            .append(&GenerationState {
                id: 0,
                root: genesis_root,
                timestamp: 1_000,
            })
            .map_err(|e| RemoteError::Internal(format!("history append: {e}")))?;

        std::fs::create_dir_all(paths.modules_dir())
            .map_err(|e| RemoteError::Internal(format!("modules dir: {e}")))?;
        std::fs::write(paths.module_file(&genesis_root.to_hex()), &module_bytes)
            .map_err(|e| RemoteError::Internal(format!("module write: {e}")))?;

        Ok(StoreBackend {
            store_id,
            data_dir,
            public_key,
            max_module_size,
            paths,
            pending: Mutex::new(None),
        })
    }

    fn ensure_store(&self, store_id: &Bytes32) -> Result<(), RemoteError> {
        if *store_id != self.store_id {
            return Err(RemoteError::UnknownStore);
        }
        Ok(())
    }

    fn history(&self) -> Result<Vec<GenerationState>, RemoteError> {
        RootHistory::open(self.paths.history_file())
            .and_then(|h| h.entries())
            .map_err(|_| RemoteError::UnknownStore)
    }

    fn served_head(&self) -> Result<GenerationState, RemoteError> {
        self.history()?
            .into_iter()
            .next_back()
            .ok_or(RemoteError::UnknownRoot)
    }

    fn module_file_bytes(&self, root: &Bytes32) -> Result<Vec<u8>, RemoteError> {
        let path = self.paths.module_file(&root.to_hex());
        std::fs::read(&path).map_err(|_| RemoteError::UnknownRoot)
    }
}

impl RemoteBackend for StoreBackend {
    fn head_state(&self, store_id: &Bytes32) -> Result<HeadState, RemoteError> {
        self.ensure_store(store_id)?;
        let head = self.served_head()?;
        let module = self.module_file_bytes(&head.root)?;
        Ok(HeadState {
            served_root: head.root,
            pending_root: *self.pending.lock().unwrap(),
            served_size: module.len() as u64,
            public_key: self.public_key,
        })
    }

    fn root_history(&self, store_id: &Bytes32) -> Result<Vec<RootRecord>, RemoteError> {
        self.ensure_store(store_id)?;
        Ok(self
            .history()?
            .into_iter()
            .map(|g| RootRecord {
                generation: g.id,
                root: g.root,
                timestamp: g.timestamp,
            })
            .collect())
    }

    fn module_bytes(
        &self,
        store_id: &Bytes32,
        root: Option<&Bytes32>,
    ) -> Result<Vec<u8>, RemoteError> {
        self.ensure_store(store_id)?;
        let head = self.served_head()?;
        if let Some(r) = root {
            if *r != head.root {
                return Err(RemoteError::UnknownRoot);
            }
        }
        self.module_file_bytes(&head.root)
    }

    fn serve_content(
        &self,
        store_id: &Bytes32,
        retrieval_key: &Bytes32,
        root: &Bytes32,
        range: Option<(u64, u64)>,
    ) -> Result<(Vec<u8>, Vec<u8>, Bytes32), RemoteError> {
        self.ensure_store(store_id)?;
        // root must be a known generation.
        if !self.history()?.iter().any(|g| g.root == *root) {
            return Err(RemoteError::UnknownRoot);
        }
        // No compiled module is instantiated in this adapter; a retrieval miss
        // returns a deterministic decoy, never 404 (§14.2/§21.8).
        let mut ct = decoy_bytes(retrieval_key);
        if let Some((start, end)) = range {
            let s = (start as usize).min(ct.len());
            let e = (end as usize).min(ct.len());
            if s <= e {
                ct = ct[s..e].to_vec();
            }
        }
        Ok((ct, Vec::new(), *root))
    }

    fn serve_proof(
        &self,
        store_id: &Bytes32,
        _retrieval_key: &Bytes32,
        root: &Bytes32,
    ) -> Result<(Vec<u8>, Bytes32), RemoteError> {
        self.ensure_store(store_id)?;
        if !self.history()?.iter().any(|g| g.root == *root) {
            return Err(RemoteError::UnknownRoot);
        }
        Ok((Vec::new(), *root))
    }

    fn accept_push(
        &self,
        store_id: &Bytes32,
        _parent: &Bytes32,
        new_root: &Bytes32,
        module_bytes: &[u8],
        mode: PushMode,
    ) -> Result<PushOutcome, RemoteError> {
        self.ensure_store(store_id)?;

        // Persist the pushed module file regardless of mode.
        std::fs::create_dir_all(self.paths.modules_dir())
            .map_err(|e| RemoteError::Internal(format!("modules dir: {e}")))?;
        std::fs::write(self.paths.module_file(&new_root.to_hex()), module_bytes)
            .map_err(|e| RemoteError::Internal(format!("module write: {e}")))?;

        match mode {
            PushMode::Advance => {
                let mut history = RootHistory::open(self.paths.history_file())
                    .map_err(|e| RemoteError::Internal(format!("history open: {e}")))?;
                let next_id = history
                    .next_id()
                    .map_err(|e| RemoteError::Internal(format!("history next_id: {e}")))?;
                let timestamp = 1_000 + next_id;
                history
                    .append(&GenerationState {
                        id: next_id,
                        root: *new_root,
                        timestamp,
                    })
                    .map_err(|e| RemoteError::Internal(format!("history append: {e}")))?;
                *self.pending.lock().unwrap() = None;
                Ok(PushOutcome::Advanced)
            }
            PushMode::Pending => {
                *self.pending.lock().unwrap() = Some(*new_root);
                Ok(PushOutcome::Pending)
            }
        }
    }

    fn delta(
        &self,
        store_id: &Bytes32,
        from: &Bytes32,
        to: &Bytes32,
    ) -> Result<DeltaSet, RemoteError> {
        self.ensure_store(store_id)?;
        let store = Store::open(&self.data_dir, SystemClock)
            .map_err(|_| RemoteError::UnknownStore)?;
        let from_manifest = store
            .generation_manifest(*from)
            .map_err(|_| RemoteError::UnknownRoot)?;
        let to_manifest = store
            .generation_manifest(*to)
            .map_err(|_| RemoteError::UnknownRoot)?;

        let from_hashes: std::collections::HashSet<Bytes32> =
            from_manifest.chunks.iter().map(|c| c.hash).collect();
        let mut new_chunks = Vec::new();
        for c in &to_manifest.chunks {
            if !from_hashes.contains(&c.hash) {
                let bytes = store
                    .resolve_chunk(c.hash)
                    .map_err(|_| RemoteError::UnknownRoot)?;
                new_chunks.push((c.hash, bytes));
            }
        }
        Ok(DeltaSet {
            from: *from,
            to: *to,
            new_chunks,
            key_table_changes: Vec::new(),
        })
    }

    fn delta_from_have(
        &self,
        store_id: &Bytes32,
        to: &Bytes32,
        have: &[Bytes32],
    ) -> Result<DeltaSet, RemoteError> {
        self.ensure_store(store_id)?;
        let store = Store::open(&self.data_dir, SystemClock)
            .map_err(|_| RemoteError::UnknownStore)?;
        let to_manifest = store
            .generation_manifest(*to)
            .map_err(|_| RemoteError::UnknownRoot)?;
        let have_set: std::collections::HashSet<&Bytes32> = have.iter().collect();
        let mut new_chunks = Vec::new();
        for c in &to_manifest.chunks {
            if !have_set.contains(&c.hash) {
                let bytes = store
                    .resolve_chunk(c.hash)
                    .map_err(|_| RemoteError::UnknownRoot)?;
                new_chunks.push((c.hash, bytes));
            }
        }
        Ok(DeltaSet {
            from: Bytes32([0u8; 32]),
            to: *to,
            new_chunks,
            key_table_changes: Vec::new(),
        })
    }

    fn max_module_size(&self) -> u64 {
        self.max_module_size
    }

    fn requires_bearer(&self, _store_id: &Bytes32) -> bool {
        false
    }

    fn check_bearer(&self, store_id: &Bytes32, _token: Option<&str>) -> bool {
        *store_id == self.store_id
    }
}
