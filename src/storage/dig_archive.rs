//! .dig archive file format for storing multiple layers in a single file
//!
//! This module implements a single-file archive format that replaces the
//! directory-based approach for storing layer files. Features:
//! - Single .dig file per store (replaces directory)
//! - Memory-mapped access for performance
//! - Layer indexing for fast lookups
//! - Compression support for space efficiency
//! - Atomic operations for consistency

use crate::core::{types::*, error::{Result, DigstoreError}};
use crate::storage::layer::Layer;
use std::path::{Path, PathBuf};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom, BufReader, BufWriter};
use std::collections::HashMap;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use memmap2::{Mmap, MmapOptions};
use crc32fast::Hasher as Crc32Hasher;

/// Magic bytes for .dig archive format
const ARCHIVE_MAGIC: &[u8; 8] = b"DIGARCH\0";
const ARCHIVE_VERSION: u32 = 1;

/// Archive header (64 bytes)
#[repr(C)]
#[derive(Debug, Clone)]
pub struct ArchiveHeader {
    /// Magic bytes: "DIGARCH\0"
    pub magic: [u8; 8],
    /// Format version
    pub version: u32,
    /// Number of layers in archive
    pub layer_count: u32,
    /// Offset to layer index section
    pub index_offset: u64,
    /// Size of index section in bytes
    pub index_size: u64,
    /// Offset to layer data section
    pub data_offset: u64,
    /// Size of data section in bytes
    pub data_size: u64,
    /// Reserved for future use
    pub reserved: [u8; 24],
}

impl ArchiveHeader {
    pub const SIZE: usize = 64;

    pub fn new() -> Self {
        Self {
            magic: *ARCHIVE_MAGIC,
            version: ARCHIVE_VERSION,
            layer_count: 0,
            index_offset: Self::SIZE as u64,
            index_size: 0,
            data_offset: Self::SIZE as u64,
            data_size: 0,
            reserved: [0; 24],
        }
    }

    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(&self.magic)?;
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(self.layer_count)?;
        writer.write_u64::<LittleEndian>(self.index_offset)?;
        writer.write_u64::<LittleEndian>(self.index_size)?;
        writer.write_u64::<LittleEndian>(self.data_offset)?;
        writer.write_u64::<LittleEndian>(self.data_size)?;
        writer.write_all(&self.reserved)?;
        Ok(())
    }

    pub fn read_from<R: Read>(&mut self, reader: &mut R) -> Result<()> {
        reader.read_exact(&mut self.magic)?;
        if &self.magic != ARCHIVE_MAGIC {
            return Err(DigstoreError::InvalidFormat {
                format: "dig archive".to_string(),
                reason: "Invalid magic bytes".to_string(),
            });
        }
        
        self.version = reader.read_u32::<LittleEndian>()?;
        if self.version != ARCHIVE_VERSION {
            return Err(DigstoreError::UnsupportedVersion {
                version: self.version,
                supported: ARCHIVE_VERSION,
            });
        }

        self.layer_count = reader.read_u32::<LittleEndian>()?;
        self.index_offset = reader.read_u64::<LittleEndian>()?;
        self.index_size = reader.read_u64::<LittleEndian>()?;
        self.data_offset = reader.read_u64::<LittleEndian>()?;
        self.data_size = reader.read_u64::<LittleEndian>()?;
        reader.read_exact(&mut self.reserved)?;
        Ok(())
    }
}

/// Layer index entry (80 bytes)
#[repr(C)]
#[derive(Debug, Clone)]
pub struct LayerIndexEntry {
    /// SHA-256 hash of layer (used as identifier)
    pub layer_hash: [u8; 32],
    /// Offset to layer data in archive
    pub offset: u64,
    /// Size of layer data in bytes
    pub size: u64,
    /// Compression type (0=none, 1=zstd)
    pub compression: u32,
    /// CRC32 checksum of layer data
    pub checksum: u32,
    /// Reserved for future use
    pub reserved: [u8; 8],
}

impl LayerIndexEntry {
    pub const SIZE: usize = 80;

