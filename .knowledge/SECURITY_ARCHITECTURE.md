# Security Architecture for Digstore Min

## Overview

Digstore Min implements a comprehensive security architecture based on URN-controlled data scrambling. This document outlines the security model, implementation requirements, and architectural decisions.

## Security Model

### Core Principle: URN-Based Access Control
All data stored in Digstore Min is protected by deterministic scrambling that can only be reversed with the correct URN. This creates a content-addressable storage system where:

1. **Data is Protected**: Raw content cannot be read without the correct URN
2. **Access is Granular**: Different URN components provide different levels of access
3. **Security is Transparent**: Users interact with URNs naturally, security is automatic
4. **Performance is Maintained**: Security overhead is minimal (<5%)

### Threat Model

#### Assets to Protect
1. **File Content**: Raw data stored in repository files
2. **File Metadata**: File names, sizes, timestamps, and structure
3. **Repository Structure**: Directory hierarchy and organization
4. **Historical Data**: Previous versions and commit history

#### Threat Actors
1. **Unauthorized File System Access**: Attacker with read access to `.dig` files
2. **Partial Information Disclosure**: Attacker with incomplete URN knowledge
3. **Statistical Analysis**: Sophisticated attacker performing cryptanalysis
4. **Insider Threats**: Users with legitimate access attempting unauthorized data access

#### Attack Vectors
1. **Direct File Access**: Reading `.dig` files from file system
2. **URN Guessing**: Attempting to construct valid URNs
3. **Pattern Analysis**: Statistical analysis of scrambled data
4. **Social Engineering**: Obtaining partial URN information

## Architectural Components

### 1. Data Scrambling Engine

#### ScrambleState
```rust
pub struct ScrambleState {
    /// 256-bit key derived from URN components
    key: [u8; 32],
    /// Current position in the keystream
    position: u64,
    /// Internal cipher state
    state: [u8; 32],
}
```

#### Key Derivation
```rust
fn derive_scrambling_key(
    store_id: &StoreId,
    root_hash: Option<&Hash>,
    resource_path: Option<&Path>,
    byte_range: Option<&ByteRange>
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    
    // Always include store ID
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
```

#### Stream Cipher
```rust
impl ScrambleState {
    /// Initialize from key
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key,
            position: 0,
            state: key, // Initialize state with key
        }
    }
    
    /// Generate keystream bytes at current position
    fn generate_keystream(&mut self, length: usize) -> Vec<u8> {
        let mut keystream = Vec::with_capacity(length);
        
        for _ in 0..length {
            // Generate next byte using position-dependent hash
            let mut hasher = Sha256::new();
            hasher.update(&self.state);
            hasher.update(&self.position.to_le_bytes());
            
            let hash = hasher.finalize();
            keystream.push(hash[0]);
            
            // Update state for next byte
            self.state[0] ^= hash[31];
            self.position += 1;
        }
        
        keystream
    }
    
    /// Scramble data in-place
    pub fn scramble_data(&mut self, data: &mut [u8]) {
        let keystream = self.generate_keystream(data.len());
        for (i, byte) in data.iter_mut().enumerate() {
            *byte ^= keystream[i];
        }
    }
}
```

### 2. Secure Layer Operations

#### Secure Write Operations
```rust
impl Layer {
    /// Write layer to .dig file with scrambling
    pub fn write_to_secure_file(&self, path: &Path, urn: &Urn) -> Result<()> {
        // Derive scrambling key from URN
        let scrambler = DataScrambler::from_urn(urn);
        
        // Serialize layer data
        let mut layer_data = self.serialize_to_bytes()?;
        
        // Scramble the data
        scrambler.scramble(&mut layer_data);
        
        // Write scrambled data to .dig file
        std::fs::write(path, layer_data)?;
        Ok(())
    }
    
    /// Read layer from .dig file with unscrambling
    pub fn read_from_secure_file(path: &Path, urn: &Urn) -> Result<Self> {
        // Read scrambled data
        let mut scrambled_data = std::fs::read(path)?;
        
        // Derive scrambling key from URN
        let scrambler = DataScrambler::from_urn(urn);
        
        // Unscramble the data
        scrambler.unscramble(&mut scrambled_data);
        
        // Deserialize layer
        Self::deserialize_from_bytes(&scrambled_data)
    }
}
```

#### Secure Chunk Operations
```rust
impl Chunk {
    /// Store chunk data in scrambled form
    pub fn store_scrambled(&mut self, urn: &Urn) {
        let scrambler = DataScrambler::from_urn_with_chunk_offset(urn, self.offset);
        scrambler.scramble(&mut self.data);
    }
    
    /// Retrieve chunk data in unscrambled form
    pub fn retrieve_unscrambled(&mut self, urn: &Urn) {
        let scrambler = DataScrambler::from_urn_with_chunk_offset(urn, self.offset);
        scrambler.unscramble(&mut self.data);
    }
}
```

### 3. URN-Based Access Control

