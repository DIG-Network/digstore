//! Comprehensive CLI command validation tests
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]
//!
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]
//! This test suite validates all 19 CLI commands to ensure they work correctly
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]
//! and prevent future regressions.
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]

use anyhow::Result;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Test utility for running CLI commands
struct CommandTester {
    temp_dir: TempDir,
    project_path: std::path::PathBuf,
}

impl CommandTester {
    fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path().to_path_buf();

        // Create test files
        fs::write(project_path.join("test1.txt"), "Hello World")?;
        fs::write(project_path.join("test2.txt"), "Test content")?;
        fs::create_dir_all(project_path.join("docs"))?;
        fs::write(project_path.join("docs/readme.md"), "Documentation")?;

        Ok(Self {
            temp_dir,
            project_path,
        })
    }

    fn run_command(&self, args: &[&str]) -> Result<std::process::Output> {
        let output = Command::new("cargo")
            .args(&["run", "--"])
            .args(args)
            .current_dir(&self.project_path)
            .output()?;
        Ok(output)
    }

    fn run_command_success(&self, args: &[&str]) -> Result<String> {
        let output = self.run_command(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Command failed: {}", stderr);
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[test]
fn test_core_workflow_commands() -> Result<()> {
    let tester = CommandTester::new()?;

    // 1. Test init command
    let init_output = tester.run_command_success(&["init", "--name", "Test Repo"])?;
    assert!(init_output.contains("Repository initialized"));
    assert!(init_output.contains("Store ID:"));

    // 2. Test add command
    let add_output = tester.run_command_success(&["--yes", "add", "test1.txt", "test2.txt"])?;
    assert!(add_output.contains("files added to staging"));

    // 3. Test staged command
    let staged_output = tester.run_command_success(&["staged", "--limit", "10"])?;
    assert!(staged_output.contains("Staged Files"));
    assert!(staged_output.contains("test1.txt"));
    assert!(staged_output.contains("test2.txt"));

    // 4. Test status command
    let status_output = tester.run_command_success(&["status"])?;
    assert!(status_output.contains("Repository Status"));
    assert!(status_output.contains("Store ID:"));

    // 5. Test commit command
    let commit_output = tester.run_command_success(&["commit", "-m", "Test commit"])?;
    assert!(commit_output.contains("Commit created"));

    Ok(())
}

#[test]
fn test_data_access_commands() -> Result<()> {
    let tester = CommandTester::new()?;

    // Setup repository with committed data
    tester.run_command_success(&["init", "--name", "Data Test"])?;
    tester.run_command_success(&["--yes", "add", "test1.txt"])?;
    tester.run_command_success(&["commit", "-m", "Initial commit"])?;

    // Test get command
    let get_output = tester.run_command_success(&["get", "test1.txt"])?;
    assert!(get_output.contains("Hello World"));

    // Test cat command
    let cat_output = tester.run_command_success(&["cat", "test1.txt"])?;
    assert!(cat_output.contains("Hello World"));

    // Test root command
    let root_output = tester.run_command_success(&["root"])?;
    assert!(root_output.contains("Current Root Information"));
    assert!(root_output.contains("Root Hash:"));

    Ok(())
}

#[test]
fn test_analysis_commands() -> Result<()> {
    let tester = CommandTester::new()?;

    // Setup repository with committed data
    tester.run_command_success(&["init", "--name", "Analysis Test"])?;
    tester.run_command_success(&["--yes", "add", "test1.txt", "test2.txt"])?;
    tester.run_command_success(&["commit", "-m", "Test commit"])?;

    // Test history command
    let history_output = tester.run_command_success(&["history"])?;
    assert!(history_output.contains("Root History Analysis"));

    // Test size command
    let size_output = tester.run_command_success(&["size"])?;
    assert!(size_output.contains("Storage Analytics"));

    // Test store-info command
    let store_info_output = tester.run_command_success(&["store-info"])?;
    assert!(store_info_output.contains("Store Information"));

    // Test stats command
    let stats_output = tester.run_command_success(&["stats"])?;
    assert!(stats_output.contains("Repository Statistics"));

    // Test layers command
    let layers_output = tester.run_command_success(&["layers"])?;
    assert!(layers_output.contains("Layer Analysis"));

    Ok(())
}

#[test]
fn test_proof_commands() -> Result<()> {
    let tester = CommandTester::new()?;

    // Setup repository with committed data
    tester.run_command_success(&["init", "--name", "Proof Test"])?;
    tester.run_command_success(&["--yes", "add", "test1.txt"])?;
    tester.run_command_success(&["commit", "-m", "Proof test commit"])?;

    // Test prove command
    let prove_output = tester.run_command_success(&["prove", "test1.txt"])?;
    assert!(prove_output.contains("Proof generated"));

    Ok(())
}

#[test]
fn test_utility_commands() -> Result<()> {
    let tester = CommandTester::new()?;

    // Test completion command (doesn't require repository)
    let completion_output = tester.run_command_success(&["completion", "bash"])?;
    assert!(completion_output.contains("complete"));
    assert!(completion_output.contains("digstore"));

    Ok(())
}

#[test]
fn test_command_options_and_flags() -> Result<()> {
    let tester = CommandTester::new()?;

    // Setup repository
    tester.run_command_success(&["init", "--name", "Options Test"])?;
    tester.run_command_success(&["--yes", "add", "test1.txt", "test2.txt"])?;

    // Test staged command with different options
    let staged_limit = tester.run_command_success(&["staged", "--limit", "1"])?;
    assert!(staged_limit.contains("Showing 1 staged files"));

    let staged_detailed = tester.run_command_success(&["staged", "--detailed"])?;
    assert!(staged_detailed.contains("Hash"));
    assert!(staged_detailed.contains("Chunks"));

    let staged_json = tester.run_command_success(&["staged", "--json"])?;
    assert!(staged_json.contains("\"staged_files\""));

    // Test add command with --dry-run
    let add_dry_run = tester.run_command_success(&["add", "--dry-run", "docs/readme.md"])?;
    assert!(add_dry_run.contains("Would process"));

    // Test global --yes flag
    let add_yes = tester.run_command_success(&["--yes", "add", "docs/readme.md"])?;
    assert!(add_yes.contains("files added to staging"));

    Ok(())
}

#[test]
fn test_error_conditions() -> Result<()> {
    let tester = CommandTester::new()?;

    // Test commands without repository (should fail gracefully)
    let no_repo_output = tester.run_command(&["status"])?;
    assert!(!no_repo_output.status.success());
    let stderr = String::from_utf8_lossy(&no_repo_output.stderr);
    assert!(stderr.contains("No repository found") || stderr.contains("Store not found"));

    // Setup repository for other error tests
    tester.run_command_success(&["init", "--name", "Error Test"])?;

    // Test commit with no staged files
    let empty_commit = tester.run_command(&["commit", "-m", "Empty commit"])?;
    assert!(!empty_commit.status.success());
    let stderr = String::from_utf8_lossy(&empty_commit.stderr);
    assert!(stderr.contains("No files staged"));

    // Test get non-existent file
    let get_missing = tester.run_command(&["get", "nonexistent.txt"])?;
    assert!(!get_missing.status.success());

    Ok(())
}

#[test]
fn test_large_repository_handling() -> Result<()> {
    let tester = CommandTester::new()?;

    // Create many test files
    for i in 0..50 {
        fs::write(
            tester.project_path.join(format!("file{}.txt", i)),
            format!("Content {}", i),
        )?;
    }

    // Test repository operations with many files
    tester.run_command_success(&["init", "--name", "Large Test"])?;

    // Test add -A with many files
    let add_all_output = tester.run_command_success(&["--yes", "add", "-A"])?;
    assert!(add_all_output.contains("files added to staging"));

    // Test staged command with pagination
    let staged_page1 = tester.run_command_success(&["staged", "--limit", "10", "--page", "1"])?;
    assert!(staged_page1.contains("Page 1"));

    let staged_page2 = tester.run_command_success(&["staged", "--limit", "10", "--page", "2"])?;
    assert!(staged_page2.contains("Page 2"));

    // Test commit with many files
    let commit_output = tester.run_command_success(&["commit", "-m", "Large commit"])?;
    assert!(commit_output.contains("Commit created"));

    Ok(())
}

#[test]
fn test_digignore_functionality() -> Result<()> {
    let tester = CommandTester::new()?;

    // Create .digignore file
    fs::write(tester.project_path.join(".digignore"), "*.tmp\n*.log\n")?;

    // Create files that should be ignored
    fs::write(tester.project_path.join("temp.tmp"), "Temporary")?;
    fs::write(tester.project_path.join("debug.log"), "Log data")?;
    fs::write(tester.project_path.join("keep.txt"), "Keep this")?;

    tester.run_command_success(&["init", "--name", "Ignore Test"])?;

    // Test add -A respects .digignore
    let add_output = tester.run_command_success(&["--yes", "add", "-A"])?;

    // Test staged command to see what was actually added
    let staged_output = tester.run_command_success(&["staged", "--all"])?;
    assert!(staged_output.contains("keep.txt"));
    // Should not contain ignored files
    assert!(!staged_output.contains("temp.tmp"));
    assert!(!staged_output.contains("debug.log"));

    Ok(())
}
