//! User Edge Case Tests
//!
//! These tests cover edge cases and unusual scenarios that users might encounter,
//! ensuring the application handles them gracefully.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_empty_files_and_directories() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create empty file and directory
    fs::write(project_path.join("empty.txt"), "").unwrap();
    fs::create_dir_all(project_path.join("empty_dir")).unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Empty Test"])
        .assert()
        .success();

    // User adds empty file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "empty.txt"])
        .assert()
        .success();

    // User tries to add empty directory (should skip or handle gracefully)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "empty_dir"])
        .assert()
        .success(); // Should succeed but skip directory

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Empty files test"])
        .assert()
        .success();

    // User can retrieve empty file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "empty.txt"])
        .assert()
        .success();
}

#[test]
fn test_special_characters_in_filenames() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create files with special characters
    let special_files = [
        "file with spaces.txt",
        "file-with-dashes.txt",
        "file_with_underscores.txt",
        "file.with.dots.txt",
        "file(with)parentheses.txt",
        "file[with]brackets.txt",
        "file{with}braces.txt",
    ];

    for filename in &special_files {
        fs::write(
            project_path.join(filename),
            format!("Content of {}", filename),
        )
        .unwrap();
    }

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Special Chars Test"])
        .assert()
        .success();

    // User adds files with special characters
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    // User checks staging
    let staged_output = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .get_output();

    let staged_stdout = String::from_utf8_lossy(&staged_output.stdout);
    for filename in &special_files {
        assert!(
            staged_stdout.contains(filename),
            "Should handle file with special characters: {}",
            filename
        );
    }

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Special characters commit"])
        .assert()
        .success();

    // User can retrieve files with special characters
    for filename in &special_files {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", filename])
            .assert()
            .success()
            .stdout(predicate::str::contains(format!("Content of {}", filename)));
    }
}

#[test]
fn test_very_large_repository() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create a repository with many files (500+)
    for i in 0..500 {
        let dir = match i % 5 {
            0 => "src",
            1 => "tests",
            2 => "docs",
            3 => "examples",
            _ => "misc",
        };
        fs::create_dir_all(project_path.join(dir)).unwrap();
        fs::write(
            project_path.join(dir).join(format!("file{:03}.txt", i)),
            format!("Content {}", i),
        )
        .unwrap();
    }

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Large Repo"])
        .assert()
        .success();

    // User adds all files - should handle efficiently
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"));

    // User can paginate through large staging area
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--limit", "50"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Page 1 of 10"));

    // User can view specific pages
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--page", "5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Page 5"));

    // User can commit large repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Large repository commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"));

    // User can analyze large repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["stats", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("500").or(predicate::str::contains("files")));
}

#[test]
fn test_file_permission_scenarios() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create files with different content types
    fs::write(project_path.join("text.txt"), "Text content").unwrap();
    fs::write(
        project_path.join("binary.bin"),
        vec![0u8, 1u8, 255u8, 128u8],
    )
    .unwrap();
    fs::write(project_path.join("unicode.txt"), "Unicode: 🚀 ✨ 🎉").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Permission Test"])
        .assert()
        .success();

    // User adds different file types
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Mixed file types"])
        .assert()
        .success();

    // User can retrieve all file types
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "text.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Text content"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "binary.bin", "-o", "retrieved_binary.bin"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content written to"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["cat", "unicode.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("🚀"));
}

#[test]
fn test_concurrent_usage_simulation() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test files
    for i in 0..10 {
        fs::write(
            project_path.join(format!("concurrent{}.txt", i)),
            format!("Concurrent test {}", i),
        )
        .unwrap();
    }

    // Initialize repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Concurrent Test"])
        .assert()
        .success();

    // Simulate multiple operations that might happen concurrently
    // (though we run them sequentially for testing)

    // User 1: Add some files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "concurrent0.txt", "concurrent1.txt"])
        .assert()
        .success();

    // User 1: Check status
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("2").or(predicate::str::contains("concurrent")));

    // User 2: Add more files (to same staging)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "concurrent2.txt", "concurrent3.txt"])
        .assert()
        .success();

    // Check that all files are staged
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("concurrent0.txt"))
        .stdout(predicate::str::contains("concurrent3.txt"));

    // Commit should handle all staged files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Concurrent operations"])
        .assert()
        .success();

    // All files should be accessible
    for i in 0..4 {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", &format!("concurrent{}.txt", i)])
            .assert()
            .success();
    }
}

#[test]
fn test_repository_corruption_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("recovery.txt"), "Recovery test").unwrap();

    // User creates repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Recovery Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "recovery.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Before corruption"])
        .assert()
        .success();

    // Simulate corruption by removing .digstore file
    fs::remove_file(project_path.join(".digstore")).unwrap();

    // User should get helpful error
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"))
        .stderr(predicate::str::contains("init"));

    // User can reinitialize if needed
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Recovered"])
        .assert()
        .success();
}

