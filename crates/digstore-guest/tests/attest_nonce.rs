//! D-ATTEST-NONCE (§12.1/§12.2): the serving gate MUST hand
//! `host_create_attestation` a real `AttestationChallenge` built from the FRESH
//! random nonce, the store_id, and the current timestamp — NOT a hardcoded
//! literal. These tests pin that the exact bytes the gate passes to
//! `create_attestation` equal `build_challenge(nonce, store_id, time)`, and that
//! `verify_attestation` (and therefore the gate) accepts ONLY a host signature
//! over that exact challenge.

mod fixtures;

use digstore_core::{Bytes32, KeyTableEntry};
use digstore_guest::attestation::build_challenge;
use digstore_guest::content::{serve_content, ContentOutcome, GateConfig};
use digstore_guest::datasection::{encode_blob, encode_key_table, DataSection, SectionId};
use digstore_guest::host::{DigHost, HostResult};
use digstore_guest::request::ContentRequest;

use digstore_core::codec::{Encode, Encoder};
use std::cell::RefCell;

const STORE_ID: [u8; 32] = [0xAA; 32];
const ROOT: [u8; 32] = [0xBB; 32];
const HOST_TIME: u64 = 1_700_000_000;

/// Build a data-section blob carrying StoreId, CurrentRoot, a TrustedKeys section
/// (id 5, compiler codec: u32 BE count then `[u8;48]` key + `String` label),
/// KeyTable, and ChunkPool.
fn section_with_trusted(table: &[u8], pool: &[u8], trusted_pubkeys: &[[u8; 48]]) -> Vec<u8> {
    let mut enc = Encoder::new();
    (trusted_pubkeys.len() as u32).encode(&mut enc);
    for pk in trusted_pubkeys {
        pk.encode(&mut enc);
        String::from("dig-host-key-v1").encode(&mut enc);
    }
    let trusted_body = enc.finish();
    encode_blob(&[
        (SectionId::StoreId as u16, STORE_ID.to_vec()),
        (SectionId::CurrentRoot as u16, ROOT.to_vec()),
        (SectionId::TrustedKeys as u16, trusted_body),
        (SectionId::KeyTable as u16, table.to_vec()),
        (SectionId::ChunkPool as u16, pool.to_vec()),
    ])
}

/// What the deterministic `random_bytes` ramp returns on the FIRST call (n = 0):
/// byte i = (0*31 + i) as u8 = i. The gate draws the nonce on its first random
/// call, so this is the exact nonce the gate uses.
fn expected_first_nonce() -> [u8; 32] {
    let mut n = [0u8; 32];
    for (i, b) in n.iter_mut().enumerate() {
        *b = i as u8;
    }
    n
}

/// A host double that (a) records the EXACT challenge bytes the gate hands it,
/// and (b) signs `sign_over` with a real Chia AugScheme BLS key. When
/// `sign_over` is `None` it signs the captured challenge verbatim; otherwise it
/// signs the supplied (different) bytes to prove the verifier is challenge-bound.
struct CapturingSigningHost {
    secret: digstore_crypto::bls::SecretKey,
    pubkey: [u8; 48],
    captured: RefCell<Option<Vec<u8>>>,
    sign_over: Option<Vec<u8>>,
    rand: std::cell::Cell<u32>,
}

impl CapturingSigningHost {
    fn new(seed: &[u8; 32], sign_over: Option<Vec<u8>>) -> Self {
        let secret = digstore_crypto::bls::SecretKey::from_seed(seed);
        let pubkey = secret.public_key().to_bytes().0;
        CapturingSigningHost {
            secret,
            pubkey,
            captured: RefCell::new(None),
            sign_over,
            rand: std::cell::Cell::new(0),
        }
    }
}

impl DigHost for CapturingSigningHost {
    fn get_public_key(&self) -> HostResult {
        Ok(self.pubkey.to_vec())
    }
    fn create_attestation(&self, challenge: &[u8]) -> HostResult {
        // Record exactly what the gate handed us.
        *self.captured.borrow_mut() = Some(challenge.to_vec());
        // Sign either the captured challenge or an explicitly different message.
        let msg: &[u8] = match &self.sign_over {
            Some(m) => m,
            None => challenge,
        };
        let sig = digstore_crypto::bls::bls_sign(&self.secret, msg).0;
        let mut resp = Vec::with_capacity(48 + 32 + 96);
        resp.extend_from_slice(&self.pubkey);
        resp.extend_from_slice(&[0x11u8; 32]); // host_instance_id (not signed)
        resp.extend_from_slice(&sig);
        Ok(resp)
    }
    fn establish_session(&self, _c: &[u8]) -> HostResult {
        Ok(vec![1u8; 16])
    }
    fn verify_session(&self) -> bool {
        true
    }
    fn jwks_fetch(&self, _u: &[u8]) -> HostResult {
        Ok(b"{}".to_vec())
    }
    fn current_time(&self) -> u64 {
        HOST_TIME
    }
    fn random_bytes(&self, count: u32) -> HostResult {
        let n = self.rand.get();
        self.rand.set(n + 1);
        Ok((0..count)
            .map(|i| (n.wrapping_mul(31).wrapping_add(i)) as u8)
            .collect())
    }
}

