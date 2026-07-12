# digstore — `.dig` store format & manifest specification

This is the NORMATIVE contract for the digstore `.dig` store format: the byte-exact
data-section blob, the capsule/generation model, per-resource crypto and merkle
commitment, and the normalized public manifest. An independent implementation that
reads or writes `.dig` modules MUST conform to this document. The single source of
truth in code is `digstore-core` (`datasection`, `merkle`, `crypto`, `urn`,
`public_manifest`); this spec MUST agree with it and with the ecosystem contracts in
the superproject `SYSTEM.md` and the user-facing protocol pages under `docs.dig.net`.

**CLI binaries.** The `digstore-cli` crate ships TWO binaries, `digstore` and `digs`.
`digs` is a first-class alias: `digs <args>` behaves IDENTICALLY to `digstore <args>`
(same subcommands, flags, `--json`, and exit codes). Both share ONE codepath
(`digstore_cli::run()`) and each reflects its own invoked name (arg0) in
`--help`/`--version`/`completion`/`--help-json`. Both binaries MUST be shipped together
everywhere `digstore` ships (cargo-install, the universal installer, the apt `.deb`).

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

## 9. Well-known origin pubkey discovery & writer authorization

Scope note: unlike §1–8, this section is a CLI/on-chain-authority contract, not a `.dig`
byte-format contract — it is kept here because it is a public surface an independent
`digstore` reimplementation must also expose.

- `digstore authorize-origin-as-writer <origin> [--pubkey <hex>] [--dry-run] [--fee <mojos>]`
  authorizes an origin's DIG identity as a CHIP-0035 **writer** delegate on the active store's
  on-chain singleton, using the existing `digstore_chain::singleton` delegation primitive
  (`writer_delegated_puzzle` + `update_store_ownership`) — never a hand-rolled puzzle.
- **Pubkey resolution**: `--pubkey <96-hex>` (a BLS12-381 G1 public key) if given; otherwise
  `GET https://<origin>/.well-known/dig/pubkey` → `{"pubkey": "<96-hex>"}`. The canonical wire
  contract for this endpoint (path, method, response shape, failure semantics) is normative in
  the superproject `SYSTEM.md` → Shared contracts → "Well-known origin pubkey discovery"; this
  repo is a CONSUMER (discovery client) only — the endpoint's SERVING side is implemented by
  whichever origin is being authorized (e.g. a hub), not by digstore.
- **Merge semantics**: `update_store_ownership`'s delegated-puzzle set is a REPLACE, not an
  append, so the command reads the store's CURRENT delegated puzzles first and re-sends every
  existing delegate plus the new writer — an existing admin/writer/oracle delegate is never
  dropped by authorizing another writer.
- **Idempotent**: authorizing an already-authorized pubkey is a no-op (no spend, `tx_id: null`,
  `already_authorized: true` in `--json` output).
- A writer delegate may advance the store's root (publish capsules) but can never change
  ownership or the delegated set itself — only the owner key can.

## 10. Per-capsule $DIG price (commit / deploy)

Scope note: like §9, this is a CLI/economic contract, not a `.dig` byte-format contract; it is
normative for how a `digstore` reimplementation prices a capsule.

Minting a store (`init`) is **FREE of $DIG** (XCH network fee only). The $DIG payment is
attached ONLY to a `commit` / root-advance (a capsule), atomic with the singleton update in one
co-signed bundle (§ the enforcement in `digstore_chain::dig::verify_commit_pays_dig_treasury`).

The per-capsule price is **dynamic and USD-pegged**, NOT a fixed token amount:

- `dig_amount = target_usd ÷ live_dig_usd`, where `target_usd ≈ $1 / capsule / year` (realistic
  AWS hosting for one fixed-size capsule) and the amount is **uniform per capsule** (every capsule
  is the same fixed size, so size-varying pricing is FORBIDDEN — it would re-leak content size).
- **ONE canonical source.** The price is computed on the DIGHub server (the pure formula +
  USD-target constants; a live DIG→USD oracle) and served at **`GET https://hub.dig.net/v1/pricing`**
  as `mint_dig` (the capsule price in DIG **base units**; 1 DIG = 1000 base units). A `digstore`
  implementation MUST consume this SAME source so it never diverges from what DIGHub charges — it
  MUST NOT hard-code a fixed price or reimplement the formula/oracle. The response is additive-only
  (`{dig_usd, computed_at, source, mint_dig, mint_usd, subdomain_dig, subdomain_usd, cert_dig,
  cert_usd, basis}`); a reader takes `mint_dig` and ignores unknown fields.
