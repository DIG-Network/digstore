# Security Implementation Guide for Digstore Min

## Overview

This guide provides step-by-step implementation instructions for adding URN-based data scrambling and migrating from `.layer` to `.dig` file format with complete legacy removal.

## Critical Requirements

### 🚨 **BREAKING CHANGES REQUIRED**
1. **File Extension**: ALL `.layer` files → `.dig` files (no backward compatibility)
2. **Data Access**: ALL data access must use URN-based unscrambling (no direct access)
3. **Legacy Removal**: COMPLETE removal of all legacy functionality

### 🔒 **Security Requirements**
1. **Data Scrambling**: All data in `.dig` files must be scrambled
2. **URN Access Control**: Only correct URN can unscramble data
3. **No Information Leakage**: Scrambled data reveals nothing without URN
4. **Deterministic Operation**: Same URN always produces same result

## Implementation Steps

### Step 1: Create Security Module Structure

Create the security module with scrambling functionality:

```
src/security/
├── mod.rs              # Security module exports
├── scrambler.rs        # DataScrambler implementation
├── access_control.rs   # AccessController for URN validation
└── error.rs           # Security-specific error types
```

#### File: `src/security/mod.rs`
```rust
//! Security module for data scrambling and access control

pub mod scrambler;
pub mod access_control;
pub mod error;

pub use scrambler::{DataScrambler, ScrambleState};
pub use access_control::{AccessController, AccessPermission};
pub use error::{SecurityError, SecurityResult};
```

#### File: `src/security/error.rs`
```rust
//! Security-specific error types

use thiserror::Error;
use crate::core::types::{StoreId, Hash};
use std::path::PathBuf;

#[derive(Error, Debug)]
pub enum SecurityError {
    #[error("Invalid store ID for access: expected {expected}, got {actual}")]
    InvalidStoreId { expected: StoreId, actual: StoreId },
    
    #[error("Invalid root hash: {hash}")]
    InvalidRootHash { hash: Hash },
    
    #[error("Invalid resource path: {path}")]
    InvalidResourcePath { path: PathBuf },
    
    #[error("URN access denied: missing required component {component}")]
    MissingUrnComponent { component: String },
    
    #[error("Data scrambling failed: {reason}")]
    ScramblingFailed { reason: String },
    
    #[error("Data unscrambling failed: {reason}")]
    UnscramblingFailed { reason: String },
}

pub type SecurityResult<T> = std::result::Result<T, SecurityError>;
```

### Step 2: Implement Data Scrambling Engine

#### File: `src/security/scrambler.rs`
```rust
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

/// Internal scrambling state
pub struct ScrambleState {
    key: [u8; 32],
    position: u64,
    cipher_state: [u8; 32],
}

impl DataScrambler {
    /// Create scrambler from URN
    pub fn from_urn(urn: &Urn) -> Self {
        let key = derive_scrambling_key(
            &urn.store_id,
            urn.root_hash.as_ref(),
            urn.resource_path.as_ref(),
            urn.byte_range.as_ref()
        );
        
        Self {
            state: ScrambleState::new(key),
        }
    }
    
    /// Create scrambler from components
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
        self.state.process_data(data);
        Ok(())
    }
    
    /// Unscramble data in-place (same as scramble for XOR)
    pub fn unscramble(&mut self, data: &mut [u8]) -> SecurityResult<()> {
        self.state.process_data(data);
        Ok(())
    }
    
    /// Process data at specific offset (for byte range access)
    pub fn process_at_offset(&mut self, data: &mut [u8], offset: u64) -> SecurityResult<()> {
        self.state.set_position(offset);
        self.state.process_data(data);
        Ok(())
    }
}

/// Derive scrambling key from URN components
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
        
        // Update cipher state for next byte
        let mut hasher = Sha256::new();
        hasher.update(&self.cipher_state);
        hasher.update(&self.position.to_le_bytes());
        
        let hash = hasher.finalize();
        self.cipher_state = hash.into();
        self.position += 1;
        
        keystream_byte
    }
}
```

### Step 3: Implement Access Control

