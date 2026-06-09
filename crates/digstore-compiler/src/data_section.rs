//! Encode the data-section blob in the BINDING contract format (D1–D5).
//!
//! The byte-exact format is owned by [`digstore_core::datasection`]; this module
//! is a thin compiler-side adapter that gathers the typed inputs and emits the
//! sections in ascending id order via [`digstore_core::datasection::encode_blob`].
//! The compiler's old private `SEG_*`/9-byte-row format is **deleted**: the
//! single source of truth is now core, so the compiler emits exactly what the
//! guest reads and the client verifies.
//!
//! Sections (D1):
//! ```text
//!  StoreId      = 1   32 bytes raw
//!  CurrentRoot  = 2   32 bytes raw (per-resource merkle root, D5)
//!  RootHistory  = 3   Vec<Bytes32> (u32 BE count + raw 32B each)
//!  PublicKey    = 4   48 bytes raw
//!  TrustedKeys  = 5   Vec<TrustedHostKey> (compiler-local codec, byte-exact)
//!  Metadata     = 6   MetadataManifest via core Encode (plaintext, §8.4)
//!  AuthInfo     = 7   AuthenticationInfo via core Encode
//!  KeyTable     = 8   core encode_key_table (D3)
//!  ChunkPool    = 9   core encode_chunk_pool, GLOBAL INDEX ORDER (D4)
//!  MerkleNodes  = 10  core encode_merkle_nodes = per-resource leaves (D5)
//!  Filler       = 11  deterministic ChaCha20 filler (unreferenced, §8.3)
//! ```
//! Multi-byte integers are big-endian (Chia streamable, deviation #1).

use digstore_core::datasection::{encode_chunk_pool, encode_key_table, encode_merkle_nodes};
use digstore_core::{
    AuthenticationInfo, Bytes32, Bytes48, Encode, Encoder, KeyTableEntry, MetadataManifest,
    TrustedHostKey,
};

/// All inputs needed to encode the contract data-section blob (gathered by the
/// pipeline). Public byte material only — no secrets (§17.2).
pub struct DataSectionInputs {
    pub store_id: Bytes32,
    /// Per-resource merkle root of the CURRENT generation (D5): equals
    /// `MerkleTree::from_leaves(merkle_leaves).root()`.
    pub current_root: Bytes32,
    pub root_history: Vec<Bytes32>,
    pub store_pubkey: Bytes48,
    pub trusted_keys: Vec<TrustedHostKey>,
    pub manifest: MetadataManifest,
    pub auth_info: AuthenticationInfo,
    /// KeyTable entries (D3), in deterministic build order.
    pub key_table: Vec<KeyTableEntry>,
    /// Unique chunk ciphertext bodies in GLOBAL INDEX order (D4); the
    /// `chunk_indices` in `key_table` address into this list.
    pub chunk_pool_bodies: Vec<Vec<u8>>,
    /// Per-resource merkle leaves of the current generation, ascending by
    /// `static_key` (D5). `MerkleNodes` (id 10) carries exactly these.
    pub merkle_leaves: Vec<Bytes32>,
    /// Deterministic ChaCha20 filler bytes (unreferenced; §8.3, deviation #2).
    pub filler: Vec<u8>,
}

/// Encode `Vec<TrustedHostKey>` using core primitive framing field-by-field.
///
/// DEVIATION: `digstore_core::TrustedHostKey` does NOT implement `Encode`/`Decode`
/// (the compiler may not edit core). We reproduce the exact framing core would use
/// for a derived impl — `Vec` is a 4-byte BE count, each entry is
/// `[u8;48] public_key` (raw, no prefix) then `String label` (4-byte BE len +
/// bytes) — so the guest's matching decode reads identical bytes.
fn encode_trusted_keys(keys: &[TrustedHostKey]) -> Vec<u8> {
    let mut enc = Encoder::new();
    (keys.len() as u32).encode(&mut enc);
    for k in keys {
        k.public_key.encode(&mut enc);
        k.label.encode(&mut enc);
    }
    enc.finish()
}

/// Encode `Vec<Bytes32>` via core framing (u32 BE count + raw 32B each).
fn encode_root_history(roots: &[Bytes32]) -> Vec<u8> {
    let mut enc = Encoder::new();
    roots.to_vec().encode(&mut enc);
    enc.finish()
}

