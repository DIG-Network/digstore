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
# Set your public key (32 bytes as hex)
digstore config crypto.public_key "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"

# Enable encrypted storage
digstore config crypto.encrypted_storage true
```

### Usage
```bash
# Add and commit files (automatically encrypted)
digstore add sensitive-file.txt
digstore commit -m "Store encrypted data"

# Retrieve encrypted data via URN
digstore get "urn:dig:chia:STORE_ID/sensitive-file.txt" -o encrypted.bin

# Decrypt the data
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

### ✅ Completed Features
1. **Cryptographic Module** - URN transformation and AES-GCM encryption
2. **Configuration Support** - Public key and encryption toggle
3. **Encrypted Commit Storage** - Automatic encryption during commits
4. **Decrypt Command** - Decrypt files using original URN
5. **Storage Address Transformation** - EncryptedArchive wrapper

### ⚠️ Current Limitations
- Zero-knowledge URN feature may interfere with encrypted storage retrieval
- Storage addresses are partially transformed (layer level only)
- Full URN transformation requires additional storage layer integration

## Use Cases

### Distributed Storage
Store encrypted digstore archives on untrusted hosts while maintaining complete privacy.

### Privacy-Preserving CDNs
Content delivery networks can cache and serve content without learning which represents real data.

### Censorship Resistance
Makes it harder to selectively block access to specific content without knowledge of which URNs are valid.
