use crate::chunkstore::ChunkStore;
use crate::clock::Clock;
use crate::config::{load_config, save_config};
use crate::error::{Result, StoreError};
use crate::generation::{ChunkRef, GenerationManifest, KeyTableRecord};
use crate::history::RootHistory;
use crate::paths::StorePaths;
use crate::staging::StagingArea;
use digstore_chunker::chunk_slice;
use digstore_core::{Bytes32, ChunkerConfig, GenerationState, MerkleTree, StoreConfig, Urn};
use std::path::Path;

/// The host-side Store entity (§4). Owns the on-disk layout, staging, and
/// generations. Generic over a `Clock` so commit timestamps are injectable.
pub struct Store<C: Clock> {
    config: StoreConfig,
    paths: StorePaths,
    clock: C,
}

impl<C: Clock> std::fmt::Debug for Store<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("config", &self.config)
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl<C: Clock> Store<C> {
    /// Create a new store: write config + the §4.4 directory tree. Refuses to
    /// overwrite an existing store (presence of `config.toml`).
    pub fn init(config: StoreConfig, clock: C) -> Result<Self> {
        let paths = StorePaths::new(&config.data_dir, config.store_id);
        if paths.config_file().exists() {
            return Err(StoreError::AlreadyExists(paths.root().display().to_string()));
        }
        std::fs::create_dir_all(paths.root())?;
        std::fs::create_dir_all(paths.generations_dir())?;
        std::fs::create_dir_all(paths.modules_dir())?;
        save_config(paths.config_file(), &config)?;
        StagingArea::open(paths.staging_file())?;
        RootHistory::open(paths.history_file())?;
        Ok(Self { config, paths, clock })
    }

    /// Open an existing store rooted at `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>, clock: C) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        let config_file = data_dir.join("config.toml");
        if !config_file.exists() {
            return Err(StoreError::NotFound(data_dir.display().to_string()));
        }
        let config = load_config(&config_file)?;
        let paths = StorePaths::new(data_dir, config.store_id);
        Ok(Self { config, paths, clock })
    }

    pub fn store_id(&self) -> Bytes32 {
        self.config.store_id
    }

    pub fn config(&self) -> &StoreConfig {
        &self.config
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    /// All generation states, oldest first (§4.3 root history).
    pub fn root_history(&self) -> Result<Vec<GenerationState>> {
        RootHistory::open(self.paths.history_file())?.entries()
    }

    /// True if a chunk with this hash is already stored under some generation
    /// directory (global dedup index, §8.2).
    fn chunk_exists_anywhere(&self, hash: Bytes32) -> Result<bool> {
        let gens = self.paths.generations_dir();
        if !gens.exists() {
            return Ok(false);
        }
        let name = hash.to_hex();
        for entry in std::fs::read_dir(&gens)? {
            let chunks = entry?.path().join("chunks");
            if chunks.join(&name).exists() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Stage raw bytes under an explicit resource key (§20.2).
    pub fn stage_file(&mut self, resource_key: &str, bytes: &[u8]) -> Result<()> {
        let mut staging = StagingArea::open(self.paths.staging_file())?;
        staging.append(resource_key, bytes)?;
        Ok(())
    }

    /// Stage a file from disk. The path relative to `base` becomes the resource
    /// key (forward-slash normalized); the file bytes are staged verbatim.
    pub fn add(&mut self, file: impl AsRef<Path>, base: impl AsRef<Path>) -> Result<()> {
        let file = file.as_ref();
        let base = base.as_ref();
        let rel = file
            .strip_prefix(base)
            .map_err(|_| StoreError::PathEscape(file.to_path_buf()))?;
        let resource_key = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = std::fs::read(file)?;
        self.stage_file(&resource_key, &bytes)
    }

    /// Finalize a generation (§20.3, §8.2): chunk staged content, build the
    /// per-generation merkle tree, append the root to history, write the
    /// generation directory. Returns the new root hash. Does NOT compile the
    /// module (that is `digstore-compiler`'s job over this generation dir).
    pub fn commit(&mut self) -> Result<Bytes32> {
        let mut staging = StagingArea::open(self.paths.staging_file())?;
        let records = staging.records()?;
        if records.is_empty() {
            return Err(StoreError::EmptyStaging);
        }

        // Catalog chunker defaults (min 16 KiB, target 64 KiB, max 256 KiB).
        // `mask` selects the average chunk size; (1<<16)-1 targets ~64 KiB.
        let chunker = ChunkerConfig {
            min_size: 16 * 1024,
            target_size: 64 * 1024,
            max_size: 256 * 1024,
            mask: (1u64 << 16) - 1,
        };

        // Build the chunk pool in staged-record order (the §8.3 source consumed
        // by the compiler) and the key table mapping each resource to its
        // ordered pool indices.
        let mut pool: Vec<(Bytes32, Vec<u8>)> = Vec::new();
        let mut key_table: Vec<KeyTableRecord> = Vec::new();

        for rec in &records {
            let chunks = chunk_slice(&rec.content, &chunker);
            let mut indices = Vec::with_capacity(chunks.len());
            let mut total: u64 = 0;
            for chunk in &chunks {
                // `chunk.hash` is SHA-256(chunk.data) (PROPERTIES 9.4 leaf rule).
                let hash = chunk.hash;
                let index = pool.len() as u32;
                pool.push((hash, chunk.data.clone()));
                indices.push(index);
                total += chunk.data.len() as u64;
            }
            // root_hash: None -> retrieval key is root-independent (documented).
            let urn = Urn {
                chain: "chia".to_string(),
                store_id: self.config.store_id,
                root_hash: None,
                resource_key: Some(rec.resource_key.clone()),
            };
            key_table.push(KeyTableRecord {
                resource_key: rec.resource_key.clone(),
                static_key: urn.retrieval_key(),
                generation: Bytes32([0u8; 32]), // placeholder set after root
                chunk_indices: indices,
                total_size: total,
            });
        }

        // Merkle tree over chunk leaves in pool order (§9.1, owned by core).
        let leaves: Vec<Bytes32> = pool.iter().map(|(h, _)| *h).collect();
        let tree = MerkleTree::from_leaves(leaves);
        let root = tree.root();
        let root_hex = root.to_hex();

        // Now that we know the root, stamp each key-table record's generation.
        for rec in &mut key_table {
            rec.generation = root;
        }

        // Write generation dir (per-directory dedup; global dedup added in
        // Task 13 over `chunk_exists_anywhere`).
        let chunks_dir = self.paths.generation_chunks_dir(&root_hex);
        std::fs::create_dir_all(&chunks_dir)?;
        let chunkstore = ChunkStore::new(&chunks_dir);
        let mut chunk_refs = Vec::with_capacity(pool.len());
        for (i, (hash, data)) in pool.iter().enumerate() {
            // §8.2: only store the chunk if it is not already present in this or
            // any prior generation. `chunk_refs` still records every chunk's
            // index so reassembly is complete regardless of where the bytes live
            // (resolved globally by `Store::resolve_chunk`, Task 14).
            if !self.chunk_exists_anywhere(*hash)? {
                chunkstore.put(*hash, data)?;
            }
            chunk_refs.push(ChunkRef { index: i as u32, hash: *hash, size: data.len() as u64 });
        }

        let next_id = RootHistory::open(self.paths.history_file())?.next_id()?;
        let timestamp = self.clock.unix_seconds();

        let manifest = GenerationManifest {
            schema_version: 1,
            generation_id: next_id,
            root,
            timestamp,
            chunks: chunk_refs,
            key_table,
        };
        manifest.write_to(self.paths.generation_manifest(&root_hex))?;

        let mut history = RootHistory::open(self.paths.history_file())?;
        history.append(&GenerationState { id: next_id, root, timestamp })?;
        staging.clear()?;

        Ok(root)
    }

    /// Resolve a chunk's bytes by content hash across ALL generation chunk dirs.
    /// Chunk bytes are content-addressed and stored once globally (§8.2), so a
    /// chunk introduced by an earlier generation lives only under that
    /// generation's `chunks/` dir; later generations referencing it have a
    /// sparse `chunks/`. Returns `ChunkNotFound` if no generation holds it.
    pub fn resolve_chunk(&self, hash: Bytes32) -> Result<Vec<u8>> {
        let gens = self.paths.generations_dir();
        if gens.exists() {
            let name = hash.to_hex();
            for entry in std::fs::read_dir(&gens)? {
                let candidate = entry?.path().join("chunks").join(&name);
                if candidate.exists() {
                    return Ok(std::fs::read(&candidate)?);
                }
            }
        }
        Err(StoreError::ChunkNotFound(hash.to_hex()))
    }

    /// Generations in chronological order (§20.4 `log`). Alias of root history.
    pub fn log(&self) -> Result<Vec<GenerationState>> {
        self.root_history()
    }

    /// Load a generation manifest by its root hash.
    pub fn generation_manifest(&self, root: Bytes32) -> Result<GenerationManifest> {
        let path = self.paths.generation_manifest(&root.to_hex());
        if !path.exists() {
            return Err(StoreError::GenerationNotFound(root.to_hex()));
        }
        GenerationManifest::read_from(path)
    }

    /// Diff two generations by root hash (§20.4 `diff`).
    pub fn diff(&self, a: Bytes32, b: Bytes32) -> Result<crate::diff::GenerationDiff> {
        let ma = self.generation_manifest(a)?;
        let mb = self.generation_manifest(b)?;
        Ok(crate::diff::GenerationDiff::between(&ma, &mb))
    }
}