    pub fn new(layer_hash: Hash, offset: u64, size: u64) -> Self {
        Self {
            layer_hash: *layer_hash.as_bytes(),
            offset,
            size,
            compression: 0,
            checksum: 0,
            reserved: [0; 8],
        }
    }

    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(&self.layer_hash)?;
        writer.write_u64::<LittleEndian>(self.offset)?;
        writer.write_u64::<LittleEndian>(self.size)?;
        writer.write_u32::<LittleEndian>(self.compression)?;
        writer.write_u32::<LittleEndian>(self.checksum)?;
        writer.write_all(&self.reserved)?;
        Ok(())
    }

    pub fn read_from<R: Read>(&mut self, reader: &mut R) -> Result<()> {
        reader.read_exact(&mut self.layer_hash)?;
        self.offset = reader.read_u64::<LittleEndian>()?;
        self.size = reader.read_u64::<LittleEndian>()?;
        self.compression = reader.read_u32::<LittleEndian>()?;
        self.checksum = reader.read_u32::<LittleEndian>()?;
        reader.read_exact(&mut self.reserved)?;
        Ok(())
    }

    pub fn hash(&self) -> Hash {
        Hash::from_bytes(self.layer_hash)
    }
}

/// Statistics about the archive
#[derive(Debug, Clone)]
pub struct ArchiveStats {
    pub layer_count: usize,
    pub total_size: u64,
    pub data_size: u64,
    pub index_size: u64,
    pub compression_ratio: f64,
    pub fragmentation: f64,
}

/// .dig archive manager
pub struct DigArchive {
    /// Path to the archive file
    archive_path: PathBuf,
    /// Archive header
    header: ArchiveHeader,
    /// Layer index for fast lookups
    pub index: HashMap<Hash, LayerIndexEntry>,
    /// Memory-mapped archive file (for reads)
    pub mmap: Option<Mmap>,
    /// Whether the archive has been modified
    dirty: bool,
}

impl DigArchive {
    /// Create a new empty archive
    pub fn create(archive_path: PathBuf) -> Result<Self> {
        let mut file = File::create(&archive_path)?;
        let header = ArchiveHeader::new();
        header.write_to(&mut file)?;
        file.sync_all()?;
        drop(file);

        let mut archive = Self {
            archive_path,
            header,
            index: HashMap::new(),
            mmap: None,
            dirty: false,
        };

        archive.load_mmap()?;
        Ok(archive)
    }

    /// Open existing archive
    pub fn open(archive_path: PathBuf) -> Result<Self> {
        if !archive_path.exists() {
            return Err(DigstoreError::file_not_found(archive_path));
        }

        let mut archive = Self {
            archive_path,
            header: ArchiveHeader::new(),
            index: HashMap::new(),
            mmap: None,
            dirty: false,
        };

        archive.load()?;
        Ok(archive)
    }

    /// Load archive header and index
    fn load(&mut self) -> Result<()> {
        // Load memory map
        self.load_mmap()?;

        // Read header
        if let Some(ref mmap) = self.mmap {
            let mut cursor = std::io::Cursor::new(&mmap[..]);
            self.header.read_from(&mut cursor)?;

            // Read index
            if self.header.layer_count > 0 {
                cursor.set_position(self.header.index_offset);
                self.index.clear();
                
                for _ in 0..self.header.layer_count {
                    let mut entry = LayerIndexEntry::new(Hash::zero(), 0, 0);
                    entry.read_from(&mut cursor)?;
                    self.index.insert(entry.hash(), entry);
                }
            }
        }

        Ok(())
    }

    /// Load memory map for the archive
    fn load_mmap(&mut self) -> Result<()> {
        let file = File::open(&self.archive_path)?;
        let mmap = unsafe { MmapOptions::new().map(&file)? };
        self.mmap = Some(mmap);
        Ok(())
    }

