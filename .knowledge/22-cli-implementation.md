# CLI Implementation Guide

## Implementing a Polished CLI Experience

This guide shows how to implement the polished CLI requirements using Rust crates.

## 1. Core CLI Structure with Progress Support

```rust
use clap::{Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use console::{style, Term};
use std::io::{self, Write, IsTerminal};

#[derive(Parser)]
#[command(author, version, about)]
#[command(propagate_version = true)]
struct Cli {
    /// Disable progress bars
    #[arg(long, global = true)]
    no_progress: bool,
    
    /// Color output: auto, always, never
    #[arg(long, default_value = "auto", global = true)]
    color: ColorChoice,
    
    /// Suppress non-error output
    #[arg(short, long, global = true)]
    quiet: bool,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add files to the repository
    Add {
        /// Files to add
        paths: Vec<PathBuf>,
        
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
        
        /// Read file list from stdin
        #[arg(long)]
        from_stdin: bool,
        
        /// Add directories recursively
        #[arg(short, long)]
        recursive: bool,
    },
    
    /// Retrieve files
    Get {
        /// Path or URN to retrieve
        path: String,
        
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
        
        /// Force progress bars even when piping
        #[arg(long)]
        progress: bool,
    },
}
```

## 2. Progress Management System

```rust
use std::sync::Arc;
use parking_lot::Mutex;

pub struct ProgressManager {
    multi: MultiProgress,
    enabled: bool,
    is_terminal: bool,
}

impl ProgressManager {
    pub fn new(cli: &Cli) -> Self {
        let is_terminal = io::stdout().is_terminal();
        let enabled = !cli.no_progress && (is_terminal || cli.progress);
        
        Self {
            multi: MultiProgress::new(),
            enabled,
            is_terminal,
        }
    }
    
    pub fn create_progress(&self, total: u64, message: &str) -> Option<ProgressBar> {
        if !self.enabled {
            return None;
        }
        
        let pb = self.multi.add(ProgressBar::new(total));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                .unwrap()
                .progress_chars("█▓▒░")
        );
        pb.set_message(message.to_string());
        Some(pb)
    }
    
    pub fn create_spinner(&self, message: &str) -> Option<ProgressBar> {
        if !self.enabled {
            return None;
        }
        
        let spinner = self.multi.add(ProgressBar::new_spinner());
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap()
        );
        spinner.set_message(message.to_string());
        spinner.enable_steady_tick(Duration::from_millis(100));
        Some(spinner)
    }
    
    pub fn println(&self, message: &str) {
        if self.enabled {
            self.multi.println(message).unwrap();
        } else {
            println!("{}", message);
        }
    }
}
```

## 3. Streaming I/O with Progress

```rust
use tokio::io::{AsyncRead, AsyncWrite, AsyncBufReadExt, BufReader, BufWriter};
use futures::stream::{Stream, StreamExt};

pub struct StreamingIO;

impl StreamingIO {
    /// Stream from reader to writer with optional progress
    pub async fn stream_with_progress<R, W>(
        reader: R,
        writer: W,
        progress: Option<ProgressBar>,
        buffer_size: usize,
    ) -> Result<u64>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut reader = BufReader::with_capacity(buffer_size, reader);
        let mut writer = BufWriter::with_capacity(buffer_size, writer);
        let mut buffer = vec![0u8; buffer_size];
        let mut total = 0u64;
        
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            
            writer.write_all(&buffer[..n]).await?;
            total += n as u64;
            
            if let Some(ref pb) = progress {
                pb.inc(n as u64);
            }
        }
        
        writer.flush().await?;
        
        if let Some(pb) = progress {
            pb.finish_with_message("Complete");
        }
        
        Ok(total)
    }
    
    /// Stream from stdin with line processing
    pub async fn stream_stdin_lines<F>(mut processor: F) -> Result<()>
    where
        F: FnMut(String) -> Result<()>,
    {
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();
        
        while let Some(line) = lines.next_line().await? {
            processor(line)?;
        }
        
        Ok(())
    }
}
```

## 4. Commit Command with Rich Progress