#[test]
fn test_network_and_sharing_scenarios() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create shareable content
    fs::write(
        project_path.join("shared_doc.md"),
        "# Shared Document\n\nThis is shared content.",
    )
    .unwrap();
    fs::write(
        project_path.join("data.json"),
        r#"{"shared": true, "version": "1.0"}"#,
    )
    .unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Shared Project"])
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
        .args(&["commit", "-m", "Shared content"])
        .assert()
        .success();

    // User generates proofs for shared content
    let proof_file = project_path.join("shared_proof.json");
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["prove", "shared_doc.md", "-o", proof_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof generated"));

    // Verify proof can be validated
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["verify", proof_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof verification successful"));

    // User can export repository information
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["store-info", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("store_id"));
}

#[test]
fn test_performance_stress_scenarios() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create files of varying sizes
    fs::write(project_path.join("tiny.txt"), "x").unwrap();
    fs::write(project_path.join("small.txt"), "small content here").unwrap();
    fs::write(
        project_path.join("medium.txt"),
        "medium content ".repeat(100),
    )
    .unwrap();
    fs::write(
        project_path.join("large.txt"),
        "large content line\n".repeat(5000),
    )
    .unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Performance Stress"])
        .assert()
        .success();

    // User adds mixed file sizes
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added"));

    // User checks performance metrics
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("Total size"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Mixed sizes"])
        .assert()
        .success();

    // User analyzes efficiency
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["size", "--efficiency"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Efficiency Metrics"));

    // User can retrieve all file sizes efficiently
    for filename in &["tiny.txt", "small.txt", "medium.txt", "large.txt"] {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", filename])
            .assert()
            .success();
    }
}

#[test]
fn test_invalid_input_handling() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Invalid Input Test"])
        .assert()
        .success();

    // Test invalid page numbers
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--page", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid page"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--page", "999"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid page"));

    // Test invalid limit values
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--limit", "0"])
        .assert()
        .failure();

    // Test invalid command combinations
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "--dry-run", "--from-stdin"])
        .assert()
        .success(); // Should handle gracefully

    // Test malformed URNs
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "invalid-urn-format"])
        .assert()
        .success(); // Should return deterministic random data

    // Test invalid hashes
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["root", "--at", "invalid-hash"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid"));
}

#[test]
fn test_cross_platform_compatibility() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create files with different line endings and path separators
    fs::write(project_path.join("unix.txt"), "Line 1\nLine 2\nLine 3").unwrap();
    fs::write(
        project_path.join("windows.txt"),
        "Line 1\r\nLine 2\r\nLine 3",
    )
    .unwrap();
    fs::write(project_path.join("mixed.txt"), "Line 1\nLine 2\r\nLine 3").unwrap();

    // Create nested directories
    fs::create_dir_all(project_path.join("deep/nested/structure")).unwrap();
    fs::write(
        project_path.join("deep/nested/structure/file.txt"),
        "Deeply nested file",
    )
    .unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Cross Platform Test"])
        .assert()
        .success();

    // User adds files with different characteristics
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    // User checks what was staged
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("unix.txt"))
        .stdout(predicate::str::contains("deep/nested/structure/file.txt"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Cross-platform commit"])
        .assert()
        .success();

    // User can retrieve files with different path formats
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "deep/nested/structure/file.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deeply nested"));

    // User can access with different line ending files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["cat", "windows.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Line 1"));
}

#[test]
fn test_repository_migration_scenarios() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create initial repository
    fs::write(project_path.join("migrate.txt"), "Migration test").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Migration Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "migrate.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Before migration"])
        .assert()
        .success();

    // User can access repository normally
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "migrate.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Migration test"));

    // Repository should be portable (can be moved)
    let new_temp_dir = TempDir::new().unwrap();
    let new_project_path = new_temp_dir.path();

    // Copy .digstore file to new location
    fs::copy(
        project_path.join(".digstore"),
        new_project_path.join(".digstore"),
    )
    .unwrap();
    fs::write(new_project_path.join("migrate.txt"), "Migration test").unwrap();

    // User should be able to access from new location
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(new_project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"));
}

#[test]
fn test_user_guidance_and_suggestions() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test guidance for uninitialized repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"))
        .stderr(predicate::str::contains("init"));

    // Initialize for further tests
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Guidance Test"])
        .assert()
        .success();

    // Test guidance for empty staging
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No files staged"))
        .stderr(predicate::str::contains("add"));

    // Test guidance for empty staging area
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("No files staged"))
        .stdout(predicate::str::contains("add"));

    // Test guidance after successful operations
    fs::write(project_path.join("guide.txt"), "Guidance test").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "guide.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("commit"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Guidance commit"])
        .assert()
        .success();

    // Status should show clean state with guidance
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes staged"));
}

