# URN Specification for Digstore Min

## Overview

Digstore Min uses Uniform Resource Names (URNs) to provide permanent, location-independent identifiers for resources. This specification extends the standard URN format with byte range support for partial content retrieval.

## URN Format

### Full URN Format
```
urn:dig:chia:{storeID}[:{rootHash}][/{resourcePath}][#{byteRange}]
```

### Simplified Format (in project directory with .digstore)
```
/{resourcePath}[#{byteRange}]
```

When in a directory containing a `.digstore` file, the store ID is automatically read from the file, allowing simplified path-only syntax.

### Components

1. **Scheme**: `urn` (required in full format, case-insensitive)
2. **Namespace**: `dig:chia` (required in full format, case-insensitive)
3. **Store ID**: 32-byte hex-encoded identifier (required in full format, automatic in simplified)
4. **Root Hash**: 32-byte hex-encoded SHA-256 hash (optional, case-sensitive)
5. **Resource Path**: File path within the store (required for files, case-sensitive)
6. **Byte Range**: Byte range specifier (optional)

## Component Details

### Store ID
- **Format**: 64 character hex string (32 bytes)
- **Example**: `a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2b1c4d7e0a3b6c9d2`
- **Purpose**: Identifies the repository

### Root Hash
- **Format**: 64 character hex string (32 bytes)
- **Example**: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`
- **Purpose**: Specifies exact repository state/generation
- **Default**: Latest root hash if omitted

### Resource Path
- **Format**: POSIX-style path relative to repository root
- **Example**: `src/main.rs`, `docs/readme.md`
- **Encoding**: UTF-8, URL-encoded for special characters
- **Default**: Repository root if omitted

### Byte Range
- **Format**: `#bytes={start}-{end}` or `#bytes={start}-`
- **Examples**: 
  - `#bytes=0-1023` (first 1024 bytes)
  - `#bytes=1024-` (from byte 1024 to end)
  - `#bytes=-1024` (last 1024 bytes)
- **Purpose**: Retrieve partial content
- **Default**: Entire resource if omitted

## URN Examples

### Simplified Format (with .digstore)

When in a project directory with a `.digstore` file:

1. **File in current project**:
   ```
   /src/main.rs
   /docs/README.md
   ```

2. **File with byte range**:
   ```
   /large_file.bin#bytes=0-1023
   /video.mp4#bytes=1048576-
   ```

3. **With explicit root hash** (using --at flag):
   ```
   digstore get /src/main.rs --at e3b0c44298fc
   ```

### Full URN Format

Required when not in a project directory or accessing other stores:

1. **Latest state of entire store**:
   ```
   urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2b1c4d7e0a3b6c9d2
   ```

2. **Specific generation of store**:
   ```
   urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
   ```

### File-Level URNs

3. **Latest version of a file**:
   ```
   urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/src/main.rs
   ```

4. **Specific version of a file**:
   ```
   urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/src/main.rs
   ```

### Byte Range URNs

5. **First 1KB of a file**:
   ```
   urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/large_file.bin#bytes=0-1023
   ```

6. **From offset to end of file**:
   ```
   urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/video.mp4#bytes=1048576-
   ```

7. **Last 4KB of a file at specific version**:
   ```
   urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/log.txt#bytes=-4096
   ```

## URN Resolution

### Resolution Process

1. **Parse URN** into components
2. **Locate store** by store ID
3. **Determine root hash**:
   - Use specified root hash, or
   - Lookup latest root hash from Layer 0
4. **Find layer(s)** containing the resource
5. **Extract resource** from layer(s)
6. **Apply byte range** if specified
7. **Return content** with metadata

### Cross-Layer Resolution

For files spanning multiple delta layers:

1. Start with the layer at specified root hash
2. Collect chunks from current layer
3. For missing chunks, traverse to parent layers
4. Reconstruct file from collected chunks
5. Apply byte range to reconstructed content

## Byte Range Specification

### Syntax

Following HTTP Range Request specification (RFC 7233):

- `bytes={start}-{end}`: Bytes from start to end (inclusive)
- `bytes={start}-`: From start to end of resource
- `bytes=-{suffix}`: Last suffix bytes

### Examples

- `#bytes=0-499`: First 500 bytes
- `#bytes=500-999`: Second 500 bytes
- `#bytes=500-`: From byte 500 to end
- `#bytes=-500`: Last 500 bytes
- `#bytes=0-0`: First byte only

### Multiple Ranges

Not supported in initial version. May be added later:
- `#bytes=0-499,1000-1499`: Two separate ranges

## URN Comparison and Equivalence

### Case Sensitivity

- **Scheme and namespace**: Case-insensitive (`urn:dig` = `URN:DIG`)
- **Store ID**: Case-sensitive (lowercase hex)
- **Root hash**: Case-sensitive (lowercase hex)
- **Resource path**: Case-sensitive
- **Byte range**: Case-sensitive

### Normalization

1. Convert scheme and namespace to lowercase
2. Ensure hex values are lowercase
3. Normalize path separators to forward slashes
4. Remove redundant path components (`.`, `..`)
5. URL-decode unnecessary escapes

### Equivalence Examples

These are equivalent:
- `URN:DIG:abc123/file.txt` 
- `urn:dig:chia:abc123/file.txt`

These are different:
- `urn:dig:chia:abc123/File.txt`
- `urn:dig:chia:abc123/file.txt`

## Security Considerations

1. **Path Traversal**: Prevent `../` attacks in resource paths
2. **Byte Range Validation**: Ensure ranges are within file bounds
3. **Hash Verification**: Validate content against merkle proofs
4. **Access Control**: URNs don't include authentication

## Performance Considerations

1. **Caching**: URNs are immutable when version-specific
2. **Partial Retrieval**: Byte ranges reduce bandwidth
3. **Layer Traversal**: Minimize with periodic full layers
4. **Chunk Alignment**: Align byte ranges with chunk boundaries

## Future Extensions

1. **Multiple byte ranges**: `#bytes=0-99,200-299`
2. **Content negotiation**: `#type=application/json`
3. **Compression hints**: `#encoding=gzip`
4. **Streaming parameters**: `#stream=true&buffer=64k`
