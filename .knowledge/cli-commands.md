# CLI Commands Reference

## Overview

Digstore Min provides a command-line interface for managing content-addressable repositories. Commands follow a familiar Git-like pattern while adding specialized functionality for merkle proofs and URN-based retrieval.

## Global Options

```bash
digstore [OPTIONS] <COMMAND>

OPTIONS:
  -v, --verbose     Enable verbose output
  -q, --quiet       Suppress non-error output
  --store <PATH>    Path to store directory (default: current directory)
  --version         Print version information
  -h, --help        Print help information
```

## Core Commands

### init

Initialize a new repository in the current directory.

```bash
digstore init [OPTIONS]

OPTIONS:
  --store-id <ID>   Use specific store ID (default: generate random)
  --no-compression  Disable compression
  --chunk-size <N>  Set chunk size in KB (default: 64)
```

**Example:**
```bash
# Initialize new repository
digstore init

# Initialize with custom store ID
digstore init --store-id a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8
```

### add

Add files to the staging area for the next layer.

```bash
digstore add [OPTIONS] <PATH>...

OPTIONS:
  -r, --recursive   Add directories recursively
  -A, --all         Add all files in the repository
  -f, --force       Force add ignored files
  --dry-run         Show what would be added
```

**Example:**
```bash
# Add single file
digstore add README.md

# Add all files in repository
digstore add -A

# Add directory recursively  
digstore add -r src/

# Add multiple paths
digstore add file1.txt file2.txt docs/
```

### commit

Create a new layer from staged changes.

```bash
digstore commit [OPTIONS]

OPTIONS:
  -m, --message <MSG>   Commit message
  --full                Create full layer (not delta)
  --author <NAME>       Set author name
```

**Example:**
```bash
# Create delta layer
digstore commit -m "Add new feature"

# Force full layer
digstore commit --full -m "Periodic snapshot"
```

### status

Show repository status and staged changes.

```bash
digstore status [OPTIONS]

OPTIONS:
  -s, --short      Show short format
  --porcelain      Machine-readable output
```

**Example:**
```bash
# Show detailed status
digstore status

# Short format
digstore status -s
```

### log

Show layer history.

```bash
digstore log [OPTIONS]

OPTIONS:
  -n, --limit <N>      Limit number of entries
  --oneline           One line per layer
  --graph             Show ASCII graph
  --since <DATE>      Show layers since date
```

**Example:**
```bash
# Show recent history
digstore log -n 10

# Compact view
digstore log --oneline
```

## Retrieval Commands

### get

Retrieve content by URN or path (when in a project directory).

```bash
digstore get [OPTIONS] <URN_OR_PATH>

OPTIONS:
  -o, --output <PATH>   Write to file (default: stdout)
  --verify              Verify with merkle proof
  --metadata            Include metadata
  --at <HASH>           Retrieve at specific root hash

ARGUMENTS:
  <URN_OR_PATH>         Full URN or just file path when in project directory
```

**Example:**
```bash
# In a project directory with .digstore file:
digstore get /src/main.rs                    # Uses store ID from .digstore
digstore get /src/main.rs --at def456        # At specific version
digstore get "/large.bin#bytes=0-1023"        # With byte range

# Full URN (works from anywhere):
digstore get urn:dig:chia:abc123/src/main.rs
digstore get urn:dig:chia:abc123:def456/src/main.rs -o main.rs
digstore get "urn:dig:chia:abc123/large.bin#bytes=0-1023" -o first_kb.bin
```

**Note:** When in a directory with a `.digstore` file, the store ID is automatically read from it, allowing you to use simple paths instead of full URNs.

### cat

Display file contents from repository.

```bash
digstore cat [OPTIONS] <PATH>

OPTIONS:
  --at <HASH>      Show at specific root hash
  -n, --number     Number lines

ARGUMENTS:
  <PATH>           File path (uses store ID from .digstore if present)
```

**Example:**
```bash
# In a project directory with .digstore:
digstore cat README.md
digstore cat src/main.rs --at e3b0c44298fc

# With byte range (in project directory):
digstore cat "large_file.dat#bytes=0-1023"
```

**Note:** Like `get`, this command automatically uses the store ID from `.digstore` when in a project directory.

### extract

Extract files from repository.

```bash
digstore extract [OPTIONS] <PATH> <DESTINATION>

OPTIONS:
  --at <HASH>           Extract at specific root hash
  -f, --force          Overwrite existing files
  --preserve-mtime     Preserve modification times
```

**Example:**
```bash
# Extract single file
digstore extract src/main.rs ./main.rs

# Extract directory
digstore extract src/ ./extracted/src/
```

## Proof Commands

### prove

Generate merkle proof for content.

```bash
digstore prove [OPTIONS] <TARGET>

OPTIONS:
  -o, --output <PATH>   Write proof to file
  --format <FMT>       Output format: json|binary
  --at <HASH>          Prove at specific root hash
  --bytes <RANGE>      Prove byte range
```

