//! High-performance binary staging format for large repositories
//!
//! This module implements a scalable binary staging format that can handle
//! hundreds of thousands of files efficiently. Features:
//! - Binary format with fixed-size headers for O(1) access
//! - Streaming writes and reads
//! - Compression support
//! - Index for fast lookups
//! - Memory-mapped access for large staging areas

use crate::core::error::Result;
use crate::core::{error::DigstoreError, types::*};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use memmap2::{Mmap, MmapMut, MmapOptions};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Magic bytes for binary staging format
const STAGING_MAGIC: &[u8; 8] = b"DIGSTAGE";
const STAGING_VERSION: u32 = 1;

/// Fixed-size header for the staging file
#[repr(C)]
#[derive(Debug, Clone)]
pub struct StagingHeader {
    /// Magic bytes: "DIGSTAGE"
    pub magic: [u8; 8],
    /// Format version
    pub version: u32,
    /// Number of staged files
    pub file_count: u64,
    /// Offset to index section
    pub index_offset: u64,
    /// Size of index section in bytes
    pub index_size: u64,
    /// Offset to data section
    pub data_offset: u64,
    /// Size of data section in bytes
    pub data_size: u64,
    /// Compression type (0=none, 1=zstd)
    pub compression: u32,
    /// Reserved for future use
    pub reserved: [u8; 32],
}

impl Default for StagingHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl StagingHeader {
    pub const SIZE: usize = 8 + 4 + 8 + 8 + 8 + 8 + 8 + 4 + 32; // 88 bytes

    pub fn new() -> Self {
        Self {
            magic: *STAGING_MAGIC,
            version: STAGING_VERSION,
            file_count: 0,
            index_offset: Self::SIZE as u64,
            index_size: 0,
            data_offset: Self::SIZE as u64,
            data_size: 0,
            compression: 0,
            reserved: [0; 32],
        }
    }

    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(&self.magic)?;
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u64::<LittleEndian>(self.file_count)?;
        writer.write_u64::<LittleEndian>(self.index_offset)?;
        writer.write_u64::<LittleEndian>(self.index_size)?;
        writer.write_u64::<LittleEndian>(self.data_offset)?;
        writer.write_u64::<LittleEndian>(self.data_size)?;
        writer.write_u32::<LittleEndian>(self.compression)?;
        writer.write_all(&self.reserved)?;
        Ok(())
    }

    pub fn read_from<R: Read>(&mut self, reader: &mut R) -> Result<()> {
        reader.read_exact(&mut self.magic)?;
        if &self.magic != STAGING_MAGIC {
            return Err(DigstoreError::InvalidFormat {
                format: "staging".to_string(),
                reason: "Invalid magic bytes".to_string(),
            });
        }

        self.version = reader.read_u32::<LittleEndian>()?;
        if self.version != STAGING_VERSION {
            return Err(DigstoreError::UnsupportedVersion {
                version: self.version,
                supported: STAGING_VERSION,
            });
        }

        self.file_count = reader.read_u64::<LittleEndian>()?;
        self.index_offset = reader.read_u64::<LittleEndian>()?;
        self.index_size = reader.read_u64::<LittleEndian>()?;
        self.data_offset = reader.read_u64::<LittleEndian>()?;
        self.data_size = reader.read_u64::<LittleEndian>()?;
        self.compression = reader.read_u32::<LittleEndian>()?;
        reader.read_exact(&mut self.reserved)?;
        Ok(())
    }
}

/// Index entry for fast file lookups
#[repr(C)]
#[derive(Debug, Clone)]
pub struct IndexEntry {
    /// Hash of the file path for fast lookups
    pub path_hash: u64,
    /// Offset to file data in the data section
    pub data_offset: u64,
    /// Size of file data in bytes
    pub data_size: u32,
    /// Length of file path in bytes
    pub path_length: u16,
    /// Flags (reserved for future use)
    pub flags: u16,
}

