# Data Scrambling Specification for Digstore Min

## Overview

This document specifies the deterministic data scrambling system that protects content in Digstore Min layer files. Only correct URNs can unscramble and access the stored data.

## Cryptographic Design

### Scrambling Algorithm
- **Base Algorithm**: XOR-based stream cipher
- **Key Size**: 256-bit (32 bytes) derived from URN components
- **State Size**: 64-bit internal state for stream generation
- **Security**: Cryptographically sound with proper key derivation

### Key Derivation Function (KDF)

The scrambling key is derived from URN components using SHA-256:

```
Input Components:
- store_id: StoreId (32 bytes)
- root_hash: Hash (32 bytes, optional)
- resource_path: String (UTF-8 bytes, optional)
- byte_range: ByteRange (start:end format, optional)

Key Derivation:
key = SHA-256(
    store_id ||
    root_hash.unwrap_or(ZERO_HASH) ||
    resource_path.unwrap_or("").as_bytes() ||
    byte_range.unwrap_or("").to_string().as_bytes()
)
```

### Stream Cipher Implementation

```rust
pub struct ScrambleState {
    key: [u8; 32],
    counter: u64,
    position: u64,
}

impl ScrambleState {
    /// Generate next 8 bytes of keystream
    fn next_keystream(&mut self) -> [u8; 8] {
        let mut hasher = Sha256::new();
        hasher.update(&self.key);
        hasher.update(&self.counter.to_le_bytes());
        hasher.update(&self.position.to_le_bytes());
        
        let hash = hasher.finalize();
        self.counter += 1;
        
        // Take first 8 bytes of hash as keystream
        let mut keystream = [0u8; 8];
        keystream.copy_from_slice(&hash[0..8]);
        keystream
    }
    
    /// Scramble/unscramble data in-place
    pub fn process_data(&mut self, data: &mut [u8]) {
        for chunk in data.chunks_mut(8) {
            let keystream = self.next_keystream();
            for (i, byte) in chunk.iter_mut().enumerate() {
                if i < keystream.len() {
                    *byte ^= keystream[i];
                }
            }
        }
    }
}
```

## URN Component Impact on Scrambling

### 1. Store ID Impact
- **Always Required**: Every URN must contain a valid store ID
- **Global Scope**: Store ID affects all data in the repository
- **Base Security**: Primary component of scrambling key

### 2. Root Hash Impact
- **Commit-Specific**: Different commits have different scrambling
- **Temporal Security**: Historical data requires correct root hash
- **Version Isolation**: Each commit's data is isolated from others

### 3. Resource Path Impact
- **File-Specific**: Each file path creates unique scrambling
- **Directory Structure**: Path hierarchy affects scrambling key
- **Access Granularity**: File-level access control through path-specific keys

### 4. Byte Range Impact
- **Range-Specific**: Different byte ranges have different scrambling states
- **Granular Access**: Sub-file access control through range-specific scrambling
- **Position-Dependent**: Scrambling state depends on byte position

## Implementation Architecture

### Core Components

#### 1. DataScrambler (`src/security/scrambler.rs`)
```rust
pub struct DataScrambler {
    state: ScrambleState,
}

impl DataScrambler {
    /// Create from URN components
    pub fn from_urn(urn: &Urn) -> Self;
    
    /// Create from individual components
    pub fn from_components(
        store_id: &StoreId,
        root_hash: Option<&Hash>,
        resource_path: Option<&Path>,
        byte_range: Option<&ByteRange>
    ) -> Self;
    
    /// Scramble data in-place
    pub fn scramble(&mut self, data: &mut [u8]);
    
    /// Unscramble data (same as scramble for XOR)
    pub fn unscramble(&mut self, data: &mut [u8]) {
        self.scramble(data);
    }
    
    /// Process data at specific offset (for byte range access)
    pub fn process_at_offset(&mut self, data: &mut [u8], offset: u64);
}
```

#### 2. Secure Layer Operations (`src/storage/secure_layer.rs`)
```rust
pub struct SecureLayer {
    layer: Layer,
    scrambler: DataScrambler,
}

impl SecureLayer {
    /// Write layer with scrambling
    pub fn write_to_file(&self, path: &Path, urn: &Urn) -> Result<()>;
    
    /// Read layer with unscrambling
    pub fn read_from_file(path: &Path, urn: &Urn) -> Result<Self>;
    
    /// Access specific file with URN
    pub fn get_file_data(&self, file_path: &Path, urn: &Urn) -> Result<Vec<u8>>;
    
    /// Access byte range with URN
    pub fn get_byte_range(&self, file_path: &Path, range: &ByteRange, urn: &Urn) -> Result<Vec<u8>>;
}
```

### Integration Points

#### 1. Store Operations
- **File Addition**: Scramble data when writing to `.dig` files
- **File Retrieval**: Unscramble data when reading from `.dig` files
- **Commit Operations**: Use commit hash as root_hash for scrambling
- **Staging Operations**: Maintain unscrambled data in memory staging

