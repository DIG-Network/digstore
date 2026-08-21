//! Collection primitive + per-item CHIP-0007 metadata generation (roadmap #34/#33, digstore side).
//!
//! Creators think in *collections*, not individual mints. This models a CHIP-0007 collection
//! (id/name/attributes/shared royalty), generates per-item CHIP-0007 metadata from a *parsed*
//! traits manifest, and converts the on-chain media fields into the serialized CLVM [`Program`] the
//! NFT mint builders ([`crate::nft`]) take. Pure data — no chain, no keys, no file IO.
//!
//! ## Relationship to `chip35_dl_coin`
//! This is the digstore-side mirror of `chip35_dl_coin`'s `core/src/collection.rs` off-chain half:
//! the [`Collection`]/[`ManifestItem`]/[`ManifestMedia`] shapes and [`generate_item_metadata`]
//! semantics match so the per-item CHIP-0007 JSON (and its `metadata_hash`) is byte-identical to the
//! wasm path. The on-chain bulk-mint spend itself is built by [`crate::nft::build_bulk_mint`] (the
//! digstore-chain builder, Simulator-tested), which this module feeds via [`item_to_metadata_program`].
//!
//! ## What is SCAFFOLDED (clear TODO, not faked)
//! - **Traits-manifest ingest at scale** (CSV/large-JSON parsing, generative trait composition,
//!   rarity, per-item capsule packing) is a TOOLKIT concern and is NOT implemented here. This module
//!   consumes an ALREADY-PARSED `&[ManifestItem]` only. See [`generate_item_metadata`].
//! - **Drop mechanics** (delayed reveal, allowlist/claim gating, phased mint scheduling, lazy mint)
//!   are out of scope for Wave-B; see the module-level TODO below.

// TODO(#34 at scale): CSV/large-JSON manifest ingest + generative trait composition + rarity tables.
// TODO(#40 drop mechanics): delayed reveal, allowlist/claim gating, phased scheduling, lazy mint.

use chia_protocol::{Bytes32, Coin, CoinSpend, Program};
use chia_puzzle_types::nft::NftMetadata;
use chia_puzzle_types::Memos;
use chia_wallet_sdk::driver::{
    Did, IntermediateLauncher, NftMint, SingletonInfo, SpendContext, StandardLayer,
};
use chia_wallet_sdk::types::conditions::TransferNft;
use chia_wallet_sdk::types::Conditions;
use serde::{Deserialize, Serialize};

use crate::error::{ChainError, Result};
use crate::keys::IndexedKeys;
use crate::metadata::{Attribute, Chip0007Metadata, CollectionAttribute, CollectionRef};

/// A CHIP-0007 collection definition: the shared identity + economics across every item.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collection {
    /// Stable collection id (the toolkit derives it from the creator DID + name, or supplies it).
    pub id: String,
    /// Human-readable collection name.
    pub name: String,
    /// Collection-level attributes (icon/banner/website/twitter/etc) as CHIP-0007 `type`/`value`
    /// pairs ([`CollectionAttribute`] — NOT the NFT-item [`Attribute`]; see #187).
    #[serde(default)]
    pub attributes: Vec<CollectionAttribute>,
    /// Shared royalty recipient puzzle hash for every item.
    pub royalty_puzzle_hash: Bytes32,
    /// Shared royalty in basis points for every item (e.g. 300 = 3%).
    pub royalty_basis_points: u16,
    /// Optional drop mechanics (#40 — delayed reveal / allowlist / phased / lazy).
    /// Absent (skipped in JSON) for an ordinary open, immediate, revealed collection.
    /// SCAFFOLDED: the data model is committed; enforcement is TODO (see [`Drop`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drop: Option<Drop>,
}

impl Collection {
    /// The [`CollectionRef`] block embedded into each item's CHIP-0007 metadata.
    pub fn as_ref_block(&self) -> CollectionRef {
        CollectionRef {
            id: self.id.clone(),
            name: self.name.clone(),
            attributes: self.attributes.clone(),
        }
    }
}

/// One scheduled mint phase of a drop (#40): an optional public-mint start time + an
/// optional per-phase supply cap. Phases run in order; a `None` start means "open as
/// soon as the previous phase fills / from the drop's start".
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropPhase {
    /// Human label (e.g. "allowlist", "public").
    pub name: String,
    /// Unix epoch seconds this phase opens minting; `None` = no time gate.
    #[serde(default)]
    pub start_unix: Option<u64>,
    /// Max items mintable in this phase; `None` = uncapped (bounded by total supply).
    #[serde(default)]
    pub supply: Option<u64>,
    /// Whether this phase is allowlist-gated (only `Drop::allowlist` may mint).
    #[serde(default)]
    pub allowlist_only: bool,
}

/// Drop mechanics for a collection (#40): delayed reveal, allowlist gating, and phased
/// scheduling. This is the SCAFFOLDED data model — it captures the drop's intent so the
/// definition is committable + tooling-readable; the ENFORCEMENT (gating mints on the
/// reveal time / allowlist membership / phase schedule) is NOT yet implemented in the
/// mint path. See the TODOs below.
///
/// All fields are optional and default to "no drop mechanics" (an immediate, open,
/// fully-revealed mint), so an ordinary collection serializes without a `drop` block.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Drop {
    /// DELAYED REVEAL: until this Unix time, items mint with placeholder metadata and
    /// the real metadata/art is swapped in at/after reveal. `None` = revealed at mint.
    ///
    /// TODO(#40 reveal): the mint path must (1) mint with the placeholder metadata
    /// before `reveal_unix`, and (2) provide a post-reveal metadata-update spend that
    /// swaps each item to its real metadata (an NFT metadata-update / re-mint flow).
    #[serde(default)]
    pub reveal_unix: Option<u64>,
    /// ALLOWLIST: the puzzle hashes (or DID launcher ids) permitted to mint during
    /// allowlist-gated phases. Empty = no allowlist.
    ///
    /// TODO(#40 allowlist): enforce membership at mint time (gate the mint spend on the
    /// recipient being in this set — e.g. an allowlist-merkle assertion or a per-address
    /// claim coin), and add a claim/redeem flow.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// PHASED SCHEDULE: ordered mint phases (allowlist → public, timed waves). Empty =
    /// a single open phase.
    ///
    /// TODO(#40 phases): enforce the phase order + per-phase start time + supply caps at
    /// mint time (assert the current time is within the active phase and the phase cap is
    /// not exceeded), and surface the active phase in `collection show`.
    #[serde(default)]
    pub phases: Vec<DropPhase>,
    /// LAZY MINT: when true, items are minted on-demand at claim time rather than all
    /// up-front. `false` = eager (mint the whole supply now).
    ///
    /// TODO(#40 lazy): a claim-coin / lazy-mint flow (the buyer's claim triggers the
    /// per-item mint), instead of `collection mint` minting the full supply eagerly.
    #[serde(default)]
    pub lazy_mint: bool,
}

impl Drop {
    /// Whether any drop mechanic is configured (an all-default `Drop` is "no drop").
    pub fn is_configured(&self) -> bool {
        self.reveal_unix.is_some()
            || !self.allowlist.is_empty()
            || !self.phases.is_empty()
            || self.lazy_mint
    }
}

/// One item in a parsed traits manifest. The toolkit produces this from a CSV/JSON manifest +
/// the per-item capsule hashes; this crate consumes the parsed form only (no file IO).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestItem {
    /// The item's name (e.g. `"DIG Punk #12"`).
    pub name: String,
    /// Optional per-item description.
    #[serde(default)]
    pub description: Option<String>,
    /// Per-item traits.
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    /// On-chain media metadata + hashes for this item (dig:// + https fallback URIs).
    pub media: ManifestMedia,
}

/// The on-chain media fields for a manifest item (a serde-friendly, hex-hash shape that converts to
/// the CLVM [`NftMetadata`]). Mirrors `chip35_dl_coin`'s `ManifestMedia`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestMedia {
    /// Primary media URIs (dig:// first, https fallback second by convention).
    #[serde(default)]
    pub data_uris: Vec<String>,
    /// `sha256(media_bytes)`.
    #[serde(default)]
    pub data_hash: Option<Bytes32>,
    /// CHIP-0007 metadata JSON URIs.
    #[serde(default)]
    pub metadata_uris: Vec<String>,
    /// `sha256(metadata_json_bytes)`.
    #[serde(default)]
    pub metadata_hash: Option<Bytes32>,
    /// License document URIs.
    #[serde(default)]
    pub license_uris: Vec<String>,
    /// `sha256(license_bytes)`.
    #[serde(default)]
    pub license_hash: Option<Bytes32>,
}

