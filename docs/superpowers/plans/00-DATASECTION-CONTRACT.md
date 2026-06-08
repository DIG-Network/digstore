# Digstore Data-Section Contract (BINDING — single source of truth)

> Resolves the compiler↔guest data-section drift discovered after Segment D. **Defines ONE byte-exact format, owned by `digstore-core`**, that the compiler emits, the guest reads, and the client verifies. Supersedes any per-crate data-section format (compiler's old `SEG_*`/9-byte rows and the guest's private parser must both be replaced by the core module below).

## D0. Why
Independent agents invented two incompatible formats (9-byte `kind:u8` rows + IDs 0–4 in the compiler; 10-byte `id:u16` rows + IDs 1–10 in the guest) at two locations (active segment @ page 1 vs the `__digstore_data` symbol). Result: a real compiled module's `serve_content` returned 0 bytes — the module did not serve itself. This contract makes the module genuinely self-serving and its merkle proof genuinely verifying.

## D1. Canonical module: `digstore_core::datasection`
A new module in `digstore-core` (no_std + alloc) is the ONLY definition of the format. Both compiler (host) and guest (wasm) call it.

### Section IDs (`u16`)
```
StoreId      = 1   // 32 bytes, raw
CurrentRoot  = 2   // 32 bytes, raw (the module's current generation root)
RootHistory  = 3   // Vec<Bytes32> via core codec (u32 BE count + raw 32B each)
PublicKey    = 4   // 48 bytes, raw (store BLS G1 pubkey)
TrustedKeys  = 5   // Vec<TrustedHostKey> via core codec
Metadata     = 6   // MetadataManifest via core codec (plaintext, §8.4)
AuthInfo     = 7   // AuthenticationInfo via core codec
KeyTable     = 8   // see D3
ChunkPool    = 9   // see D4
MerkleNodes  = 10  // see D5
Filler       = 11  // deterministic ChaCha20 filler bytes (unreferenced; preserves §8.3 "filler in gaps" without disturbing chunk indexing)
```

### Blob layout (big-endian; deviation #1)
```
magic   : b"DIGS"            (4 bytes)
version : u8 = 1             (1 byte)
count   : u32 BE             (4 bytes)          number of offset rows
rows    : count × 10 bytes   each = id:u16 BE | offset:u32 BE | len:u32 BE
                                    (offset/len are relative to byte 0 of the blob)
bodies  : concatenated section bodies
```
`offset`/`len` point into the same blob. `total_len = max(offset+len)` — the blob is **self-describing**; no end symbol needed.

### Core API (single source of truth)
```rust
pub mod datasection {
    pub const MAGIC: &[u8;4] = b"DIGS";
    pub const VERSION: u8 = 1;
    /// Fixed linear-memory offset where the compiler injects the blob and the guest reads it.
    pub const DIGS_DATA_OFFSET: u32 = 0x0010_0000; // 1 MiB; above guest static data, below 16 MiB cap

    #[repr(u16)] pub enum SectionId { StoreId=1, CurrentRoot=2, RootHistory=3, PublicKey=4,
        TrustedKeys=5, Metadata=6, AuthInfo=7, KeyTable=8, ChunkPool=9, MerkleNodes=10, Filler=11 }

    /// Build the blob from (id, body) sections in ascending id order.
    pub fn encode_blob(sections: &[(u16, Vec<u8>)]) -> Vec<u8>;
    /// Zero-copy reader.
    pub struct DataView<'a> { /* parsed rows + raw */ }
    impl<'a> DataView<'a> {
        pub fn parse(raw: &'a [u8]) -> Result<DataView<'a>, DecodeError>;
        pub fn section(&self, id: SectionId) -> Option<&'a [u8]>;
        pub fn total_len(&self) -> usize;
    }

    // Body codecs (shared, byte-exact):
    pub fn encode_key_table(entries: &[KeyTableEntry]) -> Vec<u8>; // D3
    pub fn lookup_key(key_table_body: &[u8], retrieval_key: &Bytes32) -> Option<KeyTableEntry>;
    pub fn encode_chunk_pool(chunks_in_global_index_order: &[&[u8]]) -> Vec<u8>; // D4
    pub fn read_chunk(pool_body: &[u8], global_index: u32) -> Option<&[u8]>;
    pub fn encode_merkle_nodes(leaves: &[Bytes32]) -> Vec<u8>; // D5: Vec<Bytes32>
    pub fn decode_merkle_leaves(body: &[u8]) -> Result<Vec<Bytes32>, DecodeError>;
}
```
The guest's existing private `datasection.rs` parser and `encode_key_table`, and the compiler's `data_section.rs` `SEG_*`/`encode_data_section`, are both **deleted** and replaced by calls into `digstore_core::datasection`.

