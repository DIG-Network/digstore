use crate::backend::{DeltaSet, HeadState, PushMode, PushOutcome, RemoteBackend, RootRecord};
use crate::error::RemoteError;
use digstore_core::{Bytes32, Bytes48, Bytes96};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone)]
struct Generation {
    #[allow(dead_code)]
    parent: Option<Bytes32>,
    generation_no: u64,
    timestamp: u64,
    module: Vec<u8>,
    /// chunk hash -> chunk bytes present in this generation.
    chunks: HashMap<Bytes32, Vec<u8>>,
    /// encoded KeyTableEntry blobs introduced/changed at this generation.
    key_table_changes: Vec<Vec<u8>>,
    /// retrieval_key -> (ciphertext, encoded merkle proof) for real hits.
    content: HashMap<Bytes32, (Vec<u8>, Vec<u8>)>,
    /// Publisher push signature over SHA-256(root || store_id) for this root (§21.6).
    signature: Option<Bytes96>,
}

struct StoreState {
    public_key: Bytes48,
    served_root: Bytes32,
    pending_root: Option<Bytes32>,
    requires_bearer: bool,
    bearer_token: Option<String>,
    generations: HashMap<Bytes32, Generation>,
    /// served head ordering for root history.
    history_order: Vec<Bytes32>,
}

