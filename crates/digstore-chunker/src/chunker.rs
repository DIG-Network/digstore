use crate::chunk::Chunk;
use digstore_core::ChunkerConfig;

pub struct Chunker {
    config: ChunkerConfig,
}

pub fn chunk_slice(_data: &[u8], _cfg: &ChunkerConfig) -> Vec<Chunk> {
    Vec::new()
}

pub fn chunk_stream<R: std::io::Read>(_reader: R, _cfg: &ChunkerConfig) -> std::io::Result<Vec<Chunk>> {
    Ok(Vec::new())
}
