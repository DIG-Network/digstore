//! Secure layer operations with URN-based data scrambling

use crate::core::{types::*, error::*};
use crate::storage::layer::Layer;
use crate::security::{DataScrambler, SecurityError, SecurityResult};
use crate::urn::Urn;
use std::path::Path;

/// Secure layer wrapper that handles scrambling/unscrambling
pub struct SecureLayer {
    /// The underlying layer data
    pub layer: Layer,
}

impl SecureLayer {
    /// Create a new secure layer from a regular layer
    pub fn new(layer: Layer) -> Self {
        Self { layer }
    }
    
    /// Write layer to .dig file with URN-based scrambling
    pub fn write_to_file(&self, path: &Path, urn: &Urn) -> Result<()> {
        // First serialize the layer to JSON (existing method)
        let layer_data = serde_json::json!({
            "header": {
                "magic": "DIGS",
                "version": 1,
                "layer_type": self.layer.header.get_layer_type().unwrap(),
                "layer_number": self.layer.header.layer_number,
                "timestamp": self.layer.header.timestamp,
                "parent_hash": self.layer.header.get_parent_hash().to_hex(),
                "files_count": self.layer.files.len(),
                "chunks_count": self.layer.chunks.len()
            },
            "metadata": self.layer.metadata,
            "files": self.layer.files,
            "chunks": self.layer.chunks
        });

        let mut json_bytes = serde_json::to_vec_pretty(&layer_data)?;
        
        // Scramble the data using URN-based key derivation
        let mut scrambler = DataScrambler::from_urn(urn);
        scrambler.scramble(&mut json_bytes)
            .map_err(|e| DigstoreError::internal(format!("Failed to scramble layer data: {}", e)))?;
        
        // Write scrambled data to .dig file
        std::fs::write(path, json_bytes)?;
        
        Ok(())
    }
    
