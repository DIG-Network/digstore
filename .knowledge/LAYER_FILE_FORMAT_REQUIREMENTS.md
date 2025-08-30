# Layer File Format Requirements

## Overview

Digstore Min layer files must use the `.dig` extension instead of `.layer` and implement deterministic data scrambling based on URN components for content protection.

## File Extension Change

### Current Format
- Layer files currently use `.layer` extension
- Example: `a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2.layer`

### New Format
- All layer files must use `.dig` extension
- Example: `a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2.dig`

### Migration Requirements
- ✅ **No Legacy Support**: Remove all `.layer` extension support completely
- ✅ **Clean Migration**: Update all file operations to use `.dig` extension
- ✅ **Consistent Naming**: All references to layer files must use new extension
- ✅ **Documentation Updates**: Update all documentation to reflect new extension

## Deterministic Data Scrambling

### Security Requirement
Data stored in layer files must be deterministically scrambled using URN components as the scrambling key. Only the correct URN can unscramble and retrieve the original data.

### Scrambling Key Derivation
The scrambling key must be derived from URN components in a deterministic way:

```
URN Format: urn:dig:chia:{storeID}:{rootHash}/{resourcePath}#{byteRange}

Scrambling Key = SHA-256(storeID || rootHash || resourcePath || byteRange)
```

### Key Components
1. **Store ID**: Always present in URN
2. **Root Hash**: Layer/commit hash (required for data access)
3. **Resource Path**: File path within the repository (if specified)
4. **Byte Range**: Specific byte range (if specified)

### Scrambling Algorithm
- **Algorithm**: XOR-based stream cipher with key-derived state
- **Deterministic**: Same URN always produces same scrambling/unscrambling
- **Efficient**: Minimal performance overhead during read/write operations
- **Secure**: Cryptographically sound scrambling that prevents unauthorized access

### Implementation Requirements

#### 1. Scrambling Engine
```rust
pub struct DataScrambler {
    key: [u8; 32],           // Derived from URN components
    state: ScrambleState,    // Current scrambling state
}

impl DataScrambler {
    /// Create scrambler from URN components
    pub fn from_urn_components(
        store_id: &StoreId,
        root_hash: Option<&Hash>,
        resource_path: Option<&Path>,
        byte_range: Option<&ByteRange>
    ) -> Self;
    
    /// Scramble data in-place
    pub fn scramble(&mut self, data: &mut [u8]);
    
    /// Unscramble data in-place (same as scramble for XOR)
    pub fn unscramble(&mut self, data: &mut [u8]);
}
```

#### 2. Layer File Integration
- **Write Operations**: All data written to `.dig` files must be scrambled
- **Read Operations**: All data read from `.dig` files must be unscrambled
- **Chunk Data**: Individual chunk data must be scrambled before storage
- **File Metadata**: File entries and metadata must be scrambled
- **Merkle Trees**: Merkle tree data must be scrambled in layer files

#### 3. URN-Based Access Control
- **Access Requirement**: Correct URN required to read any data from layer files
- **No URN, No Access**: Without the correct URN, data appears as random bytes
- **Byte Range Security**: Different byte ranges use different scrambling states
- **Path-Specific**: Different file paths use different scrambling keys

### Security Properties

#### 1. Content Protection
- **Unauthorized Access Prevention**: Data unreadable without correct URN
- **Path-Specific Security**: Each file path has unique scrambling
- **Range-Specific Security**: Each byte range has unique scrambling state
- **No Information Leakage**: Scrambled data reveals no information about content

#### 2. Deterministic Operation
- **Reproducible**: Same URN always produces same result
- **Consistent**: Multiple reads of same URN return identical data
- **Stateless**: Scrambling operation doesn't depend on external state
- **Portable**: Scrambled data works across different systems

#### 3. Performance Requirements
- **Minimal Overhead**: <5% performance impact for scrambling operations
- **Streaming Compatible**: Works with streaming I/O and memory-mapped files
- **Parallel Safe**: Thread-safe scrambling for concurrent operations
- **Memory Efficient**: Constant memory usage regardless of data size

### Implementation Phases

#### Phase 1: Data Scrambling Engine
1. Implement `DataScrambler` with URN-based key derivation
2. Create XOR-based stream cipher with cryptographic state
3. Add scrambling/unscrambling methods with in-place operation
4. Implement key derivation from URN components

#### Phase 2: Layer File Integration  
1. Update all layer file operations to use `.dig` extension
2. Integrate scrambling into layer write operations
3. Integrate unscrambling into layer read operations
4. Update chunk data storage with scrambling

#### Phase 3: URN Access Control
1. Modify file retrieval to require URN for unscrambling
2. Implement path-specific and range-specific scrambling
3. Add URN validation for data access operations
4. Ensure no legacy unscrambled access remains

#### Phase 4: Testing & Validation
1. Comprehensive scrambling/unscrambling tests
2. URN-based access control validation
3. Performance impact measurement
4. Security property verification

### Backward Compatibility
- ❌ **No Legacy Support**: Remove all `.layer` file support
- ❌ **No Unscrambled Access**: Remove all direct data access
- ✅ **Clean Break**: Complete migration to new secure format
- ✅ **Forward Only**: New implementation only supports secure format

### Error Handling
- **Invalid URN**: Clear error when URN doesn't match data
- **Missing Components**: Error when URN lacks required components for access
- **Scrambling Failures**: Proper error propagation for scrambling operations
- **File Format Errors**: Clear messages for `.dig` file format issues
