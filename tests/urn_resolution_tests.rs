//! URN resolution tests

use digstore_min::{
    storage::store::Store,
    urn::{Urn, ByteRange},
    core::{types::*, error::DigstoreError}
};
use tempfile::TempDir;
use std::path::Path;
use anyhow::Result;

#[test]
fn test_urn_resolve_full_file() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    let test_content = b"This is test content for URN resolution.";
    std::fs::write(temp_dir.path().join("test.txt"), test_content)?;
    
    store.add_file(Path::new("test.txt"))?;
    let commit_id = store.commit("Add test file")?;

    // Create URN for the file
    let urn = Urn {
        store_id: store.store_id(),
        root_hash: Some(commit_id),
        resource_path: Some(Path::new("test.txt").to_path_buf()),
        byte_range: None,
    };

    // Resolve URN to content
    let resolved_content = urn.resolve(&store)?;
    assert_eq!(resolved_content, test_content);

    Ok(())
}

#[test]
fn test_urn_resolve_with_byte_range() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    let test_content = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    std::fs::write(temp_dir.path().join("range_test.txt"), test_content)?;
    
    store.add_file(Path::new("range_test.txt"))?;
    let commit_id = store.commit("Add file for byte range test")?;

    // Test different byte ranges
    let test_cases = vec![
        (ByteRange::new(Some(0), Some(9)), &b"0123456789"[..]),     // First 10 bytes
        (ByteRange::new(Some(10), Some(15)), &b"ABCDEF"[..]),       // Middle 6 bytes
        (ByteRange::new(Some(30), None), &b"UVWXYZ"[..]),           // From position to end
        (ByteRange::last_bytes(5), &b"VWXYZ"[..]),                  // Last 5 bytes
    ];

    for (byte_range, expected) in test_cases {
        let urn = Urn {
            store_id: store.store_id(),
            root_hash: Some(commit_id),
            resource_path: Some(Path::new("range_test.txt").to_path_buf()),
            byte_range: Some(byte_range),
        };

        let resolved_content = urn.resolve(&store)?;
        assert_eq!(resolved_content, expected, "Byte range resolution failed");
    }

    Ok(())
}

#[test]
fn test_urn_resolve_latest_version() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit initial version
    std::fs::write(temp_dir.path().join("versioned.txt"), b"Version 1")?;
    store.add_file(Path::new("versioned.txt"))?;
    store.commit("First version")?;

    // Create and commit second version
    std::fs::write(temp_dir.path().join("versioned.txt"), b"Version 2 - updated")?;
    store.add_file(Path::new("versioned.txt"))?;
    store.commit("Second version")?;

    // URN without root hash should resolve to latest version
    let urn = Urn {
        store_id: store.store_id(),
        root_hash: None, // Latest version
        resource_path: Some(Path::new("versioned.txt").to_path_buf()),
        byte_range: None,
    };

    let resolved_content = urn.resolve(&store)?;
    assert_eq!(resolved_content, b"Version 2 - updated");

    Ok(())
}

#[test]
fn test_urn_resolve_specific_version() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit initial version
    std::fs::write(temp_dir.path().join("versioned.txt"), b"Version 1")?;
    store.add_file(Path::new("versioned.txt"))?;
    let commit1 = store.commit("First version")?;

    // Create and commit second version
    std::fs::write(temp_dir.path().join("versioned.txt"), b"Version 2")?;
    store.add_file(Path::new("versioned.txt"))?;
    store.commit("Second version")?;

    // URN with specific root hash should resolve to that version
    let urn = Urn {
        store_id: store.store_id(),
        root_hash: Some(commit1),
        resource_path: Some(Path::new("versioned.txt").to_path_buf()),
        byte_range: None,
    };

    let resolved_content = urn.resolve(&store)?;
    assert_eq!(resolved_content, b"Version 1");

    Ok(())
}

