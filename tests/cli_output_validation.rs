//! CLI Output Quality Validation Tests
//!
//! These tests ensure that CLI output is user-friendly, consistent, and informative.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_success_indicators_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("success.txt"), "Success test").unwrap();

    // All successful operations should show ✓ indicators
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Success Test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "success.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Success commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["config", "user.name", "Test User"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));
}

#[test]
fn test_error_message_quality() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test error messages are helpful and actionable
    
    // 1. No repository error
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"))
        .stderr(predicate::str::contains("init"));

    // 2. Initialize for other tests
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Error Test"])
        .assert()
        .success();

    // 3. Empty commit error
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No files staged"))
        .stderr(predicate::str::contains("add"));

    // 4. File not found error
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["add", "nonexistent.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    // 5. Invalid page number error
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["staged", "--page", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid page"));
}

#[test]
fn test_progress_and_feedback_quality() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create enough files to trigger progress display
    for i in 0..50 {
        fs::write(
            project_path.join(format!("progress{:02}.txt", i)),
            format!("Progress file {}", i),
        ).unwrap();
    }

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Progress Test"])
        .assert()
        .success();

    // User should see informative progress for large operations
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"))
        .stdout(predicate::str::contains("files/s").or(predicate::str::contains("Processing")));

    // Commit should show progress for large operations
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Progress commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));
}

#[test]
fn test_information_density_and_usefulness() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create diverse content
    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::write(project_path.join("src/main.rs"), "fn main() {}").unwrap();
    fs::write(project_path.join("README.md"), "# Project").unwrap();
    fs::write(project_path.join("data.json"), "{\"key\": \"value\"}").unwrap();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Info Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Info commit"])
        .assert()
        .success();

    // Status should provide comprehensive information
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"))
        .stdout(predicate::str::contains("Store ID:"))
        .stdout(predicate::str::contains("Current commit:"));

    // Size command should provide useful metrics
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["size", "--breakdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"))
        .stdout(predicate::str::contains("Layer Files"))
        .stdout(predicate::str::contains("Total Storage"));

    // Stats should provide comprehensive analysis
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["stats", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Statistics"))
        .stdout(predicate::str::contains("Growth Metrics"));

    // Root should provide current state information
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["root", "--verbose"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Current Root Information"))
        .stdout(predicate::str::contains("Root Hash:"));
}

#[test]
fn test_command_output_formatting() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("format.txt"), "Format test").unwrap();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Format Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "format.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Format commit"])
        .assert()
        .success();

    // Test that commands use consistent formatting
    let analysis_commands = [
        ("status", "Repository Status"),
        ("log", "Commit History"),
        ("root", "Current Root Information"),
        ("size", "Storage Analytics"),
        ("stats", "Repository Statistics"),
        ("history", "Root History Analysis"),
        ("store-info", "Store Information"),
    ];

    for (command, expected_header) in &analysis_commands {
        Command::cargo_bin("digstore").unwrap()
            .current_dir(project_path)
            .arg(command)
            .assert()
            .success()
            .stdout(predicate::str::contains(expected_header))
            .stdout(predicate::str::contains("═")); // Consistent separator
    }

    // Test table formatting for detailed views
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["status", "--show-chunks"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status"))
        .stdout(predicate::str::contains("File"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["staged", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("File"))
        .stdout(predicate::str::contains("Size"))
        .stdout(predicate::str::contains("Hash"));
}

#[test]
fn test_json_output_completeness() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("json_test.txt"), "JSON test").unwrap();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "JSON Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "json_test.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "JSON commit"])
        .assert()
        .success();

    // Test JSON output for all commands that support it
    let json_commands = [
        vec!["status", "--json"],
        vec!["staged", "--json"],
        vec!["root", "--json"],
        vec!["size", "--json"],
        vec!["stats", "--json"],
        vec!["history", "--json"],
        vec!["store-info", "--json"],
        vec!["layers", "--list", "--json"],
        vec!["config", "--list", "--json"],
    ];

    for command in &json_commands {
        let output = Command::cargo_bin("digstore").unwrap()
            .current_dir(project_path)
            .args(command)
            .assert()
            .success()
            .get_output();

        let stdout = String::from_utf8_lossy(&output.stdout);
        
        // Should be valid JSON
        assert!(stdout.starts_with('{') || stdout.starts_with('['), 
                "Command {:?} should output valid JSON", command);
        assert!(stdout.ends_with('}') || stdout.ends_with(']'), 
                "Command {:?} should output complete JSON", command);
        
        // Should be parseable as JSON
        let _: serde_json::Value = serde_json::from_str(&stdout)
            .expect(&format!("Command {:?} should output valid JSON", command));
    }
}

