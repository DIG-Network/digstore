# Encrypted Storage Design

## Overview

The encrypted storage feature provides zero-knowledge storage where:
1. Data is encrypted using the original URN as the key
2. Storage addresses are transformed using URN + public key
3. The storage layer cannot decrypt data or know the original URNs

## Current Issues

The current implementation has conflicts between:
- Zero-knowledge URN feature (returns random bytes for invalid URNs)
- Encrypted storage feature (needs to store/retrieve actual encrypted data)

## Proposed Solution

### 1. Storage Flow

When storing data:
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

### 2. Retrieval Flow

When retrieving data:
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

### 3. Decryption Flow

```
Encrypted data + Original URN
           ↓
Decrypt using URN as key
           ↓
Original plaintext data
```

## Implementation Changes Needed

1. **Modify storage layer** to use transformed URNs when encrypted storage is enabled
2. **Update get command** to check for encrypted storage mode and use transformed URNs
3. **Ensure decrypt command** works with the encrypted data format
4. **Handle both modes** - encrypted and non-encrypted storage

## Benefits

- Storage provider cannot decrypt data without original URN
- Storage provider cannot determine which URNs are being used
- Different users with different public keys store same content at different addresses
- Zero-knowledge property is maintained
