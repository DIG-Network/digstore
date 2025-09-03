# Encrypted Storage with Transformed URNs

## Overview

Digstore implements encrypted storage where data is encrypted using URNs and storage addresses are transformed using public keys. This provides zero-knowledge storage where the storage layer cannot decrypt data or determine which URNs are being accessed.

## Architecture

### Storage Flow
```
Original URN: urn:dig:chia:STORE_ID/path/to/file.txt
           ↓
Transform with public key
           ↓
Storage URN: urn:dig:transformed:HASH(URN + public_key)
           ↓
Encrypt data with original URN
           ↓
Store encrypted data at transformed URN
```

### Retrieval Flow
```
Original URN: urn:dig:chia:STORE_ID/path/to/file.txt
           ↓
Transform with public key
           ↓
Storage URN: urn:dig:transformed:HASH(URN + public_key)
           ↓
Retrieve encrypted data from transformed URN
           ↓
Return encrypted data (no decryption in get command)
```

### Decryption Flow
```
Encrypted data + Original URN
           ↓
Decrypt using URN as key
           ↓
Original plaintext data
```

## Implementation

### Configuration
```bash
# Set your public key (32 bytes as hex) - encrypted storage is enabled by default
digstore config crypto.public_key "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
```

### Complete Workflow
```bash
# Configure encryption (encrypted storage is enabled by default)
digstore config crypto.public_key "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"

# Add and commit files (automatically encrypted using URN keys)
digstore add sensitive-file.txt
digstore commit -m "Store encrypted data"

# Generate content keys for analysis
digstore keygen "urn:dig:chia:STORE_ID/sensitive-file.txt" --json

# Retrieve encrypted data (returns encrypted data, not plaintext)
digstore get sensitive-file.txt -o encrypted.bin

# Decrypt using original URN
digstore decrypt encrypted.bin --urn "urn:dig:chia:STORE_ID/sensitive-file.txt" -o decrypted.txt
```

## Security Properties

### Zero-Knowledge Storage
- Storage provider cannot determine which URNs are being used
- Layer hashes are transformed before storage
- Different public keys create different storage addresses

### Data Confidentiality
- All chunk data is encrypted with AES-256-GCM
- Encryption keys are derived from URNs
- Cannot decrypt without knowing the original URN

### Key Separation
- Different public keys cannot access each other's data
- Same content stored by different keys goes to different addresses
- No correlation possible between different users' data

## Implementation Status

### ✅ Fully Implemented Features
1. **Complete Cryptographic Module** - URN transformation and AES-256-GCM encryption
2. **Configuration Support** - Public key and encryption toggle via global config
3. **Encrypted Commit Storage** - Automatic encryption during commits using URN keys
4. **Decrypt Command** - Decrypt files using original URN
5. **Storage Address Transformation** - Complete EncryptedArchive implementation
6. **Keygen Command** - Generate content keys from URN + public key
7. **Zero-Knowledge Content Addresses** - Invalid addresses return deterministic random data
8. **Complete CLI Integration** - All commands support encrypted storage workflow

### ✅ Security Properties Achieved
- **URN-Based Encryption**: Encryption keys derived from `SHA256(URN)`
- **Address Transformation**: Storage addresses from `SHA256(transform(URN + public_key))`
- **Zero-Knowledge Storage**: Invalid URNs and content addresses return random data
- **Complete Isolation**: Different public keys cannot access each other's data
- **Deterministic Behavior**: Same inputs always produce same outputs

## Use Cases

### Distributed Storage
Store encrypted digstore archives on untrusted hosts while maintaining complete privacy.

### Privacy-Preserving CDNs
Content delivery networks can cache and serve content without learning which represents real data.

### Censorship Resistance
Makes it harder to selectively block access to specific content without knowledge of which URNs are valid.
