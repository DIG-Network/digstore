//! Wire structs shared across host/guest/remote (paper 9.1, 9.2, 9.3, 9.5).

use crate::bytes::{Bytes32, Bytes48, Bytes96};
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
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
