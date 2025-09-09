//! CLI tests for complete user workflows
//!
//! Tests realistic user scenarios from start to finish.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_new_user_onboarding() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create a simple project
    fs::write(project_path.join("hello.txt"), "Hello, Digstore!").unwrap();

    // Step 1: User gets help
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Digstore Min"))
        .stdout(predicate::str::contains("Content-addressable storage"))
        .stdout(predicate::str::contains("init"));

    // Step 2: User tries status before init (should get helpful error)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"))
        .stderr(predicate::str::contains("init"));

    // Step 3: User initializes repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository initialized"))
        .stdout(predicate::str::contains("Store ID:"))
        .stdout(predicate::str::contains("✓"));

    // Step 4: User checks status (should be clean)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"))
        .stdout(predicate::str::contains("No changes staged"));

    // Step 5: User adds file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "hello.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"))
        .stdout(predicate::str::contains("✓"));

    // Step 6: User checks what's staged
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("hello.txt"))
        .stdout(predicate::str::contains("commit"));

    // Step 7: User commits
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "My first commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"))
        .stdout(predicate::str::contains("✓"));

    // Step 8: User verifies commit
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("My first commit"));

    // Step 9: User retrieves file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "hello.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, Digstore!"));
}

#[test]
fn test_realistic_development_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create realistic project structure
    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::write(
        project_path.join("src/main.rs"),
        "fn main() {\n    println!(\"Hello, world!\");\n}",
    ).unwrap();
    fs::write(
        project_path.join("Cargo.toml"),
        "[package]\nname = \"test-app\"\nversion = \"0.1.0\"",
    ).unwrap();
    fs::write(
        project_path.join("README.md"),
        "# Test App\n\nA test application.",
    ).unwrap();

    // Developer starts new project
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Test Application"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository initialized"));

    // Developer checks initial status
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes staged"));

    // Developer adds files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"));

    // Developer validates what was staged
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("src/main.rs"))
        .stdout(predicate::str::contains("README.md"));

    // Developer commits
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Initial application setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));

    // Developer validates commit was successful
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initial application setup"));

    // Developer validates files are accessible
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, world!"));
}

#[test]
fn test_error_recovery_guidance() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test helpful error messages and recovery guidance

    // Error 1: User in wrong directory
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"))
        .stderr(predicate::str::contains("init"));

    // Error 2: User tries to add non-existent file
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

    // Error 3: User tries to commit without staging
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No files staged"))
        .stderr(predicate::str::contains("add"));
}

#[test]
fn test_progress_and_feedback() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create enough files to potentially trigger progress display
    for i in 0..20 {
        fs::write(
            project_path.join(format!("progress{:02}.txt", i)),
            format!("Progress file {}", i),
        ).unwrap();
    }

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Progress Test"])
        .assert()
        .success();

    // User should see informative feedback for operations
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"));

    // Commit should show progress for operations
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Progress commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));
}

#[test]
fn test_information_density() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create diverse content
    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::write(project_path.join("src/main.rs"), "fn main() {}").unwrap();
    fs::write(project_path.join("README.md"), "# Project").unwrap();
    fs::write(project_path.join("data.json"), "{\"key\": \"value\"}").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Info Test"])
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
        .args(&["commit", "-m", "Info commit"])
        .assert()
        .success();

    // Status should provide comprehensive information
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"))
        .stdout(predicate::str::contains("Store ID:"))
        .stdout(predicate::str::contains("Current commit:"));
}
