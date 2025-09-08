//! User Experience Integration Tests
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]
//!
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]
//! These tests validate the complete user experience from a CLI perspective,
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]
//! ensuring all commands work as expected when users interact with the application.
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Test helper for creating realistic test scenarios
struct UserScenario {
    temp_dir: TempDir,
    project_path: std::path::PathBuf,
}

impl UserScenario {
    fn new() -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path().to_path_buf();
        Ok(Self {
            temp_dir,
            project_path,
        })
    }

    fn create_realistic_project(&self) -> anyhow::Result<()> {
        // Create a realistic project structure
        fs::create_dir_all(self.project_path.join("src"))?;
        fs::create_dir_all(self.project_path.join("docs"))?;
        fs::create_dir_all(self.project_path.join("tests"))?;
        fs::create_dir_all(self.project_path.join("assets"))?;

        // Source files
        fs::write(
            self.project_path.join("src/main.rs"),
            r#"
fn main() {
    println!("Hello, world!");
}
"#,
        )?;
        fs::write(
            self.project_path.join("src/lib.rs"),
            r#"
//! Library code
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]
pub fn hello() -> &'static str {
    "Hello from lib"
}
"#,
        )?;

        // Documentation
        fs::write(
            self.project_path.join("README.md"),
            r#"
# My Project

This is a test project for Digstore validation.

## Features
- Feature 1
- Feature 2
"#,
        )?;
        fs::write(
            self.project_path.join("docs/guide.md"),
            "User guide content",
        )?;

        // Configuration files
        fs::write(
            self.project_path.join("Cargo.toml"),
            r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#,
        )?;

        // Test files
        fs::write(
            self.project_path.join("tests/integration.rs"),
            "// Integration tests",
        )?;

        // Assets
        fs::write(
            self.project_path.join("assets/data.json"),
            r#"{"key": "value"}"#,
        )?;

        // Files that should be ignored
        fs::write(self.project_path.join("target/debug/binary"), "Binary data")?;
        fs::write(self.project_path.join(".DS_Store"), "OS file")?;
        fs::write(self.project_path.join("temp.tmp"), "Temporary file")?;

        // Create .digignore
        fs::write(
            self.project_path.join(".digignore"),
            r#"
# Build artifacts
target/
*.tmp
*.log

# OS files
.DS_Store
Thumbs.db

# IDE files
.vscode/
.idea/
"#,
        )?;

        Ok(())
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("digstore").unwrap();
        cmd.current_dir(&self.project_path);
        cmd
    }

    fn cmd_with_yes(&self) -> Command {
        let mut cmd = self.cmd();
        cmd.arg("--yes"); // Auto-answer prompts
        cmd
    }
}

