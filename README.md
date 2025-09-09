# Digstore CLI Commands

Complete command reference for Digstore - a content-addressable storage system with Git-like semantics.

## Quick Start

```bash
digstore init                    # Initialize repository
digstore add file.txt           # Add files
digstore commit -m "message"    # Create commit
digstore status                 # Check status
digstore get file.txt           # Retrieve files
```

## Core Repository Commands

### `digstore init` - Initialize Repository

Initialize a new repository in the current directory.

```bash
digstore init                                    # Basic initialization
digstore init --name "My Project"               # With custom name
digstore init --encryption-key "1234567890..."  # Custom encryption key
```

**Options:**
- `--name <NAME>` - Set repository name
- `--encryption-key <HEX>` - 64-character hex encryption key for store-wide secrets

### `digstore add` - Stage Files

Add files to the staging area for the next commit.

```bash
digstore add file.txt                    # Add single file
digstore add file1.txt file2.txt         # Add multiple files
digstore add -r src/                     # Add directory recursively
digstore add -A                          # Add all files in repository
digstore add --dry-run -A                # Show what would be added
digstore add --force ignored.tmp         # Force add ignored files
find . -name "*.txt" | digstore add --from-stdin  # From stdin
```

**Options:**
- `-r, --recursive` - Add directories recursively
- `-A, --all` - Add all files in the repository
- `-f, --force` - Force add ignored files
- `--dry-run` - Show what would be added without staging
- `--from-stdin` - Read file list from stdin
- `--json` - Output as JSON

### `digstore commit` - Create Commit

Create a new commit from staged files.

```bash
digstore commit -m "Add new features"           # Basic commit
digstore commit -m "Fix bug" --author "John"   # With custom author
digstore commit -m "Backdate" --date "2023-01-01"  # Custom date
digstore commit -e -m "Initial"               # Open editor for message
```

**Options:**
- `-m, --message <MESSAGE>` - Commit message (required)
- `--author <NAME>` - Set author name
- `--date <DATE>` - Override commit date
- `-e, --edit` - Open editor for message
- `--json` - Output as JSON

### `digstore status` - Repository Status

Show repository status and staged files.

```bash
digstore status                    # Full status
digstore status --short            # Short format
digstore status --porcelain        # Machine-readable
digstore status --show-chunks      # With chunk statistics
```

**Options:**
- `-s, --short` - Show short format
- `--porcelain` - Machine-readable output
- `--show-chunks` - Display chunk statistics
- `--json` - Output as JSON

## File Access Commands

### `digstore get` - Retrieve Files

Retrieve files from the repository (returns encrypted data).

```bash
digstore get file.txt                           # To stdout
digstore get file.txt -o output.bin             # Save to file
digstore get file.txt --at COMMIT_HASH          # At specific commit
digstore get "urn:dig:chia:STORE_ID/file.txt"   # Using URN
digstore get file.txt --verify                  # With verification
digstore get file.txt --metadata                # Include metadata
digstore get file.txt --progress                # Show progress
```

**Options:**
- `-o, --output <FILE>` - Output file (default: stdout)
- `--verify` - Verify with merkle proof
- `--metadata` - Include metadata in output
- `--at <HASH>` - Retrieve at specific root hash
- `--progress` - Force show progress
- `--json` - Output as JSON


### `digstore decrypt` - Decrypt Content

Decrypt encrypted content using URN.

```bash
digstore decrypt encrypted.bin --urn "urn:dig:chia:STORE_ID/file.txt"
digstore decrypt encrypted.bin --urn "urn:..." -o decrypted.txt
digstore decrypt encrypted.bin --decryption-key "hex-key"
```

**Options:**
- `-o, --output <FILE>` - Output file (default: stdout)
- `--urn <URN>` - URN for decryption
- `--decryption-key <HEX>` - Custom decryption key
- `--json` - Output as JSON

## Staging Management

### `digstore staged` - Staging Operations

```bash
digstore staged                      # List staged files
digstore staged --page 2 --limit 10  # Pagination
digstore staged --all                # Show all files
digstore staged --detailed           # Detailed view
```

**Options:**
- `-l, --limit <NUM>` - Files per page (default: 20)
- `-p, --page <NUM>` - Page number (default: 1)
- `-d, --detailed` - Show detailed information
- `-a, --all` - Show all files (no pagination)
- `--json` - Output as JSON

### `digstore staged diff` - Show Differences

