# CLI Commands Reference (Enhanced)

## Overview

Digstore Min provides a polished command-line interface with real-time progress feedback, full streaming support, and seamless Unix pipe integration. All commands are designed for both interactive use and scripting.

## Global Options

```bash
digstore [OPTIONS] <COMMAND>

OPTIONS:
  -v, --verbose     Enable verbose output
  -q, --quiet       Suppress non-error output
  --no-progress     Disable progress bars
  --color <WHEN>    Color output: auto|always|never (default: auto)
  --store <PATH>    Path to store directory (default: current directory)
  --version         Print version information
  -h, --help        Print help information
```

## Core Commands

### init

Initialize a new repository with visual feedback.

```bash
digstore init [OPTIONS]

OPTIONS:
  --store-id <ID>   Use specific store ID (default: generate random)
  --no-compression  Disable compression
  --chunk-size <N>  Average chunk size in KB (default: 1024)
```

**Progress Display:**
```
Initializing repository...
✓ Created store directory: ~/.dig/a3f5c8d9e2b1f4a6
✓ Generated store ID: a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4
✓ Created .digstore file
✓ Initialized empty repository

Repository initialized successfully!
Store ID: a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4
Location: ~/.dig/a3f5c8d9e2b1f4a6
```

### add

Add files with real-time progress and chunking feedback.

```bash
digstore add [OPTIONS] <PATH>...

OPTIONS:
  -r, --recursive   Add directories recursively
  -A, --all         Add all files in the repository
  -f, --force       Force add ignored files
  --dry-run         Show what would be added
  --from-stdin      Read file list from stdin
```

**Progress Display:**
```
Scanning files...
✓ Found 1,234 files (156.7 MB)

Adding files to staging:
  current: src/main.rs (45.2 KB)
  [████████████████░░░░░░░░] 456/1,234 files | 37% | 23.4 MB/s | ETA: 00:00:42
  
✓ Added 1,234 files to staging
  Total size: 156.7 MB
  New content: 89.3 MB
  Deduplicated: 67.4 MB (43%)
```

**Streaming from stdin:**
```bash
# Add files listed in stdin
find . -name "*.rs" | digstore add --from-stdin

# Add tar archive contents directly
tar cf - src/ | digstore add --from-stdin --name "src.tar"
```

### commit

Create commits with detailed progress for each stage.

```bash
digstore commit [OPTIONS]

OPTIONS:
  -m, --message <MSG>   Commit message
  --full                Create full layer (not delta)
  --author <NAME>       Set author name
  --date <DATE>         Override commit date
  -e, --edit            Open editor for message
```

**Progress Display:**
```
Creating commit...

Stage 1/4: Processing files
  current: src/lib.rs
  [████████████░░░░░░░░░░░░] 234/456 files | 51% | 67.8 MB/s
  
Stage 2/4: Computing chunks
  [████████████████████░░░░] 1,234/1,567 chunks | 78% | 234.5 MB/s
  
Stage 3/4: Building merkle tree
  [████████████████████████] 1,567 nodes | 100%
  
Stage 4/4: Writing layer
  [████████████████████████] 45.6 MB | 100% | 156.7 MB/s

✓ Commit successful!

Commit: e3b0c44298fc1c149afbf4c8996fb92427ae41e4
Author: John Doe
Date: 2024-01-15 10:30:45
Files: 456 (234 modified, 222 new)
Size: 45.6 MB (delta from parent)
Chunks: 1,567 total, 890 new
Deduplication: 43.2% saved
```

### get

Retrieve content with streaming support and progress indication.

```bash
digstore get [OPTIONS] <URN_OR_PATH>

OPTIONS:
  -o, --output <PATH>   Write to file instead of stdout
  --verify              Verify with merkle proof while retrieving
  --metadata            Include metadata in output
  --at <HASH>           Retrieve at specific root hash
  --progress            Force show progress even when piping
```

**Progress Display (to file):**
```
Retrieving: /data/large_file.bin
[████████████░░░░░░░░░░░░] 2.3 GB/6.7 GB | 34% | 125.3 MB/s | ETA: 00:00:35

✓ Retrieved successfully to: output.bin
  Size: 6.7 GB
  Hash: a3f5c8d9e2b1f4a6
  Verified: ✓
```

**Piping Support:**
```bash
# Pipe to another process (progress auto-disabled)
digstore get /data/file.json | jq '.items[]'

# Force progress even when piping
digstore get /data/large.bin --progress | pv > output.bin

# Stream specific byte range
digstore get "/video.mp4#bytes=0-1048576" | ffmpeg -i pipe: -f mp3 pipe:

# Save to file
digstore get /data/archive.tar.gz -o backup.tar.gz
```

### cat

Display file contents with automatic pager detection.

```bash
digstore cat [OPTIONS] <PATH>

OPTIONS:
  --at <HASH>      Show at specific root hash
  -n, --number     Number all output lines
  --no-pager       Don't use pager for long output
  --bytes <RANGE>  Display specific byte range
```

**Smart Output:**
```bash
# Auto-detects terminal vs pipe
digstore cat README.md          # Uses pager if > terminal height
digstore cat README.md | grep TODO  # No pager when piping

# Byte range support with progress
digstore cat "large.log#bytes=1000000-2000000"
[████████████████████████] 1.0 MB | 100% | 234.5 MB/s
```

### status

Show repository status with visual formatting.

```bash
digstore status [OPTIONS]

OPTIONS:
  -s, --short      Show short format
  --porcelain      Machine-readable output
  --show-chunks    Display chunk statistics
```

