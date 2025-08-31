# .digignore File Requirements

## Overview

Digstore Min must support a `.digignore` file that works exactly like `.gitignore` to exclude files and patterns from being added to the repository. This provides users with familiar and powerful file filtering capabilities.

## Core Requirements

### 1. .digignore File Format

The `.digignore` file must use the exact same syntax and semantics as `.gitignore`:

#### Pattern Syntax
- **Blank lines**: Ignored (can be used for spacing)
- **Comments**: Lines starting with `#` are ignored
- **Negation**: Lines starting with `!` negate the pattern (include files that would otherwise be ignored)
- **Directory separator**: Use `/` for path separators on all platforms
- **Wildcards**: Support `*`, `?`, and `**` glob patterns
- **Anchoring**: Patterns with `/` are anchored to specific directory levels

#### Pattern Examples
```
# Ignore all .tmp files
*.tmp

# Ignore build directories
build/
target/

# Ignore all files in any node_modules directory
node_modules/

# Ignore all .log files in any directory
**/*.log

# Ignore specific file in root
config.local.json

# Ignore all files in cache directories at any level
**/cache/

# But include important cache files
!**/cache/important.json

# Ignore all files in .git directory
.git/

# Ignore OS-specific files
.DS_Store
Thumbs.db
desktop.ini

# Ignore IDE files
.vscode/
.idea/
*.swp
*.swo
*~

# Ignore temporary files
*.tmp
*.temp
*.bak
*.backup
```

### 2. File Location and Hierarchy

#### Primary .digignore Location
- **Root .digignore**: Located in the repository root (same directory as `.digstore`)
- **Priority**: Highest priority, applies to entire repository

#### Nested .digignore Files
- **Directory-specific**: `.digignore` files in subdirectories
- **Scope**: Apply only to their directory and subdirectories
- **Inheritance**: Rules from parent directories apply unless overridden

#### Search Order
1. Check file against current directory `.digignore`
2. Check against parent directory `.digignore` (if exists)
3. Continue up directory tree to repository root
4. Apply repository root `.digignore`

### 3. Integration with digstore add Command

#### Automatic Filtering
- **All add operations**: Filter files through `.digignore` rules
- **Recursive operations**: Apply filtering during directory traversal
- **Pattern matching**: Use efficient glob pattern matching

#### Command Behavior
```bash
# These commands must respect .digignore
digstore add file.txt              # Single file (check if ignored)
digstore add directory/            # Directory (filter contents)
digstore add -r directory/         # Recursive (filter all files)
digstore add -A                    # Add all (filter everything)
digstore add .                     # Current directory (filter contents)
```

#### Override Options
```bash
# Force add ignored files
digstore add --force file.txt      # Add even if ignored
digstore add -f ignored.tmp        # Short form

# Show what would be ignored
digstore add --dry-run -A          # Preview without adding
digstore add -n directory/         # Show filtered results
```

### 4. Progress Bar for digstore add -A

#### Requirements for Progress Display
- **File Discovery Phase**: Show progress while scanning directories
- **Filtering Phase**: Show progress while applying `.digignore` rules  
- **Processing Phase**: Show progress while adding filtered files
- **Real-time Updates**: Update progress as files are processed
- **Summary Statistics**: Show final counts and filtering results

#### Progress Bar Specifications
```
Phase 1: Discovering files...
█████████████████████████████████████████████████████████████ 100% (1,247 files found)

Phase 2: Applying .digignore filters...
█████████████████████████████████████████████████████████████ 100% (1,247 files → 892 files)

Phase 3: Adding files to repository...
█████████████████████████████████████████████████████████████ 100% (892/892 files added)

✓ Added 892 files (355 ignored by .digignore)
  • Total discovered: 1,247 files
  • Filtered out: 355 files (28.5%)
  • Successfully added: 892 files
  • Processing time: 2.3 seconds
```

### 5. Error Handling and Edge Cases

#### File Access Errors
- **Permission denied**: Skip file with warning, continue processing
- **File not found**: Skip file (may have been deleted during scan)
- **Symlink handling**: Follow symlinks but detect cycles

#### .digignore File Errors
- **Invalid patterns**: Log warning, skip invalid pattern, continue
- **Missing .digignore**: No filtering applied (normal operation)
- **Malformed .digignore**: Log warnings for problematic lines

