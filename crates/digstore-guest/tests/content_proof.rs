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
    let host = MockHost {
        time: 50, // before window
        ..MockHost::default()
    };
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
fn attestation_with_no_embedded_trusted_set_returns_decoy() {
    // §12.2/§12.3 (D-ATTEST-TRUSTSET): if the module embeds NO TrustedKeys
    // section, the gate must load an empty trusted set and fail closed — a
    // host signing with a perfectly valid key is still not a *member* of the
    // (empty) embedded set, so content calls return a Decoy, never real bytes.
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    // Build a blob WITHOUT a TrustedKeys (id 5) section.
    let blob = fixtures::section_keytable_and_pool([0xAA; 32], [0xBB; 32], &table, &pool);
    let ds = DataSection::parse(&blob).unwrap();
    assert!(
        ds.section(SectionId::TrustedKeys).is_none(),
        "fixture must omit the TrustedKeys section for this case"
    );

    let host = SigningHost::new(&[42u8; 32]); // signs validly, but not embedded
    let mut gc = gate_config();
    gc.require_attestation = true;
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    assert!(
        matches!(
            serve_content(&host, &ds, &req, &gc),
            ContentOutcome::Decoy(_)
        ),
        "no embedded trusted set MUST fail closed -> Decoy"
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
    let host = MockHost {
        attestation: Err(digstore_core::ErrorCode::AttestationFailed),
        ..MockHost::default()
    };
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

// --- D-JWT-VERIFY (Residual #4): the JWT gate must verify the token's RS256
// signature against the store's trusted JWKS, not just its claims. A token with
// a valid signature from a trusted key + valid claims releases real content; a
// tampered signature, a signature from a non-trusted key, or an absent token all
// fail closed -> Decoy. The private RSA key lives ONLY in the test (signing); the
// guest verifies with the public JWK reconstructed from (n, e). ---------------

const JWKS_URL: &str = "https://issuer.example/jwks.json";

/// Deterministic RSA keygen for test speed (seeded; never used in production).
fn rsa_keypair(seed: u8) -> rsa::RsaPrivateKey {
    use rsa::rand_core::SeedableRng;
    let mut rng = rand_chacha::ChaCha8Rng::from_seed([seed; 32]);
    rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap()
}

/// Build a single-key JWKS JSON document for an RSA public key under `kid`.
fn jwks_json(priv_key: &rsa::RsaPrivateKey, kid: &str) -> Vec<u8> {
    use rsa::traits::PublicKeyParts;
    let pk = priv_key.to_public_key();
    let n = b64url(&pk.n().to_bytes_be());
    let e = b64url(&pk.e().to_bytes_be());
    format!(r#"{{"keys":[{{"kty":"RSA","kid":"{kid}","alg":"RS256","n":"{n}","e":"{e}"}}]}}"#)
        .into_bytes()
}

/// Build a real RS256 JWT (header.payload.signature) signed with `priv_key`.
fn signed_rs256_jwt(priv_key: &rsa::RsaPrivateKey, kid: &str, payload: &str) -> Vec<u8> {
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    use sha2::Sha256;

    let header = format!(r#"{{"alg":"RS256","kid":"{kid}"}}"#);
    let mut signing_input = b64url(header.as_bytes());
    signing_input.push('.');
    signing_input.push_str(&b64url(payload.as_bytes()));

    let signing_key = SigningKey::<Sha256>::new(priv_key.clone());
    let sig = signing_key.sign(signing_input.as_bytes()).to_bytes();

    let mut jwt = signing_input.into_bytes();
    jwt.push(b'.');
    jwt.extend_from_slice(b64url(&sig).as_bytes());
    jwt
}

/// Data-section blob carrying KeyTable + ChunkPool + an AuthInfo section that
/// advertises `requires_jwt` and the JWKS URL the gate fetches from.
fn section_with_authinfo(
    store_id: [u8; 32],
    root: [u8; 32],
    table: &[u8],
    pool: &[u8],
    jwks_url: Option<&str>,
) -> Vec<u8> {
    use digstore_core::AuthenticationInfo;
    let info = AuthenticationInfo {
        requires_session: true,
        requires_jwt: true,
        jwks_url: jwks_url.map(String::from),
        accepted_algorithms: vec![String::from("RS256")],
    };
    let mut enc = Encoder::new();
    info.encode(&mut enc);
    let auth_body = enc.finish();
    encode_blob(&[
        (SectionId::StoreId as u16, store_id.to_vec()),
        (SectionId::CurrentRoot as u16, root.to_vec()),
        (SectionId::KeyTable as u16, table.to_vec()),
        (SectionId::ChunkPool as u16, pool.to_vec()),
        (SectionId::AuthInfo as u16, auth_body),
    ])
}

/// A host double that serves a scripted JWKS document and has an active session.
struct JwksHost {
    jwks: Vec<u8>,
    session_ok: bool,
    time: u64,
    rand: std::cell::Cell<u32>,
}

impl JwksHost {
    fn new(jwks: Vec<u8>) -> Self {
        JwksHost {
            jwks,
            session_ok: true,
            time: 1_700_000_000,
            rand: std::cell::Cell::new(0),
        }
    }
}

impl digstore_guest::host::DigHost for JwksHost {
    fn get_public_key(&self) -> digstore_guest::host::HostResult {
        Ok(vec![0xAB; 48])
    }
    fn create_attestation(&self, _c: &[u8]) -> digstore_guest::host::HostResult {
        Ok(vec![0u8; 176])
    }
    fn establish_session(&self, _c: &[u8]) -> digstore_guest::host::HostResult {
        Ok(vec![1u8; 16])
    }
    fn verify_session(&self) -> bool {
        self.session_ok
    }
    fn jwks_fetch(&self, _u: &[u8]) -> digstore_guest::host::HostResult {
        Ok(self.jwks.clone())
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

/// Build the standard one-resource fixture and run the JWT-gated content path.
fn jwt_gate_outcome(
    host: &JwksHost,
    jwt: Option<Vec<u8>>,
    jwks_url: Option<&str>,
) -> ContentOutcome {
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32([0xBB; 32]),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    let blob = section_with_authinfo([0xAA; 32], [0xBB; 32], &table, &pool, jwks_url);
    let ds = DataSection::parse(&blob).unwrap();
    let mut gc = gate_config();
    gc.require_jwt = true;
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt,
        window: None,
    };
    serve_content(host, &ds, &req, &gc)
}

#[test]
fn valid_signature_and_claims_returns_real() {
    let priv_key = rsa_keypair(21);
    let jwt = signed_rs256_jwt(&priv_key, "k1", r#"{"exp":9999999999,"nbf":0}"#);
    let host = JwksHost::new(jwks_json(&priv_key, "k1"));
    assert!(
        matches!(
            jwt_gate_outcome(&host, Some(jwt), Some(JWKS_URL)),
            ContentOutcome::Real(_)
        ),
        "valid RS256 signature from a trusted JWKS key + valid claims must release real content"
    );
}

#[test]
fn tampered_signature_returns_decoy() {
    let priv_key = rsa_keypair(21);
    let mut jwt = signed_rs256_jwt(&priv_key, "k1", r#"{"exp":9999999999,"nbf":0}"#);
    // Flip a byte in the trailing signature segment.
    let last = jwt.len() - 1;
    jwt[last] ^= 0x01;
    let host = JwksHost::new(jwks_json(&priv_key, "k1"));
    assert!(
        matches!(
            jwt_gate_outcome(&host, Some(jwt), Some(JWKS_URL)),
            ContentOutcome::Decoy(_)
        ),
        "a tampered RS256 signature MUST yield a Decoy even with valid-looking claims"
    );
}

#[test]
fn signature_from_wrong_key_returns_decoy() {
    // Token signed by key A, but the JWKS advertises key B under the same kid.
    let signer = rsa_keypair(21);
    let other = rsa_keypair(99);
    let jwt = signed_rs256_jwt(&signer, "k1", r#"{"exp":9999999999,"nbf":0}"#);
    let host = JwksHost::new(jwks_json(&other, "k1"));
    assert!(
        matches!(
            jwt_gate_outcome(&host, Some(jwt), Some(JWKS_URL)),
            ContentOutcome::Decoy(_)
        ),
        "a signature that does not verify under the trusted JWKS key MUST yield a Decoy"
    );
}

#[test]
fn unknown_kid_returns_decoy() {
    // Token's kid is not present in the JWKS -> no verifying key -> Decoy.
    let priv_key = rsa_keypair(21);
    let jwt = signed_rs256_jwt(&priv_key, "missing", r#"{"exp":9999999999,"nbf":0}"#);
    let host = JwksHost::new(jwks_json(&priv_key, "k1"));
    assert!(matches!(
        jwt_gate_outcome(&host, Some(jwt), Some(JWKS_URL)),
        ContentOutcome::Decoy(_)
    ));
}

#[test]
fn absent_jwks_url_returns_decoy() {
    // AuthInfo requests a JWT but advertises no JWKS endpoint -> no key source ->
    // the gate fails closed even with a perfectly valid signature.
    let priv_key = rsa_keypair(21);
    let jwt = signed_rs256_jwt(&priv_key, "k1", r#"{"exp":9999999999,"nbf":0}"#);
    let host = JwksHost::new(jwks_json(&priv_key, "k1"));
    assert!(matches!(
        jwt_gate_outcome(&host, Some(jwt), None),
        ContentOutcome::Decoy(_)
    ));
}

#[test]
fn expired_jwt_returns_decoy_despite_valid_signature() {
    // Correct signature, but the claim check rejects the expired token first.
    let priv_key = rsa_keypair(21);
    let jwt = signed_rs256_jwt(&priv_key, "k1", r#"{"exp":1000,"nbf":0}"#);
    let host = JwksHost::new(jwks_json(&priv_key, "k1")); // host clock = 1_700_000_000
    assert!(matches!(
        jwt_gate_outcome(&host, Some(jwt), Some(JWKS_URL)),
        ContentOutcome::Decoy(_)
    ));
}

#[test]
fn expired_jwt_fails_verify_request_jwt() {
    // verify_request_jwt: claim check rejects an expired token before any fetch.
    let priv_key = rsa_keypair(21);
    let jwt = signed_rs256_jwt(&priv_key, "k1", r#"{"exp":1000,"iss":"acme","aud":"dig"}"#);
    let host = JwksHost::new(jwks_json(&priv_key, "k1"));
    let policy = ClaimPolicy {
        now: 5000,
        expected_iss: Some("acme"),
        expected_aud: Some("dig"),
    };
    assert!(verify_request_jwt(&host, JWKS_URL.as_bytes(), &jwt, &policy).is_err());
}

#[test]
fn valid_jwt_passes_verify_request_jwt() {
    // verify_request_jwt: valid claims AND a verifiable signature pass.
    let priv_key = rsa_keypair(21);
    let jwt = signed_rs256_jwt(&priv_key, "k1", r#"{"exp":9999,"iss":"acme","aud":"dig"}"#);
    let host = JwksHost::new(jwks_json(&priv_key, "k1"));
    let policy = ClaimPolicy {
        now: 5000,
        expected_iss: Some("acme"),
        expected_aud: Some("dig"),
    };
    assert!(verify_request_jwt(&host, JWKS_URL.as_bytes(), &jwt, &policy).is_ok());
}

#[test]
fn require_jwt_without_token_returns_decoy() {
    let priv_key = rsa_keypair(21);
    let host = JwksHost::new(jwks_json(&priv_key, "k1"));
    assert!(matches!(
        jwt_gate_outcome(&host, None, Some(JWKS_URL)), // required but absent
        ContentOutcome::Decoy(_)
    ));
}

// --- D-SESSION-JWT-GATE (§12.4): "The session is the precondition for any JWT-
// authorization logic the module chooses to enforce before releasing real
// content." JWT-gated content with NO active session MUST return a Decoy even
// when the presented JWT would otherwise validate. The gate fails closed to
// Decoy when host.verify_session() is false. --------------------------------
#[test]
fn require_jwt_without_active_session_returns_decoy_even_with_valid_jwt() {
    let priv_key = rsa_keypair(21);
    // A JWT whose claims AND signature both validate — so only the missing
    // session can be responsible for the Decoy.
    let jwt = signed_rs256_jwt(&priv_key, "k1", r#"{"exp":9999999999,"nbf":0}"#);
    let mut host = JwksHost::new(jwks_json(&priv_key, "k1"));
    host.session_ok = false; // §12.4: no active session
    assert!(
        matches!(
            jwt_gate_outcome(&host, Some(jwt), Some(JWKS_URL)),
            ContentOutcome::Decoy(_)
        ),
        "JWT-gated content with no active session MUST be a Decoy (§12.4)"
    );
}

// Control: the SAME valid JWT WITH an active session releases real content,
// proving the Decoy above is attributable to the missing session, not the JWT.
#[test]
fn require_jwt_with_active_session_and_valid_jwt_returns_real() {
    let priv_key = rsa_keypair(21);
    let jwt = signed_rs256_jwt(&priv_key, "k1", r#"{"exp":9999999999,"nbf":0}"#);
    let host = JwksHost::new(jwks_json(&priv_key, "k1")); // active session
    assert!(
        matches!(
            jwt_gate_outcome(&host, Some(jwt), Some(JWKS_URL)),
            ContentOutcome::Real(_)
        ),
        "valid JWT with an active session must release real content"
    );
}
