# Keygen Command - Content Key Generation

## Overview

The `digstore keygen` command generates content keys from URN + public key combinations, demonstrating the complete encrypted storage key derivation process. This command is essential for understanding and debugging the zero-knowledge encrypted storage system.

## Implementation Status

✅ **FULLY IMPLEMENTED** - Complete key generation with all derivation methods

## Command Syntax

```bash
digstore keygen <URN> [OPTIONS]

Arguments:
  <URN>                     URN to generate keys for

Options:
  -o, --output <PATH>       Output file for key information (default: stdout)
  --storage-address         Show only storage address
  --encryption-key          Show only encryption key
  --json                    Output as JSON
```

## Key Derivation Methods

### 1. Encryption Key Derivation
```
Encryption Key = SHA256(URN)
```
- **Input**: Raw URN string
- **Output**: 32-byte encryption key for AES-256-GCM
- **Purpose**: Encrypt/decrypt data using original URN

### 2. Storage Address Derivation
```
Storage Address = SHA256(transform(URN + public_key))
```
- **Input**: URN + public key
- **Transform**: Cryptographic transformation combining URN and public key
- **Output**: 32-byte storage address (as hex string)
- **Purpose**: Zero-knowledge storage addressing

### 3. URN Transformation
```
Transformed Address = transform(URN + public_key)
```
- **Input**: URN + public key
- **Output**: Raw hex string (not URN format)
- **Purpose**: Generate storage addresses that cannot be correlated with original URNs

## Example Usage

### Basic Key Generation
```bash
digstore keygen "urn:dig:chia:STORE_ID/file.txt"
```

**Output:**
```
Generated Keys:
══════════════════════════════════════════════════

Storage Address:
  Address: 5b775663c26d9cbb804851cdd52c31060abe217b976089b8348fb2b1a24e453b
  Purpose: Where encrypted data is stored
  Derivation: SHA256(transform(URN + public_key))

Encryption Key:
  Key: 4ee7345d67b9bab68cf9e10038da9974fb1f01c885ff97eb4f9b5eeddceb8002
  Purpose: Encrypt/decrypt data using AES-256-GCM
  Derivation: SHA256(URN)

URN Transformation:
  Original URN: urn:dig:chia:STORE_ID/file.txt
  Transformed:  1b8f7942e9d57cd45ec56a02179813719a6465cf95b37068bd829150e51a6351
  Purpose: Zero-knowledge storage addressing
```

### JSON Output
```bash
digstore keygen "urn:dig:chia:STORE_ID/file.txt" --json
```

**Output:**
```json
{
  "urn": "urn:dig:chia:STORE_ID/file.txt",
  "public_key": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
  "transformed_address": "1b8f7942e9d57cd45ec56a02179813719a6465cf95b37068bd829150e51a6351",
  "storage_address": "5b775663c26d9cbb804851cdd52c31060abe217b976089b8348fb2b1a24e453b",
  "encryption_key": "4ee7345d67b9bab68cf9e10038da9974fb1f01c885ff97eb4f9b5eeddceb8002",
  "key_derivation": "SHA256(urn)",
  "address_derivation": "SHA256(transform(urn + public_key))"
}
```

### Specific Key Types
```bash
# Generate only storage address
digstore keygen "urn:dig:chia:STORE_ID/file.txt" --storage-address

# Generate only encryption key
digstore keygen "urn:dig:chia:STORE_ID/file.txt" --encryption-key

# Save to file
digstore keygen "urn:dig:chia:STORE_ID/file.txt" --json -o keys.json
```

## Security Properties

### 1. Deterministic Generation
- **Same URN + public key**: Always produces identical keys
- **Different URNs**: Produce completely different keys
- **Different public keys**: Produce completely different storage addresses
- **Cryptographic Security**: 256-bit security level

### 2. Zero-Knowledge Properties
- **Storage Isolation**: Storage layer cannot determine original URNs
- **Content Protection**: Storage layer cannot decrypt without original URN
- **Address Unlinkability**: Cannot correlate storage addresses with URNs
- **Publisher Privacy**: Different publishers store at different addresses

### 3. Key Isolation
```bash
# Different files produce different keys
digstore keygen "urn:dig:chia:STORE_ID/file1.txt" --encryption-key
# Output: 4ee7345d67b9bab68cf9e10038da9974fb1f01c885ff97eb4f9b5eeddceb8002

digstore keygen "urn:dig:chia:STORE_ID/file2.txt" --encryption-key  
# Output: 7a2c8f1e9b4d6a3c5e8f0a2d4b6c8e0f2a4c6e8f0b2d4a6c8e0f2a4c6e8f0b2
```

## Integration with Encrypted Storage

### Commit Process
1. **File Addition**: Files added to staging
2. **Commit Execution**: `digstore commit` encrypts data using URN keys
3. **Key Derivation**: Encryption keys derived from `SHA256(URN)` for each chunk
4. **Storage**: Encrypted data stored at transformed addresses

### Retrieval Process
1. **URN Request**: `digstore get` with URN
2. **Address Transformation**: URN transformed to storage address
3. **Encrypted Retrieval**: Returns encrypted data (not plaintext)
4. **Decryption**: Separate `digstore decrypt` command using original URN

### Key Generation Process
1. **URN Input**: Original URN provided to keygen command
2. **Public Key Loading**: Public key loaded from global config
3. **Transformation**: URN + public key transformed to storage address
4. **Encryption Key**: Direct SHA256 of URN
5. **Output**: All keys and derivation information

## Error Handling

### Missing Configuration
```bash
digstore keygen "urn:dig:chia:STORE_ID/file.txt"
# Error: No public key configured. Set with: digstore config crypto.public_key <hex-key>
```

### Invalid URN Format
```bash
digstore keygen "invalid-urn-format"
# Error: Invalid URN format
```

### Invalid Public Key
```bash
digstore config crypto.public_key "invalid-key"
# Error: crypto.public_key must be a 64-character hex string (32 bytes)
```

## Use Cases

### 1. Security Analysis
- **Key Verification**: Verify key derivation is working correctly
- **Address Analysis**: Understand storage address transformation
- **Debugging**: Troubleshoot encrypted storage issues

### 2. Integration Development
- **API Development**: Understand key derivation for external integrations
- **Testing**: Validate encryption/decryption workflows
- **Documentation**: Generate examples and test cases

### 3. Security Auditing
- **Key Inspection**: Verify cryptographic properties
- **Determinism Testing**: Validate consistent key generation
- **Isolation Testing**: Verify key separation between URNs

## Implementation Details

### Command Structure (`src/cli/commands/keygen.rs`)
```rust
pub fn execute(
    urn: String,
    output: Option<PathBuf>,
    storage_address: bool,
    encryption_key: bool,
    json: bool,
) -> Result<()>
```

### Key Generation Process
1. **Load Configuration**: Read public key from global config
2. **URN Transformation**: Transform URN using public key
3. **Address Derivation**: SHA256 of transformed URN
4. **Encryption Key**: SHA256 of original URN
5. **Output Formatting**: Human-readable or JSON format

### Security Validation
- **Public Key Validation**: Ensure 32-byte hex key
- **URN Validation**: Basic URN format checking
- **Deterministic Output**: Same inputs produce same outputs
- **Key Isolation**: Different inputs produce different outputs

This command provides complete visibility into the encrypted storage key derivation process, enabling users to understand and verify the zero-knowledge storage properties.