impl ManifestMedia {
    /// Convert into the on-chain [`NftMetadata`] CLVM struct for one mint slot.
    ///
    /// `edition_number`/`edition_total` are 1-based; both default to 1 when given 0.
    pub fn to_chain_metadata(&self, edition_number: u64, edition_total: u64) -> NftMetadata {
        NftMetadata {
            edition_number: if edition_number == 0 {
                1
            } else {
                edition_number
            },
            edition_total: if edition_total == 0 { 1 } else { edition_total },
            data_uris: self.data_uris.clone(),
            data_hash: self.data_hash,
            metadata_uris: self.metadata_uris.clone(),
            metadata_hash: self.metadata_hash,
            license_uris: self.license_uris.clone(),
            license_hash: self.license_hash,
        }
    }
}

/// Generate the per-item CHIP-0007 metadata documents for a collection from a parsed manifest.
///
/// Each item gets the collection block, its own traits, and `series_number`/`series_total` filled in
/// (1-based). This is the off-chain JSON side; the on-chain hashes come from [`ManifestMedia`]. The
/// toolkit hashes each generated document and writes it into the item's capsule. Byte-identical to
/// `chip35_dl_coin::collection::generate_item_metadata` (including `minting_tool = "DIG"`).
pub fn generate_item_metadata(
    collection: &Collection,
    items: &[ManifestItem],
) -> Vec<Chip0007Metadata> {
    let total = items.len() as u64;
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let mut md = Chip0007Metadata::new(item.name.clone());
            md.description = item.description.clone();
            md.attributes = item.attributes.clone();
            md.collection = Some(collection.as_ref_block());
            md.series_number = Some(i as u64 + 1);
            md.series_total = Some(total);
            md.minting_tool = Some("DIG".to_string());
            md
        })
        .collect()
}

/// Serialize a manifest item's on-chain media into the allocator-independent CLVM [`Program`] that
/// [`crate::nft::MintSpec::metadata`] takes. (A serialized `Program` is required, not a `HashedPtr`,
/// because the latter is allocator-relative — see [`crate::nft::MintSpec`] docs.)
pub fn item_to_metadata_program(
    item: &ManifestItem,
    edition_number: u64,
    edition_total: u64,
) -> Result<Program> {
    let chain_md = item.media.to_chain_metadata(edition_number, edition_total);
    let mut ctx = SpendContext::new();
    ctx.serialize(&chain_md)
        .map_err(|e| ChainError::Chain(format!("serialize nft metadata: {e}")))
}

/// The result of a collection bulk mint: the (UNSIGNED) coin spends + the minted NFTs' launcher ids.
#[derive(Clone, Debug)]
pub struct CollectionMint {
    /// Coin spends to sign + broadcast (the DID spend authorizing every mint).
    pub coin_spends: Vec<CoinSpend>,
    /// The minted NFTs' launcher ids, in manifest order.
    pub launcher_ids: Vec<Bytes32>,
}

/// Build the (UNSIGNED) coin spends that bulk-mint every `item` into `collection`, each attributed to
/// `did` and owned by `recipient_ph`, authorized by a SINGLE spend of the DID coin.
///
/// This is the digstore-chain twin of `chip35_dl_coin::collection::build_bulk_mint`: one
/// [`IntermediateLauncher`] per item carrying the collection's shared royalty + the DID `TransferNft`
/// attribution, then the DID is spent once (`did.update`) emitting every mint's conditions — so all
/// NFTs are minted atomically AND attributed to the creator DID in one bundle. The DID coin must be
/// the reconstructed, spendable [`Did`] (e.g. from [`crate::did::list_owned_dids`]) and `minter` must
/// hold its keys. `recipient_ph` owns every minted NFT (default it to the minter's address).
///
/// **Pure: does NOT sign or broadcast.** The DID is consumed by its spend; the caller re-fetches the
/// recreated DID from chain to chain further mints. Errors if `items` is empty.
pub fn build_collection_mint(
    minter: &IndexedKeys,
    did: Did,
    collection: &Collection,
    items: &[ManifestItem],
    recipient_ph: Bytes32,
) -> Result<CollectionMint> {
    let mut ctx = SpendContext::new();
    let out = build_collection_mint_in(&mut ctx, minter, did, collection, items, recipient_ph)?;
    Ok(CollectionMint {
        coin_spends: ctx.take(),
        launcher_ids: out,
    })
}

/// [`build_collection_mint`] into a caller-provided `ctx` — the launcher metadata + the DID spend are
/// allocator-relative, so when the DID was created/parsed in a specific context the mint MUST be built
/// in that SAME context. Returns just the launcher ids; the spends accumulate in `ctx`. (The public
/// wrapper uses a fresh context for the on-chain case where the DID is reconstructed independently.)
pub fn build_collection_mint_in(
    ctx: &mut SpendContext,
    minter: &IndexedKeys,
    did: Did,
    collection: &Collection,
    items: &[ManifestItem],
    recipient_ph: Bytes32,
) -> Result<Vec<Bytes32>> {
    Ok(build_collection_mint_core_in(ctx, minter, did, collection, items, recipient_ph)?.0)
}

/// The shared core of every collection-mint builder: emits each item's launcher/mint spends into
/// `ctx` and spends the DID once to authorize them, returning BOTH the launcher ids AND the
/// **recreated DID** — the DID's next generation left on chain by its `update` spend.
///
/// The recreated DID matters for BATCHING (#231): a large mint is split into cost-bounded batches
/// (see [`plan_batches`]), and each batch spends the DID once, advancing it one generation. The
/// next batch must spend that recreated DID — so this returns it, letting a caller chain batches
/// (the CLI re-fetches the DID from chain between confirmed batches; the Simulator test chains the
/// returned value directly). The public non-batch wrappers discard it (they build a single bundle).
fn build_collection_mint_core_in(
    ctx: &mut SpendContext,
    minter: &IndexedKeys,
    did: Did,
    collection: &Collection,
    items: &[ManifestItem],
    recipient_ph: Bytes32,
) -> Result<(Vec<Bytes32>, Did)> {
    if items.is_empty() {
        return Err(ChainError::Chain(
            "build_collection_mint: at least one item is required".into(),
        ));
    }

    let p2 = StandardLayer::new(minter.synthetic_pk);

    let did_launcher = did.info.launcher_id;
    let did_inner_ph: Bytes32 = did.info.inner_puzzle_hash().into();

    let total = items.len();
    let mut all_mint_conditions = Conditions::new();
    let mut launcher_ids = Vec::with_capacity(total);

    for (i, item) in items.iter().enumerate() {
        // Allocate the on-chain metadata for this item into THIS context (a HashedPtr is
        // allocator-relative — see `crate::nft::MintSpec`).
        let chain_md = item.media.to_chain_metadata(i as u64 + 1, total as u64);
        let metadata_ptr = ctx
            .alloc_hashed(&chain_md)
            .map_err(|e| ChainError::Chain(format!("alloc item {i} metadata: {e}")))?;

        let transfer = TransferNft::new(Some(did_launcher), Vec::new(), Some(did_inner_ph));
        let mut nft_mint = NftMint::new(
            metadata_ptr,
            recipient_ph,
            collection.royalty_basis_points,
            Some(transfer),
        );
        nft_mint.royalty_puzzle_hash = collection.royalty_puzzle_hash;

        let (mint_conditions, nft) = IntermediateLauncher::new(did.coin.coin_id(), i, total)
            .create(ctx)
            .map_err(|e| ChainError::Chain(format!("create launcher {i}: {e}")))?
            .mint_nft(ctx, &nft_mint)
            .map_err(|e| ChainError::Chain(format!("mint nft {i}: {e}")))?;
        all_mint_conditions = all_mint_conditions.extend(mint_conditions);
        launcher_ids.push(nft.info.launcher_id);
    }

    // Spend the DID once, authorizing all mints (it acknowledges every attribution). The recreated
    // DID singleton is returned so a batched caller can chain the next batch onto it (#231).
    let recreated = did
        .update(ctx, &p2, all_mint_conditions)
        .map_err(|e| ChainError::Chain(format!("spend did for collection mint: {e}")))?;

    Ok((launcher_ids, recreated))
}