#### Access Validation
```rust
pub struct AccessController {
    store: Store,
}

impl AccessController {
    /// Validate URN for data access
    pub fn validate_access(&self, urn: &Urn) -> Result<AccessPermission> {
        // Verify store ID matches
        if urn.store_id != self.store.store_id() {
            return Err(SecurityError::InvalidStoreId);
        }
        
        // Verify root hash exists (if specified)
        if let Some(root_hash) = urn.root_hash {
            if !self.store.has_commit(root_hash) {
                return Err(SecurityError::InvalidRootHash);
            }
        }
        
        // Verify resource path exists (if specified)
        if let Some(path) = &urn.resource_path {
            if !self.store.has_file_at_path(path, urn.root_hash) {
                return Err(SecurityError::InvalidResourcePath);
            }
        }
        
        Ok(AccessPermission::Granted)
    }
}
```

#### Secure File Retrieval
```rust
impl Store {
    /// Get file with URN-based access control
    pub fn get_file_secure(&self, urn: &Urn) -> Result<Vec<u8>> {
        // Validate URN for access
        self.access_controller.validate_access(urn)?;
        
        // Load layer with URN-based unscrambling
        let layer = SecureLayer::read_from_file(&self.get_layer_path(urn.root_hash?), urn)?;
        
        // Extract file data with URN-specific unscrambling
        layer.get_file_data(urn.resource_path.as_ref().unwrap(), urn)
    }
    
    /// Get byte range with URN-based access control
    pub fn get_byte_range_secure(&self, urn: &Urn) -> Result<Vec<u8>> {
        // Validate URN for access
        self.access_controller.validate_access(urn)?;
        
        // Load layer with URN-based unscrambling
        let layer = SecureLayer::read_from_file(&self.get_layer_path(urn.root_hash?), urn)?;
        
        // Extract byte range with range-specific unscrambling
        layer.get_byte_range(
            urn.resource_path.as_ref().unwrap(),
            urn.byte_range.as_ref().unwrap(),
            urn
        )
    }
}
```

## Security Properties

### 1. Confidentiality Properties
- **Content Hiding**: File content is cryptographically protected
- **Structure Hiding**: Directory structure and file names are scrambled
- **Size Hiding**: File sizes are obscured through scrambling
- **Metadata Hiding**: File timestamps and permissions are scrambled

### 2. Access Control Properties
- **URN Requirement**: Valid URN required for any data access
- **Component Validation**: All URN components must be correct
- **Granular Access**: File-level and byte-range access control
- **Temporal Access**: Commit-specific access through root hash

### 3. Integrity Properties
- **Tamper Detection**: Modifications to scrambled data are detectable
- **Authenticity Verification**: Correct URN proves data authenticity
- **Consistency Guarantee**: Deterministic operations ensure consistency
- **Error Propagation**: Invalid access attempts fail cleanly

## Implementation Phases

### Phase 1: Security Infrastructure (Priority: Critical)
1. **DataScrambler Implementation**
   - Key derivation from URN components
   - XOR-based stream cipher with SHA-256 keystream
   - Position-dependent scrambling for byte ranges
   - Performance optimization for large data

2. **Security Error Types**
   - SecurityError enum for access control failures
   - Clear error messages for invalid URN access
   - Integration with existing DigstoreError system
   - Proper error propagation through all layers

### Phase 2: Secure Layer Format (Priority: Critical)
1. **File Extension Migration**
   - Change all layer files from `.layer` to `.dig`
   - Update all file path construction logic
   - Remove all legacy `.layer` support
   - Update documentation and error messages

2. **Scrambled Data Storage**
   - Integrate scrambling into layer write operations
   - Integrate unscrambling into layer read operations
   - Ensure all chunk data is scrambled before storage
   - Maintain metadata scrambling for complete protection

### Phase 3: URN Access Control (Priority: High)
1. **Access Validation System**
   - Implement AccessController for URN validation
   - Add URN requirement to all data access operations
   - Remove all direct file access without URN
   - Ensure complete access control coverage

2. **CLI Integration**
   - Update all CLI commands to require URNs for data access
   - Maintain backward compatibility for local file access
   - Add URN generation for newly created content
   - Provide clear guidance for URN usage

### Phase 4: Testing & Validation (Priority: High)
1. **Security Testing**
   - Verify scrambled data is unreadable without URN
   - Test URN component validation
   - Validate access control enforcement
   - Performance impact measurement

2. **Integration Testing**
   - End-to-end workflows with scrambled data
   - URN-based file operations
   - Byte range access with scrambling
   - Error handling and edge cases

## Backward Compatibility

### No Legacy Support Policy
- ❌ **No `.layer` file support**: Complete removal of legacy format
- ❌ **No unscrambled access**: All data must be accessed via URN
- ❌ **No migration tools**: Clean break from previous format
- ✅ **Forward-only compatibility**: New secure format only

### Migration Requirements
- **Complete Reimplementation**: All file operations updated for security
- **Documentation Updates**: All references to new secure format
- **Test Updates**: All tests updated for scrambled data access
- **CLI Updates**: All commands updated for URN-based access

This security architecture provides robust protection for stored data while maintaining the usability and performance characteristics of Digstore Min.
