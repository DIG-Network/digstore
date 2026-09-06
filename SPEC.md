# dig-store — `.dig` store format & manifest specification

This is the NORMATIVE contract for the dig-store `.dig` store format: the byte-exact
data-section blob, the capsule/generation model, per-resource crypto and merkle
commitment, and the normalized public manifest. An independent implementation that
reads or writes `.dig` modules MUST conform to this document. The single source of
truth in code is `digstore-core` (`datasection`, `merkle`, `crypto`, `urn`,
`public_manifest`); this spec MUST agree with it and with the ecosystem contracts in
the superproject `SYSTEM.md` and the user-facing protocol pages under `docs.dig.net`.

**CLI binaries.** The `digstore-cli` crate ships TWO binaries, `dig-store` and `digs`.
`digs` is a first-class alias: `digs <args>` behaves IDENTICALLY to `dig-store <args>`
(same subcommands, flags, `--json`, and exit codes). Both share ONE codepath
(`digstore_cli::run()`) and each reflects its own invoked name (arg0) in
`--help`/`--version`/`completion`/`--help-json`. Both binaries MUST be shipped together
everywhere `dig-store` ships (cargo-install, the universal installer, the apt `.deb`).

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

**JSON surface.** The CLI `dig-store manifest --json`, the JSON-RPC `dig.getManifest`, and the
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
- **CLI** — `dig-store manifest [--json]` prints the normalized manifest.

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
`dig-store` reimplementation must also expose.

- `dig-store authorize-origin-as-writer <origin> [--pubkey <hex>] [--dry-run] [--fee <mojos>]`
  authorizes an origin's DIG identity as a CHIP-0035 **writer** delegate on the active store's
  on-chain singleton, using the existing `digstore_chain::singleton` delegation primitive
  (`writer_delegated_puzzle` + `update_store_ownership`) — never a hand-rolled puzzle.
- **Pubkey resolution**: `--pubkey <96-hex>` (a BLS12-381 G1 public key) if given; otherwise
  `GET https://<origin>/.well-known/dig/pubkey` → `{"pubkey": "<96-hex>"}`. The canonical wire
  contract for this endpoint (path, method, response shape, failure semantics) is normative in
  the superproject `SYSTEM.md` → Shared contracts → "Well-known origin pubkey discovery"; this
  repo is a CONSUMER (discovery client) only — the endpoint's SERVING side is implemented by
  whichever origin is being authorized (e.g. a hub), not by dig-store.
- **Merge semantics**: `update_store_ownership`'s delegated-puzzle set is a REPLACE, not an
  append, so the command reads the store's CURRENT delegated puzzles first and re-sends every
  existing delegate plus the new writer — an existing admin/writer/oracle delegate is never
  dropped by authorizing another writer.
- **Idempotent**: authorizing an already-authorized pubkey is a no-op (no spend, `tx_id: null`,
  `already_authorized: true` in `--json` output).
- A writer delegate may advance the store's root (publish capsules) but can never change
  ownership or the delegated set itself — only the owner key can.

### 9.1 Pinned-root chain-anchored verification (loopback read tier)

Scope note: like §9, this is a chain-authority contract exposed by `digstore_chain::singleton`,
not a `.dig` byte-format contract. It is normative for how a node serving a root-pinned URN
(`dig://<store_id>:<pinned_root>`) over the §5.3 loopback tier chain-anchors the request.

- `verify_pinned_root(chain, store_id, pinned_root) -> Result<()>` returns `Ok(())` **iff**
  `pinned_root` equals the store's CURRENT on-chain generation root, and `Err` otherwise. It is
  **fail-closed**: a chain-read failure, an absent unspent singleton, or any root mismatch is an
  `Err` — never a false `Ok`. A caller MUST treat any `Err` as "do not serve".
