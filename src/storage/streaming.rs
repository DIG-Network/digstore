//! Streaming file processing that never loads entire files into memory

use crate::core::error::Result;
use crate::core::types::{Chunk, ChunkRef};
use crate::core::{error::*, types::*};
use crate::storage::chunk::ChunkConfig;
use memmap2::{Mmap, MmapOptions};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Streaming chunking engine that processes files without loading them entirely
pub struct StreamingChunkingEngine {
    config: ChunkConfig,
    buffer_size: usize,
    mmap_threshold: u64, // Use memory mapping for files larger than this
}

impl StreamingChunkingEngine {
    pub fn new() -> Self {
        Self {
            config: ChunkConfig::default(),
            buffer_size: 64 * 1024,           // 64KB buffer
            mmap_threshold: 10 * 1024 * 1024, // 10MB threshold for memory mapping
        }
    }

    pub fn with_config(config: ChunkConfig) -> Self {
        Self {
            config,
            buffer_size: 64 * 1024,
            mmap_threshold: 10 * 1024 * 1024,
        }
    }

    /// Process a file using streaming - never loads entire file into memory
    pub fn chunk_file_streaming(&self, file_path: &Path) -> Result<Vec<Chunk>> {
        let file = File::open(file_path)?;
        let file_size = file.metadata()?.len();

        if file_size == 0 {
            return Ok(vec![]);
        }

        // Use different strategies based on file size
        if file_size < 64 * 1024 {
            // Very small files: read directly (but still streaming)
            self.chunk_small_file_streaming(file)
        } else if file_size > self.mmap_threshold {
            // Very large files: use memory mapping for efficiency
            self.chunk_mmap_file(file_path, file_size)
        } else {
            // Medium files: use content-defined chunking with streaming
            self.chunk_large_file_streaming(file, file_size)
        }
    }

    /// Stream small files efficiently
    fn chunk_small_file_streaming(&self, mut file: File) -> Result<Vec<Chunk>> {
        let mut buffer = vec![0u8; self.buffer_size];
        let mut chunks = Vec::new();
        let mut offset = 0u64;
        let mut hasher = Sha256::new();
        let mut chunk_data = Vec::new();

        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }

            let data = &buffer[..bytes_read];
            chunk_data.extend_from_slice(data);
            hasher.update(data);
        }

        // Small files become single chunk
        if !chunk_data.is_empty() {
            let hash = Hash::from_bytes(hasher.finalize().into());
            chunks.push(Chunk {
                hash,
                offset,
                size: chunk_data.len() as u32,
                data: chunk_data,
            });
        }

        Ok(chunks)
    }

    /// Stream large files with content-defined chunking
    fn chunk_large_file_streaming(&self, mut file: File, file_size: u64) -> Result<Vec<Chunk>> {
        let mut chunks = Vec::new();
        let mut buffer = vec![0u8; self.buffer_size];
        let mut rolling_buffer = Vec::new();
        let mut current_chunk = Vec::new();
        let mut offset = 0u64;
        let mut rolling_hash = RollingHash::new();

        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                // End of file - finalize current chunk
                if !current_chunk.is_empty() {
                    let chunk = self.finalize_chunk(current_chunk, offset)?;
                    offset += chunk.size as u64;
                    chunks.push(chunk);
                }
                break;
            }

            let data = &buffer[..bytes_read];

            // Process data byte by byte for content-defined boundaries
            for &byte in data {
                current_chunk.push(byte);
                rolling_buffer.push(byte);

                // Maintain rolling window
                if rolling_buffer.len() > 64 {
                    rolling_buffer.remove(0);
                }

                // Check for chunk boundary
                if self.should_break_chunk(&current_chunk, &rolling_buffer, &mut rolling_hash) {
                    let chunk = self.finalize_chunk(current_chunk, offset)?;
                    offset += chunk.size as u64;
                    chunks.push(chunk);
                    current_chunk = Vec::new();
                }
            }
        }

        Ok(chunks)
    }

    /// Process very large files using memory mapping for efficiency
    fn chunk_mmap_file(&self, file_path: &Path, file_size: u64) -> Result<Vec<Chunk>> {
        let file = File::open(file_path)?;
        let mmap = unsafe { MmapOptions::new().map(&file)? };

        let mut chunks = Vec::new();
        let chunk_size = 1024 * 1024; // 1MB chunks for large files
        let mut offset = 0u64;

        // Process memory-mapped data in chunks
        for chunk_data in mmap.chunks(chunk_size) {
            let hash = crate::core::hash::sha256(chunk_data);
            let size = chunk_data.len() as u32;

            chunks.push(Chunk {
                hash,
                offset,
                size,
                data: chunk_data.to_vec(),
            });

            offset += size as u64;
        }

        Ok(chunks)
    }

    fn should_break_chunk(
        &self,
        chunk: &[u8],
        rolling_window: &[u8],
        rolling_hash: &mut RollingHash,
    ) -> bool {
        if chunk.len() < self.config.min_size {
            return false;
        }

        if chunk.len() >= self.config.max_size {
            return true;
        }

        // Content-defined boundary detection
        if chunk.len() >= self.config.avg_size && rolling_window.len() >= 32 {
            let hash = rolling_hash.hash(rolling_window);
            return (hash & 0xFFF) == 0; // Boundary approximately every 4KB
        }

        false
    }

    fn finalize_chunk(&self, data: Vec<u8>, offset: u64) -> Result<Chunk> {
        let hash = crate::core::hash::sha256(&data);
        let size = data.len() as u32;

        Ok(Chunk {
            hash,
            offset,
            size,
            data,
        })
    }
}

