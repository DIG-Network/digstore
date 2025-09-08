//! Fast User-Centric Tests
//!
//! These tests focus on user workflows but are optimized for speed (< 3 minutes total).
//! They use smaller datasets and timeouts to ensure reliable CI performance.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
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

/// Create a minimal test project quickly
fn create_minimal_project(project_path: &std::path::Path) -> std::io::Result<()> {
    fs::write(project_path.join("test.txt"), "Hello World")?;
    fs::write(project_path.join("data.json"), r#"{"key": "value"}"#)?;
    Ok(())
}

#[test]
fn test_basic_user_workflow_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        create_minimal_project(project_path).unwrap();

        // 1. Init (should be fast)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Fast Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success()
            .stdout(predicate::str::contains("Repository initialized"));

        // 2. Status (should be instant)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("status")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Repository Status"));

        // 3. Add files (should be fast for 2 files)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "test.txt", "data.json"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success()
            .stdout(predicate::str::contains("files added to staging"));

        // 4. Check staging (should be instant)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("staged")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("test.txt"));

        // 5. Commit (should be fast for 2 files)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Fast test commit"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success()
            .stdout(predicate::str::contains("Commit created"));

        // 6. Get file (should be instant)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "test.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Hello World"));
    });
}

#[test]
fn test_error_handling_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Test commands without repository
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("status")
            .timeout(Duration::from_secs(5))
            .assert()
            .failure()
            .stderr(predicate::str::contains("repository"));

        // Init for other tests
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Error Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        // Test commit without staging
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Empty"])
            .timeout(Duration::from_secs(5))
            .assert()
            .failure()
            .stderr(predicate::str::contains("No files staged"));

        // Test get non-existent file
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "missing.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .failure();
    });
}

#[test]
fn test_json_output_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        create_minimal_project(project_path).unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "JSON Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "test.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "JSON commit"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Test JSON outputs (should be instant)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["status", "--json"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("{"))
            .stdout(predicate::str::contains("store_id"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["staged", "--json"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("staged_files"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["root", "--json"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("{"));
    });
}

#[test]
fn test_small_scale_add_all_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Create 10 small files (not 500+)
        for i in 0..10 {
            fs::write(
                project_path.join(format!("file{:02}.txt", i)),
                format!("Content {}", i),
            )
            .unwrap();
        }

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Add All Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        // Test add -A with small number of files
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "-A"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success()
            .stdout(predicate::str::contains("files added to staging"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["staged", "--all"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("10").or(predicate::str::contains("file")));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Small scale commit"])
            .timeout(Duration::from_secs(20))
            .assert()
            .success();
    });
}

#[test]
fn test_configuration_workflow_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Test config commands (should be instant)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["config", "--list"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["config", "user.name", "Fast Test User"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("✓"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["config", "user.name"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Fast Test User"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["config", "--show-origin"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Configuration file"));
    });
}

#[test]
fn test_analysis_commands_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        create_minimal_project(project_path).unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Analysis Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "-A"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Analysis commit"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Test analysis commands (should be fast for small repo)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("root")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Root Hash"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("history")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Root History"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("size")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Storage Analytics"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("stats")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Repository Statistics"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["store-info"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Store Information"));
    });
}

#[test]
fn test_layer_commands_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        create_minimal_project(project_path).unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Layer Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "test.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Layer test"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Test layer commands (should be fast for single layer)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["layers", "--list"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Layer"));
    });
}

#[test]
fn test_proof_workflow_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        create_minimal_project(project_path).unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Proof Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "test.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Proof test"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Test proof generation (should be fast for small file)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["prove", "test.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success()
            .stdout(predicate::str::contains("Proof generated"));

        // Test proof with file output
        let proof_file = project_path.join("test_proof.json");
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["prove", "test.txt", "-o", proof_file.to_str().unwrap()])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        // Verify proof file exists and is valid
        assert!(proof_file.exists());
        let proof_content = fs::read_to_string(&proof_file).unwrap();
        assert!(proof_content.contains("proof_type"));

        // Test proof verification
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["verify", proof_file.to_str().unwrap()])
            .timeout(Duration::from_secs(10))
            .assert()
            .success()
            .stdout(predicate::str::contains("Proof verification successful"));
    });
}