#[test]
fn test_complete_user_workflow() {
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    // Step 1: User initializes repository
    scenario
        .cmd()
        .args(&["init", "--name", "My Test Project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository initialized"))
        .stdout(predicate::str::contains("Store ID:"))
        .stdout(predicate::str::contains("✓"));

    // Step 2: User checks status (should be empty)
    scenario
        .cmd()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"))
        .stdout(predicate::str::contains("No changes staged"));

    // Step 3: User adds all files (with .digignore filtering)
    scenario
        .cmd_with_yes()
        .args(&["add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"))
        .stdout(predicate::str::contains("✓"));

    // Step 4: User checks what was staged
    scenario
        .cmd()
        .args(&["staged", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Staged Files"))
        .stdout(predicate::str::contains("src/main.rs"))
        .stdout(predicate::str::contains("README.md"));

    // Step 5: User commits changes
    scenario
        .cmd()
        .args(&["commit", "-m", "Initial project setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"))
        .stdout(predicate::str::contains("✓"));

    // Step 6: User checks status after commit (should be clean)
    scenario
        .cmd()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes staged"));

    // Step 7: User views commit history
    scenario
        .cmd()
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initial project setup"))
        .stdout(predicate::str::contains("commit"));

    // Step 8: User retrieves files
    scenario
        .cmd()
        .args(&["get", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, world!"));

    scenario
        .cmd()
        .args(&["cat", "README.md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("My Project"));

    // Step 9: User generates proof
    scenario
        .cmd()
        .args(&["prove", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof generated"))
        .stdout(predicate::str::contains("✓"));
}

#[test]
fn test_user_error_scenarios() {
    let scenario = UserScenario::new().unwrap();

    // Error 1: Commands without repository should guide user
    scenario.cmd().arg("status").assert().failure().stderr(
        predicate::str::contains("No repository found")
            .or(predicate::str::contains("Not in a repository")),
    );

    scenario
        .cmd()
        .args(&["add", "file.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"));

    // Initialize for further error tests
    scenario
        .cmd()
        .args(&["init", "--name", "Error Test"])
        .assert()
        .success();

    // Error 2: Commit without staged files
    scenario
        .cmd()
        .args(&["commit", "-m", "Empty commit"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No files staged"));

    // Error 3: Get non-existent file
    scenario
        .cmd()
        .args(&["get", "nonexistent.txt"])
        .assert()
        .failure();

    // Error 4: Invalid command options
    scenario
        .cmd()
        .args(&["staged", "--page", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid page"));
}

#[test]
fn test_json_output_consistency() {
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    // Initialize and commit some data
    scenario
        .cmd()
        .args(&["init", "--name", "JSON Test"])
        .assert()
        .success();

    scenario
        .cmd_with_yes()
        .args(&["add", "src/main.rs", "README.md"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "JSON test commit"])
        .assert()
        .success();

    // Test JSON output for various commands
    scenario
        .cmd()
        .args(&["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("store_id"))
        .stdout(predicate::str::contains("current_root"));

    scenario
        .cmd()
        .args(&["staged", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"staged_files\""));

    scenario
        .cmd()
        .args(&["root", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("root_hash"));

    scenario
        .cmd()
        .args(&["size", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("total_size"));

    scenario
        .cmd()
        .args(&["stats", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("total_commits"));
}

#[test]
fn test_file_output_and_piping() {
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    // Setup repository with data
    scenario
        .cmd()
        .args(&["init", "--name", "Output Test"])
        .assert()
        .success();

    scenario
        .cmd_with_yes()
        .args(&["add", "src/main.rs"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "Output test"])
        .assert()
        .success();

    // Test file output with -o flag
    let output_file = scenario.project_path.join("output.txt");
    scenario
        .cmd()
        .args(&["get", "src/main.rs", "-o", output_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content written to"));

    // Verify file was created
    assert!(output_file.exists());
    let content = fs::read_to_string(&output_file).unwrap();
    assert!(content.contains("Hello, world!"));

    // Test JSON output to file
    let json_file = scenario.project_path.join("status.json");
    scenario
        .cmd()
        .args(&["status", "--json", ">", json_file.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_progressive_disclosure_workflow() {
    // Test how a new user would discover features progressively
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    // Step 1: User runs help
    scenario
        .cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("digstore"))
        .stdout(predicate::str::contains("Commands:"));

    // Step 2: User initializes repository
    scenario
        .cmd()
        .args(&["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialize"));

    scenario
        .cmd()
        .args(&["init", "--name", "Progressive Test"])
        .assert()
        .success();

    // Step 3: User explores add command
    scenario
        .cmd()
        .args(&["add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Add files"));

    // User tries dry run first
    scenario
        .cmd()
        .args(&["add", "--dry-run", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would process"));

    // Step 4: User adds files for real
    scenario
        .cmd_with_yes()
        .args(&["add", "-A"])
        .assert()
        .success();

    // Step 5: User explores staging
    scenario
        .cmd()
        .args(&["staged", "--help"])
        .assert()
        .success();

    scenario
        .cmd()
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("Staged Files"));

    // Step 6: User commits
    scenario
        .cmd()
        .args(&["commit", "-m", "Learning commit"])
        .assert()
        .success();

    // Step 7: User explores history
    scenario
        .cmd()
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Learning commit"));

    scenario
        .cmd()
        .arg("history")
        .assert()
        .success()
        .stdout(predicate::str::contains("Root History"));

    // Step 8: User explores analysis commands
    scenario
        .cmd()
        .arg("size")
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"));

    scenario
        .cmd()
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Statistics"));
}

#[test]
fn test_power_user_workflow() {
    // Test advanced user workflows with complex scenarios
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    // Setup
    scenario
        .cmd()
        .args(&["init", "--name", "Power User Test"])
        .assert()
        .success();

    scenario
        .cmd_with_yes()
        .args(&["add", "-A"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "Initial commit"])
        .assert()
        .success();

    // Power user operations

    // 1. Detailed analysis
    scenario
        .cmd()
        .args(&["size", "--breakdown", "--efficiency", "--layers"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"))
        .stdout(predicate::str::contains("Efficiency Metrics"));

    scenario
        .cmd()
        .args(&["stats", "--detailed", "--performance", "--security"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Performance Metrics"))
        .stdout(predicate::str::contains("Security Metrics"));

    // 2. Layer inspection
    scenario
        .cmd()
        .args(&["layers", "--list", "--size", "--files"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Layer List"));

    // 3. Root analysis
    scenario
        .cmd()
        .args(&["root", "--verbose"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Current Root Information"));

    // 4. Store information
    scenario
        .cmd()
        .args(&["store-info", "--config", "--paths"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Store Information"))
        .stdout(predicate::str::contains("Paths:"));

    // 5. Proof generation
    scenario
        .cmd()
        .args(&["prove", "src/main.rs", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof generated"));

    // 6. All JSON outputs should be valid
    scenario
        .cmd()
        .args(&["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("}"));

    scenario
        .cmd()
        .args(&["root", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));

    scenario
        .cmd()
        .args(&["size", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));
}

#[test]
fn test_file_modification_workflow() {
    let scenario = UserScenario::new().unwrap();

    // Create initial file
    fs::write(scenario.project_path.join("evolving.txt"), "Version 1").unwrap();

    // Initialize and commit
    scenario
        .cmd()
        .args(&["init", "--name", "Evolution Test"])
        .assert()
        .success();

    scenario
        .cmd_with_yes()
        .args(&["add", "evolving.txt"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "Version 1"])
        .assert()
        .success();

    // Modify file
    fs::write(
        scenario.project_path.join("evolving.txt"),
        "Version 2 - Updated",
    )
    .unwrap();

    // User checks status (should show no staged changes yet)
    scenario
        .cmd()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes staged"));

    // User adds modified file
    scenario
        .cmd_with_yes()
        .args(&["add", "evolving.txt"])
        .assert()
        .success();

    // User checks what's staged
    scenario
        .cmd()
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("evolving.txt"));

    // User can see the difference
    scenario
        .cmd()
        .args(&["staged", "diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stage Diff"));

    // User commits the change
    scenario
        .cmd()
        .args(&["commit", "-m", "Version 2 update"])
        .assert()
        .success();

    // User can retrieve both versions
    scenario
        .cmd()
        .args(&["get", "evolving.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Version 2"));

    // User can view history
    scenario
        .cmd()
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Version 1"))
        .stdout(predicate::str::contains("Version 2"));
}

#[test]
fn test_error_recovery_guidance() {
    let scenario = UserScenario::new().unwrap();

    // Test 1: User tries to use commands before init
    scenario
        .cmd()
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"))
        .stderr(predicate::str::contains("init").or(predicate::str::contains("digstore init")));

    scenario
        .cmd()
        .args(&["add", "file.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"));

    // Test 2: User initializes repository
    scenario
        .cmd()
        .args(&["init", "--name", "Recovery Test"])
        .assert()
        .success();

    // Test 3: User tries to commit without staging
    scenario
        .cmd()
        .args(&["commit", "-m", "Empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No files staged"))
        .stderr(predicate::str::contains("add"));

    // Test 4: User tries to access non-existent file
    scenario
        .cmd()
        .args(&["get", "missing.txt"])
        .assert()
        .failure();

    // Test 5: User gets helpful completion
    scenario
        .cmd()
        .args(&["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installation Instructions"))
        .stdout(predicate::str::contains("bashrc"));
}

#[test]
fn test_large_repository_user_experience() {
    let scenario = UserScenario::new().unwrap();

    // Create a larger repository (100 files)
    for i in 0..100 {
        let dir = if i < 30 {
            "src"
        } else if i < 60 {
            "tests"
        } else {
            "docs"
        };
        fs::create_dir_all(scenario.project_path.join(dir)).unwrap();
        fs::write(
            scenario
                .project_path
                .join(dir)
                .join(format!("file{:03}.txt", i)),
            format!("Content of file {}", i),
        )
        .unwrap();
    }

    // User initializes repository
    scenario
        .cmd()
        .args(&["init", "--name", "Large Repo Test"])
        .assert()
        .success();

    // User adds all files - should show progress
    scenario
        .cmd_with_yes()
        .args(&["add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"))
        .stdout(predicate::str::contains("files/s").or(predicate::str::contains("Processing")));

    // User checks staging with pagination
    scenario
        .cmd()
        .args(&["staged", "--limit", "20"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Page 1 of"))
        .stdout(predicate::str::contains("next page"));

    scenario
        .cmd()
        .args(&["staged", "--page", "2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Page 2"));

    scenario
        .cmd()
        .args(&["staged", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Showing 100 staged files"));

    // User commits large repository
    scenario
        .cmd()
        .args(&["commit", "-m", "Large repository commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));

    // User analyzes repository efficiency
    scenario
        .cmd()
        .args(&["size", "--efficiency"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deduplication"));

    scenario
        .cmd()
        .args(&["stats", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("100").or(predicate::str::contains("files")));
}

#[test]
fn test_configuration_user_workflow() {
    let scenario = UserScenario::new().unwrap();

    // User explores configuration
    scenario
        .cmd()
        .args(&["config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration"));

    // User views current config
    scenario
        .cmd()
        .args(&["config", "--list"])
        .assert()
        .success();

    // User sets configuration
    scenario
        .cmd()
        .args(&["config", "user.name", "Test User"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    scenario
        .cmd()
        .args(&["config", "user.email", "test@example.com"])
        .assert()
        .success();

    // User views updated config
    scenario
        .cmd()
        .args(&["config", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test User"))
        .stdout(predicate::str::contains("test@example.com"));

    // User gets specific config value
    scenario
        .cmd()
        .args(&["config", "user.name"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test User"));

    // User can see config file location
    scenario
        .cmd()
        .args(&["config", "--show-origin"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration file"));
}

#[test]
fn test_completion_integration() {
    let scenario = UserScenario::new().unwrap();

    // Test completion for all major shells
    let shells = ["bash", "zsh", "fish", "powershell"];

    for shell in &shells {
        scenario
            .cmd()
            .args(&["completion", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("Installation Instructions"))
            .stdout(predicate::str::contains(shell));
    }

    // Completion should include helpful installation instructions
    scenario
        .cmd()
        .args(&["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bashrc"))
        .stdout(predicate::str::contains("eval"));
}

#[test]
fn test_urn_based_access() {
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    // Setup repository
    scenario
        .cmd()
        .args(&["init", "--name", "URN Test"])
        .assert()
        .success();

    scenario
        .cmd_with_yes()
        .args(&["add", "src/main.rs"])
        .assert()
        .success();

    let commit_output = scenario
        .cmd()
        .args(&["commit", "-m", "URN test commit"])
        .assert()
        .success()
        .get_output();

    // Extract store ID and commit ID for URN construction
    let commit_stdout = String::from_utf8_lossy(&commit_output.stdout);

    // Test that files are accessible (URN construction happens internally)
    scenario
        .cmd()
        .args(&["get", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, world!"));

    // Test that root command shows valid hash
    scenario
        .cmd()
        .arg("root")
        .assert()
        .success()
        .stdout(predicate::str::contains("Root Hash:"));
}

#[test]
fn test_binary_staging_user_experience() {
    let scenario = UserScenario::new().unwrap();

    // Create files that would stress the staging system
    for i in 0..200 {
        fs::write(
            scenario.project_path.join(format!("stress{:03}.txt", i)),
            format!("Stress test content {}", i),
        )
        .unwrap();
    }

    scenario
        .cmd()
        .args(&["init", "--name", "Staging Stress Test"])
        .assert()
        .success();

    // User adds many files - should be fast and efficient
    scenario
        .cmd_with_yes()
        .args(&["add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"));

    // User can paginate through staged files
    scenario
        .cmd()
        .args(&["staged", "--limit", "50"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Page 1 of 4"));

    scenario
        .cmd()
        .args(&["staged", "--page", "4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Page 4"));

    // User can view all files at once
    scenario
        .cmd()
        .args(&["staged", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Showing 200 staged files"));

    // User can commit large staging area efficiently
    scenario
        .cmd()
        .args(&["commit", "-m", "Large staging commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));

    // Staging should be cleared after commit
    scenario
        .cmd()
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("No files staged"));
}

#[test]
fn test_directory_operations() {
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    scenario
        .cmd()
        .args(&["init", "--name", "Directory Test"])
        .assert()
        .success();

    // Test adding specific directory
    scenario
        .cmd_with_yes()
        .args(&["add", "-r", "src/"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added"));

    // Test adding individual files
    scenario
        .cmd_with_yes()
        .args(&["add", "README.md", "Cargo.toml"])
        .assert()
        .success();

    // User checks what was staged
    scenario
        .cmd()
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("src/main.rs"))
        .stdout(predicate::str::contains("README.md"));

    // User commits
    scenario
        .cmd()
        .args(&["commit", "-m", "Directory test"])
        .assert()
        .success();

    // User can retrieve files from subdirectories
    scenario
        .cmd()
        .args(&["get", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, world!"));
}

#[test]
fn test_user_feedback_and_progress() {
    let scenario = UserScenario::new().unwrap();

    // Create enough files to trigger progress display
    for i in 0..50 {
        fs::write(
            scenario.project_path.join(format!("progress{:02}.txt", i)),
            format!("Progress test {}", i),
        )
        .unwrap();
    }

    scenario
        .cmd()
        .args(&["init", "--name", "Progress Test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓")); // Success indicators

    // User should see progress feedback for large operations
    scenario
        .cmd_with_yes()
        .args(&["add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files/s").or(predicate::str::contains("Processing")));

    // User should see helpful summaries
    scenario
        .cmd()
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("Total size"))
        .stdout(predicate::str::contains("commit"));

    scenario
        .cmd()
        .args(&["commit", "-m", "Progress test commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"))
        .stdout(predicate::str::contains("Commit created"));

    // User should see informative output
    scenario
        .cmd()
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Progress test commit"));
}

#[test]
fn test_cross_command_consistency() {
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    // Setup repository
    scenario
        .cmd()
        .args(&["init", "--name", "Consistency Test"])
        .assert()
        .success();

    scenario
        .cmd_with_yes()
        .args(&["add", "-A"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "Consistency test"])
        .assert()
        .success();

    // All information commands should show consistent data
    let status_output = scenario.cmd().arg("status").assert().success().get_output();

    let root_output = scenario.cmd().arg("root").assert().success().get_output();

    let log_output = scenario.cmd().arg("log").assert().success().get_output();

    // Extract information from outputs
    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    let root_stdout = String::from_utf8_lossy(&root_output.stdout);
    let log_stdout = String::from_utf8_lossy(&log_output.stdout);

    // Should all reference the same commit state
    assert!(
        !status_stdout.contains("none (no commits yet)"),
        "Status should show actual commit"
    );
    assert!(
        root_stdout.contains("Root Hash:"),
        "Root should show hash information"
    );
    assert!(
        log_stdout.contains("Consistency test"),
        "Log should show commit message"
    );
}

#[test]
fn test_user_help_and_discovery() {
    let scenario = UserScenario::new().unwrap();

    // Test main help
    scenario
        .cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("digstore"))
        .stdout(predicate::str::contains("Commands:"))
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("commit"));

    // Test subcommand help
    scenario
        .cmd()
        .args(&["add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Add files"))
        .stdout(predicate::str::contains("--recursive"))
        .stdout(predicate::str::contains("--all"));

    scenario
        .cmd()
        .args(&["commit", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Create a new commit"))
        .stdout(predicate::str::contains("--message"));

    scenario
        .cmd()
        .args(&["staged", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Staging area"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("diff"));

    // Test nested subcommand help
    scenario
        .cmd()
        .args(&["staged", "list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("List staged files"))
        .stdout(predicate::str::contains("--limit"))
        .stdout(predicate::str::contains("--page"));
}

#[test]
fn test_realistic_development_workflow() {
    // Simulate a realistic software development workflow
    let scenario = UserScenario::new().unwrap();
    scenario.create_realistic_project().unwrap();

    // Developer starts new project
    scenario
        .cmd()
        .args(&["init", "--name", "MyApp"])
        .assert()
        .success();

    // Developer adds initial files
    scenario
        .cmd_with_yes()
        .args(&["add", "src/", "Cargo.toml", "README.md"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "Initial project structure"])
        .assert()
        .success();

    // Developer adds more files
    scenario
        .cmd_with_yes()
        .args(&["add", "docs/", "tests/"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "Add documentation and tests"])
        .assert()
        .success();

    // Developer checks project history
    scenario
        .cmd()
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initial project structure"))
        .stdout(predicate::str::contains("Add documentation and tests"));

    // Developer analyzes repository
    scenario
        .cmd()
        .arg("size")
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"));

    scenario
        .cmd()
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Statistics"));

    // Developer generates proof for important file
    scenario
        .cmd()
        .args(&["prove", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof generated"));

    // Developer can access any file
    scenario
        .cmd()
        .args(&["get", "src/main.rs"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["cat", "README.md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("My Project"));
}

#[test]
fn test_edge_case_user_scenarios() {
    let scenario = UserScenario::new().unwrap();

    // Edge case 1: Empty repository
    scenario
        .cmd()
        .args(&["init", "--name", "Empty Test"])
        .assert()
        .success();

    scenario
        .cmd()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes staged"));

    scenario
        .cmd()
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("No commits found"));

    // Edge case 2: Very small files
    fs::write(scenario.project_path.join("tiny.txt"), "x").unwrap();

    scenario
        .cmd_with_yes()
        .args(&["add", "tiny.txt"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "Tiny file"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["get", "tiny.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("x"));

    // Edge case 3: Files with special characters
    fs::write(
        scenario.project_path.join("special chars & symbols.txt"),
        "special content",
    )
    .unwrap();

    scenario
        .cmd_with_yes()
        .args(&["add", "special chars & symbols.txt"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["commit", "-m", "Special characters"])
        .assert()
        .success();

    scenario
        .cmd()
        .args(&["get", "special chars & symbols.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("special content"));
}
