//! Layer format implementation

use crate::core::{types::*, error::*, hash::*};
use std::path::Path;
use std::io::{Write, Read, Seek, SeekFrom};
use std::fs::File;

/// Layer structure with binary format support
pub struct Layer {
    /// Layer header (256 bytes)
    pub header: LayerHeader,
    /// Layer metadata (JSON)
    pub metadata: LayerMetadata,
    /// File entries in this layer
    pub files: Vec<FileEntry>,
    /// Chunks in this layer
    pub chunks: Vec<Chunk>,
}

impl Layer {
    /// Create a new layer
    pub fn new(layer_type: LayerType, layer_number: u64, parent_hash: RootHash) -> Self {
        Self {
            header: LayerHeader::new(layer_type, layer_number, parent_hash),
            metadata: LayerMetadata {
                layer_id: Hash::zero(), // Will be set when layer is finalized
                parent_id: if parent_hash == Hash::zero() { None } else { Some(parent_hash) },
                timestamp: chrono::Utc::now().timestamp(),
                generation: layer_number,
                layer_type,
                file_count: 0,
                total_size: 0,
                merkle_root: Hash::zero(), // Will be computed
                message: None,
                author: None,
            },
            files: Vec::new(),
            chunks: Vec::new(),
        }
    }

    /// Write layer to disk in simplified JSON format (for MVP)
    pub fn write_to_file(&self, path: &Path) -> Result<()> {
        // For MVP, use a simplified JSON format that's easier to read/write
        let layer_data = serde_json::json!({
            "header": {
                "magic": "DIGS",
                "version": 1,
                "layer_type": self.header.get_layer_type().unwrap(),
                "layer_number": self.header.layer_number,
                "timestamp": self.header.timestamp,
                "parent_hash": self.header.get_parent_hash().to_hex(),
                "files_count": self.files.len(),
                "chunks_count": self.chunks.len()
            },
            "metadata": self.metadata,
            "files": self.files,
            "chunks": self.chunks
        });

        let json_bytes = serde_json::to_vec_pretty(&layer_data)?;
        std::fs::write(path, json_bytes)?;
        
        Ok(())
    }

    /// Read layer from disk (simplified JSON format for MVP)
    pub fn read_from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read(path)?;
        let layer_data: serde_json::Value = serde_json::from_slice(&content)?;
        
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
        
