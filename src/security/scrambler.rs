//! Data scrambling engine with URN-based key derivation

use crate::core::types::{StoreId, Hash};
use crate::urn::{Urn, ByteRange};
use crate::security::error::{SecurityError, SecurityResult};
use sha2::{Sha256, Digest};
use std::path::Path;

/// Data scrambler with URN-based key derivation
pub struct DataScrambler {
    state: ScrambleState,
}

/// Internal scrambling state for stream cipher
pub struct ScrambleState {
    /// 256-bit key derived from URN components
    key: [u8; 32],
    /// Current position in the data stream
    position: u64,
    /// Internal cipher state for keystream generation
    cipher_state: [u8; 32],
}

impl DataScrambler {
    /// Create scrambler from URN
    pub fn from_urn(urn: &Urn) -> Self {
        let key = derive_scrambling_key(
            &urn.store_id,
            urn.root_hash.as_ref(),
            urn.resource_path.as_ref().map(|p| p.as_path()),
            urn.byte_range.as_ref()
        );
        
        Self {
            state: ScrambleState::new(key),
        }
    }
    
    /// Create scrambler from individual components
    pub fn from_components(
        store_id: &StoreId,
        root_hash: Option<&Hash>,
        resource_path: Option<&Path>,
        byte_range: Option<&ByteRange>
    ) -> Self {
        let key = derive_scrambling_key(store_id, root_hash, resource_path, byte_range);
        Self {
            state: ScrambleState::new(key),
        }
    }
    
    /// Scramble data in-place
    pub fn scramble(&mut self, data: &mut [u8]) -> SecurityResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        
        self.state.process_data(data);
        Ok(())
    }
    
    /// Unscramble data in-place (same as scramble for XOR cipher)
    pub fn unscramble(&mut self, data: &mut [u8]) -> SecurityResult<()> {
        // XOR cipher is symmetric - scrambling and unscrambling are the same operation
        self.scramble(data)
    }
    
    /// Process data at specific offset (for byte range access)
    pub fn process_at_offset(&mut self, data: &mut [u8], offset: u64) -> SecurityResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        
        // Set position for byte range access
        self.state.set_position(offset);
        self.state.process_data(data);
        Ok(())
    }
    
    /// Get the scrambling key (for debugging/testing)
    pub fn get_key(&self) -> &[u8; 32] {
        &self.state.key
    }
    
    /// Reset scrambler to initial state
    pub fn reset(&mut self) {
        self.state.position = 0;
        self.state.cipher_state = self.state.key;
    }
}

impl ScrambleState {
    /// Create new scrambling state with key
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key,
            position: 0,
            cipher_state: key, // Initialize cipher state with key
        }
    }
    
    /// Set position for byte range access
    pub fn set_position(&mut self, position: u64) {
        self.position = position;
        
        // Reset cipher state based on position
        let mut hasher = Sha256::new();
        hasher.update(&self.key);
        hasher.update(&position.to_le_bytes());
        self.cipher_state = hasher.finalize().into();
    }
    
    /// Process data with scrambling/unscrambling
    pub fn process_data(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            *byte ^= self.next_keystream_byte();
        }
    }
    
    /// Generate next keystream byte
    fn next_keystream_byte(&mut self) -> u8 {
        let keystream_byte = self.cipher_state[0];
        
        // Update cipher state for next byte using position-dependent hash
        let mut hasher = Sha256::new();
        hasher.update(&self.cipher_state);
        hasher.update(&self.position.to_le_bytes());
        
        let hash = hasher.finalize();
        self.cipher_state = hash.into();
        self.position += 1;
        
        keystream_byte
    }
    
    /// Get current position
    pub fn position(&self) -> u64 {
        self.position
    }
}