    /// Read layer from .dig file with URN-based unscrambling
    pub fn read_from_file(path: &Path, urn: &Urn) -> Result<Self> {
        // Read scrambled data from .dig file
        let mut scrambled_data = std::fs::read(path)?;
        
        // Unscramble the data using URN-based key derivation
        let mut scrambler = DataScrambler::from_urn(urn);
        scrambler.unscramble(&mut scrambled_data)
            .map_err(|e| DigstoreError::internal(format!("Failed to unscramble layer data: {}", e)))?;
        
        // Parse the unscrambled JSON data
        let layer_data: serde_json::Value = serde_json::from_slice(&scrambled_data)?;
        
        // Parse header
        let header_data = layer_data.get("header")
            .ok_or_else(|| DigstoreError::invalid_layer_format("Missing header section"))?;
            
        let layer_type_str = header_data.get("layer_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DigstoreError::invalid_layer_format("Missing layer_type"))?;
            
        let layer_type = match layer_type_str {
            "Header" => LayerType::Header,
            "Full" => LayerType::Full,
            "Delta" => LayerType::Delta,
            _ => return Err(DigstoreError::invalid_layer_format("Invalid layer_type")),
        };
        
        let layer_number = header_data.get("layer_number")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| DigstoreError::invalid_layer_format("Missing layer_number"))?;
            
        let parent_hash_str = header_data.get("parent_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DigstoreError::invalid_layer_format("Missing parent_hash"))?;
            
        let parent_hash = Hash::from_hex(parent_hash_str)
            .map_err(|_| DigstoreError::invalid_layer_format("Invalid parent_hash"))?;
        
        // Create header
        let mut header = LayerHeader::new(layer_type, layer_number, parent_hash);
        
        // Parse metadata
        let metadata: LayerMetadata = serde_json::from_value(
            layer_data.get("metadata").cloned()
                .ok_or_else(|| DigstoreError::invalid_layer_format("Missing metadata section"))?
        )?;
        
        // Parse files
        let files: Vec<FileEntry> = serde_json::from_value(
            layer_data.get("files").cloned()
                .ok_or_else(|| DigstoreError::invalid_layer_format("Missing files section"))?
        )?;
        
        // Parse chunks
        let chunks: Vec<Chunk> = serde_json::from_value(
            layer_data.get("chunks").cloned()
                .ok_or_else(|| DigstoreError::invalid_layer_format("Missing chunks section"))?
        )?;
        
        // Update header counts to match actual data
        header.files_count = files.len() as u32;
        header.chunks_count = chunks.len() as u32;
        
        let layer = Layer {
            header,
            metadata,
            files,
            chunks,
        };
        
        Ok(Self::new(layer))
    }
    
    /// Get specific file data with URN-based unscrambling
    pub fn get_file_data(&self, file_path: &Path, urn: &Urn) -> SecurityResult<Vec<u8>> {
        // Find the file in the layer
        let file_entry = self.layer.files.iter()
            .find(|f| f.path == file_path)
            .ok_or_else(|| SecurityError::access_denied(format!("File not found: {}", file_path.display())))?;
        
        // Collect chunks for this file
        let mut file_chunks = Vec::new();
        for chunk_ref in &file_entry.chunks {
            if let Some(chunk) = self.layer.chunks.iter().find(|c| c.hash == chunk_ref.hash) {
                file_chunks.push(chunk.clone());
            }
        }
        
        // Reconstruct file data from chunks
        let mut file_data = Vec::new();
        for chunk in file_chunks {
            // Each chunk was scrambled individually, so we need to unscramble it
            let mut chunk_data = chunk.data.clone();
            
            // Create URN for this specific chunk (includes chunk offset)
            let chunk_urn = Urn {
                store_id: urn.store_id,
                root_hash: urn.root_hash,
                resource_path: Some(file_path.to_path_buf()),
                byte_range: Some(crate::urn::ByteRange::new(Some(chunk.offset), Some(chunk.offset + chunk.size as u64 - 1))),
            };
            
            let mut chunk_scrambler = DataScrambler::from_urn(&chunk_urn);
            chunk_scrambler.process_at_offset(&mut chunk_data, chunk.offset)
                .map_err(|e| SecurityError::access_denied(format!("Failed to unscramble chunk: {}", e)))?;
            
            file_data.extend_from_slice(&chunk_data);
        }
        
        Ok(file_data)
    }
    
    /// Get byte range data with URN-based unscrambling
    pub fn get_byte_range(&self, file_path: &Path, range: &crate::urn::ByteRange, urn: &Urn) -> SecurityResult<Vec<u8>> {
        // Get full file data first
        let file_data = self.get_file_data(file_path, urn)?;
        
        // Extract the requested byte range
        let file_len = file_data.len() as u64;
        let start = range.start.unwrap_or(0);
        let end = range.end.map(|e| (e + 1).min(file_len)).unwrap_or(file_len);
        
        if start >= file_len {
            return Ok(Vec::new());
        }
        
        Ok(file_data[start as usize..end as usize].to_vec())
    }
    
    /// Scramble chunk data before storage
    pub fn scramble_chunk_data(&self, chunk: &mut Chunk, file_path: &Path, urn: &Urn) -> SecurityResult<()> {
        // Create chunk-specific URN for scrambling
        let chunk_urn = Urn {
            store_id: urn.store_id,
            root_hash: urn.root_hash,
            resource_path: Some(file_path.to_path_buf()),
            byte_range: Some(crate::urn::ByteRange::new(Some(chunk.offset), Some(chunk.offset + chunk.size as u64 - 1))),
        };
        
        let mut scrambler = DataScrambler::from_urn(&chunk_urn);
        scrambler.process_at_offset(&mut chunk.data, chunk.offset)
            .map_err(|e| SecurityError::scrambling_failed(format!("Chunk scrambling failed: {}", e)))?;
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_secure_layer_roundtrip() -> Result<()> {
        // Create test layer
        let mut layer = Layer::new(LayerType::Full, 1, Hash::zero());
        
        // Add test file
        let file_entry = FileEntry {
            path: std::path::PathBuf::from("test.txt"),
            hash: Hash::zero(),
            size: 100,
            chunks: vec![ChunkRef {
                hash: Hash::zero(),
                offset: 0,
                size: 100,
            }],
            metadata: FileMetadata {
                mode: 0o644,
                modified: 0,
                is_new: true,
                is_modified: false,
                is_deleted: false,
            },
        };
        
        layer.add_file(file_entry);
        
        // Create test URN
        let store_id = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let root_hash = Hash::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
        let urn = Urn {
            store_id,
            root_hash: Some(root_hash),
            resource_path: Some(std::path::PathBuf::from("test.txt")),
            byte_range: None,
        };
        
        // Test write and read with scrambling
        let secure_layer = SecureLayer::new(layer);
        let mut temp_file = NamedTempFile::new()?;
        
        // Write with scrambling
        secure_layer.write_to_file(temp_file.path(), &urn)?;
        
        // Verify file was created and contains scrambled data
        let scrambled_content = std::fs::read(temp_file.path())?;
        assert!(!scrambled_content.is_empty());
        
        // Read with unscrambling
        let restored_layer = SecureLayer::read_from_file(temp_file.path(), &urn)?;
        
        // Verify layer was restored correctly
        assert_eq!(restored_layer.layer.files.len(), 1);
        assert_eq!(restored_layer.layer.files[0].path, std::path::PathBuf::from("test.txt"));
        
        Ok(())
    }
    
    #[test]
    fn test_scrambled_data_unreadable_without_urn() -> Result<()> {
        // Create test layer with data
        let mut layer = Layer::new(LayerType::Full, 1, Hash::zero());
        
        let file_entry = FileEntry {
            path: std::path::PathBuf::from("secret.txt"),
            hash: Hash::zero(),
            size: 50,
            chunks: vec![],
            metadata: FileMetadata {
                mode: 0o644,
                modified: 0,
                is_new: true,
                is_modified: false,
                is_deleted: false,
            },
        };
        
        layer.add_file(file_entry);
        
        // Create URN for scrambling
        let store_id = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let correct_urn = Urn {
            store_id,
            root_hash: Some(Hash::zero()),
            resource_path: Some(std::path::PathBuf::from("secret.txt")),
            byte_range: None,
        };
        
        // Create wrong URN (different store_id)
        let wrong_store_id = Hash::from_hex("b3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let wrong_urn = Urn {
            store_id: wrong_store_id,
            root_hash: Some(Hash::zero()),
            resource_path: Some(std::path::PathBuf::from("secret.txt")),
            byte_range: None,
        };
        
        let secure_layer = SecureLayer::new(layer);
        let mut temp_file = NamedTempFile::new()?;
        
        // Write with correct URN
        secure_layer.write_to_file(temp_file.path(), &correct_urn)?;
        
        // Reading with correct URN should work
        let restored_correct = SecureLayer::read_from_file(temp_file.path(), &correct_urn);
        assert!(restored_correct.is_ok());
        
        // Reading with wrong URN should fail or produce garbage
        let restored_wrong = SecureLayer::read_from_file(temp_file.path(), &wrong_urn);
        // This will likely fail during JSON parsing because the unscrambled data will be garbage
        assert!(restored_wrong.is_err());
        
        Ok(())
    }
    
    #[test]
    fn test_deterministic_scrambling() -> Result<()> {
        // Create identical layers
        let layer1 = Layer::new(LayerType::Full, 1, Hash::zero());
        let layer2 = Layer::new(LayerType::Full, 1, Hash::zero());
        
        // Create identical URNs
        let store_id = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let urn = Urn {
            store_id,
            root_hash: Some(Hash::zero()),
            resource_path: None,
            byte_range: None,
        };
        
        let secure_layer1 = SecureLayer::new(layer1);
        let secure_layer2 = SecureLayer::new(layer2);
        
        let mut temp_file1 = NamedTempFile::new()?;
        let mut temp_file2 = NamedTempFile::new()?;
        
        // Write both layers with same URN
        secure_layer1.write_to_file(temp_file1.path(), &urn)?;
        secure_layer2.write_to_file(temp_file2.path(), &urn)?;
        
        // Scrambled data should be identical
        let data1 = std::fs::read(temp_file1.path())?;
        let data2 = std::fs::read(temp_file2.path())?;
        assert_eq!(data1, data2, "Deterministic scrambling should produce identical results");
        
        Ok(())
    }
}