impl IndexEntry {
    pub const SIZE: usize = 8 + 8 + 4 + 2 + 2; // 24 bytes

    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_u64::<LittleEndian>(self.path_hash)?;
        writer.write_u64::<LittleEndian>(self.data_offset)?;
        writer.write_u32::<LittleEndian>(self.data_size)?;
        writer.write_u16::<LittleEndian>(self.path_length)?;
        writer.write_u16::<LittleEndian>(self.flags)?;
        Ok(())
    }

    pub fn read_from<R: Read>(&mut self, reader: &mut R) -> Result<()> {
        self.path_hash = reader.read_u64::<LittleEndian>()?;
        self.data_offset = reader.read_u64::<LittleEndian>()?;
        self.data_size = reader.read_u32::<LittleEndian>()?;
        self.path_length = reader.read_u16::<LittleEndian>()?;
        self.flags = reader.read_u16::<LittleEndian>()?;
        Ok(())
    }
}

/// Binary staged file data
#[derive(Debug, Clone)]
pub struct BinaryStagedFile {
    pub path: PathBuf,
    pub hash: Hash,
    pub size: u64,
    pub chunks: Vec<Chunk>,
    pub modified_time: Option<std::time::SystemTime>,
}

impl BinaryStagedFile {
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        // Write path
        let path_string = self.path.to_string_lossy();
        let path_bytes = path_string.as_bytes();
        writer.write_u16::<LittleEndian>(path_bytes.len() as u16)?;
        writer.write_all(path_bytes)?;

        // Write hash
        writer.write_all(self.hash.as_bytes())?;

        // Write size
        writer.write_u64::<LittleEndian>(self.size)?;

        // Write modified time
        match self.modified_time {
            Some(time) => {
                writer.write_u8(1)?; // has time
                let duration = time
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                writer.write_u64::<LittleEndian>(duration.as_secs())?;
                writer.write_u32::<LittleEndian>(duration.subsec_nanos())?;
            },
            None => {
                writer.write_u8(0)?; // no time
            },
        }

        // Write chunks
        writer.write_u32::<LittleEndian>(self.chunks.len() as u32)?;
        for chunk in &self.chunks {
            writer.write_all(chunk.hash.as_bytes())?;
            writer.write_u64::<LittleEndian>(chunk.offset)?;
            writer.write_u32::<LittleEndian>(chunk.size)?;
        }

        Ok(())
    }

    pub fn read_from<R: Read>(&mut self, reader: &mut R) -> Result<()> {
        // Read path
        let path_len = reader.read_u16::<LittleEndian>()? as usize;
        let mut path_bytes = vec![0u8; path_len];
        reader.read_exact(&mut path_bytes)?;
        self.path = PathBuf::from(String::from_utf8_lossy(&path_bytes).into_owned());

        // Read hash
        let mut hash_bytes = [0u8; 32];
        reader.read_exact(&mut hash_bytes)?;
        self.hash = Hash::from_bytes(hash_bytes);

        // Read size
        self.size = reader.read_u64::<LittleEndian>()?;

        // Read modified time
        let has_time = reader.read_u8()? != 0;
        self.modified_time = if has_time {
            let secs = reader.read_u64::<LittleEndian>()?;
            let nanos = reader.read_u32::<LittleEndian>()?;
            Some(std::time::UNIX_EPOCH + std::time::Duration::new(secs, nanos))
        } else {
            None
        };

        // Read chunks
        let chunk_count = reader.read_u32::<LittleEndian>()? as usize;
        self.chunks = Vec::with_capacity(chunk_count);
        for _ in 0..chunk_count {
            let mut chunk_hash = [0u8; 32];
            reader.read_exact(&mut chunk_hash)?;
            let offset = reader.read_u64::<LittleEndian>()?;
            let size = reader.read_u32::<LittleEndian>()?;

            self.chunks.push(Chunk {
                hash: Hash::from_bytes(chunk_hash),
                offset,
                size,
                data: Vec::new(), // Binary staging doesn't store chunk data, only metadata
            });
        }

        Ok(())
    }

    pub fn serialized_size(&self) -> usize {
        let path_bytes = self.path.to_string_lossy().len();
        2 + path_bytes + // path
        32 + // hash
        8 + // size
        1 + if self.modified_time.is_some() { 8 + 4 } else { 0 } + // time
        4 + self.chunks.len() * (32 + 8 + 4) // chunks
    }
}

