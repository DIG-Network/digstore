#![allow(dead_code)]
use std::sync::Arc;

use assert_cmd::Command;
use tempfile::TempDir;

/// Public BIP-39 test vector (NOT a real wallet). The anchoring seam unlocks
/// with this via a cached-unlock session so `init` can mint against the mock.
pub const ABANDON_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon \
    abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
    abandon abandon abandon abandon abandon art";

/// Provide a seeded, mocked anchoring environment on `cmd`.
///
/// `digstore init` now MINTS a store singleton (it requires a seed + an anchor
/// backend). To keep the offline integration suite working, this:
/// - points `DIGSTORE_HOME` at a stable per-project `<dir>/.dighome`,
/// - writes a cached-unlock session there carrying the ABANDON test mnemonic
///   (the exact `digstore_chain::unlock` Session format: `{expires_at, phrase}`),
///   so unlock returns the phrase with no seed.enc and no passphrase prompt, and
/// - sets `DIGSTORE_ANCHOR_MOCK=1` so anchoring is the in-memory mock (no network).
///
/// ABANDON is a public test vector and the mock spends nothing, so this is safe.
pub fn seed_mock_env(cmd: &mut Command, dir: &std::path::Path) {
    let home = dir.join(".dighome");
    std::fs::create_dir_all(&home).unwrap();
    let session = home.join("session");
    if !session.exists() {
        // Far-future absolute expiry (year 2100) so the session never lapses.
        let body = serde_json::json!({
            "expires_at": 4_102_444_800u64,
            "phrase": ABANDON_MNEMONIC,
        });
        std::fs::write(&session, serde_json::to_vec(&body).unwrap()).unwrap();
    }
    // Network/account commands (push, pull, clone, revoke) are gated behind a
    // dighub login. Point the dighub session dir (`DIG_IDENTITY_DIR`, the same dir
    // the identity key uses) at a per-project location and pre-write a valid
    // `session.json`, so the offline suite satisfies the gate exactly the way a
    // logged-in user would. The token is a throwaway test string (the gate is a
    // product check, not push crypto — push still uses the store/identity keys).
    let id_dir = dir.join(".digid");
    std::fs::create_dir_all(&id_dir).unwrap();
    let session_json = id_dir.join("session.json");
    if !session_json.exists() {
        let body = serde_json::json!({
            "access_token": "test-token",
            "handle": "tester",
            "account_ph": null,
            "api_base": "https://hub.dig.net/v1",
            "obtained_at": 1_700_000_000u64,
            "expires_in": null,
        });
        std::fs::write(&session_json, serde_json::to_vec(&body).unwrap()).unwrap();
    }

    cmd.env("DIGSTORE_HOME", &home)
        .env("DIGSTORE_ANCHOR_MOCK", "1")
        .env("DIG_IDENTITY_DIR", &id_dir);
}

/// A `digstore` invocation against the temp project `dir`. The workspace lives at
/// `<dir>/.dig` (a SUBDIR of the build/content dir) and the command runs WITH
/// `current_dir(dir)`, so `op_dir` defaults to `<dir>` — exactly like a real user
/// running `digstore` from inside their project. Content files written under
/// `<dir>` therefore key RELATIVE to `<dir>` (never as absolute paths), and the
/// `.dig` skip only excludes `<dir>/.dig` (a proper subdir of op_dir = `<dir>`).
///
/// Transparently carries the seeded mock anchoring env (see [`seed_mock_env`]) so
/// `init` mints against the in-memory mock instead of Chia mainnet.
pub fn dig(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("digstore").unwrap();
    cmd.arg("--dig-dir")
        .arg(dir.path().join(".dig"))
        .current_dir(dir.path());
    seed_mock_env(&mut cmd, dir.path());
    cmd
}

pub fn tmp_dig() -> TempDir {
    TempDir::new().unwrap()
}

/// The per-store directory for the default store. With the workspace at
/// `<dir>/.dig`, the default store's files (config.toml, signing_key.bin,
/// modules/, ...) live under `<dir>/.dig/stores/default/`.
pub fn store_dir(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join(".dig").join("stores").join("default")
}