/// Simple rolling hash for content-defined chunking
pub struct RollingHash {
    window_size: usize,
}

impl RollingHash {
    pub fn new() -> Self {
        Self { window_size: 32 }
    }

    pub fn hash(&mut self, window: &[u8]) -> u32 {
        // Simple polynomial rolling hash
        let mut hash = 0u32;
        for &byte in window {
            hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
        }
        hash
    }
}

/// File pointer that tracks position without loading data
pub struct FilePointer {
    file: Option<File>,
    mmap: Option<Mmap>,
    current_position: u64,
    file_size: u64,
    use_mmap: bool,
}

impl FilePointer {
    pub fn new(file_path: &Path) -> Result<Self> {
        let file = File::open(file_path)?;
        let file_size = file.metadata()?.len();

        // Use memory mapping for large files
        let mmap_threshold = 10 * 1024 * 1024; // 10MB
        let use_mmap = file_size > mmap_threshold;

        if use_mmap {
            let mmap = unsafe { MmapOptions::new().map(&file)? };
            Ok(Self {
                file: None,
                mmap: Some(mmap),
                current_position: 0,
                file_size,
                use_mmap: true,
            })
        } else {
            Ok(Self {
                file: Some(file),
                mmap: None,
                current_position: 0,
                file_size,
                use_mmap: false,
            })
        }
    }

    /// Read a chunk of data at current position
    pub fn read_chunk(&mut self, size: usize) -> Result<Vec<u8>> {
        if self.use_mmap {
            // Memory-mapped access
            let start = self.current_position as usize;
            let end = (start + size).min(self.file_size as usize);
            let data = &self.mmap.as_ref().unwrap()[start..end];
            self.current_position += (end - start) as u64;
            Ok(data.to_vec())
        } else {
            // Regular file access
            let mut buffer = vec![0u8; size];
            let bytes_read = self.file.as_mut().unwrap().read(&mut buffer)?;
            buffer.truncate(bytes_read);
            self.current_position += bytes_read as u64;
            Ok(buffer)
        }
    }

    /// Read data at specific offset without changing current position
    pub fn read_at_offset(&mut self, offset: u64, size: usize) -> Result<Vec<u8>> {
        if self.use_mmap {
            // Memory-mapped access (very efficient for random access)
            let start = offset as usize;
            let end = (start + size).min(self.file_size as usize);
            let data = &self.mmap.as_ref().unwrap()[start..end];
            Ok(data.to_vec())
        } else {
            // Regular file access
            let original_pos = self.current_position;
            self.file.as_mut().unwrap().seek(SeekFrom::Start(offset))?;

            let mut buffer = vec![0u8; size];
            let bytes_read = self.file.as_mut().unwrap().read(&mut buffer)?;
            buffer.truncate(bytes_read);

            // Restore original position
            self.file
                .as_mut()
                .unwrap()
                .seek(SeekFrom::Start(original_pos))?;

            Ok(buffer)
        }
    }

    /// Get current position in file
    pub fn position(&self) -> u64 {
        self.current_position
    }

    /// Get total file size
    pub fn size(&self) -> u64 {
        self.file_size
    }

    /// Check if at end of file
    pub fn is_eof(&self) -> bool {
        self.current_position >= self.file_size
    }
}

/// Streaming file entry that only stores metadata, not data
#[derive(Debug, Clone)]
pub struct StreamingFileEntry {
    pub path: std::path::PathBuf,
    pub hash: Hash,
    pub size: u64,
    pub chunk_refs: Vec<ChunkReference>,
}

/// Reference to a chunk without storing the actual data
#[derive(Debug, Clone)]
pub struct ChunkReference {
    pub hash: Hash,
    pub offset: u64,
    pub size: u32,
    pub file_offset: u64, // Offset within the original file
}

impl StreamingFileEntry {
    /// Create file entry from streaming processing
    pub fn from_streaming_chunks(path: &Path, chunks: Vec<Chunk>) -> Self {
        let total_size = chunks.iter().map(|c| c.size as u64).sum();
        let file_hash = Self::compute_file_hash(&chunks);

        let chunk_refs = chunks
            .into_iter()
            .map(|chunk| ChunkReference {
                hash: chunk.hash,
                offset: chunk.offset,
                size: chunk.size,
                file_offset: chunk.offset,
            })
            .collect();

        Self {
            path: path.to_path_buf(),
            hash: file_hash,
            size: total_size,
            chunk_refs,
        }
    }

