//! Public, owner-independent NFT-collection READ/INDEX over coinset (roadmap #39).
//!
//! [`crate::nft::read_collection`] answers "which of MY NFTs belong to this collection"
//! — it enumerates the WALLET's hinted coins and filters by the attributed DID. That is a
//! wallet view, not a public one: it can only see items the querying wallet currently holds.
//!
//! This module is the **public** read: given a collection's authoritative item set (the NFT
//! launcher ids the mint produced) it resolves EACH item to its CURRENT on-chain state —
//! current owner, royalty, and CHIP-0007 metadata — regardless of who holds it now, using
//! nothing but coinset reads (NO third-party indexer). It is the platform owning its own
//! collection reads (paper §; SYSTEM.md → the read path is DIG's, not MintGarden's).
//!
//! ## Why launcher ids are the discovery anchor (the design decision)
//! A DID-attributed NFT records its creator DID only in its OWNERSHIP layer
//! (`NftInfo::current_owner`), NOT as a coin hint: chia-wallet-sdk hints the minted NFT's eve
//! coin to the OWNER p2 puzzle hash (`nft_launcher.rs` `ctx.hint(mint.p2_puzzle_hash)`), not
//! to the DID. So `get_coin_records_by_hint(did_launcher)` does NOT return a collection's
//! items, and a public reader cannot enumerate "every owner" to find them. The robust,
//! deterministic anchor is therefore the **launcher id**: the bulk-mint builder
//! ([`crate::collection::build_collection_mint`]) returns exactly the launcher ids it created,
//! so the minting client (hub / CLI) holds the authoritative item set and pins it. The index
//! resolves each launcher to its live state by walking the singleton lineage FORWARD from the
//! eve coin to the current unspent tip — owner-independent and stable.
//!
//! (A fuller "discover the launcher set from the creator DID alone" path would walk the DID
//! singleton's spend lineage and hand-parse each DID spend's CREATE_COIN outputs down through
//! the intermediate launchers; that has no clean SDK helper and is a later enhancement. The
//! explicit-launcher path here needs no such hand-rolled puzzle execution and is the reliable
//! contract the RPC exposes today.)
//!
//! ## Forward lineage walk
//! A singleton (NFT) coin is created by SPENDING its parent. So from any coin in the lineage:
//! read its `coin_record`; if SPENT, fetch its spend and run [`Nft::parse_child`] to get the
//! single child coin; repeat until `coin_record` reports the coin UNSPENT — that is the tip.
//! This is the same `parse_child` step [`crate::nft::reconstruct_nft_in`] uses for one hop,
//! iterated forward. Pure reads through the [`ChainReads`] trait, so the whole walk is
//! testable against the in-process Chia [`Simulator`](chia_sdk_test::Simulator) via the mock.

use crate::coinset::ChainReads;
use crate::error::{ChainError, Result};
use chia::puzzles::nft::NftMetadata;
use chia_protocol::{Bytes32, Coin};
use chia_wallet_sdk::driver::{Nft, Puzzle, SpendContext};

/// One collection item resolved to its CURRENT on-chain state — the public, owner-independent
/// view the read RPC returns. Unlike [`crate::nft::OwnedNft`] (a spendable wallet record) this
/// is a pure READ projection: it carries the flattened display fields PLUS the decoded
/// CHIP-0007 on-chain [`NftMetadata`], and is owned by no particular wallet.
#[derive(Clone, Debug)]
pub struct IndexedNft {
    /// The launcher coin id — the NFT's stable identity (its `nft1…` id encodes this).
    pub launcher_id: Bytes32,
    /// The current (unspent) coin id of the NFT singleton — its live tip.
    pub coin_id: Bytes32,
    /// The current assigned owner: a DID launcher id when attributed, else `None`
    /// (an NFT held by a plain wallet after a transfer/offer unassigns the DID).
    pub owner_did: Option<Bytes32>,
    /// The puzzle hash royalties are paid to in offer trades.
    pub royalty_puzzle_hash: Bytes32,
    /// Royalty as hundredths of a percent (300 = 3%).
    pub royalty_basis_points: u16,
    /// The CURRENT owner (p2) puzzle hash — where the NFT lives now.
    pub owner_puzzle_hash: Bytes32,
    /// The decoded on-chain CHIP-0007 metadata (data/metadata/license URIs + hashes, edition).
    /// `None` when the metadata pointer does not decode as `NftMetadata` (a custom/odd metadata
    /// updater) — the item is still surfaced, just without typed metadata.
    pub metadata: Option<NftMetadata>,
}

