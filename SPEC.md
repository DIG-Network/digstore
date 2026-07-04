# digstore — `.dig` store format & manifest specification

This is the NORMATIVE contract for the digstore `.dig` store format: the byte-exact
data-section blob, the capsule/generation model, per-resource crypto and merkle
commitment, and the normalized public manifest. An independent implementation that
reads or writes `.dig` modules MUST conform to this document. The single source of
truth in code is `digstore-core` (`datasection`, `merkle`, `crypto`, `urn`,
`public_manifest`); this spec MUST agree with it and with the ecosystem contracts in
the superproject `SYSTEM.md` and the user-facing protocol pages under `docs.dig.net`.

All multi-byte integers are **big-endian** (Chia streamable framing). The shared codec
(`digstore_core::codec`) frames: `uN` as `N/8` BE bytes; `String` as `u32` byte length
+ UTF-8 bytes; `Vec<T>` as `u32` count + each `T`; `Bytes32`/`Bytes48` as raw fixed
bytes; `Option<T>` as a `1`-byte tag (`0`=None, `1`=Some) then the value.

## 1. Identity, URN, and retrieval key

- A **store** is identified by its 32-byte `store_id` (the on-chain CHIP-0035 DataStore
  launcher id). It is NOT a hash of any key.
- A resource's canonical **URN** is `urn:dig:<chain>:<storeID>[:<rootHash>][/<resourceKey>]`
  with `chain = "chia"`. Key derivation uses the ROOT-INDEPENDENT form (root omitted).
- The **retrieval key** = `SHA-256(canonical_rootless_urn)`, lowercase hex. It is the only
  store-side identifier a client sends to a CDN/RPC; the URN itself is never transmitted.
- A URN with no resource key resolves to the default view `index.html` (§8.5 convention).

## 2. Visibility

- **Public** store: the per-resource AES key derives from the URN alone; anyone who knows
  the `store_id` and a path can derive the retrieval + decryption keys. All resources are
  public.
- **Private** store: a 32-byte secret salt is additionally mixed into key derivation.
  Resource paths are secret — nothing may map a public name to a private resource.

## 3. Per-resource crypto and content commitment

- Each resource is chunked (FastCDC-style; min 16 KiB, target 64 KiB, max 256 KiB).
- Each chunk is sealed with **AES-256-GCM-SIV** under the per-resource content key derived
  by **HKDF-SHA256** (salt `b"digstore-hkdf-salt-v1"`, info `b"digstore-aes-256-gcm-key-v1"`;
  private stores mix the 32-byte secret salt). Chunks are stored and content-addressed as
  CIPHERTEXT, keyed by `SHA-256(ciphertext)`.
- The served bytes for a resource are the PLAIN ordered concatenation of its chunk
  ciphertexts (no length framing on the wire).
- The **per-resource merkle leaf (D5)** is `resource_leaf = SHA-256(concat(ordered chunk
  ciphertext bodies))` — i.e. `SHA-256` over exactly the bytes served for the resource. This
  is the single content→leaf binding shared by the producer and the browser verifier.