#[test]
fn test_zero_knowledge_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Test invalid URNs return data quickly
        let invalid_urns = [
            "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/fake1.txt",
            "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/fake2.txt",
            "malformed-urn-format",
        ];

        for urn in &invalid_urns {
            Command::cargo_bin("digstore")
                .unwrap()
                .current_dir(project_path)
                .args(&["get", urn])
                .timeout(Duration::from_secs(10))
                .assert()
                .success(); // Should always return data, never errors
        }

        // Test deterministic behavior
        let test_urn =
            "urn:dig:chia:abcd1234567890abcdef1234567890abcdef1234567890abcdef1234567890/test.txt";

        let output1 = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", test_urn])
            .timeout(Duration::from_secs(10))
            .assert()
            .success()
            .get_output();

        let output2 = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", test_urn])
            .timeout(Duration::from_secs(10))
            .assert()
            .success()
            .get_output();

        assert_eq!(output1.stdout, output2.stdout, "Should be deterministic");
    });
}

#[test]
fn test_encryption_setup_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Test encryption configuration (should be instant)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&[
                "config",
                "crypto.public_key",
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            ])
            .timeout(Duration::from_secs(5))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["config", "crypto.public_key"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("1234567890abcdef"));

        // Test keygen (should be fast)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["keygen", "urn:dig:chia:test123/file.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Generated Keys"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["keygen", "urn:dig:chia:test123/file.txt", "--json"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("storage_address"));
    });
}

#[test]
fn test_help_system_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Test help commands (should be instant)
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("--help")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Digstore Min"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--help"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Initialize"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["add", "--help"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Add files"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "--help"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Create a new commit"));
    });
}

#[test]
fn test_completion_generation_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Test completion generation (should be fast)
        let shells = ["bash", "zsh", "fish"];

        for shell in &shells {
            Command::cargo_bin("digstore")
                .unwrap()
                .current_dir(project_path)
                .args(&["completion", shell])
                .timeout(Duration::from_secs(5))
                .assert()
                .success()
                .stdout(predicate::str::contains("Installation Instructions"));
        }
    });
}

#[test]
fn test_file_operations_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Create test files with different content
        fs::write(project_path.join("text.txt"), "Text file content").unwrap();
        fs::write(project_path.join("binary.bin"), vec![0u8, 255u8, 128u8]).unwrap();
        fs::write(project_path.join("empty.txt"), "").unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "File Ops Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "-A"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "File operations test"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Test retrieving different file types
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "text.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Text file"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["cat", "text.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Text file"));

        // Test file output
        let output_file = project_path.join("output.txt");
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "text.txt", "-o", output_file.to_str().unwrap()])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Content written to"));

        assert!(output_file.exists());
    });
}

#[test]
fn test_staging_operations_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Create 5 test files
        for i in 0..5 {
            fs::write(
                project_path.join(format!("stage{}.txt", i)),
                format!("Staging content {}", i),
            )
            .unwrap();
        }

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Staging Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        // Test progressive staging
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "stage0.txt", "stage1.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("staged")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("stage0.txt"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "stage2.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["staged", "--detailed"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Hash"))
            .stdout(predicate::str::contains("Size"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["staged", "diff"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Stage Diff"));

        // Test staging clear
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["staged", "clear", "--force"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Cleared"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("staged")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("No files staged"));
    });
}

#[test]
fn test_digignore_functionality_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Create .digignore and test files
        fs::write(project_path.join(".digignore"), "*.tmp\n*.log\n").unwrap();
        fs::write(project_path.join("keep.txt"), "Keep this").unwrap();
        fs::write(project_path.join("ignore.tmp"), "Ignore this").unwrap();
        fs::write(project_path.join("debug.log"), "Log file").unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Ignore Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        // Test that .digignore filtering works
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "-A"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        let staged_output = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("staged")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .get_output();

        let staged_stdout = String::from_utf8_lossy(&staged_output.stdout);
        assert!(staged_stdout.contains("keep.txt"));
        assert!(!staged_stdout.contains("ignore.tmp"));
        assert!(!staged_stdout.contains("debug.log"));

        // Test force add ignored files
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["add", "--force", "ignore.tmp"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success();
    });
}