/// Build a MULTI-item DID-attributed collection mint FUNDED by a separate XCH coin (#199).
///
/// [`build_collection_mint_in`] is structurally correct for any `items.len()` — but a REAL on-chain
/// mint of N>1 items needs more value than the DID singleton carries. Each item's
/// [`IntermediateLauncher`] uses the standard Chia bulk-mint idiom: a 0-value "intermediate" coin
/// whose OWN spend creates a 1-mojo singleton launcher coin — i.e. it prints 1 mojo per item that
/// must be donated from elsewhere in the SAME spend bundle (Chia's coin-value conservation is
/// bundle-wide, not per-coin). The DID's `update` spend conserves its OWN coin value exactly (it
/// recreates itself at the same amount), so it cannot supply that extra value for more than the one
/// mojo it might have been over-funded with at creation. `funding_coin` is that donor: it is spent
/// through `funding_key`'s standard puzzle to contribute exactly `items.len()` mojos to the bundle,
/// with any excess returned as change to `funding_key.owner_puzzle_hash` (a larger-than-needed coin
/// is never silently burned as network fee).
///
/// `funding_key` may be a different HD address than `minter` (whichever address actually holds a
/// sufficient XCH coin) — both keys are needed to sign the resulting bundle.
///
/// **Pure: does NOT sign or broadcast.** Errors if `funding_coin.amount < items.len() as u64`.
pub fn build_collection_mint_funded(
    minter: &IndexedKeys,
    did: Did,
    collection: &Collection,
    items: &[ManifestItem],
    recipient_ph: Bytes32,
    funding_coin: Coin,
    funding_key: &IndexedKeys,
) -> Result<CollectionMint> {
    let mut ctx = SpendContext::new();
    let launcher_ids = build_collection_mint_funded_in(
        &mut ctx,
        minter,
        did,
        collection,
        items,
        recipient_ph,
        funding_coin,
        funding_key,
    )?;
    Ok(CollectionMint {
        coin_spends: ctx.take(),
        launcher_ids,
    })
}

/// [`build_collection_mint_funded`] into a caller-provided `ctx` (see [`build_collection_mint_in`]'s
/// docs for why a shared context matters when the DID was created/parsed in that same context).
#[allow(clippy::too_many_arguments)]
pub fn build_collection_mint_funded_in(
    ctx: &mut SpendContext,
    minter: &IndexedKeys,
    did: Did,
    collection: &Collection,
    items: &[ManifestItem],
    recipient_ph: Bytes32,
    funding_coin: Coin,
    funding_key: &IndexedKeys,
) -> Result<Vec<Bytes32>> {
    Ok(build_collection_mint_funded_core_in(
        ctx,
        minter,
        did,
        collection,
        items,
        recipient_ph,
        funding_coin,
        funding_key,
    )?
    .0)
}

/// The funded core: [`build_collection_mint_core_in`] plus the XCH funding-coin spend, returning the
/// launcher ids AND the recreated DID (for batch chaining, #231).
#[allow(clippy::too_many_arguments)]
fn build_collection_mint_funded_core_in(
    ctx: &mut SpendContext,
    minter: &IndexedKeys,
    did: Did,
    collection: &Collection,
    items: &[ManifestItem],
    recipient_ph: Bytes32,
    funding_coin: Coin,
    funding_key: &IndexedKeys,
) -> Result<(Vec<Bytes32>, Did)> {
    let needed = items.len() as u64;
    if funding_coin.amount < needed {
        return Err(ChainError::Chain(format!(
            "funding coin has {} mojo but minting {} item(s) needs at least {needed} mojo (1 mojo \
             per item — the DID's own value cannot fund the extra singleton launchers)",
            funding_coin.amount,
            items.len(),
        )));
    }

    let (launcher_ids, recreated) =
        build_collection_mint_core_in(ctx, minter, did, collection, items, recipient_ph)?;

    // Fund the `needed` mojos the intermediate-launcher trick prints per item (see docs above); any
    // excess returns as change so a larger-than-needed coin is never silently burned as network fee.
    let change = funding_coin.amount - needed;
    let mut funding_conditions = Conditions::new();
    if change > 0 {
        funding_conditions =
            funding_conditions.create_coin(funding_key.owner_puzzle_hash, change, Memos::None);
    }
    StandardLayer::new(funding_key.synthetic_pk)
        .spend(ctx, funding_coin, funding_conditions)
        .map_err(|e| ChainError::Chain(format!("spend funding coin: {e}")))?;

    Ok((launcher_ids, recreated))
}

// ===========================================================================
// #231 — cost-bounded auto-batching for large collection mints
//
// A single spend bundle for N items exceeds Chia's per-block CLVM cost limit once N grows
// (dkackman hit this at ~200 items: the full node rejected the oversized `push_tx` and coinset
// returned an aborted body, misreported as a connectivity error). The fix splits a large mint
// into COST-BOUNDED batches — each a self-contained bundle under the block limit — built, funded,
// signed, broadcast, and confirmed sequentially, all attributed to the same collection DID.
// ===========================================================================

/// Chia mainnet per-block CLVM cost ceiling (`ConsensusConstants::max_block_cost_clvm`). A spend
/// bundle whose total CLVM cost exceeds this is rejected by every full node, so a bulk mint MUST be
/// split into bundles that each stay under it (with margin — see [`batch_cost_budget`]).
pub const MAX_BLOCK_COST_CLVM: u64 = 11_000_000_000;

/// The fraction (numerator/denominator) of [`MAX_BLOCK_COST_CLVM`] a single mint batch may occupy.
/// A batch is packed so its ESTIMATED cost stays under `MAX_BLOCK_COST_CLVM * NUM / DEN`. The
/// remaining margin absorbs (a) cost-estimate error, (b) other transactions competing for the same
/// block, and (c) the practical request/response-size limits of the coinset.org gateway. `1/4` is
/// deliberately conservative — a bulk mint is not latency-critical, and overshooting the block cost
/// fails the entire batch on-chain (real XCH already committed to the earlier batches).
pub const BATCH_COST_BUDGET_NUM: u64 = 1;
/// Denominator of the per-batch cost budget fraction (see [`BATCH_COST_BUDGET_NUM`]).
pub const BATCH_COST_BUDGET_DEN: u64 = 4;

/// Estimated CLVM cost of ONE collection-mint item: its intermediate-launcher spend, the singleton
/// launcher spend, the eve-NFT spend, their CREATE_COIN / AGG_SIG conditions, and the generator
/// bytes of those puzzle reveals. Rounded UP from the measured cost so the estimate never
/// UNDER-counts — an over-estimate only makes batches smaller, which is always safe. The test
/// `est_cost_per_item_is_conservative` runs a real batch through the Chia consensus cost model
/// (`run_spendbundle`) and fails if this constant drops below the measured marginal per-item cost.
/// Measured marginal (mainnet cost model, chia-wallet-sdk 0.30): ~69.76M CLVM per item; this is set
/// ~15% higher for margin against build/puzzle variation.
pub const EST_COST_PER_ITEM_CLVM: u64 = 80_000_000;

/// Estimated FIXED per-batch cost, independent of item count: the single DID `update` spend, the
/// funding-coin spend, and base generator framing. Kept conservative for the same reason as
/// [`EST_COST_PER_ITEM_CLVM`] (`est_cost_per_item_is_conservative` also guards this — measured fixed
/// overhead is ~100–150M CLVM; this is set well above it).
pub const EST_BATCH_BASE_COST_CLVM: u64 = 300_000_000;

/// Estimated total CLVM cost of a batch of `n_items` collection-mint items.
pub fn estimate_batch_cost(n_items: usize) -> u64 {
    EST_BATCH_BASE_COST_CLVM.saturating_add(EST_COST_PER_ITEM_CLVM.saturating_mul(n_items as u64))
}

/// The per-batch CLVM-cost budget: the block ceiling times the safety fraction. Every batch's
/// [`estimate_batch_cost`] must stay at or under this.
pub fn batch_cost_budget() -> u64 {
    MAX_BLOCK_COST_CLVM / BATCH_COST_BUDGET_DEN * BATCH_COST_BUDGET_NUM
}

/// The default cost-bounded number of items per batch: the largest N whose [`estimate_batch_cost`]
/// stays within [`batch_cost_budget`] (at least 1). This is what `collection mint` uses when
/// `--batch-size` is not given — COMPUTED from the cost model, never a hard-coded count.
pub fn default_batch_size() -> usize {
    let usable = batch_cost_budget().saturating_sub(EST_BATCH_BASE_COST_CLVM);
    (usable / EST_COST_PER_ITEM_CLVM.max(1)).max(1) as usize
}

