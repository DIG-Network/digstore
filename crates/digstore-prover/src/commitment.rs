use crate::error::{ProverError, Result};
use digstore_core::codec::{Decode, Decoder, Encode, Encoder};
use digstore_core::ChiaBlockRef;

/// Length of the client nonce that prefixes `public_input` (§13.5).
pub const NONCE_LEN: usize = 32;

/// Fixed encoded length of a `ChiaBlockRef` (header_hash 32 + height 4 + ts 8).
const CHIA_BLOCK_REF_LEN: usize = 44;

/// `public_input = client_nonce(32) || ChiaBlockRef(codec)`.
pub fn build_public_input(nonce: &[u8; 32], block: &ChiaBlockRef) -> Vec<u8> {
    let mut enc = Encoder::new();
    enc.write_bytes(nonce);
    block.encode(&mut enc);
    enc.finish()
}

/// Inverse of [`build_public_input`]. Rejects under- AND over-length input.
pub fn parse_public_input(bytes: &[u8]) -> Result<([u8; 32], ChiaBlockRef)> {
    if bytes.len() < NONCE_LEN {
        return Err(ProverError::Codec("public_input too short for nonce".into()));
    }
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&bytes[..NONCE_LEN]);
    let mut dec = Decoder::new(&bytes[NONCE_LEN..]);
    let block = ChiaBlockRef::decode(&mut dec)
        .map_err(|e| ProverError::Codec(format!("ChiaBlockRef decode: {e:?}")))?;
    if dec.remaining() != 0 {
        return Err(ProverError::Codec(format!(
            "public_input has {} trailing bytes after ChiaBlockRef",
            dec.remaining()
        )));
    }
    Ok((nonce, block))
}

/// The message a node signs for attribution: `proof || public_input` (§13.7).
pub fn signing_message(proof: &[u8], public_input: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(proof.len() + public_input.len());
    msg.extend_from_slice(proof);
    msg.extend_from_slice(public_input);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;

    fn sample_block() -> ChiaBlockRef {
        ChiaBlockRef { header_hash: Bytes32([0xABu8; 32]), height: 5_000_000, timestamp: 1_900_000_000 }
    }

    #[test]
    fn public_input_round_trips() {
        let nonce = [0x11u8; 32];
        let block = sample_block();
        let pi = build_public_input(&nonce, &block);
        assert_eq!(pi.len(), 76); // 32 nonce + 32 header_hash + 4 height + 8 timestamp
        let (got_nonce, got_block) = parse_public_input(&pi).unwrap();
        assert_eq!(got_nonce, nonce);
        assert_eq!(got_block, block);
    }

    #[test]
    fn public_input_block_ref_is_44_bytes() {
        let pi = build_public_input(&[0u8; 32], &sample_block());
        assert_eq!(pi.len() - NONCE_LEN, CHIA_BLOCK_REF_LEN);
    }

    #[test]
    fn parse_rejects_short_input() {
        let short = vec![0u8; 10];
        assert!(matches!(parse_public_input(&short), Err(ProverError::Codec(_))));
    }

    #[test]
    fn parse_rejects_trailing_bytes() {
        let nonce = [0x11u8; 32];
        let mut pi = build_public_input(&nonce, &sample_block());
        pi.push(0xFF); // 77 bytes — one trailing byte
        assert!(matches!(parse_public_input(&pi), Err(ProverError::Codec(_))));
    }

    #[test]
    fn signing_message_is_proof_then_public_input() {
        let pi = vec![9u8; 76];
        let proof = vec![1u8, 2, 3];
        let msg = signing_message(&proof, &pi);
        assert_eq!(msg.len(), proof.len() + pi.len());
        assert_eq!(&msg[..3], &proof[..]);
        assert_eq!(&msg[3..], &pi[..]);
    }
}
