//! Build the normalized public manifest (the store's complete public file set,
//! latest version per path) by walking every generation on disk.
//!
//! The per-generation [`GenerationManifest`] each commit writes lists only the
//! resources committed IN that generation. The public manifest flattens the whole
//! history into one entry per public path, holding that path's LATEST version and
//! its provenance (see [`digstore_core::PublicManifest`]). This is the source the
//! compiler embeds as the `.dig` `PublicManifest` section and the CLI `manifest`
//! command prints.

use crate::error::{Result, StoreError};
use crate::generation::GenerationManifest;
use digstore_core::serving::concat_output;
use digstore_core::{resource_leaf, Bytes32, PublicManifest, PublicManifestEntry};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The winning (latest) occurrence of a path while scanning generations oldest → newest.
struct Winner {
    latest_root: Bytes32,
    generation_index: u64,
    /// The generation manifest that holds the winning version (for chunk lookup).
    manifest_idx: usize,
    /// Ordered chunk hashes of the winning version's content.
    chunk_hashes: Vec<Bytes32>,
    version_count: u32,
}

/// Read every `generations/<root>/manifest.json` under `generations_dir` and
/// compute the normalized public manifest: one entry per public path, with the
/// latest version's root + generation index + content SHA-256 + total version
/// count across the store history.
///
/// `sha256_latest` is the D5 per-resource content leaf — `SHA-256` over the
/// concatenated ordered chunk ciphertext bodies of the latest version — recomputed
/// from the on-disk chunk bodies (resolved globally across generation chunk dirs,
/// matching the store's content-addressed dedup). Entries are ordered ascending by
/// path.
///
/// A missing/empty `generations_dir` yields an empty manifest (a store with no
/// committed capsule has no public files yet).
pub fn build_public_manifest(generations_dir: impl AsRef<Path>) -> Result<PublicManifest> {
    let generations_dir = generations_dir.as_ref();
    if !generations_dir.exists() {
        return Ok(PublicManifest::new(Vec::new()));
    }

    // Load every generation manifest, then order oldest → newest by generation id.
    let mut manifests: Vec<GenerationManifest> = Vec::new();
    for entry in std::fs::read_dir(generations_dir)? {
        let path = entry?.path();
        let manifest_path = path.join("manifest.json");
        if manifest_path.exists() {
            manifests.push(GenerationManifest::read_from(&manifest_path)?);
        }
    }
    manifests.sort_by_key(|m| m.generation_id);

    // First pass: latest occurrence per path + version count. "Latest" is the
    // highest generation id containing the path (we scan ascending, last wins).
    let mut winners: BTreeMap<String, Winner> = BTreeMap::new();
    for (manifest_idx, m) in manifests.iter().enumerate() {
        for rec in &m.key_table {
            let chunk_hashes = ordered_chunk_hashes(m, rec);
            match winners.get_mut(&rec.resource_key) {
                Some(w) => {
                    w.version_count = w.version_count.saturating_add(1);
                    w.latest_root = m.root;
                    w.generation_index = m.generation_id;
                    w.manifest_idx = manifest_idx;
                    w.chunk_hashes = chunk_hashes;
                }
                None => {
                    winners.insert(
                        rec.resource_key.clone(),
                        Winner {
                            latest_root: m.root,
                            generation_index: m.generation_id,
                            manifest_idx,
                            chunk_hashes,
                            version_count: 1,
                        },
                    );
                }
            }
        }
    }

    // Global chunk index (content hash → on-disk path) for content-hash recompute.
    // Only built if there is at least one winner needing chunk bytes.
    let chunk_index = if winners.is_empty() {
        BTreeMap::new()
    } else {
        global_chunk_index(generations_dir)?
    };

    // Second pass: compute sha256_latest for each winning version only.
    let mut entries = Vec::with_capacity(winners.len());
    for (path, w) in winners {
        let sha256_latest = content_leaf(&w.chunk_hashes, &chunk_index, w.manifest_idx)?;
        entries.push(PublicManifestEntry {
            path,
            latest_root: w.latest_root,
            generation_index: w.generation_index,
            sha256_latest,
            version_count: w.version_count,
        });
    }
    Ok(PublicManifest::new(entries))
}

/// The ordered content chunk hashes for a resource record: map its
/// `chunk_indices` (into the generation's pool) to the pool's chunk hashes.
fn ordered_chunk_hashes(
    m: &GenerationManifest,
    rec: &crate::generation::KeyTableRecord,
) -> Vec<Bytes32> {
    let by_index: BTreeMap<u32, Bytes32> = m.chunks.iter().map(|c| (c.index, c.hash)).collect();
    rec.chunk_indices
        .iter()
        .filter_map(|i| by_index.get(i).copied())
        .collect()
}

/// Recompute the D5 content leaf for a resource: `SHA-256(concat(ordered chunk
/// ciphertext bodies))`. Chunk bodies are content-addressed and resolved globally
/// across generation chunk dirs (dedup means a chunk may live under any generation).
fn content_leaf(
    chunk_hashes: &[Bytes32],
    chunk_index: &BTreeMap<String, PathBuf>,
    manifest_idx: usize,
) -> Result<Bytes32> {
    let mut bodies: Vec<Vec<u8>> = Vec::with_capacity(chunk_hashes.len());
    for h in chunk_hashes {
        let p = chunk_index.get(&h.to_hex()).ok_or_else(|| {
            StoreError::ChunkNotFound(format!("{} (generation index {manifest_idx})", h.to_hex()))
        })?;
        bodies.push(std::fs::read(p)?);
    }
    let slices: Vec<&[u8]> = bodies.iter().map(|b| b.as_slice()).collect();
    Ok(resource_leaf(&concat_output(&slices)))
}