/// Validate an explicit `--batch-size`: it must be at least 1 and a batch of that many items must
/// fit within [`batch_cost_budget`]. On a too-large size the error is a terminal
/// [`ChainError::BundleTooLarge`] naming the maximum allowed size, so the CLI can surface an
/// actionable message (never the misleading "check your connection to coinset.org").
pub fn validate_batch_size(size: usize) -> Result<()> {
    if size == 0 {
        return Err(ChainError::Chain("--batch-size must be at least 1".into()));
    }
    if estimate_batch_cost(size) > batch_cost_budget() {
        return Err(ChainError::BundleTooLarge(format!(
            "--batch-size {size} is too large: its estimated CLVM cost ({}) exceeds the safe \
             per-batch budget ({}). Use --batch-size {} or lower.",
            estimate_batch_cost(size),
            batch_cost_budget(),
            default_batch_size(),
        )));
    }
    Ok(())
}

/// Split `total` items into contiguous, cost-bounded batch ranges. With `batch_size == None` the
/// [`default_batch_size`] is used; an explicit size is [`validate_batch_size`]-checked. Every
/// returned range has length `<=` the chosen size, the ranges are contiguous and cover `0..total`
/// exactly, and each stays within [`batch_cost_budget`].
pub fn plan_batches(
    total: usize,
    batch_size: Option<usize>,
) -> Result<Vec<std::ops::Range<usize>>> {
    if total == 0 {
        return Err(ChainError::Chain(
            "plan_batches: at least one item is required".into(),
        ));
    }
    let size = match batch_size {
        Some(s) => {
            validate_batch_size(s)?;
            s
        }
        None => default_batch_size(),
    };
    let mut batches = Vec::new();
    let mut start = 0;
    while start < total {
        let end = (start + size).min(total);
        batches.push(start..end);
        start = end;
    }
    Ok(batches)
}

/// A built collection-mint BATCH: the coin spends (unsigned) to broadcast, the launcher ids minted,
/// and the **recreated DID** the batch leaves on chain. Batching advances the DID one generation per
/// batch — the NEXT batch spends [`CollectionBatch::next_did`] (the CLI re-fetches it from chain
/// after confirmation; a same-process caller can chain the returned value directly). The recreated
/// DID's coin id is also the deterministic confirmation target for the batch (its appearance on
/// chain proves the batch landed).
#[derive(Clone, Debug)]
pub struct CollectionBatch {
    /// Coin spends to sign + broadcast (the DID spend + funding-coin spend + every item's mint).
    pub coin_spends: Vec<CoinSpend>,
    /// The minted NFTs' launcher ids, in item order.
    pub launcher_ids: Vec<Bytes32>,
    /// The DID's next generation, recreated by this batch's DID spend (spend it for the next batch).
    pub next_did: Did,
}

/// Build ONE funded, DID-attributed collection-mint batch (a fresh [`SpendContext`]), returning the
/// coin spends, launcher ids, AND the recreated DID for chaining the next batch (#231). This is the
/// batch primitive `collection mint` loops over for a large collection; it is
/// [`build_collection_mint_funded`] plus the recreated-DID return.
#[allow(clippy::too_many_arguments)]
pub fn build_collection_batch(
    minter: &IndexedKeys,
    did: Did,
    collection: &Collection,
    items: &[ManifestItem],
    recipient_ph: Bytes32,
    funding_coin: Coin,
    funding_key: &IndexedKeys,
) -> Result<CollectionBatch> {
    let mut ctx = SpendContext::new();
    let (launcher_ids, next_did) = build_collection_mint_funded_core_in(
        &mut ctx,
        minter,
        did,
        collection,
        items,
        recipient_ph,
        funding_coin,
        funding_key,
    )?;
    Ok(CollectionBatch {
        coin_spends: ctx.take(),
        launcher_ids,
        next_did,
    })
}

/// A stable fingerprint of a manifest's raw bytes (SHA-256, hex) — the resume key component that
/// ties a [`MintProgress`] record to the EXACT manifest it was started for, so a re-run against a
/// different manifest never resumes onto the wrong items.
pub fn manifest_fingerprint(manifest_bytes: &[u8]) -> String {
    use chia_sha2::Sha256;
    let mut h = Sha256::new();
    h.update(manifest_bytes);
    hex::encode(h.finalize())
}

/// Persisted progress of a resumable multi-batch collection mint (#231).
///
/// A large mint spends real XCH one batch at a time. This record lets a re-run SKIP batches that
/// already landed on chain, so an interruption after batch K never re-mints or double-spends batches
/// `0..=K`. Correctness rests on the DID being a single-use coin per generation: each batch spends
/// the DID exactly once, so at most one mint can confirm per DID generation — the mint can never
/// double-mint a batch even if a re-run rebuilds it. Each [`BatchRecord`] additionally captures the
/// DID coin it spent (for chain reconciliation of a pushed-but-unconfirmed tail), the recreated DID
/// coin (the confirmation target), the tx id, and the launcher ids, for progress display + auditing.
///
/// The type is pure serializable data; the CLI owns the file path (`~/.dig/collection-mints/…`) and
/// the chain reconciliation queries.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MintProgress {
    /// The collection id being minted.
    pub collection_id: String,
    /// The creator DID launcher id (hex) the mint is attributed to.
    pub did: String,
    /// [`manifest_fingerprint`] of the manifest bytes — resume applies only to the SAME manifest.
    pub manifest_hash: String,
    /// Total items in the manifest.
    pub total_items: usize,
    /// The batch size in effect (so a resume keeps identical batch boundaries).
    pub batch_size: usize,
    /// Recorded batches, in item order. A batch appears here once broadcast (unconfirmed) and is
    /// flipped to `confirmed` once its landing is verified on chain.
    pub batches: Vec<BatchRecord>,
}

/// One batch's record within a [`MintProgress`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchRecord {
    /// Half-open item range `[start, end)` this batch mints.
    pub start: usize,
    /// End (exclusive) of the batch's item range.
    pub end: usize,
    /// The DID coin id (hex) this batch's DID spend consumed — its generation. On resume, if this
    /// coin is already SPENT on chain, the batch landed even if we never recorded its confirmation.
    pub did_coin_id: String,
    /// The recreated DID coin id (hex) this batch produces — the deterministic confirmation target
    /// (its presence on chain proves the batch landed).
    pub next_did_coin_id: String,
    /// The broadcast tx id (hex).
    pub tx_id: String,
    /// The launcher ids (hex) minted in this batch, in item order.
    pub launcher_ids: Vec<String>,
    /// True once the batch's landing is confirmed on chain.
    pub confirmed: bool,
}

impl MintProgress {
    /// A fresh, empty progress record for a mint of `total_items` at `batch_size`.
    pub fn new(
        collection_id: impl Into<String>,
        did: impl Into<String>,
        manifest_hash: impl Into<String>,
        total_items: usize,
        batch_size: usize,
    ) -> Self {
        Self {
            collection_id: collection_id.into(),
            did: did.into(),
            manifest_hash: manifest_hash.into(),
            total_items,
            batch_size,
            batches: Vec::new(),
        }
    }

    /// Whether a stored record matches the current mint parameters. A mismatch means the stored
    /// record is stale/foreign (different collection, DID, or manifest) and MUST NOT be resumed.
    pub fn matches(&self, collection_id: &str, did: &str, manifest_hash: &str) -> bool {
        self.collection_id == collection_id
            && self.did == did
            && self.manifest_hash == manifest_hash
    }

    /// The number of items confirmed minted so far (sum of confirmed batch lengths).
    pub fn minted_count(&self) -> usize {
        self.batches
            .iter()
            .filter(|b| b.confirmed)
            .map(|b| b.end - b.start)
            .sum()
    }

    /// Whether the batch covering `[start, end)` is recorded AND confirmed.
    pub fn is_confirmed(&self, start: usize, end: usize) -> bool {
        self.batches
            .iter()
            .any(|b| b.start == start && b.end == end && b.confirmed)
    }

    /// The recorded batch covering `[start, end)`, if any (confirmed or not).
    pub fn record(&self, start: usize, end: usize) -> Option<&BatchRecord> {
        self.batches
            .iter()
            .find(|b| b.start == start && b.end == end)
    }