#[test]
fn test_output_redirection_and_piping() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("pipe_test.txt"), "Pipe test content").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Pipe Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "pipe_test.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Pipe test"])
        .assert()
        .success();

    // Test output to file
    let output_file = project_path.join("output.txt");
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "pipe_test.txt", "-o", output_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content written to"));

    // Verify file content
    let content = fs::read_to_string(&output_file).unwrap();
    assert_eq!(content, "Pipe test content");

    // Test JSON output
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("store_id"));

    // Test that get command streams to stdout by default
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "pipe_test.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Pipe test content"));
}

#[test]
fn test_real_world_project_simulation() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Simulate a real software project
    fs::create_dir_all(project_path.join("src/components")).unwrap();
    fs::create_dir_all(project_path.join("src/utils")).unwrap();
    fs::create_dir_all(project_path.join("tests/unit")).unwrap();
    fs::create_dir_all(project_path.join("tests/integration")).unwrap();
    fs::create_dir_all(project_path.join("docs")).unwrap();
    fs::create_dir_all(project_path.join("assets")).unwrap();

    // Source files
    fs::write(
        project_path.join("src/main.rs"),
        "fn main() {\n    println!(\"Hello\");\n}",
    )
    .unwrap();
    fs::write(
        project_path.join("src/lib.rs"),
        "pub mod components;\npub mod utils;",
    )
    .unwrap();
    fs::write(
        project_path.join("src/components/button.rs"),
        "// Button component",
    )
    .unwrap();
    fs::write(
        project_path.join("src/utils/helpers.rs"),
        "// Helper functions",
    )
    .unwrap();

    // Test files
    fs::write(
        project_path.join("tests/unit/test_main.rs"),
        "// Unit tests",
    )
    .unwrap();
    fs::write(
        project_path.join("tests/integration/test_api.rs"),
        "// Integration tests",
    )
    .unwrap();

    // Documentation
    fs::write(
        project_path.join("README.md"),
        "# Real World Project\n\nA test project.",
    )
    .unwrap();
    fs::write(project_path.join("docs/api.md"), "# API Documentation").unwrap();
    fs::write(project_path.join("docs/guide.md"), "# User Guide").unwrap();

    // Config files
    fs::write(
        project_path.join("Cargo.toml"),
        "[package]\nname = \"real-project\"\nversion = \"0.1.0\"",
    )
    .unwrap();
    fs::write(project_path.join(".gitignore"), "target/\n*.tmp\n.DS_Store").unwrap();

    // Create .digignore (similar to .gitignore)
    fs::write(
        project_path.join(".digignore"),
        "target/\n*.tmp\n.DS_Store\n*.log",
    )
    .unwrap();

    // Assets
    fs::write(
        project_path.join("assets/config.json"),
        "{\"app\": \"config\"}",
    )
    .unwrap();
    fs::write(
        project_path.join("assets/styles.css"),
        "body { margin: 0; }",
    )
    .unwrap();

    // Temporary files that should be ignored
    fs::write(project_path.join("temp.tmp"), "Temporary").unwrap();
    fs::write(project_path.join("debug.log"), "Debug output").unwrap();

    // User workflow
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Real World Project"])
        .assert()
        .success();

    // User adds all project files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    // User checks what was staged (should respect .digignore)
    let staged_output = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .get_output();

    let staged_stdout = String::from_utf8_lossy(&staged_output.stdout);

    // Should include project files
    assert!(staged_stdout.contains("src/main.rs"));
    assert!(staged_stdout.contains("README.md"));
    assert!(staged_stdout.contains("Cargo.toml"));

    // Should exclude ignored files
    assert!(!staged_stdout.contains("temp.tmp"));
    assert!(!staged_stdout.contains("debug.log"));

    // User commits project
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Initial project commit"])
        .assert()
        .success();

    // User analyzes project
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["stats", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Statistics"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["size", "--breakdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Analytics"));

    // User can access any project file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "src/components/button.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Button component"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["cat", "docs/api.md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("API Documentation"));

    // User generates proof for important file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["prove", "src/main.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Proof generated"));
}

#[test]
fn test_user_recovery_scenarios() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create repository and commit some data
    fs::write(project_path.join("important.txt"), "Important data").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Recovery Test"])
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
        .args(&["commit", "-m", "Important commit"])
        .assert()
        .success();

    // User accidentally stages wrong file
    fs::write(project_path.join("wrong.txt"), "Wrong file").unwrap();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "wrong.txt"])
        .assert()
        .success();

    // User can clear staging to recover
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "clear", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleared"));

    // Staging should be empty
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("No files staged"));

    // Original committed data should still be accessible
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "important.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Important data"));
}

#[test]
fn test_comprehensive_help_system() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test main help
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Digstore Min"))
        .stdout(predicate::str::contains("Commands:"));

    // Test help for all major commands
    let commands = [
        "init", "add", "commit", "status", "get", "cat", "staged", "config", "store", "layer",
        "proof",
    ];

    for cmd in &commands {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&[cmd, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Usage:").or(predicate::str::contains("USAGE:")));
    }

    // Test help for subcommands
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("List staged files"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["store", "info", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("store information"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["proof", "generate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generate"));
}
