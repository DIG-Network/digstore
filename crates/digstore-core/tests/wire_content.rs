use digstore_core::codec::{Decode, Encode};
use digstore_core::merkle::{MerkleProof, ProofStep};
use digstore_core::wire::{
    AttestationChallenge, AttestationResponse, AuthenticationInfo, ContentResponse, ProofPrelude,
};
use digstore_core::Bytes32;

fn sample_merkle_proof() -> MerkleProof {
    MerkleProof {
        leaf: Bytes32([1; 32]),
        path: vec![ProofStep {
            hash: Bytes32([2; 32]),
            is_left: true,
        }],
        root: Bytes32([3; 32]),
    }
}

#[test]
fn content_response_roundtrip() {
    let r = ContentResponse {
        ciphertext: vec![10, 20, 30, 40],
        merkle_proof: sample_merkle_proof(),
        roothash: Bytes32([4; 32]),
        chunk_lens: vec![4],
    };
    assert_eq!(ContentResponse::from_bytes(&r.to_bytes()).unwrap(), r);
}

#[test]
fn attestation_challenge_roundtrip() {
    let c = AttestationChallenge {
        nonce: [1; 32],
        store_id: [2; 32],
        timestamp: 999,
    };
    assert_eq!(AttestationChallenge::from_bytes(&c.to_bytes()).unwrap(), c);
}

#[test]
fn attestation_response_roundtrip() {
    let r = AttestationResponse {
        host_public_key: [3; 48],
        host_instance_id: [4; 32],
        signature: [5; 96],
    };
    assert_eq!(AttestationResponse::from_bytes(&r.to_bytes()).unwrap(), r);
}

#[test]
fn authentication_info_roundtrip() {
    let a = AuthenticationInfo {
        requires_session: true,
        requires_jwt: false,
        jwks_url: Some("https://issuer/.well-known/jwks.json".into()),
        accepted_algorithms: vec!["RS256".into(), "ES256".into()],
    };
    assert_eq!(AuthenticationInfo::from_bytes(&a.to_bytes()).unwrap(), a);
}

#[test]
fn proof_prelude_roundtrip() {
    // C3: guest get_proof returns a ProofPrelude (NOT a finished ExecutionProof).
    let p = ProofPrelude {
        roothash: Bytes32([6; 32]),
        output_commitment: Bytes32([7; 32]),
        serving_digest: Bytes32([8; 32]),
    };
    assert_eq!(ProofPrelude::from_bytes(&p.to_bytes()).unwrap(), p);
}
