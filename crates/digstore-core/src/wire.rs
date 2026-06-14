//! Wire structs shared across host/guest/remote (paper 9.1, 9.2, 9.3, 9.5).

use crate::bytes::{Bytes32, Bytes48, Bytes96};
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::merkle::MerkleProof;
use alloc::string::String;
use alloc::vec::Vec;

/// Reference to a Chia block used to anchor a proof in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChiaBlockRef {
    pub header_hash: Bytes32,
    pub height: u32,
    pub timestamp: u64,
}

impl Encode for ChiaBlockRef {
    fn encode(&self, enc: &mut Encoder) {
        self.header_hash.encode(enc);
        self.height.encode(enc);
        self.timestamp.encode(enc);
    }
}

impl Decode for ChiaBlockRef {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(ChiaBlockRef {
            header_hash: Bytes32::decode(dec)?,
            height: u32::decode(dec)?,
            timestamp: u64::decode(dec)?,
        })
    }
}

/// A ZK execution proof of a faithful re-execution of the serving computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProof {
    pub program_hash: Bytes32,
    pub public_input: Vec<u8>,
    pub public_output: Bytes32,
    pub proof: Vec<u8>,
    pub chia_block: ChiaBlockRef,
    pub node_pubkey: Bytes48,
    pub node_signature: Bytes96,
}

impl Encode for ExecutionProof {
    fn encode(&self, enc: &mut Encoder) {
        self.program_hash.encode(enc);
        self.public_input.encode(enc);
        self.public_output.encode(enc);
        self.proof.encode(enc);
        self.chia_block.encode(enc);
        self.node_pubkey.encode(enc);
        self.node_signature.encode(enc);
    }
}

impl Decode for ExecutionProof {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(ExecutionProof {
            program_hash: Bytes32::decode(dec)?,
            public_input: Vec::<u8>::decode(dec)?,
            public_output: Bytes32::decode(dec)?,
            proof: Vec::<u8>::decode(dec)?,
            chia_block: ChiaBlockRef::decode(dec)?,
            node_pubkey: Bytes48::decode(dec)?,
            node_signature: Bytes96::decode(dec)?,
        })
    }
}

/// Response carrying an execution proof plus the active root hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofResponse {
    pub proof: ExecutionProof,
    pub roothash: Bytes32,
}

impl Encode for ProofResponse {
    fn encode(&self, enc: &mut Encoder) {
        self.proof.encode(enc);
        self.roothash.encode(enc);
    }
}

impl Decode for ProofResponse {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(ProofResponse {
            proof: ExecutionProof::decode(dec)?,
            roothash: Bytes32::decode(dec)?,
        })
    }
}

/// CONVENTIONS C3: the guest cannot build an `ExecutionProof` (no prover, no
/// ChainSource, no node signing key inside wasm32). Its `get_proof` therefore
/// returns this `ProofPrelude`; the serving host turns it into a full
/// `ExecutionProof` via `digstore_prover`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofPrelude {
    pub roothash: Bytes32,
    /// SHA-256 of the served output bytes (same bytes `get_content` returns).
    pub output_commitment: Bytes32,
    /// Commitment over (retrieval_key, ordered chunk indices).
    pub serving_digest: Bytes32,
}

impl Encode for ProofPrelude {
    fn encode(&self, enc: &mut Encoder) {
        self.roothash.encode(enc);
        self.output_commitment.encode(enc);
        self.serving_digest.encode(enc);
    }
}

impl Decode for ProofPrelude {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(ProofPrelude {
            roothash: Bytes32::decode(dec)?,
            output_commitment: Bytes32::decode(dec)?,
            serving_digest: Bytes32::decode(dec)?,
        })
    }
}

/// Content (or decoy) response. Decoy uses this exact shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentResponse {
    pub ciphertext: Vec<u8>,
    pub merkle_proof: MerkleProof,
    pub roothash: Bytes32,
    /// Per-chunk CIPHERTEXT byte lengths, in order, of `ciphertext` (which is the PLAIN
    /// concatenation of the resource's chunk ciphertexts — D5/C9, no length framing in the
    /// bytes themselves). A streaming client splits `ciphertext` by these lengths and
    /// AES-256-GCM-SIV-opens each chunk; without them a multi-chunk resource (>~64 KiB) cannot
    /// be decrypted client-side. Empty for a single-chunk resource. NOT covered by the merkle
    /// leaf (`leaf == sha256(ciphertext)`): this is serving metadata, not content.
    ///
    /// Appended AFTER `roothash` so the wire stays backward-compatible: a module compiled by a
    /// pre-chunk-lens producer emits nothing here, and `decode` reads it only if bytes remain.
    pub chunk_lens: Vec<u32>,
}