/// Scrape store_id (hex) from config.toml and newest root (hex) from `log --json`.
pub fn store_id_and_root(dir: &TempDir) -> (String, String) {
    let out = dig(dir).args(["log", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let root = v[0]["root"].as_str().unwrap().to_string();
    let cfg = std::fs::read_to_string(store_dir(dir).join("config.toml")).unwrap();
    let line = cfg.lines().find(|l| l.contains("store_id")).unwrap();
    let store_id = line.split('"').nth(1).unwrap().to_string();
    (store_id, root)
}

/// Compute the publisher push signature over a store's genesis root, using the
/// source store's BLS signing key (the seed written to `signing_key.bin` by
/// `digstore init`). This is what a real publisher's first push would carry; the
/// test seeds it so a clone of the genesis head passes the §21.6 authenticated-
/// head check.
pub fn genesis_push_sig(dir: &TempDir, store_id_hex: &str, root_hex: &str) -> [u8; 96] {
    let seed = std::fs::read(store_dir(dir).join("signing_key.bin")).unwrap();
    let sk = digstore_crypto::bls::SecretKey::from_seed(&seed);
    let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
    let root = digstore_core::Bytes32::from_hex(root_hex).unwrap();
    digstore_crypto::sign_push(&sk, &root, &store_id).0
}

/// Sign a `Root`-scoped revocation tombstone over `root` for `store_id`, using
/// the source store's BLS signing key (SECURITY.md residual #1 Layer 1). Returns
/// the canonical record bytes + the 96-byte signature so a test can seed a VALID
/// signed tombstone into the in-memory backend.
pub fn sign_root_tombstone(
    dir: &TempDir,
    store_id_hex: &str,
    root_hex: &str,
) -> (digstore_core::Tombstone, [u8; 96]) {
    let seed = std::fs::read(store_dir(dir).join("signing_key.bin")).unwrap();
    let sk = digstore_crypto::bls::SecretKey::from_seed(&seed);
    let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
    let root = digstore_core::Bytes32::from_hex(root_hex).unwrap();
    let t = digstore_core::Tombstone::root(
        store_id,
        root,
        1_700_000_000,
        digstore_core::RevocationReason::Compromise,
    );
    let sig = digstore_crypto::sign_tombstone(&sk, &t).0;
    (t, sig)
}

/// Sign a `Store`-scoped revocation tombstone for `store_id` with the source
/// store's signing key.
pub fn sign_store_tombstone(
    dir: &TempDir,
    store_id_hex: &str,
) -> (digstore_core::Tombstone, [u8; 96]) {
    let seed = std::fs::read(store_dir(dir).join("signing_key.bin")).unwrap();
    let sk = digstore_crypto::bls::SecretKey::from_seed(&seed);
    let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
    let t = digstore_core::Tombstone::store(
        store_id,
        1_700_000_000,
        digstore_core::RevocationReason::Takedown,
    );
    let sig = digstore_crypto::sign_tombstone(&sk, &t).0;
    (t, sig)
}

/// Seed a pre-built (record, signature) tombstone directly into a test server's
/// in-memory backend, bypassing the POST handler's signature check. Used to model
/// a remote that already holds a tombstone — including a wrong-key/unsigned one,
/// to prove the CLIENT ignores it.
pub fn seed_tombstone(
    server: &TestServer,
    store_id_hex: &str,
    tombstone: digstore_core::Tombstone,
    signature: [u8; 96],
) {
    use digstore_remote::RemoteBackend;
    let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
    server
        .backend()
        .store_tombstone(
            &store_id,
            &digstore_remote::StoredTombstone {
                tombstone,
                signature: digstore_core::Bytes96(signature),
            },
        )
        .unwrap();
}

/// Corrupt the injected data section by flipping a byte well past the header.
/// Lands inside the chunk-ciphertext pool region so the host still instantiates
/// the module (no code corruption) and the failure is CLIENT-side merkle/GCM.
pub fn corrupt_data_section(module_path: &std::path::Path) {
    use digstore_core::datasection::{DataView, SectionId};

    let mut bytes = std::fs::read(module_path).unwrap();
    let magic = b"DIGS";

    // The real guest wasm embeds the `DIGS` magic byte string in its own rodata
    // (the `MAGIC`/`EMPTY_BLOB` constants), so the FIRST occurrence is NOT the
    // injected data section. Scan ALL occurrences and pick the one that parses as
    // a valid contract blob carrying a ChunkPool — that is the injected blob.
    let mut found: Option<(usize, usize)> = None; // (blob_start, pool_off_in_blob)
    for start in 0..bytes.len().saturating_sub(magic.len()) {
        if &bytes[start..start + magic.len()] != magic {
            continue;
        }
        let blob = &bytes[start..];
        if let Ok(view) = DataView::parse(blob) {
            if let Some(pool) = view.section(SectionId::ChunkPool) {
                let pool_off = pool.as_ptr() as usize - blob.as_ptr() as usize;
                found = Some((start, pool_off));
                break;
            }
        }
    }
    let (start, pool_off_in_blob) =
        found.expect("compiled module has an injected DIGS blob with a ChunkPool");

    // Flip a byte a few into the FIRST chunk's ciphertext. ChunkPool body =
    // count(u32 BE) || per chunk: len(u32 BE) || bytes. The first ciphertext byte
    // is at pool_offset + 4 (count) + 4 (first len). Flipping it makes the client
    // merkle leaf (and GCM tag) fail while the module still instantiates.
    let target = start + pool_off_in_blob + 4 + 4 + 2;
    assert!(target < bytes.len(), "module too small to corrupt");
    bytes[target] ^= 0xFF;
    std::fs::write(module_path, &bytes).unwrap();
}

/// A running `digstore-remote` test server over an in-memory backend.
pub struct TestServer {
    base: String,
    backend: Arc<digstore_remote::InMemoryBackend>,
    _rt: tokio::runtime::Runtime,
}

impl TestServer {
    pub fn base_url(&self) -> String {
        self.base.clone()
    }

    pub fn backend(&self) -> Arc<digstore_remote::InMemoryBackend> {
        self.backend.clone()
    }

    /// Start a server seeded with one store at `genesis_root` carrying `module`.
    /// `genesis_sig` is the publisher push signature over the genesis root; it is
    /// required because the seeded genesis IS the served head a client clones, and
    /// clone/pull now fail closed on an unauthenticated head (§21.6). See
    /// [`genesis_push_sig`].
    pub fn start_with_module(
        store_id_hex: &str,
        root_hex: &str,
        public_key: [u8; 48],
        module: &[u8],
        genesis_sig: [u8; 96],
    ) -> Self {
        let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
        let root = digstore_core::Bytes32::from_hex(root_hex).unwrap();
        let backend = Arc::new(digstore_remote::InMemoryBackend::new());
        backend.add_store(
            store_id,
            digstore_core::Bytes48(public_key),
            root,
            module.to_vec(),
            Some(digstore_core::Bytes96(genesis_sig)),
        );
        Self::launch(backend)
    }

    /// Start an empty server with one store registered (genesis = empty module).
    /// The empty genesis is never cloned directly (a push advances the head to a
    /// real, signed root first), so it carries no signature.
    pub fn start_empty(store_id_hex: &str, public_key: [u8; 48]) -> Self {
        let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
        let backend = Arc::new(digstore_remote::InMemoryBackend::new());
        backend.add_store(
            store_id,
            digstore_core::Bytes48(public_key),
            digstore_core::Bytes32([0u8; 32]),
            vec![0u8; 4],
            None,
        );
        Self::launch(backend)
    }

    fn launch(backend: Arc<digstore_remote::InMemoryBackend>) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let be = backend.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        rt.spawn(async move {
            let server = digstore_remote::RemoteServer::new(be);
            let router = server.router();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tx.send(addr).unwrap();
            axum::serve(listener, router).await.unwrap();
        });
        let addr = rx.recv().unwrap();
        TestServer {
            base: format!("http://{addr}"),
            backend,
            _rt: rt,
        }
    }
}
