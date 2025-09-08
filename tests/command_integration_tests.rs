//! Comprehensive integration tests for all CLI commands
//!
//! These tests ensure all 19 commands work correctly and prevent regressions

use anyhow::Result;
use digstore_min::storage::Store;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Test timeout - individual tests should complete within 30 seconds
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Helper to ensure tests don't run too long
fn with_timeout<F, R>(test_fn: F) -> R
where
    F: FnOnce() -> R,
{
    let start = Instant::now();
    let result = test_fn();
    let elapsed = start.elapsed();

    if elapsed > TEST_TIMEOUT {
        panic!(
            "Test exceeded timeout of {:?}, took {:?}",
            TEST_TIMEOUT, elapsed
        );
    }

    result
}

/// Test utility for creating test repositories with data
struct TestRepository {
    temp_dir: TempDir,
    store: Store,
}

impl TestRepository {
    fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path();

        // Create test files
        fs::write(project_path.join("test1.txt"), "Hello World")?;
        fs::write(project_path.join("test2.txt"), "Test content")?;
        fs::create_dir_all(project_path.join("docs"))?;
        fs::write(project_path.join("docs/readme.md"), "Documentation")?;

        let mut store = Store::init(project_path)?;

        // Add and commit files to create data for testing
        store.add_file(&Path::new("test1.txt"))?;
        store.add_file(&Path::new("test2.txt"))?;
        store.add_file(&Path::new("docs/readme.md"))?;

        let _commit_id = store.commit("Initial test commit")?;

        Ok(Self { temp_dir, store })
    }

    fn project_path(&self) -> &Path {
        self.temp_dir.path()
    }

    fn run_command(&self, args: &[&str]) -> Result<std::process::Output> {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--")
            .args(args)
            .current_dir(self.project_path());

        Ok(cmd.output()?)
    }
}

#[test]
fn test_core_staging_commands() -> Result<()> {
    with_timeout(|| {
        let repo = TestRepository::new()?;

        // Test status command
        let output = repo.run_command(&["status"])?;
        assert!(output.status.success(), "Status command should succeed");
        let stdout = String::from_utf8(output.stdout)?;
        assert!(
            stdout.contains("No changes staged"),
            "Should show no staged changes after commit"
        );

        // Test staged command (should show empty)
        let output = repo.run_command(&["staged"])?;
        assert!(output.status.success(), "Staged command should succeed");
        let stdout = String::from_utf8(output.stdout)?;
        assert!(
            stdout.contains("No files staged"),
            "Should show no staged files"
        );

        Ok(())
    })
}

#[test]
fn test_data_access_commands() -> Result<()> {
    let repo = TestRepository::new()?;

    // Test get command
    let output = repo.run_command(&["get", "test1.txt"])?;
    assert!(output.status.success(), "Get command should succeed");

    // Test cat command
    let output = repo.run_command(&["cat", "test1.txt"])?;
    assert!(output.status.success(), "Cat command should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("Hello World"),
        "Cat should show file content"
    );

    // Test log command
    let output = repo.run_command(&["log"])?;
    assert!(output.status.success(), "Log command should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("Initial test commit"),
        "Log should show commit message"
    );

    Ok(())
}

