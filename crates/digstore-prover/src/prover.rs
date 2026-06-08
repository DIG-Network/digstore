use crate::chain::ChainSource;
use crate::error::{ProverError, Result};
use crate::serving_inputs::ServingInputs;
use digstore_core::{Bytes32, ExecutionProof, ProofResponse};

/// Produces an [`ExecutionProof`] for a serving run (§13.1-13.3).
///
/// `public_input` is `client_nonce(32) || ChiaBlockRef(codec)` (build via
/// [`crate::commitment::build_public_input`]). `serving_inputs` carries the
/// deterministic serving-computation inputs (deviation #3).
pub trait Prover {
    fn prove(
        &self,
        program_hash: Bytes32,
        public_input: &[u8],
        serving_inputs: &ServingInputs,
    ) -> Result<ExecutionProof>;
}

/// Verifies an [`ExecutionProof`] (§13.4-13.8): program-hash match, ZK /
/// attestation validity, output commitment, node BLS attribution, and chain
/// freshness via `chain`.
pub trait Verifier {
    fn verify(
        &self,
        proof: &ExecutionProof,
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        chain: &dyn ChainSource,
    ) -> Result<()>;

    /// Verify a full [`ProofResponse`] (§13.4): the inner proof must verify,
    /// the response's `roothash` must be in `trusted_roots`, AND that root
    /// must equal the root the proof is cryptographically bound to (recovered
    /// by recomputing the output commitment via `expected_output_bytes`).
    fn verify_response(
        &self,
        response: &ProofResponse,
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        expected_output_bytes: &[u8],
        chain: &dyn ChainSource,
    ) -> Result<()> {
        if !trusted_roots.contains(&response.roothash) {
            return Err(ProverError::UntrustedRoot(response.roothash.to_hex()));
        }
        // Recompute the bound commitment from the asserted root + returned bytes;
        // if the proof's committed output disagrees, the root binding is forged.
        let bound = bound_public_output(&response.roothash, expected_output_bytes);
        if bound != response.proof.public_output {
            return Err(ProverError::RootBindingMismatch {
                bound: bound.to_hex(),
                asserted: response.roothash.to_hex(),
            });
        }
        self.verify(&response.proof, expected_program_hash, trusted_roots, chain)
    }

    /// Verify a proof AND confirm it is bound to `expected_nonce` (§13.5).
    /// A proof for any other nonce is rejected, defeating replay.
    fn verify_with_nonce(
        &self,
        proof: &ExecutionProof,
        expected_nonce: &[u8; 32],
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        chain: &dyn ChainSource,
    ) -> Result<()> {
        let (nonce, _block) = crate::commitment::parse_public_input(&proof.public_input)?;
        if &nonce != expected_nonce {
            return Err(ProverError::NonceMismatch);
        }
        self.verify(proof, expected_program_hash, trusted_roots, chain)
    }
}

/// Recompute the roothash-bound output commitment from the asserted root and
/// the returned bytes: `SHA-256( roothash || returned_bytes )`. Mirrors
/// [`ServingInputs::compute_public_output`].
pub fn bound_public_output(roothash: &Bytes32, output_bytes: &[u8]) -> Bytes32 {
    let mut preimage = Vec::with_capacity(32 + output_bytes.len());
    preimage.extend_from_slice(&roothash.0);
    preimage.extend_from_slice(output_bytes);
    digstore_crypto::sha256(&preimage)
}
