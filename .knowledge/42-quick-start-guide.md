# Digstore Min - Quick Start Implementation Guide

## 🚀 Day 1: Get Started Immediately

### Step 1: Initialize the Rust Project (15 minutes)
```bash
# Create the project
cargo new digstore_min --bin
cd digstore_min

# Initialize git repository
git init
git add .
git commit -m "Initial commit"
```

### Step 2: Set Up Dependencies (10 minutes)
Edit `Cargo.toml`:
```toml
[package]
name = "digstore_min"
version = "0.1.0"
edition = "2021"

[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
hex = "0.4"
thiserror = "1"
anyhow = "1"
directories = "5"
uuid = { version = "1", features = ["v4"] }
chrono = "0.4"

[dev-dependencies]
tempfile = "3"
```

### Step 3: Create Project Structure (20 minutes)
```bash
# Create directory structure
mkdir -p src/{core,storage,proofs,urn,cli/commands}
mkdir -p tests benches digstore_min/.knowledge

# Create module files
touch src/lib.rs
touch src/core/{mod.rs,types.rs,hash.rs,error.rs}
touch src/storage/{mod.rs,store.rs,layer.rs,chunk.rs}
touch src/proofs/{mod.rs,merkle.rs,proof.rs}
touch src/urn/{mod.rs,parser.rs}
touch src/cli/{mod.rs,commands/mod.rs}
```

### Step 4: Implement Core Types First (1 hour)

Start with `src/core/types.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::fmt;

/// 32-byte SHA-256 hash
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Hash(bytes)
    }
    
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// Store identifier
pub type StoreId = Hash;

/// Layer types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerType {
    Full,
    Delta,
}
```

### Step 5: Create Basic CLI Structure (30 minutes)

Edit `src/main.rs`:
```rust
use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new repository
    Init,
    
    /// Add files to the repository
    Add {
        /// Files or directories to add
        paths: Vec<String>,
        
        /// Recursively add directories
        #[arg(short, long)]
        recursive: bool,
    },
    
    /// Create a new commit
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: String,
    },
    
    /// Retrieve files from the repository
    Get {
        /// Path or URN to retrieve
        path: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Init => {
            println!("Initializing new repository...");
            // TODO: Implement
            Ok(())
        }
        Commands::Add { paths, recursive } => {
            println!("Adding files: {:?} (recursive: {})", paths, recursive);
            // TODO: Implement
            Ok(())
        }
        Commands::Commit { message } => {
            println!("Creating commit: {}", message);
            // TODO: Implement
            Ok(())
        }
        Commands::Get { path } => {
            println!("Retrieving: {}", path);
            // TODO: Implement
            Ok(())
        }
    }
}
```

### Step 6: Run and Test (5 minutes)
```bash
# Build the project
cargo build

# Test the CLI
cargo run -- --help
cargo run -- init
cargo run -- add README.md
cargo run -- commit -m "Test commit"

# Run tests (even though empty)
cargo test
```

## 📋 Next Priority Tasks

### Week 1 Focus: Core Foundation
1. **Day 2**: Implement hash utilities and error types
2. **Day 3**: Create store initialization and management
3. **Day 4**: Implement basic layer format (without chunks initially)
4. **Day 5**: Add simple file operations (add/get without chunking)

### Week 2 Focus: Storage Engine
1. **Day 1-2**: Implement content-defined chunking
2. **Day 3-4**: Complete layer format with chunk support
3. **Day 5**: Add staging area functionality

### Week 3 Focus: Merkle Trees & URNs
1. **Day 1-2**: Implement merkle tree construction
2. **Day 3**: Add proof generation
3. **Day 4-5**: Implement URN parsing and resolution

## 🎯 MVP Milestones

### Milestone 1: Basic Operations (Week 1)
- [ ] `digstore init` creates a repository
- [ ] `digstore add file.txt` stages a file
- [ ] `digstore commit -m "message"` creates a commit
- [ ] `digstore get file.txt` retrieves the file

### Milestone 2: Directory Support (Week 2)
- [ ] `digstore add -r src/` adds directories
- [ ] Chunking works for large files
- [ ] Basic deduplication functional

### Milestone 3: Proofs & URNs (Week 3)
- [ ] `digstore prove file.txt` generates proof
- [ ] `digstore verify proof.json` validates proof
- [ ] URN-based retrieval works

## 💡 Development Tips

1. **Start Simple**: Get init/add/commit/get working with whole files first
2. **Iterate**: Add chunking, proofs, and URNs incrementally
3. **Test Early**: Write tests for each component as you build
4. **Use Examples**: Create example files to test with:
   ```bash
   echo "Hello, Digstore!" > test.txt
   mkdir -p testdir/{sub1,sub2}
   echo "Content 1" > testdir/sub1/file1.txt
   echo "Content 2" > testdir/sub2/file2.txt
   ```

5. **Debug Output**: Use `env_logger` for debugging:
   ```toml
   [dependencies]
   env_logger = "0.10"
   ```
   ```rust
   // In main()
   env_logger::init();
   ```
   Run with: `RUST_LOG=debug cargo run -- init`

## 🔧 Useful Commands During Development

```bash
# Watch for changes and rebuild
cargo install cargo-watch
cargo watch -x check -x test -x run

# Format code
cargo fmt

# Check for common mistakes
cargo clippy

# Generate documentation
cargo doc --open

# Run specific test
cargo test test_name

# Run with verbose output
cargo run -- -v command
```

## 📚 Resources

- [Rabin Fingerprinting](https://en.wikipedia.org/wiki/Rabin_fingerprint)
- [Merkle Trees](https://en.wikipedia.org/wiki/Merkle_tree)
- [Content-Defined Chunking](https://restic.readthedocs.io/en/stable/100_references.html#chunking)
- [Git Internals](https://git-scm.com/book/en/v2/Git-Internals-Git-Objects)

---

**Remember**: Focus on getting a working MVP first. You can always optimize and add features later!
