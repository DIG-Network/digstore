# CLI Example Implementation

## Complete Example: Implementing the `add` Command

Here's a complete example showing how to implement the `add` command with all the polish requirements:

```rust
use clap::Args;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use console::style;
use std::path::PathBuf;
use std::io::{self, IsTerminal};
use tokio::io::{AsyncBufReadExt, BufReader};
use walkdir::WalkDir;
use bytesize::ByteSize;

#[derive(Args)]
pub struct AddCommand {
    /// Files or directories to add
    paths: Vec<PathBuf>,
    
    /// Add directories recursively
    #[arg(short, long)]
    recursive: bool,
    
    /// Read file list from stdin
    #[arg(long)]
    from_stdin: bool,
    
    /// Show what would be added without adding
    #[arg(long)]
    dry_run: bool,
}

pub async fn execute_add(
    cmd: AddCommand,
    store: &mut Store,
    progress_mgr: &ProgressManager,
) -> Result<()> {
    // Collect files to add
    let files = if cmd.from_stdin {
        collect_files_from_stdin().await?
    } else {
        collect_files_from_paths(&cmd.paths, cmd.recursive)?
    };
    
    if files.is_empty() {
        println!("{} No files to add", style("!").yellow());
        return Ok(());
    }
    
    // Show scanning progress
    let scan_spinner = progress_mgr.create_spinner("Scanning files...");
    
    let mut total_size = 0u64;
    let mut file_infos = Vec::new();
    
    for path in &files {
        let metadata = tokio::fs::metadata(path).await?;
        total_size += metadata.len();
        file_infos.push(FileInfo {
            path: path.clone(),
            size: metadata.len(),
            modified: false,
            new: !store.has_file(path)?,
        });
    }
    
    if let Some(spinner) = scan_spinner {
        spinner.finish_with_message(format!(
            "✓ Found {} files ({})",
            files.len(),
            ByteSize(total_size)
        ));
    }
    
    if cmd.dry_run {
        print_dry_run_summary(&file_infos, progress_mgr);
        return Ok(());
    }
    
    // Add files with progress
    let add_progress = progress_mgr.create_progress(
        files.len() as u64,
        "Adding files to staging"
    );
    
    let mut stats = AddStats::default();
    
    for (idx, file_info) in file_infos.iter().enumerate() {
        if let Some(ref pb) = add_progress {
            pb.set_message(format!("current: {}", file_info.path.display()));
        }
        
        let result = add_file_with_chunking(store, &file_info.path).await?;
        stats.update(&result);
        
        if let Some(ref pb) = add_progress {
            pb.inc(1);
        }
    }
    
    if let Some(pb) = add_progress {
        pb.finish_and_clear();
    }
    
    // Print summary
    print_add_summary(&stats, progress_mgr);
    
    Ok(())
}

async fn collect_files_from_stdin() -> Result<Vec<PathBuf>> {
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut files = Vec::new();
    
    while let Some(line) = lines.next_line().await? {
        let path = PathBuf::from(line.trim());
        if path.exists() {
            files.push(path);
        }
    }
    
    Ok(files)
}

fn collect_files_from_paths(
    paths: &[PathBuf],
    recursive: bool,
) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    
    for path in paths {
        if path.is_file() {
            files.push(path.clone());
        } else if path.is_dir() && recursive {
            for entry in WalkDir::new(path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                files.push(entry.path().to_path_buf());
            }
        } else if path.is_dir() && !recursive {
            eprintln!(
                "{} {}: {} (use -r to add recursively)",
                style("!").yellow(),
                style("Skipping directory").yellow(),
                path.display()
            );
        }
    }
    
    Ok(files)
}

async fn add_file_with_chunking(
    store: &mut Store,
    path: &Path,
) -> Result<AddResult> {
    let data = tokio::fs::read(path).await?;
    let chunks = chunk_file(&data)?;
    
    let mut new_chunks = 0;
    let mut reused_chunks = 0;
    
    for chunk in &chunks {
        if store.has_chunk(&chunk.hash)? {
            reused_chunks += 1;
        } else {
            store.add_chunk(chunk)?;
            new_chunks += 1;
        }
    }
    
    store.stage_file(path, chunks)?;
    
    Ok(AddResult {
        size: data.len() as u64,
        total_chunks: chunks.len(),
        new_chunks,
        reused_chunks,
    })
}

#[derive(Default)]
struct AddStats {
    total_files: usize,
    total_size: u64,
    new_content: u64,
    deduplicated: u64,
    total_chunks: usize,
    new_chunks: usize,
}

impl AddStats {
    fn update(&mut self, result: &AddResult) {
        self.total_files += 1;
        self.total_size += result.size;
        self.total_chunks += result.total_chunks;
        self.new_chunks += result.new_chunks;
        
        let new_size = (result.new_chunks as f64 / result.total_chunks as f64) * result.size as f64;
        self.new_content += new_size as u64;
        self.deduplicated += result.size - new_size as u64;
    }
    
    fn dedup_percentage(&self) -> f64 {
        if self.total_size == 0 {
            0.0
        } else {
            (self.deduplicated as f64 / self.total_size as f64) * 100.0
        }
    }
}

fn print_add_summary(stats: &AddStats, progress_mgr: &ProgressManager) {
    let summary = format!(
        "\n{} Added {} files to staging\n  \
        Total size: {}\n  \
        New content: {}\n  \
        Deduplicated: {} ({:.1}%)",
        style("✓").green().bold(),
        stats.total_files,
        ByteSize(stats.total_size),
        ByteSize(stats.new_content),
        ByteSize(stats.deduplicated),
        stats.dedup_percentage()
    );
    
    progress_mgr.println(&summary);
}

fn print_dry_run_summary(files: &[FileInfo], progress_mgr: &ProgressManager) {
    use tabled::{Table, Tabled, Style};
    
    #[derive(Tabled)]
    struct DryRunRow {
        #[tabled(rename = "Status")]
        status: String,
        #[tabled(rename = "File")]
        file: String,
        #[tabled(rename = "Size")]
        size: String,
    }
    
    let rows: Vec<DryRunRow> = files.iter().map(|f| {
        DryRunRow {
            status: if f.new { "A" } else { "M" }.to_string(),
            file: f.path.display().to_string(),
            size: ByteSize(f.size).to_string(),
        }
    }).collect();
    
    progress_mgr.println("\nFiles that would be added:");
    let table = Table::new(rows).with(Style::modern());
    progress_mgr.println(&table.to_string());
}

// Progress Manager Implementation
pub struct ProgressManager {
    multi: MultiProgress,
    enabled: bool,
}

impl ProgressManager {
    pub fn new(no_progress: bool) -> Self {
        let is_terminal = io::stdout().is_terminal();
        Self {
            multi: MultiProgress::new(),
            enabled: !no_progress && is_terminal,
        }
    }
    
    pub fn create_progress(&self, total: u64, message: &str) -> Option<ProgressBar> {
        if !self.enabled {
            return None;
        }
        
        let pb = self.multi.add(ProgressBar::new(total));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg}\n  {prefix}\n  [{bar:40.cyan/blue}] {pos}/{len} files | {percent}% | {bytes_per_sec} | ETA: {eta}")
                .unwrap()
                .progress_chars("█▓▒░")
        );
        pb.set_message(message.to_string());
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
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
        spinner.enable_steady_tick(std::time::Duration::from_millis(100));
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

// Example usage in main.rs
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Set up progress manager
    let progress_mgr = ProgressManager::new(cli.no_progress);
    
    // Open store
    let mut store = Store::open()?;
    
    // Execute command
    match cli.command {
        Commands::Add(cmd) => {
            execute_add(cmd, &mut store, &progress_mgr).await?;
        }
        // ... other commands
    }
    
    Ok(())
}
```

## Key Patterns Demonstrated

1. **Progress Management**
   - Automatic terminal detection
   - Multiple progress bars with `MultiProgress`
   - Graceful fallback when piping

2. **Streaming Support**
   - Reading file lists from stdin
   - Async file operations
   - Chunked processing

3. **Rich Output**
   - Colored status indicators
   - Table formatting for summaries
   - Clear error messages

4. **User Experience**
   - Current file display during processing
   - Deduplication statistics
   - Dry-run support
   - Helpful warnings

This example can be adapted for all other commands following the same patterns.
