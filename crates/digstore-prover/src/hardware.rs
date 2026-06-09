use crate::chain::ChainSource;
use crate::commitment::{parse_public_input, signing_message};
use crate::error::{ProverError, Result};
use crate::mock::DEFAULT_FRESHNESS_WINDOW_SECS;
use crate::prover::{Prover, Verifier};
use crate::serving_inputs::ServingInputs;
use digstore_core::{Bytes32, Bytes48, Bytes96, ChiaBlockRef, ExecutionProof};
use digstore_crypto::{bls, sha256};

const TEE_DOMAIN: &[u8] = b"digstore-tee-attest-v1";

/// Digest the enclave signs to vouch for the serving statement (§13.6).
fn attest_digest(program_hash: &Bytes32, public_input: &[u8], public_output: &Bytes32) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(TEE_DOMAIN);
    buf.extend_from_slice(&program_hash.0);
    buf.extend_from_slice(public_input);
    buf.extend_from_slice(&public_output.0);
    sha256(&buf).0.to_vec()
}

/// §13.6 alternative: a TEE/HSM-attested run replaces the ZK proof. The
/// attestation (enclave BLS signature, 96 bytes) is carried in `proof`.
pub struct HardwareAttestProver {
    node_secret: bls::SecretKey,
    node_pubkey: bls::PublicKey,
    enclave_secret: bls::SecretKey,
    chia_block: ChiaBlockRef,
}

impl HardwareAttestProver {
    pub fn new(
        node_secret: bls::SecretKey,
        node_pubkey: bls::PublicKey,
        enclave_secret: bls::SecretKey,
        chia_block: ChiaBlockRef,
    ) -> Self {
        Self {
            node_secret,
            node_pubkey,
            enclave_secret,
            chia_block,
        }
    }
}

impl Prover for HardwareAttestProver {
    fn prove(
        &self,
        program_hash: Bytes32,
        public_input: &[u8],
        serving_inputs: &ServingInputs,
    ) -> Result<ExecutionProof> {
        let (_nonce, block) = parse_public_input(public_input)?;
        if block != self.chia_block {
            return Err(ProverError::Backend("public_input block mismatch".into()));
        }
        let public_output = serving_inputs.compute_public_output();
        let digest = attest_digest(&program_hash, public_input, &public_output);
        let attestation: Bytes96 = bls::bls_sign(&self.enclave_secret, &digest);
        let proof = attestation.0.to_vec(); // 96 bytes
        let msg = signing_message(&proof, public_input);
        let node_sig = bls::bls_sign(&self.node_secret, &msg);
        Ok(ExecutionProof {
            program_hash,
            public_input: public_input.to_vec(),
            public_output,
            proof,
            chia_block: self.chia_block.clone(),
            node_pubkey: self.node_pubkey.to_bytes(),
            node_signature: node_sig,
        })
    }
}

/// Verifier for [`HardwareAttestProver`] proofs; configured with the trusted
/// enclave BLS public key.
pub struct HardwareVerifier {
    trusted_enclave_pubkey: Bytes48,
}

impl HardwareVerifier {
    pub fn new(trusted_enclave_pubkey: Bytes48) -> Self {
        Self {
            trusted_enclave_pubkey,
        }
    }
}

impl Verifier for HardwareVerifier {
    fn verify(
        &self,
        proof: &ExecutionProof,
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        chain: &dyn ChainSource,
    ) -> Result<()> {
        if proof.program_hash != expected_program_hash {
            return Err(ProverError::ProgramHashMismatch {
                expected: expected_program_hash.to_hex(),
                actual: proof.program_hash.to_hex(),
            });
        }
        let (_nonce, pi_block) = parse_public_input(&proof.public_input)?;
        if pi_block != proof.chia_block {
            return Err(ProverError::Codec(
                "public_input block != proof.chia_block".into(),
            ));
        }
        if proof.proof.len() != 96 {
            return Err(ProverError::AttestationInvalid(
                "attestation not 96 bytes".into(),
            ));
        }
        let mut sig = [0u8; 96];
        sig.copy_from_slice(&proof.proof);
        let sig = Bytes96(sig);
        let digest = attest_digest(
            &proof.program_hash,
            &proof.public_input,
            &proof.public_output,
        );
        if !bls::bls_verify(&self.trusted_enclave_pubkey, &digest, &sig) {
            return Err(ProverError::AttestationInvalid(
                "enclave signature invalid".into(),
            ));
        }
        let msg = signing_message(&proof.proof, &proof.public_input);
        if !bls::bls_verify(&proof.node_pubkey, &msg, &proof.node_signature) {
            return Err(ProverError::NodeSignatureInvalid);
        }
        if trusted_roots.is_empty() {
            return Err(ProverError::UntrustedRoot(
                "no trusted roots provided".into(),
            ));
        }
        chain.verify_block(&proof.chia_block, DEFAULT_FRESHNESS_WINDOW_SECS)?;
        Ok(())
    }
}