#[test]
fn test_user_workflow_validation() {
    // Test the complete user workflow with validation at each step
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create realistic project
    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::write(project_path.join("src/main.rs"), "fn main() {\n    println!(\"Hello, world!\");\n}").unwrap();
    fs::write(project_path.join("Cargo.toml"), "[package]\nname = \"test-app\"\nversion = \"0.1.0\"").unwrap();
    fs::write(project_path.join("README.md"), "# Test App\n\nA test application.").unwrap();

    // Step 1: User initializes repository
    let init_output = Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Test Application"])
        .assert()
        .success()
        .get_output();

    let init_stdout = String::from_utf8_lossy(&init_output.stdout);
    assert!(init_stdout.contains("Repository initialized"));
    assert!(init_stdout.contains("Store ID:"));
    assert!(init_stdout.contains("✓"));

    // Step 2: User checks initial status
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"))
        .stdout(predicate::str::contains("No changes staged"));

    // Step 3: User adds files
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"))
        .stdout(predicate::str::contains("✓"));

    // Step 4: User validates what was staged
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("Staged Files"))
        .stdout(predicate::str::contains("src/main.rs"))
        .stdout(predicate::str::contains("README.md"));

    // Step 5: User commits with validation
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Initial application setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"))
        .stdout(predicate::str::contains("✓"));

    // Step 6: User validates commit was successful
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initial application setup"));

    // Step 7: User validates files are accessible
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, world!"));

    // Step 8: User validates repository state
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes staged"));
}

#[test]
fn test_informative_summaries() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create files with different characteristics
    fs::write(project_path.join("small.txt"), "small").unwrap();
    fs::write(project_path.join("medium.txt"), "medium content ".repeat(50)).unwrap();
    fs::write(project_path.join("large.txt"), "large content line\n".repeat(500)).unwrap();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Summary Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Summary commit"])
        .assert()
        .success();

    // Commands should provide informative summaries
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["size", "--breakdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"))
        .stdout(predicate::str::contains("Total Storage:"))
        .stdout(predicate::str::contains("Layer Files:"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["stats", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Statistics"))
        .stdout(predicate::str::contains("Total Commits:"))
        .stdout(predicate::str::contains("Active Files:"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["layers", "--list", "--size"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Layer List"))
        .stdout(predicate::str::contains("files"));
}

#[test]
fn test_user_guidance_quality() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test guidance for various scenarios

    // 1. No repository guidance
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("digstore init"));

    // 2. Initialize repository
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Guidance Test"])
        .assert()
        .success();

    // 3. Empty staging guidance
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("No files staged"))
        .stdout(predicate::str::contains("add"));

    // 4. Empty commit guidance
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("add"));

    // 5. Add file and check guidance
    fs::write(project_path.join("guide.txt"), "Guidance test").unwrap();
    
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "guide.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("commit"));

    // 6. After commit guidance
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Guidance commit"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes staged"));
}

#[test]
fn test_configuration_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test configuration workflow
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration Management"))
        .stdout(predicate::str::contains("Usage:"));

    // User views empty configuration
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["config", "--list"])
        .assert()
        .success();

    // User sets configuration values
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["config", "user.name", "Test User"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"))
        .stdout(predicate::str::contains("Test User"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["config", "user.email", "test@example.com"])
        .assert()
        .success();

    // User views updated configuration
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["config", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test User"))
        .stdout(predicate::str::contains("test@example.com"));

    // User gets specific configuration value
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["config", "user.name"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test User"));

    // User can see configuration file location
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["config", "--show-origin"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration file"));
}

#[test]
fn test_staging_area_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test files
    for i in 0..25 {
        fs::write(
            project_path.join(format!("stage{:02}.txt", i)),
            format!("Staging test {}", i),
        ).unwrap();
    }

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Staging Test"])
        .assert()
        .success();

    // User adds files progressively
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "stage00.txt", "stage01.txt", "stage02.txt"])
        .assert()
        .success();

    // User checks staging
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("3").or(predicate::str::contains("stage")));

    // User adds more files
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    // User checks staging with pagination
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["staged", "--limit", "10"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Page 1"));

    // User can view all staged files
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["staged", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("25").or(predicate::str::contains("staged files")));

    // User can see staging diff
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["staged", "diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stage Diff"));

    // User can clear staging if needed
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["staged", "clear", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleared"));

    // Staging should be empty
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("No files staged"));
}

