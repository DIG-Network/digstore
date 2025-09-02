//! URN-based access control system

use crate::core::types::{Hash, StoreId};
use crate::security::error::{SecurityError, SecurityResult};
use crate::storage::Store;
use crate::urn::{ByteRange, Urn};
use std::path::Path;

/// Access controller for URN validation
pub struct AccessController<'a> {
    store: &'a Store,
}

/// Access permission result
#[derive(Debug, Clone, PartialEq)]
pub enum AccessPermission {
    /// Access granted with validated URN
    Granted,
    /// Access denied with reason
    Denied(String),
}

impl<'a> AccessController<'a> {
    /// Create new access controller
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Validate URN for data access
    pub fn validate_access(&self, urn: &Urn) -> SecurityResult<AccessPermission> {
        // Verify store ID matches
        if urn.store_id != self.store.store_id() {
            return Ok(AccessPermission::Denied(format!(
                "Store ID mismatch: expected {}, got {}",
                self.store.store_id().to_hex(),
                urn.store_id.to_hex()
            )));
        }

        // Verify root hash exists (if specified)
        if let Some(root_hash) = urn.root_hash {
            if !self.store.has_commit(root_hash) {
                return Ok(AccessPermission::Denied(format!(
                    "Root hash not found: {}",
                    root_hash.to_hex()
                )));
            }
        }

        // Verify resource path exists (if specified)
        if let Some(path) = &urn.resource_path {
            if !self.store.has_file_at_path(path, urn.root_hash) {
                return Ok(AccessPermission::Denied(format!(
                    "Resource path not found: {}",
                    path.display()
                )));
            }
        }

        Ok(AccessPermission::Granted)
    }

    /// Check if URN has required components for specific operation
    pub fn validate_urn_completeness(&self, urn: &Urn, operation: &str) -> SecurityResult<()> {
        match operation {
            "file_access" => {
                if urn.resource_path.is_none() {
                    return Err(SecurityError::missing_urn_component("resource_path"));
                }
                if urn.root_hash.is_none() {
                    return Err(SecurityError::missing_urn_component("root_hash"));
                }
            },
            "byte_range_access" => {
                if urn.resource_path.is_none() {
                    return Err(SecurityError::missing_urn_component("resource_path"));
                }
                if urn.byte_range.is_none() {
                    return Err(SecurityError::missing_urn_component("byte_range"));
                }
                if urn.root_hash.is_none() {
                    return Err(SecurityError::missing_urn_component("root_hash"));
                }
            },
            "layer_access" => {
                if urn.root_hash.is_none() {
                    return Err(SecurityError::missing_urn_component("root_hash"));
                }
            },
            _ => {
                // Unknown operation - require at least store_id
                // store_id is always present in URN, so no additional validation needed
            },
        }

        Ok(())
    }

    /// Create URN for newly created content
    pub fn create_access_urn(
        &self,
        root_hash: Hash,
        resource_path: Option<&Path>,
        byte_range: Option<&ByteRange>,
    ) -> Urn {
        Urn {
            store_id: self.store.store_id(),
            root_hash: Some(root_hash),
            resource_path: resource_path.map(|p| p.to_path_buf()),
            byte_range: byte_range.cloned(),
        }
    }
}

/// Extension trait for Store to support access control
pub trait StoreAccessControl {
    /// Check if commit exists
    fn has_commit(&self, root_hash: Hash) -> bool;

    /// Check if file exists at path in specific commit
    fn has_file_at_path(&self, path: &Path, root_hash: Option<Hash>) -> bool;

    /// Get file with URN-based access control
    fn get_file_secure(&self, urn: &Urn) -> SecurityResult<Vec<u8>>;

    /// Get byte range with URN-based access control
    fn get_byte_range_secure(&self, urn: &Urn) -> SecurityResult<Vec<u8>>;
}

impl StoreAccessControl for Store {
    /// Check if commit exists (uses archive format)
    fn has_commit(&self, root_hash: Hash) -> bool {
        self.archive.has_layer(&root_hash)
    }

    /// Check if file exists at path in specific commit
    fn has_file_at_path(&self, path: &Path, root_hash: Option<Hash>) -> bool {
        if let Some(hash) = root_hash {
            // For committed files, we would need to load the layer and check
            // This is a simplified implementation
            if let Ok(layer) = self.load_layer(hash) {
                return layer.files.iter().any(|f| f.path == path);
            }
        }

        // Check in staging (unscrambled)
        self.staging.is_staged(path)
    }

