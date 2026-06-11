//! §17.2 Secretless Module property test.
//!
//! Property: the compiled `.wasm` embeds NO decryption key, NO BLS signing
//! secret, and NO `SecretSalt` — only ciphertext, public metadata, and public
//! keys (trusted host keys, store public key). This test compiles a real module
//! through the full pipeline, reads the emitted bytes, and asserts:
//!   1. distinctive SECRET byte-patterns are ABSENT from the module, and
//!   2. distinctive PUBLIC byte-patterns (store pubkey, trusted host pubkey,
//!      manifest text) are PRESENT — proving the scan actually reaches the data
//!      the guest serves (so absence of secrets is meaningful, not vacuous).

mod common;

use common::{chunk, resource_key, sample_manifest, store_id, FakeGeneration, ResourceSpec};
use digstore_compiler::{Compiler, CompilerConfig};
use digstore_core::{Bytes32, Bytes48, TrustedHostKey};

/// Distinctive byte patterns that represent SECRETS. None of these may ever be
/// baked into the module. Each is a 32/48-byte run of a unique sentinel value so
/// an accidental short coincidence in the binary is astronomically unlikely.
const DECRYPTION_KEY_SENTINEL: [u8; 32] = [0xDE; 32]; // a per-resource AEAD key
const BLS_SECRET_SENTINEL: [u8; 32] = [0x5E; 32]; // a BLS signing scalar
const SECRET_SALT_SENTINEL: [u8; 32] = [0x5A; 32]; // the SecretSalt

fn cfg(dir: &std::path::Path) -> CompilerConfig {
    CompilerConfig {
        output_dir: dir.to_path_buf(),
        obfuscate: false,
        optimize: false,
        template_override: None,
        // Small uniform budget keeps the emitted module tiny/fast (the 128 MiB
        // default would make every test module ~128 MiB).
        uniform_blob_len: 64 * 1024,
    }
}

/// Build a generation set whose PUBLIC material uses distinctive markers so we
/// can prove the scan reaches the served data.
fn generations() -> Vec<FakeGeneration> {
    // Ciphertext stand-ins: in production these chunk bodies are already
    // encrypted; here they are simply public, distinctive bytes.
    let a = chunk(b"PUBLIC-CIPHERTEXT-MARKER-AAAA");
    let b = chunk(b"PUBLIC-CIPHERTEXT-MARKER-BBBB");
    vec![FakeGeneration {
        root: Bytes32([0x11; 32]),
        generation_id: 1,
        resources: vec![ResourceSpec {
            resource_key: resource_key("index.html"),
            chunks: vec![a, b],
        }],
    }]
}

fn trusted_pubkey() -> [u8; 48] {
    [0x42u8; 48] // distinctive PUBLIC host key
}

fn store_pubkey() -> Bytes48 {
    Bytes48([0xCD; 48]) // distinctive PUBLIC store key
}

/// True iff `needle` appears as a contiguous run inside `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn test_module_is_secretless() {
    let dir = std::env::temp_dir().join(format!("digc-secretless-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let trusted = vec![TrustedHostKey {
        public_key: trusted_pubkey(),
        label: "dig-host-key-v1".into(),
    }];

    let outcome = Compiler::compile(
        &cfg(&dir),
        store_id(),
        store_pubkey(),
        &generations(),
        sample_manifest(),
        common::no_auth(),
        &trusted,
        None,
    )
    .expect("compiles");

    let module = std::fs::read(&outcome.result.output_path).expect("read emitted module");

    // ---- (1) NO secret may be embedded ----
    assert!(
        !contains(&module, &DECRYPTION_KEY_SENTINEL),
        "module leaks a decryption key"
    );
    assert!(
        !contains(&module, &BLS_SECRET_SENTINEL),
        "module leaks a BLS signing secret"
    );
    assert!(
        !contains(&module, &SECRET_SALT_SENTINEL),
        "module leaks the SecretSalt"
    );

    // ---- (2) PUBLIC material IS embedded (scan is non-vacuous) ----
    assert!(
        contains(&module, &store_pubkey().0),
        "store public key missing from module — scan would be vacuous"
    );
    assert!(
        contains(&module, &trusted_pubkey()),
        "trusted host public key missing from module"
    );
    assert!(
        contains(&module, b"sample-store"),
        "public manifest name missing from module"
    );
    assert!(
        contains(&module, b"PUBLIC-CIPHERTEXT-MARKER-AAAA"),
        "served ciphertext missing from module"
    );

    std::fs::remove_dir_all(&dir).ok();
}