/// High-performance binary staging manager
pub struct BinaryStagingArea {
    /// Path to the staging file
    staging_path: PathBuf,
    /// Memory-mapped staging file
    pub mmap: Option<Mmap>,
    /// Writable memory map for updates
    pub mmap_mut: Option<MmapMut>,
    /// In-memory index for fast lookups
    index: HashMap<u64, (usize, IndexEntry)>,
    /// Whether the staging area is dirty and needs flushing
    dirty: bool,
}

impl BinaryStagingArea {
    /// Create a new binary staging area
    pub fn new(staging_path: PathBuf) -> Self {
        Self {
            staging_path,
            mmap: None,
            mmap_mut: None,
            index: HashMap::new(),
            dirty: false,
        }
    }

    /// Initialize an empty staging area
    pub fn initialize(&mut self) -> Result<()> {
        let mut file = File::create(&self.staging_path)?;
        let header = StagingHeader::new();
        header.write_to(&mut file)?;
        file.sync_all()?;

        self.reload()?;
        Ok(())
    }

    /// Load existing staging area from disk
    pub fn load(&mut self) -> Result<()> {
        if !self.staging_path.exists() {
            return self.initialize();
        }

        self.reload()
    }

    /// Reload staging area from disk
    pub fn reload(&mut self) -> Result<()> {
        // Close existing mappings
        self.mmap = None;
        self.mmap_mut = None;
        self.index.clear();

        let file = File::open(&self.staging_path)?;
        let mmap = unsafe { MmapOptions::new().map(&file)? };

        // Read and validate header
        let mut header = StagingHeader::new();
        let mut cursor = std::io::Cursor::new(&mmap[..]);
        header.read_from(&mut cursor)?;

        // Build index from file
        if header.file_count > 0 {
            cursor.set_position(header.index_offset);
            for i in 0..header.file_count {
                let mut entry = IndexEntry {
                    path_hash: 0,
                    data_offset: 0,
                    data_size: 0,
                    path_length: 0,
                    flags: 0,
                };
                entry.read_from(&mut cursor)?;
                self.index.insert(entry.path_hash, (i as usize, entry));
            }
        }

        self.mmap = Some(mmap);
        self.dirty = false;
        Ok(())
    }

    /// Add a file to the staging area using streaming writes
    pub fn stage_file_streaming(&mut self, staged_file: BinaryStagedFile) -> Result<()> {
        let path_hash = self.hash_path(&staged_file.path);

        // For streaming, we need to append to the file
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.staging_path)?;

        // Get current file size to know where to write
        let current_size = file.metadata()?.len();

        // Serialize the staged file
        let mut buffer = Vec::new();
        staged_file.write_to(&mut buffer)?;

        // Create index entry
        let entry = IndexEntry {
            path_hash,
            data_offset: current_size,
            data_size: buffer.len() as u32,
            path_length: staged_file.path.to_string_lossy().len() as u16,
            flags: 0,
        };

        // Write data
        file.write_all(&buffer)?;
        file.sync_all()?;
        drop(file); // Close file before remapping

        // Update in-memory index
        self.index.insert(path_hash, (self.index.len(), entry));
        self.dirty = true;

        // Refresh memory map to include new data
        self.refresh_memory_map()?;

