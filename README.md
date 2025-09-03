# Digstore Min

A simplified content-addressable storage system with Git-like semantics, merkle proofs, and URN-based retrieval.

## Overview

Digstore Min is a streamlined version of the Digstore project, focusing on core functionality without encryption, privacy features, or blockchain integration. It provides:

- **Content-Addressable Storage**: Every piece of data identified by its SHA-256 hash
- **Layer-Based Architecture**: Git-like commits with full and delta layers
- **Merkle Proofs**: Cryptographic proofs for any data item, byte range, or layer
- **URN-Based Retrieval**: Permanent identifiers with byte range support
- **Portable Format**: Self-contained repositories that work anywhere

## Key Features

### 🔐 Cryptographic Integrity
- Generate merkle proofs for any data
- Verify data authenticity at any point in history
- Tamper-evident layer chain

### 📦 Efficient Storage
- Content-defined chunking for deduplication
- Delta layers for space efficiency
- Optional compression support

### 🔍 Flexible Retrieval
- URN format: `urn:dig:chia:{storeID}[:{rootHash}][/{path}][#{byteRange}]`
- Stream specific byte ranges
- Access any historical version

### 🚀 Portable & Simple
- Single directory contains entire repository
- No external dependencies
- Easy transfer between systems

## Quick Start

```bash
# Initialize a new repository
digstore init

# Add files
digstore add -r src/
digstore add README.md

# Create a commit
digstore commit -m "Initial commit"

# Retrieve files (in project directory with .digstore)
digstore get /README.md
digstore cat /src/main.rs

# Or use full URN from anywhere
digstore get urn:dig:chia:STORE_ID/README.md

# Generate proof
digstore prove README.md -o proof.json

# Verify proof
digstore verify proof.json
```


## Download Latest Build

You can download the latest development build installers:

- [Windows Installer (MSI)](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-windows-x64.msi)
- [macOS Installer (DMG)](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-macos.dmg)
- [Linux DEB Package](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore_0.1.0_amd64.deb)
- [Linux RPM Package](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-0.1.0-1.x86_64.rpm)
- [Linux AppImage](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-linux-x86_64.AppImage)

For stable releases, visit the [Releases](https://github.com/DIG-Network/digstore/releases) page.

## Installation

```bash
# From source
cargo install --path .

# Or download pre-built binary
wget https://github.com/yourrepo/digstore_min/releases/latest/digstore
chmod +x digstore
```

## Core Concepts

### Store Structure

Digstore Min separates project metadata from repository data:

**Global Store (~/.dig/)**
```
~/.dig/
└── {store_id}/                # 32-byte hex identifier
    ├── 0000000000000000.layer # Layer 0 (metadata)
    ├── {hash1}.layer          # Layer 1
    ├── {hash2}.layer          # Layer 2
    └── ...
```

**Local Project**
```
my-project/
├── .digstore                  # Links to global store
├── src/
└── README.md
```

### URN Examples
```
# Latest version of entire store
urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2

# Specific file at latest version
urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/src/main.rs

# File at specific version
urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2:e3b0c44298fc/src/main.rs

# Byte range
urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/video.mp4#bytes=0-1048576
```

## Documentation

Comprehensive documentation is available in the `.knowledge/` directory:

- [Overview](digstore_min/.knowledge/overview.md) - High-level introduction
- [Store Structure](digstore_min/.knowledge/store-structure.md) - Repository layout and organization
- [URN Specification](digstore_min/.knowledge/urn-specification.md) - URN format with byte ranges
- [Layer Format](digstore_min/.knowledge/layer-format.md) - Binary layer file specification
- [Merkle Proofs](digstore_min/.knowledge/merkle-proof.md) - Proof generation and verification
- [CLI Commands](digstore_min/.knowledge/cli-commands.md) - Complete command reference
- [API Design](digstore_min/.knowledge/api-design.md) - Library API documentation
- [Implementation Guide](digstore_min/.knowledge/implementation-guide.md) - Development roadmap

## Use Cases

1. **Version Control**: Track changes to any files over time
2. **Data Integrity**: Cryptographically verify data authenticity
3. **Archival Storage**: Long-term preservation with proof of existence
4. **Content Distribution**: Share verifiable, tamper-proof data sets
5. **Audit Trails**: Maintain immutable history of all changes

## Architecture

```
┌─────────────────────────────┐
│      CLI Interface          │
├─────────────────────────────┤
│    High-Level API           │
├─────────────────────────────┤
│     Core Services           │
├─────────────────────────────┤
│    Storage Engine           │
├─────────────────────────────┤
│   Platform Abstractions     │
└─────────────────────────────┘
```

## Development

### Building from Source

```bash
# Clone repository
git clone https://github.com/yourrepo/digstore_min
cd digstore_min

# Build
cargo build --release

# Run tests
cargo test

# Install
cargo install --path .
```

### Project Structure

```
digstore_min/
├── src/
│   ├── main.rs          # CLI entry point
│   ├── lib.rs           # Library interface
│   ├── core/            # Core types and logic
│   ├── storage/         # Storage engine
│   ├── proofs/          # Merkle proof system
│   └── cli/             # CLI implementation
├── tests/               # Integration tests
├── benches/             # Performance benchmarks
└── .knowledge/          # Documentation
```

## Performance

- **Chunking**: ~100 MB/s on modern hardware
- **Hashing**: ~500 MB/s using SHA-256
- **Compression**: Optional zstd compression
- **Memory**: O(n) for file count, streaming for large files

## Comparison with Git

| Feature | Git | Digstore Min |
|---------|-----|--------------|
| Version Control | ✓ | ✓ |
| Merkle Trees | ✓ | ✓ |
| Delta Storage | ✓ | ✓ |
| URN Support | ✗ | ✓ |
| Byte Range Retrieval | ✗ | ✓ |
| Proof Generation | ✗ | ✓ |
| Binary Optimization | Limited | ✓ |

## Future Enhancements

While keeping the core simple, potential additions include:

- Network synchronization protocol
- S3/cloud storage backends
- Watch mode for automatic commits
- GUI for visualization
- Plugin system for extensions

## Contributing

Contributions are welcome! Please read the implementation guide and ensure:

1. Tests pass: `cargo test`
2. Code is formatted: `cargo fmt`
3. Lints pass: `cargo clippy`
4. Documentation is updated

## License

[Your chosen license]

## Acknowledgments

This is a simplified version of the original Digstore project, focusing on core content-addressable storage functionality without the complexity of encryption, privacy features, or blockchain integration.