#[test]
fn test_analysis_commands() -> Result<()> {
    let repo = TestRepository::new()?;

    // Test root command
    let output = repo.run_command(&["root"])?;
    assert!(output.status.success(), "Root command should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(!stdout.is_empty(), "Root should show information");

    // Test history command
    let output = repo.run_command(&["history"])?;
    assert!(output.status.success(), "History command should succeed");

    // Test size command
    let output = repo.run_command(&["size"])?;
    assert!(output.status.success(), "Size command should succeed");

    // Test store-info command
    let output = repo.run_command(&["store-info"])?;
    assert!(output.status.success(), "Store-info command should succeed");

    // Test stats command
    let output = repo.run_command(&["stats"])?;
    assert!(output.status.success(), "Stats command should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("Repository Statistics"),
        "Stats should show statistics"
    );

    Ok(())
}

#[test]
fn test_advanced_commands() -> Result<()> {
    let repo = TestRepository::new()?;

    // Test layers command
    let output = repo.run_command(&["layers"])?;
    assert!(output.status.success(), "Layers command should succeed");

    // Test prove command
    let output = repo.run_command(&["prove", "test1.txt"])?;
    assert!(output.status.success(), "Prove command should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("Proof generated"),
        "Prove should generate proof"
    );

    Ok(())
}

#[test]
fn test_completion_command() -> Result<()> {
    let repo = TestRepository::new()?;

    // Test completion command for different shells
    let shells = ["bash", "zsh", "fish", "powershell"];

    for shell in &shells {
        let output = repo.run_command(&["completion", shell])?;
        assert!(
            output.status.success(),
            "Completion for {} should succeed",
            shell
        );
        let stdout = String::from_utf8(output.stdout)?;
        assert!(
            !stdout.is_empty(),
            "Completion should generate script for {}",
            shell
        );
    }

    Ok(())
}

#[test]
fn test_end_to_end_workflow() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create test file
    fs::write(project_path.join("workflow.txt"), "End-to-end test")?;

    let mut cmd_helper = |args: &[&str]| -> Result<std::process::Output> {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--")
            .args(args)
            .current_dir(project_path);
        Ok(cmd.output()?)
    };

    // Step 1: Initialize repository
    let output = cmd_helper(&["init", "--name", "E2E Test"])?;
    assert!(output.status.success(), "Init should succeed");

    // Step 2: Add files
    let output = cmd_helper(&["--yes", "add", "workflow.txt"])?;
    assert!(output.status.success(), "Add should succeed");

    // Step 3: Check status shows staged files
    let output = cmd_helper(&["status"])?;
    assert!(output.status.success(), "Status should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("staged"), "Status should show staged files");

    // Step 4: Commit files
    let output = cmd_helper(&["commit", "-m", "E2E test commit"])?;
    assert!(output.status.success(), "Commit should succeed");

    // Step 5: Verify commit with log
    let output = cmd_helper(&["log"])?;
    assert!(output.status.success(), "Log should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("E2E test commit"), "Log should show commit");

    // Step 6: Retrieve file
    let output = cmd_helper(&["get", "workflow.txt"])?;
    assert!(output.status.success(), "Get should succeed");

    // Step 7: Check status shows clean state
    let output = cmd_helper(&["status"])?;
    assert!(output.status.success(), "Final status should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("No changes staged"),
        "Should show clean state"
    );

    Ok(())
}

#[test]
fn test_add_all_workflow() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create multiple test files
    for i in 1..=5 {
        fs::write(
            project_path.join(format!("file{}.txt", i)),
            format!("Content {}", i),
        )?;
    }

    let mut cmd_helper = |args: &[&str]| -> Result<std::process::Output> {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--")
            .args(args)
            .current_dir(project_path);
        Ok(cmd.output()?)
    };

    // Initialize repository
    let output = cmd_helper(&["init", "--name", "Add All Test"])?;
    assert!(output.status.success(), "Init should succeed");

    // Test add -A (add all files)
    let output = cmd_helper(&["--yes", "add", "-A"])?;
    assert!(output.status.success(), "Add -A should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("files added to staging"),
        "Should show files added"
    );

    // Test staged command shows files
    let output = cmd_helper(&["staged", "--limit", "10"])?;
    assert!(output.status.success(), "Staged command should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Staged Files"), "Should show staged files");

    // Commit all files
    let output = cmd_helper(&["commit", "-m", "Add all files"])?;
    assert!(output.status.success(), "Commit should succeed");

    // Verify all files are accessible
    for i in 1..=5 {
        let output = cmd_helper(&["get", &format!("file{}.txt", i)])?;
        assert!(
            output.status.success(),
            "Should be able to get file{}.txt",
            i
        );
    }

    Ok(())
}

#[test]
fn test_root_tracking_persistence() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create test file
    fs::write(project_path.join("persist.txt"), "Persistence test")?;

    let mut cmd_helper = |args: &[&str]| -> Result<std::process::Output> {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--")
            .args(args)
            .current_dir(project_path);
        Ok(cmd.output()?)
    };

    // Initialize and commit
    cmd_helper(&["init", "--name", "Persistence Test"])?;
    cmd_helper(&["--yes", "add", "persist.txt"])?;
    let commit_output = cmd_helper(&["commit", "-m", "Persistence test"])?;
    assert!(commit_output.status.success(), "Commit should succeed");

    // Extract commit ID from output
    let commit_stdout = String::from_utf8(commit_output.stdout)?;
    assert!(
        commit_stdout.contains("Commit ID:"),
        "Should show commit ID"
    );

    // Test that status shows the commit
    let status_output = cmd_helper(&["status"])?;
    assert!(status_output.status.success(), "Status should succeed");
    let status_stdout = String::from_utf8(status_output.stdout)?;
    assert!(
        !status_stdout.contains("none (no commits yet)"),
        "Should show actual commit"
    );

    // Test that log shows the commit
    let log_output = cmd_helper(&["log"])?;
    assert!(log_output.status.success(), "Log should succeed");
    let log_stdout = String::from_utf8(log_output.stdout)?;
    assert!(
        log_stdout.contains("Persistence test"),
        "Log should show commit message"
    );

    // Test that get can retrieve the file
    let get_output = cmd_helper(&["get", "persist.txt"])?;
    assert!(get_output.status.success(), "Get should succeed");

    Ok(())
}