    /// Add a layer to the archive
    pub fn add_layer(&mut self, layer_hash: Hash, layer_data: &[u8]) -> Result<()> {
        // Calculate checksum
        let mut hasher = Crc32Hasher::new();
        hasher.update(layer_data);
        let checksum = hasher.finalize();

        // If this is the first layer, we need to write after the header
        let offset = if self.index.is_empty() {
            ArchiveHeader::SIZE as u64
        } else {
            // Append after existing data
            let mut file = File::open(&self.archive_path)?;
            file.metadata()?.len()
        };

        // Write layer data to archive
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&self.archive_path)?;
        
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(layer_data)?;
        file.sync_all()?;
        drop(file);

        // Create index entry
        let entry = LayerIndexEntry {
            layer_hash: *layer_hash.as_bytes(),
            offset,
            size: layer_data.len() as u64,
            compression: 0,
            checksum,
            reserved: [0; 8],
        };

        // Update in-memory index
        self.index.insert(layer_hash, entry);
        self.header.layer_count = self.index.len() as u32;
        self.dirty = true;

        // Immediately flush to update header and index
        self.flush()?;

        Ok(())
    }

    /// Get raw layer data by hash (for metadata layers like Layer 0)
    pub fn get_layer_data(&self, layer_hash: &Hash) -> Result<Vec<u8>> {
        let entry = self.index.get(layer_hash)
            .ok_or_else(|| DigstoreError::layer_not_found(*layer_hash))?;

        if let Some(ref mmap) = self.mmap {
            let start = entry.offset as usize;
            let end = start + entry.size as usize;
            
            if end <= mmap.len() {
                // Verify checksum
                let mut hasher = Crc32Hasher::new();
                hasher.update(&mmap[start..end]);
                let calculated_checksum = hasher.finalize();
                
                if calculated_checksum != entry.checksum {
                    return Err(DigstoreError::ChecksumMismatch {
                        expected: entry.checksum.to_string(),
                        actual: calculated_checksum.to_string(),
                    });
                }

                Ok(mmap[start..end].to_vec())
            } else {
                Err(DigstoreError::internal("Layer data extends beyond archive"))
            }
        } else {
            // Fallback: read from file directly
            let mut file = File::open(&self.archive_path)?;
            file.seek(SeekFrom::Start(entry.offset))?;
            let mut buffer = vec![0u8; entry.size as usize];
            file.read_exact(&mut buffer)?;
            
            // Verify checksum
            let mut hasher = Crc32Hasher::new();
            hasher.update(&buffer);
            let calculated_checksum = hasher.finalize();
            
            if calculated_checksum != entry.checksum {
                return Err(DigstoreError::ChecksumMismatch {
                    expected: entry.checksum.to_string(),
                    actual: calculated_checksum.to_string(),
                });
            }
            
            Ok(buffer)
        }
    }

    /// Get a layer by hash
    pub fn get_layer(&self, layer_hash: &Hash) -> Result<Layer> {
        let entry = self.index.get(layer_hash)
            .ok_or_else(|| DigstoreError::layer_not_found(*layer_hash))?;

        if let Some(ref mmap) = self.mmap {
            let start = entry.offset as usize;
            let end = start + entry.size as usize;
            
            if end <= mmap.len() {
                // Verify checksum
                let mut hasher = Crc32Hasher::new();
                hasher.update(&mmap[start..end]);
                let calculated_checksum = hasher.finalize();
                
                if calculated_checksum != entry.checksum {
                    return Err(DigstoreError::ChecksumMismatch {
                        expected: entry.checksum.to_string(),
                        actual: calculated_checksum.to_string(),
                    });
                }

                // Parse layer from data
                let layer_data = &mmap[start..end];
                let mut cursor = std::io::Cursor::new(layer_data);
                Layer::read_from_reader(&mut cursor)
            } else {
                Err(DigstoreError::internal("Layer data extends beyond archive"))
            }
        } else {
            Err(DigstoreError::internal("Archive not memory-mapped"))
        }
    }

    /// List all layers in the archive
    pub fn list_layers(&self) -> Vec<(Hash, &LayerIndexEntry)> {
        self.index.iter()
            .map(|(hash, entry)| (*hash, entry))
            .collect()
    }

    /// Get archive statistics
    pub fn stats(&self) -> ArchiveStats {
        let total_size = self.archive_path.metadata()
            .map(|m| m.len())
            .unwrap_or(0);

        let data_size = self.index.values()
            .map(|entry| entry.size)
            .sum::<u64>();

        let index_size = self.header.index_size;
        let overhead = if total_size >= data_size {
            total_size - data_size
        } else {
            0 // Prevent overflow
        };
        
        ArchiveStats {
            layer_count: self.index.len(),
            total_size,
            data_size,
            index_size,
            compression_ratio: if data_size > 0 {
                data_size as f64 / total_size as f64
            } else {
                1.0
            },
            fragmentation: if total_size > 0 {
                overhead as f64 / total_size as f64
            } else {
                0.0
            },
        }
    }

    /// Check if a layer exists in the archive
    pub fn has_layer(&self, layer_hash: &Hash) -> bool {
        self.index.contains_key(layer_hash)
    }

    /// Get the number of layers in the archive
    pub fn layer_count(&self) -> usize {
        self.index.len()
    }

    /// Flush any pending changes to disk
    pub fn flush(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        // Rebuild the archive with updated header and index
        let temp_path = self.archive_path.with_extension("tmp");
        let mut temp_file = File::create(&temp_path)?;
        
        // Write header (we'll update it later)
        self.header.write_to(&mut temp_file)?;
        
        let data_start = temp_file.stream_position()?;
        self.header.data_offset = data_start;
        
        // Copy all layer data and update offsets
        let mut data_size = 0u64;
        let mut updated_index = HashMap::new();
        
        // Read from the original file instead of stale memory map
        let mut source_file = File::open(&self.archive_path)?;
        
        for (layer_hash, entry) in &self.index {
            let new_offset = temp_file.stream_position()?;
            
            // Read layer data from source file
            source_file.seek(SeekFrom::Start(entry.offset))?;
            let mut buffer = vec![0u8; entry.size as usize];
            source_file.read_exact(&mut buffer)?;
            
            // Write to temp file
            temp_file.write_all(&buffer)?;
            data_size += entry.size;
            
            // Create updated index entry
            let mut updated_entry = entry.clone();
            updated_entry.offset = new_offset;
            updated_index.insert(*layer_hash, updated_entry);
        }
        
        // Update our index with new offsets
        self.index = updated_index;
        
        self.header.data_size = data_size;
        
        // Write index
        self.header.index_offset = temp_file.stream_position()?;
        let mut index_size = 0u64;
        
        for entry in self.index.values() {
            entry.write_to(&mut temp_file)?;
            index_size += LayerIndexEntry::SIZE as u64;
        }
        
        self.header.index_size = index_size;
        self.header.layer_count = self.index.len() as u32;
        
        // Update header
        temp_file.seek(SeekFrom::Start(0))?;
        self.header.write_to(&mut temp_file)?;
        temp_file.sync_all()?;
        drop(temp_file);
        
        // Replace original file
        std::fs::rename(&temp_path, &self.archive_path)?;
        
        // Reload
        self.load()?;
        
        Ok(())
    }

    /// Compact the archive by removing gaps and optimizing layout
    pub fn compact(&mut self) -> Result<()> {
        // Force a rebuild which will compact the archive
        self.dirty = true;
        self.flush()
    }

    /// Verify archive integrity
    pub fn verify(&self) -> Result<Vec<String>> {
        let mut issues = Vec::new();

        // Verify header
        if &self.header.magic != ARCHIVE_MAGIC {
            issues.push("Invalid magic bytes in header".to_string());
        }

        if self.header.version != ARCHIVE_VERSION {
            issues.push(format!("Unsupported version: {}", self.header.version));
        }

        // Verify index entries
        if let Some(ref mmap) = self.mmap {
            for (hash, entry) in &self.index {
                let start = entry.offset as usize;
                let end = start + entry.size as usize;
                
                if end > mmap.len() {
                    issues.push(format!("Layer {} extends beyond archive", hash));
                    continue;
                }

                // Verify checksum
                let mut hasher = Crc32Hasher::new();
                hasher.update(&mmap[start..end]);
                let calculated_checksum = hasher.finalize();
                
                if calculated_checksum != entry.checksum {
                    issues.push(format!("Checksum mismatch for layer {}", hash));
                }

                // Try to parse layer
                let layer_data = &mmap[start..end];
                let mut cursor = std::io::Cursor::new(layer_data);
                if Layer::read_from_reader(&mut cursor).is_err() {
                    issues.push(format!("Layer {} is corrupted or unparseable", hash));
                }
            }
        }

        Ok(issues)
    }

    /// Migrate from old directory-based format
    pub fn migrate_from_directory(archive_path: PathBuf, directory_path: &Path) -> Result<Self> {
        // Create new archive
        let mut archive = Self::create(archive_path)?;

        // Find all .layer files in directory
        let layer_files = std::fs::read_dir(directory_path)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry.path().extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext == "layer")
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        // Add each layer to archive
        for layer_file in layer_files {
            let layer_path = layer_file.path();
            let layer_data = std::fs::read(&layer_path)?;
            
            // Extract layer hash from filename
            if let Some(filename) = layer_path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(layer_hash) = Hash::from_hex(filename) {
                    archive.add_layer(layer_hash, &layer_data)?;
                }
            }
        }

        // Flush to disk
        archive.flush()?;
        
        Ok(archive)
    }

    /// Get archive file path
    pub fn path(&self) -> &Path {
        &self.archive_path
    }
}