/// Map every stored chunk's content-hash hex → its on-disk path, across all
/// `generations/<root>/chunks/<hash>` files (global dedup index).
fn global_chunk_index(generations_dir: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut index = BTreeMap::new();
    for entry in std::fs::read_dir(generations_dir)? {
        let chunks_dir = entry?.path().join("chunks");
        if !chunks_dir.exists() {
            continue;
        }
        for chunk in std::fs::read_dir(&chunks_dir)? {
            let path = chunk?.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                index.entry(name.to_string()).or_insert(path);
            }
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::{ChunkRef, KeyTableRecord};
    use digstore_core::resource_leaf;
    use tempfile::tempdir;

    /// One test resource: `(resource_key, ordered (chunk_hash, ciphertext body))`.
    type TestResource<'a> = (&'a str, Vec<(Bytes32, Vec<u8>)>);

    fn write_gen(gens: &Path, gen_id: u64, root: Bytes32, resources: &[TestResource<'_>]) {
        let root_hex = root.to_hex();
        let chunks_dir = gens.join(&root_hex).join("chunks");
        std::fs::create_dir_all(&chunks_dir).unwrap();
        let mut chunk_refs = Vec::new();
        let mut key_table = Vec::new();
        let mut idx = 0u32;
        for (rk, chunks) in resources {
            let mut indices = Vec::new();
            let mut total = 0u64;
            for (hash, body) in chunks {
                std::fs::write(chunks_dir.join(hash.to_hex()), body).unwrap();
                chunk_refs.push(ChunkRef {
                    index: idx,
                    hash: *hash,
                    size: body.len() as u64,
                });
                indices.push(idx);
                total += body.len() as u64;
                idx += 1;
            }
            key_table.push(KeyTableRecord {
                resource_key: (*rk).into(),
                static_key: Bytes32([0xEE; 32]),
                generation: root,
                chunk_indices: indices,
                total_size: total,
            });
        }
        let m = GenerationManifest {
            schema_version: 1,
            generation_id: gen_id,
            root,
            timestamp: 1_000 + gen_id,
            chunks: chunk_refs,
            key_table,
        };
        m.write_to(gens.join(&root_hex).join("manifest.json"))
            .unwrap();
    }

    #[test]
    fn empty_when_no_generations() {
        let td = tempdir().unwrap();
        let pm = build_public_manifest(td.path().join("generations")).unwrap();
        assert!(pm.entries.is_empty());
    }

    #[test]
    fn normalizes_latest_per_path_across_generations() {
        let td = tempdir().unwrap();
        let gens = td.path().join("generations");
        // gen 0: index.html (v1), style.css (v1)
        let idx_v1 = (Bytes32([1; 32]), b"<h1>v1</h1>".to_vec());
        let css_v1 = (Bytes32([2; 32]), b"body{}".to_vec());
        write_gen(
            &gens,
            0,
            Bytes32([0xA0; 32]),
            &[
                ("index.html", vec![idx_v1]),
                ("style.css", vec![css_v1.clone()]),
            ],
        );
        // gen 1: index.html (v2) only — style.css unchanged (latest stays gen 0).
        let idx_v2 = (Bytes32([3; 32]), b"<h1>v2 longer</h1>".to_vec());
        write_gen(
            &gens,
            1,
            Bytes32([0xA1; 32]),
            &[("index.html", vec![idx_v2.clone()])],
        );

        let pm = build_public_manifest(&gens).unwrap();
        assert_eq!(pm.entries.len(), 2);
        // Sorted by path: index.html, style.css.
        let index = &pm.entries[0];
        assert_eq!(index.path, "index.html");
        assert_eq!(index.latest_root, Bytes32([0xA1; 32])); // latest is gen 1
        assert_eq!(index.generation_index, 1);
        assert_eq!(index.version_count, 2);
        // sha256_latest = resource_leaf(v2 body).
        assert_eq!(index.sha256_latest, resource_leaf(&idx_v2.1));

        let style = &pm.entries[1];
        assert_eq!(style.path, "style.css");
        assert_eq!(style.latest_root, Bytes32([0xA0; 32])); // latest still gen 0
        assert_eq!(style.generation_index, 0);
        assert_eq!(style.version_count, 1);
        assert_eq!(style.sha256_latest, resource_leaf(&css_v1.1));
    }

    #[test]
    fn multi_chunk_content_leaf_is_ordered_concat() {
        let td = tempdir().unwrap();
        let gens = td.path().join("generations");
        let c0 = (Bytes32([10; 32]), b"AAAA".to_vec());
        let c1 = (Bytes32([11; 32]), b"BBBB".to_vec());
        write_gen(
            &gens,
            0,
            Bytes32([0xB0; 32]),
            &[("data.bin", vec![c0.clone(), c1.clone()])],
        );
        let pm = build_public_manifest(&gens).unwrap();
        let mut concat = c0.1.clone();
        concat.extend_from_slice(&c1.1);
        assert_eq!(pm.entries[0].sha256_latest, resource_leaf(&concat));
    }
}
