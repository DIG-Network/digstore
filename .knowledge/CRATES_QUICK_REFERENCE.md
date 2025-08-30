# Rust Crates Quick Reference for Digstore Min

## Essential Crates Cheat Sheet

### 🔧 Core Building Blocks

| Task | Crate | One-Liner Usage |
|------|-------|-----------------|
| **Chunking** | `fastcdc` | `FastCDC::new(&data, 512*1024, 1024*1024, 4*1024*1024)` |
| **Merkle Trees** | `rs_merkle` | `MerkleTree::<Sha256>::from_leaves(&hashes)` |
| **CLI Parsing** | `clap` | `#[derive(Parser)] struct Cli { #[command(subcommand)] cmd: Commands }` |
| **Progress Bars** | `indicatif` | `ProgressBar::new(total).set_style(ProgressStyle::default_bar())` |
| **Binary Format** | `bincode` | `bincode::serialize(&data)?` / `bincode::deserialize(&bytes)?` |
| **Path Walking** | `walkdir` | `WalkDir::new(path).into_iter().filter_map(Result::ok)` |

### 📦 Complete Examples

#### 1. Chunk a File (5 lines)
```rust
use fastcdc::v2020::FastCDC;

let data = std::fs::read("file.txt")?;
let chunker = FastCDC::new(&data, 512_000, 1_048_576, 4_194_304);
let chunks: Vec<_> = chunker.collect();
```

#### 2. Generate Merkle Proof (6 lines)
```rust
use rs_merkle::{MerkleTree, algorithms::Sha256};

let leaves: Vec<[u8; 32]> = file_hashes.iter().map(|h| h.to_bytes()).collect();
let tree = MerkleTree::<Sha256>::from_leaves(&leaves);
let proof = tree.proof(&[index]);
let verified = proof.verify(tree.root().unwrap(), &[index], &[leaves[index]], tree.leaves_len());
```

#### 3. Parse URN with nom (10 lines)
```rust
use nom::{IResult, bytes::complete::tag, character::complete::alphanumeric1};

fn parse_store_id(input: &str) -> IResult<&str, &str> {
    let (input, _) = tag("urn:dig:chia:")(input)?;
    alphanumeric1(input)
}

let urn = "urn:dig:chia:abc123def456";
let (_, store_id) = parse_store_id(urn).unwrap();
```

#### 4. Show Progress (4 lines)
```rust
use indicatif::ProgressBar;

let pb = ProgressBar::new(file_size);
pb.set_message("Processing file");
pb.inc(bytes_processed);
pb.finish_with_message("Done!");
```

#### 5. Pretty Table Output (8 lines)
```rust
use tabled::{Table, Tabled};

#[derive(Tabled)]
struct Row { file: String, size: u64, status: &'static str }

let data = vec![Row { file: "test.txt".into(), size: 1024, status: "Added" }];
let table = Table::new(data);
println!("{}", table);
```

### 🚀 Performance Patterns

#### Parallel Processing with Rayon
```rust
use rayon::prelude::*;

let results: Vec<_> = files
    .par_iter()
    .map(|file| process_file(file))
    .collect();
```

#### Memory-Mapped Files
```rust
use memmap2::MmapOptions;
use std::fs::File;

let file = File::open("large.bin")?;
let mmap = unsafe { MmapOptions::new().map(&file)? };
// Access bytes directly: &mmap[0..1024]
```

#### Concurrent Cache with DashMap
```rust
use dashmap::DashMap;

let cache = DashMap::new();
cache.insert(hash, chunk_data);
if let Some(data) = cache.get(&hash) { /* use data */ }
```

### 🎨 UI Enhancement Patterns

#### Colored Output
```rust
use colored::Colorize;

println!("{} {}", "✓".green(), "File added successfully".white());
println!("{} {}", "✗".red(), "Error:".red().bold());
```

#### Interactive Prompts
```rust
use dialoguer::{Confirm, Input};

let proceed = Confirm::new()
    .with_prompt("Continue?")
    .interact()?;

let message = Input::<String>::new()
    .with_prompt("Commit message")
    .interact()?;
```

#### Multi-Progress Bars
```rust
use indicatif::{MultiProgress, ProgressBar};

let multi = MultiProgress::new();
let pb1 = multi.add(ProgressBar::new(100));
let pb2 = multi.add(ProgressBar::new(200));
```

### 🧪 Testing Patterns

#### Property Testing
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_chunking_deterministic(data: Vec<u8>) {
        let chunks1 = chunk_data(&data);
        let chunks2 = chunk_data(&data);
        assert_eq!(chunks1, chunks2);
    }
}
```

#### Snapshot Testing
```rust
use insta::assert_snapshot;

#[test]
fn test_output() {
    let result = format_output(&data);
    assert_snapshot!(result);
}
```

#### Benchmarking
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_chunking(c: &mut Criterion) {
    c.bench_function("chunk 1MB", |b| {
        b.iter(|| chunk_data(black_box(&data)))
    });
}

criterion_group!(benches, bench_chunking);
criterion_main!(benches);
```

### 📝 Common Patterns

#### Error Handling Chain
```rust
use anyhow::{Result, Context};

fn process_file(path: &Path) -> Result<()> {
    let data = std::fs::read(path)
        .context("Failed to read file")?;
    
    let chunks = chunk_data(&data)
        .context("Failed to chunk data")?;
    
    Ok(())
}
```

#### Path Handling
```rust
use directories::BaseDirs;
use path_clean::PathClean;

let base = BaseDirs::new().unwrap();
let store_path = base.home_dir()
    .join(".dig")
    .join(store_id)
    .clean();
```

#### Serialization Round-Trip
```rust
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct Layer { /* fields */ }

// To bytes
let bytes = bincode::serialize(&layer)?;

// From bytes
let layer: Layer = bincode::deserialize(&bytes)?;
```

### 🔥 Pro Tips

1. **Use `?` operator liberally** - Let errors bubble up with context
2. **Prefer `Arc<DashMap>` over `Arc<Mutex<HashMap>>`** for concurrent access
3. **Use `tracing` instead of `println!` for debugging** - Better structured logging
4. **Enable `rayon` feature in `indicatif`** for parallel progress bars
5. **Use `cargo-edit` to add dependencies**: `cargo add clap --features derive`
6. **Profile before optimizing**: Use `cargo-flamegraph` or `pprof`

### 📊 Crate Selection Matrix

| Need | Good | Better | Best (Recommended) |
|------|------|--------|-------------------|
| CLI | `structopt` | `argh` | **`clap v4`** |
| Serialization | `json` | `msgpack` | **`bincode`** |
| Progress | `pbr` | `progress` | **`indicatif`** |
| Errors | `failure` | `error-chain` | **`thiserror` + `anyhow`** |
| Async | `async-std` | `smol` | **`tokio`** |
| Testing | `std` | `mockito` | **`proptest` + `insta`** |

### 🚦 Quick Decision Tree

```
Need to process files?
├─ Single file? → Use std::fs
├─ Directory tree? → Use walkdir
├─ Pattern matching? → Use glob
└─ Large files? → Use memmap2

Need concurrency?
├─ CPU parallelism? → Use rayon
├─ Async I/O? → Use tokio
├─ Shared state? → Use dashmap
└─ Channels? → Use crossbeam

Need UI feedback?
├─ Progress bars? → Use indicatif
├─ Colors? → Use colored/console
├─ Tables? → Use tabled
└─ Prompts? → Use dialoguer
```

This reference should help you quickly implement features using the best available crates!