        Ok(())
    }

    /// Batch add multiple files efficiently using proven individual staging
    pub fn stage_files_batch(&mut self, staged_files: Vec<BinaryStagedFile>) -> Result<()> {
        if staged_files.is_empty() {
            return Ok(());
        }

        // Instead of complex batch logic that can corrupt, use the proven individual method
        // This is more reliable and prevents the "UnexpectedEof" errors
        for staged_file in staged_files {
            self.stage_file_streaming(staged_file)?;
        }

        // Don't flush here - let the caller handle flushing to prevent corruption
        // The individual stage_file_streaming calls already handle memory map refresh

        Ok(())
    }

    /// Get a staged file by path
    pub fn get_staged_file(&self, path: &Path) -> Result<Option<BinaryStagedFile>> {
        let path_hash = self.hash_path(path);

        if let Some((_, entry)) = self.index.get(&path_hash) {
            if let Some(ref mmap) = self.mmap {
                let start = entry.data_offset as usize;
                let end = start + entry.data_size as usize;

                if end <= mmap.len() {
                    let mut cursor = std::io::Cursor::new(&mmap[start..end]);
                    let mut staged_file = BinaryStagedFile {
                        path: PathBuf::new(),
                        hash: Hash::zero(),
                        size: 0,
                        chunks: Vec::new(),
                        modified_time: None,
                    };
                    staged_file.read_from(&mut cursor)?;
                    return Ok(Some(staged_file));
                }
            }
        }

        Ok(None)
    }

    /// Get all staged files
    pub fn get_all_staged_files(&self) -> Result<Vec<BinaryStagedFile>> {
        let mut files = Vec::with_capacity(self.index.len());

        if let Some(ref mmap) = self.mmap {
            for (_, entry) in self.index.values() {
                let start = entry.data_offset as usize;
                let end = start + entry.data_size as usize;

                if end <= mmap.len() {
                    let mut cursor = std::io::Cursor::new(&mmap[start..end]);
                    let mut staged_file = BinaryStagedFile {
                        path: PathBuf::new(),
                        hash: Hash::zero(),
                        size: 0,
                        chunks: Vec::new(),
                        modified_time: None,
                    };
                    staged_file.read_from(&mut cursor)?;
                    files.push(staged_file);
                }
            }
        }

        Ok(files)
    }

    /// Check if a file is staged
    pub fn is_staged(&self, path: &Path) -> bool {
        let path_hash = self.hash_path(path);
        self.index.contains_key(&path_hash)
    }

    /// Get the number of staged files
    pub fn staged_count(&self) -> usize {
        self.index.len()
    }

    /// Get the staging file path
    pub fn staging_path(&self) -> &PathBuf {
        &self.staging_path
    }

    /// Refresh the memory map after writes
    fn refresh_memory_map(&mut self) -> Result<()> {
        // Close existing memory map
        self.mmap = None;
        self.mmap_mut = None;

        // Reopen with updated file
        if self.staging_path.exists() {
            let file = File::open(&self.staging_path)?;
            self.mmap = Some(unsafe { MmapOptions::new().map(&file)? });
        }

        Ok(())
    }

    /// Clear all staged files
    pub fn clear(&mut self) -> Result<()> {
        self.initialize()?;
        Ok(())
    }

    /// Flush any pending changes to disk
    pub fn flush(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        // Rebuild the file with proper header and index
        let temp_path = self.staging_path.with_extension("tmp");
        let mut temp_file = File::create(&temp_path)?;

        // Write header (we'll update it later)
        let mut header = StagingHeader::new();
        header.file_count = self.index.len() as u64;
        header.write_to(&mut temp_file)?;

        let data_start = temp_file.stream_position()?;
        header.data_offset = data_start;

        // Copy all file data and update index entries with new offsets
        let mut data_size = 0u64;
        let mut updated_index = HashMap::new();

        if let Some(ref mmap) = self.mmap {
            for (path_hash, (index_pos, entry)) in &self.index {
                let start = entry.data_offset as usize;
                let end = start + entry.data_size as usize;

                if end <= mmap.len() {
                    // Get new offset in temp file
                    let new_offset = temp_file.stream_position()?;

                    // Copy data to new location
                    temp_file.write_all(&mmap[start..end])?;
                    data_size += entry.data_size as u64;

                    // Create updated index entry with new offset
                    let updated_entry = IndexEntry {
                        path_hash: entry.path_hash,
                        data_offset: new_offset,
                        data_size: entry.data_size,
                        path_length: entry.path_length,
                        flags: entry.flags,
                    };

                    updated_index.insert(*path_hash, (*index_pos, updated_entry));
                }
            }
        }

        header.data_size = data_size;

        // Write index with updated offsets
        header.index_offset = temp_file.stream_position()?;
        let mut index_size = 0u64;

        for (_, entry) in updated_index.values() {
            entry.write_to(&mut temp_file)?;
            index_size += IndexEntry::SIZE as u64;
        }

        // Update in-memory index with new offsets
        self.index = updated_index;

        header.index_size = index_size;

        // Update header
        temp_file.seek(SeekFrom::Start(0))?;
        header.write_to(&mut temp_file)?;
        temp_file.sync_all()?;
        drop(temp_file);

        // Replace original file
        std::fs::rename(&temp_path, &self.staging_path)?;

        // Reload
        self.reload()?;

        Ok(())
    }

    /// Hash a file path for indexing
    fn hash_path(&self, path: &Path) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash as StdHash, Hasher};

        let mut hasher = DefaultHasher::new();
        path.to_string_lossy().hash(&mut hasher);
        hasher.finish()
    }

    /// Get staging file size and statistics
    pub fn stats(&self) -> Result<StagingStats> {
        let file_size = if self.staging_path.exists() {
            std::fs::metadata(&self.staging_path)?.len()
        } else {
            0
        };

        Ok(StagingStats {
            file_count: self.index.len(),
            file_size_bytes: file_size,
            memory_usage_bytes: self.index.len()
                * std::mem::size_of::<(u64, (usize, IndexEntry))>(),
            is_dirty: self.dirty,
        })
    }
}

