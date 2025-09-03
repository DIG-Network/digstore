# Encrypted Storage Implementation Status

## What's Been Implemented

### 1. Cryptographic Module (`src/crypto/`)
- **URN Transformation**: `transform_urn()` combines URN + public key to create deterministic transformed addresses
- **Data Encryption**: AES-256-GCM encryption using URN as key source
- **Key Derivation**: SHA256-based key derivation from URNs

### 2. Configuration Support
- Added `crypto.public_key` - 32-byte hex public key for URN transformation
- Added `crypto.encrypted_storage` - boolean to enable/disable encryption
- Integrated with global config system

### 3. Storage Layer Changes
- Modified commit process to encrypt chunk data when `crypto.encrypted_storage` is enabled
- Chunks are encrypted using URN format: `urn:dig:chia:{store_id}/chunk/{chunk_hash}`
- Encrypted data is stored in place of plaintext chunks

### 4. Decrypt Command
- New `digstore decrypt` command to decrypt encrypted content
- Takes encrypted file and URN, outputs decrypted data
- Supports auto-detection of URN from file paths

## Current Limitations

### 1. Storage Address Transformation Not Complete
The current implementation encrypts data but stores it at the original addresses. Full zero-knowledge storage requires:
- Transforming storage addresses using public key + URN
- Storing encrypted data at transformed addresses
- Retrieving from transformed addresses

### 2. Get Command Behavior
Currently, `digstore get` with URNs may return zero-knowledge random bytes instead of encrypted data due to interaction with the zero-knowledge URN feature.

## How It Works Now

```bash
# Enable encryption
digstore config crypto.public_key "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
digstore config crypto.encrypted_storage true

# Add and commit files (automatically encrypted)
digstore add file.txt
digstore commit -m "Encrypted commit"

# Retrieve encrypted data
digstore get file.txt -o encrypted.bin

# Decrypt the data
digstore decrypt encrypted.bin --urn "urn:dig:chia:STORE_ID/file.txt" -o decrypted.txt
```

## Next Steps for Full Implementation

1. **Implement Storage Address Transformation**
   - Modify layer storage to use transformed URNs as keys
   - Update retrieval logic to transform URNs before lookup

2. **Fix Get Command**
   - Distinguish between zero-knowledge random bytes and actual encrypted data
   - Ensure encrypted data is returned when available

3. **Integration Testing**
   - Comprehensive tests for encrypted storage workflow
   - Performance testing with large files

## Security Properties Achieved

✅ **Data Encryption**: All chunk data is encrypted before storage
✅ **URN-based Keys**: Encryption keys derived from URNs
✅ **Configurable**: Can be enabled/disabled per repository
⚠️ **Partial Zero-Knowledge**: Storage addresses not yet transformed

## Example Use Cases

1. **Private Cloud Storage**: Store encrypted digstore repositories on untrusted servers
2. **Distributed Storage**: Multiple storage providers cannot read or correlate data
3. **Compliance**: Meet data-at-rest encryption requirements
