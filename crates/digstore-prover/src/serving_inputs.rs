use digstore_core::serving::concat_output;
use digstore_core::Bytes32;
use digstore_crypto::sha256;

/// Inputs to the deterministic serving computation that a proof attests
/// (deviation #3). The serving node resolves a retrieval key, looks it up in
/// the key table, gathers and concatenates the resource's chunk ciphertext,
/// and commits the result bound to the generation root. The risc0 guest
/// re-runs exactly this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServingInputs {
    /// Resolved retrieval key for the request.
    pub retrieval_key: Bytes32,
    /// Generation root the response is bound to (§13.4).
    pub roothash: Bytes32,
    /// The gathered chunk ciphertext, in order, for the resolved resource.
    pub chunk_ciphertext: Vec<Vec<u8>>,
}

impl ServingInputs {
    /// The concatenated, in-order chunk ciphertext (the returned bytes). Uses
    /// the single canonical ordering helper [`digstore_core::serving::concat_output`]
    /// (CONVENTIONS C9), so these bytes equal what the guest's `get_content`
    /// concatenates and what program re-execution commits.
    pub fn output_bytes(&self) -> Vec<u8> {
        let refs: Vec<&[u8]> = self.chunk_ciphertext.iter().map(|c| c.as_slice()).collect();
        concat_output(&refs)
    }

    /// The serving computation's `public_output` commitment:
    /// `SHA-256( roothash || concat(chunk_ciphertext) )`. Binding the root
    /// into the commitment means a genuine proof cannot be re-paired with a
    /// different generation root (§13.4).
    pub fn compute_public_output(&self) -> Bytes32 {
        let output = self.output_bytes();
        let mut preimage = Vec::with_capacity(32 + output.len());
        preimage.extend_from_slice(&self.roothash.0);
        preimage.extend_from_slice(&output);
        sha256(&preimage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(root: [u8; 32], chunks: Vec<Vec<u8>>) -> ServingInputs {
        ServingInputs {
            retrieval_key: Bytes32([7u8; 32]),
            roothash: Bytes32(root),
            chunk_ciphertext: chunks,
        }
    }

    #[test]
    fn public_output_binds_roothash_then_ciphertext() {
        let inp = inputs([9u8; 32], vec![vec![1, 2, 3], vec![4, 5]]);
        // commitment = SHA-256( roothash || concat(ciphertext) )
        let mut preimage = vec![9u8; 32];
        preimage.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(inp.compute_public_output(), sha256(&preimage));
    }

    #[test]
    fn output_bytes_is_concatenated_ciphertext() {
        let inp = inputs([0u8; 32], vec![vec![0xDE, 0xAD], vec![0xBE, 0xEF]]);
        assert_eq!(inp.output_bytes(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn different_ciphertext_gives_different_output() {
        let a = inputs([9u8; 32], vec![vec![1, 2, 3]]);
        let b = inputs([9u8; 32], vec![vec![1, 2, 4]]);
        assert_ne!(a.compute_public_output(), b.compute_public_output());
    }

    #[test]
    fn different_roothash_gives_different_output() {
        let a = inputs([9u8; 32], vec![vec![1, 2, 3]]);
        let b = inputs([8u8; 32], vec![vec![1, 2, 3]]);
        assert_ne!(a.compute_public_output(), b.compute_public_output());
    }

    #[test]
    fn output_bytes_matches_core_concat_output_ordering() {
        // C9: ServingInputs::output_bytes() MUST equal digstore_core::serving::concat_output
        // ordering, so the guest's get_content concat and the prover's serving output agree.
        let inp = inputs(
            [0u8; 32],
            vec![vec![0x01, 0x02], vec![0x03], vec![0x04, 0x05, 0x06]],
        );
        let refs: Vec<&[u8]> = inp.chunk_ciphertext.iter().map(|c| c.as_slice()).collect();
        let guest_style = digstore_core::serving::concat_output(&refs);
        assert_eq!(inp.output_bytes(), guest_style);
    }
}