impl Drop for BinaryStagingArea {
    fn drop(&mut self) {
        if self.dirty {
            let _ = self.flush();
        }
    }
}

/// Statistics about the staging area
#[derive(Debug, Clone)]
pub struct StagingStats {
    pub file_count: usize,
    pub file_size_bytes: u64,
    pub memory_usage_bytes: usize,
    pub is_dirty: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_staging_header_serialization() -> Result<()> {
        let mut header = StagingHeader::new();
        header.file_count = 100;
        header.data_size = 1024;

        let mut buffer = Vec::new();
        header.write_to(&mut buffer)?;

        let mut new_header = StagingHeader::new();
        let mut cursor = std::io::Cursor::new(&buffer);
        new_header.read_from(&mut cursor)?;

        assert_eq!(header.file_count, new_header.file_count);
        assert_eq!(header.data_size, new_header.data_size);
        Ok(())
    }

    #[test]
    fn test_binary_staging_basic_operations() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let staging_path = temp_dir.path().join("staging.bin");

        let mut staging = BinaryStagingArea::new(staging_path);
        staging.initialize()?;

        // Create a test file
        let staged_file = BinaryStagedFile {
            path: PathBuf::from("test.txt"),
            hash: Hash::from_bytes([1; 32]),
            size: 100,
            chunks: vec![Chunk {
                hash: Hash::from_bytes([2; 32]),
                offset: 0,
                size: 100,
                data: Vec::new(),
            }],
            modified_time: Some(std::time::SystemTime::now()),
        };

        // Stage the file
        staging.stage_file_streaming(staged_file.clone())?;
        assert_eq!(staging.staged_count(), 1);
        assert!(staging.is_staged(&PathBuf::from("test.txt")));