```rust
pub async fn commit_with_progress(
    store: &mut Store,
    message: &str,
    progress_mgr: &ProgressManager,
) -> Result<Hash> {
    // Stage 1: Scan files
    let scan_spinner = progress_mgr.create_spinner("Scanning files...");
    let files = store.scan_staged_files().await?;
    
    if let Some(spinner) = scan_spinner {
        spinner.finish_with_message(format!("✓ Found {} files", files.len()));
    }
    
    // Stage 2: Process files with progress
    let file_progress = progress_mgr.create_progress(
        files.len() as u64,
        "Processing files"
    );
    
    let mut chunks = Vec::new();
    for (idx, file) in files.iter().enumerate() {
        if let Some(ref pb) = file_progress {
            pb.set_message(format!("current: {}", file.path.display()));
        }
        
        let file_chunks = process_file(store, file).await?;
        chunks.extend(file_chunks);
        
        if let Some(ref pb) = file_progress {
            pb.inc(1);
        }
    }
    
    if let Some(pb) = file_progress {
        pb.finish_with_message("✓ Files processed");
    }
    
    // Stage 3: Build merkle tree
    let tree_progress = progress_mgr.create_progress(
        chunks.len() as u64,
        "Building merkle tree"
    );
    
    let tree = build_merkle_tree(&chunks, tree_progress.as_ref()).await?;
    
    if let Some(pb) = tree_progress {
        pb.finish_with_message("✓ Merkle tree built");
    }
    
    // Stage 4: Write layer
    let write_spinner = progress_mgr.create_spinner("Writing layer...");
    let commit_hash = store.write_layer(&tree, message).await?;
    
    if let Some(spinner) = write_spinner {
        spinner.finish_with_message("✓ Layer written");
    }
    
    // Print summary
    print_commit_summary(&commit_hash, &files, &chunks, progress_mgr);
    
    Ok(commit_hash)
}

fn print_commit_summary(
    hash: &Hash,
    files: &[FileInfo],
    chunks: &[Chunk],
    progress_mgr: &ProgressManager,
) {
    let summary = format!(
        "\n{} Commit successful!\n\n\
        Commit: {}\n\
        Files: {} ({} modified, {} new)\n\
        Chunks: {} total, {} new\n\
        Size: {}\n\
        Deduplication: {:.1}% saved",
        style("✓").green().bold(),
        style(hash.to_string()).cyan(),
        files.len(),
        files.iter().filter(|f| f.modified).count(),
        files.iter().filter(|f| f.new).count(),
        chunks.len(),
        chunks.iter().filter(|c| c.new).count(),
        bytesize::ByteSize(total_size),
        dedup_percent
    );
    
    progress_mgr.println(&summary);
}
```

## 5. Get Command with Streaming and Progress

```rust
pub async fn get_with_progress(
    store: &Store,
    path: &str,
    output: Option<PathBuf>,
    force_progress: bool,
    progress_mgr: &ProgressManager,
) -> Result<()> {
    // Determine output destination
    let is_piped = !io::stdout().is_terminal();
    let show_progress = progress_mgr.enabled || force_progress;
    
    // Get file metadata
    let metadata = store.get_file_metadata(path).await?;
    
    // Create progress bar if appropriate
    let progress = if show_progress && (output.is_some() || force_progress) {
        progress_mgr.create_progress(
            metadata.size,
            &format!("Retrieving: {}", path)
        )
    } else {
        None
    };
    
    // Set up output writer
    let writer: Box<dyn AsyncWrite + Unpin> = match output {
        Some(path) => Box::new(tokio::fs::File::create(path).await?),
        None => Box::new(tokio::io::stdout()),
    };
    
    // Stream with progress
    let bytes_written = StreamingIO::stream_with_progress(
        store.open_file(path).await?,
        writer,
        progress,
        64 * 1024, // 64KB buffer
    ).await?;
    
    // Print success message if not piping
    if !is_piped && output.is_some() {
        progress_mgr.println(&format!(
            "\n{} Retrieved successfully to: {}\n  Size: {}\n  Hash: {}",
            style("✓").green().bold(),
            output.unwrap().display(),
            bytesize::ByteSize(bytes_written),
            metadata.hash
        ));
    }
    
    Ok(())
}
```

## 6. Status Command with Rich Formatting