#### File: `src/security/access_control.rs`
```rust
//! URN-based access control system

use crate::core::types::{StoreId, Hash};
use crate::storage::Store;
use crate::urn::Urn;
use crate::security::error::{SecurityError, SecurityResult};
use std::path::Path;

/// Access controller for URN validation
pub struct AccessController<'a> {
    store: &'a Store,
}

/// Access permission result
#[derive(Debug, Clone, PartialEq)]
pub enum AccessPermission {
    Granted,
    Denied(String),
}

impl<'a> AccessController<'a> {
    /// Create new access controller
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }
    
    /// Validate URN for data access
    pub fn validate_access(&self, urn: &Urn) -> SecurityResult<AccessPermission> {
        // Verify store ID matches
        if urn.store_id != self.store.store_id() {
            return Ok(AccessPermission::Denied(
                format!("Store ID mismatch: expected {}, got {}", 
                       self.store.store_id().to_hex(), 
                       urn.store_id.to_hex())
            ));
        }
        
        // Verify root hash exists (if specified)
        if let Some(root_hash) = urn.root_hash {
            if !self.store.has_commit(root_hash) {
                return Ok(AccessPermission::Denied(
                    format!("Root hash not found: {}", root_hash.to_hex())
                ));
            }
        }
        
        // Verify resource path exists (if specified)
        if let Some(path) = &urn.resource_path {
            if !self.store.has_file_at_path(path, urn.root_hash) {
                return Ok(AccessPermission::Denied(
                    format!("Resource path not found: {}", path.display())
                ));
            }
        }
        
        Ok(AccessPermission::Granted)
    }
    
    /// Check if URN has required components for operation
    pub fn validate_urn_completeness(&self, urn: &Urn, operation: &str) -> SecurityResult<()> {
        match operation {
            "file_access" => {
                if urn.resource_path.is_none() {
                    return Err(SecurityError::MissingUrnComponent { 
                        component: "resource_path".to_string() 
                    });
                }
            }
            "byte_range_access" => {
                if urn.resource_path.is_none() || urn.byte_range.is_none() {
                    return Err(SecurityError::MissingUrnComponent { 
                        component: "resource_path and byte_range".to_string() 
                    });
                }
            }
            _ => {}
        }
        
        Ok(())
    }
}

/// Extension methods for Store to support access control
impl Store {
    /// Check if commit exists
    pub fn has_commit(&self, root_hash: Hash) -> bool {
        let layer_path = self.global_path().join(format!("{}.dig", root_hash.to_hex()));
        layer_path.exists()
    }
    
    /// Check if file exists at path in specific commit
    pub fn has_file_at_path(&self, path: &Path, root_hash: Option<Hash>) -> bool {
        if let Some(hash) = root_hash {
            if let Ok(layer) = self.load_layer(hash) {
                return layer.files.iter().any(|f| f.path == path);
            }
        }
        
        // Check in staging
        self.staging.contains_key(path)
    }
}
```

### Step 4: Update Layer Operations for Security

#### Changes to `src/storage/layer.rs`
1. **File Extension Migration**:
   - Change all `.layer` references to `.dig`
   - Update `write_to_file()` and `read_from_file()` methods
   - Remove any legacy `.layer` support

2. **Scrambling Integration**:
   - Add scrambling to `write_to_file()` operation
   - Add unscrambling to `read_from_file()` operation
   - Scramble all chunk data before storage
   - Scramble file metadata and structure information

#### Changes to `src/storage/store.rs`
1. **File Path Updates**:
   - Update `load_layer()` to use `.dig` extension
   - Update `commit()` to create `.dig` files
   - Update all layer file path construction

2. **URN-Based Access**:
   - Add `get_file_secure()` method requiring URN
   - Add `get_byte_range_secure()` method requiring URN
   - Remove or deprecate direct file access methods
   - Integrate access control validation

### Step 5: Update CLI Commands for Security

#### Command Updates Required
1. **get command**: Require URN for accessing scrambled data
2. **cat command**: Use URN-based data access for content display
3. **prove command**: Work with scrambled data and URN validation
4. **verify command**: Handle scrambled data verification
5. **info command**: Update to work with `.dig` files
6. **log command**: Update to work with `.dig` files

#### URN Generation
Add URN generation for newly committed content to provide users with access URNs.