/// Collection-level facts derived from a resolved item set — what `dig.getCollection` returns
/// alongside the per-item list. Computed purely from the items' current on-chain state.
#[derive(Clone, Debug)]
pub struct CollectionIndex {
    /// The collection's creator DID launcher id, if the items agree on one (every item is
    /// attributed to the same DID). `None` when items disagree or none are DID-attributed.
    pub did: Option<Bytes32>,
    /// How many items were resolved (launchers that resolved to a live NFT).
    pub item_count: usize,
    /// The shared royalty in basis points if every item agrees, else `None`.
    pub royalty_basis_points: Option<u16>,
}

/// Resolve ONE NFT launcher id to its current on-chain state (its live tip), or `Ok(None)` if
/// the launcher does not resolve to an NFT on `chain` (never minted, not yet confirmed, or not
/// an NFT launcher).
///
/// Walk (all via `ChainReads`, no hand-rolled puzzle execution):
/// 1. The launcher coin is spent in the mint bundle to create the EVE coin; the eve is its
///    single child — find it with [`child_of`] (`coin_records_by_parent_ids([launcher_id])`).
///    The launcher puzzle is the singleton launcher (NOT an NFT), so we cannot `parse_child`
///    the launcher itself — we locate the eve by parentage instead.
/// 2. The eve coin is itself spent immediately (the mint spends it to create the first owned
///    NFT), and the eve's puzzle IS an NFT singleton, so [`Nft::parse_child`] of the eve's spend
///    yields the first real NFT — the entry point to the typed lineage.
/// 3. From there [`walk_to_tip`] follows the singleton forward (each coin's child via
///    `coin_records_by_parent_ids`, each hop via `parse_child`) to the current UNSPENT coin.
///
/// Finally we flatten the live `NftInfo` and decode its CHIP-0007 [`NftMetadata`].
pub async fn resolve_nft_by_launcher(
    chain: &dyn ChainReads,
    launcher_id: Bytes32,
) -> Result<Option<IndexedNft>> {
    let mut ctx = SpendContext::new();

    // 1. The launcher must be present + spent (a launcher is always spent in its mint bundle).
    let Some(launcher_rec) = chain.coin_record(launcher_id).await? else {
        return Ok(None);
    };
    if !launcher_rec.spent {
        return Ok(None);
    }

    // 2. The eve coin is the launcher's single child. Parse the eve coin's OWN spend (an NFT
    //    singleton spend) into the first real NFT.
    let Some(eve_rec) = child_of(chain, launcher_id).await? else {
        return Ok(None);
    };
    if !eve_rec.spent {
        // An eve that was never spent is a degenerate/incomplete mint — no owned NFT to report.
        return Ok(None);
    }
    let Some(first) =
        parse_child_of(&mut ctx, chain, eve_rec.coin, eve_rec.spent_block_index).await?
    else {
        return Ok(None);
    };

    // 3. Walk forward from the first NFT to the current unspent tip.
    let tip = walk_to_tip(&mut ctx, chain, first).await?;

    // Decode the CHIP-0007 on-chain metadata (best-effort: a non-standard metadata updater may
    // not decode as NftMetadata — surface the item without typed metadata rather than failing).
    let metadata = ctx.extract::<NftMetadata>(tip.info.metadata.ptr()).ok();

    Ok(Some(IndexedNft {
        launcher_id: tip.info.launcher_id,
        coin_id: tip.coin.coin_id(),
        owner_did: tip.info.current_owner,
        royalty_puzzle_hash: tip.info.royalty_puzzle_hash,
        royalty_basis_points: tip.info.royalty_basis_points,
        owner_puzzle_hash: tip.info.p2_puzzle_hash,
        metadata,
    }))
}