```bash
digstore staged diff                     # Show differences
digstore staged diff --name-only         # File names only
digstore staged diff --stat              # Statistics
digstore staged diff --file "src/main.rs"  # Specific file
digstore staged diff -U 5               # Custom context lines
```

**Options:**
- `--name-only` - Show only file names
- `--stat` - Show statistics summary
- `-U, --unified <LINES>` - Context lines (default: 3)
- `--file <FILE>` - Specific file to diff
- `--json` - Output as JSON

### `digstore staged clear` - Clear Staging

```bash
digstore staged clear         # Clear with confirmation
digstore staged clear --force # Force clear
```

**Options:**
- `-f, --force` - Force clear without confirmation
- `--json` - Output as JSON

## Configuration Management

### `digstore config` - Global Configuration

```bash
digstore config                              # Show usage
digstore config user.name                    # Get value
digstore config user.name "Your Name"        # Set value
digstore config user.email "your@email.com"  # Set email (optional)
digstore config --list                       # List all values
digstore config --unset user.email           # Unset value
digstore config --show-origin                # Show config file location
digstore config --edit                       # Edit in editor
```

**Common Configuration Keys:**
- `user.name` - Your name for commits
- `user.email` - Your email for commits (optional)
- `core.editor` - Default editor
- `crypto.public_key` - Public key for URN transformation (64-char hex)
- `wallet.active_profile` - Active wallet profile

**Options:**
- `-l, --list` - List all configuration values
- `--unset` - Unset a configuration value
- `--show-origin` - Show config file location
- `-e, --edit` - Edit configuration file in editor
- `--json` - Output as JSON

### `digstore wallet` - Wallet Management

```bash
# List and info
digstore wallet list                           # List all wallets
digstore wallet info                          # Show active wallet
digstore wallet info --profile "my-profile"   # Specific profile
digstore wallet active                        # Show active profile

# Create and manage
digstore wallet create my-profile             # Create new wallet
digstore wallet create my-profile --set-active # Create and set active
digstore wallet set-active my-profile         # Set active profile
digstore wallet delete my-profile --force     # Delete wallet

# Export (DANGEROUS - only in secure environments)
digstore wallet export                        # Export active wallet mnemonic
digstore wallet info --show-mnemonic          # Show mnemonic in info
```

**Wallet Subcommands:**
- `list` - List all wallets
- `info [--profile NAME] [--show-mnemonic]` - Show wallet information
- `create <PROFILE> [--from-mnemonic PHRASE] [--set-active]` - Create wallet
- `delete <PROFILE> [--force]` - Delete wallet
- `set-active <PROFILE>` - Set active wallet profile
- `active` - Show active wallet profile
- `export [--profile NAME]` - Export mnemonic (DANGEROUS)

## Update and Version Management

### `digstore update` - Update System

```bash
digstore update                    # Check and install updates
digstore update --check-only       # Check only, don't install
digstore update --force            # Force update without confirmation
```

**Options:**
- `--check-only` - Only check for updates, don't install
- `--force` - Force update without confirmation
- `--json` - Output as JSON

### `digstore version` - Version Management

Manage multiple digstore versions with automatic PATH management.

```bash
# Version information
digstore version                           # Show version info
digstore version current                   # Show current version
digstore version list                      # User-installed versions
digstore version list-system              # System-installed versions

# Install and manage versions
digstore version install-current           # Install current binary
digstore version install-msi installer.msi # Install from MSI
digstore version set 0.4.5                # Set active version
digstore version remove 0.4.3             # Remove version

# PATH management
digstore version update-path 0.4.5        # Update PATH for version
digstore version fix-path                 # Analyze PATH conflicts
digstore version fix-path-auto            # Auto-fix PATH ordering
```

**Version Subcommands:**
- `list` - List user-installed versions
- `list-system` - List system-installed versions
- `install-current` - Install currently running binary
- `install-msi <PATH>` - Install from MSI file
- `set <VERSION>` - Set active version
- `remove <VERSION>` - Remove a version
- `update-path <VERSION>` - Update PATH for version
- `fix-path` - Analyze PATH conflicts
- `fix-path-auto` - Automatically fix PATH ordering
- `current` - Show current version

## Cryptographic Proof System

### `digstore proof generate` - Generate Proofs

Generate merkle proofs for content verification.