### Step 6: Testing Strategy

#### Security Tests Required
1. **Scrambling Determinism**: Same URN produces same scrambling
2. **Access Control**: Invalid URN cannot access data
3. **Data Protection**: Scrambled data is unreadable without URN
4. **Performance Impact**: Measure scrambling overhead
5. **Integration Tests**: End-to-end workflows with scrambled data

#### Legacy Removal Validation
1. **No `.layer` Files**: Verify no `.layer` files are created or read
2. **No Direct Access**: Verify no unscrambled data access remains
3. **Complete Migration**: All operations use secure format
4. **Error Handling**: Proper errors for legacy access attempts

## Migration Strategy

### Phase 1: Infrastructure (Critical)
1. Create security module structure
2. Implement DataScrambler with URN-based key derivation
3. Implement AccessController for URN validation
4. Add security error types and result types

### Phase 2: Layer Format Migration (Breaking)
1. **REMOVE ALL**: Delete all `.layer` file support
2. Update all file operations to use `.dig` extension
3. Integrate scrambling into layer write operations
4. Integrate unscrambling into layer read operations

### Phase 3: Access Control Integration (Critical)
1. Add URN requirements to all data access operations
2. Remove all direct file access without URN
3. Update Store methods to require URN for data access
4. Implement access validation throughout the system

### Phase 4: CLI Security Integration (High Priority)
1. Update CLI commands to work with scrambled data
2. Add URN generation for user access
3. Update error messages for security failures
4. Remove any CLI access that bypasses URN requirements

### Phase 5: Testing & Validation (Critical)
1. Comprehensive security testing
2. Performance impact validation
3. Legacy removal verification
4. Integration test updates

## Performance Considerations

### Scrambling Performance Targets
- **Throughput**: >500 MB/s for scrambling operations
- **Memory**: Constant memory usage regardless of data size
- **CPU Overhead**: <5% additional CPU usage
- **Latency**: <1ms for key derivation operations

### Optimization Strategies
1. **Keystream Caching**: Cache generated keystream for repeated access
2. **SIMD Operations**: Use vectorized XOR for large data blocks
3. **Memory-Mapped Compatibility**: Ensure scrambling works with mmap
4. **Streaming Support**: Maintain streaming I/O compatibility

## Error Handling Strategy

### Security Error Categories
1. **Access Denied**: Invalid URN components for data access
2. **Missing Components**: Required URN components not provided
3. **Scrambling Failures**: Technical failures in scrambling operations
4. **Legacy Access**: Attempts to access legacy `.layer` files

### User-Friendly Error Messages
```rust
// Example error messages
"Access denied: Invalid store ID in URN"
"Access denied: File path not found in specified commit"
"Access denied: URN missing required resource path"
"Security error: Data scrambling failed during write operation"
"Format error: Legacy .layer files are no longer supported"
```

## Validation Checklist

### ✅ **Security Implementation Complete**
- [ ] DataScrambler implemented with URN-based key derivation
- [ ] XOR-based stream cipher with SHA-256 keystream
- [ ] Position-dependent scrambling for byte ranges
- [ ] AccessController for URN validation
- [ ] Security error types and handling

### ✅ **File Format Migration Complete**
- [ ] All `.layer` references changed to `.dig`
- [ ] All layer file operations use `.dig` extension
- [ ] Layer 0 (metadata) uses `.dig` format
- [ ] No legacy `.layer` file support remains

### ✅ **Access Control Integration Complete**
- [ ] All data access requires URN validation
- [ ] Store methods updated for URN-based access
- [ ] CLI commands updated for secure access
- [ ] No direct file access without URN remains

### ✅ **Legacy Removal Complete**
- [ ] No `.layer` file support in any code
- [ ] No unscrambled data access methods
- [ ] No direct file access bypassing URN
- [ ] All documentation updated to `.dig` format

### ✅ **Testing & Validation Complete**
- [ ] Security tests pass (scrambling, access control)
- [ ] Performance tests pass (overhead <5%)
- [ ] Integration tests updated for secure format
- [ ] No legacy functionality accessible

This implementation guide ensures a secure, complete migration to the new URN-based access control system with deterministic data scrambling.
