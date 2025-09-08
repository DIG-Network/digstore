//! CLI User Validation Tests
//!
//! These tests validate that the CLI behaves correctly from a user's perspective,
//! testing actual command execution, output formatting, and error handling.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Test the complete new user onboarding experience
#[test]
fn test_new_user_onboarding() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create a simple project
    fs::write(project_path.join("hello.txt"), "Hello, Digstore!").unwrap();

    // User runs digstore for the first time
    let mut cmd = Command::cargo_bin("digstore").unwrap();
    cmd.current_dir(project_path);

    // 1. User gets help
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Digstore Min"))
        .stdout(predicate::str::contains("Content-addressable storage"))
        .stdout(predicate::str::contains("init"));

    // 2. User tries status before init (should get helpful error)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"))
        .stderr(predicate::str::contains("init"));

    // 3. User initializes repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository initialized"))
        .stdout(predicate::str::contains("Store ID:"))
        .stdout(predicate::str::contains("✓"));

    // 4. User checks status (should be clean)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"))
        .stdout(predicate::str::contains("No changes staged"));

    // 5. User adds file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "hello.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"))
        .stdout(predicate::str::contains("✓"));

    // 6. User checks what's staged
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("hello.txt"))
        .stdout(predicate::str::contains("commit"));

    // 7. User commits
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "My first commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"))
        .stdout(predicate::str::contains("✓"));

    // 8. User verifies commit
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("My first commit"));

    // 9. User retrieves file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "hello.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, Digstore!"));
}

#[test]
fn test_collaborative_workflow() {
    // Simulate multiple users working with the same repository
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create project files
    fs::write(project_path.join("shared.txt"), "Shared document").unwrap();
    fs::write(project_path.join("user1.txt"), "User 1 file").unwrap();
    fs::write(project_path.join("user2.txt"), "User 2 file").unwrap();

    // User 1 initializes project
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Collaborative Project"])
        .assert()
        .success();

    // User 1 adds their files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "shared.txt", "user1.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "User 1 initial commit"])
        .assert()
        .success();

    // User 2 adds their files to the same repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "user2.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "User 2 contribution"])
        .assert()
        .success();

    // Both users can access all files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "user1.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("User 1 file"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "user2.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("User 2 file"));

    // History shows both contributions
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("User 1 initial commit"))
        .stdout(predicate::str::contains("User 2 contribution"));
}

#[test]
fn test_file_versioning_user_story() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create initial version
    fs::write(
        project_path.join("document.txt"),
        "Version 1.0\nInitial content",
    )
    .unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Version Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "document.txt"])
        .assert()
        .success();

    let v1_commit = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Version 1.0"])
        .assert()
        .success()
        .get_output();

    // Extract commit ID for later use
    let v1_stdout = String::from_utf8_lossy(&v1_commit.stdout);
    let commit_line = v1_stdout
        .lines()
        .find(|line| line.contains("Commit ID:"))
        .expect("Should have commit ID");
    let v1_hash = commit_line.split_whitespace()
        .last()
        .expect("Should have hash")
        .trim_end_matches('\u{1b}') // Remove color codes
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>();

    // User updates document
    fs::write(
        project_path.join("document.txt"),
        "Version 2.0\nUpdated content\nNew features",
    )
    .unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "document.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Version 2.0"])
        .assert()
        .success();

    // User can access current version
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "document.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Version 2.0"))
        .stdout(predicate::str::contains("New features"));

    // User can access historical version
    if v1_hash.len() >= 8 {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "document.txt", "--at", &v1_hash[..8]])
            .assert()
            .success()
            .stdout(predicate::str::contains("Version 1.0"));
    }

    // User can see version history
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Version 1.0"))
        .stdout(predicate::str::contains("Version 2.0"));
}

