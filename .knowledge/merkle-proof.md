# Merkle Proof Specification

## Overview

Digstore Min provides comprehensive merkle proof capabilities for verifying the integrity and membership of any data within the repository. Proofs can be generated for individual bytes, files, or entire layers against the repository root hash.

## Proof Types

### 1. File Proof
Proves a file exists in a specific repository state.

### 2. Byte Range Proof  
Proves specific bytes within a file are authentic.

### 3. Layer Proof
Proves a layer is part of the repository history.

### 4. Chunk Proof
Proves a chunk belongs to a file and layer.

## Merkle Tree Structure

### Repository Tree (Three Levels)

```
                Repository Root
                      |
        +-------------+-------------+
        |                           |
    Layer Root 1               Layer Root 2
        |                           |
   File Hashes                 File Hashes
```

### Layer Tree

Each layer contains its own merkle tree:

```
                Layer Root
                    |
        +-----------+-----------+
        |           |           |
    File Hash 1  File Hash 2  File Hash 3
```

### File Chunking Tree

Large files are split into chunks with their own tree:

```
                File Root
                    |
        +-----------+-----------+
        |           |           |
    Chunk 1     Chunk 2     Chunk 3
```

## Proof Format

### Standard Proof Structure

```json
{
  "version": "1.0",
  "proof_type": "file|byte_range|layer|chunk",
  "target": {
    "type": "file|bytes|layer|chunk",
    "path": "path/to/file.txt",
    "hash": "target_element_hash",
    "byte_range": {
      "start": 0,
      "end": 1024
    }
  },
  "root": {
    "type": "repository|layer|file",
    "hash": "root_hash_to_verify_against"
  },
  "proof_path": [
    {
      "hash": "sibling_hash",
      "position": "left|right"
    }
  ],
  "metadata": {
    "timestamp": 1699564800,
    "layer_number": 42,
    "store_id": "store_identifier"
  }
}
```

### Proof Path Elements

Each element in the proof path contains:
- **hash**: Sibling hash needed for verification
- **position**: Whether sibling is on left or right

## Proof Generation

### File Proof Generation

1. **Locate file** in target layer
2. **Compute file hash** from chunks
3. **Build layer proof**:
   - Get file position in layer tree
   - Collect sibling hashes up to layer root
4. **Build repository proof**:
   - Get layer position in repository
   - Collect sibling hashes up to repository root
5. **Combine paths** into complete proof

### Byte Range Proof Generation

1. **Locate file** and identify chunks
2. **Identify affected chunks** for byte range
3. **Generate chunk proofs**:
   - Proof for each chunk containing bytes
   - Include chunk offset/size information
4. **Build file proof** from chunks
5. **Extend to repository root**

### Optimization for Byte Ranges

For efficient byte range proofs:
- Only include affected chunks
- Provide chunk boundaries
- Allow partial chunk verification

## Proof Verification

### Standard Verification Algorithm

```python
def verify_proof(target_hash, root_hash, proof_path):
    current_hash = target_hash
    
    for sibling in proof_path:
        if sibling.position == "left":
            current_hash = hash(sibling.hash + current_hash)
        else:
            current_hash = hash(current_hash + sibling.hash)
    
    return current_hash == root_hash
```

### Byte Range Verification

1. **Verify chunk proofs** individually
2. **Reconstruct file segment** from chunks
3. **Verify segment hash**
4. **Verify file proof** up to root

## Proof Examples

### Example 1: Simple File Proof

```json
{
  "version": "1.0",
  "proof_type": "file",
  "target": {
    "type": "file",
    "path": "src/main.rs",
    "hash": "a5c3f8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2"
  },
  "root": {
    "type": "repository",
    "hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
  },
  "proof_path": [
    {
      "hash": "b2c4d6e8fa0c2e4f6a8ca0c2e4f6a8ca0c2e4f6a8ca0c2e4f6a8ca0c2e4f6a8c",
      "position": "right"
    },
    {
      "hash": "c3d5e7f9a1b3c5e7f9a1b3c5e7f9a1b3c5e7f9a1b3c5e7f9a1b3c5e7f9a1b3c",
      "position": "left"
    }
  ]
}
```

### Example 2: Byte Range Proof

```json
{
  "version": "1.0",
  "proof_type": "byte_range",
  "target": {
    "type": "bytes",
    "path": "large_file.bin",
    "byte_range": {
      "start": 1024,
      "end": 2048
    },
    "hash": "f1e2d3c4b5a6978685746352413021110f0e0d0c0b0a09080706050403020100"
  },
  "chunks": [
    {
      "index": 1,
      "offset": 1024,
      "size": 1024,
      "hash": "123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
      "proof_path": [
        {
          "hash": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
          "position": "left"
        }
      ]
    }
  ],
  "root": {
    "type": "repository",
    "hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
  }
}
```

## Batch Proofs

For efficiency, multiple proofs can be combined:

```json
{
  "version": "1.0",
  "proof_type": "batch",
  "targets": [
    {
      "path": "file1.txt",
      "hash": "hash1"
    },
    {
      "path": "file2.txt", 
      "hash": "hash2"
    }
  ],
  "common_path": [
    {
      "hash": "common_ancestor_hash",
      "position": "right"
    }
  ],
  "individual_paths": {
    "file1.txt": [...],
    "file2.txt": [...]
  }
}
```

## Security Properties

### Collision Resistance
- SHA-256 provides 256-bit security
- Computationally infeasible to find collisions

### Proof Minimality
- Proofs contain only necessary hashes
- Cannot be reduced without losing verifiability

### Non-repudiation
- Valid proofs demonstrate data existed
- Timestamp provides temporal evidence

## Performance Optimization

### Proof Caching
- Cache frequently requested proofs
- Reuse common path segments
- Update incrementally with new layers

### Batch Generation
- Generate multiple proofs together
- Share common computations
- Reduce tree traversals

### Sparse Trees
- Use efficient sparse merkle trees
- Handle large file counts efficiently
- Optimize for proof size

## Implementation Considerations

### Hash Function
- Use SHA-256 for all hashing
- Consider BLAKE3 for performance
- Ensure consistent byte ordering

### Tree Construction
- Use balanced binary trees
- Pad with null hashes if needed
- Deterministic construction

### Proof Serialization
- Use canonical JSON formatting
- Support binary format for efficiency
- Include version for compatibility

## Verification Best Practices

1. **Always verify proof version**
2. **Check proof type matches expectation**
3. **Validate all hashes are correct length**
4. **Verify timestamp reasonableness**
5. **Cache verification results**

## Future Extensions

1. **Aggregated Proofs**: Combine multiple proofs efficiently
2. **Incremental Proofs**: Update proofs as repository grows  
3. **Privacy-Preserving Proofs**: Zero-knowledge variants
4. **Compression**: Optimize proof size
5. **Hardware Acceleration**: Use crypto accelerators
