use crate::derive_decryption_key;
use digstore_core::SecretSalt;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// One HKDF known-answer vector: a canonical URN (+ optional secret salt) and
/// the 32-byte derived AES-256 key. Frozen so derivation cannot silently drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfFixture {
    pub name: String,
    pub canonical_urn: String,
    /// `None` for public stores; hex of the 32-byte `SecretSalt` otherwise.
    pub secret_salt_hex: Option<String>,
    pub key_hex: String,
}

/// The full frozen KDF KAT set, tagged with the crate's `CRYPTO_VERSION`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfFixtureSet {
    pub crypto_version: u32,
    pub vectors: Vec<KdfFixture>,
}

impl KdfFixtureSet {
    /// Deterministically generate the canonical KAT set.
    pub fn generate() -> Self {
        // (name, urn, optional secret salt) tuples.
        let specs: &[(&str, &str, Option<[u8; 32]>)] = &[
            (
                "public_root_a",
                "urn:dig:mainnet:0000000000000000000000000000000000000000000000000000000000000000/a",
                None,
            ),
            (
                "public_root_file",
                "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/file.txt",
                None,
            ),
            (
                "private_salt_07",
                "urn:dig:mainnet:0000000000000000000000000000000000000000000000000000000000000000/a",
                Some([0x07; 32]),
            ),
            (
                "private_salt_09",
                "urn:dig:mainnet:2222222222222222222222222222222222222222222222222222222222222222/a",
                Some([0x09; 32]),
            ),
        ];

        let mut vectors = Vec::with_capacity(specs.len());
        for (name, urn, salt) in specs {
            let salt_opt = salt.map(SecretSalt);
            let key = derive_decryption_key(urn, salt_opt.as_ref());
            vectors.push(KdfFixture {
                name: name.to_string(),
                canonical_urn: urn.to_string(),
                secret_salt_hex: salt.map(hex::encode),
                key_hex: hex::encode(key),
            });
        }

        KdfFixtureSet {
            crypto_version: crate::CRYPTO_VERSION,
            vectors,
        }
    }
}

/// Generate and write the KDF KAT set as pretty JSON to `path`, creating parent
/// directories as needed. Called only by `examples/gen_fixtures.rs`.
pub fn write_kdf_fixtures(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let set = KdfFixtureSet::generate();
    let json =
        serde_json::to_string_pretty(&set).map_err(io::Error::other)?;
    std::fs::write(path, json)
}

use crate::bls::{bls_keygen, bls_sign};

/// One cross-implementation parity vector: a message and the host-side (blst)
/// AugScheme public key + signature. The guest's pure-Rust `bls12_381` verifier
/// must accept every vector (CONVENTIONS C8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlsFixture {
    pub name: String,
    pub seed_hex: String,
    pub message_hex: String,
    pub pubkey_hex: String,
    pub signature_hex: String,
}

/// The full set of parity vectors, tagged with the shared scheme constant so
/// the guest asserts it is verifying the same scheme (Chia AugScheme).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlsFixtureSet {
    pub scheme: String,
    pub vectors: Vec<BlsFixture>,
}

impl BlsFixtureSet {
    /// Deterministically generate the canonical parity set.
    pub fn generate() -> Self {
        // (name, seed, message): empty, short, node-proof shape, push shape.
        let specs: &[(&str, [u8; 32], Vec<u8>)] = &[
            ("empty_message", [0x01; 32], vec![]),
            ("short_message", [0x02; 32], b"digstore".to_vec()),
            ("node_proof_shape", [0x03; 32], {
                let mut m = vec![0u8; 64];
                m.extend_from_slice(&[0xAB; 8]);
                m
            }),
            ("push_shape", [0x04; 32], {
                let mut m = vec![0x11; 32];
                m.extend_from_slice(&[0x22; 32]);
                m
            }),
        ];

        let mut vectors = Vec::with_capacity(specs.len());
        for (name, seed, msg) in specs {
            let (sk, pk) = bls_keygen(seed);
            let sig = bls_sign(&sk, msg);
            vectors.push(BlsFixture {
                name: name.to_string(),
                seed_hex: hex::encode(seed),
                message_hex: hex::encode(msg),
                pubkey_hex: hex::encode(pk.0),
                signature_hex: hex::encode(sig.0),
            });
        }

        BlsFixtureSet {
            scheme: crate::CHIA_BLS_SCHEME.to_string(),
            vectors,
        }
    }
}

/// Generate the canonical parity set and write it as pretty JSON to `path`,
/// creating parent directories as needed. Called only by the gen_fixtures example.
pub fn write_bls_fixtures(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let set = BlsFixtureSet::generate();
    let json =
        serde_json::to_string_pretty(&set).map_err(io::Error::other)?;
    std::fs::write(path, json)
}