#[test]
fn test_urn_resolve_nonexistent_file() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let store = Store::init(temp_dir.path())?;

    let urn = Urn {
        store_id: store.store_id(),
        root_hash: None,
        resource_path: Some(Path::new("nonexistent.txt").to_path_buf()),
        byte_range: None,
    };

    let result = urn.resolve(&store);
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_urn_resolve_invalid_byte_range() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a small test file
    let test_content = b"Small file";
    std::fs::write(temp_dir.path().join("small.txt"), test_content)?;
    
    store.add_file(Path::new("small.txt"))?;
    store.commit("Add small file")?;

    // Test invalid byte ranges
    let invalid_ranges = vec![
        ByteRange::new(Some(5), Some(3)),  // Start > end
        ByteRange::new(Some(100), Some(200)), // Start beyond file
    ];

    for byte_range in invalid_ranges {
        let urn = Urn {
            store_id: store.store_id(),
            root_hash: None,
            resource_path: Some(Path::new("small.txt").to_path_buf()),
            byte_range: Some(byte_range),
        };

        let result = urn.resolve(&store);
        assert!(result.is_err(), "Expected error for invalid byte range");
    }

    Ok(())
}

#[test]
fn test_urn_resolve_no_resource_path() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let store = Store::init(temp_dir.path())?;

    // URN without resource path should fail
    let urn = Urn {
        store_id: store.store_id(),
        root_hash: None,
        resource_path: None,
        byte_range: None,
    };

    let result = urn.resolve(&store);
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_urn_resolve_edge_case_byte_ranges() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create file with known content
    let test_content = b"ABCDEFGHIJ"; // 10 bytes
    std::fs::write(temp_dir.path().join("edge_test.txt"), test_content)?;
    
    store.add_file(Path::new("edge_test.txt"))?;
    store.commit("Add file for edge case tests")?;

    // Test edge cases
    let test_cases = vec![
        (ByteRange::new(Some(0), Some(0)), &b"A"[..]),              // Single byte (inclusive)
        (ByteRange::new(Some(9), Some(9)), &b"J"[..]),              // Last byte
        (ByteRange::new(Some(0), Some(9)), test_content),           // Entire file
        (ByteRange::new(Some(5), None), &b"FGHIJ"[..]),             // From middle to end
        (ByteRange::last_bytes(3), &b"HIJ"[..]),                    // Last 3 bytes
        (ByteRange::last_bytes(20), test_content),                  // More than file size
    ];

    for (byte_range, expected) in test_cases {
        let urn = Urn {
            store_id: store.store_id(),
            root_hash: None,
            resource_path: Some(Path::new("edge_test.txt").to_path_buf()),
            byte_range: Some(byte_range),
        };

        let resolved_content = urn.resolve(&store)?;
        assert_eq!(resolved_content, expected, "Edge case byte range failed");
    }

    Ok(())
}

#[test]
fn test_urn_parse_and_resolve() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    let test_content = b"Parse and resolve test content.";
    std::fs::write(temp_dir.path().join("parse_test.txt"), test_content)?;
    
    store.add_file(Path::new("parse_test.txt"))?;
    let commit_id = store.commit("Add file for parse test")?;

    // Create URN string
    let urn_string = format!(
        "urn:dig:chia:{}:{}/parse_test.txt#bytes=5-10",
        store.store_id().to_hex(),
        commit_id.to_hex()
    );

    // Parse URN and resolve
    let urn = Urn::parse(&urn_string)?;
    let resolved_content = urn.resolve(&store)?;
    
    // Should get bytes 5-10: " and r"
    assert_eq!(resolved_content, b" and r");

    Ok(())
}

#[test]
fn test_urn_resolve_large_file_byte_range() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create a larger file for realistic byte range testing
    let mut large_content = Vec::new();
    for i in 0..1000 {
        large_content.extend_from_slice(format!("Line {} content.\n", i).as_bytes());
    }
    
    std::fs::write(temp_dir.path().join("large.txt"), &large_content)?;
    store.add_file(Path::new("large.txt"))?;
    store.commit("Add large file")?;

    // Test byte range in the middle
    let urn = Urn {
        store_id: store.store_id(),
        root_hash: None,
        resource_path: Some(Path::new("large.txt").to_path_buf()),
        byte_range: Some(ByteRange::new(Some(100), Some(200))),
    };

    let resolved_content = urn.resolve(&store)?;
    assert_eq!(resolved_content.len(), 101); // 200-100+1 bytes (inclusive)
    assert_eq!(resolved_content, &large_content[100..201]);

    Ok(())
}