#[test]
fn test_medium_scale_performance() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Create 50 files (medium scale, not 500+)
        for i in 0..50 {
            fs::write(
                project_path.join(format!("medium{:02}.txt", i)),
                format!("Medium scale content {}", i),
            )
            .unwrap();
        }

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Medium Scale Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        // This should complete within timeout
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "-A"])
            .timeout(Duration::from_secs(25))
            .assert()
            .success()
            .stdout(predicate::str::contains("files added to staging"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["staged", "--limit", "10"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Page 1 of 5"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Medium scale commit"])
            .timeout(Duration::from_secs(25))
            .assert()
            .success();

        // Test that analysis commands work quickly
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("stats")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("50").or(predicate::str::contains("files")));
    });
}

#[test]
fn test_user_recovery_scenarios_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        create_minimal_project(project_path).unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Recovery Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "test.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Recovery test"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Test that user can access committed data
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "test.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Hello World"));

        // Test staging after commit
        fs::write(project_path.join("new.txt"), "New file").unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "new.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["staged", "clear", "--force"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success();

        // Original file should still be accessible
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "test.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Hello World"));
    });
}

#[test]
fn test_comprehensive_workflow_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Create realistic but small project
        fs::create_dir_all(project_path.join("src")).unwrap();
        fs::write(
            project_path.join("src/main.rs"),
            "fn main() { println!(\"Hello\"); }",
        )
        .unwrap();
        fs::write(project_path.join("README.md"), "# Test Project").unwrap();
        fs::write(
            project_path.join("Cargo.toml"),
            "[package]\nname = \"test\"",
        )
        .unwrap();

        // Create .digignore
        fs::write(project_path.join(".digignore"), "target/\n*.tmp\n").unwrap();

        // Full workflow test
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Comprehensive Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "-A"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Comprehensive test"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Test all major commands work
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("status")
            .timeout(Duration::from_secs(5))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("log")
            .timeout(Duration::from_secs(5))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "src/main.rs"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Hello"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("size")
            .timeout(Duration::from_secs(5))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["prove", "src/main.rs"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();
    });
}

#[test]
fn test_edge_cases_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        // Create edge case files
        fs::write(project_path.join("empty.txt"), "").unwrap();
        fs::write(project_path.join("single.txt"), "x").unwrap();
        fs::write(project_path.join("spaces file.txt"), "File with spaces").unwrap();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Edge Cases"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "-A"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Edge cases"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Test retrieving edge case files
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "empty.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "single.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("x"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "spaces file.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("spaces"));
    });
}

#[test]
fn test_persistence_fast() {
    with_timeout(|| {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        create_minimal_project(project_path).unwrap();

        // Test that operations persist across command invocations
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["init", "--name", "Persistence Test"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["--yes", "add", "test.txt"])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();

        // Check staging persists
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("staged")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("test.txt"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["commit", "-m", "Persistence test"])
            .timeout(Duration::from_secs(15))
            .assert()
            .success();

        // Check commit persists
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg("log")
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Persistence test"));

        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", "test.txt"])
            .timeout(Duration::from_secs(5))
            .assert()
            .success()
            .stdout(predicate::str::contains("Hello World"));
    });
}
