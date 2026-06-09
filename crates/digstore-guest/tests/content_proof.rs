use digstore_core::merkle::MerkleTree;
use digstore_core::Bytes32;
use digstore_guest::content::emit_merkle_proof;

#[test]
fn emitted_proof_verifies_against_core() {
    // Four chunks -> leaves = SHA-256(chunk). Build the core tree, then emit a
    // proof for chunk index 2 inside the guest and verify it with core rules.
    let chunks: Vec<Vec<u8>> = vec![
        b"alpha".to_vec(),
        b"beta".to_vec(),
        b"gamma".to_vec(),
        b"delta".to_vec(),
    ];
    let tree = MerkleTree::build(&chunks);
    let root: Bytes32 = tree.root();

    let proof = emit_merkle_proof(&tree, 2);
    assert_eq!(proof.root, root);
    assert!(
        proof.verify(),
        "guest-emitted proof must verify under core rules"
    );
}

mod fixtures;
mod mock_host;
use digstore_core::{ContentResponse, KeyTableEntry};
use digstore_guest::content::{serve_content, ContentOutcome, GateConfig};
use digstore_guest::datasection::{encode_key_table, DataSection, SectionId};
use digstore_guest::request::{ContentRequest, ValidityWindow};
use mock_host::MockHost;

fn gate_config() -> GateConfig {
    GateConfig {
        require_attestation: false,
        require_jwt: false,
        expected_iss: None,
        expected_aud: None,
    }
}

#[test]
fn hit_returns_real_content_response() {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0, 1, 2, 3],
        total_size: 20,
    };
    let table = encode_key_table(&[entry]);
    // Pool stores 4 chunk ciphertexts of fixed 5 bytes each in section ChunkPool.
    let pool = fixtures::pack_pool(&[b"alpha", b"beta_", b"gamma", b"delta"]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();

    let host = MockHost::default();
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    match serve_content(&host, &ds, &req, &gate_config()) {
        ContentOutcome::Real(resp) => {
            let r: ContentResponse = resp;
            assert!(!r.ciphertext.is_empty());
            assert_eq!(r.roothash, ds.current_root());
        }
        ContentOutcome::Decoy(_) => panic!("hit must return Real, not Decoy"),
    }
}

#[test]
fn miss_returns_decoy() {
    let table = encode_key_table(&[]); // empty table => every key misses
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &[]);
    let ds = DataSection::parse(&blob).unwrap();
    let host = MockHost::default();
    let req = ContentRequest {
        retrieval_key: Bytes32([0x99; 32]),
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    assert!(matches!(
        serve_content(&host, &ds, &req, &gate_config()),
        ContentOutcome::Decoy(_)
    ));
}

#[test]
fn outside_temporal_window_returns_decoy_even_on_hit() {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();
    let mut host = MockHost::default();
    host.time = 50; // before window
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: Some(ValidityWindow {
            not_before: 100,
            not_after: 200,
        }),
    };
    assert!(matches!(
        serve_content(&host, &ds, &req, &gate_config()),
        ContentOutcome::Decoy(_)
    ));
}

// --- D-ATTEST-VERIFY (§12.2): the gate MUST verify the host's BLS attestation
// signature over the challenge, against the trusted set embedded in the data
// section. A response signed by a key NOT in the trusted set, or with a
// corrupted signature, must yield a Decoy — never real content. -------------

use digstore_core::codec::{Encode, Encoder};
use digstore_guest::datasection::encode_blob;

/// Build a data-section blob carrying StoreId, CurrentRoot, KeyTable, ChunkPool,
/// and a TrustedKeys section (id 5). TrustedKeys body matches the compiler's
/// codec: u32 BE count, then per entry `[u8;48] public_key` + `String label`.
fn section_with_trusted(
    store_id: [u8; 32],
    root: [u8; 32],
    table: &[u8],
    pool: &[u8],
    trusted_pubkeys: &[[u8; 48]],
) -> Vec<u8> {
    let mut enc = Encoder::new();
    (trusted_pubkeys.len() as u32).encode(&mut enc);
    for pk in trusted_pubkeys {
        pk.encode(&mut enc);
        String::from("dig-host-key-v1").encode(&mut enc);
    }
    let trusted_body = enc.finish();
    encode_blob(&[
        (SectionId::StoreId as u16, store_id.to_vec()),
        (SectionId::CurrentRoot as u16, root.to_vec()),
        (SectionId::TrustedKeys as u16, trusted_body),
        (SectionId::KeyTable as u16, table.to_vec()),
        (SectionId::ChunkPool as u16, pool.to_vec()),
    ])
}

