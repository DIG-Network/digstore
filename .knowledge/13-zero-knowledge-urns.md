# Zero-Knowledge URN Retrieval

## Overview

The Digstore implements a zero-knowledge property for URN retrieval, making it impossible for storage hosts to distinguish between valid and invalid URNs. This enhances privacy and prevents enumeration attacks.

## How It Works

When retrieving content via URN using `digstore get`, the system will:

1. **Valid URNs**: Return the actual content
2. **Invalid URNs**: Return deterministic random bytes instead of an error

The random bytes are:
- **Deterministic**: The same invalid URN always returns the same random data
- **Unique**: Different invalid URNs return different random data
- **Size-aware**: Respects byte ranges specified in the URN

## Implementation Details

### Deterministic Random Generation

Random bytes are generated using SHA256-based expansion:
```rust
// Seed with the full URN string
SHA256(urn_string || counter) -> 32 bytes
// Repeat with incrementing counter until desired size is reached
```

### Default Behavior

- **Default size**: 1MB (1,048,576 bytes) for invalid URNs without byte ranges
- **With byte range**: Generates exactly the requested number of bytes
- **Silent errors**: No error messages are printed to maintain zero-knowledge property

### Examples

```bash
# Invalid store ID - returns 1MB of deterministic random data
digstore get urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/file.txt

# Invalid URN format - returns 1MB of deterministic random data
digstore get urn:dig:chia:invalid-format/file.dat

# With byte range - returns exactly 100 bytes
digstore get "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/file.txt#bytes=0-99"

# Valid store but non-existent file - also returns random data
digstore get urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/nonexistent.txt
```

## Security Benefits

### 1. Privacy Protection
Hosts cannot determine which files actually exist in a store, preventing them from learning about the content structure.

### 2. Enumeration Prevention
Attackers cannot probe for valid URNs by observing error responses, as all URNs return data.

### 3. Traffic Analysis Resistance
Network observers cannot distinguish between requests for real vs. fake content based on response patterns.

### 4. Plausible Deniability
Users can claim any URN they possess is valid, as invalid URNs behave identically to valid ones.

## Use Cases

### Distributed Storage
When storing encrypted digstore archives on untrusted hosts, the zero-knowledge property ensures hosts cannot learn:
- Which URNs are valid
- The structure of stored data
- Access patterns to real vs. fake content

### Privacy-Preserving CDNs
Content delivery networks can cache and serve both valid and invalid URNs without learning which represent real content.

### Censorship Resistance
Makes it harder to selectively block access to specific content, as blocking requires knowledge of which URNs are valid.

## Technical Considerations

### Performance
- Generating random bytes is computationally lightweight
- No additional storage required for invalid URNs
- Deterministic generation ensures consistent responses

### Compatibility
- Works with all URN formats including byte ranges
- Transparent to clients - they receive data either way
- Output redirection and piping work normally

### Limitations
- Only applies to URN-based retrieval (not local file paths)
- Random data is not cryptographically secure (uses SHA256 expansion)
- Large byte ranges may take time to generate

## Best Practices

1. **Always use URNs** when sharing references to content in untrusted environments
2. **Include byte ranges** when only partial content is needed to minimize random data generation
3. **Store URNs securely** - they are effectively access tokens when combined with zero-knowledge retrieval
4. **Monitor performance** when retrieving large amounts of data from potentially invalid URNs

## Implementation Note

This feature is implemented in the `digstore get` command and activates automatically when retrieving content via URN. No configuration or flags are required.