- A generation's **merkle tree** has ONE leaf per resource, ordered ascending by `static_key`
  (the resource's retrieval key). Its root is the generation's `CurrentRoot`.

## 4. Capsules, generations, and root history

- A **capsule** = one immutable generation = `(store_id, rootHash)`, canonical string
  `storeId:rootHash`. Each `commit` produces one capsule.
- Each generation persists a `GenerationManifest` (JSON) listing the resources committed IN
  that generation: `{ schema_version, generation_id, root, timestamp, chunks[], key_table[] }`,
  where each `key_table` record carries `resource_key`, `static_key`, `generation`,
  `chunk_indices`, `total_size`. A generation's key table is the DELTA committed in that
  generation, not the cumulative file set.
- **RootHistory** is the append-only, strictly monotonic sequence of `(generation_id, root,
  timestamp)`, oldest → newest.

## 5. The DIGS data-section blob

The compiled `.dig` module embeds a self-describing data-section blob (BINDING contract D1):

```text
magic    "DIGS"              (4 bytes)
version  u8 = 1              (1 byte)
count    u32 BE              (4 bytes)   number of offset rows
rows     count × 10 bytes:   id:u16 BE | offset:u32 BE | len:u32 BE   (offset/len from byte 0)
bodies   concatenated section bodies
```

`total_len = max(offset + len)`. `DataView::parse` validates the magic, `version == 1`, and
that every row lies within the blob.

### 5.1 Section ids

| id | Section | Body | Presence |
|---|---|---|---|
| 1 | StoreId | 32 raw bytes | always |
| 2 | CurrentRoot | 32 raw bytes (current generation's merkle root, D5) | always |
| 3 | RootHistory | `Vec<Bytes32>` | always |
| 4 | PublicKey | 48 raw bytes (BLS G1 publisher key) | always |
| 5 | TrustedKeys | `Vec<{ [u8;48] public_key, String label }>` | always |
| 6 | Metadata | `MetadataManifest` (plaintext) | always |
| 7 | AuthInfo | `AuthenticationInfo` | always |
| 8 | KeyTable (D3) | `u32` count, per entry `static_key(32)` \| `generation(32)` \| `chunk_indices(Vec<u32>)` \| `total_size(u64)` | always |
| 9 | ChunkPool (D4) | `u32` count, per chunk `len(u32)` \| `ciphertext` — global index order | always |
| 10 | MerkleNodes (D5) | `u32` count + count×32 raw — per-resource leaves, ascending by `static_key` | always |
| 11 | Filler | unreferenced deterministic ChaCha20 padding (uniform-size obfuscation) | always; MUST be the trailing/highest-offset body |
| 12 | ChainState | on-chain anchor pointer | optional |
| 13 | **PublicManifest** | normalized public file set (§6) | optional (PUBLIC stores only) |

The producer emits the always-present sections in ascending id order, then the optional
ChainState (12) and PublicManifest (13) BEFORE Filler, so Filler remains the highest-offset
body (the uniform-size padding grows only Filler).

### 5.2 Forward/backward compatibility (HARD RULE)

- Section ids are **only ever added** — never removed, renumbered, or repurposed. The body
  layout of an existing id never changes meaning. The blob `version` stays `1`.
- A reader looks each section up by id and **ignores unknown ids**. A newer writer's blob
  therefore parses in an older reader (which simply sees fewer sections), and a newer reader
  treats an absent optional section as "not present" (never an error).
- Producers keep golden `.dig` data-section fixtures of each released format; a reader change
  MUST prove it decodes older golden fixtures unchanged.

## 6. The normalized public manifest (SectionId 13)

The public manifest is the store's COMPLETE public file surface, flattened across every
published capsule: **one entry per public file path, holding that path's LATEST version and
its provenance**. Where the KeyTable (8) lists a single generation's resources by hashed
`static_key`, the public manifest exposes the human path and, for each path, which capsule +
version index hold its latest content — including files whose latest version lives in an
EARLIER capsule.

**Presence.** The section is embedded **only for PUBLIC stores**. A private store's paths are
secret, so it carries NO PublicManifest section.

**Normalization rule.** Walk every generation oldest → newest (by `generation_id`). For each
path (resource key): the LATEST version is the one in the highest `generation_id` whose file
set includes the path; `version_count` is the number of generations whose file set includes
the path.

**Body layout** (codec framing):

```text
schema_version : u32 BE
entries        : Vec, u32 BE count, per entry:
  path             : String   (u32 BE len + utf8 bytes)
  latest_root      : 32 raw bytes
  generation_index : u64 BE
  sha256_latest    : 32 raw bytes
  version_count    : u32 BE
```

Entries are ordered **ascending by `path`** (UTF-8 byte order); the encoding is deterministic.

**Field contract.**

| Field | Type | Meaning |
|---|---|---|
| `path` | string | The public file path (resource key), e.g. `index.html`, `assets/app.js`. |
| `latest_root` | Bytes32 | The capsule (root) holding this path's latest version. |
| `generation_index` | u64 | The generation id of that latest version (the commit that last wrote the path). |
| `sha256_latest` | Bytes32 | SHA-256 of the latest version's content = the D5 per-resource leaf: `SHA-256` over the concatenated ordered chunk ciphertext bodies of the latest version. |
| `version_count` | u32 | How many versions of the path exist across the whole history (generations that include it). |

`schema_version` starts at `1`; future fields are only APPENDED, so a reader dispatches on
the version and older bodies remain readable.

**JSON surface.** The CLI `digstore manifest --json`, the JSON-RPC `dig.getManifest`, and the
browser reader `readPublicManifest` all emit the SAME shape with the byte fields as 64-char
lowercase hex:

```json
{
  "schema_version": 1,
  "entries": [
    { "path": "index.html", "latest_root": "<64-hex>", "generation_index": 1,
      "sha256_latest": "<64-hex>", "version_count": 2 }
  ]
}
```

## 7. Reader/producer surfaces

- **Producer** — `digstore_store::build_public_manifest(generations_dir)` computes the
  manifest from on-disk generations; the compiler (`digstore-compiler`) embeds it as section
  13 for public stores.
- **Blob reader** — `digstore_core::datasection::read_public_manifest(blob)` returns
  `Option<PublicManifest>` (None when absent). `dig-client-wasm::readPublicManifest(blob)`
  exposes it to the browser as JSON.
- **CLI** — `digstore manifest [--json]` prints the normalized manifest.

## 8. Conformance

- The public manifest field names, types, ordering, and byte layout MUST match §6 exactly so
  hub.dig.net and dig-node reproduce it byte-for-byte (see `SYSTEM.md` → Shared contracts →
  "Normalized public manifest").
- `sha256_latest` MUST equal the D5 `resource_leaf` of the latest version, so a consumer can
  cross-check it against the served bytes and the merkle root.
- The capsule format and the additive-section rule are mirrored in
  `docs.dig.net/docs/protocol/capsule-format.md`; all three MUST agree.