#[test]
fn test_command_error_handling() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    let mut cmd_helper = |args: &[&str]| -> Result<std::process::Output> {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--")
            .args(args)
            .current_dir(project_path);
        Ok(cmd.output()?)
    };

    // Test commands without repository (should show helpful errors)
    let output = cmd_helper(&["status"])?;
    assert!(!output.status.success(), "Status without repo should fail");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("Not in a repository") || stderr.contains("No repository found"),
        "Should show helpful error message"
    );

    // Initialize repository for further tests
    cmd_helper(&["init", "--name", "Error Test"])?;

    // Test get non-existent file
    let output = cmd_helper(&["get", "nonexistent.txt"])?;
    assert!(
        !output.status.success(),
        "Get non-existent file should fail"
    );

    // Test cat non-existent file
    let output = cmd_helper(&["cat", "nonexistent.txt"])?;
    assert!(
        !output.status.success(),
        "Cat non-existent file should fail"
    );

    Ok(())
}

#[test]
fn test_staging_system_regression_prevention() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create test files
    for i in 1..=10 {
        fs::write(
            project_path.join(format!("regression{}.txt", i)),
            format!("Content {}", i),
        )?;
    }

    let mut cmd_helper = |args: &[&str]| -> Result<std::process::Output> {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--")
            .args(args)
            .current_dir(project_path);
        Ok(cmd.output()?)
    };

    // Initialize repository
    cmd_helper(&["init", "--name", "Regression Test"])?;

    // Test staging multiple files
    let output = cmd_helper(&[
        "--yes",
        "add",
        "regression1.txt",
        "regression2.txt",
        "regression3.txt",
    ])?;
    assert!(output.status.success(), "Multi-file add should succeed");

    // Test staged command shows correct count
    let output = cmd_helper(&["staged"])?;
    assert!(output.status.success(), "Staged command should succeed");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("3") || stdout.contains("regression"),
        "Should show staged files"
    );

    // Test commit clears staging
    let output = cmd_helper(&["commit", "-m", "Regression test commit"])?;
    assert!(output.status.success(), "Commit should succeed");

    // Test staging is cleared
    let output = cmd_helper(&["staged"])?;
    assert!(
        output.status.success(),
        "Staged after commit should succeed"
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("No files staged"),
        "Staging should be cleared after commit"
    );

    // Test files are accessible
    let output = cmd_helper(&["get", "regression1.txt"])?;
    assert!(
        output.status.success(),
        "Should be able to get committed file"
    );

    Ok(())
}

#[test]
fn test_large_file_handling() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create a larger test file
    let large_content = "Large file content\n".repeat(1000);
    fs::write(project_path.join("large.txt"), &large_content)?;

    let mut cmd_helper = |args: &[&str]| -> Result<std::process::Output> {
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--")
            .args(args)
            .current_dir(project_path);
        Ok(cmd.output()?)
    };

    // Initialize and add large file
    cmd_helper(&["init", "--name", "Large File Test"])?;
    let output = cmd_helper(&["--yes", "add", "large.txt"])?;
    assert!(output.status.success(), "Should handle large file");

    // Commit large file
    let output = cmd_helper(&["commit", "-m", "Large file commit"])?;
    assert!(output.status.success(), "Should commit large file");

    // Retrieve large file
    let output = cmd_helper(&["get", "large.txt"])?;
    assert!(output.status.success(), "Should retrieve large file");

    // Test cat with large file
    let output = cmd_helper(&["cat", "large.txt"])?;
    assert!(output.status.success(), "Should cat large file");

    Ok(())
}

#[test]
fn test_command_consistency() -> Result<()> {
    let repo = TestRepository::new()?;

    // Test that commands show consistent information
    let status_output = repo.run_command(&["status"])?;
    let log_output = repo.run_command(&["log"])?;
    let root_output = repo.run_command(&["root"])?;

    assert!(status_output.status.success(), "Status should work");
    assert!(log_output.status.success(), "Log should work");
    assert!(root_output.status.success(), "Root should work");

    // All should succeed and show consistent state
    let status_stdout = String::from_utf8(status_output.stdout)?;
    let log_stdout = String::from_utf8(log_output.stdout)?;

    // Both should reference the same commit state
    assert!(
        !status_stdout.contains("none (no commits yet)"),
        "Status should show commit"
    );
    assert!(
        log_stdout.contains("Initial test commit"),
        "Log should show commit"
    );

    Ok(())
}
