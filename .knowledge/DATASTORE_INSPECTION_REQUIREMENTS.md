# Datastore Inspection CLI Commands Requirements

## Overview

Digstore Min needs comprehensive CLI commands to inspect and analyze datastore information. These commands should provide detailed insights into repository state, storage metrics, and historical data.

## Required CLI Commands

### 1. `digstore root` - Current Root Information

Display information about the current root commit.

#### Usage
```bash
digstore root [OPTIONS]

Options:
  --json              Output as JSON
  --verbose, -v       Show detailed information
  --hash-only         Show only the root hash
```

#### Output
```
Current Root Information
═══════════════════════════

Root Hash: a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2
Generation: 5
Timestamp: 2025-08-30 19:30:42 UTC
Layer File: a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2.dig
Layer Size: 2.4 MB
Files: 127 files (15.8 MB total)
Chunks: 543 chunks
Commit Message: "Add advanced security features"
Author: john.doe
```

### 2. `digstore history` - Root History Analysis

Show the complete root history with detailed analytics.

#### Usage
```bash
digstore history [OPTIONS]

Options:
  --json              Output as JSON
  --limit, -n NUM     Limit number of entries
  --stats             Show statistics
  --graph             Show ASCII graph
  --since DATE        Show entries since date
```

#### Output
```
Root History Analysis
════════════════════════

Total Commits: 15
Repository Age: 7 days
Average Commit Size: 1.2 MB
Growth Rate: 2.3 MB/day

Commit History:
┌─ a3f5c8d9... (Gen 5) - 2025-08-30 19:30:42
│  Files: 127 (+12), Size: 15.8 MB (+2.1 MB)
│  Message: "Add advanced security features"
│
├─ b2e4f1a8... (Gen 4) - 2025-08-30 18:15:23  
│  Files: 115 (+8), Size: 13.7 MB (+1.8 MB)
│  Message: "Implement streaming architecture"
│
└─ Initial commit (Gen 0) - 2025-08-23 10:00:00
   Files: 1, Size: 1.2 KB
   Message: "Initial repository setup"
```

### 3. `digstore store-info` - Store Metadata

Display comprehensive store metadata and configuration.

#### Usage
```bash
digstore store-info [OPTIONS]

Options:
  --json              Output as JSON
  --config            Show configuration details
  --paths             Show all paths
```

#### Output
```
Store Information
════════════════

Store ID: a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2
Format Version: 1.0
Protocol Version: 1.0
Digstore Version: 0.1.0

Paths:
  Global Store: ~/.dig/a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/
  Project Path: /home/user/my-project/
  .digstore File: /home/user/my-project/.digstore

Configuration:
  Compression: zstd
  Chunk Size: 1024 KB (avg), 512 KB (min), 4096 KB (max)
  Encryption: Enabled (URN-based scrambling)
  
Creation:
  Created: 2025-08-23 10:00:00 UTC
  Last Accessed: 2025-08-30 19:30:42 UTC
  Repository Name: my-project
```

### 4. `digstore size` - Storage Analytics

Analyze storage usage and efficiency metrics.

#### Usage
```bash
digstore size [OPTIONS]

Options:
  --json              Output as JSON
  --breakdown         Show detailed breakdown
  --efficiency        Show deduplication metrics
  --layers            Show per-layer analysis
```

#### Output
```
Storage Analytics
════════════════

Total Storage: 45.2 MB
├─ Layer Files: 42.8 MB (94.7%)
├─ Metadata: 1.8 MB (4.0%)
├─ Staging: 0.6 MB (1.3%)
└─ Overhead: 0.0 MB (0.0%)

Layer Breakdown:
  Layer 0 (metadata): 320 bytes
  Layer 1-5 (commits): 42.8 MB
  Average layer size: 8.56 MB

Efficiency Metrics:
  Deduplication Ratio: 23.4% (12.8 MB saved)
  Compression Ratio: 31.2% (19.4 MB saved)
  Storage Efficiency: 68.9%
  
File Distribution:
  Small files (<64KB): 89 files (12.3 MB)
  Medium files (64KB-10MB): 35 files (18.7 MB)  
  Large files (>10MB): 3 files (14.2 MB)
```

### 5. `digstore layers` - Layer Analysis

Detailed analysis of individual layers.

#### Usage
```bash
digstore layers [OPTIONS] [LAYER_HASH]

Options:
  --json              Output as JSON
  --list              List all layers
  --size              Show size information
  --files             Show file details
  --chunks            Show chunk details
```