- **Resolution order (commit/deploy):** an explicit amount — `--dig-amount <DIG>` flag >
  `DIGSTORE_DIG_AMOUNT` env > `dig.toml` `dig-amount` (a human DIG decimal, ≤ 3 dp) — always wins
  and is deterministic (no fetch). Absent any override, the CLI FETCHES the live price from the
  pricing endpoint and uses `mint_dig`. The endpoint URL is overridable via `DIGSTORE_PRICING_URL`
  (default `https://hub.dig.net/v1/pricing`).
- **Fail LOUD (money-path discipline).** If no explicit amount is set AND the pricing endpoint is
  unreachable/undecodable/omits a valid `mint_dig`, the command MUST error clearly (pointing the
  user at `--dig-amount`) and spend NOTHING — it MUST NOT silently fall back to a stale flat
  amount. (The endpoint has its own server-side fallback price, so a reachable endpoint always
  returns a usable `mint_dig`; digstore surfaces a note when `source` is `"fallback"`/`"… (stale)"`.)
- The amount displayed to the user (and in `--dry-run`'s `cost_dig`) is byte-for-byte the amount
  built into the on-chain DIG-CAT payment (`digstore_chain::cat::build_dig_store_payment`).

### 10.1 XCH coin selection & consolidation (init / commit / deploy)

Scope note: a CLI/economic contract (not a `.dig` byte-format contract) governing how the money
commands choose which XCH coins fund a spend, so a coin-fragmented wallet is never silently unable
to publish. This is digstore's expression of the ecosystem-wide **coin-management contract**
(`SYSTEM.md` → coin-management; the shared primitive is `dig-l1-wallet`). A `digstore`
reimplementation MUST replicate it and MUST NOT hand-roll its own selection heuristic.

Every XCH-funding spend built by `init` (mint fee), `commit` and `deploy` (root-advance XCH fee):

- **Selects high-value-first** — candidate XCH coins are ordered by amount DESCENDING, tie-broken by
  coin id (deterministic regardless of the order the chain returned them). The largest coins are
  taken greedily until the target (`fee` for a root advance, `fee + 1` for a mint) is met. This
  minimizes the number of inputs, keeping the bundle's CLVM cost well under Chia's per-block ceiling
  (§11.3).
- **Caps the attempt at 50 coins** (the shared `DEFAULT_COIN_CAP`; the boundary the browser/JS spend
  layer uses too). Only the largest 50 coins are eligible for a single spend.
- **Distinguishes three outcomes** — never a flat failure that hides the counts:
  1. **selectable** — the largest ≤ 50 coins cover the target; the bundle is built from exactly those.
  2. **needs consolidation** — the wallet's TOTAL XCH covers the target, but the largest 50 coins do
     not; the spend cannot be built within the cap.
  3. **insufficient funds** — the wallet's total XCH is below the target. DISTINCT from (2):
     consolidation cannot create value, so "not enough money" is never reported as "too fragmented".
- **On "needs consolidation", runs an auto-consolidate loop** (with the user's consent): build a
  CONSOLIDATION spend that merges the highest-value coins (≤ 50) into a SINGLE coin back to the
  wallet, reserving a fee; broadcast it; WAIT for the merged coin to confirm on-chain; re-scan the
  wallet; then re-attempt the original spend. Repeat until the spend is selectable, the user
  declines, or a bounded round limit. Consent is required because consolidation spends a real XCH
  fee: `--consolidate` (or the global `--yes`) proceeds without prompting; an interactive run asks
  (`[y/N]`, default No); a non-interactive run without the flag fails with a clear
  `NEEDS_CONSOLIDATION` error (exit 18) rather than spending unprompted. `--json` emits a
  `{"event":"consolidated", asset, merged_coins, merged_mojos, output_coin_id, tx_id}` record per
  round.
- **Never hand-rolls the selection or the merge** — both are the `dig-l1-wallet` primitives
  (`select_for_spend` / `select_for_consolidation`); only the bundle construction (the datalayer_driver
  / `chia-wallet-sdk` builder) stays digstore's.

The per-capsule $DIG (CAT) payment (§10) rides in the same commit/deploy bundle; its selection is
largest-first. (Capping + consolidating the $DIG-CAT side under the same contract is a follow-up.)

## 11. CHIP-0007 NFT & collection metadata (nft/collection commands)

Scope note: like §9–10, this is a CLI/off-chain-JSON contract (`nft mint`/`nft bulk`/`collection
create`/`collection mint`), not a `.dig` byte-format contract; it is normative for how a
`digstore` reimplementation reads/writes CHIP-0007 documents so third-party tooling (and the
`chip35_dl_coin` wasm) stays byte-compatible (see `SYSTEM.md` → CHIP-0007 metadata contract).

CHIP-0007 defines **two distinct attribute shapes** that MUST NOT be confused (issue #187):

- **NFT item `attributes`** (an individual NFT's traits, `Chip0007Metadata.attributes` /
  `ManifestItem.attributes`) — each entry is `{"trait_type": "<category>", "value": "<value>"}`.
  The field is `trait_type`.
- **Collection `attributes`** (the collection-level block — icon/banner/website/twitter/etc,
  `Collection.attributes` and the `collection` block embedded in each item's CHIP-0007 JSON,
  `CollectionRef.attributes`) — each entry is `{"type": "<category>", "value": "<value>"}`. The
  field is `type`, **NOT** `trait_type`.

A `digstore` implementation:

- MUST serialize collection-level attributes with the field name `type` (never `trait_type`).
- MUST serialize NFT-item attributes with the field name `trait_type` (never `type`).
- MUST, on READ, additionally accept `trait_type` as an alias for a collection attribute's `type`
  field (back-compat, §5.2/format-compat discipline: an already-emitted DIG collection.json using
  the old, non-conformant `trait_type` spelling still parses). This is a READ-only accommodation —
  it MUST NOT change what is WRITTEN.
- MUST NOT accept `type` in place of `trait_type` for an NFT item's attributes — the two shapes
  stay distinct; item attributes are conformant CHIP-0007 as originally implemented and are not
  part of this alias.

Example collection.json fragment (conformant):

```json
{
  "id": "dig-punks",
  "name": "DIG Punks",
  "attributes": [{ "type": "icon", "value": "https://dig.net/icon.png" }],
  "royalty_puzzle_hash": "…",
  "royalty_basis_points": 300
}
```

Example per-item CHIP-0007 JSON fragment (conformant — note `trait_type` for the item's own
attributes vs. `type` inside the embedded `collection` block):

```json
{
  "format": "CHIP-0007",
  "name": "DIG Punk #1",
  "collection": {
    "id": "dig-punks",
    "name": "DIG Punks",
    "attributes": [{ "type": "icon", "value": "https://dig.net/icon.png" }]
  },
  "attributes": [{ "trait_type": "Background", "value": "Blue" }]
}
```

### 11.1 DID identifiers on `--did` (bech32m + hex)

Every `--did` flag (`collection mint`, `collection show`, `nft mint --did`) accepts EITHER form:

- a 64-hex launcher id (a leading `0x` is tolerated), or
- a `did:chia:1…` bech32m address — the form Sage and CNI display DIDs in. Chia's DID bech32m
  encoding uses the literal `"did:chia:"` (colon included) as the bech32 human-readable part, so
  the FULL string (not a stripped suffix) is the bech32m payload; a `digstore` reimplementation
  decodes it the same way it would decode an `xch1…`/`nft1…` bech32m address, then checks the
  decoded prefix is exactly `"did:chia:"`.

A malformed bech32m string, or one whose decoded prefix isn't `"did:chia:"`, is rejected with a
clear argument error — never silently treated as hex.

### 11.2 Multi-item collection mint funding (coin conservation)

`collection mint` bulk-mints every item in a manifest into a collection, attributed to a creator
DID, in ONE atomic bundle authorized by a single DID spend (§11 above covers the metadata shape;
this subsection covers the on-chain funding a reimplementation must replicate).

Each item's NFT launcher is created via the standard Chia bulk-mint idiom: a 0-value
"intermediate" coin parented off the DID's current coin, whose own spend creates a 1-mojo
singleton launcher coin. Chia's coin-value conservation is **bundle-wide, not per-coin** — that
0→1 mojo creation must be balanced by an equal-or-greater deficit elsewhere in the SAME spend
bundle. The DID's own spend (`did.update`) recreates its coin at EXACTLY its current amount, so it
cannot supply more than one item's worth of that value on its own.

Consequently:

- **N = 1 item:** the DID's own coin funds the single launcher directly; no separate funding coin
  is required (this is the original, on-chain-validated single-item path).
- **N > 1 items:** a separate XCH coin MUST be spent in the same bundle, contributing at least
  `N` mojos (1 per item) via the wallet's standard puzzle. A reimplementation:
  1. selects an XCH coin (or several) covering at least `N` mojos from the minter's wallet, erroring
     with a clear, actionable "insufficient funds — need `N`, have `<balance>`" message otherwise;
  2. spends it through the standard layer, returning any amount over `N` as CHANGE to the funding
     coin's own address — the excess MUST NOT be left to silently become network fee;
  3. includes that spend in the SAME `SpendBundle` as the DID spend and every item's
     launcher/mint spends, signed together.

The collection definition, per-item metadata, and royalty/attribution semantics are unaffected —
only the coin-level funding differs between N=1 and N>1.

### 11.3 Large-collection auto-batching, resumability, and the oversized-bundle guard

A single spend bundle for N items grows with N; once its total CLVM cost exceeds Chia's per-block
cost ceiling the full node rejects the `push_tx`, so an arbitrarily large `collection mint` MUST be
split into cost-bounded batches. A reimplementation:

- **Cost-bounds each batch, computed — never a hard-coded count.** The per-block CLVM cost ceiling
  is `MAX_BLOCK_COST_CLVM = 11_000_000_000` (Chia mainnet `ConsensusConstants::max_block_cost_clvm`).
  A batch's estimated cost MUST stay at or under a conservative fraction of that ceiling
  (digstore uses `1/4`) so that estimate error, block contention, and gateway request-size limits are
  all absorbed. The estimate is `base + per_item * n` where the per-item constant is proven
  conservative against the real Chia consensus cost model (`run_spendbundle` under
  `MAINNET_CONSTANTS`): the measured marginal per-item cost MUST NOT exceed the constant. The default
  batch size is the largest `n` whose estimate fits the budget (≥ 1). An explicit `--batch-size`
  override is honoured but MUST be validated against the same budget; a too-large size is a **terminal**
  error naming the maximum allowed size, never a retryable one.
- **One batch = one self-contained bundle**, built, funded (§11.2 N-launcher funding — a separate XCH
  coin contributing 1 mojo per item in the batch, change returned), signed, broadcast, and confirmed
  before the next batch. Every batch is attributed to the SAME creator DID: each batch spends the DID
  exactly once (advancing it one generation), and the next batch spends the DID coin the prior batch
  recreated. The DID's acknowledgement of every item's attribution is preserved on every batch.
- **Resumable (mainnet money).** Because each batch spends real XCH, a failure after batch K MUST NOT
  re-mint or double-spend batches `0..=K` on re-run. Per-batch progress (item range, the DID coin
  spent, the recreated DID coin as the deterministic confirmation target, the tx id, and the minted
  launcher ids, each flagged confirmed once its landing is verified on chain) is persisted, keyed by a
  stable fingerprint of the manifest bytes so a resume applies ONLY to the same collection + DID +
  manifest + batch size. A re-run skips confirmed batches and reconciles a pushed-but-unconfirmed tail
  against chain — if the tail's recreated DID coin already exists, the batch landed and is marked
  confirmed. Correctness rests on the DID being single-use per generation: at most one mint can confirm
  per DID generation, so a rebuilt batch can never double-mint.
- **The oversized-bundle rejection is terminal, not transient.** A bundle whose serialized
  generator-byte cost alone (`bytes * cost_per_byte`, `cost_per_byte = 12_000`) meets or exceeds
  `MAX_BLOCK_COST_CLVM` is definitively too large; broadcast MUST refuse it up-front with an actionable
  "transaction SIZE limit — split into smaller batches" error, and MUST NOT retry it or misreport it as
  a coinset.org connectivity/`error decoding response body` problem (the transient-retry path is for
  genuine transport hiccups only).