/// A host double that signs the exact challenge bytes it is handed with a real
/// Chia AugScheme BLS key, returning a wire `AttestationResponse`.
struct SigningHost {
    secret: digstore_crypto::bls::SecretKey,
    pubkey: [u8; 48],
    time: u64,
    rand: std::cell::Cell<u32>,
    corrupt_sig: bool,
}

impl SigningHost {
    fn new(seed: &[u8; 32]) -> Self {
        let secret = digstore_crypto::bls::SecretKey::from_seed(seed);
        let pubkey = secret.public_key().to_bytes().0;
        SigningHost {
            secret,
            pubkey,
            time: 1_700_000_000,
            rand: std::cell::Cell::new(0),
            corrupt_sig: false,
        }
    }
}

impl digstore_guest::host::DigHost for SigningHost {
    fn get_public_key(&self) -> digstore_guest::host::HostResult {
        Ok(self.pubkey.to_vec())
    }
    fn create_attestation(&self, challenge: &[u8]) -> digstore_guest::host::HostResult {
        // Sign the EXACT challenge bytes the gate handed us (AugScheme).
        let mut sig = digstore_crypto::bls::bls_sign(&self.secret, challenge).0;
        if self.corrupt_sig {
            sig[0] ^= 0x01; // tamper -> verification must fail
        }
        let mut resp = Vec::with_capacity(48 + 32 + 96);
        resp.extend_from_slice(&self.pubkey);
        resp.extend_from_slice(&[0x11u8; 32]); // host_instance_id (not signed)
        resp.extend_from_slice(&sig);
        Ok(resp)
    }
    fn establish_session(&self, _c: &[u8]) -> digstore_guest::host::HostResult {
        Ok(vec![1u8; 16])
    }
    fn verify_session(&self) -> bool {
        true
    }
    fn jwks_fetch(&self, _u: &[u8]) -> digstore_guest::host::HostResult {
        Ok(b"{}".to_vec())
    }
    fn current_time(&self) -> u64 {
        self.time
    }
    fn random_bytes(&self, count: u32) -> digstore_guest::host::HostResult {
        let n = self.rand.get();
        self.rand.set(n + 1);
        Ok((0..count)
            .map(|i| (n.wrapping_mul(31).wrapping_add(i)) as u8)
            .collect())
    }
}

fn attest_fixture(corrupt: bool, trusted_includes_signer: bool) -> ContentOutcome {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);

    let mut host = SigningHost::new(&[42u8; 32]);
    host.corrupt_sig = corrupt;

    // The embedded trusted set either contains the signer's key (positive case)
    // or only an unrelated trusted key (negative case).
    let trusted: Vec<[u8; 48]> = if trusted_includes_signer {
        vec![host.pubkey]
    } else {
        let other = digstore_crypto::bls::SecretKey::from_seed(&[7u8; 32]);
        vec![other.public_key().to_bytes().0]
    };
    let blob = section_with_trusted([0xAA; 32], [0xBB; 32], &table, &pool, &trusted);
    let ds = DataSection::parse(&blob).unwrap();

    let mut gc = gate_config();
    gc.require_attestation = true;
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    serve_content(&host, &ds, &req, &gc)
}

#[test]
fn valid_attestation_from_trusted_key_returns_real() {
    assert!(
        matches!(attest_fixture(false, true), ContentOutcome::Real(_)),
        "a valid AugScheme signature from a trusted key must release real content"
    );
}

#[test]
fn attestation_from_untrusted_key_returns_decoy() {
    // The host signs with a real key, but that key is NOT in the embedded
    // trusted set -> §12.2 verification fails -> Decoy, not real content.
    assert!(
        matches!(attest_fixture(false, false), ContentOutcome::Decoy(_)),
        "a signature from a key outside the trusted set MUST yield a Decoy"
    );
}

#[test]
fn attestation_with_corrupted_signature_returns_decoy() {
    // Trusted key, fresh, correct shape — but the signature bytes are tampered.
    // The pairing check must fail -> Decoy.
    assert!(
        matches!(attest_fixture(true, true), ContentOutcome::Decoy(_)),
        "a corrupted attestation signature MUST yield a Decoy"
    );
}

