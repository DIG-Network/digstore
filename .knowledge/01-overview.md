# Digstore Min - Advanced Content-Addressable Storage System

## Overview

Digstore Min is an enterprise-grade content-addressable storage system with advanced security, performance, and zero-knowledge features. It provides a Git-like repository system with encrypted storage, URN transformation, comprehensive file filtering, and exceptional performance optimization.

## Core Concepts

### 1. Content-Addressable Storage
Every piece of data is identified by its SHA-256 hash, ensuring data integrity and enabling deduplication.

### 2. Layer-Based Architecture
- Data is organized into layers, similar to Git commits
- Each layer represents a snapshot of the repository state
- Layers can be full (complete state) or delta (changes only)
- Layers are stored as single portable files

### 3. Merkle Tree Structure
- Every layer has a merkle root representing all data in that layer
- The repository has a root hash representing all layers
- Merkle proofs can be generated for any data item against the root hash

### 4. URN-Based Retrieval
- Uniform Resource Names (URNs) provide permanent identifiers
- Format: `urn:dig:chia:{storeID}:{rootHash}/{resourcePath}#{byteRange}`
- Supports retrieving data at any historical state
- Byte range support for partial file retrieval

### 5. Portable Repository Format
- Repository data stored globally in `~/.dig/{store_id}/`
- Local projects contain only a `.digstore` file linking to the global store
- All layers are self-contained portable files
- Easy transfer by copying the store directory from `~/.dig/`
- No external dependencies or complex configuration

## Key Features

1. **Zero-Knowledge URN Retrieval**: Invalid URNs return deterministic random data, preventing enumeration attacks
2. **Encrypted Storage with URN Transformation**: Data encrypted using URN keys, stored at transformed addresses
3. **Single-File Archive Format**: `.dig` archive files replace directory-based storage
4. **Advanced Performance Engine**: Adaptive processing, parallel batch operations, streaming for large files
5. **Comprehensive File Filtering**: `.digignore` support with exact `.gitignore` syntax
6. **Memory-Efficient Processing**: Constant memory usage regardless of file size
7. **Enterprise Security**: URN-based access control, data scrambling, multi-layer encryption
8. **Rich CLI Interface**: 15+ commands with progress bars, JSON output, colored formatting
9. **Binary Staging System**: High-performance staging for large repositories (20,000+ files)
10. **Cryptographic Integrity**: Merkle proofs, SHA-256 verification, tamper-evident storage

## Architecture Components

1. **Store ID**: 32-byte random identifier for the repository
2. **Layer 0**: Special header layer containing metadata and root history
3. **Data Layers**: Sequential layers containing actual data
4. **Layer Format**: Binary format optimized for streaming and diff storage
5. **URN System**: Permanent identifiers with byte range support

## Use Cases

1. **Version Control**: Track changes to files over time
2. **Data Integrity**: Cryptographic verification of all data
3. **Archival Storage**: Long-term preservation with proof of authenticity
4. **Content Distribution**: Share verifiable data sets
5. **Audit Trails**: Immutable history of all changes

## Design Principles

1. **Simplicity**: Focus on core functionality without complexity
2. **Portability**: Self-contained repositories that work anywhere
3. **Verifiability**: Cryptographic proofs for all data
4. **Efficiency**: Optimized for storage and retrieval performance
5. **Extensibility**: Clean architecture allowing future enhancements