/// Derive scrambling key from URN components using SHA-256
fn derive_scrambling_key(
    store_id: &StoreId,
    root_hash: Option<&Hash>,
    resource_path: Option<&Path>,
    byte_range: Option<&ByteRange>
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    
    // Always include store ID (primary security component)
    hasher.update(store_id.as_bytes());
    
    // Include root hash (or zero hash if not specified)
    hasher.update(root_hash.unwrap_or(&Hash::zero()).as_bytes());
    
    // Include resource path (or empty string if not specified)
    if let Some(path) = resource_path {
        hasher.update(path.to_string_lossy().as_bytes());
    }
    
    // Include byte range (or empty string if not specified)
    if let Some(range) = byte_range {
        hasher.update(range.to_string().as_bytes());
    }
    
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Hash;
    use std::path::PathBuf;

    #[test]
    fn test_key_derivation_deterministic() {
        let store_id = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let root_hash = Hash::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
        let path = PathBuf::from("test/file.txt");
        
        // Same components should produce same key
        let key1 = derive_scrambling_key(&store_id, Some(&root_hash), Some(&path), None);
        let key2 = derive_scrambling_key(&store_id, Some(&root_hash), Some(&path), None);
        assert_eq!(key1, key2);
        
        // Different components should produce different keys
        let different_store = Hash::from_hex("b3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let key3 = derive_scrambling_key(&different_store, Some(&root_hash), Some(&path), None);
        assert_ne!(key1, key3);
    }
    
    #[test]
    fn test_scrambling_deterministic() {
        let store_id = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let mut scrambler1 = DataScrambler::from_components(&store_id, None, None, None);
        let mut scrambler2 = DataScrambler::from_components(&store_id, None, None, None);
        
        let mut data1 = b"Hello, World!".to_vec();
        let mut data2 = data1.clone();
        
        // Same scrambler should produce same result
        scrambler1.scramble(&mut data1).unwrap();
        scrambler2.scramble(&mut data2).unwrap();
        assert_eq!(data1, data2);
        
        // Scrambled data should be different from original
        assert_ne!(data1, b"Hello, World!");
    }
    
    #[test]
    fn test_scramble_unscramble_roundtrip() {
        let store_id = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let mut scrambler = DataScrambler::from_components(&store_id, None, None, None);
        
        let original_data = b"This is test data for scrambling verification.";
        let mut data = original_data.to_vec();
        
        // Scramble then unscramble should restore original
        scrambler.scramble(&mut data).unwrap();
        assert_ne!(data, original_data); // Should be different after scrambling
        
        scrambler.reset(); // Reset to initial state
        scrambler.unscramble(&mut data).unwrap();
        assert_eq!(data, original_data); // Should match original after unscrambling
    }
    
    #[test]
    fn test_byte_range_scrambling() {
        let store_id = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let mut scrambler = DataScrambler::from_components(&store_id, None, None, None);
        
        let mut data = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".to_vec();
        let original = data.clone();
        
        // Scramble data at offset 10
        let offset = 10;
        let mut range_data = data[offset..offset+10].to_vec();
        scrambler.process_at_offset(&mut range_data, offset as u64).unwrap();
        
        // Should be different from original range
        assert_ne!(range_data, &original[offset..offset+10]);
        
        // Unscramble should restore original
        scrambler.reset();
        scrambler.process_at_offset(&mut range_data, offset as u64).unwrap();
        assert_eq!(range_data, &original[offset..offset+10]);
    }
    
    #[test]
    fn test_different_urn_components_different_keys() {
        let store_id = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
        let root_hash1 = Hash::from_hex("1111111111111111111111111111111111111111111111111111111111111111").unwrap();
        let root_hash2 = Hash::from_hex("2222222222222222222222222222222222222222222222222222222222222222").unwrap();
        
        let scrambler1 = DataScrambler::from_components(&store_id, Some(&root_hash1), None, None);
        let scrambler2 = DataScrambler::from_components(&store_id, Some(&root_hash2), None, None);
        
        // Different root hashes should produce different keys
        assert_ne!(scrambler1.get_key(), scrambler2.get_key());
    }
}