    /// The last recorded batch that is NOT yet confirmed. Because batches are broadcast strictly in
    /// order (each spends the DID recreated by the prior), at most one such "in-flight" batch can
    /// exist — the tail — and it is the one a resume must reconcile against chain.
    pub fn pending_tail(&self) -> Option<&BatchRecord> {
        self.batches.last().filter(|b| !b.confirmed)
    }

    /// Record a batch as broadcast-but-unconfirmed (idempotent: re-recording the same range updates
    /// it in place rather than appending a duplicate).
    #[allow(clippy::too_many_arguments)]
    pub fn record_pending(
        &mut self,
        start: usize,
        end: usize,
        did_coin_id: String,
        next_did_coin_id: String,
        tx_id: String,
        launcher_ids: Vec<String>,
    ) {
        let rec = BatchRecord {
            start,
            end,
            did_coin_id,
            next_did_coin_id,
            tx_id,
            launcher_ids,
            confirmed: false,
        };
        match self
            .batches
            .iter_mut()
            .find(|b| b.start == start && b.end == end)
        {
            Some(existing) => *existing = rec,
            None => self.batches.push(rec),
        }
    }

    /// Mark the batch covering `[start, end)` confirmed. No-op if there is no such record.
    pub fn confirm(&mut self, start: usize, end: usize) {
        if let Some(b) = self
            .batches
            .iter_mut()
            .find(|b| b.start == start && b.end == end)
        {
            b.confirmed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection() -> Collection {
        Collection {
            id: "dig-punks".into(),
            name: "DIG Punks".into(),
            attributes: vec![CollectionAttribute {
                kind: "website".into(),
                value: "https://dig.net".into(),
            }],
            royalty_puzzle_hash: Bytes32::from([0x22; 32]),
            royalty_basis_points: 300,
            drop: None,
        }
    }

    /// #40 drop model: an unconfigured `Drop` is "no drop"; configured flags round-trip
    /// through JSON and a plain collection serializes WITHOUT a `drop` block (so existing
    /// definitions are unchanged). Scaffold guard — pins the committable data model.
    #[test]
    fn drop_model_round_trips_and_is_optional() {
        // Default drop is not configured.
        assert!(!Drop::default().is_configured());

        // A configured drop round-trips every mechanic.
        let drop = Drop {
            reveal_unix: Some(1_900_000_000),
            allowlist: vec!["abcd".into()],
            phases: vec![DropPhase {
                name: "allowlist".into(),
                start_unix: Some(1_800_000_000),
                supply: Some(100),
                allowlist_only: true,
            }],
            lazy_mint: true,
        };
        assert!(drop.is_configured());
        let json = serde_json::to_string(&drop).unwrap();
        let back: Drop = serde_json::from_str(&json).unwrap();
        assert_eq!(back, drop);

        // A plain collection omits the drop block entirely.
        let plain = serde_json::to_string(&collection()).unwrap();
        assert!(
            !plain.contains("\"drop\""),
            "no drop block on a plain collection: {plain}"
        );

        // A collection WITH a drop serializes it and round-trips.
        let mut c = collection();
        c.drop = Some(drop);
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"drop\""));
        let back: Collection = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    fn items() -> Vec<ManifestItem> {
        vec![
            ManifestItem {
                name: "DIG Punk #1".into(),
                description: Some("first".into()),
                attributes: vec![Attribute {
                    trait_type: "Background".into(),
                    value: "Blue".into(),
                }],
                media: ManifestMedia {
                    data_uris: vec!["dig://store/1.png".into(), "https://gw/1.png".into()],
                    data_hash: Some(Bytes32::from([0x11; 32])),
                    ..Default::default()
                },
            },
            ManifestItem {
                name: "DIG Punk #2".into(),
                description: None,
                attributes: vec![],
                media: ManifestMedia {
                    data_uris: vec!["dig://store/2.png".into()],
                    data_hash: Some(Bytes32::from([0x12; 32])),
                    ..Default::default()
                },
            },
        ]
    }

    /// `n` distinct manifest items (each with a unique `data_hash`) — for the multi-item (N>1) funded
    /// mint tests (#199), where `items()`'s fixed 2 aren't enough to prove an arbitrary N.
    fn items_n(n: usize) -> Vec<ManifestItem> {
        (0..n)
            .map(|i| ManifestItem {
                name: format!("DIG Punk #{i}"),
                description: None,
                attributes: vec![],
                media: ManifestMedia {
                    data_uris: vec![format!("dig://store/{i}.png")],
                    data_hash: Some(Bytes32::from([i as u8; 32])),
                    ..Default::default()
                },
            })
            .collect()
    }

    #[test]
    fn generate_item_metadata_fills_series_and_collection() {
        let col = collection();
        let mds = generate_item_metadata(&col, &items());
        assert_eq!(mds.len(), 2);
        // 1-based series numbering with the total.
        assert_eq!(mds[0].series_number, Some(1));
        assert_eq!(mds[1].series_number, Some(2));
        assert_eq!(mds[0].series_total, Some(2));
        // Each item carries the collection ref block and the DIG minting tool tag.
        assert_eq!(mds[0].collection.as_ref().unwrap().id, "dig-punks");
        assert_eq!(mds[0].minting_tool.as_deref(), Some("DIG"));
        // Per-item traits + description are preserved.
        assert_eq!(mds[0].attributes[0].value, "Blue");
        assert_eq!(mds[0].description.as_deref(), Some("first"));
        assert_eq!(mds[1].description, None);
    }

    /// The first item's generated CHIP-0007 JSON must be EXACTLY this byte string — the cross-module
    /// parity guard for the collection path (it must match `chip35_dl_coin`'s output byte-for-byte).
    /// #187: the embedded collection-level attribute renders with `"type"`, NOT `"trait_type"`
    /// (CHIP-0007); the item-level attribute stays `"trait_type"`.
    #[test]
    fn generated_item_json_is_pinned() {
        let col = collection();
        let mds = generate_item_metadata(&col, &items());
        assert_eq!(
            mds[0].to_canonical_json().unwrap(),
            r#"{"format":"CHIP-0007","name":"DIG Punk #1","description":"first","collection":{"id":"dig-punks","name":"DIG Punks","attributes":[{"type":"website","value":"https://dig.net"}]},"attributes":[{"trait_type":"Background","value":"Blue"}],"series_number":1,"series_total":2,"minting_tool":"DIG"}"#
        );
    }

    // ---------- #187: collection.json parses CHIP-0007 `type` attributes (dkackman's bug) ----------

    /// dkackman's exact bug reproduced at the `Collection` deserialization level: a CHIP-0007-
    /// conformant collection.json using `"type"` for a collection attribute must parse. Before the
    /// #187 fix this failed with "missing field `trait_type`" because `Collection::attributes` was
    /// typed `Vec<Attribute>` (the NFT-item shape).
    #[test]
    fn collection_json_with_chip0007_type_attribute_parses() {
        let raw = r#"{
            "id": "dig-punks",
            "name": "DIG Punks",
            "attributes": [{"type": "icon", "value": "https://dig.net/icon.png"}],
            "royalty_puzzle_hash": "2222222222222222222222222222222222222222222222222222222222222222",
            "royalty_basis_points": 300
        }"#;
        let col: Collection = serde_json::from_str(raw)
            .expect("a CHIP-0007-conformant collection.json (attribute `type`) must parse");
        assert_eq!(col.attributes[0].kind, "icon");
        assert_eq!(col.attributes[0].value, "https://dig.net/icon.png");
    }

