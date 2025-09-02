use crate::core::error::DigstoreError;
use crate::storage::Store;
use crate::urn::parse_urn;
use anyhow::Result;
use clap::Args;
use colored::Colorize;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Args)]
pub struct CatArgs {
    /// File path or URN to output
    #[arg(value_name = "PATH_OR_URN")]
    pub target: String,

    /// Show line numbers
    #[arg(short = 'n', long)]
    pub number: bool,

    /// Disable pager (always output to stdout)
    #[arg(long)]
    pub no_pager: bool,

    /// Retrieve file at specific commit hash
    #[arg(long, value_name = "HASH")]
    pub at: Option<String>,
}

pub fn execute(
    path: String,
    at: Option<String>,
    number: bool,
    no_pager: bool,
    bytes: Option<String>,
    json: bool,
) -> Result<()> {
    let args = CatArgs {
        target: path,
        number,
        no_pager,
        at,
    };
    let current_dir = std::env::current_dir()?;

    // Try to open store from current directory
    let store = match Store::open(&current_dir) {
        Ok(store) => store,
        Err(_) => {
            // If no local store, try to parse as URN
            if args.target.starts_with("urn:dig:") {
                return handle_urn_cat(&args, &bytes, json);
            } else {
                return Err(DigstoreError::store_not_found(current_dir).into());
            }
        }
    };

    // Check if target is a URN
    if args.target.starts_with("urn:dig:") {
        return handle_urn_cat(&args, &bytes, json);
    }

    // Handle as file path
    handle_path_cat(&store, &args, &bytes, json)
}

fn handle_path_cat(
    store: &Store,
    args: &CatArgs,
    bytes: &Option<String>,
    json: bool,
) -> Result<()> {
    let file_path = Path::new(&args.target);

    // Retrieve file content
    let mut content = if let Some(at_hash) = &args.at {
        let root_hash = crate::core::types::Hash::from_hex(at_hash)
            .map_err(|_| DigstoreError::internal("Invalid hash format"))?;
        store.get_file_at(file_path, Some(root_hash))?
    } else {
        store.get_file(file_path)?
    };

    // Apply byte range if specified
    if let Some(byte_range_str) = bytes {
        let byte_range =
            crate::urn::parser::parse_byte_range(&format!("#bytes={}", byte_range_str))?;
        let file_len = content.len();
        let start = byte_range.start.unwrap_or(0) as usize;
        let end = byte_range.end.map(|e| (e + 1) as usize).unwrap_or(file_len);
        let start = start.min(file_len);
        let end = end.min(file_len);
        content = content[start..end].to_vec();
    }

    output_content(&content, args, json)
}

fn handle_urn_cat(args: &CatArgs, bytes: &Option<String>, json: bool) -> Result<()> {
    let mut urn = parse_urn(&args.target)?;

    // Add byte range if specified
    if let Some(byte_range_str) = bytes {
        let byte_range =
            crate::urn::parser::parse_byte_range(&format!("#bytes={}", byte_range_str))?;
        urn.byte_range = Some(byte_range);
    }

    // We need a store to resolve URNs, try to get one
    let current_dir = std::env::current_dir()?;
    let store = Store::open(&current_dir).or_else(|_| {
        // Try to extract store ID from URN and open global store
        let store_id = urn.store_id;
        Store::open_global(&store_id)
    })?;

    let content = urn.resolve(&store)?;
    output_content(&content, args, json)
}

fn output_content(content: &[u8], args: &CatArgs, json: bool) -> Result<()> {
    if json {
        // JSON metadata to stderr, content to stdout
        let metadata = serde_json::json!({
            "action": "content_displayed",
            "size": content.len(),
            "line_numbers": args.number,
            "pager_disabled": args.no_pager,
            "at_root": args.at.as_ref()
        });
        eprintln!("{}", serde_json::to_string_pretty(&metadata)?);

        // Stream content to stdout
        std::io::stdout().write_all(content)?;
        return Ok(());
    }

    // Convert to string for processing
    let text = String::from_utf8_lossy(content);

    // Check if we should use a pager
    let should_page = !args.no_pager && should_use_pager(&text);

    if should_page {
        output_with_pager(&text, args)
    } else {
        output_direct(&text, args)
    }
}

fn should_use_pager(content: &str) -> bool {
    // Use pager if content has more than 25 lines or is longer than 2000 characters
    let line_count = content.lines().count();
    line_count > 25 || content.len() > 2000
}

fn output_with_pager(content: &str, args: &CatArgs) -> Result<()> {
    // Try to use system pager (less, more, or built-in)
    let pager_cmd = std::env::var("PAGER").unwrap_or_else(|_| {
        if cfg!(windows) {
            "more".to_string()
        } else {
            "less".to_string()
        }
    });

    let mut pager = match Command::new(&pager_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(pager) => pager,
        Err(_) => {
            // Fallback to direct output if pager fails
            return output_direct(content, args);
        }
    };

    if let Some(stdin) = pager.stdin.as_mut() {
        write_content_to_writer(stdin, content, args)?;
    }

    let _ = pager.wait();
    Ok(())
}

fn output_direct(content: &str, args: &CatArgs) -> Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    write_content_to_writer(&mut handle, content, args)?;
    Ok(())
}

fn write_content_to_writer<W: Write>(writer: &mut W, content: &str, args: &CatArgs) -> Result<()> {
    if args.number {
        // Output with line numbers
        for (line_num, line) in content.lines().enumerate() {
            writeln!(writer, "{:6}  {}", (line_num + 1).to_string().cyan(), line)?;
        }
    } else {
        // Direct output
        write!(writer, "{}", content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_should_use_pager() {
        let short_content = "Hello\nWorld";
        assert!(!should_use_pager(short_content));

        let long_content = "line\n".repeat(30);
        assert!(should_use_pager(&long_content));

        let wide_content = "x".repeat(3000);
        assert!(should_use_pager(&wide_content));
    }

    #[test]
    fn test_write_content_with_numbers() {
        let mut output = Vec::new();
        let content = "line1\nline2\nline3";
        let args = CatArgs {
            target: "test".to_string(),
            number: true,
            no_pager: true,
            at: None,
        };

        write_content_to_writer(&mut output, content, &args).unwrap();
        let result = String::from_utf8(output).unwrap();

        // Check that line numbers are present (ignoring color codes)
        assert!(result.contains("1"));
        assert!(result.contains("2"));
        assert!(result.contains("3"));
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(result.contains("line3"));
    }
}
