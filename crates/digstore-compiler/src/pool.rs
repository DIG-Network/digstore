use digstore_core::{Bytes32, Decode, DecodeError, Decoder, Encode, Encoder};

use crate::filler::deterministic_filler;

/// Location of one chunk inside the interleaved pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkLoc {
    pub offset: u32,
    pub len: u32,
}

/// The assembled interleaved pool: a flat byte buffer with chunk bodies packed in
/// global-index order, the trailing gap filled with deterministic filler, and a
/// parallel list of `(offset,len)` descriptors. No resource boundary is encoded.
#[derive(Debug)]
pub struct InterleavedPool {
    pub bytes: Vec<u8>,
    pub descriptors: Vec<ChunkLoc>,
}

/// Smallest bucket >= `n`, stepping in powers of two from a 64-byte floor. Hides
/// the exact content byte count so module size leaks only a coarse bucket (§8.3).
pub fn next_pool_bucket(n: usize) -> usize {
    let mut b = 64usize;
    while b < n {
        b <<= 1;
    }
    b
}

/// Build the interleaved pool from chunk bodies in global-index order.
pub fn build_pool(store_id: &Bytes32, roothash: &Bytes32, bodies: &[Vec<u8>]) -> InterleavedPool {
    let content_len: usize = bodies.iter().map(|b| b.len()).sum();
    let total = next_pool_bucket(content_len);

    // Start from the full-length deterministic filler, then overwrite the content
    // prefix. Because the keystream is positional, the filler tail is identical to
    // what a fresh `deterministic_filler` of the same length produces at that range.
    let mut bytes = deterministic_filler(store_id, roothash, total);

    let mut descriptors = Vec::with_capacity(bodies.len());
    let mut cursor = 0u32;
    for body in bodies {
        let len = body.len() as u32;
        let start = cursor as usize;
        bytes[start..start + body.len()].copy_from_slice(body);
        descriptors.push(ChunkLoc { offset: cursor, len });
        cursor += len;
    }

    InterleavedPool { bytes, descriptors }
}

// `ChunkLoc` is a compiler-local type, so the compiler implements the canonical
// core codec for it here (big-endian, deviation #1). Two BE u32s.
impl Encode for ChunkLoc {
    fn encode(&self, enc: &mut Encoder) {
        self.offset.encode(enc);
        self.len.encode(enc);
    }
}

impl Decode for ChunkLoc {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let offset = u32::decode(dec)?;
        let len = u32::decode(dec)?;
        Ok(ChunkLoc { offset, len })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_function_rounds_up_to_next_power_of_two_from_floor() {
        assert_eq!(next_pool_bucket(0), 64);
        assert_eq!(next_pool_bucket(35), 64);
        assert_eq!(next_pool_bucket(64), 64);
        assert_eq!(next_pool_bucket(65), 128);
        assert_eq!(next_pool_bucket(4096), 4096);
        assert_eq!(next_pool_bucket(4097), 8192);
    }

    fn bodies() -> Vec<Vec<u8>> {
        vec![vec![1u8; 10], vec![2u8; 20], vec![3u8; 5]]
    }

    #[test]
    fn pool_contains_each_chunk_body_in_index_order() {
        let sid = Bytes32([1; 32]);
        let root = Bytes32([2; 32]);
        let pool = build_pool(&sid, &root, &bodies());
        assert_eq!(pool.descriptors[0], ChunkLoc { offset: 0, len: 10 });
        assert_eq!(pool.descriptors[1], ChunkLoc { offset: 10, len: 20 });
        assert_eq!(pool.descriptors[2], ChunkLoc { offset: 30, len: 5 });
        assert_eq!(&pool.bytes[0..10], &[1u8; 10]);
        assert_eq!(&pool.bytes[10..30], &[2u8; 20]);
        assert_eq!(&pool.bytes[30..35], &[3u8; 5]);
    }

    #[test]
    fn pool_length_is_bucketed_above_content_and_filled_with_filler() {
        let sid = Bytes32([1; 32]);
        let root = Bytes32([2; 32]);
        let pool = build_pool(&sid, &root, &bodies()); // 35 content bytes
        assert_eq!(pool.bytes.len(), 64);
        let filler = crate::filler::deterministic_filler(&sid, &root, 64);
        assert_eq!(&pool.bytes[35..64], &filler[35..64]);
    }

    #[test]
    fn empty_chunk_set_still_yields_filled_bucket() {
        let pool = build_pool(&Bytes32([0; 32]), &Bytes32([0; 32]), &[]);
        assert_eq!(pool.bytes.len(), 64);
        assert!(pool.descriptors.is_empty());
    }
}

#[cfg(test)]
mod codec_tests {
    use super::*;

    #[test]
    fn chunkloc_round_trips_via_core_codec() {
        let loc = ChunkLoc { offset: 7, len: 41 };
        let buf = loc.to_bytes();
        assert_eq!(buf, vec![0, 0, 0, 7, 0, 0, 0, 41]); // two BE u32s
        let mut dec = Decoder::new(&buf);
        let back = ChunkLoc::decode(&mut dec).unwrap();
        assert_eq!(back, loc);
        assert_eq!(dec.remaining(), 0);
    }
}