#### Pattern Matching Edge Cases
- **Case sensitivity**: Follow filesystem case sensitivity rules
- **Unicode handling**: Proper UTF-8 support for international filenames
- **Special characters**: Handle spaces, quotes, and special characters in filenames

### 6. Performance Requirements

#### Efficiency Targets
- **Large repositories**: Handle 100,000+ files efficiently
- **Pattern matching**: <1ms per file for pattern evaluation
- **Memory usage**: <100MB for pattern matching data structures
- **Caching**: Cache compiled patterns for reuse

#### Optimization Strategies
- **Early filtering**: Apply filters during directory traversal
- **Pattern compilation**: Compile glob patterns once, reuse many times
- **Directory pruning**: Skip entire directories if pattern matches
- **Batch processing**: Group files for efficient processing

### 7. Integration with Existing Systems

#### Staging System Integration
- **Pre-staging filtering**: Apply `.digignore` before staging files
- **Status reporting**: Show ignored files in status (optional flag)
- **Conflict resolution**: Handle cases where staged files become ignored

#### CLI Integration
- **Consistent behavior**: All commands that add files respect `.digignore`
- **Verbose output**: Option to show ignored files and reasons
- **Help and documentation**: Clear documentation of pattern syntax

## Implementation Architecture

### Core Components

#### 1. DigignoreParser
```rust
pub struct DigignoreParser {
    patterns: Vec<CompiledPattern>,
    negation_patterns: Vec<CompiledPattern>,
}

impl DigignoreParser {
    pub fn from_file(path: &Path) -> Result<Self>;
    pub fn is_ignored(&self, file_path: &Path, is_dir: bool) -> bool;
    pub fn add_pattern(&mut self, pattern: &str) -> Result<()>;
}
```

#### 2. IgnoreChecker
```rust
pub struct IgnoreChecker {
    parsers: Vec<(PathBuf, DigignoreParser)>, // (directory, parser)
}

impl IgnoreChecker {
    pub fn new(repo_root: &Path) -> Result<Self>;
    pub fn is_ignored(&self, file_path: &Path) -> bool;
    pub fn reload(&mut self) -> Result<()>;
}
```

#### 3. FilteredFileScanner
```rust
pub struct FilteredFileScanner {
    ignore_checker: IgnoreChecker,
    progress_callback: Option<Box<dyn Fn(ScanProgress)>>,
}

pub struct ScanProgress {
    pub phase: ScanPhase,
    pub files_discovered: usize,
    pub files_filtered: usize,
    pub current_file: Option<PathBuf>,
}

pub enum ScanPhase {
    Discovery,
    Filtering,
    Processing,
}
```

### Integration Points

#### Store Integration
- Update `Store::add_directory()` to use `FilteredFileScanner`
- Add `Store::add_all()` method for `digstore add -A`
- Integrate progress callbacks with CLI progress bars

#### CLI Integration
- Add `--force` flag to bypass `.digignore`
- Add `--show-ignored` flag to display filtered files
- Update help text with `.digignore` documentation

## Testing Requirements

### Unit Tests
- Pattern parsing and compilation
- Pattern matching against various file paths
- Negation pattern handling
- Directory hierarchy handling

### Integration Tests
- End-to-end file filtering during add operations
- Multiple `.digignore` files in directory hierarchy
- Performance testing with large file sets
- Error handling and edge cases

### Property-Based Tests
- Pattern matching consistency
- Directory traversal correctness
- Performance characteristics

## Documentation Requirements

### User Documentation
- `.digignore` syntax reference (identical to `.gitignore`)
- Examples and common patterns
- Performance tips for large repositories

### Developer Documentation
- API documentation for ignore system
- Integration examples
- Performance characteristics and limitations

## Success Criteria

### Functional Requirements
- ✅ Exact `.gitignore` syntax compatibility
- ✅ Hierarchical `.digignore` file support
- ✅ Efficient pattern matching performance
- ✅ Progress bars for `digstore add -A`
- ✅ Comprehensive error handling

### Performance Requirements
- ✅ Handle 100,000+ files without performance degradation
- ✅ <1ms pattern matching per file
- ✅ <100MB memory usage for pattern data
- ✅ Real-time progress updates during scanning

### User Experience Requirements
- ✅ Familiar `.gitignore` syntax (zero learning curve)
- ✅ Clear progress indication for long operations
- ✅ Helpful error messages and warnings
- ✅ Consistent behavior across all add operations