#### 2. URN Resolution
- **Access Control**: URN components must match scrambling key
- **Byte Range Access**: Adjust scrambling state for range operations
- **Path Resolution**: Use full path for scrambling key derivation
- **Error Handling**: Clear errors for invalid URN access attempts

#### 3. CLI Commands
- **get Command**: Require URN for accessing scrambled data
- **cat Command**: URN-based access for content display
- **prove Command**: Generate proofs for scrambled data access
- **verify Command**: Verify proofs against scrambled data

## Security Analysis

### Threat Model
1. **Unauthorized File System Access**: Attacker has read access to `.dig` files
2. **Partial URN Knowledge**: Attacker knows some but not all URN components
3. **Statistical Analysis**: Attacker attempts frequency analysis of scrambled data
4. **Brute Force**: Attacker attempts to guess URN components

### Security Properties

#### 1. Confidentiality
- **Data Protection**: Raw file content is unreadable without correct URN
- **Metadata Protection**: File names and structure protected by path scrambling
- **Size Hiding**: Chunk boundaries obscured by scrambling
- **Pattern Hiding**: File content patterns not visible in scrambled form

#### 2. Access Control
- **URN-Based Access**: Only correct URN can access specific data
- **Granular Control**: File-level and byte-range level access control
- **Temporal Control**: Commit-specific access through root hash
- **Path-Based Control**: Directory-level access through path components

#### 3. Integrity
- **Tamper Detection**: Modified scrambled data fails unscrambling validation
- **Authenticity**: URN components verified through successful unscrambling
- **Consistency**: Deterministic scrambling ensures consistent results
- **Error Detection**: Invalid URNs produce detectably invalid data

### Attack Resistance

#### 1. Known-Plaintext Attacks
- **Mitigation**: URN-specific scrambling prevents pattern analysis
- **Key Diversity**: Each URN component creates unique scrambling key
- **Position Dependency**: Scrambling state depends on data position
- **No Reuse**: Each file/range has unique scrambling sequence

#### 2. Statistical Analysis
- **Uniform Distribution**: XOR with SHA-256 derived keystream creates uniform output
- **No Patterns**: Scrambled data shows no statistical correlation with plaintext
- **Entropy Preservation**: High-entropy data remains high-entropy when scrambled
- **Frequency Hiding**: Character frequency analysis yields no information

#### 3. Partial URN Attacks
- **Component Dependency**: All URN components required for correct key
- **Avalanche Effect**: Small URN changes create completely different keys
- **No Partial Success**: Incorrect URN components yield completely wrong data
- **Brute Force Resistance**: 256-bit effective key space prevents brute force

## Performance Requirements

### Scrambling Performance
- **Throughput**: >500 MB/s scrambling/unscrambling speed
- **Memory Usage**: Constant memory regardless of data size
- **CPU Overhead**: <5% additional CPU usage for scrambling operations
- **Latency**: <1ms additional latency for key derivation

### Streaming Compatibility
- **Stream Processing**: Work with streaming I/O operations
- **Memory Mapping**: Compatible with memory-mapped file access
- **Partial Reads**: Support efficient byte range access
- **Concurrent Access**: Thread-safe scrambling operations

## Implementation Considerations

### 1. Key Management
- **No Key Storage**: Keys derived on-demand from URN components
- **Stateless Operation**: No persistent key state required
- **Memory Security**: Clear sensitive data from memory after use
- **Key Derivation Caching**: Cache derived keys for performance (optional)

### 2. Error Handling
- **Invalid URN Detection**: Detect and report invalid URN components
- **Scrambling Failures**: Handle scrambling operation errors gracefully
- **Data Corruption**: Detect and report corrupted scrambled data
- **Access Denied**: Clear error messages for unauthorized access attempts

### 3. Testing Requirements
- **Determinism Tests**: Verify same URN produces same scrambling
- **Security Tests**: Verify scrambled data is unreadable without URN
- **Performance Tests**: Measure scrambling overhead
- **Integration Tests**: Test URN-based access control end-to-end

### 4. Migration Strategy
- **Clean Break**: No support for legacy `.layer` files
- **Format Validation**: Reject any `.layer` files with clear error
- **Extension Consistency**: All file operations use `.dig` extension
- **Documentation Updates**: Update all references to new format

## Compliance and Validation

### Format Compliance
- ✅ All layer files use `.dig` extension
- ✅ All data in layer files is scrambled
- ✅ No unscrambled data accessible without URN
- ✅ No legacy `.layer` file support

### Security Validation
- ✅ Scrambled data is cryptographically secure
- ✅ URN components required for data access
- ✅ No information leakage from scrambled data
- ✅ Deterministic scrambling/unscrambling operations

### Performance Validation
- ✅ Scrambling overhead <5% of total operation time
- ✅ Streaming operations maintain throughput
- ✅ Memory usage remains constant
- ✅ Key derivation is efficient (<1ms)

This specification ensures that Digstore Min provides robust content protection while maintaining excellent performance and usability.