```bash
digstore proof generate file.txt                      # File proof (compact format)
digstore proof generate file.txt -o proof.json       # Save to file
digstore proof generate file.txt --bytes "0-1023"    # Byte range proof
digstore proof generate file.txt --at COMMIT_HASH    # At specific commit
digstore proof generate file.txt --format text       # Text format
digstore proof generate file.txt --json              # JSON format
digstore proof generate "urn:dig:chia:STORE_ID/file.txt"  # From URN
```

**Options:**
- `-o, --output <FILE>` - Write proof to file (default: stdout)
- `--format <FORMAT>` - Output format: json, binary, text (default: compact)
- `--at <HASH>` - Prove at specific root hash
- `--bytes <RANGE>` - Prove specific byte range
- `--json` - Output as JSON (overrides format)

### `digstore proof verify` - Verify Proofs

Verify merkle proofs.

```bash
digstore proof verify proof.json                      # Verify proof
digstore proof verify proof.json --target "hash"     # With expected target
digstore proof verify proof.json --root "hash"       # With expected root
digstore proof verify proof.json --verbose           # Verbose verification
echo "proof" | digstore proof verify --from-stdin    # From stdin
```

**Options:**
- `--target <HASH>` - Expected target hash
- `--root <HASH>` - Expected root hash
- `-v, --verbose` - Show detailed verification steps
- `--from-stdin` - Read proof from stdin

### `digstore proof generate-archive-size` - Archive Size Proofs

Generate tamper-proof archive size proofs without requiring file downloads.

```bash
# Auto-detect store from .digstore file
digstore proof generate-archive-size
digstore proof generate-archive-size -o size_proof.txt
digstore proof generate-archive-size --verbose

# With specific store ID
digstore proof generate-archive-size STORE_ID
digstore proof generate-archive-size STORE_ID -o proof.txt --verbose

# Show compression statistics
digstore proof generate-archive-size --show-compression

# JSON format
digstore proof generate-archive-size --json
```

**Options:**
- `store_id` - Store ID (32-byte hex), optional if in repository
- `-o, --output <FILE>` - Output file (default: stdout)
- `--format <FORMAT>` - compressed, json, binary (default: compressed)
- `-v, --verbose` - Show verbose proof generation steps
- `--show-compression` - Show compression statistics
- `--json` - Output as JSON

### `digstore proof verify-archive-size` - Verify Size Proofs

Verify archive size proofs without file access.

```bash
# Verify from file
digstore proof verify-archive-size --from-file proof.txt STORE_ID ROOT_HASH SIZE PUBLISHER_KEY

# Verify from command line
digstore proof verify-archive-size "proof-data" STORE_ID ROOT_HASH SIZE PUBLISHER_KEY

# Verbose verification
digstore proof verify-archive-size --from-file proof.txt STORE_ID ROOT_HASH SIZE PUBLISHER_KEY --verbose
```

**Arguments:**
- `proof` - Proof data (hex string or file with --from-file)
- `store_id` - Store ID (32-byte hex)
- `root_hash` - Root hash (32-byte hex)
- `expected_size` - Expected size in bytes
- `publisher_public_key` - Publisher public key (32-byte hex)

**Options:**
- `--from-file` - Read proof from file
- `-v, --verbose` - Show detailed verification steps
- `--json` - Output as JSON

### `digstore keygen` - Generate Keys

Generate content keys from URN and public key.

```bash
digstore keygen "urn:dig:chia:STORE_ID/file.txt"              # Generate keys
digstore keygen "urn:dig:chia:STORE_ID/file.txt" -o keys.txt  # Save to file
digstore keygen "urn:..." --storage-address                   # Only storage address
digstore keygen "urn:..." --encryption-key                    # Only encryption key
```

**Options:**
- `-o, --output <FILE>` - Output file (default: stdout)
- `--storage-address` - Show only storage address
- `--encryption-key` - Show only encryption key
- `--json` - Output as JSON

## Store Information Commands

### `digstore store` - Store Operations

```bash
# Store information
digstore store info                    # Basic info
digstore store info --config          # With configuration
digstore store info --paths           # Show all paths

# History and logs
digstore store log                     # Commit history
digstore store log -n 10 --graph      # Limited with graph
digstore store history --stats         # Root history analysis
digstore store root --verbose          # Current root info

# Analytics
digstore store size --breakdown        # Storage analytics
digstore store stats --performance     # Repository statistics
```