/// Deterministic in-memory backend for tests and as a reference implementation.
pub struct InMemoryBackend {
    stores: Mutex<HashMap<Bytes32, StoreState>>,
    max_module_size: u64,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        InMemoryBackend {
            stores: Mutex::new(HashMap::new()),
            max_module_size: 16 * 1024 * 1024,
        }
    }

    pub fn with_max_module_size(max: u64) -> Self {
        InMemoryBackend {
            stores: Mutex::new(HashMap::new()),
            max_module_size: max,
        }
    }

    /// Register a store with a genesis generation. `genesis_sig` is the publisher
    /// push signature over the genesis root; supply it when the genesis head will
    /// be cloned/pulled (clients fail closed on an unsigned served head), or
    /// `None` for a genesis that is never served as the clone target.
    pub fn add_store(
        &self,
        store_id: Bytes32,
        public_key: Bytes48,
        genesis_root: Bytes32,
        module: Vec<u8>,
        genesis_sig: Option<Bytes96>,
    ) {
        let g = Generation {
            parent: None,
            generation_no: 0,
            timestamp: 1_000,
            module,
            chunks: HashMap::new(),
            key_table_changes: Vec::new(),
            content: HashMap::new(),
            signature: genesis_sig,
        };
        let mut state = StoreState {
            public_key,
            served_root: genesis_root,
            pending_root: None,
            requires_bearer: false,
            bearer_token: None,
            generations: HashMap::new(),
            history_order: vec![genesis_root],
        };
        state.generations.insert(genesis_root, g);
        self.stores.lock().unwrap().insert(store_id, state);
    }

    /// Add a child generation directly (test helper, bypasses push auth).
    #[allow(clippy::too_many_arguments)]
    pub fn add_generation(
        &self,
        store_id: &Bytes32,
        parent: Bytes32,
        new_root: Bytes32,
        module: Vec<u8>,
        chunks: Vec<(Bytes32, Vec<u8>)>,
        key_table_changes: Vec<Vec<u8>>,
        advance: bool,
    ) {
        let mut stores = self.stores.lock().unwrap();
        let st = stores.get_mut(store_id).expect("store");
        let gen_no = st.generations.len() as u64;
        let gen = Generation {
            parent: Some(parent),
            generation_no: gen_no,
            timestamp: 1_000 + gen_no,
            module,
            chunks: chunks.into_iter().collect(),
            key_table_changes,
            content: HashMap::new(),
            signature: None,
        };
        st.generations.insert(new_root, gen);
        if advance {
            st.served_root = new_root;
            st.history_order.push(new_root);
        }
    }

    /// Require a bearer token for push (§21.6).
    pub fn set_bearer(&self, store_id: &Bytes32, token: &str) {
        let mut stores = self.stores.lock().unwrap();
        if let Some(st) = stores.get_mut(store_id) {
            st.requires_bearer = true;
            st.bearer_token = Some(token.to_string());
        }
    }

    /// Insert a real content hit for a retrieval key at the served head.
    pub fn put_content(
        &self,
        store_id: &Bytes32,
        retrieval_key: Bytes32,
        ciphertext: Vec<u8>,
        proof: Vec<u8>,
    ) {
        let mut stores = self.stores.lock().unwrap();
        let st = stores.get_mut(store_id).expect("store");
        let root = st.served_root;
        let gen = st.generations.get_mut(&root).expect("gen");
        gen.content.insert(retrieval_key, (ciphertext, proof));
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Deterministic decoy bytes: length from a logarithmic distribution keyed by
/// the retrieval key; bytes are a SHA-256 keystream over the key (§14.2).
fn decoy_bytes(retrieval_key: &Bytes32) -> Vec<u8> {
    use digstore_crypto::sha256;
    // length: 256..=32Ki, bucketed by the low bits of the key (log distribution).
    let bucket = (retrieval_key.0[0] % 8) as u32; // 0..7
    let len = 256usize << bucket; // 256, 512, ... 32768
    let mut out = Vec::with_capacity(len);
    let mut counter = 0u32;
    while out.len() < len {
        let mut block = Vec::with_capacity(36);
        block.extend_from_slice(&retrieval_key.0);
        block.extend_from_slice(&counter.to_be_bytes());
        let h = sha256(&block);
        out.extend_from_slice(&h.0);
        counter += 1;
    }
    out.truncate(len);
    out
}

impl RemoteBackend for InMemoryBackend {
    fn head_state(&self, store_id: &Bytes32) -> Result<HeadState, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let served = st
            .generations
            .get(&st.served_root)
            .ok_or(RemoteError::UnknownRoot)?;
        Ok(HeadState {
            served_root: st.served_root,
            pending_root: st.pending_root,
            served_size: served.module.len() as u64,
            public_key: st.public_key,
            served_sig: served.signature,
        })
    }

    fn root_history(&self, store_id: &Bytes32) -> Result<Vec<RootRecord>, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        Ok(st
            .history_order
            .iter()
            .map(|r| {
                let g = &st.generations[r];
                RootRecord {
                    generation: g.generation_no,
                    root: *r,
                    timestamp: g.timestamp,
                }
            })
            .collect())
    }

    fn module_bytes(
        &self,
        store_id: &Bytes32,
        root: Option<&Bytes32>,
    ) -> Result<Vec<u8>, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let target = match root {
            Some(r) => *r,
            None => st.served_root,
        };
        let g = st
            .generations
            .get(&target)
            .ok_or(RemoteError::UnknownRoot)?;
        // Only the served head is downloadable as the current module.
        if target != st.served_root {
            return Err(RemoteError::UnknownRoot);
        }
        Ok(g.module.clone())
    }

    fn serve_content(
        &self,
        store_id: &Bytes32,
        retrieval_key: &Bytes32,
        root: &Bytes32,
        range: Option<(u64, u64)>,
    ) -> Result<(Vec<u8>, Vec<u8>, Bytes32), RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let g = st.generations.get(root).ok_or(RemoteError::UnknownRoot)?;
        let (mut ct, proof) = match g.content.get(retrieval_key) {
            Some((ct, p)) => (ct.clone(), p.clone()),
            // Retrieval MISS: deterministic decoy, never 404 (§14.2/§21.8).
            None => (decoy_bytes(retrieval_key), Vec::new()),
        };
        if let Some((start, end)) = range {
            let s = (start as usize).min(ct.len());
            let e = (end as usize).min(ct.len());
            if s <= e {
                ct = ct[s..e].to_vec();
            }
        }
        Ok((ct, proof, *root))
    }

    fn serve_proof(
        &self,
        store_id: &Bytes32,
        _retrieval_key: &Bytes32,
        root: &Bytes32,
    ) -> Result<(Vec<u8>, Bytes32), RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        st.generations.get(root).ok_or(RemoteError::UnknownRoot)?;
        // reference: empty proof blob; real backend returns ExecutionProof bytes.
        Ok((Vec::new(), *root))
    }

    fn accept_push(
        &self,
        store_id: &Bytes32,
        parent: &Bytes32,
        new_root: &Bytes32,
        module_bytes: &[u8],
        sig: Option<&Bytes96>,
        mode: PushMode,
    ) -> Result<PushOutcome, RemoteError> {
        let mut stores = self.stores.lock().unwrap();
        let st = stores.get_mut(store_id).ok_or(RemoteError::UnknownStore)?;
        let gen_no = st.generations.len() as u64;
        let gen = Generation {
            parent: Some(*parent),
            generation_no: gen_no,
            timestamp: 1_000 + gen_no,
            module: module_bytes.to_vec(),
            chunks: HashMap::new(),
            key_table_changes: Vec::new(),
            content: HashMap::new(),
            signature: sig.copied(),
        };
        st.generations.insert(*new_root, gen);
        match mode {
            PushMode::Advance => {
                st.served_root = *new_root;
                st.pending_root = None;
                st.history_order.push(*new_root);
                Ok(PushOutcome::Advanced)
            }
            PushMode::Pending => {
                st.pending_root = Some(*new_root);
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
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let from_gen = st.generations.get(from).ok_or(RemoteError::UnknownRoot)?;
        let to_gen = st.generations.get(to).ok_or(RemoteError::UnknownRoot)?;
        let new_chunks: Vec<(Bytes32, Vec<u8>)> = to_gen
            .chunks
            .iter()
            .filter(|(h, _)| !from_gen.chunks.contains_key(*h))
            .map(|(h, d)| (*h, d.clone()))
            .collect();
        Ok(DeltaSet {
            from: *from,
            to: *to,
            new_chunks,
            key_table_changes: to_gen.key_table_changes.clone(),
        })
    }

    fn delta_from_have(
        &self,
        store_id: &Bytes32,
        to: &Bytes32,
        have: &[Bytes32],
    ) -> Result<DeltaSet, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let to_gen = st.generations.get(to).ok_or(RemoteError::UnknownRoot)?;
        let have_set: std::collections::HashSet<&Bytes32> = have.iter().collect();
        let new_chunks: Vec<(Bytes32, Vec<u8>)> = to_gen
            .chunks
            .iter()
            .filter(|(h, _)| !have_set.contains(*h))
            .map(|(h, d)| (*h, d.clone()))
            .collect();
        Ok(DeltaSet {
            from: Bytes32([0u8; 32]),
            to: *to,
            new_chunks,
            key_table_changes: to_gen.key_table_changes.clone(),
        })
    }

    fn max_module_size(&self) -> u64 {
        self.max_module_size
    }

    fn requires_bearer(&self, store_id: &Bytes32) -> bool {
        let stores = self.stores.lock().unwrap();
        stores
            .get(store_id)
            .map(|s| s.requires_bearer)
            .unwrap_or(false)
    }

    fn check_bearer(&self, store_id: &Bytes32, token: Option<&str>) -> bool {
        let stores = self.stores.lock().unwrap();
        match stores.get(store_id) {
            Some(st) => match (&st.bearer_token, token) {
                (Some(expected), Some(given)) => expected == given,
                (Some(_), None) => false,
                (None, _) => true,
            },
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b32(x: u8) -> Bytes32 {
        Bytes32([x; 32])
    }
    fn b48(x: u8) -> Bytes48 {
        Bytes48([x; 48])
    }

    fn backend_with_one_store() -> (InMemoryBackend, Bytes32) {
        let be = InMemoryBackend::new();
        let id = b32(1);
        be.add_store(id, b48(2), b32(0x10), vec![0u8; 64], None);
        (be, id)
    }

    #[test]
    fn head_state_returns_served_root_and_size() {
        let (be, id) = backend_with_one_store();
        let hs = be.head_state(&id).unwrap();
        assert_eq!(hs.served_root, b32(0x10));
        assert_eq!(hs.served_size, 64);
        assert_eq!(hs.pending_root, None);
    }

    #[test]
    fn head_state_unknown_store_errors() {
        let be = InMemoryBackend::new();
        assert!(matches!(
            be.head_state(&b32(9)),
            Err(RemoteError::UnknownStore)
        ));
    }

    #[test]
    fn content_miss_returns_deterministic_decoy_never_error() {
        let (be, id) = backend_with_one_store();
        let key = b32(0x55);
        let (a, _, _) = be.serve_content(&id, &key, &b32(0x10), None).unwrap();
        let (b, _, _) = be.serve_content(&id, &key, &b32(0x10), None).unwrap();
        assert!(!a.is_empty());
        assert_eq!(a, b, "same miss must yield identical decoy bytes (§14.2)");
    }

    #[test]
    fn content_hit_returns_real_ciphertext() {
        let (be, id) = backend_with_one_store();
        let key = b32(0x77);
        be.put_content(&id, key, vec![1, 2, 3, 4], vec![9, 9]);
        let (ct, proof, root) = be.serve_content(&id, &key, &b32(0x10), None).unwrap();
        assert_eq!(ct, vec![1, 2, 3, 4]);
        assert_eq!(proof, vec![9, 9]);
        assert_eq!(root, b32(0x10));
    }

    #[test]
    fn pending_push_does_not_advance_served_head() {
        let (be, id) = backend_with_one_store();
        let out = be
            .accept_push(
                &id,
                &b32(0x10),
                &b32(0x20),
                &[0u8; 32],
                None,
                PushMode::Pending,
            )
            .unwrap();
        assert_eq!(out, PushOutcome::Pending);
        let hs = be.head_state(&id).unwrap();
        assert_eq!(
            hs.served_root,
            b32(0x10),
            "served head unchanged on pending (§21.4)"
        );
        assert_eq!(hs.pending_root, Some(b32(0x20)));
    }

    #[test]
    fn advance_push_moves_served_head() {
        let (be, id) = backend_with_one_store();
        let out = be
            .accept_push(
                &id,
                &b32(0x10),
                &b32(0x30),
                &[0u8; 48],
                None,
                PushMode::Advance,
            )
            .unwrap();
        assert_eq!(out, PushOutcome::Advanced);
        let hs = be.head_state(&id).unwrap();
        assert_eq!(hs.served_root, b32(0x30));
        assert_eq!(hs.served_size, 48);
    }

    #[test]
    fn delta_returns_only_new_chunks() {
        let (be, id) = backend_with_one_store();
        // genesis (0x10) has no chunks; add a child with two chunks.
        be.add_generation(
            &id,
            b32(0x10),
            b32(0x40),
            vec![0u8; 10],
            vec![(b32(0xA1), vec![1]), (b32(0xA2), vec![2])],
            vec![vec![7, 7]],
            true,
        );
        let d = be.delta(&id, &b32(0x10), &b32(0x40)).unwrap();
        assert_eq!(d.new_chunks.len(), 2);
        assert_eq!(d.key_table_changes, vec![vec![7, 7]]);
    }

    #[test]
    fn bearer_required_rejects_wrong_token() {
        let (be, id) = backend_with_one_store();
        be.set_bearer(&id, "secret");
        assert!(be.requires_bearer(&id));
        assert!(be.check_bearer(&id, Some("secret")));
        assert!(!be.check_bearer(&id, Some("wrong")));
        assert!(!be.check_bearer(&id, None));
    }
}
