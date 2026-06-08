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
    let mut bytes = std::fs::read(module_path).unwrap();
    let magic = b"DIGS";
    let start = bytes
        .windows(magic.len())
        .position(|w| w == magic)
        .expect("DIGS magic present in compiled module");
    // Header = magic(4) + version(1) + count(4) + 5 rows*(1+4+4)=45 => 54 bytes.
    // SEG_POOL is the first segment; its body is `u32 len || pool bytes`, so the
    // pool ciphertext begins at header+4. Flip a byte a few into the first chunk
    // ciphertext so the client merkle (leaf) and GCM tag both fail.
    let target = start + 54 + 4 + 2;
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
    pub fn start_with_module(store_id_hex: &str, root_hex: &str, public_key: [u8; 48], module: &[u8]) -> Self {
        let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
        let root = digstore_core::Bytes32::from_hex(root_hex).unwrap();
        let backend = Arc::new(digstore_remote::InMemoryBackend::new());
        backend.add_store(store_id, digstore_core::Bytes48(public_key), root, module.to_vec());
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
