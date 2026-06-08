use digstore_core::Bytes32;

pub struct Chunk {
    pub hash: Bytes32,
    pub data: Vec<u8>,
    pub offset: usize,
}

pub fn hash_data(_data: &[u8]) -> Bytes32 {
    Bytes32([0u8; 32])
}
