//! URN (Uniform Resource Name) system for Digstore Min
//!
//! This module provides parsing and resolution of URNs with the format:
//! `urn:dig:chia:{storeID}[:{rootHash}][/{resourcePath}][#{byteRange}]`

pub mod parser;

use crate::core::{types::*, error::*};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use parser::parse_urn;

/// Parsed URN structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Urn {
    /// Store identifier
    pub store_id: StoreId,
    /// Root hash (optional, defaults to latest)
    pub root_hash: Option<Hash>,
    /// Resource path within the store
    pub resource_path: Option<PathBuf>,
    /// Byte range specification
    pub byte_range: Option<ByteRange>,
}

/// Byte range specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByteRange {
    /// Start byte (inclusive, None means from beginning)
    pub start: Option<u64>,
    /// End byte (inclusive, None means to end)
    pub end: Option<u64>,
}

impl Urn {
    /// Parse a URN string
    pub fn parse(urn_str: &str) -> Result<Self> {
        parse_urn(urn_str)
    }

    /// Convert URN back to string representation
    pub fn to_string(&self) -> String {
        let mut result = format!("urn:dig:chia:{}", self.store_id.to_hex());
        
        if let Some(root_hash) = &self.root_hash {
            result.push_str(&format!(":{}", root_hash.to_hex()));
        }
        
        if let Some(path) = &self.resource_path {
            result.push('/');
            result.push_str(&path.to_string_lossy());
        }
        
        if let Some(byte_range) = &self.byte_range {
            result.push_str(&byte_range.to_string());
        }
        
        result
    }

    /// Create a URN with a byte range
    pub fn with_byte_range(mut self, range: ByteRange) -> Self {
        self.byte_range = Some(range);
        self
    }
}

impl ByteRange {
    /// Create a range from start to end (inclusive)
    pub fn new(start: Option<u64>, end: Option<u64>) -> Self {
        Self { start, end }
    }

    /// Create a range from start to end of file
    pub fn from_start(start: u64) -> Self {
        Self {
            start: Some(start),
            end: None,
        }
    }

    /// Create a range for the last N bytes
    pub fn last_bytes(count: u64) -> Self {
        Self {
            start: None,
            end: Some(count),
        }
    }

    /// Convert to string representation (e.g., "#bytes=0-1023")
    pub fn to_string(&self) -> String {
        match (self.start, self.end) {
            (Some(start), Some(end)) => format!("#bytes={}-{}", start, end),
            (Some(start), None) => format!("#bytes={}-", start),
            (None, Some(end)) => format!("#bytes=-{}", end),
            (None, None) => String::new(),
        }
    }
}