fn one_entry_fixture(
    host: &CapturingSigningHost,
) -> (DataSection<'static>, ContentRequest, GateConfig) {
    // Materialize the section into a Box so it lives for the test; we leak it to
    // get a 'static slice — fine for a unit test.
    let key = Bytes32([0x11; 32]);
    let entry = KeyTableEntry {
        static_key: key,
        generation: Bytes32(ROOT),
        chunk_indices: vec![0],
        total_size: 5,
    };
    let table = encode_key_table(&[entry]);
    let pool = fixtures::pack_pool(&[b"alpha"]);
    let blob = section_with_trusted(&table, &pool, &[host.pubkey]);
    let blob: &'static [u8] = Box::leak(blob.into_boxed_slice());
    let ds = DataSection::parse(blob).unwrap();
    let req = ContentRequest {
        retrieval_key: key,
        root_hash: None,
        range: None,
        jwt: None,
        window: None,
    };
    let gc = GateConfig {
        require_attestation: true,
        require_jwt: false,
        expected_iss: None,
        expected_aud: None,
    };
    (ds, req, gc)
}

#[test]
fn gate_signs_real_challenge_built_from_nonce_store_id_time() {
    // The gate must hand create_attestation exactly
    //   build_challenge(fresh_nonce, store_id, current_time)
    // — never a hardcoded literal like b"challenge".
    let host = CapturingSigningHost::new(&[42u8; 32], None);
    let (ds, req, gc) = one_entry_fixture(&host);

    let outcome = serve_content(&host, &ds, &req, &gc);

    let captured = host
        .captured
        .borrow()
        .clone()
        .expect("gate must call create_attestation");

    let expected = build_challenge(expected_first_nonce(), STORE_ID, HOST_TIME);
    assert_eq!(
        captured, expected,
        "gate must sign build_challenge(nonce, store_id, time), not a literal"
    );
    // Sanity: it is the tagged challenge wire (SECURITY.md residual #2):
    // ATTEST_DST || nonce(32)+store_id(32)+time(8), not the 9-byte literal.
    let tl = digstore_core::ATTEST_DST.len();
    assert_eq!(
        captured.len(),
        tl + 72,
        "challenge must be ATTEST_DST || nonce(32)+store_id(32)+time(8)"
    );
    assert_eq!(
        &captured[..tl],
        digstore_core::ATTEST_DST,
        "challenge must carry the per-role attestation domain tag"
    );
    assert_ne!(
        captured.as_slice(),
        b"challenge",
        "the signed message must NOT be the hardcoded literal"
    );
    // The store_id and timestamp must actually come from the data section / host
    // (after the leading role tag).
    assert_eq!(
        &captured[tl + 32..tl + 64],
        &STORE_ID,
        "challenge embeds the store_id"
    );
    assert_eq!(
        &captured[tl + 64..tl + 72],
        &HOST_TIME.to_be_bytes(),
        "challenge embeds the current timestamp"
    );

    // A valid host signature over that exact challenge releases real content.
    assert!(
        matches!(outcome, ContentOutcome::Real(_)),
        "a signature over the real challenge from a trusted key must release content"
    );
}

#[test]
fn gate_accepts_only_a_signature_over_the_exact_challenge() {
    // Same trusted key, fresh + correct shape, but the host signs a DIFFERENT
    // message (the old literal b\"challenge\") instead of the real challenge.
    // §12.2 verification binds the signature to the challenge, so this MUST be a
    // Decoy — proving the gate accepts only a signature over the exact challenge.
    let host = CapturingSigningHost::new(&[42u8; 32], Some(b"challenge".to_vec()));
    let (ds, req, gc) = one_entry_fixture(&host);

    let outcome = serve_content(&host, &ds, &req, &gc);

    // The gate still built and handed over the real challenge...
    let captured = host
        .captured
        .borrow()
        .clone()
        .expect("gate must call create_attestation");
    let expected = build_challenge(expected_first_nonce(), STORE_ID, HOST_TIME);
    assert_eq!(captured, expected, "gate still builds the real challenge");

    // ...but because the host signed other bytes, verification fails closed.
    assert!(
        matches!(outcome, ContentOutcome::Decoy(_)),
        "a signature over bytes other than the exact challenge MUST yield a Decoy"
    );
}
