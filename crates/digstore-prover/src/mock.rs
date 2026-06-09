use crate::chain::ChainSource;
use crate::commitment::{parse_public_input, signing_message};
use crate::error::{ProverError, Result};
use crate::prover::{Prover, Verifier};
use crate::serving_inputs::ServingInputs;
use digstore_core::{Bytes32, ChiaBlockRef, ExecutionProof};
use digstore_crypto::{bls, sha256};

const MOCK_DOMAIN: &[u8] = b"digstore-mock-proof-v1";

/// Default freshness window for chain anchoring (10 minutes).
pub const DEFAULT_FRESHNESS_WINDOW_SECS: u64 = 600;

/// The mock commitment-chain proof bytes (deviation #3): a SHA-256 over the
/// full statement. Recomputed identically by the verifier.
fn mock_proof_bytes(
    program_hash: &Bytes32,
    public_input: &[u8],
    public_output: &Bytes32,
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MOCK_DOMAIN);
    buf.extend_from_slice(&program_hash.0);
    buf.extend_from_slice(public_input);
    buf.extend_from_slice(&public_output.0);
    sha256(&buf).0.to_vec()
}

/// Default deterministic prover. Emits a commitment-chain proof and a genuine
/// node BLS signature over `(proof || public_input)`.
pub struct MockProver {
    secret: bls::SecretKey,
    pubkey: bls::PublicKey,
    chia_block: ChiaBlockRef,
}

impl MockProver {
    pub fn new(secret: bls::SecretKey, pubkey: bls::PublicKey, chia_block: ChiaBlockRef) -> Self {
        Self {
            secret,
            pubkey,
            chia_block,
        }
    }
}

impl Prover for MockProver {
    fn prove(
        &self,
        program_hash: Bytes32,
        public_input: &[u8],
        serving_inputs: &ServingInputs,
    ) -> Result<ExecutionProof> {
        let (_nonce, block) = parse_public_input(public_input)?;
        if block != self.chia_block {
            return Err(ProverError::Backend(
                "public_input block does not match prover's bound chia_block".into(),
            ));
        }
        let public_output = serving_inputs.compute_public_output();
        let proof = mock_proof_bytes(&program_hash, public_input, &public_output);
        let msg = signing_message(&proof, public_input);
        let node_signature = bls::bls_sign(&self.secret, &msg);
        Ok(ExecutionProof {
            program_hash,
            public_input: public_input.to_vec(),
            public_output,
            proof,
            chia_block: self.chia_block.clone(),
            node_pubkey: self.pubkey.to_bytes(),
            node_signature,
        })
    }
}

/// Verifier for [`MockProver`] proofs.
#[derive(Debug, Default)]
pub struct MockVerifier;

impl Verifier for MockVerifier {
    fn verify(
        &self,
        proof: &ExecutionProof,
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        chain: &dyn ChainSource,
    ) -> Result<()> {
        // 1. program-hash match (§13.1, §13.4)
        if proof.program_hash != expected_program_hash {
            return Err(ProverError::ProgramHashMismatch {
                expected: expected_program_hash.to_hex(),
                actual: proof.program_hash.to_hex(),
            });
        }
        // 2. public_input parse; bound block must equal proof.chia_block (§13.8)
        let (_nonce, pi_block) = parse_public_input(&proof.public_input)?;
        if pi_block != proof.chia_block {
            return Err(ProverError::Codec(
                "public_input block != proof.chia_block".into(),
            ));
        }
        // 3. recompute the mock commitment chain (deviation #3). Tampering
        //    public_output OR proof bytes surfaces here.
        let expected_proof = mock_proof_bytes(
            &proof.program_hash,
            &proof.public_input,
            &proof.public_output,
        );
        if expected_proof != proof.proof {
            return Err(ProverError::ZkProofInvalid(
                "mock commitment chain mismatch".into(),
            ));
        }
        // 4. node attribution: BLS over (proof || public_input) (§13.7)
        let msg = signing_message(&proof.proof, &proof.public_input);
        if !bls::bls_verify(&proof.node_pubkey, &msg, &proof.node_signature) {
            return Err(ProverError::NodeSignatureInvalid);
        }
        // 5. require a non-empty trusted-root set; root *binding* is enforced
        //    in Verifier::verify_response against the asserted ProofResponse root.
        if trusted_roots.is_empty() {
            return Err(ProverError::UntrustedRoot(
                "no trusted roots provided".into(),
            ));
        }
        // 6. chain freshness (§13.8)
        chain.verify_block(&proof.chia_block, DEFAULT_FRESHNESS_WINDOW_SECS)?;
        Ok(())
    }
}