/// Build the full data-section blob in the canonical contract format (D1).
///
/// Sections are emitted in ascending id order (1..=11); the offset table and
/// header are produced by [`digstore_core::datasection::encode_blob`], so the
/// bytes are byte-identical to what the guest's `DataView` parses.
pub fn encode_data_section(i: &DataSectionInputs) -> Vec<u8> {
    use digstore_core::datasection::SectionId;

    let pool_refs: Vec<&[u8]> = i.chunk_pool_bodies.iter().map(|b| b.as_slice()).collect();

    let sections: Vec<(u16, Vec<u8>)> = vec![
        (SectionId::StoreId as u16, i.store_id.0.to_vec()),
        (SectionId::CurrentRoot as u16, i.current_root.0.to_vec()),
        (
            SectionId::RootHistory as u16,
            encode_root_history(&i.root_history),
        ),
        (SectionId::PublicKey as u16, i.store_pubkey.0.to_vec()),
        (
            SectionId::TrustedKeys as u16,
            encode_trusted_keys(&i.trusted_keys),
        ),
        (SectionId::Metadata as u16, i.manifest.to_bytes()),
        (SectionId::AuthInfo as u16, i.auth_info.to_bytes()),
        (SectionId::KeyTable as u16, encode_key_table(&i.key_table)),
        (SectionId::ChunkPool as u16, encode_chunk_pool(&pool_refs)),
        (
            SectionId::MerkleNodes as u16,
            encode_merkle_nodes(&i.merkle_leaves),
        ),
        (SectionId::Filler as u16, i.filler.clone()),
    ];

    digstore_core::datasection::encode_blob(&sections)
}

