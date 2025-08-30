# CLI Experience Requirements

## Overview

The Digstore Min CLI must provide a polished, professional experience with real-time progress feedback, streaming support, and seamless integration with Unix pipes.

## Core Requirements

### 1. Progress Indication

All long-running operations must show progress bars with:
- Current operation status
- File being processed
- Transfer speed
- ETA (estimated time to arrival)
- Completion percentage

### 2. Streaming Support

- **All data operations must support streaming** - never load entire files into memory
- Support for arbitrarily large files (TB+)
- Efficient chunked processing
- Backpressure handling

### 3. Pipe Support

- **All output commands must support Unix pipes**
- Detect when stdout is a terminal vs pipe
- Disable progress bars when piping
- Support `-o/--output` flag as alternative to piping

## Command-Specific Requirements

### `commit` Command

#### Progress Display
```
Creating commit...
✓ Scanning files... 1,234 files found
✓ Computing hashes...
  
Chunking files:
  processing: src/main.rs
  [████████████████████░░░░░] 156/234 files | 67% | 45.2 MB/s | ETA: 00:02:34
  
Building merkle tree:
  [████████████████████████] 100% | 1,234 nodes
  
Writing layer:
  [████████████████████████] 100% | 156.7 MB

✓ Commit created: abc123def456
  Files: 234
  Total size: 156.7 MB
  Chunks: 1,892 (423 new)
  Deduplication: 34.2% saved
```

#### Implementation with `indicatif`
```rust
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use console::style;

pub struct CommitProgress {
    multi: MultiProgress,
    file_scan: ProgressBar,
    chunking: ProgressBar,
    merkle: ProgressBar,
    writing: ProgressBar,
}

impl CommitProgress {
    pub fn new() -> Self {
        let multi = MultiProgress::new();
        
        let file_scan = multi.add(ProgressBar::new_spinner());
        file_scan.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {prefix:.bold} {msg}")
                .unwrap()
        );
        file_scan.set_prefix("Scanning files...");
        
        let chunking = multi.add(ProgressBar::new(0));
        chunking.set_style(
            ProgressStyle::default_bar()
                .template("  processing: {msg}\n  [{bar:40.cyan/blue}] {pos}/{len} files | {percent}% | {bytes_per_sec} | ETA: {eta}")
                .unwrap()
                .progress_chars("█▓▒░")
        );
        
        Self { multi, file_scan, chunking, merkle, writing }
    }
}
```

### `get` / `cat` Commands

#### Progress Display for Retrieval
```
Retrieving: /data/large_file.bin
[████████████░░░░░░░░░░░░] 45.2 GB/120.5 GB | 37% | 125.3 MB/s | ETA: 00:10:23
```

#### Streaming Implementation
```rust
use tokio::io::{AsyncRead, AsyncWrite};
use futures::stream::StreamExt;

pub async fn stream_file<R, W>(
    reader: R,
    writer: W,
    progress: Option<ProgressBar>,
) -> Result<()> 
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);
    let mut buffer = vec![0u8; 64 * 1024]; // 64KB chunks
    
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 { break; }
        
        writer.write_all(&buffer[..n]).await?;
        
        if let Some(pb) = &progress {
            pb.inc(n as u64);
        }
    }
    
    writer.flush().await?;
    Ok(())
}
```

### Pipe Detection

```rust
use atty::Stream;

pub fn is_stdout_piped() -> bool {
    !atty::is(Stream::Stdout)
}

pub fn setup_output(args: &Args) -> Result<Box<dyn Write>> {
    if let Some(output_path) = &args.output {
        // Write to file
        Ok(Box::new(File::create(output_path)?))
    } else if is_stdout_piped() {
        // Piping to another process - disable progress
        Ok(Box::new(io::stdout()))
    } else {
        // Interactive terminal - show progress
        Ok(Box::new(io::stdout()))
    }
}
```

## Progress Bar Patterns

### 1. File Operations Progress
```rust
let style = ProgressStyle::default_bar()
    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
    .unwrap()
    .with_key("eta", |state: &ProgressState, w: &mut dyn fmt::Write| {
        write!(w, "{:.1}s", state.eta().as_secs_f64())
    });
```

### 2. Multi-Stage Operations
```rust
pub struct OperationProgress {
    multi: MultiProgress,
    current_stage: Arc<Mutex<String>>,
}

impl OperationProgress {
    pub fn stage(&self, name: &str) -> ProgressBar {
        let pb = self.multi.add(ProgressBar::new_spinner());
        pb.set_message(name.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }
    
    pub fn finish_stage(&self, pb: ProgressBar, message: &str) {
        pb.finish_with_message(format!("✓ {}", message));
    }
}
```

### 3. Parallel Operations
```rust
use rayon::prelude::*;

let progress = Arc::new(Mutex::new(ProgressBar::new(files.len() as u64)));

files.par_iter().for_each(|file| {
    process_file(file);
    progress.lock().unwrap().inc(1);
});
```

