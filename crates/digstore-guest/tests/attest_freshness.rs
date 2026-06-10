//! D-ATTEST-FRESHNESS (§12.2): the serving gate MUST check the timestamp bound
//! into the attestation challenge for freshness. §12.1 puts a `timestamp: u64`
//! ("unix seconds, for freshness") into the `AttestationChallenge`; §12.2 says
//! the module "verifies the BLS signature over the challenge under
//! host_public_key, checks the timestamp for freshness, and checks that
//! host_public_key is a member of the trusted set."
//!
//! `verify_attestation` already enforces FRESHNESS_SECS = 300. These tests pin
//! that the gate feeds it the timestamp it actually bound into the challenge
//! (compared against the module's current clock), so a stale challenge timestamp
//! (older than 300s relative to the module clock at verification) fails closed to
//! a Decoy, while a fresh one releases content. This proves the gate does NOT
//! short-circuit freshness by comparing the timestamp against itself.

mod fixtures;

use digstore_core::{Bytes32, KeyTableEntry};
use digstore_guest::content::{serve_content, ContentOutcome, GateConfig};
use digstore_guest::datasection::{encode_blob, encode_key_table, DataSection, SectionId};
use digstore_guest::host::{DigHost, HostResult};
use digstore_guest::request::ContentRequest;

use digstore_core::codec::{Encode, Encoder};
use std::cell::RefCell;

const STORE_ID: [u8; 32] = [0xAA; 32];
const ROOT: [u8; 32] = [0xBB; 32];

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

/// A trusted host that signs the exact challenge it is handed (so BLS verify and
/// trusted-set membership both pass), but whose clock JUMPS forward by
/// `clock_jump` seconds AFTER it issues the challenge. This isolates the
/// freshness check: the challenge carries the time at issuance, and the module's
/// clock at verification is `issue_time + clock_jump`.
struct ClockJumpHost {
    secret: digstore_crypto::bls::SecretKey,
    pubkey: [u8; 48],
    base_time: u64,
    clock_jump: u64,
    signed: std::cell::Cell<bool>,
    rand: std::cell::Cell<u32>,
    captured: RefCell<Option<Vec<u8>>>,
}

impl ClockJumpHost {
    fn new(seed: &[u8; 32], base_time: u64, clock_jump: u64) -> Self {
        let secret = digstore_crypto::bls::SecretKey::from_seed(seed);
        let pubkey = secret.public_key().to_bytes().0;
        ClockJumpHost {
            secret,
            pubkey,
            base_time,
            clock_jump,
            signed: std::cell::Cell::new(false),
            rand: std::cell::Cell::new(0),
            captured: RefCell::new(None),
        }
    }
}

impl DigHost for ClockJumpHost {
    fn get_public_key(&self) -> HostResult {
        Ok(self.pubkey.to_vec())
    }
    fn create_attestation(&self, challenge: &[u8]) -> HostResult {
        *self.captured.borrow_mut() = Some(challenge.to_vec());
        // Sign the exact challenge so membership + BLS verify pass; only freshness
        // can fail. After the host has signed, its clock has moved forward.
        let sig = digstore_crypto::bls::bls_sign(&self.secret, challenge).0;
        self.signed.set(true);
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
        // Before the host signs the challenge, time is `base_time` (this is the
        // timestamp bound into the challenge). After signing, the module's clock
        // has advanced by `clock_jump` seconds.
        if self.signed.get() {
            self.base_time + self.clock_jump
        } else {
            self.base_time
        }
    }
    fn random_bytes(&self, count: u32) -> HostResult {
        let n = self.rand.get();
        self.rand.set(n + 1);
        Ok((0..count)
            .map(|i| (n.wrapping_mul(31).wrapping_add(i)) as u8)
            .collect())
    }
}

fn one_entry_fixture(host: &ClockJumpHost) -> (DataSection<'static>, ContentRequest, GateConfig) {
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
fn fresh_challenge_timestamp_releases_content() {
    // Clock jump of 0: the challenge timestamp equals the verification clock, so
    // freshness trivially holds. A trusted key over the exact challenge -> Real.
    let host = ClockJumpHost::new(&[7u8; 32], 1_700_000_000, 0);
    let (ds, req, gc) = one_entry_fixture(&host);
    let outcome = serve_content(&host, &ds, &req, &gc);
    assert!(
        matches!(outcome, ContentOutcome::Real(_)),
        "a fresh, trusted, correctly-signed attestation must release content"
    );
}

#[test]
fn near_edge_fresh_challenge_releases_content() {
    // 299s < FRESHNESS_SECS (300): still fresh.
    let host = ClockJumpHost::new(&[7u8; 32], 1_700_000_000, 299);
    let (ds, req, gc) = one_entry_fixture(&host);
    let outcome = serve_content(&host, &ds, &req, &gc);
    assert!(
        matches!(outcome, ContentOutcome::Real(_)),
        "a challenge timestamp 299s old (< 300s) is still fresh -> content"
    );
}

#[test]
fn stale_challenge_timestamp_yields_decoy() {
    // The challenge was issued at base_time, but by verification the module's
    // clock has advanced 600s (> FRESHNESS_SECS = 300). Even though the key is
    // trusted and the signature is over the exact challenge, the stale timestamp
    // bound into the challenge MUST fail freshness -> Decoy. This fails if the
    // gate compares the challenge timestamp against itself instead of the
    // module's current clock.
    let host = ClockJumpHost::new(&[7u8; 32], 1_700_000_000, 600);
    let (ds, req, gc) = one_entry_fixture(&host);
    let outcome = serve_content(&host, &ds, &req, &gc);

    // Confirm the gate did build & sign a real challenge (so the only failing
    // gate is freshness, not membership/signature).
    let captured = host
        .captured
        .borrow()
        .clone()
        .expect("gate must call create_attestation");
    assert_eq!(
        captured.len(),
        digstore_core::ATTEST_DST.len() + 72,
        "real tagged challenge (ATTEST_DST || nonce||store_id||time) was issued"
    );

    assert!(
        matches!(outcome, ContentOutcome::Decoy(_)),
        "a challenge timestamp older than 300s relative to the module clock MUST yield a Decoy"
    );
}
