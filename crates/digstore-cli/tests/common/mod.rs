#![allow(dead_code)]
use std::sync::Arc;

use assert_cmd::Command;
use tempfile::TempDir;

pub fn dig(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("digstore").unwrap();
    cmd.arg("--dig-dir").arg(dir.path());
    cmd
}

pub fn tmp_dig() -> TempDir {
    TempDir::new().unwrap()
}

/// Scrape store_id (hex) from config.toml and newest root (hex) from `log --json`.
pub fn store_id_and_root(dir: &TempDir) -> (String, String) {
    let out = dig(dir).args(["log", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let root = v[0]["root"].as_str().unwrap().to_string();
    let cfg = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    let line = cfg.lines().find(|l| l.contains("store_id")).unwrap();
    let store_id = line.split('"').nth(1).unwrap().to_string();
    (store_id, root)
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
    pub fn start_with_module(
        store_id_hex: &str,
        root_hex: &str,
        public_key: [u8; 48],
        module: &[u8],
    ) -> Self {
        let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
        let root = digstore_core::Bytes32::from_hex(root_hex).unwrap();
        let backend = Arc::new(digstore_remote::InMemoryBackend::new());
        backend.add_store(
            store_id,
            digstore_core::Bytes48(public_key),
            root,
            module.to_vec(),
        );
        Self::launch(backend)
    }

    /// Start an empty server with one store registered (genesis = empty module).
    pub fn start_empty(store_id_hex: &str, public_key: [u8; 48]) -> Self {
        let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
        let backend = Arc::new(digstore_remote::InMemoryBackend::new());
        backend.add_store(
            store_id,
            digstore_core::Bytes48(public_key),
            digstore_core::Bytes32([0u8; 32]),
            vec![0u8; 4],
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
