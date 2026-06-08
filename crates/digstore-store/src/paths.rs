use digstore_core::Bytes32;
use std::path::{Path, PathBuf};

/// Pure builder for the §4.4 on-disk layout. Performs no filesystem I/O.
///
/// ```text
/// {data_dir}/
///   {store_id_hex}.staging.bin
///   config.toml
///   roots.log                         // append-only root history
///   generations/{roothash_hex}/manifest.json
///   generations/{roothash_hex}/chunks/{chunk_hash_hex}   // sparse after dedup
///   modules/{store_id_hex}-{roothash_hex}.wasm
/// ```
#[derive(Debug, Clone)]
pub struct StorePaths {
    root: PathBuf,
    store_id_hex: String,
}

impl StorePaths {
    pub fn new(data_dir: impl AsRef<Path>, store_id: Bytes32) -> Self {
        Self {
            root: data_dir.as_ref().to_path_buf(),
            store_id_hex: store_id.to_hex(),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn history_file(&self) -> PathBuf {
        self.root.join("roots.log")
    }

    pub fn staging_file(&self) -> PathBuf {
        self.root.join(format!("{}.staging.bin", self.store_id_hex))
    }

    pub fn generations_dir(&self) -> PathBuf {
        self.root.join("generations")
    }

    pub fn modules_dir(&self) -> PathBuf {
        self.root.join("modules")
    }

    pub fn generation_dir(&self, root_hex: &str) -> PathBuf {
        self.generations_dir().join(root_hex)
    }

    pub fn generation_manifest(&self, root_hex: &str) -> PathBuf {
        self.generation_dir(root_hex).join("manifest.json")
    }

    pub fn generation_chunks_dir(&self, root_hex: &str) -> PathBuf {
        self.generation_dir(root_hex).join("chunks")
    }

    pub fn chunk_file(&self, root_hex: &str, chunk_hash_hex: &str) -> PathBuf {
        self.generation_chunks_dir(root_hex).join(chunk_hash_hex)
    }

    pub fn module_file(&self, root_hex: &str) -> PathBuf {
        self.modules_dir()
            .join(format!("{}-{}.wasm", self.store_id_hex, root_hex))
    }

    pub fn store_id_hex(&self) -> &str {
        &self.store_id_hex
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;
    use std::path::PathBuf;

    fn sid() -> Bytes32 {
        Bytes32([0x11u8; 32])
    }

    #[test]
    fn root_and_top_level_files() {
        let p = StorePaths::new("/data", sid());
        assert_eq!(p.root(), PathBuf::from("/data"));
        assert_eq!(p.config_file(), PathBuf::from("/data/config.toml"));
        let hex = "11".repeat(32);
        assert_eq!(
            p.staging_file(),
            PathBuf::from(format!("/data/{hex}.staging.bin"))
        );
    }

    #[test]
    fn generation_subtree() {
        let p = StorePaths::new("/data", sid());
        let root_hex = "ab".repeat(32);
        let gen = p.generation_dir(&root_hex);
        assert_eq!(gen, PathBuf::from(format!("/data/generations/{root_hex}")));
        assert_eq!(
            p.generation_manifest(&root_hex),
            PathBuf::from(format!("/data/generations/{root_hex}/manifest.json"))
        );
        assert_eq!(
            p.generation_chunks_dir(&root_hex),
            PathBuf::from(format!("/data/generations/{root_hex}/chunks"))
        );
        assert_eq!(
            p.chunk_file(&root_hex, "cc"),
            PathBuf::from(format!("/data/generations/{root_hex}/chunks/cc"))
        );
    }

    #[test]
    fn module_and_history_paths() {
        let p = StorePaths::new("/data", sid());
        let sid_hex = "11".repeat(32);
        let root_hex = "ab".repeat(32);
        assert_eq!(
            p.module_file(&root_hex),
            PathBuf::from(format!("/data/modules/{sid_hex}-{root_hex}.wasm"))
        );
        assert_eq!(p.generations_dir(), PathBuf::from("/data/generations"));
        assert_eq!(p.modules_dir(), PathBuf::from("/data/modules"));
        assert_eq!(p.history_file(), PathBuf::from("/data/roots.log"));
    }
}