**Rich Output:**
```
Repository Status
═════════════════

Current commit: e3b0c44298fc
Store ID: a3f5c8d9e2b1f4a6

Changes to be committed:
  new file:   src/new_module.rs     12.3 KB
  modified:   src/main.rs           +45 -23 lines
  deleted:    old/legacy.rs         -5.6 KB

Untracked files:
  tests/test_new.rs
  .env.local

Summary:
  Files staged: 3 (2 modified, 1 new)
  Total changes: +57.8 KB
  Chunks affected: 23
```

## Retrieval Commands

### extract

Extract files with progress tracking.

```bash
digstore extract [OPTIONS] <SOURCE> <DESTINATION>

OPTIONS:
  --at <HASH>           Extract at specific root hash
  -f, --force          Overwrite existing files
  --preserve-mtime     Preserve modification times
  --strip-path <N>     Strip N path components
```

**Progress Display:**
```
Extracting files...
  current: src/modules/auth.rs
  [██████████░░░░░░░░░░░░░░] 123/456 files | 27% | 45.6 MB/s | ETA: 00:01:23
  
✓ Extracted 456 files (234.5 MB) to ./output/
```

## Proof Commands

### prove

Generate proofs with progress indication.

```bash
digstore prove [OPTIONS] <TARGET>

OPTIONS:
  -o, --output <PATH>   Write proof to file (default: stdout)
  --format <FMT>       Output format: json|binary|text
  --at <HASH>          Prove at specific root hash
  --bytes <RANGE>      Prove specific byte range
  --compact            Generate compact proof
```

**Progress Display:**
```
Generating merkle proof...
  Building proof path...
  [████████████████████████] 12 nodes | 100%
  
✓ Proof generated successfully
  Target: /data/important.pdf
  Root: e3b0c44298fc1c149afbf4c8996fb92427ae41e4
  Proof size: 1.2 KB
  Nodes: 12
```

**Output Options:**
```bash
# Output to stdout (for piping)
digstore prove /data/file.txt | base64

# Save to file
digstore prove /data/file.txt -o proof.json

# Compact binary format
digstore prove /data/file.txt --format binary -o proof.bin
```

### verify

Verify proofs with detailed feedback.

```bash
digstore verify [OPTIONS] <PROOF>

OPTIONS:
  --target <HASH>      Expected target hash
  --root <HASH>        Expected root hash
  --verbose            Show detailed verification steps
  --from-stdin         Read proof from stdin
```

**Progress Display:**
```
Verifying proof...
  ✓ Proof format valid
  ✓ Target hash matches
  ✓ Root hash verified
  ✓ Merkle path valid (12 nodes)
  
✓ Proof verification PASSED

Target: /data/important.pdf
Root: e3b0c44298fc1c149afbf4c8996fb92427ae41e4
Timestamp: 2024-01-15 10:30:45 UTC
```

## Streaming Examples

### Large File Handling
```bash
# Stream large file through compression
digstore get /backups/database.sql | gzip > db_backup.sql.gz

# Process video while downloading
digstore get /media/video.mp4 | ffmpeg -i pipe: -c:v libx264 pipe: | digstore add --from-stdin --name "compressed.mp4"

# Real-time log analysis
digstore cat /logs/app.log --follow | grep ERROR | tee errors.log
```

### Pipeline Integration
```bash
# Complex pipeline with progress
digstore get /data/records.jsonl --progress 2>&1 | \
  tee >(jq -r '.id' > ids.txt) | \
  jq 'select(.status == "active")' | \
  digstore add --from-stdin --name "active_records.jsonl"

# Backup with verification
digstore export --since $LAST_BACKUP | \
  tee >(sha256sum > backup.sha256) | \
  ssh backup@server "cat > /backups/$(date +%Y%m%d).tar"
```

## Performance Considerations

### Chunk Size Optimization
```bash
# Test different chunk sizes
for size in 512 1024 2048 4096; do
  echo "Testing chunk size: ${size}KB"
  time digstore add large_file.bin --chunk-size $size --dry-run
done
```

### Parallel Processing
```bash
# Enable parallel chunking (auto-detected by default)
digstore add -r /large/dataset --parallel 8

# Monitor resource usage
digstore commit -m "Large commit" --verbose 2>&1 | tee commit.log
```

## Interactive Features

### Shell Mode
```bash
digstore shell

digstore> add src/
  ✓ Added 123 files

digstore> status -s
  M src/main.rs
  A src/new.rs
  
digstore> commit -m "Update source"
  [████████████████████████] 100% | Complete
  ✓ Commit: abc123def

digstore> exit
```

### Auto-completion
```bash
# Generate completion scripts
digstore completions bash > /etc/bash_completion.d/digstore
digstore completions zsh > /usr/share/zsh/site-functions/_digstore
digstore completions fish > ~/.config/fish/completions/digstore.fish
```

## Error Handling

All commands provide clear error messages with recovery suggestions:

```
✗ Error: Cannot retrieve file
  File not found: /data/missing.txt
  
  Suggestions:
  • Check if the file exists with: digstore ls /data/
  • Verify you're using the correct root hash
  • Try: digstore find "*missing*" to search for similar files
```

## Summary

The enhanced CLI provides:
1. **Rich Progress Feedback** - Every operation shows clear progress
2. **Full Streaming Support** - Handle files of any size efficiently  
3. **Pipe Integration** - Seamless Unix pipeline compatibility
4. **Smart Output** - Automatic detection of terminal vs pipe context
5. **Professional Polish** - Consistent, beautiful output formatting

This ensures Digstore Min provides a modern, efficient CLI experience that users expect from professional tools.
