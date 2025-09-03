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

After installation, verify digstore is working:
```bash
digstore --version
```

### Basic Workflow

```bash
# 1. Initialize a new repository
digstore init

# 2. Add files to staging
digstore add README.md              # Add single file
digstore add -r src/                # Add directory recursively
digstore add -A                     # Add all files

# 3. Create a commit
digstore commit -m "Initial commit"

# 4. Check repository status
digstore status

# 5. View commit history
digstore log

# 6. Retrieve files
digstore get README.md              # Get file content
digstore cat src/main.rs            # Display file content
digstore get README.md -o copy.md   # Save to file
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

### Option 1: Download Pre-built Installers (Recommended)

**Windows:**
1. Download the [Windows Installer (MSI)](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-windows-x64.msi)
2. Run the installer and follow the prompts
3. Restart your terminal or log out/in for PATH changes to take effect
4. Verify installation: `digstore --version`

**macOS:**
1. Download the [macOS Installer (DMG)](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-macos.dmg)
2. Mount the DMG and drag Digstore to Applications
3. Add to PATH: `echo 'export PATH="/Applications/Digstore.app/Contents/MacOS:$PATH"' >> ~/.zshrc`
4. Reload terminal: `source ~/.zshrc`

**Linux (Ubuntu/Debian):**
```bash
# Download and install DEB package
wget https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore_0.1.0_amd64.deb
sudo dpkg -i digstore_0.1.0_amd64.deb
```

**Linux (RHEL/CentOS/Fedora):**
```bash
# Download and install RPM package
wget https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-0.1.0-1.x86_64.rpm
sudo rpm -i digstore-0.1.0-1.x86_64.rpm
```

**Linux (AppImage - Universal):**
```bash
# Download AppImage
wget https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-linux-x86_64.AppImage
chmod +x digstore-linux-x86_64.AppImage

# Run directly or add to PATH
./digstore-linux-x86_64.AppImage --version
```

### Option 2: Build from Source

**Prerequisites:**
- Rust 1.70+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Git

**Build Steps:**
```bash
# Clone repository
git clone https://github.com/DIG-Network/digstore.git
cd digstore

# Build release binary
cargo build --release

# Install to system
cargo install --path .

# Verify installation
digstore --version
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

## Available Commands

### Core Commands

| Command | Description | Example |
|---------|-------------|---------|
| `init` | Initialize a new repository | `digstore init` |
| `add` | Add files to staging area | `digstore add file.txt` |
| `commit` | Create a new commit | `digstore commit -m "message"` |
| `status` | Show repository status | `digstore status` |
| `get` | Retrieve file content | `digstore get file.txt` |
| `cat` | Display file content | `digstore cat file.txt` |

### Repository Management

| Command | Description | Example |
|---------|-------------|---------|
| `log` | Show commit history | `digstore log` |
| `config` | Get/set configuration | `digstore config user.name "Your Name"` |

### Advanced Commands

| Command | Description | Example |
|---------|-------------|---------|
| `prove` | Generate merkle proof | `digstore prove file.txt -o proof.json` |
| `verify` | Verify a merkle proof | `digstore verify proof.json` |
| `decrypt` | Decrypt encrypted content | `digstore decrypt file.enc --urn "urn:dig:chia:..." |

### Subcommands

| Command | Description | Example |
|---------|-------------|---------|
| `layer list` | List all layers | `digstore layer list` |
| `layer inspect` | Inspect layer details | `digstore layer inspect HASH` |
| `store info` | Show store information | `digstore store info` |
| `store stats` | Show storage statistics | `digstore store stats` |
| `staged list` | List staged files | `digstore staged list` |
| `staged diff` | Show staged changes | `digstore staged diff` |

### Command Options

Most commands support these common options:

| Option | Description |
|--------|-------------|
| `--help` | Show command help |
| `--json` | Output in JSON format |
| `--verbose` | Enable verbose output |
| `--quiet` | Suppress output |
| `-o, --output` | Specify output file |

### Examples

**Initialize and commit:**
```bash
digstore init
digstore add -A
digstore commit -m "Initial commit"
```

**Retrieve specific versions:**
```bash
# Get latest version
digstore get file.txt

# Get at specific commit
digstore get file.txt --at COMMIT_HASH

# Get byte range
digstore get "file.txt#bytes=0-1023" -o first_kb.bin
```

**Work with URNs:**
```bash
# Get file using full URN
digstore get urn:dig:chia:STORE_ID/file.txt

# Generate and verify proof
digstore prove file.txt -o proof.json
digstore verify proof.json
```

**Configuration:**
```bash
# Set user info
digstore config user.name "Your Name"
digstore config user.email "your@email.com"