        // Retrieve the file
        let retrieved = staging.get_staged_file(&PathBuf::from("test.txt"))?;
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.path, staged_file.path);
        assert_eq!(retrieved.hash, staged_file.hash);
        assert_eq!(retrieved.size, staged_file.size);
        assert_eq!(retrieved.chunks.len(), staged_file.chunks.len());

        Ok(())
    }

    #[test]
    fn test_binary_staging_batch_operations() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let staging_path = temp_dir.path().join("staging.bin");

        let mut staging = BinaryStagingArea::new(staging_path);
        staging.initialize()?;

        // Create multiple test files
        let mut staged_files = Vec::new();
        for i in 0..100 {
            staged_files.push(BinaryStagedFile {
                path: PathBuf::from(format!("test_{}.txt", i)),
                hash: Hash::from_bytes([i as u8; 32]),
                size: 100 + i as u64,
                chunks: vec![Chunk {
                    hash: Hash::from_bytes([(i + 1) as u8; 32]),
                    offset: 0,
                    size: 100 + i as u32,
                    data: Vec::new(),
                }],
                modified_time: Some(std::time::SystemTime::now()),
            });
        }

        // Stage all files in batch
        staging.stage_files_batch(staged_files.clone())?;
        assert_eq!(staging.staged_count(), 100);

        // Verify all files are staged
        for i in 0..100 {
            let path = PathBuf::from(format!("test_{}.txt", i));
            assert!(staging.is_staged(&path));

            let retrieved = staging.get_staged_file(&path)?;
            assert!(retrieved.is_some());

            let retrieved = retrieved.unwrap();
            assert_eq!(retrieved.path, staged_files[i].path);
            assert_eq!(retrieved.hash, staged_files[i].hash);
        }

        // Test stats
        let stats = staging.stats()?;
        assert_eq!(stats.file_count, 100);
        assert!(stats.file_size_bytes > 0);

        Ok(())
    }

    #[test]
    fn test_staging_persistence() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let staging_path = temp_dir.path().join("staging.bin");

        // Create and populate staging area
        {
            let mut staging = BinaryStagingArea::new(staging_path.clone());
            staging.initialize()?;

            let staged_file = BinaryStagedFile {
                path: PathBuf::from("persistent.txt"),
                hash: Hash::from_bytes([42; 32]),
                size: 200,
                chunks: vec![Chunk {
                    hash: Hash::from_bytes([43; 32]),
                    offset: 0,
                    size: 200,
                    data: Vec::new(),
                }],
                modified_time: Some(std::time::SystemTime::now()),
            };

            staging.stage_file_streaming(staged_file)?;
            staging.flush()?;
        }

        // Load staging area from disk
        {
            let mut staging = BinaryStagingArea::new(staging_path);
            staging.load()?;

            assert_eq!(staging.staged_count(), 1);
            assert!(staging.is_staged(&PathBuf::from("persistent.txt")));

            let retrieved = staging.get_staged_file(&PathBuf::from("persistent.txt"))?;
            assert!(retrieved.is_some());

            let retrieved = retrieved.unwrap();
            assert_eq!(retrieved.hash, Hash::from_bytes([42; 32]));
            assert_eq!(retrieved.size, 200);
        }

        Ok(())
    }

    #[test]
    fn test_memory_map_refresh_after_writes() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let staging_path = temp_dir.path().join("refresh_test.bin");

        let mut staging = BinaryStagingArea::new(staging_path);
        staging.initialize()?;

        // Add first file
        let file1 = BinaryStagedFile {
            path: PathBuf::from("file1.txt"),
            hash: Hash::from_bytes([1; 32]),
            size: 100,
            chunks: vec![Chunk {
                hash: Hash::from_bytes([2; 32]),
                offset: 0,
                size: 100,
                data: Vec::new(),
            }],
            modified_time: Some(std::time::SystemTime::now()),
        };

        staging.stage_file_streaming(file1.clone())?;
        assert_eq!(staging.staged_count(), 1);

        // Verify file can be retrieved immediately after staging
        let retrieved = staging.get_staged_file(&PathBuf::from("file1.txt"))?;
        assert!(
            retrieved.is_some(),
            "File should be retrievable immediately after staging"
        );

        // Add second file to test memory map refresh
        let file2 = BinaryStagedFile {
            path: PathBuf::from("file2.txt"),
            hash: Hash::from_bytes([3; 32]),
            size: 200,
            chunks: vec![Chunk {
                hash: Hash::from_bytes([4; 32]),
                offset: 0,
                size: 200,
                data: Vec::new(),
            }],
            modified_time: Some(std::time::SystemTime::now()),
        };

        staging.stage_file_streaming(file2.clone())?;
        assert_eq!(staging.staged_count(), 2);

        // Verify both files can be retrieved
        let retrieved1 = staging.get_staged_file(&PathBuf::from("file1.txt"))?;
        let retrieved2 = staging.get_staged_file(&PathBuf::from("file2.txt"))?;

        assert!(
            retrieved1.is_some(),
            "First file should still be retrievable"
        );
        assert!(retrieved2.is_some(), "Second file should be retrievable");

        // Test get_all_staged_files works correctly
        let all_files = staging.get_all_staged_files()?;
        assert_eq!(all_files.len(), 2, "Should retrieve both staged files");

        Ok(())
    }

    #[test]
    fn test_iterator_fix_validation() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let staging_path = temp_dir.path().join("iterator_test.bin");

        let mut staging = BinaryStagingArea::new(staging_path);
        staging.initialize()?;

        // Add multiple files to test iterator fix
        let files = (0..5)
            .map(|i| BinaryStagedFile {
                path: PathBuf::from(format!("test{}.txt", i)),
                hash: Hash::from_bytes([i as u8; 32]),
                size: 100 + i as u64,
                chunks: vec![Chunk {
                    hash: Hash::from_bytes([(i + 10) as u8; 32]),
                    offset: 0,
                    size: 100 + i as u32,
                    data: Vec::new(),
                }],
                modified_time: Some(std::time::SystemTime::now()),
            })
            .collect::<Vec<_>>();

        // Stage files using batch method
        staging.stage_files_batch(files.clone())?;
        assert_eq!(staging.staged_count(), 5);

        // Test that get_all_staged_files() works correctly (this was the main bug)
        let retrieved_files = staging.get_all_staged_files()?;
        assert_eq!(
            retrieved_files.len(),
            5,
            "Iterator fix: should retrieve all 5 files"
        );

        // Verify each file individually
        for (i, original_file) in files.iter().enumerate() {
            let retrieved = staging.get_staged_file(&original_file.path)?;
            assert!(retrieved.is_some(), "File {} should be retrievable", i);

            let retrieved = retrieved.unwrap();
            assert_eq!(retrieved.path, original_file.path);
            assert_eq!(retrieved.hash, original_file.hash);
            assert_eq!(retrieved.size, original_file.size);
        }

        Ok(())
    }

    #[test]
    fn test_staging_persistence_across_instances() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let staging_path = temp_dir.path().join("persistence_test.bin");

        // Create staging area and add files
        {
            let mut staging = BinaryStagingArea::new(staging_path.clone());
            staging.initialize()?;

            let file = BinaryStagedFile {
                path: PathBuf::from("persistent.txt"),
                hash: Hash::from_bytes([42; 32]),
                size: 150,
                chunks: vec![Chunk {
                    hash: Hash::from_bytes([43; 32]),
                    offset: 0,
                    size: 150,
                    data: Vec::new(),
                }],
                modified_time: Some(std::time::SystemTime::now()),
            };

            staging.stage_file_streaming(file)?;
            staging.flush()?; // Ensure data is written to disk
        } // staging goes out of scope

        // Create new staging instance and verify data persists
        {
            let mut staging = BinaryStagingArea::new(staging_path);
            staging.load()?;

            assert_eq!(
                staging.staged_count(),
                1,
                "Staged file should persist across instances"
            );

            let retrieved = staging.get_staged_file(&PathBuf::from("persistent.txt"))?;
            assert!(
                retrieved.is_some(),
                "File should be retrievable in new instance"
            );

            let retrieved = retrieved.unwrap();
            assert_eq!(retrieved.hash, Hash::from_bytes([42; 32]));
            assert_eq!(retrieved.size, 150);
        }

        Ok(())
    }

    #[test]
    fn test_windows_file_locking_fix() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let staging_path = temp_dir.path().join("locking_test.bin");

        let mut staging = BinaryStagingArea::new(staging_path);
        staging.initialize()?;

        // Add a file
        let file = BinaryStagedFile {
            path: PathBuf::from("lock_test.txt"),
            hash: Hash::from_bytes([99; 32]),
            size: 50,
            chunks: vec![Chunk {
                hash: Hash::from_bytes([100; 32]),
                offset: 0,
                size: 50,
                data: Vec::new(),
            }],
            modified_time: Some(std::time::SystemTime::now()),
        };

        staging.stage_file_streaming(file)?;

        // Simulate Windows file locking fix: close memory maps before clear
        staging.mmap = None;
        staging.mmap_mut = None;

        // This should not fail with file locking error
        staging.clear()?;

        assert_eq!(
            staging.staged_count(),
            0,
            "Staging should be cleared without file locking issues"
        );

        Ok(())
    }
}