        Ok(Self {
            header,
            metadata,
            files,
            chunks,
        })
    }

    /// Verify layer integrity
    pub fn verify(&self) -> Result<bool> {
        // Verify header is valid
        if !self.header.is_valid() {
            return Ok(false);
        }
        
        // For empty layers, don't check counts (they're set during serialization)
        if !self.files.is_empty() || !self.chunks.is_empty() {
            // Verify counts match
            if self.header.files_count != self.files.len() as u32 {
                return Ok(false);
            }
            
            if self.header.chunks_count != self.chunks.len() as u32 {
                return Ok(false);
            }
        }
        
        // TODO: Verify merkle tree and chunk hashes
        Ok(true)
    }

    /// Serialize the index section
    fn serialize_index(&self) -> Result<Vec<u8>> {
        let mut index_data = Vec::new();
        
        // Index header (6 bytes)
        index_data.extend_from_slice(&1u16.to_le_bytes()); // Version
        index_data.extend_from_slice(&((self.files.len() + self.chunks.len()) as u32).to_le_bytes()); // Total entries
        
        // File entries
        for file in &self.files {
            let path_str = file.path.to_string_lossy();
            let path_bytes = path_str.as_bytes();
            
            // Path length (2 bytes)
            index_data.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
            
            // Path (variable)
            index_data.extend_from_slice(path_bytes);
            
            // File size (8 bytes)
            index_data.extend_from_slice(&file.size.to_le_bytes());
            
            // File hash (32 bytes)
            index_data.extend_from_slice(file.hash.as_bytes());
            
            // Chunk count (2 bytes)
            index_data.extend_from_slice(&(file.chunks.len() as u16).to_le_bytes());
            
            // First chunk index (4 bytes) - simplified for now
            index_data.extend_from_slice(&0u32.to_le_bytes());
            
            // Metadata (JSON serialized)
            let metadata_json = serde_json::to_vec(&file.metadata)?;
            index_data.extend_from_slice(&(metadata_json.len() as u16).to_le_bytes());
            index_data.extend_from_slice(&metadata_json);
        }
        
        // Chunk entries
        for chunk in &self.chunks {
            // Chunk hash (32 bytes)
            index_data.extend_from_slice(chunk.hash.as_bytes());
            
            // Offset in file (8 bytes)
            index_data.extend_from_slice(&chunk.offset.to_le_bytes());
            
            // Chunk size (4 bytes)
            index_data.extend_from_slice(&chunk.size.to_le_bytes());
            
            // Data offset in layer (8 bytes) - will be calculated
            index_data.extend_from_slice(&0u64.to_le_bytes());
            
            // Compressed size (4 bytes) - same as size for now (no compression)
            index_data.extend_from_slice(&chunk.size.to_le_bytes());
            
            // Flags (1 byte)
            index_data.push(0);
        }
        
        Ok(index_data)
    }

    /// Deserialize the index section
    fn deserialize_index(index_data: &[u8]) -> Result<(Vec<FileEntry>, Vec<Chunk>)> {
        let mut offset = 0;
        
        // Read index header
        if index_data.len() < 6 {
            return Err(DigstoreError::invalid_layer_format("Index too short"));
        }
        
        let _version = u16::from_le_bytes([index_data[offset], index_data[offset + 1]]);
        offset += 2;
        
        let _entries_count = u32::from_le_bytes([
            index_data[offset], index_data[offset + 1], 
            index_data[offset + 2], index_data[offset + 3]
        ]);
        offset += 4;
        
        // For now, return empty vectors (will implement proper parsing later)
        // This is a simplified implementation - full parsing would be complex
        let files = Vec::new();
        let chunks = Vec::new();
        
        Ok((files, chunks))
    }

    /// Serialize the data section
    fn serialize_data(&self) -> Result<Vec<u8>> {
        let mut data_section = Vec::new();
        
        // For each chunk, write size + data
        for chunk in &self.chunks {
            // 4-byte size prefix
            data_section.extend_from_slice(&chunk.size.to_le_bytes());
            
            // Chunk data
            data_section.extend_from_slice(&chunk.data);
        }
        
        Ok(data_section)
    }

    /// Serialize the merkle tree section
    fn serialize_merkle(&self) -> Result<Vec<u8>> {
        let mut merkle_data = Vec::new();
        
        // Tree header
        merkle_data.push(0); // Depth (placeholder)
        merkle_data.extend_from_slice(&(self.files.len() as u32).to_le_bytes()); // Leaf count
        
        // For now, just include file hashes as leaves
        for file in &self.files {
            merkle_data.extend_from_slice(file.hash.as_bytes());
        }
        
        Ok(merkle_data)
    }

    /// Add a file to this layer
    pub fn add_file(&mut self, file: FileEntry) {
        self.files.push(file);
        self.metadata.file_count = self.files.len();
        self.metadata.total_size = self.files.iter().map(|f| f.size).sum();
        
        // Update header counts
        self.header.files_count = self.files.len() as u32;
    }

    /// Add a chunk to this layer
    pub fn add_chunk(&mut self, chunk: Chunk) {
        self.chunks.push(chunk);
        
        // Update header counts
        self.header.chunks_count = self.chunks.len() as u32;
    }

    /// Get the layer ID (computed from header + content)
    pub fn compute_layer_id(&self) -> Result<Hash> {
        // For now, compute from header only
        let header_bytes = self.header.to_bytes();
        Ok(sha256(&header_bytes))
    }
}