impl Drop for DigArchive {
    fn drop(&mut self) {
        if self.dirty {
            let _ = self.flush();
        }
    }
}

/// Helper function to get archive path for a store ID
pub fn get_archive_path(store_id: &StoreId) -> Result<PathBuf> {
    use directories::UserDirs;
    
    let user_dirs = UserDirs::new()
        .ok_or_else(|| DigstoreError::internal("Could not determine user directory"))?;
    
    let dig_dir = user_dirs.home_dir().join(".dig");
    std::fs::create_dir_all(&dig_dir)?;
    
    Ok(dig_dir.join(format!("{}.dig", store_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_archive_header_serialization() -> Result<()> {
        let mut header = ArchiveHeader::new();
        header.layer_count = 5;
        header.data_size = 1024;

        let mut buffer = Vec::new();
        header.write_to(&mut buffer)?;

        let mut new_header = ArchiveHeader::new();
        let mut cursor = std::io::Cursor::new(&buffer);
        new_header.read_from(&mut cursor)?;

        assert_eq!(header.layer_count, new_header.layer_count);
        assert_eq!(header.data_size, new_header.data_size);
        Ok(())
    }

    #[test]
    fn test_layer_index_entry_serialization() -> Result<()> {
        let hash = Hash::from_bytes([42; 32]);
        let entry = LayerIndexEntry::new(hash, 1000, 2048);

        let mut buffer = Vec::new();
        entry.write_to(&mut buffer)?;

        let mut new_entry = LayerIndexEntry::new(Hash::zero(), 0, 0);
        let mut cursor = std::io::Cursor::new(&buffer);
        new_entry.read_from(&mut cursor)?;

        assert_eq!(entry.layer_hash, new_entry.layer_hash);
        assert_eq!(entry.offset, new_entry.offset);
        assert_eq!(entry.size, new_entry.size);
        Ok(())
    }

    #[test]
    fn test_archive_creation_and_layer_operations() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let archive_path = temp_dir.path().join("test.dig");
        
        // Create archive
        let mut archive = DigArchive::create(archive_path.clone())?;
        assert_eq!(archive.layer_count(), 0);

        // Add test layer
        let layer_hash = Hash::from_bytes([1; 32]);
        let layer_data = b"test layer data";
        archive.add_layer(layer_hash, layer_data)?;
        
        assert_eq!(archive.layer_count(), 1);
        assert!(archive.has_layer(&layer_hash));

        // Flush and reload
        archive.flush()?;
        drop(archive);

        // Reopen and verify
        let archive = DigArchive::open(archive_path)?;
        assert_eq!(archive.layer_count(), 1);
        assert!(archive.has_layer(&layer_hash));

        // List layers
        let layers = archive.list_layers();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].0, layer_hash);

        Ok(())
    }

    #[test]
    fn test_archive_migration() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let old_dir = temp_dir.path().join("old_store");
        let archive_path = temp_dir.path().join("migrated.dig");
        
        // Create old directory structure
        fs::create_dir_all(&old_dir)?;
        
        // Create test layer files
        let layer1_hash = Hash::from_bytes([1; 32]);
        let layer2_hash = Hash::from_bytes([2; 32]);
        
        fs::write(old_dir.join(format!("{}.layer", layer1_hash)), "layer 1 data")?;
        fs::write(old_dir.join(format!("{}.layer", layer2_hash)), "layer 2 data")?;

        // Migrate to archive
        let archive = DigArchive::migrate_from_directory(archive_path, &old_dir)?;
        
        // Verify migration
        assert_eq!(archive.layer_count(), 2);
        assert!(archive.has_layer(&layer1_hash));
        assert!(archive.has_layer(&layer2_hash));

        Ok(())
    }

    #[test]
    fn test_archive_stats() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let archive_path = temp_dir.path().join("stats_test.dig");
        
        let mut archive = DigArchive::create(archive_path)?;
        
        // Add multiple layers
        for i in 0..5 {
            let hash = Hash::from_bytes([i; 32]);
            let data = vec![i; 1024]; // 1KB each
            archive.add_layer(hash, &data)?;
        }
        
        archive.flush()?;
        
        let stats = archive.stats();
        assert_eq!(stats.layer_count, 5);
        assert!(stats.total_size > 0); // Archive should have some size
        assert_eq!(stats.data_size, 5120); // Exactly 5KB of layer data
        assert!(stats.compression_ratio <= 1.0);
        
        Ok(())
    }

    #[test]
    fn test_multiple_layer_additions_after_creation() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let archive_path = temp_dir.path().join("multi_layer.dig");
        
        let mut archive = DigArchive::create(archive_path.clone())?;
        
        // Add Layer 0 (metadata) first
        let layer_zero_hash = Hash::zero();
        let metadata = serde_json::json!({
            "store_id": "test_store",
            "root_history": []
        });
        let metadata_bytes = serde_json::to_vec_pretty(&metadata)?;
        archive.add_layer(layer_zero_hash, &metadata_bytes)?;
        
        // Add multiple regular layers one by one
        for i in 1..=5 {
            let layer_hash = Hash::from_bytes([i; 32]);
            let layer_data = format!("Layer {} data content", i);
            archive.add_layer(layer_hash, layer_data.as_bytes())?;
            
            // Verify we can read the layer back immediately
            let retrieved_data = archive.get_layer_data(&layer_hash)?;
            assert_eq!(retrieved_data, layer_data.as_bytes());
        }
        
        // Verify all layers are accessible
        assert_eq!(archive.layer_count(), 6); // Layer 0 + 5 regular layers
        
        // Verify Layer 0 is still accessible
        let layer_zero_data = archive.get_layer_data(&layer_zero_hash)?;
        let parsed_metadata: serde_json::Value = serde_json::from_slice(&layer_zero_data)?;
        assert_eq!(parsed_metadata["store_id"], "test_store");
        
        // Test reopening the archive
        drop(archive);
        let archive = DigArchive::open(archive_path)?;
        assert_eq!(archive.layer_count(), 6);
        
        // Verify all layers are still accessible after reopen
        for i in 1..=5 {
            let layer_hash = Hash::from_bytes([i; 32]);
            let retrieved_data = archive.get_layer_data(&layer_hash)?;
            let expected_data = format!("Layer {} data content", i);
            assert_eq!(retrieved_data, expected_data.as_bytes());
        }
        
        Ok(())
    }
}