#[test]
fn test_repository_analysis_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create repository with diverse content
    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::create_dir_all(project_path.join("assets")).unwrap();

    // Different file types and sizes
    fs::write(project_path.join("src/small.rs"), "fn main() {}").unwrap();
    fs::write(project_path.join("src/medium.rs"), "// ".repeat(1000)).unwrap();
    fs::write(project_path.join("assets/data.bin"), vec![0u8; 10000]).unwrap();
    fs::write(
        project_path.join("README.md"),
        "# Project\n\nDescription here.",
    )
    .unwrap();

    // Setup repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Analysis Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Diverse content"])
        .assert()
        .success();

    // User analyzes repository size
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["size", "--breakdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"))
        .stdout(predicate::str::contains("Layer Files"));

    // User checks efficiency metrics
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["size", "--efficiency"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Efficiency Metrics"));

    // User gets detailed statistics
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["stats", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Growth Metrics"));

    // User inspects layers
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["layers", "--list", "--size"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Layer List"));
}

#[test]
fn test_data_integrity_user_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create important document
    fs::write(project_path.join("important.txt"), "Critical business data").unwrap();

    // Setup repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Integrity Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "important.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Important data"])
        .assert()
        .success();

    // User generates proof for data integrity
    let proof_file = project_path.join("proof.json");
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["prove", "important.txt", "-o", proof_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof generated"))
        .stdout(predicate::str::contains("written to"));

    // Verify proof file was created
    assert!(proof_file.exists());
    let proof_content = fs::read_to_string(&proof_file).unwrap();
    assert!(proof_content.contains("proof_type"));
    assert!(proof_content.contains("target"));

    // User verifies proof
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["verify", proof_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof verification successful"))
        .stdout(predicate::str::contains("✓"));
}

#[test]
fn test_troubleshooting_user_scenarios() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Scenario 1: User in wrong directory
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"));

    // Scenario 2: User tries to add non-existent file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "missing.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    // Scenario 3: User tries to commit without staging
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No files staged"))
        .stderr(predicate::str::contains("add"));

    // Scenario 4: User gets help for specific command
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Add files"))
        .stdout(predicate::str::contains("--recursive"));
}

#[test]
fn test_output_formatting_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test files
    fs::write(project_path.join("format.txt"), "Formatting test").unwrap();

    // Setup repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Format Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "format.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Format test"])
        .assert()
        .success();

    // Test consistent formatting across commands
    let commands_with_formatting = [
        (vec!["status"], vec!["Repository Status", "═"]),
        (vec!["log"], vec!["Commit History", "═"]),
        (vec!["root"], vec!["Current Root Information", "═"]),
        (vec!["size"], vec!["Storage Analytics", "═"]),
        (vec!["stats"], vec!["Repository Statistics", "═"]),
        (vec!["history"], vec!["Root History Analysis", "═"]),
        (vec!["store-info"], vec!["Store Information", "═"]),
    ];

    for (command, expected_elements) in commands_with_formatting {
        let output = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&command)
            .assert()
            .success()
            .get_output();

        let stdout = String::from_utf8_lossy(&output.stdout);
        for element in expected_elements {
            assert!(
                stdout.contains(element),
                "Command {:?} should contain '{}' in output",
                command,
                element
            );
        }
    }
}

#[test]
fn test_large_file_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create a large file (1MB)
    let large_content = "Large file content line\n".repeat(50000);
    fs::write(project_path.join("large.txt"), &large_content).unwrap();

    // Setup repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Large File Test"])
        .assert()
        .success();

    // User adds large file - should handle efficiently
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "large.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"));

    // User commits large file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Large file commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));

    // User can retrieve large file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "large.txt", "-o", "retrieved_large.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content written to"));

    // Verify retrieved file matches
    let retrieved = fs::read_to_string(project_path.join("retrieved_large.txt")).unwrap();
    assert_eq!(retrieved, large_content);

    // User can analyze storage efficiency
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["size", "--efficiency"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"));
}

#[test]
fn test_configuration_user_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // User explores configuration
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration"));

    // User checks current config (should show defaults or empty)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "--list"])
        .assert()
        .success();

    // User sets their identity (optional)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "user.name", "Test User"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "user.email", "test@example.com"])
        .assert()
        .success();

    // User verifies configuration
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test User"))
        .stdout(predicate::str::contains("test@example.com"));

    // User can get specific values
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "user.name"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test User"));

    // User can see where config is stored
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "--show-origin"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration file"));
}

#[test]
fn test_interactive_features() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test files
    fs::write(project_path.join("interactive.txt"), "Interactive test").unwrap();

    // Test auto-yes flag for non-interactive usage
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "init", "--name", "Interactive Test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository initialized"));

    // Test that commands work with --yes flag
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "interactive.txt"])
        .assert()
        .success();

    // Test dry-run mode
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "--dry-run", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would process"));

    // Test quiet mode
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--quiet", "commit", "-m", "Quiet commit"])
        .assert()
        .success();
}

#[test]
fn test_advanced_user_features() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create complex project structure
    fs::create_dir_all(project_path.join("src/modules")).unwrap();
    fs::create_dir_all(project_path.join("tests/integration")).unwrap();

    for i in 0..20 {
        fs::write(
            project_path
                .join("src/modules")
                .join(format!("mod{}.rs", i)),
            format!("// Module {}", i),
        )
        .unwrap();
    }

    // Setup repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Advanced Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Advanced test setup"])
        .assert()
        .success();

    // Advanced analysis commands
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["size", "--breakdown", "--efficiency", "--layers"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Layer Breakdown"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["stats", "--detailed", "--performance", "--security"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Performance Metrics"))
        .stdout(predicate::str::contains("Security Metrics"));

    // Layer inspection
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["layers", "--list", "--files", "--chunks"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files"))
        .stdout(predicate::str::contains("chunks"));

    // Detailed store information
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["store-info", "--config", "--paths"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration:"))
        .stdout(predicate::str::contains("Paths:"));
}

#[test]
fn test_shell_completion_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test completion generation for all shells
    let shells = ["bash", "zsh", "fish", "powershell"];

    for shell in &shells {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["completion", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("Generating shell completion"))
            .stdout(predicate::str::contains("Installation Instructions"))
            .stdout(predicate::str::contains(shell));
    }

    // Bash completion should include specific instructions
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bashrc"))
        .stdout(predicate::str::contains("eval"));

    // Zsh completion should include zsh-specific instructions
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["completion", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("zshrc"))
        .stdout(predicate::str::contains("fpath"));
}