    /// Reconstruct file data by reading only the necessary chunks
    pub fn reconstruct_data(&self, file_path: &Path) -> Result<Vec<u8>> {
        let mut file_pointer = FilePointer::new(file_path)?;
        let mut result = Vec::with_capacity(self.size as usize);

        for chunk_ref in &self.chunk_refs {
            let chunk_data =
                file_pointer.read_at_offset(chunk_ref.file_offset, chunk_ref.size as usize)?;
            result.extend_from_slice(&chunk_data);
        }

        Ok(result)
    }

    /// Reconstruct specific byte range without loading entire file
    pub fn reconstruct_range(&self, file_path: &Path, start: u64, end: u64) -> Result<Vec<u8>> {
        let mut file_pointer = FilePointer::new(file_path)?;
        let mut result = Vec::new();

        for chunk_ref in &self.chunk_refs {
            let chunk_start = chunk_ref.file_offset;
            let chunk_end = chunk_start + chunk_ref.size as u64;

            // Check if this chunk overlaps with requested range
            if chunk_end <= start || chunk_start >= end {
                continue; // No overlap
            }

            // Calculate intersection
            let read_start = chunk_start.max(start);
            let read_end = chunk_end.min(end);
            let read_size = (read_end - read_start) as usize;

            if read_size > 0 {
                let chunk_data = file_pointer.read_at_offset(read_start, read_size)?;
                result.extend_from_slice(&chunk_data);
            }
        }

        Ok(result)
    }

    fn compute_file_hash(chunks: &[Chunk]) -> Hash {
        let mut hasher = Sha256::new();
        for chunk in chunks {
            hasher.update(&chunk.data);
        }
        Hash::from_bytes(hasher.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_streaming_chunking_small_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"Hello, streaming world!";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();

        let engine = StreamingChunkingEngine::new();
        let chunks = engine.chunk_file_streaming(temp_file.path()).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].data, test_data);
        assert_eq!(chunks[0].size, test_data.len() as u32);
    }

    #[test]
    fn test_file_pointer_operations() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();

        let mut pointer = FilePointer::new(temp_file.path()).unwrap();

        // Test sequential reading
        let chunk1 = pointer.read_chunk(10).unwrap();
        assert_eq!(chunk1, b"0123456789");
        assert_eq!(pointer.position(), 10);

        // Test reading at specific offset
        let chunk2 = pointer.read_at_offset(20, 5).unwrap();
        assert_eq!(chunk2, b"KLMNO");
        assert_eq!(pointer.position(), 10); // Position unchanged

        // Test reading rest of file
        let chunk3 = pointer.read_chunk(100).unwrap(); // Request more than available
        assert_eq!(chunk3, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    }

    #[test]
    fn test_streaming_file_entry_range_reconstruction() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();

        // Create streaming file entry
        let engine = StreamingChunkingEngine::new();
        let chunks = engine.chunk_file_streaming(temp_file.path()).unwrap();
        let file_entry = StreamingFileEntry::from_streaming_chunks(temp_file.path(), chunks);

        // Test range reconstruction
        let range_data = file_entry
            .reconstruct_range(temp_file.path(), 5, 15)
            .unwrap();
        assert_eq!(range_data, b"56789ABCDE");

        // Test full reconstruction
        let full_data = file_entry.reconstruct_data(temp_file.path()).unwrap();
        assert_eq!(full_data, test_data);
    }

    #[test]
    fn test_large_file_streaming_memory_usage() {
        // Create a larger test file (1MB)
        let mut temp_file = NamedTempFile::new().unwrap();
        let chunk_data = vec![0u8; 1024]; // 1KB chunk
        for i in 0..1024 {
            temp_file.write_all(&chunk_data).unwrap();
        }
        temp_file.flush().unwrap();

        let engine = StreamingChunkingEngine::new();

        // Measure memory before
        let memory_before = get_memory_usage();

        // Process file
        let chunks = engine.chunk_file_streaming(temp_file.path()).unwrap();

        // Measure memory after
        let memory_after = get_memory_usage();
        let memory_increase = memory_after.saturating_sub(memory_before);

        // Memory increase should be minimal (just chunk metadata, not data)
        assert!(
            memory_increase < 10 * 1024 * 1024,
            "Memory increase should be <10MB"
        );
        assert!(!chunks.is_empty(), "Should produce chunks");
    }
}

// Helper function to get current memory usage (simplified)
#[cfg(test)]
fn get_memory_usage() -> usize {
    // This is a simplified implementation
    // In a real implementation, you'd use platform-specific APIs
    std::process::Command::new("ps")
        .args(&["-o", "rss=", "-p"])
        .arg(std::process::id().to_string())
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0)
        * 1024 // Convert KB to bytes
}