## D2. Injection + read location
- **Compiler** injects the blob as an **active data segment** at constant offset `DIGS_DATA_OFFSET` (use wasm-encoder `ActiveData` with `i32.const DIGS_DATA_OFFSET`). It must ensure the module's **minimum memory pages** cover `DIGS_DATA_OFFSET + total_len` (raise the template's memory `min` if needed via wasmparser/wasm-encoder).
- **Guest** reads the blob from the fixed pointer `DIGS_DATA_OFFSET as *const u8`, parses the header to learn `total_len`, then `DataView::parse(slice)`. The `__digstore_data`/`__digstore_data_end` extern-symbol scheme is **removed**; the native-test path passes a blob slice directly.

## D3. KeyTable body (id 8)
```
count : u32 BE
per entry:
  static_key   : 32 raw     (= retrieval_key the resource is found by)
  generation   : 32 raw
  index_count  : u32 BE
  indices      : index_count × u32 BE   (global chunk indices into ChunkPool, in order)
  total_size   : u64 BE     (reassembled plaintext size)
```

## D4. ChunkPool body (id 9)
```
count : u32 BE
per chunk (in GLOBAL INDEX ORDER): len : u32 BE | bytes(ciphertext)
```
`read_chunk(pool, i)` returns the i-th chunk's ciphertext. Deterministic ChaCha20 filler is NOT interleaved into the pool (that would break global indexing); it lives in the separate `Filler` section (id 11) so the module still carries indistinguishable filler bytes per §8.3 while chunk indexing stays exact.

## D5. Merkle model (RESOLVES the proof stub; documented deviation #5)
**Leaf = SHA-256(resource ciphertext blob)**, where the resource ciphertext blob is `concat_output(ordered chunk ciphertexts)` (the exact bytes `get_content` returns). The generation's merkle tree is built over **one leaf per resource** (ascending by `static_key`). `generation root = tree root`.

- *Deviation from §9.1 literal* ("leaf = SHA-256(chunk)"): the commitment is per-**resource**, not per-chunk. Rationale: a `ContentResponse` carries exactly one `merkle_proof` and one served resource; a per-resource leaf lets that single proof verify the entire served resource to the trusted root. Chunk-level dedup is unaffected (it happens in the ChunkPool/global index); only the merkle commitment granularity changes. This is consistent across store, compiler, guest, and cli.
- **Compiler** injects `MerkleNodes` (id 10) = the ordered resource leaves (`Vec<Bytes32>`), and sets `CurrentRoot` (id 2) = `MerkleTree::from_leaves(leaves).root()`.
- **Guest** `build_real_proof`: rebuild `MerkleTree::from_leaves(decode_merkle_leaves(nodes))`, find the served resource's leaf index (its position among sorted static_keys; the KeyTable order = leaf order), and emit `tree.prove(leaf_index)`. The emitted `MerkleProof { leaf, path, root }` now **verifies** (`MerkleProof::verify()` recomputes `root`).
- **Store** builds the generation tree with per-resource leaves (leaf = SHA-256 of the resource's ciphertext blob) so `GenerationState.root` matches what the compiler injects. (§9.4 invariant: `state.root == tree.root()` still holds.)
- **Client (cli)** verifies the served `ContentResponse.merkle_proof` against the **trusted root** (from root history), confirms `proof.leaf == SHA-256(ciphertext)`, then GCM-opens each chunk.

## D6. Authoritative serving path (the core promise)
The CLI's `cat`/`checkout` and the remote's content endpoint MUST obtain served bytes via `digstore_host::HostRuntime::serve_content` on the **real compiled module** — NOT by parsing the data section host-side. The module serves itself. Required new test (host or cli integration):
```
init → add(file) → commit(compile real .wasm) → HostRuntime::new(module).serve_content(request)
 → decode ContentResponse → assert proof.verify() && proof.root == trusted_root
 → assert proof.leaf == sha256(ciphertext)
 → client derive key + GCM-open each chunk → assert == original file bytes
```
Plus: `serve_content` for a miss returns a decoy whose `proof.root != trusted_root` (fails the client gate); a private-store cat without salt fails the GCM tag.

## D7. Determinism + secretless retained
- `encode_blob` is deterministic; the `Filler` section uses the existing deterministic ChaCha20 stream (deviation #2). Double-compile must stay byte-identical (compiler determinism test).
- Secretless scan still holds: blob contains only ciphertext, public metadata, public keys, deterministic filler, and merkle leaves (hashes) — no decryption key, signing key, or salt.