#[test]
fn test_repository_portability() {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();
    let project1 = temp_dir1.path();
    let project2 = temp_dir2.path();

    // User creates repository in first location
    fs::write(project1.join("portable.txt"), "Portable content").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project1)
        .args(&["init", "--name", "Portable Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project1)
        .args(&["--yes", "add", "portable.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project1)
        .args(&["commit", "-m", "Portable commit"])
        .assert()
        .success();

    // Copy .digstore file to second location (simulating project move)
    fs::copy(project1.join(".digstore"), project2.join(".digstore")).unwrap();
    fs::write(project2.join("portable.txt"), "Portable content").unwrap();

    // User should be able to access repository from new location
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project2)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project2)
        .args(&["get", "portable.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Portable content"));
}

#[test]
fn test_performance_user_feedback() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create many files to trigger performance feedback
    for i in 0..100 {
        fs::write(
            project_path.join(format!("perf{:03}.txt", i)),
            format!("Performance test file {}", i),
        )
        .unwrap();
    }

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Performance Test"])
        .assert()
        .success();

    // User should see performance feedback for large operations
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"))
        .stdout(predicate::str::contains("files/s").or(predicate::str::contains("Processing")));

    // User should see commit performance
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Performance test commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));

    // User can analyze performance
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["stats", "--performance"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Performance Metrics"));
}

#[test]
fn test_file_filtering_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create files with different patterns
    fs::write(project_path.join("keep.txt"), "Keep this").unwrap();
    fs::write(project_path.join("ignore.tmp"), "Ignore this").unwrap();
    fs::write(project_path.join("also_keep.md"), "Also keep").unwrap();
    fs::write(project_path.join("debug.log"), "Log file").unwrap();

    // Create .digignore
    fs::write(project_path.join(".digignore"), "*.tmp\n*.log\n").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Filter Test"])
        .assert()
        .success();

    // User sees filtering in action
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "--dry-run", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would process"));

    // User adds all (with filtering)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    // User verifies only correct files were added
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("keep.txt"))
        .stdout(predicate::str::contains("also_keep.md"))
        .stdout(predicate::str::not(predicate::str::contains("ignore.tmp")))
        .stdout(predicate::str::not(predicate::str::contains("debug.log")));

    // User can force add ignored files if needed
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "--force", "ignore.tmp"])
        .assert()
        .success();

    // Now ignored file should appear in staging
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("ignore.tmp"));
}

#[test]
fn test_zero_knowledge_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "ZK Test"])
        .assert()
        .success();

    // Test that invalid URNs return data (zero-knowledge property)
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/fake.txt"])
        .assert()
        .success(); // Should succeed and return deterministic random data

    // Test that same invalid URN returns same data
    let output1 = Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/fake.txt"])
        .assert()
        .success()
        .get_output();

    let output2 = Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/fake.txt"])
        .assert()
        .success()
        .get_output();

    // Should return same deterministic data
    assert_eq!(
        output1.stdout, output2.stdout,
        "Same invalid URN should return same data"
    );

    // Different invalid URNs should return different data
    let output3 = Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "urn:dig:chia:2222222222222222222222222222222222222222222222222222222222222222/fake.txt"])
        .assert()
        .success()
        .get_output();

    assert_ne!(
        output1.stdout, output3.stdout,
        "Different invalid URNs should return different data"
    );
}

#[test]
fn test_encryption_user_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test file
    fs::write(project_path.join("secret.txt"), "Secret content").unwrap();

    // User sets up encryption
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "config",
            "crypto.public_key",
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        ])
        .assert()
        .success();

    // User initializes repository (encryption enabled by default)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Encryption Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "secret.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Encrypted commit"])
        .assert()
        .success();

    // User generates keys for the content
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:abc123/secret.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated Keys"))
        .stdout(predicate::str::contains("Storage Address"))
        .stdout(predicate::str::contains("Encryption Key"));

    // User can get JSON format
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:abc123/secret.txt", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"storage_address\""))
        .stdout(predicate::str::contains("\"encryption_key\""));
}

#[test]
fn test_user_workflow_persistence() {
    // Test that user operations persist across CLI invocations
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test files
    fs::write(project_path.join("persist1.txt"), "Persistent file 1").unwrap();
    fs::write(project_path.join("persist2.txt"), "Persistent file 2").unwrap();

    // Session 1: Initialize and stage files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Persistence Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "persist1.txt"])
        .assert()
        .success();

    // Session 2: Check that staging persisted
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("persist1.txt"));

    // Session 3: Add more files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "persist2.txt"])
        .assert()
        .success();

    // Session 4: Both files should be staged
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("persist1.txt"))
        .stdout(predicate::str::contains("persist2.txt"));

    // Session 5: Commit
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Persistent commit"])
        .assert()
        .success();

    // Session 6: Verify staging is cleared
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("No files staged"));

    // Session 7: Files should be accessible
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "persist1.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Persistent file 1"));
}