- It is a **bounded, launcher-anchored** verification. Identity is anchored on the UNFORGEABLE
  launcher coin (`coin_id == store_id`, a 256-bit hash preimage an attacker cannot grind), NEVER
  on the curried `SingletonStruct.launcher_id` — that value is attacker-controllable, so a coin
  merely discovered by hint whose curried `launcher_id == store_id` is NOT proof of identity. The
  current unspent singleton is discovered with `unspent_coins_by_hint(store_id)`, then each
  candidate tip is verified by a BOUNDED backward walk of coin records (following
  `coin.parent_coin_info`, capped at 100_000 hops) that MUST reach the launcher coin whose
  `coin_id == store_id` — which itself MUST exist, be spent, and have
  `puzzle_hash == SINGLETON_LAUNCHER_HASH`. The tip's root is read from the ONE spend that created
  it (the tip's parent). Intermediate generations' SPENDS are never required — only their coin
  records — so this still sidesteps the full forward walk (`sync_datastore`), which aborts if any
  single intermediate generation's spend is unparseable (#747). As defense-in-depth, each hop
  whose spend IS available is parsed for ONLY its `SingletonLayer` and its curried
  `launcher_id == store_id` asserted (best-effort — a missing/unparseable intermediate spend does
  not fail the verification; the coin-record parent-chain to the launcher is the real proof).
- **Anti-rollback**: only the current on-chain root passes; a root that was never on-chain and a
  stale (superseded) generation are both rejected. Verifying a historical-but-real generation is
  intentionally out of scope (it would require the per-generation enumeration this API avoids).
  The trust root is the launcher coin (`coin_id == store_id`); a singleton discovered by hint that
  does not chain back to that coin is rejected regardless of its curried `launcher_id` (the
  pre-#1473 forgeability class).

### 9.2 Singleton terminal-state classification (melt detection)

Scope note: like §9, this is a chain-authority contract exposed by `digstore_chain::singleton`,
not a `.dig` byte-format contract. It is normative for how a consumer distinguishes the terminal
states of a store's on-chain singleton lineage — in particular a genuine owner **melt** (which
downstream may authorize an IRREVERSIBLE local-data delete) from a store that was never minted and
from a corrupt/unreachable chain.

`classify_singleton(chain, launcher_id) -> Result<SingletonTerminal>` walks the singleton lineage
(launcher → eve → … → tip) exactly once and returns one of THREE well-formed terminal states, or
`Err`:

- **`Live(DataStore)`** — the launcher is spent and the forward walk reached an UNSPENT tip. The
  returned `DataStore` is the live, spendable singleton.
- **`NeverMinted`** — the launcher coin exists on-chain but is still UNSPENT (no eve singleton was
  ever created). This is NOT a melt.
- **`Melted { last_root, melt_spent_height }`** — the launcher is spent and the walk reached a
  singleton coin that was SPENT to a spend consuming the singleton with no singleton child (an
  owner-authorized melt). `last_root` is the metadata root of the generation that was melt-spent;
  `melt_spent_height` is the block height at which the melt spend was confirmed.

Normative invariants (custody-critical):

- **Corrupt is NEVER melt.** A `Melted` is returned ONLY when the terminal spend parses as a valid
  datastore singleton spend whose inner conditions carry no odd (singleton) create-coin — the exact
  on-chain signature of a melt, which `DataStore::from_spend` surfaces as
  `Err(DriverError::MissingChild)`. EVERY other outcome — a missing/unreadable coin record, a
  missing/unparseable spend, a spend that is not a datastore singleton at all
  (`from_spend -> Ok(None)`), or any other parse/CLVM error — is `Err`, NEVER `Melted`. A consumer
  MUST treat any `Err` as "unknown / do not delete".
- **Burial-depth anchor.** A consumer that acts irreversibly on `Melted` (e.g. deleting locally
  cached content) MUST enforce a burial depth against `melt_spent_height` before doing so: a reorg
  that un-melts the store AFTER a delete is permanent data loss. `melt_spent_height` is provided for
  exactly this check.
- **Single walker.** `classify_singleton` and `sync_datastore` share ONE lineage traversal;
  `sync_datastore` maps the non-`Live` terminals back to its legacy `ChainError::Chain` strings, so
  its signature and behaviour are unchanged. Reimplementing this walk elsewhere (e.g. in dig-node)
  is forbidden — a second canonical walker on a custody path is a byte-drift bug.

## 10. Per-capsule $DIG price (commit / deploy)

Scope note: like §9, this is a CLI/economic contract, not a `.dig` byte-format contract; it is
normative for how a `dig-store` reimplementation prices a capsule.

Minting a store (`init`) is **FREE of $DIG** (XCH network fee only). The $DIG payment is
attached ONLY to a `commit` / root-advance (a capsule), atomic with the singleton update in one
co-signed bundle (§ the enforcement in `digstore_chain::dig::verify_commit_pays_dig_treasury`).

The per-capsule price is **dynamic and USD-pegged**, NOT a fixed token amount:

- `dig_amount = target_usd ÷ live_dig_usd`, where `target_usd ≈ $1 / capsule / year` (realistic
  AWS hosting for one fixed-size capsule) and the amount is **uniform per capsule** (every capsule
  is the same fixed size, so size-varying pricing is FORBIDDEN — it would re-leak content size).
- **ONE canonical source.** The price is computed on the DIGHub server (the pure formula +
  USD-target constants; a live DIG→USD oracle) and served at **`GET https://hub.dig.net/v1/pricing`**
  as `mint_dig` (the capsule price in DIG **base units**; 1 DIG = 1000 base units). A `dig-store`
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
  returns a usable `mint_dig`; dig-store surfaces a note when `source` is `"fallback"`/`"… (stale)"`.)