#[test]
fn test_proof_system_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("verify.txt"), "Verification test content").unwrap();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Proof Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "verify.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Proof test commit"])
        .assert()
        .success();

    // User generates proof
    let proof_file = project_path.join("test_proof.json");
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["prove", "verify.txt", "-o", proof_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof generated"))
        .stdout(predicate::str::contains("✓"));

    // Verify proof file exists and is valid JSON
    assert!(proof_file.exists());
    let proof_content = fs::read_to_string(&proof_file).unwrap();
    let _: serde_json::Value = serde_json::from_str(&proof_content)
        .expect("Proof should be valid JSON");

    // User verifies proof
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["verify", proof_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof verification successful"))
        .stdout(predicate::str::contains("✓"));

    // User can generate different proof formats
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["prove", "verify.txt", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["prove", "verify.txt", "--format", "text"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof Type:"));
}

#[test]
fn test_repository_inspection_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create comprehensive test repository
    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::create_dir_all(project_path.join("assets")).unwrap();

    for i in 0..10 {
        fs::write(
            project_path.join("src").join(format!("module{}.rs", i)),
            format!("// Module {}\npub fn function{}() {{}}", i, i),
        ).unwrap();
    }

    fs::write(project_path.join("assets/large.bin"), vec![0u8; 50000]).unwrap();
    fs::write(project_path.join("README.md"), "# Inspection Test\n\nComprehensive test.").unwrap();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Inspection Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Comprehensive commit"])
        .assert()
        .success();

    // User inspects repository comprehensively
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["size", "--breakdown", "--efficiency"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"))
        .stdout(predicate::str::contains("Efficiency Metrics"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["stats", "--detailed", "--performance"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Statistics"))
        .stdout(predicate::str::contains("Performance Metrics"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["layers", "--list", "--files", "--chunks"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Layer List"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["store-info", "--config", "--paths"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Store Information"))
        .stdout(predicate::str::contains("Configuration:"))
        .stdout(predicate::str::contains("Paths:"));
}

#[test]
fn test_command_flag_combinations() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("flags.txt"), "Flag test").unwrap();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Flag Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "flags.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Flag commit"])
        .assert()
        .success();

    // Test different flag combinations
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["status", "--short"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["log", "--oneline"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["root", "--hash-only"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["root", "--verbose"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Layer Details"));

    // Test global flags
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--quiet", "status"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--verbose", "status"])
        .assert()
        .success();
}

#[test]
fn test_file_operations_edge_cases() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create edge case files
    fs::write(project_path.join("single_char.txt"), "x").unwrap();
    fs::write(project_path.join("empty.txt"), "").unwrap();
    fs::write(project_path.join("newlines.txt"), "\n\n\n").unwrap();
    fs::write(project_path.join("binary.bin"), vec![0, 255, 128, 64]).unwrap();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Edge Cases"])
        .assert()
        .success();

    // User adds edge case files
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Edge case files"])
        .assert()
        .success();

    // User can retrieve all edge case files
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "single_char.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("x"));

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "empty.txt"])
        .assert()
        .success(); // Should succeed with empty output

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "binary.bin", "-o", "retrieved.bin"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content written to"));

    // Verify binary file was retrieved correctly
    let original = fs::read(project_path.join("binary.bin")).unwrap();
    let retrieved = fs::read(project_path.join("retrieved.bin")).unwrap();
    assert_eq!(original, retrieved);
}

#[test]
fn test_user_workflow_interruption_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test files
    for i in 0..20 {
        fs::write(
            project_path.join(format!("interrupt{:02}.txt", i)),
            format!("Interruption test {}", i),
        ).unwrap();
    }

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Interruption Test"])
        .assert()
        .success();

    // User starts adding files
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "interrupt00.txt", "interrupt01.txt"])
        .assert()
        .success();

    // User checks staging (should persist)
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("interrupt00.txt"));

    // Simulate interruption and resumption - user adds more files
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "interrupt02.txt", "interrupt03.txt"])
        .assert()
        .success();

    // All files should be in staging
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("interrupt00.txt"))
        .stdout(predicate::str::contains("interrupt03.txt"));

    // User can successfully commit after interruption
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Recovered commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));

    // All files should be accessible
    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "interrupt00.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore").unwrap()
        .current_dir(project_path)
        .args(&["get", "interrupt03.txt"])
        .assert()
        .success();
}