## Output Formatting

### Success Messages
```rust
use console::style;

println!("{} Commit created: {}", 
    style("✓").green().bold(),
    style(&commit_hash).cyan()
);
```

### Error Messages
```rust
eprintln!("{} {}: {}", 
    style("✗").red().bold(),
    style("Error").red().bold(),
    error_message
);
```

### Summaries
```rust
use tabled::{Table, Tabled, Style};

#[derive(Tabled)]
struct CommitSummary {
    #[tabled(rename = "Metric")]
    metric: &'static str,
    #[tabled(rename = "Value")]
    value: String,
}

let summary = vec![
    CommitSummary { metric: "Files", value: format!("{}", file_count) },
    CommitSummary { metric: "Total Size", value: format_bytes(total_size) },
    CommitSummary { metric: "Chunks", value: format!("{} ({} new)", total_chunks, new_chunks) },
    CommitSummary { metric: "Deduplication", value: format!("{:.1}% saved", dedup_percent) },
];

println!("\n{}", Table::new(summary).with(Style::modern()));
```

## Streaming Requirements

### 1. Input Streaming
```rust
pub async fn add_from_stdin(store: &mut Store) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    
    // Stream chunks directly without loading full content
    let chunker = StreamingChunker::new(CHUNK_SIZE);
    let progress = ProgressBar::new_spinner();
    progress.set_message("Reading from stdin...");
    
    while let Some(chunk) = chunker.next_chunk(&mut reader).await? {
        store.add_chunk(chunk)?;
        progress.tick();
    }
    
    Ok(())
}
```

### 2. Output Streaming
```rust
pub async fn cat_to_stdout(store: &Store, path: &str) -> Result<()> {
    let stdout = tokio::io::stdout();
    let mut writer = BufWriter::new(stdout);
    
    // Stream directly from storage to stdout
    store.stream_file(path, &mut writer).await?;
    writer.flush().await?;
    
    Ok(())
}
```

### 3. Byte Range Streaming
```rust
pub async fn stream_range(
    store: &Store,
    path: &str,
    start: u64,
    end: u64,
    writer: &mut (dyn AsyncWrite + Unpin),
) -> Result<()> {
    let chunks = store.get_chunks_for_range(path, start, end)?;
    
    for chunk in chunks {
        let data = store.read_chunk(&chunk.hash)?;
        let chunk_start = (start - chunk.offset).max(0);
        let chunk_end = ((end - chunk.offset).min(chunk.length));
        
        writer.write_all(&data[chunk_start..chunk_end]).await?;
    }
    
    Ok(())
}
```

## Error Handling with Progress

```rust
use std::panic;

// Ensure progress bars are cleaned up on error
pub fn with_progress<F, R>(f: F) -> Result<R>
where
    F: FnOnce() -> Result<R> + panic::UnwindSafe,
{
    let result = panic::catch_unwind(f);
    
    // Clean up any active progress bars
    if result.is_err() {
        // Force clear the terminal
        term::clear_last_lines(10)?;
    }
    
    match result {
        Ok(res) => res,
        Err(e) => {
            eprintln!("{} Operation failed", style("✗").red().bold());
            panic::resume_unwind(e);
        }
    }
}
```

## Configuration for CLI Experience

```toml
# Default configuration for CLI behavior
[cli]
progress = true           # Show progress bars
color = "auto"           # auto, always, never
quiet = false            # Suppress non-error output
verbose = false          # Extra debug output
pager = "less"          # Pager for long output
editor = "$EDITOR"      # Editor for commit messages

[progress]
style = "default"        # Progress bar style
refresh_rate = 10        # Updates per second
show_speed = true        # Show transfer speed
show_eta = true          # Show time remaining
```

## Testing CLI Experience

```rust
#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use predicates::prelude::*;
    
    #[test]
    fn test_progress_output() {
        let mut cmd = Command::cargo_bin("digstore").unwrap();
        cmd.arg("add").arg("test.txt")
            .assert()
            .success()
            .stdout(predicate::str::contains("✓"));
    }
    
    #[test]
    fn test_pipe_detection() {
        let output = Command::cargo_bin("digstore").unwrap()
            .arg("cat").arg("test.txt")
            .output()
            .unwrap();
        
        // Should not contain ANSI codes when piped
        assert!(!String::from_utf8_lossy(&output.stdout).contains("\x1b["));
    }
}
```

## Summary

The CLI experience must be:
1. **Informative** - Clear progress indication at all times
2. **Responsive** - Immediate feedback for all operations
3. **Professional** - Polished output with proper formatting
4. **Efficient** - Streaming support for large data
5. **Integrable** - Full support for Unix pipes and redirection

By following these requirements, Digstore Min will provide a CLI experience that matches or exceeds tools like Git, Docker, and other modern CLI applications.
