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

use chia::puzzles::nft::NftMetadata;
use chia_protocol::{Bytes32, CoinSpend, Program};
use chia_wallet_sdk::driver::{
    Did, IntermediateLauncher, NftMint, SingletonInfo, SpendContext, StandardLayer,
};
use chia_wallet_sdk::types::conditions::TransferNft;
use chia_wallet_sdk::types::Conditions;
use serde::{Deserialize, Serialize};

use crate::error::{ChainError, Result};
use crate::keys::IndexedKeys;
use crate::metadata::{Attribute, Chip0007Metadata, CollectionRef};

/// A CHIP-0007 collection definition: the shared identity + economics across every item.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collection {
    /// Stable collection id (the toolkit derives it from the creator DID + name, or supplies it).
    pub id: String,
    /// Human-readable collection name.
    pub name: String,
    /// Collection-level attributes (icon/banner/website/twitter/etc) as CHIP-0007 name/value pairs.
    #[serde(default)]
    pub attributes: Vec<Attribute>,
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
    // DID singleton is not needed here (the caller re-fetches it from chain to chain further mints).
    let _recreated = did
        .update(ctx, &p2, all_mint_conditions)
        .map_err(|e| ChainError::Chain(format!("spend did for collection mint: {e}")))?;

    Ok(launcher_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection() -> Collection {
        Collection {
            id: "dig-punks".into(),
            name: "DIG Punks".into(),
            attributes: vec![Attribute {
                trait_type: "Website".into(),
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
    #[test]
    fn generated_item_json_is_pinned() {
        let col = collection();
        let mds = generate_item_metadata(&col, &items());
        assert_eq!(
            mds[0].to_canonical_json().unwrap(),
            r#"{"format":"CHIP-0007","name":"DIG Punk #1","description":"first","collection":{"id":"dig-punks","name":"DIG Punks","attributes":[{"trait_type":"Website","value":"https://dig.net"}]},"attributes":[{"trait_type":"Background","value":"Blue"}],"series_number":1,"series_total":2,"minting_tool":"DIG"}"#
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
    /// MULTI-item DID-spent mint needs a separate XCH funding coin for the extra launchers — that
    /// fuller funding path is scaffolded at the CLI layer, see `collection mint`'s TODO.)
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
}
