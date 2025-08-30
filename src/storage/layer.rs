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

    /// Write layer to disk in binary format
    pub fn write_to_file(&self, path: &Path) -> Result<()> {
        let mut file = File::create(path)?;
        
        // Calculate section offsets
        let header_size = LayerHeader::SIZE;
        let index_offset = header_size as u64;
        
        // Serialize index section
        let index_data = self.serialize_index()?;
        let index_size = index_data.len() as u64;
        
        // Data section comes after index
        let data_offset = index_offset + index_size;
        
        // Serialize data section
        let data_section = self.serialize_data()?;
        let data_size = data_section.len() as u64;
        
        // Merkle section comes after data
        let merkle_offset = data_offset + data_size;
        
        // Serialize merkle section
        let merkle_data = self.serialize_merkle()?;
        let merkle_size = merkle_data.len() as u64;
        
        // Update header with calculated offsets
        let mut header = self.header.clone();
        header.files_count = self.files.len() as u32;
        header.chunks_count = self.chunks.len() as u32;
        header.index_offset = index_offset;
        header.index_size = index_size;
        header.data_offset = data_offset;
        header.data_size = data_size;
        header.merkle_offset = merkle_offset;
        header.merkle_size = merkle_size;
        
        // Write header (256 bytes)
        let header_bytes = header.to_bytes();
        file.write_all(&header_bytes)?;
        
        // Write index section
        file.write_all(&index_data)?;
        
        // Write data section
        file.write_all(&data_section)?;
        
        // Write merkle section
        file.write_all(&merkle_data)?;
        
        // Calculate and write footer (layer hash)
        // We need to read the file content to calculate the hash
        file.flush()?;
        
        // Read the entire file content for hashing
        let layer_content = std::fs::read(path)?;
        let layer_hash = sha256(&layer_content);
        
        // Append the hash as footer
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)?;
        file.write_all(layer_hash.as_bytes())?;
        
        Ok(())
    }

    /// Read layer from disk
    pub fn read_from_file(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        
        // Read and parse header
        let mut header_bytes = vec![0u8; LayerHeader::SIZE];
        file.read_exact(&mut header_bytes)?;
        
        let header = LayerHeader::from_bytes(&header_bytes)
            .map_err(|e| DigstoreError::invalid_layer_format(e))?;
        
        if !header.is_valid() {
            return Err(DigstoreError::invalid_layer_format("Invalid magic or version"));
        }
        
        // Read index section
        file.seek(SeekFrom::Start(header.index_offset))?;
        let mut index_data = vec![0u8; header.index_size as usize];
        file.read_exact(&mut index_data)?;
        
        let (files, chunks) = Self::deserialize_index(&index_data)?;
        
        // Create metadata from header
        let layer_type = header.get_layer_type()
            .ok_or_else(|| DigstoreError::invalid_layer_format("Invalid layer type"))?;
        
        let metadata = LayerMetadata {
            layer_id: Hash::zero(), // Will be computed
            parent_id: if header.get_parent_hash() == Hash::zero() { 
                None 
            } else { 
                Some(header.get_parent_hash()) 
            },
            timestamp: header.timestamp as i64,
            generation: header.layer_number,
            layer_type,
            file_count: files.len(),
            total_size: files.iter().map(|f| f.size).sum(),
            merkle_root: Hash::zero(), // Will be computed from merkle section
            message: None,
            author: None,
        };
        
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
        
        let entries_count = u32::from_le_bytes([
            index_data[offset], index_data[offset + 1], 
            index_data[offset + 2], index_data[offset + 3]
        ]);
        offset += 4;
        
        // For now, return empty vectors (will implement proper parsing later)
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