# Enable encrypted storage
digstore config crypto.public_key "your-32-byte-hex-key"
digstore config crypto.encrypted_storage true

# List all settings
digstore config --list
```

## Documentation

Comprehensive documentation is available in the [`.knowledge/`](.knowledge/) directory:

### Quick Links
- **[📖 Knowledge Base Index](.knowledge/00-index.md)** - Complete documentation index
- **[🚀 Quick Start Guide](.knowledge/42-quick-start-guide.md)** - Get started quickly
- **[💻 CLI Commands Reference](.knowledge/20-cli-commands.md)** - Complete command documentation
- **[🏗️ System Overview](.knowledge/01-overview.md)** - High-level introduction

### Key Topics
- **[🔐 Encrypted Storage](.knowledge/12-encrypted-storage.md)** - Zero-knowledge encrypted storage
- **[🎭 Zero-Knowledge URNs](.knowledge/13-zero-knowledge-urns.md)** - Privacy-preserving URN behavior
- **[📦 Store Structure](.knowledge/02-store-structure.md)** - Repository layout and organization
- **[🔍 URN Specification](.knowledge/04-urn-specification.md)** - URN format with byte ranges
- **[🌳 Merkle Proofs](.knowledge/05-merkle-proofs.md)** - Proof generation and verification

### For Developers
- **[👨‍💻 Implementation Checklist](.knowledge/40-implementation-checklist.md)** - Development roadmap
- **[📚 Rust Crates Guide](.knowledge/41-rust-crates-guide.md)** - Recommended dependencies
- **[🔧 API Design](.knowledge/70-api-design.md)** - Library API documentation

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

## Troubleshooting

### Common Issues

**Command not found after installation:**
- **Windows**: Restart your terminal or log out/in
- **macOS/Linux**: Run `source ~/.bashrc` or `source ~/.zshrc`
- **All**: Verify PATH includes digstore location

**Permission errors:**
- **Linux/macOS**: Use `sudo` for system-wide installation
- **Windows**: Run installer as Administrator

**Build from source fails:**
- Ensure Rust 1.70+ is installed: `rustc --version`
- Update Rust: `rustup update`
- Clean and retry: `cargo clean && cargo build --release`

### Getting Help

```bash
# General help
digstore --help

# Command-specific help
digstore init --help
digstore add --help
digstore commit --help

# Version information
digstore --version
```

## Development

### Building from Source

```bash
# Clone repository
git clone https://github.com/DIG-Network/digstore.git
cd digstore

# Build
cargo build --release

# Run tests
cargo test

# Install
cargo install --path .
```

### Project Structure

```
digstore/
├── src/
│   ├── main.rs          # CLI entry point
│   ├── lib.rs           # Library interface
│   ├── core/            # Core types and logic
│   ├── storage/         # Storage engine
│   ├── crypto/          # Encryption and security
│   ├── proofs/          # Merkle proof system
│   └── cli/             # CLI implementation
├── tests/               # Integration tests
├── benches/             # Performance benchmarks
├── docs/                # Additional documentation
└── .knowledge/          # Comprehensive knowledge base
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

Contributions are welcome! Please see our [Implementation Checklist](.knowledge/40-implementation-checklist.md) and ensure:

1. Tests pass: `cargo test`
2. Code is formatted: `cargo fmt`
3. Lints pass: `cargo clippy`
4. Documentation is updated
5. Follow the [coding guidelines](CONTRIBUTING.md)

## License

MIT OR Apache-2.0

This project is licensed under either of:
- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Features

### ✅ Implemented
- **Core Storage**: Content-addressable storage with SHA-256 hashing
- **Layer System**: Git-like commits with full and delta layers  
- **URN Support**: Permanent identifiers with byte range retrieval
- **Merkle Proofs**: Generate and verify cryptographic proofs
- **CLI Interface**: 11+ commands for complete repository management
- **Encrypted Storage**: Zero-knowledge encrypted storage with URN transformation
- **Zero-Knowledge URNs**: Privacy-preserving URN behavior
- **Cross-Platform**: Windows, macOS, and Linux support
- **Portable Format**: Self-contained repositories
- **Performance Optimized**: Efficient chunking and compression

### 🚧 Roadmap
- Network synchronization protocol
- S3/cloud storage backends  
- Watch mode for automatic commits
- GUI for repository visualization
- Plugin system for extensions

## Acknowledgments

This project implements a content-addressable storage system with advanced features including encrypted storage, zero-knowledge properties, and comprehensive merkle proof capabilities. It provides a Git-like interface while adding unique features for data integrity verification and privacy-preserving storage.