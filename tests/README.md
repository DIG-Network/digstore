# Digstore Test Organization

This directory contains a semantically organized test suite for Digstore Min, structured to mirror the codebase organization and provide clear separation of concerns.

## Directory Structure

```
tests_new/
├── unit/                    # Unit tests for individual modules
│   ├── core/               # Core functionality tests
│   │   ├── types_and_hash.rs      # Hash functions, basic types
│   │   └── error_handling.rs      # Error types and propagation
│   ├── storage/            # Storage layer tests
│   │   ├── chunking.rs            # Content-defined chunking
│   │   ├── store_management.rs    # Store init/open operations
│   │   ├── layers.rs              # Layer format and operations
│   │   ├── archive.rs             # .dig archive format
│   │   └── staging.rs             # Binary staging area
│   ├── crypto/             # Cryptographic operations
│   ├── proofs/             # Proof generation and verification
│   ├── security/           # Security and access control
│   ├── config/             # Configuration management
│   ├── ignore/             # .digignore functionality
│   ├── urn/                # URN parsing and resolution
│   ├── wallet/             # Wallet integration
│   └── update/             # Update and version management
├── integration/             # Integration tests across modules
│   ├── workflows/          # Complete workflow tests
│   │   └── basic_workflow.rs     # Init → Add → Commit → Get
│   └── features/           # Feature-specific integration
│       ├── zero_knowledge.rs     # Zero-knowledge properties
│       └── encryption.rs         # End-to-end encryption
├── cli/                    # CLI command and interface tests
│   ├── commands/           # Command-specific tests
│   │   ├── core_commands.rs      # init, add, commit, status
│   │   ├── data_access.rs        # get, cat
│   │   └── staging_commands.rs   # staged list/diff/clear
│   ├── output/             # Output format and display tests
│   │   └── formatting_and_display.rs
│   └── user_experience/    # User workflow and UX tests
│       └── complete_workflows.rs
├── regression/             # Regression tests for fixed bugs
│   ├── bug_fixes/          # Specific bug fix tests
│   │   ├── windows_file_mapping.rs  # Error 1224 fixes
│   │   ├── json_serialization.rs    # Layer 0 corruption fix
│   │   ├── user_configuration.rs    # Optional email fix
│   │   └── version_management.rs    # Version system fix
│   └── compatibility/      # Compatibility regression tests
├── benchmarks/             # Performance and benchmark tests
└── README.md              # This file
```

## Test Categories

### 1. Unit Tests (`unit/`)
- **Purpose**: Test individual modules in isolation
- **Scope**: Single functions, methods, or small components
- **Organization**: Mirrors `src/` directory structure
- **Examples**: Hash functions, chunking algorithms, layer serialization

### 2. Integration Tests (`integration/`)
- **Purpose**: Test interaction between multiple modules
- **Scope**: Cross-module functionality and workflows
- **Organization**: By feature or workflow
- **Examples**: Complete init-to-commit workflows, encryption end-to-end

### 3. CLI Tests (`cli/`)
- **Purpose**: Test command-line interface and user interaction
- **Scope**: CLI commands, output formatting, user experience
- **Organization**: By command groups and UX concerns
- **Examples**: Command execution, help systems, error messages

### 4. Regression Tests (`regression/`)
- **Purpose**: Prevent previously fixed bugs from reoccurring
- **Scope**: Specific bug fixes and compatibility issues
- **Organization**: By bug category and compatibility concerns
- **Examples**: Windows file mapping fixes, JSON serialization fixes

### 5. Benchmarks (`benchmarks/`)
- **Purpose**: Performance testing and optimization validation
- **Scope**: Performance-critical operations
- **Organization**: By performance area
- **Examples**: Large file handling, batch operations

## Test Naming Conventions

### File Names
- Use descriptive names that indicate the functionality being tested
- Group related tests in the same file
- Use `snake_case` for file names

### Test Function Names
- Start with `test_` prefix
- Use descriptive names that explain what is being tested
- Include the expected behavior or outcome

### Examples
```rust
// Good
#[test]
fn test_hash_deterministic_output() { ... }

#[test]
fn test_chunk_reconstruction_accuracy() { ... }

#[test]
fn test_windows_file_mapping_no_error_1224() { ... }

// Avoid
#[test]
fn test1() { ... }

#[test]
fn test_stuff() { ... }
```

## Test Organization Principles

### 1. **Single Responsibility**
Each test file focuses on a specific module or feature area.

### 2. **Clear Dependencies**
- Unit tests: Minimal dependencies, test modules in isolation
- Integration tests: Test realistic module interactions
- CLI tests: Test user-facing behavior

### 3. **Semantic Grouping**
Tests are grouped by what they test, not by how they test it.

### 4. **Regression Protection**
All bug fixes have corresponding regression tests to prevent reoccurrence.

### 5. **Performance Awareness**
- Unit tests: Fast execution (< 1s each)
- Integration tests: Reasonable execution (< 10s each)
- Benchmarks: Separate category for longer-running performance tests

## Running Tests

### Run all tests
```bash
cargo test
```

### Run specific categories
```bash
# Unit tests only
cargo test --test "unit/*"

# Integration tests only
cargo test --test "integration/*"

# CLI tests only
cargo test --test "cli/*"

# Regression tests only
cargo test --test "regression/*"
```

### Run specific modules
```bash
# Core functionality
cargo test --test "unit/core/*"

# Storage layer
cargo test --test "unit/storage/*"

# Specific bug fixes
cargo test --test "regression/bug_fixes/windows_file_mapping"
```

## Migration from Old Tests

The old test files in the `tests/` directory have been reorganized as follows:

- `basic_tests.rs` → `unit/core/types_and_hash.rs`
- `store_tests.rs` → `unit/storage/store_management.rs`
- `chunking_tests.rs` → `unit/storage/chunking.rs`
- `layer_tests.rs` → `unit/storage/layers.rs`
- `bug_fix_regression_tests.rs` → `regression/bug_fixes/` (split by bug type)
- `cli_*_tests.rs` → `cli/` (organized by command group)
- User experience tests → `cli/user_experience/`
- Performance tests → `benchmarks/`

## Benefits of This Organization

1. **Easier Navigation**: Find tests related to specific modules quickly
2. **Better Maintainability**: Changes to code modules have corresponding test locations
3. **Clear Test Scope**: Unit vs integration vs CLI tests are clearly separated
4. **Regression Protection**: Bug fixes are tracked with dedicated regression tests
5. **Performance Isolation**: Slow benchmarks don't affect fast unit test runs
6. **Semantic Clarity**: Test purpose is clear from directory and file structure