```rust
use tabled::{Table, Tabled, Style, Alignment, Modify, object::Columns};

pub fn display_status(
    store: &Store,
    short: bool,
    porcelain: bool,
) -> Result<()> {
    let status = store.get_status()?;
    
    if porcelain {
        // Machine-readable format
        for change in &status.changes {
            println!("{} {}", change.status_code(), change.path);
        }
        return Ok(());
    }
    
    if short {
        // Short format like git
        for change in &status.changes {
            println!("{} {}", 
                style(change.status_code()).color256(change.status_color()),
                change.path
            );
        }
        return Ok(());
    }
    
    // Rich formatted output
    println!("{}", style("Repository Status").bold().underlined());
    println!("{}", "═".repeat(40));
    println!();
    println!("Current commit: {}", style(&status.head).cyan());
    println!("Store ID: {}", style(&status.store_id).dim());
    println!();
    
    if !status.staged.is_empty() {
        println!("{}", style("Changes to be committed:").green());
        for file in &status.staged {
            println!("  {} {:<40} {}",
                style(file.status_icon()).green(),
                file.path,
                style(format_size_change(file)).dim()
            );
        }
        println!();
    }
    
    if !status.untracked.is_empty() {
        println!("{}", style("Untracked files:").yellow());
        for file in &status.untracked {
            println!("  {}", file);
        }
        println!();
    }
    
    // Summary table
    let summary = vec![
        SummaryRow { metric: "Files staged", value: status.staged.len().to_string() },
        SummaryRow { metric: "Total changes", value: format_bytes(status.total_change_size) },
        SummaryRow { metric: "Chunks affected", value: status.affected_chunks.to_string() },
    ];
    
    println!("{}", style("Summary:").bold());
    let table = Table::new(summary)
        .with(Style::blank())
        .with(Modify::new(Columns::single(0)).with(Alignment::left()))
        .with(Modify::new(Columns::single(1)).with(Alignment::right()));
    
    println!("{}", table);
    
    Ok(())
}
```

## 7. Error Handling with Suggestions

```rust
use color_eyre::{eyre::Result, Help, SectionExt};

pub trait ErrorWithSuggestions {
    fn with_suggestions(self) -> Self;
}

impl<T> ErrorWithSuggestions for Result<T> {
    fn with_suggestions(self) -> Self {
        self.map_err(|e| {
            match e.downcast_ref::<DigstoreError>() {
                Some(DigstoreError::FileNotFound(path)) => {
                    e.with_section(|| {
                        format!(
                            "Suggestions:\n\
                            • Check if the file exists with: digstore ls {}\n\
                            • Search for similar files: digstore find \"*{}*\"\n\
                            • Verify you're using the correct root hash",
                            path.parent().unwrap_or(Path::new("/")),
                            path.file_stem().unwrap_or_default().to_string_lossy()
                        )
                    }.header("Help:"))
                },
                Some(DigstoreError::InvalidUrn(urn)) => {
                    e.with_section(|| {
                        "URN format: urn:dig:chia:{storeID}[:{rootHash}][/{path}][#{byteRange}]"
                    }.header("Expected format:"))
                },
                _ => e,
            }
        })
    }
}
```

## 8. Auto-completion Generation

```rust
use clap_complete::{generate, Generator, Shell};

pub fn generate_completions<G: Generator>(gen: G, cmd: &mut Command) {
    generate(gen, cmd, "digstore", &mut io::stdout());
}

// In main.rs
if let Some(shell) = args.completions {
    let mut cmd = Cli::command();
    match shell {
        Shell::Bash => generate_completions(Bash, &mut cmd),
        Shell::Zsh => generate_completions(Zsh, &mut cmd),
        Shell::Fish => generate_completions(Fish, &mut cmd),
        Shell::PowerShell => generate_completions(PowerShell, &mut cmd),
    }
    return Ok(());
}
```

## 9. Testing the CLI Experience

```rust
#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use predicates::prelude::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_progress_output_to_file() {
        let temp = TempDir::new().unwrap();
        let output_file = temp.path().join("output.txt");
        
        Command::cargo_bin("digstore").unwrap()
            .arg("get")
            .arg("/test.txt")
            .arg("-o")
            .arg(&output_file)
            .assert()
            .success()
            .stderr(predicate::str::contains("✓ Retrieved successfully"));
    }
    
    #[test]
    fn test_quiet_mode() {
        Command::cargo_bin("digstore").unwrap()
            .arg("--quiet")
            .arg("add")
            .arg("test.txt")
            .assert()
            .success()
            .stdout(predicate::str::is_empty());
    }
    
    #[test]
    fn test_pipe_detection() {
        // When piped, should not output ANSI codes
        let output = Command::cargo_bin("digstore").unwrap()
            .arg("get")
            .arg("/test.txt")
            .output()
            .unwrap();
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(!stdout.contains("\x1b["));
    }
}
```

## Summary

This implementation provides:

1. **Automatic Progress Detection** - Shows progress in terminals, hides when piping
2. **Flexible Output** - Support for both stdout and file output with `-o`
3. **Rich Formatting** - Beautiful tables, colors, and icons
4. **Streaming Everything** - Never loads full files into memory
5. **Error Recovery** - Helpful suggestions on failures
6. **Testing Support** - Comprehensive test utilities

The result is a professional CLI that feels polished and responsive, matching the experience of tools like Git, Docker, and Cargo.
