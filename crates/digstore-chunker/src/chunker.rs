use crate::boundary::find_boundary;
use crate::chunk::Chunk;
use digstore_core::ChunkerConfig;

/// A reusable content-defined chunker bound to a `ChunkerConfig`.
pub struct Chunker {
    config: ChunkerConfig,
}

impl Chunker {
    /// Create a chunker with the given configuration.
    pub fn new(config: ChunkerConfig) -> Self {
        Chunker { config }
    }

    /// The configuration this chunker uses.
    pub fn config(&self) -> &ChunkerConfig {
        &self.config
    }

    /// Chunk a full byte slice, returning content-addressed chunks in order.
    pub fn chunk_slice(&self, data: &[u8]) -> Vec<Chunk> {
        chunk_slice(data, &self.config)
    }
}

/// Chunk a full byte slice into content-defined chunks.
///
/// Empty input yields zero chunks. Input shorter than `min_size` yields a single
/// whole chunk. Concatenating the chunks in order reproduces the input exactly,
/// and every chunk except possibly the last satisfies
/// `min_size <= len <= max_size` (paper §8.1).
pub fn chunk_slice(data: &[u8], cfg: &ChunkerConfig) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < data.len() {
        let end = find_boundary(data, start, cfg);
        debug_assert!(end > start, "boundary must advance");
        chunks.push(Chunk::new(start, data[start..end].to_vec()));
        start = end;
    }
    chunks
}

/// Stream chunking over any `std::io::Read`, emitting chunks incrementally
/// WITHOUT buffering the entire reader.
///
/// Invariant maintained each iteration before calling `find_boundary`: either
/// at least `max_size` bytes are buffered, or the reader has hit EOF. Under that
/// invariant `find_boundary(&buf, 0, cfg)` returns the true boundary for the
/// chunk starting at `buf[0]` — a forced max cut (it saw the full max window) or,
/// only at EOF, a trailing short chunk. Output is byte-identical to `chunk_slice`
/// over the concatenated reader contents.
pub fn chunk_stream<R: std::io::Read>(
    mut reader: R,
    cfg: &ChunkerConfig,
) -> std::io::Result<Vec<Chunk>> {
    const READ_BLOCK: usize = 64 * 1024;
    let mut chunks = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut consumed = 0usize; // absolute offset of buf[0] in the original stream
    let mut eof = false;

    loop {
        // Refill until we have the full max window buffered, or hit EOF.
        while !eof && buf.len() < cfg.max_size {
            let old = buf.len();
            buf.resize(old + READ_BLOCK, 0);
            let n = reader.read(&mut buf[old..])?;
            buf.truncate(old + n);
            if n == 0 {
                eof = true;
            }
        }

        if buf.is_empty() {
            break;
        }

        // Boundary relative to the current buffer (chunk start = 0).
        let end = find_boundary(&buf, 0, cfg);
        debug_assert!(end > 0 && end <= buf.len());

        let chunk_data = buf[..end].to_vec();
        chunks.push(Chunk::new(consumed, chunk_data));
        consumed += end;
        buf.drain(..end);
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::ChunkerConfig;

    fn small_cfg() -> ChunkerConfig {
        // Small bounds so tests run on modest inputs.
        ChunkerConfig {
            min_size: 64,
            target_size: 256,
            max_size: 1024,
            mask: 0xFF,
        }
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        let chunks = chunk_slice(&[], &small_cfg());
        assert!(chunks.is_empty());
    }

    #[test]
    fn tiny_input_yields_single_whole_chunk() {
        let data = vec![1u8, 2, 3, 4, 5];
        let chunks = chunk_slice(&data, &small_cfg());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].data, data);
    }

    #[test]
    fn chunks_reconstruct_original_input() {
        let data: Vec<u8> = (0..5000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        let chunks = chunk_slice(&data, &small_cfg());
        let mut rebuilt = Vec::new();
        for c in &chunks {
            rebuilt.extend_from_slice(&c.data);
        }
        assert_eq!(rebuilt, data);
    }

    #[test]
    fn chunk_offsets_are_contiguous_from_zero() {
        let data: Vec<u8> = (0..5000u32)
            .map(|i| (i.wrapping_mul(40503) >> 7) as u8)
            .collect();
        let chunks = chunk_slice(&data, &small_cfg());
        let mut expected_offset = 0usize;
        for c in &chunks {
            assert_eq!(c.offset, expected_offset);
            expected_offset += c.data.len();
        }
        assert_eq!(expected_offset, data.len());
    }

    #[test]
    fn all_but_last_chunk_obey_size_bounds() {
        let cfg = small_cfg();
        let data: Vec<u8> = (0..20_000u32)
            .map(|i| (i.wrapping_mul(2246822519) >> 11) as u8)
            .collect();
        let chunks = chunk_slice(&data, &cfg);
        assert!(chunks.len() > 1, "expected multiple chunks");
        for c in &chunks[..chunks.len() - 1] {
            assert!(
                c.len() >= cfg.min_size,
                "chunk len {} < min {}",
                c.len(),
                cfg.min_size
            );
            assert!(
                c.len() <= cfg.max_size,
                "chunk len {} > max {}",
                c.len(),
                cfg.max_size
            );
        }
        // Last chunk only needs <= max.
        assert!(chunks.last().unwrap().len() <= cfg.max_size);
    }

    #[test]
    fn chunk_hashes_match_their_data() {
        let data: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
        let chunks = chunk_slice(&data, &small_cfg());
        for c in &chunks {
            assert_eq!(c.hash, crate::chunk::hash_data(&c.data));
        }
    }

    #[test]
    fn chunker_struct_uses_its_config() {
        let chunker = Chunker::new(small_cfg());
        assert_eq!(chunker.config().target_size, 256);
        let data: Vec<u8> = (0..4000u32).map(|i| i as u8).collect();
        assert_eq!(chunker.chunk_slice(&data), chunk_slice(&data, &small_cfg()));
    }

    // --- streaming tests (appended to the same `mod tests`) ---
    use std::io::Read;

    /// A reader that yields `step` bytes per call from `data`, and counts how
    /// many bytes have been handed out.
    struct CountingReader<'a> {
        data: &'a [u8],
        pos: usize,
        step: usize,
    }
    impl<'a> Read for CountingReader<'a> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let want = self.step.min(out.len()).min(self.data.len() - self.pos);
            out[..want].copy_from_slice(&self.data[self.pos..self.pos + want]);
            self.pos += want;
            Ok(want)
        }
    }

    #[test]
    fn stream_equals_slice_for_various_read_sizes() {
        let cfg = small_cfg();
        let data: Vec<u8> = (0..30_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 9) as u8)
            .collect();
        let want = chunk_slice(&data, &cfg);
        for step in [1usize, 7, 64, 250, 1024, 4096, 100_000] {
            let reader = CountingReader {
                data: &data,
                pos: 0,
                step,
            };
            let got = chunk_stream(reader, &cfg).unwrap();
            assert_eq!(got, want, "stream != slice for read step {step}");
        }
    }

    #[test]
    fn stream_empty_reader_yields_no_chunks() {
        let reader = CountingReader {
            data: &[],
            pos: 0,
            step: 16,
        };
        let got = chunk_stream(reader, &small_cfg()).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn stream_tiny_reader_yields_single_chunk() {
        let data = vec![9u8, 8, 7];
        let reader = CountingReader {
            data: &data,
            pos: 0,
            step: 1,
        };
        let got = chunk_stream(reader, &small_cfg()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].data, data);
        assert_eq!(got[0].offset, 0);
    }
}