    /// Back-compat (§5.1): a collection.json already emitted with the OLD, non-conformant
    /// `trait_type` field on its collection attributes still parses (the alias).
    #[test]
    fn collection_json_with_legacy_trait_type_attribute_still_parses() {
        let raw = r#"{
            "id": "dig-punks",
            "name": "DIG Punks",
            "attributes": [{"trait_type": "icon", "value": "https://dig.net/icon.png"}],
            "royalty_puzzle_hash": "2222222222222222222222222222222222222222222222222222222222222222",
            "royalty_basis_points": 300
        }"#;
        let col: Collection = serde_json::from_str(raw)
            .expect("the legacy trait_type collection attribute spelling must still parse");
        assert_eq!(col.attributes[0].kind, "icon");
    }

    /// A parsed manifest item's own `attributes` are UNCHANGED by #187 — they still use
    /// `trait_type`, and a collection-style `type` field is rejected (the two shapes stay distinct).
    #[test]
    fn manifest_item_attribute_still_requires_trait_type() {
        let raw = r#"{"name":"A","attributes":[{"type":"Foo","value":"Bar"}],"media":{}}"#;
        let err = serde_json::from_str::<ManifestItem>(raw).unwrap_err();
        assert!(
            err.to_string().contains("trait_type"),
            "manifest item attributes must still demand trait_type, got: {err}"
        );
    }

    #[test]
    fn to_chain_metadata_defaults_editions_to_one() {
        let m = ManifestMedia::default();
        let chain = m.to_chain_metadata(0, 0);
        assert_eq!(chain.edition_number, 1);
        assert_eq!(chain.edition_total, 1);
    }

    #[test]
    fn item_to_metadata_program_serializes() {
        let its = items();
        let prog = item_to_metadata_program(&its[0], 1, 2).unwrap();
        // A serialized NftMetadata is non-empty CLVM bytes.
        assert!(!prog.to_vec().is_empty());
    }

    #[test]
    fn build_collection_mint_rejects_empty_items() {
        use crate::keys::derive_indexed_keys;
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::Launcher;

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let alice = sim.bls(2);
        let alice_p2 = StandardLayer::new(alice.pk);
        let (create_did, did) = Launcher::new(alice.coin.coin_id(), 1)
            .create_simple_did(ctx, &alice_p2)
            .unwrap();
        alice_p2.spend(ctx, alice.coin, create_did).unwrap();

        let minter = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let err = build_collection_mint(&minter, did, &collection(), &[], minter.owner_puzzle_hash)
            .unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("at least one item")),
            "got: {err}"
        );
    }

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    /// The public [`build_collection_mint`] PRODUCES coin spends for every item, attributed to the DID
    /// (the chip35 `build_bulk_mint` contract: produces a valid spend set). Uses a freshly created DID
    /// in its own context — mirrors `chip35_dl_coin`'s `build_bulk_mint_produces_spends_for_all_items`.
    #[test]
    fn build_collection_mint_produces_spends_for_all_items() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::Launcher;

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let alice = sim.bls(2);
        let alice_p2 = StandardLayer::new(alice.pk);
        let (_create_did, did) =
            Launcher::new(alice.coin.coin_id(), 1).create_simple_did(ctx, &alice_p2)?;

        let alice_keys = crate::keys::IndexedKeys {
            index: 0,
            synthetic_sk: alice.sk.clone(),
            synthetic_pk: alice.pk,
            owner_puzzle_hash: alice.puzzle_hash,
        };
        let col = collection();
        let out = build_collection_mint(&alice_keys, did, &col, &items(), alice.puzzle_hash)?;
        assert_eq!(out.launcher_ids.len(), 2, "two NFTs produced");
        assert_ne!(out.launcher_ids[0], out.launcher_ids[1]);
        assert!(!out.coin_spends.is_empty(), "spends produced");
        Ok(())
    }

    /// Mint a 1-item collection attributed to a DID in ONE atomic bundle and VALIDATE it on the
    /// in-process Chia simulator: create the DID and mint in the SAME context (so the eve DID is spent
    /// in the same bundle as the launcher it parents — the validated DID-attributed mint shape, like
    /// `crate::nft::mint_nft_attributed_to_did`). Proves the conditions the builder emits actually
    /// pass consensus and that the minted NFT is assigned to the collection's DID.
    ///
    /// (One item, because the DID singleton carries 1 mojo and parents the launcher directly; a
    /// MULTI-item DID-spent mint needs a separate XCH funding coin for the extra launchers — see
    /// [`build_collection_mint_funded_in_validates_on_simulator`] for the funded N>1 path, #199.)
    #[test]
    fn build_collection_mint_in_validates_on_simulator() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::Launcher;

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        // Create the DID and spend its funding coin (in `ctx`); the returned eve `did` is spendable
        // here. The collection mint reuses THIS ctx so the eve DID is spent in the same bundle.
        let alice = sim.bls(2);
        let alice_p2 = StandardLayer::new(alice.pk);
        let (create_did, did) =
            Launcher::new(alice.coin.coin_id(), 1).create_simple_did(ctx, &alice_p2)?;
        alice_p2.spend(ctx, alice.coin, create_did)?;
        let did_launcher = did.info.launcher_id;

        let alice_keys = crate::keys::IndexedKeys {
            index: 0,
            synthetic_sk: alice.sk.clone(),
            synthetic_pk: alice.pk,
            owner_puzzle_hash: alice.puzzle_hash,
        };
        let col = collection();
        let recipient = crate::keys::derive_indexed_keys(ABANDON, 0..1)?[0].owner_puzzle_hash;
        let one_item = vec![items().remove(0)];
        let launcher_ids =
            build_collection_mint_in(ctx, &alice_keys, did, &col, &one_item, recipient)?;
        assert_eq!(launcher_ids.len(), 1);

        // Apply the whole bundle (DID create + DID-spent mint) atomically; consensus validates it.
        let spends = ctx.take();
        let sig = crate::nft::sign_nft_spends(&spends, std::slice::from_ref(&alice.sk), true)?;
        sim.new_transaction(chia_protocol::SpendBundle::new(spends, sig))?;
        // The launcher landed: its singleton coin exists and the DID acknowledged it.
        let _ = (did_launcher, launcher_ids);
        Ok(())
    }

    // ---------- #199: multi-item (N>1) funded collection mint ----------

    /// [`build_collection_mint_funded`] refuses a funding coin that is too small (1 mojo needed per
    /// item), with a clear, actionable message — BEFORE building any spend.
    #[test]
    fn build_collection_mint_funded_rejects_underfunded_coin() {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::Launcher;

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let alice = sim.bls(2);
        let alice_p2 = StandardLayer::new(alice.pk);
        let (create_did, did) = Launcher::new(alice.coin.coin_id(), 1)
            .create_simple_did(ctx, &alice_p2)
            .unwrap();
        alice_p2.spend(ctx, alice.coin, create_did).unwrap();

        let alice_keys = crate::keys::IndexedKeys {
            index: 0,
            synthetic_sk: alice.sk.clone(),
            synthetic_pk: alice.pk,
            owner_puzzle_hash: alice.puzzle_hash,
        };
        let col = collection();
        let three_items = items_n(3);
        // A funding coin worth only 2 mojo can't cover 3 items (needs >= 3).
        let underfunded = Coin::new(Bytes32::from([0x99; 32]), alice.puzzle_hash, 2);
        let err = build_collection_mint_funded(
            &alice_keys,
            did,
            &col,
            &three_items,
            alice.puzzle_hash,
            underfunded,
            &alice_keys,
        )
        .unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("needs at least 3 mojo")),
            "got: {err}"
        );
    }

    /// THE #199 proof: a MULTI-item (N=3) DID-attributed collection mint, funded by a separate XCH
    /// coin, VALIDATES on the in-process Chia simulator — every NFT mints, all attributed to the same
    /// collection DID, and the whole bundle (DID create + funding-coin spend + 3 launchers) balances
    /// under real consensus. This is the proof the pre-#199 code lacked (`build_collection_mint_in`
    /// alone builds N>1 spends structurally, but they fail consensus for real value-conservation
    /// reasons — see [`build_collection_mint_funded_in`]'s docs).
    #[test]
    fn build_collection_mint_funded_in_validates_on_simulator() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::Launcher;

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        // Create the DID (1-mojo singleton, no spare change) in the SAME context as the mint, exactly
        // as the validated single-item test does.
        let alice = sim.bls(1);
        let alice_p2 = StandardLayer::new(alice.pk);
        let (create_did, did) =
            Launcher::new(alice.coin.coin_id(), 1).create_simple_did(ctx, &alice_p2)?;
        alice_p2.spend(ctx, alice.coin, create_did)?;
        let did_launcher = did.info.launcher_id;

        let alice_keys = crate::keys::IndexedKeys {
            index: 0,
            synthetic_sk: alice.sk.clone(),
            synthetic_pk: alice.pk,
            owner_puzzle_hash: alice.puzzle_hash,
        };
        let col = collection();
        let three_items = items_n(3);

        // A SEPARATE XCH coin funds the 3 mojo the intermediate-launcher trick needs (more than
        // exactly 3, to also prove change comes back rather than being burned as fee).
        let funding = sim.new_coin(alice.puzzle_hash, 10);

        let launcher_ids = build_collection_mint_funded_in(
            ctx,
            &alice_keys,
            did,
            &col,
            &three_items,
            alice.puzzle_hash,
            funding,
            &alice_keys,
        )?;
        assert_eq!(launcher_ids.len(), 3, "three NFTs produced");
        let unique: std::collections::HashSet<_> = launcher_ids.iter().collect();
        assert_eq!(unique.len(), 3, "launcher ids are distinct");

        // Apply the whole bundle atomically; consensus validates the value conservation + every mint.
        let spends = ctx.take();
        let sig = crate::nft::sign_nft_spends(&spends, std::slice::from_ref(&alice.sk), true)?;
        sim.new_transaction(chia_protocol::SpendBundle::new(spends, sig))?;

        // The 7-mojo change (10 funded - 3 needed) landed back at alice's own address — proving the
        // excess was NOT silently burned as network fee.
        let change_coins: Vec<_> = sim
            .children(funding.coin_id())
            .into_iter()
            .filter(|cs| cs.coin.puzzle_hash == alice.puzzle_hash && cs.coin.amount == 7)
            .collect();
        assert_eq!(change_coins.len(), 1, "7-mojo change coin should exist");

        let _ = did_launcher;
        Ok(())
    }

    // ---------- #231: cost-bounded auto-batching ----------

    /// The cost budget is a strict fraction of the block ceiling, the default batch size is derived
    /// from the cost model (never a hard-coded 25), and a default-sized batch's estimate stays within
    /// budget while a hugely-oversized size does not.
    #[test]
    fn cost_model_default_batch_size_fits_budget() {
        assert_eq!(batch_cost_budget(), MAX_BLOCK_COST_CLVM / 4);
        let n = default_batch_size();
        assert!(n >= 1, "at least one item per batch");
        assert!(
            estimate_batch_cost(n) <= batch_cost_budget(),
            "a default-sized batch ({n}) must fit the budget: est {} > budget {}",
            estimate_batch_cost(n),
            batch_cost_budget()
        );
        // One more item than the default would exceed the budget (the default is the LARGEST that fits).
        assert!(
            estimate_batch_cost(n + 1) > batch_cost_budget(),
            "default batch size {n} must be the largest that fits the budget"
        );
        // Sanity: the computed default is in a realistic range (dkackman's prior tooling used ~25).
        assert!(
            (10..=80).contains(&n),
            "computed default batch size {n} out of the expected realistic range"
        );
    }

    /// `estimate_batch_cost` is monotone and the base+per-item model composes as documented.
    #[test]
    fn estimate_batch_cost_is_monotone() {
        assert_eq!(estimate_batch_cost(0), EST_BATCH_BASE_COST_CLVM);
        assert_eq!(
            estimate_batch_cost(10),
            EST_BATCH_BASE_COST_CLVM + 10 * EST_COST_PER_ITEM_CLVM
        );
        assert!(estimate_batch_cost(50) > estimate_batch_cost(49));
    }

    /// `plan_batches` splits N items into contiguous ranges that cover `0..N` exactly, none larger
    /// than the chosen size, using the cost-derived default when no override is given.
    #[test]
    fn plan_batches_covers_all_items_contiguously() {
        // Default size: a 200-item mint splits into >1 batch, each <= default_batch_size.
        let n = 200usize;
        let plan = plan_batches(n, None).unwrap();
        assert!(plan.len() > 1, "200 items must need more than one batch");
        let size = default_batch_size();
        // Contiguous, gapless, covering exactly 0..n.
        let mut expected_start = 0;
        for r in &plan {
            assert_eq!(r.start, expected_start, "ranges must be contiguous");
            assert!(
                r.end > r.start && r.end - r.start <= size,
                "range within batch size"
            );
            expected_start = r.end;
        }
        assert_eq!(expected_start, n, "ranges must cover every item");
        let total: usize = plan.iter().map(|r| r.end - r.start).sum();
        assert_eq!(total, n);

        // Explicit override is honoured.
        let plan = plan_batches(10, Some(4)).unwrap();
        assert_eq!(
            plan,
            vec![0..4, 4..8, 8..10],
            "explicit --batch-size 4 splits 10 into 4+4+2"
        );

        // A single item is one batch.
        assert_eq!(plan_batches(1, None).unwrap(), vec![0..1]);
        // Zero items is an error.
        assert!(plan_batches(0, None).is_err());
    }

    /// `validate_batch_size` rejects 0 and a size whose estimated cost exceeds the budget, the latter
    /// with the terminal [`ChainError::BundleTooLarge`] (so the CLI never mislabels it a coinset
    /// connectivity problem).
    #[test]
    fn validate_batch_size_rejects_zero_and_oversized() {
        assert!(validate_batch_size(0).is_err());
        assert!(validate_batch_size(default_batch_size()).is_ok());
        let huge = default_batch_size() * 100;
        let err = validate_batch_size(huge).unwrap_err();
        assert!(
            matches!(&err, ChainError::BundleTooLarge(m) if m.contains("too large")),
            "oversized --batch-size must be terminal BundleTooLarge, got: {err}"
        );
    }

    /// THE #231 cost-bound PROOF: `EST_COST_PER_ITEM_CLVM` / `EST_BATCH_BASE_COST_CLVM` are
    /// CONSERVATIVE — the real CLVM cost of a batch (measured with the Chia consensus cost model,
    /// `run_spendbundle`, against MAINNET_CONSTANTS) never exceeds our estimate, for two batch sizes.
    /// This is what lets us pack batches under the block limit by ESTIMATE alone (no per-build CLVM
    /// run at mint time). If a future change makes an item more expensive than the constant assumes,
    /// this test fails, forcing the constant back up.
    #[test]
    fn est_cost_per_item_is_conservative() -> anyhow::Result<()> {
        use chia_consensus::flags::MEMPOOL_MODE;
        use chia_consensus::owned_conditions::OwnedSpendBundleConditions;
        use chia_consensus::spendbundle_conditions::run_spendbundle;
        use chia_consensus::spendbundle_validation::get_flags_for_height_and_constants;
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::Launcher;

        // A recent mainnet height (the `hard_fork2_height` argument is inert in both SDK versions).
        const HEIGHT: u32 = 6_000_000;

        // chia-consensus 0.36.1 dropped `run_spendbundle`'s `prev_tx_height` parameter: the
        // caller now composes the flags the old signature derived internally from the height.
        // `get_flags_for_height_and_constants` only sets bits at `height >= hard_fork2_height`,
        // which mainnet has NOT reached, so the height contributes nothing today and the
        // measurement runs under plain mempool rules. Pinning that keeps the bound honest: if
        // hard fork 2 activates below HEIGHT the flags change, the cost model changes with them,
        // and this assertion fails rather than letting the proof drift onto a different model.
        let height_flags =
            get_flags_for_height_and_constants(HEIGHT, &chia_sdk_types::MAINNET_CONSTANTS);
        assert_eq!(
            height_flags, 0,
            concat!(
                "mainnet hard fork 2 is now active at or below height {}; ",
                "re-verify the cost bound under the new flags"
            ),
            HEIGHT
        );
        let softfork_flags = height_flags;

        // Measure a bundle's real total CLVM cost under the mainnet consensus cost model.
        let measure = |bundle: &chia_protocol::SpendBundle| -> u64 {
            let mut a = clvmr::Allocator::new();
            let (sbc, _pk) = run_spendbundle(
                &mut a,
                bundle,
                MAX_BLOCK_COST_CLVM,
                softfork_flags | MEMPOOL_MODE,
                &chia_sdk_types::MAINNET_CONSTANTS,
            )
            .expect("batch bundle must validate under the consensus cost model");
            OwnedSpendBundleConditions::from(&a, sbc).cost
        };

        // Create + apply a DID so we can build real batches against its eve generation.
        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let alice = sim.bls(1);
        let alice_p2 = StandardLayer::new(alice.pk);
        let (create_did, did) =
            Launcher::new(alice.coin.coin_id(), 1).create_simple_did(ctx, &alice_p2)?;
        alice_p2.spend(ctx, alice.coin, create_did)?;
        let create_spends = ctx.take();
        let sig =
            crate::nft::sign_nft_spends(&create_spends, std::slice::from_ref(&alice.sk), true)?;
        sim.new_transaction(chia_protocol::SpendBundle::new(create_spends, sig))?;

        let keys = crate::keys::IndexedKeys {
            index: 0,
            synthetic_sk: alice.sk.clone(),
            synthetic_pk: alice.pk,
            owner_puzzle_hash: alice.puzzle_hash,
        };
        let col = collection();

        // Build two batch sizes against the SAME eve DID (not applied — we only measure cost).
        let mut build_cost = |n: usize| -> anyhow::Result<u64> {
            let funding = sim.new_coin(alice.puzzle_hash, 1_000);
            let batch = build_collection_batch(
                &keys,
                did,
                &col,
                &items_n(n),
                alice.puzzle_hash,
                funding,
                &keys,
            )?;
            let sig = crate::nft::sign_nft_spends(
                &batch.coin_spends,
                std::slice::from_ref(&alice.sk),
                true,
            )?;
            Ok(measure(&chia_protocol::SpendBundle::new(
                batch.coin_spends,
                sig,
            )))
        };

        let (n1, n2) = (5usize, 20usize);
        let cost1 = build_cost(n1)?;
        let cost2 = build_cost(n2)?;

        // Our whole-batch estimate must bound the measured cost at both sizes.
        assert!(
            estimate_batch_cost(n1) >= cost1,
            "estimate for {n1} items ({}) must be >= measured ({cost1})",
            estimate_batch_cost(n1)
        );
        assert!(
            estimate_batch_cost(n2) >= cost2,
            "estimate for {n2} items ({}) must be >= measured ({cost2})",
            estimate_batch_cost(n2)
        );

        // The measured MARGINAL per-item cost must not exceed our per-item constant.
        let marginal = (cost2 - cost1) / (n2 - n1) as u64;
        assert!(
            EST_COST_PER_ITEM_CLVM >= marginal,
            "EST_COST_PER_ITEM_CLVM ({EST_COST_PER_ITEM_CLVM}) must be >= measured marginal \
             per-item cost ({marginal}); raise the constant"
        );

        // And a default-sized batch's measured cost stays under the mainnet block limit with margin.
        assert!(
            estimate_batch_cost(default_batch_size()) < MAX_BLOCK_COST_CLVM,
            "a default-sized batch must be under the block limit"
        );
        Ok(())
    }

    /// THE #231 batching PROOF: a multi-batch collection mint chains across batches on the Simulator
    /// — each batch is a SEPARATE, self-contained, cost-bounded bundle that validates under consensus
    /// (cost + value conservation), the DID advances one generation per batch (the next batch spends
    /// the recreated DID), and EVERY item across all batches is minted with a distinct launcher id.
    /// A small `--batch-size` forces several batches from a modest item set (a real cost-bounded
    /// 200-item run would take far longer on the sim; the chaining logic is identical).
    #[test]
    fn build_collection_batch_chains_across_batches_on_simulator() -> anyhow::Result<()> {
        use chia_sdk_test::Simulator;
        use chia_wallet_sdk::driver::Launcher;

        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        // Create + apply the DID; each batch below is its own transaction against the current DID.
        let alice = sim.bls(1);
        let alice_p2 = StandardLayer::new(alice.pk);
        let (create_did, mut did) =
            Launcher::new(alice.coin.coin_id(), 1).create_simple_did(ctx, &alice_p2)?;
        alice_p2.spend(ctx, alice.coin, create_did)?;
        let create_spends = ctx.take();
        let sig =
            crate::nft::sign_nft_spends(&create_spends, std::slice::from_ref(&alice.sk), true)?;
        sim.new_transaction(chia_protocol::SpendBundle::new(create_spends, sig))?;

        let keys = crate::keys::IndexedKeys {
            index: 0,
            synthetic_sk: alice.sk.clone(),
            synthetic_pk: alice.pk,
            owner_puzzle_hash: alice.puzzle_hash,
        };
        let col = collection();

        let total = 7usize;
        let batch_size = 3usize;
        let items = items_n(total);
        let plan = plan_batches(total, Some(batch_size))?;
        assert!(
            plan.len() >= 3,
            "a batch size of 3 over 7 items yields >= 3 batches"
        );

        let mut all_launchers: Vec<Bytes32> = Vec::new();
        for range in plan {
            let n = range.len();
            // On chain this is the wallet's next/change coin; on the sim we mint a fresh funding coin.
            let funding = sim.new_coin(alice.puzzle_hash, 1_000);
            let batch = build_collection_batch(
                &keys,
                did,
                &col,
                &items[range.clone()],
                alice.puzzle_hash,
                funding,
                &keys,
            )?;
            assert_eq!(
                batch.launcher_ids.len(),
                n,
                "batch mints every item in its range"
            );
            // Apply the batch as its OWN bundle — consensus validates its cost + value conservation.
            let sig = crate::nft::sign_nft_spends(
                &batch.coin_spends,
                std::slice::from_ref(&alice.sk),
                true,
            )?;
            sim.new_transaction(chia_protocol::SpendBundle::new(
                batch.coin_spends.clone(),
                sig,
            ))?;
            all_launchers.extend(batch.launcher_ids);
            // Chain: the next batch spends the DID this batch recreated.
            did = batch.next_did;
        }

        assert_eq!(
            all_launchers.len(),
            total,
            "every item minted across the batches"
        );
        let uniq: std::collections::HashSet<_> = all_launchers.iter().collect();
        assert_eq!(
            uniq.len(),
            total,
            "all launcher ids are distinct across batches"
        );
        Ok(())
    }

    // ---------- #231: resumable mint progress ----------

    fn progress_with(n_batches_confirmed: usize) -> MintProgress {
        let mut p = MintProgress::new("c", "ab".repeat(32), "deadbeef", 30, 10);
        for i in 0..3 {
            let (start, end) = (i * 10, i * 10 + 10);
            p.record_pending(
                start,
                end,
                format!("{:064x}", i),
                format!("{:064x}", i + 100),
                format!("{:064x}", i + 200),
                vec![format!("{:064x}", i + 300)],
            );
            if i < n_batches_confirmed {
                p.confirm(start, end);
            }
        }
        p
    }

    #[test]
    fn mint_progress_tracks_confirmed_and_pending() {
        let p = progress_with(2);
        assert_eq!(p.minted_count(), 20, "two confirmed 10-item batches");
        assert!(p.is_confirmed(0, 10) && p.is_confirmed(10, 20));
        assert!(!p.is_confirmed(20, 30), "third batch is still pending");
        // Only the tail (last, unconfirmed) batch is in-flight.
        let tail = p.pending_tail().expect("a pending tail exists");
        assert_eq!((tail.start, tail.end), (20, 30));
        // A fully-confirmed record has no pending tail.
        let done = progress_with(3);
        assert!(done.pending_tail().is_none());
        assert_eq!(done.minted_count(), 30);
    }

    #[test]
    fn mint_progress_record_pending_is_idempotent() {
        let mut p = MintProgress::new("c", "did", "mh", 20, 10);
        p.record_pending(
            0,
            10,
            "d0".into(),
            "n0".into(),
            "t0".into(),
            vec!["l0".into()],
        );
        p.record_pending(
            0,
            10,
            "d0".into(),
            "n0".into(),
            "t0b".into(),
            vec!["l0".into()],
        );
        assert_eq!(p.batches.len(), 1, "re-recording a range updates in place");
        assert_eq!(
            p.batches[0].tx_id, "t0b",
            "the record is updated, not duplicated"
        );
        p.confirm(0, 10);
        assert!(p.is_confirmed(0, 10));
    }

    #[test]
    fn mint_progress_matches_guards_stale_records() {
        let p = MintProgress::new("dig-punks", "abcd", "hash1", 5, 2);
        assert!(p.matches("dig-punks", "abcd", "hash1"));
        assert!(!p.matches("other", "abcd", "hash1"), "different collection");
        assert!(!p.matches("dig-punks", "ffff", "hash1"), "different DID");
        assert!(
            !p.matches("dig-punks", "abcd", "hash2"),
            "different manifest"
        );
    }

    #[test]
    fn manifest_fingerprint_is_stable_and_content_addressed() {
        let a = manifest_fingerprint(b"[{\"name\":\"A\"}]");
        let b = manifest_fingerprint(b"[{\"name\":\"A\"}]");
        let c = manifest_fingerprint(b"[{\"name\":\"B\"}]");
        assert_eq!(a, b, "same bytes -> same fingerprint");
        assert_ne!(a, c, "different bytes -> different fingerprint");
        assert_eq!(a.len(), 64, "sha256 hex is 64 chars");
    }
}