**Store Subcommands:**
- `info [--config] [--paths] [--layer HASH]` - Show store information
- `log [-n NUM] [--oneline] [--graph] [--since DATE]` - Show commit history
- `history [-n NUM] [--stats] [--graph] [--since DATE]` - Root history analysis
- `root [--verbose] [--hash-only]` - Show current root information
- `size [--breakdown] [--efficiency] [--layers]` - Show storage analytics
- `stats [--detailed] [--performance] [--security]` - Show repository statistics

### `digstore layer` - Layer Management

```bash
# List and analyze layers
digstore layer list                    # List all layers
digstore layer list --size --files    # With details
digstore layer analyze LAYER_HASH     # Analyze specific layer
digstore layer inspect LAYER_HASH --verify  # Deep inspection
```

**Layer Subcommands:**
- `list [--size] [--files] [--chunks]` - List all layers
- `analyze <HASH> [--size] [--files] [--chunks]` - Analyze specific layer
- `inspect <HASH> [--header] [--merkle] [--chunks] [--verify]` - Deep inspection

## URN Format

URN Format: `urn:dig:chia:{storeID}[:{rootHash}][/{resourcePath}][#{byteRange}]`

### URN Examples 

```bash
# Entire store (latest version)
urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2

# Specific file (latest version)
urn:dig:chia:STORE_ID/src/main.rs

# File at specific commit
urn:dig:chia:STORE_ID:COMMIT_HASH/src/main.rs

# Byte ranges
urn:dig:chia:STORE_ID/video.mp4#bytes=0-1048575    # First 1MB
urn:dig:chia:STORE_ID/log.txt#bytes=-1024          # Last 1KB
urn:dig:chia:STORE_ID/data.bin#bytes=1000-         # From byte 1000 to end
```

### Using URNs

```bash
digstore get "urn:dig:chia:STORE_ID/file.txt"
digstore proof generate "urn:dig:chia:STORE_ID/file.txt"
digstore get "urn:dig:chia:STORE_ID/video.mp4#bytes=0-1048575"
digstore decrypt encrypted.bin --urn "urn:dig:chia:STORE_ID/file.txt"
```

## Common Workflows

### Repository Setup
```bash
digstore init --name "My Project"
digstore config user.name "Your Name"
digstore config user.email "your@email.com"  # Optional
digstore add -A
digstore commit -m "Initial commit"
```

### Daily Usage
```bash
digstore status                      # Check current state
digstore add changed-files.txt       # Stage changes
digstore staged diff                 # Review changes
digstore commit -m "Update files"    # Commit
digstore store log                   # View history
```

### File Retrieval
```bash
digstore get file.txt                # Latest version (encrypted)
digstore get file.txt --at HASH     # Specific version
digstore get "file.txt#bytes=0-1023" -o first_kb.bin  # Byte range
```

### Proof Generation
```bash
digstore proof generate file.txt -o proof.json
digstore proof verify proof.json --verbose
digstore proof generate-archive-size -o size_proof.txt
digstore proof verify-archive-size --from-file size_proof.txt STORE_ID ROOT_HASH SIZE PUBLISHER_KEY
```

### Version Management
```bash
digstore version list               # See installed versions
digstore update --check-only        # Check for updates
digstore update                     # Install updates
digstore version fix-path-auto      # Fix PATH conflicts
```

## Global Options

All commands support these options:

- `-v, --verbose` - Enable verbose output
- `-q, --quiet` - Suppress non-error output
- `--no-progress` - Disable progress bars
- `--color <MODE>` - Color output: auto, always, never
- `-y, --yes` - Auto-answer yes to all prompts
- `--non-interactive` - Suppress all prompts, use defaults
- `--auto-generate-wallet` - Auto-generate wallet if needed
- `--wallet-profile <PROFILE>` - Wallet profile to use
- `-h, --help` - Print help

## Getting Help

```bash
digstore --help                     # General help
digstore <command> --help           # Command-specific help
digstore <command> <subcommand> --help  # Subcommand help

# Examples:
digstore init --help
digstore add --help
digstore proof generate --help
digstore store info --help
```


## Download Latest Build

You can download the latest development build installers:

- [Windows Installer (MSI)](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-windows-x64.msi)
- [macOS Installer (DMG)](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-macos.dmg)
- [Linux DEB Package](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore_0.1.0_amd64.deb)
- [Linux RPM Package](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-0.1.0-1.x86_64.rpm)
- [Linux AppImage](https://github.com/DIG-Network/digstore/releases/download/latest-build/digstore-linux-x86_64.AppImage)

For stable releases, visit the [Releases](https://github.com/DIG-Network/digstore/releases) page.
