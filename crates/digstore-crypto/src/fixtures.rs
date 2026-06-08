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
        serde_json::to_string_pretty(&set).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}