    /// Get file with URN-based access control and unscrambling
    fn get_file_secure(&self, urn: &Urn) -> SecurityResult<Vec<u8>> {
        // Validate access
        let access_controller = AccessController::new(self);
        match access_controller.validate_access(urn)? {
            AccessPermission::Granted => {},
            AccessPermission::Denied(reason) => {
                return Err(SecurityError::access_denied(reason));
            },
        }

        // Validate URN completeness for file access
        access_controller.validate_urn_completeness(urn, "file_access")?;

        // Get the file data (this will be updated to use secure layer operations)
        let file_data = self
            .get_file_at(urn.resource_path.as_ref().unwrap(), urn.root_hash)
            .map_err(|e| SecurityError::access_denied(format!("File access failed: {}", e)))?;

        Ok(file_data)
    }

    /// Get byte range with URN-based access control and unscrambling
    fn get_byte_range_secure(&self, urn: &Urn) -> SecurityResult<Vec<u8>> {
        // Validate access
        let access_controller = AccessController::new(self);
        match access_controller.validate_access(urn)? {
            AccessPermission::Granted => {},
            AccessPermission::Denied(reason) => {
                return Err(SecurityError::access_denied(reason));
            },
        }

        // Validate URN completeness for byte range access
        access_controller.validate_urn_completeness(urn, "byte_range_access")?;

        // Get full file first, then extract range
        let file_data = self.get_file_secure(urn)?;

        // Extract byte range
        if let Some(byte_range) = &urn.byte_range {
            let file_len = file_data.len() as u64;
            let start = byte_range.start.unwrap_or(0);
            let end = byte_range
                .end
                .map(|e| (e + 1).min(file_len))
                .unwrap_or(file_len);

            if start >= file_len {
                return Ok(Vec::new());
            }

            Ok(file_data[start as usize..end as usize].to_vec())
        } else {
            Ok(file_data)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_store() -> (Store, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let store = Store::init(temp_dir.path()).unwrap();
        (store, temp_dir)
    }

    #[test]
    fn test_access_controller_validation() {
        let (store, _temp_dir) = create_test_store();
        let controller = AccessController::new(&store);

        // Valid URN with matching store ID
        let valid_urn = Urn {
            store_id: store.store_id(),
            root_hash: None,
            resource_path: None,
            byte_range: None,
        };

        let result = controller.validate_access(&valid_urn).unwrap();
        assert_eq!(result, AccessPermission::Granted);

        // Invalid URN with wrong store ID
        let invalid_store_id =
            Hash::from_hex("1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap();
        let invalid_urn = Urn {
            store_id: invalid_store_id,
            root_hash: None,
            resource_path: None,
            byte_range: None,
        };

        let result = controller.validate_access(&invalid_urn).unwrap();
        assert!(matches!(result, AccessPermission::Denied(_)));
    }

    #[test]
    fn test_urn_completeness_validation() {
        let (store, _temp_dir) = create_test_store();
        let controller = AccessController::new(&store);

        // URN missing resource_path for file access
        let incomplete_urn = Urn {
            store_id: store.store_id(),
            root_hash: Some(Hash::zero()),
            resource_path: None,
            byte_range: None,
        };

        let result = controller.validate_urn_completeness(&incomplete_urn, "file_access");
        assert!(result.is_err());

        // Complete URN for file access
        let complete_urn = Urn {
            store_id: store.store_id(),
            root_hash: Some(Hash::zero()),
            resource_path: Some(PathBuf::from("test.txt")),
            byte_range: None,
        };

        let result = controller.validate_urn_completeness(&complete_urn, "file_access");
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_access_urn() {
        let (store, _temp_dir) = create_test_store();
        let controller = AccessController::new(&store);

        let root_hash = Hash::zero();
        let path = PathBuf::from("test/file.txt");

        let urn = controller.create_access_urn(root_hash, Some(&path), None);

        assert_eq!(urn.store_id, store.store_id());
        assert_eq!(urn.root_hash, Some(root_hash));
        assert_eq!(urn.resource_path, Some(path));
        assert_eq!(urn.byte_range, None);
    }
}