#[test]
fn failed_attestation_returns_decoy() {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();
    let mut host = MockHost::default();
    host.attestation = Err(digstore_core::ErrorCode::AttestationFailed);
    let mut gc = gate_config();
    gc.require_attestation = true;
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    assert!(matches!(
        serve_content(&host, &ds, &req, &gc),
        ContentOutcome::Decoy(_)
    ));
}

// --- Task 20: get_proof returns a ProofPrelude (CONVENTIONS C3) --------------
use digstore_core::ProofPrelude;
use digstore_guest::proof::{serve_proof, ProofOutcome};
use digstore_guest::request::ProofRequest;

#[test]
fn proof_hit_returns_prelude_binding_output_and_nonce() {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();
    let host = MockHost::default();
    let req = ProofRequest {
        retrieval_key: key,
        root_hash: None,
        client_nonce: [3u8; 32],
    };
    let gc = gate_config();
    match serve_proof(&host, &ds, &req, &gc) {
        ProofOutcome::Real(p) => {
            let p: ProofPrelude = p;
            assert_eq!(p.roothash, ds.current_root());
            // output_commitment = SHA-256 of the served output bytes ("alpha").
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(b"alpha");
            let mut want = [0u8; 32];
            want.copy_from_slice(&h.finalize());
            assert_eq!(
                p.output_commitment,
                Bytes32(want),
                "commits to served bytes"
            );
            // serving_digest binds (retrieval_key, ordered indices, client_nonce):
            // a different nonce must change it.
            let req2 = ProofRequest {
                retrieval_key: key,
                root_hash: None,
                client_nonce: [9u8; 32],
            };
            if let ProofOutcome::Real(p2) = serve_proof(&host, &ds, &req2, &gc) {
                assert_ne!(
                    p.serving_digest, p2.serving_digest,
                    "serving_digest must bind the client nonce"
                );
            } else {
                panic!("hit must return Real");
            }
        }
        ProofOutcome::Decoy(_) => panic!("hit must return Real"),
    }
}

#[test]
fn proof_miss_returns_decoy() {
    let table = encode_key_table(&[]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &[]);
    let ds = DataSection::parse(&blob).unwrap();
    let host = MockHost::default();
    let req = ProofRequest {
        retrieval_key: Bytes32([0x99; 32]),
        root_hash: None,
        client_nonce: [0u8; 32],
    };
    let gc = gate_config();
    assert!(matches!(
        serve_proof(&host, &ds, &req, &gc),
        ProofOutcome::Decoy(_)
    ));
}

use digstore_guest::content::verify_request_jwt;
use digstore_guest::jwt::ClaimPolicy;

fn b64url(b: &[u8]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(b)
}
fn make_jwt(header: &str, payload: &str) -> Vec<u8> {
    let mut s = b64url(header.as_bytes());
    s.push('.');
    s.push_str(&b64url(payload.as_bytes()));
    s.push('.');
    s.push_str(&b64url(b"sig"));
    s.into_bytes()
}

#[test]
fn expired_jwt_fails_claim_check() {
    // Claim check alone (signature skipped here) must reject an expired token,
    // which serve_content turns into a decoy.
    let jwt = make_jwt(
        r#"{"alg":"ES256"}"#,
        r#"{"exp":1000,"iss":"acme","aud":"dig"}"#,
    );
    let policy = ClaimPolicy {
        now: 5000,
        expected_iss: Some("acme"),
        expected_aud: Some("dig"),
    };
    assert!(verify_request_jwt(&jwt, &policy).is_err());
}

#[test]
fn valid_jwt_passes_claim_check() {
    let jwt = make_jwt(
        r#"{"alg":"ES256"}"#,
        r#"{"exp":9999,"iss":"acme","aud":"dig"}"#,
    );
    let policy = ClaimPolicy {
        now: 5000,
        expected_iss: Some("acme"),
        expected_aud: Some("dig"),
    };
    assert!(verify_request_jwt(&jwt, &policy).is_ok());
}

#[test]
fn require_jwt_without_token_returns_decoy() {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();
    let host = MockHost::default();
    let mut gc = gate_config();
    gc.require_jwt = true;
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None, // required but absent
        window: None,
    };
    assert!(matches!(
        serve_content(&host, &ds, &req, &gc),
        ContentOutcome::Decoy(_)
    ));
}