#### Output
```
Layer Analysis
═════════════

Layer: a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2
Type: Full Layer
Generation: 5
Parent: b2e4f1a8c7d2e9f3a6b4c8d1e5f7a2b9c3d6e0f4a7b1c5d8e2f6a9b3c7d0e4f1
Created: 2025-08-30 19:30:42 UTC

Storage Details:
  Layer File Size: 8.56 MB
  Uncompressed Size: 12.34 MB
  Compression Ratio: 30.6%
  Scrambling: Enabled (URN-protected)

Content Summary:
  Files: 127 files
  Total File Size: 15.8 MB
  Chunks: 543 chunks
  Average Chunk Size: 29.1 KB
  
Files Added This Layer: 12 files (+2.1 MB)
Files Modified This Layer: 3 files
Files Deleted This Layer: 0 files

Top Files by Size:
  1. video/demo.mp4 (8.2 MB, 289 chunks)
  2. docs/manual.pdf (2.1 MB, 73 chunks)  
  3. assets/images/ (1.8 MB, 45 files)
```

### 6. `digstore stats` - Repository Statistics

Comprehensive repository statistics and analytics.

#### Usage
```bash
digstore stats [OPTIONS]

Options:
  --json              Output as JSON
  --detailed          Show detailed statistics
  --performance       Show performance metrics
  --security          Show security metrics
```

#### Output
```
Repository Statistics
════════════════════

Repository Overview:
  Total Commits: 15
  Repository Age: 7 days, 9 hours
  Current Generation: 5
  Active Files: 127 files
  Total Storage: 45.2 MB

Growth Metrics:
  Average Commit Size: 3.01 MB
  Growth Rate: 6.46 MB/day
  Commit Frequency: 2.14 commits/day
  File Growth: +18.1 files/day

Storage Efficiency:
  Deduplication: 23.4% space saved
  Compression: 31.2% space saved
  Total Efficiency: 47.8% space saved
  Raw Data Size: 86.1 MB
  Stored Size: 45.2 MB

Performance Metrics:
  Average Chunk Size: 29.1 KB
  Chunking Efficiency: 94.2%
  Merkle Tree Depth: 9 levels
  Proof Generation: <1ms average

Security Metrics:
  Scrambling: 100% of data protected
  URN Access Control: Active
  Legacy Access: Disabled
  Security Overhead: <2%
```

### 7. `digstore inspect` - Deep Layer Inspection

Deep inspection of layer internals for debugging and analysis.

#### Usage
```bash
digstore inspect [OPTIONS] <LAYER_HASH>

Options:
  --json              Output as JSON
  --header            Show layer header details
  --merkle            Show merkle tree information
  --chunks            Show chunk analysis
  --verify            Verify layer integrity
```

#### Output
```
Layer Deep Inspection
════════════════════

Layer: a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2

Header Information:
  Magic: DIGS
  Version: 1
  Type: Full Layer (0x01)
  Layer Number: 5
  Timestamp: 1693422642 (2025-08-30 19:30:42 UTC)
  Parent Hash: b2e4f1a8c7d2e9f3a6b4c8d1e5f7a2b9c3d6e0f4a7b1c5d8e2f6a9b3c7d0e4f1
  Files Count: 127
  Chunks Count: 543

Merkle Tree:
  Root Hash: f8e7d6c5b4a3928170695847362514a3b2c1d0e9f8a7b6c5d4e3f2a1b0c9d8e7
  Tree Depth: 9 levels
  Leaf Count: 127 files
  Proof Size: 288 bytes average

Chunk Analysis:
  Size Distribution:
    < 1KB: 23 chunks (4.2%)
    1KB-32KB: 387 chunks (71.3%)
    32KB-1MB: 128 chunks (23.6%)
    > 1MB: 5 chunks (0.9%)
  
  Deduplication:
    Unique Chunks: 489 (90.1%)
    Duplicated: 54 chunks (9.9%)
    Space Saved: 1.7 MB

Integrity Verification:
  ✓ Header checksum valid
  ✓ All chunk hashes verified
  ✓ Merkle tree consistent
  ✓ File reconstructions valid
  ✓ Scrambling integrity confirmed
```

## Implementation Requirements

### Core Functionality

#### 1. Repository State Access
- Access to current root hash and generation
- Root history traversal and analysis
- Store metadata and configuration
- Layer enumeration and details

#### 2. Storage Analytics
- Total storage size calculation
- Per-layer size analysis
- Deduplication and compression metrics
- File distribution analysis

#### 3. Performance Metrics
- Chunk size distribution
- Merkle tree statistics
- Access patterns and efficiency
- Security overhead measurement

#### 4. Security Information
- Scrambling status and coverage
- URN access control status
- Legacy access detection
- Security metric reporting

### Output Formats

#### 1. Human-Readable Format
- Colored output with visual hierarchy
- Clear section headers and separators
- Progress indicators for long operations
- Formatted numbers and sizes

#### 2. JSON Format
- Machine-readable structured data
- Complete information preservation
- Consistent field naming
- Nested structure for complex data

