//! URN parsing implementation

use crate::core::{types::*, error::*};
use crate::urn::{Urn, ByteRange};
use std::path::PathBuf;

/// Parse a URN string into a Urn struct
pub fn parse_urn(urn_str: &str) -> Result<Urn> {
    // TODO: Implement URN parsing using nom
    // For now, provide a simple implementation
    
    if !urn_str.starts_with("urn:dig:chia:") {
        return Err(DigstoreError::invalid_urn(format!(
            "URN must start with 'urn:dig:chia:', got: {}", urn_str
        )));
    }
    
    // Remove the prefix
    let remainder = &urn_str[13..]; // "urn:dig:chia:".len() == 13
    
    // Split by '#' to separate byte range
    let (main_part, byte_range_str) = if let Some(pos) = remainder.find('#') {
        (&remainder[..pos], Some(&remainder[pos..]))
    } else {
        (remainder, None)
    };
    
    // Parse byte range if present
    let byte_range = if let Some(range_str) = byte_range_str {
        Some(parse_byte_range(range_str)?)
    } else {
        None
    };
    
    // Split by '/' to separate path
    let (id_part, path_str) = if let Some(pos) = main_part.find('/') {
        (&main_part[..pos], Some(&main_part[pos + 1..]))
    } else {
        (main_part, None)
    };
    
    // Parse resource path if present
    let resource_path = path_str.map(|p| PathBuf::from(p));
    
    // Split store ID and root hash by ':'
    let (store_id_str, root_hash_str) = if let Some(pos) = id_part.find(':') {
        (&id_part[..pos], Some(&id_part[pos + 1..]))
    } else {
        (id_part, None)
    };
    
    // Parse store ID
    let store_id = Hash::from_hex(store_id_str)
        .map_err(|_| DigstoreError::invalid_urn(format!("Invalid store ID: {}", store_id_str)))?;
    
    // Parse root hash if present
    let root_hash = if let Some(hash_str) = root_hash_str {
        Some(Hash::from_hex(hash_str)
            .map_err(|_| DigstoreError::invalid_urn(format!("Invalid root hash: {}", hash_str)))?)
    } else {
        None
    };
    
    Ok(Urn {
        store_id,
        root_hash,
        resource_path,
        byte_range,
    })
}

/// Parse byte range specification
fn parse_byte_range(range_str: &str) -> Result<ByteRange> {
    if !range_str.starts_with("#bytes=") {
        return Err(DigstoreError::invalid_urn(format!(
            "Invalid byte range format: {}", range_str
        )));
    }
    
    let range_part = &range_str[7..]; // "#bytes=".len() == 7
    
    if range_part.starts_with('-') {
        // Last N bytes: #bytes=-1024
        let count_str = &range_part[1..];
        let count = count_str.parse::<u64>()
            .map_err(|_| DigstoreError::invalid_urn(format!("Invalid byte count: {}", count_str)))?;
        Ok(ByteRange::last_bytes(count))
    } else if range_part.ends_with('-') {
        // From start to end: #bytes=1024-
        let start_str = &range_part[..range_part.len() - 1];
        let start = start_str.parse::<u64>()
            .map_err(|_| DigstoreError::invalid_urn(format!("Invalid start byte: {}", start_str)))?;
        Ok(ByteRange::from_start(start))
    } else if let Some(pos) = range_part.find('-') {
        // Range: #bytes=0-1023
        let start_str = &range_part[..pos];
        let end_str = &range_part[pos + 1..];
        
        let start = start_str.parse::<u64>()
            .map_err(|_| DigstoreError::invalid_urn(format!("Invalid start byte: {}", start_str)))?;
        let end = end_str.parse::<u64>()
            .map_err(|_| DigstoreError::invalid_urn(format!("Invalid end byte: {}", end_str)))?;
        
        if start > end {
            return Err(DigstoreError::invalid_urn(format!(
                "Start byte ({}) cannot be greater than end byte ({})", start, end
            )));
        }
        
        Ok(ByteRange::new(Some(start), Some(end)))
    } else {
        Err(DigstoreError::invalid_urn(format!(
            "Invalid byte range format: {}", range_str
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_urn() {
        let urn_str = "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2";
        let urn = parse_urn(urn_str).unwrap();
        
        assert_eq!(urn.store_id.to_hex(), "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2");
        assert!(urn.root_hash.is_none());
        assert!(urn.resource_path.is_none());
        assert!(urn.byte_range.is_none());
    }

    #[test]
    fn test_parse_urn_with_path() {
        let urn_str = "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/src/main.rs";
        let urn = parse_urn(urn_str).unwrap();
        
        assert_eq!(urn.resource_path.unwrap(), PathBuf::from("src/main.rs"));
    }

    #[test]
    fn test_parse_urn_with_byte_range() {
        let urn_str = "urn:dig:chia:a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2/file.txt#bytes=0-1023";
        let urn = parse_urn(urn_str).unwrap();
        
        let byte_range = urn.byte_range.unwrap();
        assert_eq!(byte_range.start, Some(0));
        assert_eq!(byte_range.end, Some(1023));
    }

    #[test]
    fn test_parse_invalid_urn() {
        let result = parse_urn("invalid:urn:format");
        assert!(result.is_err());
    }
}