- The amount displayed to the user (and in `--dry-run`'s `cost_dig`) is byte-for-byte the amount
  built into the on-chain DIG-CAT payment (`digstore_chain::cat::build_dig_store_payment`).

### 10.1 XCH coin selection & consolidation (init / commit / deploy)

Scope note: a CLI/economic contract (not a `.dig` byte-format contract) governing how the money
commands choose which XCH coins fund a spend, so a coin-fragmented wallet is never silently unable
to publish. This is dig-store's expression of the ecosystem-wide **coin-management contract**
(`SYSTEM.md` → coin-management; the shared primitive is `dig-wallet-backend`'s engine seam `engine::selection`). A `dig-store`
reimplementation MUST replicate it and MUST NOT hand-roll its own selection heuristic.

Every XCH-funding spend built by `init` (mint fee), `commit` and `deploy` (root-advance XCH fee):

- **Selects high-value-first** — candidate XCH coins are ordered by amount DESCENDING, tie-broken by
  coin id (deterministic regardless of the order the chain returned them). The largest coins are
  taken greedily until the target (`fee` for a root advance, `fee + 1` for a mint) is met. This
  minimizes the number of inputs, keeping the bundle's CLVM cost well under Chia's per-block ceiling
  (§11.3).
- **Caps the attempt at 50 coins** — digstore uses a LOCAL `COIN_CAP = 50`, distinct from
  dig-wallet-backend's `DEFAULT_COIN_CAP = 500`, because digstore's spend bundles must stay under
  Chia's mempool cost ceiling. Only the largest 50 coins are eligible for a single dig-store
  XCH-funding spend.
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
- **Never hand-rolls the selection or the merge** — both are the `dig-wallet-backend` primitives
  (`select_for_spend` / `select_for_consolidation`); only the bundle construction (the datalayer_driver
  / `chia-wallet-sdk` builder) stays dig-store's.

The per-capsule $DIG (CAT) payment (§10) rides in the same commit/deploy bundle; its selection is
largest-first. (Capping + consolidating the $DIG-CAT side under the same contract is a follow-up.)

## 11. CHIP-0007 NFT & collection metadata (nft/collection commands)

Scope note: like §9–10, this is a CLI/off-chain-JSON contract (`nft mint`/`nft bulk`/`collection
create`/`collection mint`), not a `.dig` byte-format contract; it is normative for how a
`dig-store` reimplementation reads/writes CHIP-0007 documents so third-party tooling (and the
`chip35_dl_coin` wasm) stays byte-compatible (see `SYSTEM.md` → CHIP-0007 metadata contract).

CHIP-0007 defines **two distinct attribute shapes** that MUST NOT be confused (issue #187):

- **NFT item `attributes`** (an individual NFT's traits, `Chip0007Metadata.attributes` /
  `ManifestItem.attributes`) — each entry is `{"trait_type": "<category>", "value": "<value>"}`.
  The field is `trait_type`.
- **Collection `attributes`** (the collection-level block — icon/banner/website/twitter/etc,
  `Collection.attributes` and the `collection` block embedded in each item's CHIP-0007 JSON,
  `CollectionRef.attributes`) — each entry is `{"type": "<category>", "value": "<value>"}`. The
  field is `type`, **NOT** `trait_type`.

A `dig-store` implementation:

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
  the FULL string (not a stripped suffix) is the bech32m payload; a `dig-store` reimplementation
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
  (dig-store uses `1/4`) so that estimate error, block contention, and gateway request-size limits are
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

### 11.4 On-chain NFT media URIs — canonical URN + https backup (NFT1 multi-url)

`nft mint` writes the art + generated CHIP-0007 metadata into a real capsule and sets the minted
NFT's on-chain NFT1 `data_uris` / `metadata_uris` to TWO entries each, in this fixed order:

1. the canonical **bare root-pinned URN** `urn:dig:chia:<storeId>:<rootHash>/<resourceKey>` (the
   data resource key is `art`; the metadata resource key is `metadata.json`) — the PRIMARY entry;
2. an **https gateway url** `<gateway>/urn:dig:chia:<storeId>:<rootHash>/<resourceKey>` — the
   FALLBACK (`<gateway>` defaults to `https://rpc.dig.net`; `--gateway <base>` overrides the host).

NFT1 `uris`/`meta_uris` are lists that accept multiple backup urls, so both are emitted: a DIG-aware
wallet resolves the URN natively (dig-node / rpc.dig.net) while a legacy wallet (Sage) uses the https
url. The URN is root-PINNED because NFT media is immutable content — it names the exact capsule
generation the on-chain `data_hash`/`metadata_hash` are pinned to. A conforming reimplementation MUST
emit the canonical bare `urn:dig:chia:…` form (the single resource-identifier grammar, §URN) —
**never** a `dig://`-prefixed URN — and MUST keep the URN first. The list is additive: an old reader
simply reads whichever entry it understands.

## 11a. Post-commit seed push to the local dig-node (content-replication flywheel)

Scope note: a CLI/networking contract (not a `.dig` byte-format contract) governing how a successful
`commit`/`deploy` seeds the publisher's OWN local node, so freshly-published content is discoverable
+ reshareable immediately instead of only after some other node is first asked for it. This is the
publisher-side (seed) leg of the ecosystem content-replication flywheel (`SYSTEM.md` → dig-node
`cache.pushCapsule`; dig_ecosystem#1476).

**When.** After a `commit` CONFIRMS on-chain and the generation is finalized (the `.dig` is on disk),
the CLI pushes the produced capsule to the operator's local dig-node. It fires on EVERY successful
commit, INDEPENDENT of `--push`/DIGHub. `deploy` (which drives `commit`) seeds identically.

**Best-effort + STRICTLY NON-FATAL.** The commit has already succeeded before the seed runs, so EVERY
failure mode — no local node running, the control token unavailable, any transport/RPC error — prints
a single YELLOW warning (`local dig-node not running: committed locally, not yet cached/reshared`) and
returns SUCCESS. The seed push MUST NEVER fail a commit and MUST NEVER block on a slow/absent node
(short-timeout probe).

**Local tiers ONLY.** The target node is resolved via the §5.3 ladder RESTRICTED to the local tiers —
`dig.local` then `localhost` (default port 9778, `DIG_NODE_PORT`-overridable), over plain HTTP (the
node's loopback JSON-RPC surface). `rpc.dig.net` is NEVER seeded (seeding the public gateway is not the
local-cache flywheel), and an explicit `--node` override is deliberately ignored for this path.

**Wire — `cache.pushCapsule` (locked by dig-node `SPEC.md` §5.5.3).** The `.dig` is pushed in ≤3 MiB
base64 windows the node reassembles: params `{ store_id (64-hex), root (64-hex), data (base64,
standard), offset (u64, default 0), total_length (u64) }`; the node acks `{ offset, complete (bool),
next_offset (u64|null), size_bytes }` (+ `served_root` and `already_cached: true` on completion). The
client sends `offset = 0` first and follows `next_offset` until `complete == true`.

**Auth — the local control token.** `cache.pushCapsule` makes the node a durable holder, so over
loopback HTTP it carries the node's master control token in the `X-Dig-Control-Token` header — the SAME
gate as `cache.fetchAndCache` (dig-node `SPEC.md` §5.5.3/§7), NOT a §21.9 signature (that is only for
the opt-in opened peer surface). A same-machine caller obtains the token from `DIG_NODE_CONTROL_TOKEN`
(the headless/CI escape hatch), else by reading the node's on-disk `control-token` file from the state
dir dig-node resolves (`DIG_NODE_STATE_DIR` override → the per-OS machine-wide state dir → the legacy
per-user dir). When the token cannot be obtained (a service-run node under another OS account) the push
proceeds without the header, the node answers `Unauthorized`, and that surfaces as the same non-fatal
YELLOW warning — never a bypass.

**Config (default-ON).** Auto-push is ON by default. Precedence (uniform `flag > env > dig.toml >
default`): `--no-cache` (alias `--no-seed`) opts out > `DIGSTORE_AUTOPUSH` (`true`/`1`/… vs
`false`/`0`/…) > `dig.toml` `auto-push` (bool) > default-ON. `commit --json` folds the outcome into its
object (`seeded`, and `already_cached` / `seed_warning` / `seed_skipped`).

## 12. Release pipeline — nightly cron + manual dispatch

How the `dig-store` CLI binary + its `digs` alias are built and released. The shape is copied from
the ecosystem's reference nightlies implementation (`dig-updater`); the ops runbook is
`runbooks/release.md`.

Releases are **batched to a nightly cron plus manual dispatch** — NOT cut on every merge to `main`.
Two channels ship from one orchestrator (`.github/workflows/nightly-release.yml`):

### 12.1 Trigger

The orchestrator triggers ONLY on:

- `schedule: cron '0 0 * * *'` — **midnight UTC** (GitHub Actions cron is always UTC; a top-of-hour
  cron MAY be delayed under load — acceptable, since the nightly channel is idempotent), and
- `workflow_dispatch` with two inputs: `channel` (`both` | `stable` | `nightly`, default `both`) and
  `force` (boolean, default `false`).

It MUST NOT trigger on `push` to `main`. **A schedule run exercises ONLY the nightly channel — the
stable channel MUST be reachable ONLY from a manual `workflow_dispatch` selecting `channel: stable`
or `channel: both`.** Cutting a stable release is a deliberate act; the cron MUST NEVER perform one
unattended (dig_ecosystem#698 / digs#63).

**60-day auto-disable caveat.** GitHub auto-disables a `schedule:` trigger after 60 days with no
repo activity on a public repo, with no auto-re-enable — and since this cron is the ONLY automatic
trigger for the **nightly** channel (the stable channel is never automatic, disabled or not), a
quiet repo can silently stop shipping nightlies with no error. Detect it with
`gh api repos/DIG-Network/digs/actions/workflows/nightly-release.yml --jq .state` (a value of
`disabled_inactivity` means it was auto-disabled) and recover with `gh workflow enable
nightly-release.yml` (see `runbooks/release.md`). Any repo activity resets the 60-day counter.

### 12.2 Stable channel

Cuts a semver `vX.Y.Z` **stable** release ONLY on a manual `workflow_dispatch` (never the
`schedule` trigger — §12.1) selecting `channel: stable` or `channel: both`, and only when the
`[workspace.package].version` in the root `Cargo.toml` has advanced beyond the newest `vX.Y.Z` tag
(the skip-if-already-tagged check IS the version-changed check). Cutting a release means: `git-cliff`
regenerates `CHANGELOG.md`, commits it to `main` as `chore(release): vX.Y.Z`, tags THAT commit (so
the changelog is inside the tag), and pushes commit + tag with `RELEASE_TOKEN`. The pushed `v*` tag
fires `release.yml`, which builds every OS/arch (both asset shapes) and publishes the GitHub Release.
It ALSO uploads the Linux x86_64 binary to the dighub S3 artifact bucket for the hub compile-worker
(tag-only — a nightly never moves the `latest` binary the worker reads).

`force: true` on a manual dispatch bypasses the skip-if-tagged guard and re-cuts the current version
(moving the tag onto a fresh changelog commit — `main` is never force-pushed).

**Force is guarded against mutating a published release (supply-chain invariant).** A force re-cut
MUST be refused — non-zero exit, clear error — when BOTH: (a) a PUBLISHED (non-draft) GitHub Release
already exists at the version's `vX.Y.Z` tag, AND (b) that tag currently points at a commit
DIFFERENT from the commit this run would build. Force MAY proceed when either is false: a
same-commit re-cut (a failed-build retry) or a tag with no published release (a tag repair). A
version that needs new code released MUST bump `Cargo.toml`, not force-move a tag.

### 12.3 Nightly channel

Every night (and on demand) builds `main` HEAD for every OS/arch and publishes a GitHub
**pre-release** — so a fresh nightly always exists regardless of a version bump. It:

- **Synthesizes the version at build time** (nothing is committed): `X.Y.Z-nightly.YYYYMMDD.<shortsha>`.
  As a semver prerelease it sorts BELOW the plain `X.Y.Z`.
- Publishes under a **dated tag `nightly-YYYYMMDD`** AND force-moves a **rolling `nightly` tag**,
  with `prerelease: true` and **never** `latest`. Idempotent: a same-day re-run refreshes today's
  dated release + the rolling pointer.
- **Retention:** keeps the newest **14** dated nightlies plus the rolling `nightly`, pruning older
  dated pre-releases AND their tags together (`gh release delete --cleanup-tag`). `v*` stable
  tags/releases and the rolling `nightly` are NEVER pruned. (The nightly channel does NOT run the S3
  publish — that stays stable-only.)

### 12.4 Reusable build

The cross-OS build lives once in `.github/workflows/build-binaries.yml` (`on: workflow_call`, inputs
`version` + `ref`). Both `release.yml` (stable) and the nightly channel call it, so the two paths
can never diverge. It builds `dig-store` + the `digs` alias for `windows-x64`, `linux-x64`,
`linux-arm64` (native `ubuntu-24.04-arm` runner), `macos-arm64`, and `macos-x64`, in the two asset
shapes (bare per-OS binaries + apt `.tar.gz`). Every Linux arch publishes BOTH shapes: the bare
`linux-arm64` binaries are what `dig-updater`'s feed resolves for `(linux, arm64)`, so omitting them
leaves an arm64 host with a tarball apt can package and no update path at all.

The arm64 leg carries a mandatory `verify-linux-arm64` job (no `continue-on-error`, no skippable
`if:`) that every caller's publish waits on. It asserts the EXACT staged asset set before inspecting
any file — `if-no-files-found: error` is satisfied by a short `dist/`, so a count check must come
first — then reads `ARM aarch64` out of each ELF header, then executes each binary in a bare
`ubuntu:24.04` arm64 container with no toolchain. The architecture and execution checks are separate
because the runner has binfmt/qemu registered: an x86-64 binary runs perfectly well under an arm64
filename, so execution alone proves liveness, not architecture.

BUILD PREREQ (§3.5 / BINDING contract D6): the
`digstore-guest` wasm is built for `wasm32-unknown-unknown` BEFORE the CLI on every leg, because
`digstore-cli`'s `build.rs` embeds it.

TRANSITIONAL DUAL-PUBLISH (rename epic #703): the primary binary was renamed `digstore` ->
`dig-store` (the Cargo package name `digstore-cli` and all library crate names are UNCHANGED). For
one transition cycle every asset is published under BOTH the new `dig-store-<ver>-<os_arch>` stem
AND the legacy `digstore-<ver>-<os_arch>` stem (bare binaries + apt `.tar.gz`), and each apt tarball
ships a `digstore` -> `dig-store` compat symlink at its root, so apt.dig.net + dig-installer stay
green until they cut over. The `digs` alias asset name is derived independently (there is no
`dig-store` -> `digs` substring). The legacy stem + symlink drop in a later release once both
installers have cut over.

### 12.5 RELEASE_TOKEN posture

Releasing uses the `RELEASE_TOKEN` org PAT, not `GITHUB_TOKEN`. If `RELEASE_TOKEN` is absent, EVERY
channel NO-OPS with a clear `::warning::` — never a half-release. A `concurrency: nightly-release`
group (cancel-in-progress `false`) serializes runs so an overlapping cron + dispatch cannot race.

## 13. Serving-module execution bounds

A compiled serving module is UNTRUSTED code. Every host that executes one
(`digstore_host::HostRuntime`) MUST bound it with all four of the following. The fourth —
pinning the accepted language — is what makes the other three complete, so it is normative,
not hardening.

### 13.1 The accepted language is pinned explicitly

The host MUST enumerate the wasm proposals it accepts rather than inheriting the engine's
default set. Engine defaults are NOT stable across major versions, so inheriting them means an
engine upgrade silently widens what an untrusted module may do.

The **threads**, **GC**, **exceptions**, and **function-references** proposals MUST be
disabled. The first two are the load-bearing ones: each hands a guest a growable resource that
the store's resource limiter does not account for — shared memory allocated outside the
limiter, and, for GC, an entire second heap that the limiter's memory count does not see (it
counts only the module's *defined linear memories*). With GC enabled, a module receives its
full memory allowance a second time, doubling the host footprint it can reach.

### 13.2 Resource ceilings

An outer ceiling enforced by the store's resource limiter, covering every growable resource the
accepted language can express — linear memory (default 384 MiB, matching the guest's declared
maximum), tables, table elements, memories, and instances — not linear memory alone. This
enumeration is exhaustive ONLY in combination with §13.1: a proposal that introduces a resource
outside the limiter's accounting MUST be disabled there rather than left to the ceiling.

### 13.3 Time and fuel

- A **wall-clock budget** enforced by engine epoch interruption (default 5 s per call).
- A **fuel budget** per unit of guest execution (default 5 000 000 000).

### 13.4 Instantiation counts as guest execution

A module's wasm `start` function runs while the module is being instantiated, before any export
is called, so the fuel budget and the epoch deadline MUST be armed on the store BEFORE
instantiation, not only around export calls. A host that arms them afterwards either leaves the
start function outside the sandbox entirely or — where the engine enables fuel consumption
globally, leaving an un-armed store at zero fuel — rejects every legitimate module that has a
start function.

A bound-induced instantiation failure MUST surface with the same error taxonomy as a
bound-induced export failure (timeout vs fuel exhaustion), not as an opaque engine error.

### 13.5 Budgets are per call

Each export call is armed with its own fresh budget; a serve sequence (alloc → call → read →
dealloc) is deliberately NOT a single combined budget.

### 13.6 Host identity: never substituted, required to attest, sign, or push, and not consulted to read

A store's host identity is its BLS signing key (`signing_key.bin`) and the trusted host keys
(`trusted_keys.json`) persisted at init.

- A host MUST NOT substitute a default, fixed, hardcoded, or all-zero value for a missing signing
  key or public key. A fixed seed is reproducible by anyone with the source, so a host holding one
  carries no identity at all rather than a weaker one, and an all-zero public key is a nonexistent
  identity rather than a weak one.
- A host MUST refuse to attest, sign, or push when the identity is absent or unreadable.
- Serving committed content consumes NO host identity. A host MUST carry none on that path, MUST NOT
  read the identity files to take it, and MUST NOT refuse a read because the identity is absent,
  unreadable, or malformed. Reading DIG content requires no account and no key (§14), so a missing
  identity is not a reason to withhold content that is already committed and merkle-verifiable
  against its trusted root.
- Where a host does carry no identity, that absence MUST be representable as absence rather than
  encoded as a placeholder value, so that a path which later begins consuming the identity fails
  closed instead of accepting a key nobody controls.
- Making the identity optional MUST NOT make any gate optional. Where the content gate does require
  attestation (§12.2), a host holding no identity MUST fail that gate closed and return a decoy,
  exactly as a host presenting an untrusted or invalid key does.
- The signing key MUST be exactly 32 bytes. A shorter or longer file is malformed and MUST be
  reported as a corrupt-identity error — never truncated, never padded, and never handed to key
  derivation, which is permitted to abort the process on a short seed.
- Wherever a code path loads the identity, an unreadable or malformed identity MUST surface as an
  error naming the offending file, rather than as a downstream symptom several layers away.

The substitution ban holds regardless of whether a given path currently verifies the identity it
loads. A path that does not consume the identity today MUST NOT be treated as licence to supply a
placeholder, because the placeholder becomes forgeable identity the moment any consumer begins
verifying it. Where a path genuinely does not need an identity, the correct expression is to carry
no identity at all — not to carry a fabricated one, and not to refuse the operation.

These two rules are one rule seen from both sides, and neither implies the other. Refusing a read
over a missing identity is not a stricter form of not substituting one: it withholds content whose
integrity does not depend on the host at all, while leaving every path that DOES consume an identity
exactly as safe as it was. Conversely, tolerating a missing identity on the read path grants nothing
to the signing paths, which continue to require one.

## 14. Client → node resolution (the origin)

This section is normative for every command that must reach a DIG node: which endpoint is
chosen, how a project pins its own, and when a missing local node is an error rather than a
fall-through. It implements `CLAUDE.md` §5.3.

### 14.1 Precedence

The endpoint is decided by the FIRST of these that is present; a configured value overrides the
probe ladder entirely and is never probed:

1. `--node <url>`
2. `$DIG_NODE_URL` (an empty value counts as unset)
3. the PROJECT `node.url` — `<workspace>/node.toml`, i.e. `.dig/node.toml`, found by the same
   nearest-ancestor `.dig` walk that locates the workspace — **only when approved** (§14.3)
4. the MACHINE `node.url` — `<DIG_IDENTITY_DIR | OS config dir>/dig/config.toml`
5. otherwise, the probe ladder (§14.2)

A trailing `/` is stripped from every configured value.

### 14.2 The probe ladder

With no configured value, these candidates are probed IN ORDER with `GET {base}/health` and a
short timeout; the FIRST to answer 2xx wins. A non-2xx, a transport error, or an elapsed
timeout falls through to the next candidate; a timeout MUST NOT abort the remaining candidates.

| # | Candidate | Tier | dig-node listener |
|---|---|---|---|
| 1 | `https://dig.local` | `DigLocal` | `127.0.0.2:443` (present only with a dig-cert leaf) |
| 2 | `http://dig.local` | `DigLocal` | `127.0.0.2:80` |
| 3 | `http://localhost:<port>` | `Localhost` | `127.0.0.1:<port>` and `[::1]:<port>` |
| — | `https://rpc.dig.net` | `PublicGateway` | terminal fallback, returned UNPROBED |

`<port>` is `$DIG_NODE_PORT` when it parses to a non-zero `u16`, else `9778`. The port applies
ONLY to candidate 3: the `dig.local` binds are fixed at 443/80 by the `127.0.0.2` hosts alias.
The candidate URLs MUST match the addresses `dig-node` actually binds (`dig-node/SPEC.md`
§4.1, §4.1a) — in particular the loopback listener is PLAINTEXT, never TLS.

Resolution MUST NOT fail: the public gateway is always a valid last resort. The resolved choice
is cached for the invocation.

`https://rpc.dig.net` is an ordinary node that happens to be well known. It MUST NOT be the
primary or hard-coded endpoint of any surface.

### 14.3 A project-declared node is untrusted until approved (HARD RULE)

`.dig/node.toml` can travel inside a repository, and every request this CLI sends to the
resolved node carries the caller's §21.9 identity SIGNATURE. A project-declared value is
therefore an untrusted input that PROPOSES an endpoint; it MUST NOT route any request until the
user has approved it on this machine.

Approval is recorded in `<global dig dir>/trusted-project-nodes.toml`, keyed by the
canonicalized project directory and holding the exact approved URL. Both halves are required:

- Approval MUST be scoped to the project that was approved — it MUST NOT authorize the same URL
  in another directory.
- Approval MUST be scoped to the value that was approved — if the project later declares a
  DIFFERENT URL, approval is re-armed and the new value MUST NOT be used until re-approved.

`digstore config node.url --local <url>` writes the value AND records approval, because typing
the URL is itself the approval. An unapproved value MAY be approved by an interactive
confirmation. When the CLI cannot prompt (non-interactive, `--quiet`, `--json`, no TTY) the
answer is always NO: it MUST warn, ignore the value, fall back to the ladder, and MUST NOT
record approval.

### 14.4 The `origin` remote

An `origin` that has been configured with `digstore remote add` resolves to that URL.

An UNCONFIGURED `origin` resolves through §14.1/§14.2 — the user's own node. It MUST NOT
default to `https://rpc.dig.net`. Any other unconfigured remote name is an error.

### 14.5 Missing local node — read vs. write

When resolution reaches the `PublicGateway` tier (nothing local answered AND no value was
configured), the behaviour depends on what the operation needs:

- **Read** (`pull`, `clone`, `cat`, `doctor`) — proceeds against the gateway, so consuming
  content works without a node installed, and MUST tell the user the read left this machine.
- **Local node required** (`push`, `revoke`, and any other identity-signed write) — MUST fail
  with `NO_LOCAL_NODE` (exit 19) rather than degrade. Falling through would publish the user's
  content and their §21.9 request signatures to a server they never chose.

The error MUST state how to check the node (`dig-node status`), where to install it
(`https://dig.net/install.sh`, `https://dig.net/install.ps1`,
`https://docs.dig.net/docs/run-a-node`), and the escape hatch
(`digstore config node.url --local <url>`).

Any tier OTHER than `PublicGateway` — including `Override` — satisfies both requirements: an
explicitly named endpoint is the user's own choice even when it is `rpc.dig.net`.