/// Resolve a whole collection's item set (the NFT launcher ids the mint produced) to their
/// current on-chain state, in the SAME order as `launcher_ids` (deterministic). Launchers that
/// do not resolve to a live NFT are skipped (so a partially-confirmed mint still indexes).
///
/// This is the per-item body behind `dig.listCollectionItems`; the RPC applies offset/limit
/// over the input launcher ids before calling so only the requested page is resolved.
pub async fn index_collection_items(
    chain: &dyn ChainReads,
    launcher_ids: &[Bytes32],
) -> Result<Vec<IndexedNft>> {
    let mut out = Vec::with_capacity(launcher_ids.len());
    for &launcher_id in launcher_ids {
        if let Some(item) = resolve_nft_by_launcher(chain, launcher_id).await? {
            out.push(item);
        }
    }
    Ok(out)
}

/// Derive the collection-level facts (`dig.getCollection`) from a resolved item set: the shared
/// creator DID (if uniform), the item count, and the uniform royalty (if uniform).
pub fn summarize_collection(items: &[IndexedNft]) -> CollectionIndex {
    CollectionIndex {
        did: uniform(items.iter().map(|i| i.owner_did)).flatten(),
        item_count: items.len(),
        royalty_basis_points: uniform(items.iter().map(|i| i.royalty_basis_points)),
    }
}

/// The single value shared across `vals` if they all agree (and there is at least one), else
/// `None`. Used for the uniform-DID and uniform-royalty derivations.
fn uniform<T: PartialEq + Copy>(mut vals: impl Iterator<Item = T>) -> Option<T> {
    let first = vals.next()?;
    if vals.all(|v| v == first) {
        Some(first)
    } else {
        None
    }
}

/// Walk a singleton's lineage FORWARD from `nft` to its current unspent tip.
///
/// Each step looks up the current coin's CHILD ([`child_of`]). If there is no child, the current
/// coin is the live unspent tip — return it. If there IS a child, the current coin was spent, so
/// we [`parse_child_of`] its OWN spend to recover the typed next NFT and continue. A spent coin
/// whose spend yields no NFT child (it left the NFT puzzle family — e.g. melted/burned) ends the
/// walk at the last NFT state we held.
///
/// Bounded by `MAX_LINEAGE_HOPS` so a malformed/cyclic chain can never loop forever (a real NFT
/// lineage is short — a handful of transfers).
async fn walk_to_tip(ctx: &mut SpendContext, chain: &dyn ChainReads, mut nft: Nft) -> Result<Nft> {
    const MAX_LINEAGE_HOPS: usize = 10_000;
    for _ in 0..MAX_LINEAGE_HOPS {
        let coin_id = nft.coin.coin_id();
        let Some(child_rec) = child_of(chain, coin_id).await? else {
            return Ok(nft); // no child → the live unspent tip
        };
        // The current coin has a child, so it was spent. Recover the typed next NFT from this
        // coin's own spend (its puzzle is an NFT singleton, so parse_child applies).
        match parse_child_of(ctx, chain, nft.coin, child_rec.confirmed_block_index).await? {
            Some(next) => nft = next,
            // A child exists on-chain but the spend doesn't parse as an NFT child (left the
            // family / odd spend) — the coin we held was the last NFT state.
            None => return Ok(nft),
        }
    }
    Err(ChainError::Chain(format!(
        "NFT lineage exceeded {MAX_LINEAGE_HOPS} hops resolving launcher {:?} (malformed chain)",
        nft.info.launcher_id
    )))
}

/// The single child of `parent_coin_id` (a singleton has exactly one child per generation), or
/// `Ok(None)` when the coin has no child yet (it is the unspent tip). Queries
/// `coin_records_by_parent_ids` including spent children (a mid-lineage child is itself spent).
async fn child_of(
    chain: &dyn ChainReads,
    parent_coin_id: Bytes32,
) -> Result<Option<crate::coinset::CoinRecord>> {
    let children = chain
        .coin_records_by_parent_ids(&[parent_coin_id], true)
        .await?;
    Ok(children.into_iter().next())
}