**Example:**
```bash
# Prove file exists
digstore prove src/main.rs -o proof.json

# Prove byte range
digstore prove src/data.bin --bytes 1024-2048 -o range_proof.json
```

### verify

Verify a merkle proof.

```bash
digstore verify [OPTIONS] <PROOF>

OPTIONS:
  --target <HASH>      Expected target hash
  --root <HASH>        Expected root hash
  --verbose            Show detailed verification
```

**Example:**
```bash
# Verify proof file
digstore verify proof.json

# Verify with expected hashes
digstore verify proof.json --root e3b0c44298fc
```

## Layer Management

### layers

List all layers in repository.

```bash
digstore layers [OPTIONS]

OPTIONS:
  --full               Show only full layers
  --delta              Show only delta layers  
  --limit <N>          Limit number shown
  --size               Show layer sizes
```

**Example:**
```bash
# List all layers
digstore layers

# Show with sizes
digstore layers --size
```

### info

Show detailed repository information.

```bash
digstore info [OPTIONS]

OPTIONS:
  --json               Output as JSON
  --layer <HASH>       Show specific layer info
```

**Example:**
```bash
# Repository info
digstore info

# Specific layer info
digstore info --layer e3b0c44298fc
```

### gc

Garbage collect unreferenced objects.

```bash
digstore gc [OPTIONS]

OPTIONS:
  --dry-run            Show what would be removed
  --aggressive         Remove more aggressively
  --keep-history <N>   Keep N latest generations
```

**Example:**
```bash
# See what would be cleaned
digstore gc --dry-run

# Clean keeping 10 generations
digstore gc --keep-history 10
```

## Import/Export Commands

### export

Export repository or layers.

```bash
digstore export [OPTIONS] <DESTINATION>

OPTIONS:
  --layers <HASH>...   Export specific layers
  --since <HASH>       Export since root hash
  --format <FMT>       Archive format: tar|zip
```

**Example:**
```bash
# Export entire repository
digstore export backup.tar

# Export recent layers
digstore export --since abc123 recent.tar
```

### import

Import repository or layers.

```bash
digstore import [OPTIONS] <SOURCE>

OPTIONS:
  --verify             Verify all proofs
  --merge              Merge with existing
  --prefix <PATH>      Import under prefix
```

**Example:**
```bash
# Import repository
digstore import backup.tar

# Import and merge
digstore import --merge other-repo.tar
```

## Utility Commands

### diff

Show differences between versions.

```bash
digstore diff [OPTIONS] <FROM> <TO>

OPTIONS:
  --name-only          Show only file names
  --stat               Show diffstat
  --unified <N>        Context lines (default: 3)
```

**Example:**
```bash
# Diff two versions
digstore diff abc123 def456

# Show changed files only
digstore diff abc123 def456 --name-only
```

### find

Search for files in repository.

```bash
digstore find [OPTIONS] <PATTERN>

OPTIONS:
  --type <TYPE>        File type: f|d
  --size <SIZE>        File size filter
  --at <HASH>          Search at root hash
```

**Example:**
```bash
# Find all .rs files
digstore find "*.rs"

# Find large files
digstore find "*" --size +1M
```

### check

Verify repository integrity.

```bash
digstore check [OPTIONS]

OPTIONS:
  --deep               Deep verification
  --fix                Attempt to fix issues
  --layer <HASH>       Check specific layer
```

**Example:**
```bash
# Quick check
digstore check

# Deep verification
digstore check --deep
```

## Configuration Commands

### config

Get or set configuration options.

```bash
digstore config [OPTIONS] [<KEY>] [<VALUE>]

OPTIONS:
  --global             Use global config
  --list               List all settings
  --unset              Remove setting
```

**Example:**
```bash
# List configuration
digstore config --list

# Set chunk size
digstore config core.chunkSize 128

# Get setting
digstore config core.compression
```

## Interactive Mode

### shell

Enter interactive shell mode.

```bash
digstore shell [OPTIONS]

OPTIONS:
  --no-history         Disable command history
  --prompt <FMT>       Custom prompt format
```

**Example:**
```bash
# Enter shell
digstore shell

# In shell:
> add src/
> commit -m "Update source"
> exit
```

## Examples

### Common Workflows

**Initialize and add files:**
```bash
digstore init
digstore add -A
digstore commit -m "Initial commit"
```

**Retrieve historical version:**
```bash
# Find version
digstore log --oneline
# Get file from that version
digstore get urn:dig:chia:STORE_ID:ROOT_HASH/file.txt -o old_file.txt
```

**Generate and verify proof:**
```bash
# Generate proof
digstore prove important.dat -o proof.json
# Later, verify it
digstore verify proof.json
```

**Export for backup:**
```bash
# Full export
digstore export repository_backup.tar
# Incremental export
digstore export --since LAST_BACKUP_HASH incremental.tar
```