#### 3. Compact Formats
- Hash-only output for scripting
- Single-line summaries
- Tab-separated values for parsing
- CSV export for analysis

### Error Handling

#### 1. Repository State Errors
- No repository found (clear guidance)
- Corrupted metadata (recovery suggestions)
- Missing layers (integrity issues)
- Access permission errors

#### 2. Data Access Errors
- Layer not found (with suggestions)
- URN access failures (security errors)
- File system permission issues
- Network/storage errors

### Performance Requirements

#### 1. Response Time
- Simple queries: <100ms
- Complex analysis: <5 seconds
- Large repository stats: <30 seconds
- Real-time updates where possible

#### 2. Memory Usage
- Constant memory for most operations
- Streaming analysis for large repositories
- Efficient data structures
- Minimal memory footprint

#### 3. I/O Efficiency
- Minimal file system access
- Cached metadata where appropriate
- Batch operations for efficiency
- Progress feedback for long operations

## Command Integration

### 1. Existing Command Enhancement
- Enhance `digstore info` with more detailed output
- Add options to existing commands for inspection
- Integrate with `digstore status` for current state
- Cross-reference with `digstore log` for history

### 2. New Command Categories
- **State Commands**: `root`, `history`, `store-info`
- **Analytics Commands**: `size`, `stats`, `inspect`
- **Utility Commands**: Enhanced existing commands
- **Debug Commands**: Deep inspection and verification

### 3. Output Consistency
- Consistent color scheme across commands
- Standard format for hashes, sizes, timestamps
- Common flags and options
- Unified error message format

## Security Considerations

### 1. Access Control
- Respect URN-based access requirements
- No unauthorized data exposure
- Secure handling of sensitive information
- Audit trail for inspection commands

### 2. Information Disclosure
- Hash information is safe to display
- File paths may need sanitization
- Size information is generally safe
- Metadata should be carefully filtered

### 3. Performance Security
- No timing attacks through analysis
- Consistent response times
- No information leakage through performance
- Secure error handling

## Implementation Architecture

### 1. Command Structure
```rust
// Core inspection traits
pub trait RepositoryInspector {
    fn get_current_root(&self) -> Result<RootInfo>;
    fn get_root_history(&self) -> Result<Vec<RootEntry>>;
    fn get_storage_stats(&self) -> Result<StorageStats>;
    fn analyze_layer(&self, layer_hash: Hash) -> Result<LayerAnalysis>;
}

// Data structures for inspection
pub struct RootInfo {
    pub hash: Hash,
    pub generation: u64,
    pub timestamp: i64,
    pub layer_file_size: u64,
    pub files_count: usize,
    pub chunks_count: usize,
    pub commit_message: Option<String>,
    pub author: Option<String>,
}

pub struct StorageStats {
    pub total_size: u64,
    pub layer_files_size: u64,
    pub metadata_size: u64,
    pub staging_size: u64,
    pub deduplication_ratio: f64,
    pub compression_ratio: f64,
    pub efficiency_ratio: f64,
}

pub struct LayerAnalysis {
    pub header: LayerHeaderInfo,
    pub content: LayerContentInfo,
    pub merkle: MerkleTreeInfo,
    pub chunks: ChunkAnalysis,
    pub integrity: IntegrityStatus,
}
```

### 2. CLI Command Implementation
```rust
// Command implementations
pub mod commands {
    pub mod root;      // Current root information
    pub mod history;   // Root history analysis
    pub mod store_info; // Store metadata
    pub mod size;      // Storage analytics
    pub mod layers;    // Layer analysis
    pub mod stats;     // Repository statistics  
    pub mod inspect;   // Deep layer inspection
}
```

### 3. Output Formatting
```rust
// Formatters for different output types
pub mod formatters {
    pub mod human;     // Human-readable output
    pub mod json;      // JSON output
    pub mod compact;   // Compact formats
    pub mod table;     // Table formatting
}
```

## Testing Requirements

### 1. Command Testing
- All commands work with and without repositories
- JSON output is valid and complete
- Error handling for all failure modes
- Performance testing for large repositories

### 2. Output Validation
- Human-readable output is clear and informative
- JSON output matches schema
- Compact formats are parseable
- Color output works correctly

### 3. Integration Testing
- Commands work together consistently
- Cross-command data consistency
- End-to-end workflow testing
- Security integration testing

## Documentation Requirements

### 1. Command Documentation
- Complete usage examples
- All options and flags documented
- Output format specifications
- Error code explanations

### 2. Integration Guide
- How to use commands together
- Scripting and automation examples
- Performance optimization tips
- Troubleshooting guide

This specification ensures comprehensive datastore inspection capabilities while maintaining security, performance, and usability standards.