/// Re-key a compiled module to a new set of trusted host keys.
///
/// §12.2: a module verifies the serving host's BLS attestation against the
/// trusted set embedded in its data section. A party that re-deploys a module on
/// its OWN serving node (e.g. `dig clone`) must therefore re-embed its own host
/// key so its node can attest. This extracts the module's DIGS blob, replaces
/// ONLY the `TrustedKeys` section (every other section — chunks, key table,
/// merkle nodes, current root — is preserved byte-for-byte, so the served
/// content and its proof are unchanged), and re-injects the blob.
pub fn rekey_module_trusted(
    module: &[u8],
    new_trusted: &[TrustedHostKey],
) -> Result<Vec<u8>, crate::error::CompilerError> {
    use crate::inject::{extract_data_section, inject_data_section};
    use crate::pipeline::DATA_SECTION_MEM_OFFSET;
    use digstore_core::datasection::{encode_blob, DataView, SectionId};

    let blob = extract_data_section(module, DATA_SECTION_MEM_OFFSET)?;
    let view = DataView::parse(&blob).map_err(|e| {
        crate::error::CompilerError::InvalidTemplate(format!("bad DIGS blob: {e:?}"))
    })?;

    // Rebuild every section in ascending id order, swapping TrustedKeys.
    const IDS: [SectionId; 11] = [
        SectionId::StoreId,
        SectionId::CurrentRoot,
        SectionId::RootHistory,
        SectionId::PublicKey,
        SectionId::TrustedKeys,
        SectionId::Metadata,
        SectionId::AuthInfo,
        SectionId::KeyTable,
        SectionId::ChunkPool,
        SectionId::MerkleNodes,
        SectionId::Filler,
    ];
    let mut sections: Vec<(u16, Vec<u8>)> = Vec::new();
    for id in IDS {
        let body = if id == SectionId::TrustedKeys {
            encode_trusted_keys(new_trusted)
        } else {
            match view.section(id) {
                Some(b) => b.to_vec(),
                None => continue, // absent section: keep it absent
            }
        };
        sections.push((id as u16, body));
    }
    let new_blob = encode_blob(&sections);
    inject_data_section(module, &new_blob, DATA_SECTION_MEM_OFFSET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::datasection::{
        decode_merkle_leaves, lookup_key, read_chunk, DataView, SectionId,
    };
    use digstore_core::merkle::MerkleTree;
    use digstore_core::{AuthenticationInfo, Bytes32, Bytes48, Decode, Decoder, MetadataManifest};

    fn manifest() -> MetadataManifest {
        MetadataManifest {
            schema_version: 1,
            name: "n".into(),
            version: None,
            description: None,
            authors: vec![],
            license: None,
            homepage: None,
            repository: None,
            keywords: vec![],
            categories: vec![],
            icon: None,
            content_type: None,
            links: Default::default(),
            custom: Default::default(),
        }
    }

    fn auth() -> AuthenticationInfo {
        AuthenticationInfo {
            requires_session: false,
            requires_jwt: false,
            jwks_url: None,
            accepted_algorithms: vec![],
        }
    }

    fn inputs() -> DataSectionInputs {
        let leaves = vec![Bytes32([0x33; 32]), Bytes32([0x44; 32])];
        let root = MerkleTree::from_leaves(leaves.clone()).root();
        DataSectionInputs {
            store_id: Bytes32([0xAB; 32]),
            current_root: root,
            root_history: vec![Bytes32([0x11; 32]), Bytes32([0x22; 32])],
            store_pubkey: Bytes48([0xCD; 48]),
            trusted_keys: vec![TrustedHostKey {
                public_key: [0x42u8; 48],
                label: "L".into(),
            }],
            manifest: manifest(),
            auth_info: auth(),
            key_table: vec![KeyTableEntry {
                static_key: Bytes32([1; 32]),
                generation: Bytes32([0x11; 32]),
                chunk_indices: vec![0],
                total_size: 6,
            }],
            chunk_pool_bodies: vec![b"abcdef".to_vec()],
            merkle_leaves: leaves,
            filler: vec![0x09u8; 16],
        }
    }

    #[test]
    fn starts_with_magic_and_version() {
        let blob = encode_data_section(&inputs());
        assert_eq!(&blob[0..4], b"DIGS");
        assert_eq!(blob[4], 1u8);
    }

    #[test]
    fn offset_table_has_eleven_sections_in_ascending_id_order() {
        let blob = encode_data_section(&inputs());
        let count = u32::from_be_bytes([blob[5], blob[6], blob[7], blob[8]]);
        assert_eq!(count, 11);
        // Rows: id(u16 BE) | offset(u32) | len(u32) = 10 bytes each, starting at 9.
        let mut prev_id = 0u16;
        for row in 0..11usize {
            let p = 9 + row * 10;
            let id = u16::from_be_bytes([blob[p], blob[p + 1]]);
            assert!(id > prev_id, "ids must be strictly ascending");
            prev_id = id;
        }
        assert_eq!(prev_id, 11, "last section id is Filler=11");
    }

    #[test]
    fn view_round_trips_every_section() {
        let inp = inputs();
        let blob = encode_data_section(&inp);
        let view = DataView::parse(&blob).expect("parses");

        assert_eq!(view.section(SectionId::StoreId).unwrap(), &inp.store_id.0);
        assert_eq!(
            view.section(SectionId::CurrentRoot).unwrap(),
            &inp.current_root.0
        );
        assert_eq!(
            view.section(SectionId::PublicKey).unwrap(),
            &inp.store_pubkey.0
        );

        // RootHistory decodes as Vec<Bytes32>.
        let rh = view.section(SectionId::RootHistory).unwrap();
        let mut dec = Decoder::new(rh);
        let hist = Vec::<Bytes32>::decode(&mut dec).unwrap();
        assert_eq!(hist, inp.root_history);

        // Metadata decodes as MetadataManifest.
        let md = view.section(SectionId::Metadata).unwrap();
        let mut dec = Decoder::new(md);
        let m = MetadataManifest::decode(&mut dec).unwrap();
        assert_eq!(m.name, "n");

        // AuthInfo decodes.
        let ai = view.section(SectionId::AuthInfo).unwrap();
        let mut dec = Decoder::new(ai);
        let a = AuthenticationInfo::decode(&mut dec).unwrap();
        assert_eq!(a, inp.auth_info);

        // KeyTable: lookup by static_key.
        let kt = view.section(SectionId::KeyTable).unwrap();
        let entry = lookup_key(kt, &Bytes32([1; 32])).expect("found");
        assert_eq!(entry.chunk_indices, vec![0]);
        assert_eq!(entry.total_size, 6);

        // ChunkPool: read chunk 0.
        let pool = view.section(SectionId::ChunkPool).unwrap();
        assert_eq!(read_chunk(pool, 0).unwrap(), b"abcdef");

        // MerkleNodes decodes back to the leaves, and CurrentRoot == tree root.
        let mn = view.section(SectionId::MerkleNodes).unwrap();
        let leaves = decode_merkle_leaves(mn).unwrap();
        assert_eq!(leaves, inp.merkle_leaves);
        assert_eq!(MerkleTree::from_leaves(leaves).root(), inp.current_root);

        // Filler present.
        assert_eq!(view.section(SectionId::Filler).unwrap(), &inp.filler[..]);
    }

    #[test]
    fn deterministic_encode() {
        let a = encode_data_section(&inputs());
        let b = encode_data_section(&inputs());
        assert_eq!(a, b);
    }
}
