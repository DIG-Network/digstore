use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Wire input from the host prover. MUST be byte-identical to the host-side
/// `GuestInput` in `risc0_backend.rs`.
#[derive(Serialize, Deserialize)]
struct GuestInput {
    program_hash: [u8; 32],
    public_input: Vec<u8>,
    roothash: [u8; 32],
    chunks: Vec<Vec<u8>>,
}

fn main() {
    let input: GuestInput = env::read();

    // Deterministic serving computation: gather + concatenate the ciphertext,
    // then commit SHA-256(roothash || concat) — identical to the host.
    let mut preimage = Vec::new();
    preimage.extend_from_slice(&input.roothash);
    for c in &input.chunks {
        preimage.extend_from_slice(c);
    }
    let public_output: [u8; 32] = Sha256::digest(&preimage).into();
    let public_input_hash: [u8; 32] = Sha256::digest(&input.public_input).into();

    // Journal: (program_hash, public_input_hash, roothash, public_output)
    env::commit(&(input.program_hash, public_input_hash, input.roothash, public_output));
}
