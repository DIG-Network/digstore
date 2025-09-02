# Contributing to Digstore

Thank you for your interest in contributing to Digstore!

## Code Formatting

We use `rustfmt` to maintain consistent code formatting across the project. 

### Before Submitting

**Always run `cargo fmt` before committing your changes:**

```bash
cargo fmt
```

### CI Formatting Check

Our CI pipeline runs `cargo fmt -- --check` on all pull requests. If your code doesn't pass the formatting check, the CI will fail.

### Setting Up Git Hooks (Optional)

To automatically check formatting before each commit:

```bash
# Make the hook executable
chmod +x .githooks/pre-commit

# Configure git to use our hooks directory
git config core.hooksPath .githooks
```

This will prevent commits with formatting issues.

### Editor Integration

We recommend configuring your editor to run `rustfmt` on save:

- **VS Code**: Install the `rust-analyzer` extension and enable format on save
- **IntelliJ/CLion**: Enable the Rustfmt integration in Rust plugin settings
- **Vim/Neovim**: Use `rust.vim` or configure your LSP to format on save

## Running Tests

Before submitting a PR, ensure all tests pass:

```bash
# Run all tests
cargo test

# Run tests with all features
cargo test --all-features

# Run specific test
cargo test test_name
```

## Code Style Guidelines

- Follow Rust naming conventions
- Write descriptive commit messages
- Add tests for new functionality
- Update documentation as needed
- Keep PRs focused on a single change

## Questions?

Feel free to open an issue if you have any questions about contributing!