/// Fetch the spend of `parent` (which was spent at `spent_height`) and parse it into its single
/// child [`Nft`] in `ctx`. Returns `Ok(None)` if the spend is unavailable or does not yield an
/// NFT child. Shared by the eve→first-NFT step and each forward hop.
async fn parse_child_of(
    ctx: &mut SpendContext,
    chain: &dyn ChainReads,
    parent: Coin,
    spent_height: u32,
) -> Result<Option<Nft>> {
    let Some(spend) = chain.coin_spend(parent.coin_id(), spent_height).await? else {
        return Ok(None);
    };
    let puzzle_ptr = ctx
        .alloc(&spend.puzzle_reveal)
        .map_err(|e| ChainError::Chain(format!("alloc parent puzzle: {e}")))?;
    let puzzle = Puzzle::parse(ctx, puzzle_ptr);
    let solution = ctx
        .alloc(&spend.solution)
        .map_err(|e| ChainError::Chain(format!("alloc parent solution: {e}")))?;
    Nft::parse_child(ctx, spend.coin, puzzle, solution)
        .map_err(|e| ChainError::Chain(format!("Nft::parse_child: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::coinset::CoinInfo;
    use crate::keys::derive_indexed_keys;
    use chia_protocol::SpendBundle;
    use chia_sdk_test::Simulator;
    use chia_wallet_sdk::driver::{
        IntermediateLauncher, Launcher, NftMint, SingletonInfo, SpendContext, StandardLayer,
    };
    use chia_wallet_sdk::types::conditions::TransferNft;

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    /// Seed a [`MockChain`] from the simulator for the given coin IDS, mirroring what coinset
    /// returns so the forward lineage walk can resolve a launcher to its live tip purely from
    /// these reads: a `coin_record` per coin (its full `Coin`, spent flag, and the height it was
    /// spent in `spent_block_index`) and a `coin_spend` per spent coin (keyed by that coin's own
    /// id, since the walk fetches a coin's OWN spend to find its child). Each id's full `Coin` +
    /// state is pulled from the simulator's `coin_state`.
    fn mock_from_sim(sim: &Simulator, coin_ids: &[Bytes32]) -> MockChain {
        let mut mock = MockChain::default();
        for &coin_id in coin_ids {
            let Some(cs) = sim.coin_state(coin_id) else {
                continue;
            };
            let spent_h = cs.spent_height;
            mock.records.insert(
                coin_id,
                CoinInfo {
                    coin: cs.coin,
                    spent: spent_h.is_some(),
                    confirmed_block_index: cs.created_height.unwrap_or(0),
                    // The height THIS coin was spent (the walk passes it to coin_spend; the mock
                    // ignores the height and keys by coin id, matching the real impl's contract).
                    spent_block_index: spent_h.unwrap_or(0),
                    timestamp: 0,
                    coinbase: false,
                },
            );
            // A spent coin's own spend (its puzzle+solution) lets parse_child_of walk to the
            // child. The mock keys spends by the SPENT coin's id (see MockChain::coin_spend).
            if spent_h.is_some() {
                if let Some(spend) = sim.coin_spend(coin_id) {
                    mock.spends.insert(coin_id, spend);
                }
            }
        }
        mock
    }

    /// Mint a DID-attributed NFT for `alice` on `sim` (the validated #38 shape) and return its
    /// eve NFT. Shared setup for the resolve tests.
    fn mint_did_nft(
        sim: &mut Simulator,
        alice: &chia_sdk_test::BlsPairWithCoin,
        royalty_bp: u16,
    ) -> anyhow::Result<Nft> {
        let ctx = &mut SpendContext::new();
        let alice_p2 = StandardLayer::new(alice.pk);
        let (create_did, did) =
            Launcher::new(alice.coin.coin_id(), 1).create_simple_did(ctx, &alice_p2)?;
        alice_p2.spend(ctx, alice.coin, create_did)?;
        let did_launcher = did.info.launcher_id;

        let metadata = ctx.serialize(&NftMetadata {
            data_uris: vec!["dig://store/punk.png".to_string()],
            data_hash: Some(Bytes32::from([0x44; 32])),
            metadata_uris: vec!["dig://store/punk.json".to_string()],
            metadata_hash: Some(Bytes32::from([0x55; 32])),
            ..Default::default()
        })?;
        let metadata_ptr = ctx.alloc_hashed(&metadata)?;
        let transfer = TransferNft::new(
            Some(did_launcher),
            Vec::new(),
            Some(did.info.inner_puzzle_hash().into()),
        );
        let nft_mint = NftMint::new(metadata_ptr, alice.puzzle_hash, royalty_bp, Some(transfer));
        let (mint_conditions, eve_nft) = IntermediateLauncher::new(did.coin.coin_id(), 0, 1)
            .create(ctx)?
            .mint_nft(ctx, &nft_mint)?;
        let _ = did.update(ctx, &alice_p2, mint_conditions)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&alice.sk))?;
        Ok(eve_nft)
    }

    /// #39 RED→GREEN: a freshly-minted DID-attributed 3% NFT resolves BY LAUNCHER ID (an
    /// owner-independent anchor) to the minter as its current owner, with the royalty and the
    /// CHIP-0007 metadata decoded — purely from coinset reads. `summarize_collection` derives
    /// the collection's DID + uniform royalty from the single item.
    #[tokio::test]
    async fn resolve_freshly_minted_nft_by_launcher() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let alice = sim.bls(2);
        let eve = mint_did_nft(&mut sim, &alice, 300)?;
        let launcher_id = eve.info.launcher_id;
        let did_launcher = eve.info.current_owner.expect("minted with a DID");

        // Resolve over a mock seeded with the launcher coin + the eve coin (both confirmed on
        // sim). The launcher is spent (it minted the eve); the eve is the unspent tip.
        // Lineage on chain: launcher_id → eve coin → `eve` (the minter-owned NFT the mint
        // returns). The launcher is spent (it launched the eve); the eve is spent (the mint
        // spends it to create the minter NFT); `eve.coin` is the current unspent tip. Seed all
        // three so the resolver can walk launcher → eve-child → tip.
        let mock = mock_from_sim(
            &sim,
            &[launcher_id, eve.coin.parent_coin_info, eve.coin.coin_id()],
        );
        let item = resolve_nft_by_launcher(&mock, launcher_id)
            .await?
            .expect("launcher resolves to a live NFT");

        assert_eq!(item.launcher_id, launcher_id);
        assert_eq!(item.owner_did, Some(did_launcher), "attributed to the DID");
        assert_eq!(item.royalty_basis_points, 300);
        assert_eq!(
            item.owner_puzzle_hash, alice.puzzle_hash,
            "current owner is the minter (no transfer yet)"
        );
        let md = item.metadata.as_ref().expect("CHIP-0007 metadata decodes");
        assert_eq!(md.data_uris, vec!["dig://store/punk.png".to_string()]);
        assert_eq!(md.data_hash, Some(Bytes32::from([0x44; 32])));

        // Collection-level summary from this one item.
        let summary = summarize_collection(std::slice::from_ref(&item));
        assert_eq!(summary.item_count, 1);
        assert_eq!(summary.did, Some(did_launcher));
        assert_eq!(summary.royalty_basis_points, Some(300));
        Ok(())
    }

    /// #39 the core public-read claim: after the NFT is TRANSFERRED to a new owner, resolving
    /// the SAME launcher id follows the singleton lineage forward and reports the CURRENT owner
    /// (the recipient), not the minter. This is exactly what the wallet-scoped
    /// `read_collection` cannot do (it only sees the querying wallet's holdings).
    #[tokio::test]
    async fn resolve_follows_lineage_to_current_owner_after_transfer() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let alice = sim.bls(2);
        let eve = mint_did_nft(&mut sim, &alice, 300)?;
        let launcher_id = eve.info.launcher_id;

        // A fresh recipient address (bob). Transfer the eve NFT alice → bob and apply it.
        let bob_ph = derive_indexed_keys(ABANDON, 0..1)?[0].owner_puzzle_hash;
        let xfer_ctx = &mut SpendContext::new();
        let alice_p2 = StandardLayer::new(alice.pk);
        // Re-parse the eve NFT in this ctx from its launcher spend (allocator-relative metadata).
        let launcher_spend = sim
            .coin_spend(eve.coin.parent_coin_info)
            .expect("eve parent (launcher) spend recorded");
        let pp = xfer_ctx.alloc(&launcher_spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(xfer_ctx, pp);
        let sol = xfer_ctx.alloc(&launcher_spend.solution)?;
        let eve_in_ctx = Nft::parse_child(xfer_ctx, launcher_spend.coin, puzzle, sol)?
            .expect("eve NFT parses from launcher spend");
        let child = eve_in_ctx.transfer(
            xfer_ctx,
            &alice_p2,
            bob_ph,
            chia_wallet_sdk::types::Conditions::new(),
        )?;
        let xfer_spends = xfer_ctx.take();
        let sig = crate::nft::sign_nft_spends(&xfer_spends, std::slice::from_ref(&alice.sk), true)?;
        sim.new_transaction(SpendBundle::new(xfer_spends, sig))?;

        // Seed the full lineage: launcher → eve coin → minter NFT (now spent) → bob NFT (tip).
        let mock = mock_from_sim(
            &sim,
            &[
                launcher_id,
                eve.coin.parent_coin_info, // the eve coin
                eve.coin.coin_id(),        // the minter NFT (now spent by the transfer)
                child.coin.coin_id(),      // bob's NFT — the new tip
            ],
        );
        let item = resolve_nft_by_launcher(&mock, launcher_id)
            .await?
            .expect("launcher still resolves after transfer");

        assert_eq!(
            item.launcher_id, launcher_id,
            "stable identity across transfer"
        );
        assert_eq!(
            item.owner_puzzle_hash, bob_ph,
            "the lineage walk reports the CURRENT owner (bob), not the minter"
        );
        assert_eq!(
            item.coin_id,
            child.coin.coin_id(),
            "tip is the post-transfer coin"
        );
        assert_eq!(item.royalty_basis_points, 300, "royalty survives transfer");
        Ok(())
    }

    /// An unknown launcher id (never minted on this chain) resolves to `None`, never an error —
    /// the index simply omits it (a partially-confirmed mint still lists its confirmed items).
    #[tokio::test]
    async fn unknown_launcher_resolves_to_none() -> anyhow::Result<()> {
        let mock = MockChain::default();
        let got = resolve_nft_by_launcher(&mock, Bytes32::from([0xab; 32])).await?;
        assert!(got.is_none());
        // And the batch index skips it, returning an empty list (not an error).
        let items = index_collection_items(&mock, &[Bytes32::from([0xab; 32])]).await?;
        assert!(items.is_empty());
        Ok(())
    }

    /// `summarize_collection` reports a uniform DID + royalty only when items agree; an empty
    /// set has no DID/royalty and zero items.
    #[test]
    fn summarize_handles_empty_and_disagreement() {
        assert_eq!(summarize_collection(&[]).item_count, 0);
        assert_eq!(summarize_collection(&[]).did, None);
        assert_eq!(summarize_collection(&[]).royalty_basis_points, None);

        let a = IndexedNft {
            launcher_id: Bytes32::from([1; 32]),
            coin_id: Bytes32::from([2; 32]),
            owner_did: Some(Bytes32::from([9; 32])),
            royalty_puzzle_hash: Bytes32::from([3; 32]),
            royalty_basis_points: 300,
            owner_puzzle_hash: Bytes32::from([4; 32]),
            metadata: None,
        };
        let mut b = a.clone();
        b.royalty_basis_points = 500; // disagree on royalty
        b.owner_did = Some(Bytes32::from([8; 32])); // disagree on DID
        let s = summarize_collection(&[a.clone(), b]);
        assert_eq!(s.item_count, 2);
        assert_eq!(s.did, None, "DIDs disagree → no uniform DID");
        assert_eq!(s.royalty_basis_points, None, "royalties disagree → none");

        // Two agreeing items → uniform.
        let s2 = summarize_collection(&[a.clone(), a]);
        assert_eq!(s2.did, Some(Bytes32::from([9; 32])));
        assert_eq!(s2.royalty_basis_points, Some(300));
    }
}
