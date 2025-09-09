# Test Migration Guide

This document explains how to migrate from the old disorganized test structure to the new semantic organization.

## Migration Steps

### 1. Replace Old Test Directory

```bash
# Backup old tests
mv tests tests_old

# Move new organized tests
mv tests_new tests
```

### 2. Update Cargo.toml Test Configuration

The new test structure may require updates to test configuration in `Cargo.toml` if there are any specific test settings.

### 3. Update CI/CD Configuration

Update any CI/CD scripts to use the new test organization:

```bash
# Run all tests
cargo test

# Run specific test categories
cargo test --test "unit/*"
cargo test --test "integration/*"
cargo test --test "cli/*"
cargo test --test "regression/*"
```

## Content Migration Map

| Old File | New Location | Notes |
|----------|--------------|--------|
| `basic_tests.rs` | `unit/core/types_and_hash.rs` | Core functionality |
| `chunking_tests.rs` | `unit/storage/chunking.rs` | Chunking algorithms |
| `layer_tests.rs` | `unit/storage/layers.rs` | Layer format |
| `store_tests.rs` | `unit/storage/store_management.rs` | Store operations |
| `proof_tests.rs` | `unit/proofs/proof_generation.rs` | Proof system |
| `urn_tests.rs` | `unit/urn/` | URN parsing |
| `file_operations_tests.rs` | `integration/workflows/basic_workflow.rs` | File workflows |
| `command_integration_tests.rs` | `cli/commands/` | CLI commands |
| `cli_output_validation.rs` | `cli/output/formatting_and_display.rs` | Output formatting |
| `cli_user_validation.rs` | `cli/user_experience/complete_workflows.rs` | User experience |
| `user_experience_tests.rs` | `cli/user_experience/` | User workflows |
| `bug_fix_regression_tests.rs` | `regression/bug_fixes/` | Split by bug type |
| `performance_tests.rs` | `benchmarks/performance.rs` | Performance tests |
| `zero_knowledge_integration.rs` | `integration/features/zero_knowledge.rs` | ZK features |

## Benefits After Migration

### 1. **Clear Test Scope**
- Unit tests: Fast, isolated module testing
- Integration tests: Cross-module functionality
- CLI tests: User interface and experience
- Regression tests: Bug fix protection
- Benchmarks: Performance validation

### 2. **Easy Navigation**
```bash
# Want to test storage functionality?
ls tests/unit/storage/

# Want to test CLI commands?
ls tests/cli/commands/

# Want to check regression tests?
ls tests/regression/bug_fixes/
```

### 3. **Faster Test Execution**
```bash
# Run only fast unit tests during development
cargo test --test "unit/*"

# Run integration tests before commits
cargo test --test "integration/*"

# Run full suite before releases
cargo test
```

### 4. **Better Maintainability**
- Changes to `src/storage/` → check `tests/unit/storage/`
- Changes to CLI commands → check `tests/cli/commands/`
- Bug fixes → add to `tests/regression/bug_fixes/`

### 5. **Semantic Clarity**
Test purpose is immediately clear from the directory structure and file organization.

## Validation After Migration

Run these commands to ensure the migration was successful:

```bash
# Verify all tests still pass
cargo test

# Check test coverage by category
cargo test --test "unit/*" -- --nocapture
cargo test --test "integration/*" -- --nocapture
cargo test --test "cli/*" -- --nocapture
cargo test --test "regression/*" -- --nocapture

# Ensure no tests were lost
cargo test -- --list | wc -l  # Should have similar count to before
```

## Future Test Organization

When adding new tests, follow this structure:

### New Feature Tests
1. **Unit tests**: Add to appropriate `unit/module/` directory
2. **Integration tests**: Add to `integration/features/`
3. **CLI tests**: Add to `cli/commands/` or `cli/user_experience/`

### Bug Fix Tests
1. **Always add regression test**: `regression/bug_fixes/descriptive_name.rs`
2. **Include reproduction case**: Test the exact scenario that was broken
3. **Document the fix**: Comments explaining what was fixed and why

### Performance Tests
1. **Add to benchmarks**: `benchmarks/performance.rs` or new file
2. **Set reasonable expectations**: Don't make tests too strict
3. **Include performance metrics**: Log timing information for analysis