impl Encode for ContentResponse {
    fn encode(&self, enc: &mut Encoder) {
        self.ciphertext.encode(enc);
        self.merkle_proof.encode(enc);
        self.roothash.encode(enc);
        self.chunk_lens.encode(enc);
    }
}

impl Decode for ContentResponse {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let ciphertext = Vec::<u8>::decode(dec)?;
        let merkle_proof = MerkleProof::decode(dec)?;
        let roothash = Bytes32::decode(dec)?;
        // Backward-compat: a module from a pre-chunk-lens producer has no trailing bytes here.
        let chunk_lens = if dec.remaining() > 0 {
            Vec::<u32>::decode(dec)?
        } else {
            Vec::new()
        };
        Ok(ContentResponse {
            ciphertext,
            merkle_proof,
            roothash,
            chunk_lens,
        })
    }
}

/// Per-role BLS domain-separation tag for host attestation signatures
/// (SECURITY.md residual #2). Prepended to the signed attestation message so an
/// attestation signature cannot be replayed as a push or node-proof signature.
///
/// This is the SINGLE SOURCE OF TRUTH shared by the producer
/// (`digstore_crypto::bls::attestation_signing_message`, via the re-export
/// `digstore_crypto::bls::ATTEST_DST`) and the verifier (the guest's
/// `build_challenge`), so the bytes the host signs and the bytes the guest
/// verifies stay byte-identical.
pub const ATTEST_DST: &[u8] = b"digstore:attest:v1";

/// Challenge issued to a host during attestation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationChallenge {
    pub nonce: [u8; 32],
    pub store_id: [u8; 32],
    pub timestamp: u64,
}

impl Encode for AttestationChallenge {
    fn encode(&self, enc: &mut Encoder) {
        enc.write_bytes(&self.nonce);
        enc.write_bytes(&self.store_id);
        self.timestamp.encode(enc);
    }
}

impl Decode for AttestationChallenge {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let nonce = <[u8; 32]>::decode(dec)?;
        let store_id = <[u8; 32]>::decode(dec)?;
        let timestamp = u64::decode(dec)?;
        Ok(AttestationChallenge {
            nonce,
            store_id,
            timestamp,
        })
    }
}

/// A host's signed attestation response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationResponse {
    pub host_public_key: [u8; 48],
    pub host_instance_id: [u8; 32],
    pub signature: [u8; 96],
}

impl Encode for AttestationResponse {
    fn encode(&self, enc: &mut Encoder) {
        enc.write_bytes(&self.host_public_key);
        enc.write_bytes(&self.host_instance_id);
        enc.write_bytes(&self.signature);
    }
}

impl Decode for AttestationResponse {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let host_public_key = <[u8; 48]>::decode(dec)?;
        let host_instance_id = <[u8; 32]>::decode(dec)?;
        let signature = <[u8; 96]>::decode(dec)?;
        Ok(AttestationResponse {
            host_public_key,
            host_instance_id,
            signature,
        })
    }
}

/// Authentication requirements advertised by a store (get_authentication_info).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationInfo {
    pub requires_session: bool,
    pub requires_jwt: bool,
    pub jwks_url: Option<String>,
    pub accepted_algorithms: Vec<String>,
}

impl Encode for AuthenticationInfo {
    fn encode(&self, enc: &mut Encoder) {
        (self.requires_session as u8).encode(enc);
        (self.requires_jwt as u8).encode(enc);
        self.jwks_url.encode(enc);
        self.accepted_algorithms.encode(enc);
    }
}

impl Decode for AuthenticationInfo {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let requires_session = u8::decode(dec)? != 0;
        let requires_jwt = u8::decode(dec)? != 0;
        let jwks_url = Option::<String>::decode(dec)?;
        let accepted_algorithms = Vec::<String>::decode(dec)?;
        Ok(AuthenticationInfo {
            requires_session,
            requires_jwt,
            jwks_url,
            accepted_algorithms,
        })
    }
}
