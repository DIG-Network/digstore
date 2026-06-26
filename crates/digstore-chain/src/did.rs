//! DIDs (decentralized identifiers) over coinset — the wallet's Sage-parity DID
//! surface: create a DID, enumerate owned DIDs, transfer a DID to a new owner, and
//! attribute a DID to an NFT.
//!
//! Like [`crate::nft`], [`crate::offer`], and [`crate::cat`], this module is **pure
//! build (+ parse)**: the builders return UNSIGNED `Vec<CoinSpend>` and the enumerator
//! returns parsed [`OwnedDid`] records. NOTHING here broadcasts and nothing fetches the
//! network — chain reads go through the [`ChainReads`] trait so the logic is testable
//! against the in-crate mock and the in-process Chia
//! [`Simulator`](chia_sdk_test::Simulator). Signing the assembled bundle and pushing it
//! stay the caller's gated decision (the `dig-wallet` `DIG_WALLET_ALLOW_BROADCAST` gate).
//!
//! ## Scope
//! This covers the DID primitives chia-wallet-sdk 0.30 exposes via `did_launcher`
//! (`create_eve_did`/`create_did`/`create_simple_did`), `did_info` (`DidInfo::parse`),
//! and the `did_layer`/`singleton_layer` spend path (transfer + the no-op `update` spend
//! used to acknowledge an NFT-DID assignment). Chia Verifiable Credentials (a separate
//! puzzle family) and the vault/MIPS multi-key wallets are intentionally OUT of scope
//! here — they are a separate later task per the wallet program plan.
//!
//! ## Enumeration model
//! `chia-wallet-sdk` decodes one DID at a time; it does not enumerate. We enumerate by
//! querying coinset for coins HINTED to each of the wallet's owner puzzle hashes (a DID's
//! child coin is hinted to its p2/owner puzzle hash), then reconstructing each hinted
//! coin from its parent spend via [`Did::parse_child`] — the same parent-spend walk
//! [`crate::nft`] and [`crate::singleton::sync_datastore`] use, with coinset reads swapped
//! in for a `Peer`.

use crate::coinset::ChainReads;
use crate::error::{ChainError, Result};
use crate::keys::IndexedKeys;
use chia_protocol::{Bytes32, Coin, CoinSpend};
use chia_wallet_sdk::driver::{Did, Launcher, Puzzle, SingletonInfo, SpendContext, StandardLayer};
use chia_wallet_sdk::types::conditions::TransferNft;
use chia_wallet_sdk::types::Conditions;
use datalayer_driver::{sign_coin_spends, SecretKey, Signature};

/// A decoded DID the wallet owns, carried as a spendable [`Did`] plus the flattened
/// on-chain fields the UI needs to display it WITHOUT re-parsing.
#[derive(Clone, Debug)]
pub struct OwnedDid {
    /// The full spendable DID (coin + lineage proof + info), valid only in the context it
    /// was parsed in (see [`build_did_transfer`] re-parsing it for a spend).
    pub did: Did,
    /// The launcher coin id — the DID's stable identity (its `did:chia:…` id encodes this).
    pub launcher_id: Bytes32,
    /// The current coin id of the unspent DID singleton.
    pub coin_id: Bytes32,
    /// The current owner (p2) puzzle hash — where the DID lives.
    pub p2_puzzle_hash: Bytes32,
    /// The number of recovery verifications required (a rarely used DID feature).
    pub num_verifications_required: u64,
}

impl OwnedDid {
    fn from_did(did: Did) -> Self {
        Self {
            launcher_id: did.info.launcher_id,
            coin_id: did.coin.coin_id(),
            p2_puzzle_hash: did.info.p2_puzzle_hash,
            num_verifications_required: did.info.num_verifications_required,
            did,
        }
    }
}

/// Build the (UNSIGNED) coin spends that create a simple DID from `funding_coin` (an XCH
/// coin the wallet controls at `creator.owner_puzzle_hash`), returning the spends and the
/// created [`Did`].
///
/// A "simple" DID has no recovery list and `num_verifications_required = 1` — the common
/// case Sage and the reference wallet create. The funding coin launches the DID singleton
/// (1 mojo) through the standard layer; any excess over 1 mojo is left to the consensus
/// as an implicit fee, so size the funding coin to the singleton amount for no fee.
///
/// **Pure: does NOT sign or broadcast.** `creator`'s synthetic key authorizes the
/// funding-coin spend; the caller signs the assembled bundle.
pub fn create_simple_did(
    creator: &IndexedKeys,
    funding_coin: Coin,
) -> Result<(Vec<CoinSpend>, Did)> {
    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(creator.synthetic_pk);

    let (create_conditions, did) = Launcher::new(funding_coin.coin_id(), 1)
        .create_simple_did(&mut ctx, &p2)
        .map_err(|e| ChainError::Chain(format!("create simple did: {e}")))?;

    p2.spend(&mut ctx, funding_coin, create_conditions)
        .map_err(|e| ChainError::Chain(format!("spend funding coin: {e}")))?;

    Ok((ctx.take(), did))
}

/// Build the (UNSIGNED) coin spends that create a DID with explicit recovery config from
/// `funding_coin`, returning the spends and the created [`Did`].
///
/// `recovery_list_hash` is the hash of the recovery DID list (`None` to disable recovery;
/// note the Chia reference wallet historically requires it present even when unused).
/// `num_verifications_required` is how many recovery verifications a recovery would need.
/// For the common case prefer [`create_simple_did`] (`None`, `1`).
///
/// **Pure: does NOT sign or broadcast.**
pub fn create_did(
    creator: &IndexedKeys,
    funding_coin: Coin,
    recovery_list_hash: Option<Bytes32>,
    num_verifications_required: u64,
) -> Result<(Vec<CoinSpend>, Did)> {
    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(creator.synthetic_pk);

    let (create_conditions, did) = Launcher::new(funding_coin.coin_id(), 1)
        .create_did(
            &mut ctx,
            recovery_list_hash,
            num_verifications_required,
            chia_wallet_sdk::driver::HashedPtr::NIL,
            &p2,
        )
        .map_err(|e| ChainError::Chain(format!("create did: {e}")))?;

    p2.spend(&mut ctx, funding_coin, create_conditions)
        .map_err(|e| ChainError::Chain(format!("spend funding coin: {e}")))?;

    Ok((ctx.take(), did))
}

/// Reconstruct the wallet's unspent DIDs owned by `owner_phs` (its HD owner puzzle
/// hashes), over `chain`.
///
/// For each owner puzzle hash: query coinset for coins HINTED to it
/// ([`ChainReads::unspent_coins_by_hint`]); a DID's child coin is hinted to its p2/owner
/// puzzle hash, so an owned DID's current coin surfaces here. For each hinted coin, look
/// up its own record to find the height its PARENT was spent (the child's confirmation
/// block == the parent's spend block), fetch the parent spend, and run
/// [`Did::parse_child`] over it.
///
/// Non-DID hinted coins (plain XCH, CATs, NFTs) are skipped silently — a hint is not
/// DID-specific, so the puzzle parse is the authority on what each coin actually is.
pub async fn list_owned_dids(
    chain: &dyn ChainReads,
    owner_phs: &[Bytes32],
) -> Result<Vec<OwnedDid>> {
    let mut out = Vec::new();
    for owner_ph in owner_phs {
        let coins = chain.unspent_coins_by_hint(*owner_ph).await?;
        for coin in coins {
            if let Some(did) = reconstruct_did(chain, &coin).await? {
                out.push(OwnedDid::from_did(did));
            }
        }
    }
    Ok(out)
}

/// Reconstruct one spendable [`Did`] from a hinted child coin by parsing its parent spend.
///
/// Returns `Ok(None)` if the coin is not a DID (a hint is not DID-specific) or if the
/// parent's spend yields no matching child. Errors only on a genuine chain-read failure
/// or a malformed parent spend.
///
/// Uses a fresh internal [`SpendContext`]; the returned DID's metadata pointer is valid
/// only for inspection — to build a SPEND, re-parse via [`reconstruct_did_in`] (which
/// [`build_did_transfer`] does).
async fn reconstruct_did(chain: &dyn ChainReads, coin: &Coin) -> Result<Option<Did>> {
    let mut ctx = SpendContext::new();
    reconstruct_did_in(&mut ctx, chain, coin).await
}

/// Reconstruct a spendable [`Did`] from its parent spend INTO the caller-provided `ctx`,
/// so the returned DID is valid for spends built in that same context.
///
/// [`Did::parse_child`] needs the CHILD coin (the coin we found via the hint) in addition
/// to the parent spend, because a DID child is identified by its hint memo, not derivable
/// from the parent puzzle alone.
async fn reconstruct_did_in(
    ctx: &mut SpendContext,
    chain: &dyn ChainReads,
    coin: &Coin,
) -> Result<Option<Did>> {
    let coin_id = coin.coin_id();

    let Some(rec) = chain.coin_record(coin_id).await? else {
        return Ok(None);
    };
    let parent_spent_height = rec.confirmed_block_index;

    let Some(parent_spend) = chain
        .coin_spend(coin.parent_coin_info, parent_spent_height)
        .await?
    else {
        return Ok(None);
    };

    let parent_puzzle_ptr = ctx
        .alloc(&parent_spend.puzzle_reveal)
        .map_err(|e| ChainError::Chain(format!("alloc parent puzzle: {e}")))?;
    let parent_puzzle = Puzzle::parse(ctx, parent_puzzle_ptr);
    let parent_solution = ctx
        .alloc(&parent_spend.solution)
        .map_err(|e| ChainError::Chain(format!("alloc parent solution: {e}")))?;

    let child = Did::parse_child(
        ctx,
        parent_spend.coin,
        parent_puzzle,
        parent_solution,
        *coin,
    )
    .map_err(|e| ChainError::Chain(format!("Did::parse_child: {e}")))?;

    Ok(child.filter(|did| did.coin.coin_id() == coin_id))
}

/// Build the (UNSIGNED) coin spends that transfer the DID currently at `did_coin` to
/// `new_owner_ph`, optionally paying a network `fee` from `fee_coin`, returning the spends
/// and the child [`Did`] (the DID at its new owner).
///
/// The spendable DID is reconstructed from `chain` (its parent spend) INSIDE this
/// function's own [`SpendContext`] — a spendable [`Did`] is allocator-relative, so it must
/// be re-parsed in the same context the transfer spend is built in (this is why a
/// pre-parsed `Did` is NOT taken directly). Pass `did_coin` from an [`OwnedDid`]
/// (`owned.did.coin`).
///
/// A non-zero `fee` requires `fee_coin` (an XCH coin at `owner.owner_puzzle_hash`): it is
/// spent through the standard layer to reserve the fee. `owner`'s synthetic key authorizes
/// both the DID inner spend and the fee-coin spend.
///
/// **Pure: does NOT sign or broadcast.**
pub async fn build_did_transfer(
    chain: &dyn ChainReads,
    owner: &IndexedKeys,
    did_coin: Coin,
    new_owner_ph: Bytes32,
    fee: u64,
    fee_coin: Option<Coin>,
) -> Result<(Vec<CoinSpend>, Did)> {
    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(owner.synthetic_pk);

    let did = reconstruct_did_in(&mut ctx, chain, &did_coin)
        .await?
        .ok_or_else(|| {
            ChainError::Chain(format!(
                "coin {:?} is not a spendable DID (or its parent spend was not found)",
                did_coin.coin_id()
            ))
        })?;

    let child = did
        .transfer(&mut ctx, &p2, new_owner_ph, Conditions::new())
        .map_err(|e| ChainError::Chain(format!("transfer did: {e}")))?;

    if fee > 0 {
        let fee_coin = fee_coin.ok_or_else(|| {
            ChainError::Chain("a non-zero fee requires a fee_coin to reserve it from".into())
        })?;
        p2.spend(&mut ctx, fee_coin, Conditions::new().reserve_fee(fee))
            .map_err(|e| ChainError::Chain(format!("spend fee coin: {e}")))?;
    }

    Ok((ctx.take(), child))
}

/// The `TransferNft` condition that assigns an NFT to a DID, given the DID's launcher id
/// and the inner puzzle hash of its current (unspent) coin.
///
/// This is the canonical DID→NFT attribution mechanism (the NFT ownership layer records
/// the DID launcher id as the NFT's owner, proving who created/owns it). The condition is
/// applied to the NFT spend (see [`crate::nft`]'s mint/`MintSpec.did`), and the DID coin
/// must be spent in the same bundle (e.g. via [`build_did_attribute_nft`]) to acknowledge
/// the assignment.
pub fn nft_attribution_condition(
    did_launcher_id: Bytes32,
    did_inner_puzzle_hash: Bytes32,
) -> TransferNft {
    TransferNft::new(
        Some(did_launcher_id),
        Vec::new(),
        Some(did_inner_puzzle_hash),
    )
}

/// Build the (UNSIGNED) DID coin spend that acknowledges attributing an NFT to the DID
/// currently at `did_coin`, returning the spends and the child [`Did`].
///
/// Attributing an NFT to a DID is a two-coin operation: the NFT spend carries the
/// [`nft_attribution_condition`] (assigning the DID as its owner — built in [`crate::nft`])
/// and the DID is spent in the SAME bundle with an `update` (a no-op spend that recreates
/// the DID unchanged) emitting `extra_conditions` that acknowledge the assignment. This
/// builder produces the DID half; the caller concatenates it with the NFT half (and signs
/// the combined bundle).
///
/// `extra_conditions` are the assignment-acknowledging conditions the NFT mint/assign
/// returns (e.g. the `assert_puzzle_announcement` from `nft.assign_owner` / the mint).
/// `owner`'s synthetic key authorizes the DID spend.
///
/// **Pure: does NOT sign or broadcast.**
pub async fn build_did_attribute_nft(
    chain: &dyn ChainReads,
    owner: &IndexedKeys,
    did_coin: Coin,
    extra_conditions: Conditions,
) -> Result<(Vec<CoinSpend>, Did)> {
    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(owner.synthetic_pk);

    let did = reconstruct_did_in(&mut ctx, chain, &did_coin)
        .await?
        .ok_or_else(|| {
            ChainError::Chain(format!(
                "coin {:?} is not a spendable DID (or its parent spend was not found)",
                did_coin.coin_id()
            ))
        })?;

    let child = did
        .update(&mut ctx, &p2, extra_conditions)
        .map_err(|e| ChainError::Chain(format!("update did for nft attribution: {e}")))?;

    Ok((ctx.take(), child))
}

/// The inner puzzle hash of a spendable [`Did`] — the value the NFT attribution condition
/// needs ([`nft_attribution_condition`]'s `did_inner_puzzle_hash`). Convenience so callers
/// holding an [`OwnedDid`] can build the attribution without reaching into SDK traits.
pub fn did_inner_puzzle_hash(did: &Did) -> Bytes32 {
    did.info.inner_puzzle_hash().into()
}

/// Sign assembled DID `coin_spends` with `keys` (the wallet's synthetic secret keys),
/// returning the aggregated signature. `for_testnet` selects the `agg_sig_me` network
/// (mainnet in production; the Simulator tests pass `true`).
pub fn sign_did_spends(
    coin_spends: &[CoinSpend],
    keys: &[SecretKey],
    for_testnet: bool,
) -> Result<Signature> {
    sign_coin_spends(coin_spends, keys, for_testnet)
        .map_err(|e| ChainError::Chain(format!("sign did spends: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinset::mock::MockChain;
    use crate::keys::derive_indexed_keys;
    use chia_protocol::SpendBundle;
    use chia_sdk_test::Simulator;

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    /// Seed a [`MockChain`] so `reconstruct_did`/`build_did_transfer` can rebuild the DID
    /// at `did_coin` from its parent spend, mirroring what coinset would return after the
    /// create: the DID coin's own record (carrying the height its parent was spent) and the
    /// parent's coin spend (keyed by the parent coin id), both read from the simulator.
    fn seed_mock_from_sim(sim: &Simulator, did_coin: Coin) -> MockChain {
        let mut mock = MockChain::default();
        let coin_id = did_coin.coin_id();
        let confirmed = sim
            .coin_state(coin_id)
            .and_then(|cs| cs.created_height)
            .expect("did coin should exist on the simulator after the create");
        mock.records.insert(
            coin_id,
            crate::coinset::CoinInfo {
                coin: did_coin,
                spent: false,
                confirmed_block_index: confirmed,
                spent_block_index: 0,
                timestamp: 0,
                coinbase: false,
            },
        );
        let parent_spend = sim
            .coin_spend(did_coin.parent_coin_info)
            .expect("parent (eve) spend should be recorded on the simulator");
        mock.spends.insert(did_coin.parent_coin_info, parent_spend);
        mock
    }

    // ----- offline: input validation -----

    #[tokio::test]
    async fn transfer_requires_fee_coin_for_nonzero_fee() -> anyhow::Result<()> {
        // A non-zero fee with no fee_coin must error cleanly (not panic). Create a DID in
        // the simulator, seed a mock chain so the transfer can reconstruct it, then call
        // transfer with a fee but no fee_coin.
        let mut sim = Simulator::new();
        let creator = derive_indexed_keys(ABANDON, 0..1)?[0].clone();
        let funding = sim.new_coin(creator.owner_puzzle_hash, 2);
        let (create_spends, did) = create_simple_did(&creator, funding)?;
        sim.spend_coins(create_spends, std::slice::from_ref(&creator.synthetic_sk))?;

        let mock = seed_mock_from_sim(&sim, did.coin);
        let err = build_did_transfer(
            &mock,
            &creator,
            did.coin,
            creator.owner_puzzle_hash,
            1_000,
            None,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("fee_coin")),
            "got: {err}"
        );
        Ok(())
    }

    // ----- Simulator: a DID create + transfer round-trip -----

    /// Create a simple DID for the wallet's index-0 owner ph using `create_simple_did`,
    /// fund it from a simulator XCH coin at that ph, prove the DID lands hinted to the
    /// owner, enumerate it over a mock chain, then transfer it to index 1 and prove it
    /// lands there. Exercises create + reconstruct + transfer end-to-end on the in-process
    /// Chia simulator (the offer.rs/nft.rs pattern).
    #[tokio::test]
    async fn create_did_then_transfer_round_trip() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let keys = derive_indexed_keys(ABANDON, 0..2)?;
        let creator = keys[0].clone();
        let recipient = keys[1].clone();

        let funding = sim.new_coin(creator.owner_puzzle_hash, 2);
        let (create_spends, did) = create_simple_did(&creator, funding)?;

        // A simple DID: no recovery list, 1 verification required.
        assert_eq!(did.info.num_verifications_required, 1);
        assert_eq!(did.info.p2_puzzle_hash, creator.owner_puzzle_hash);

        sim.spend_coins(create_spends, std::slice::from_ref(&creator.synthetic_sk))?;

        // The DID lands hinted to the creator's owner puzzle hash.
        assert!(
            !sim.hinted_coins(creator.owner_puzzle_hash).is_empty(),
            "the created DID should be hinted to the creator"
        );

        // Enumerate over a mock chain seeded from the simulator: list finds the DID.
        let mut mock = seed_mock_from_sim(&sim, did.coin);
        mock.records_by_hint.insert(
            creator.owner_puzzle_hash,
            vec![mock.records[&did.coin.coin_id()].clone()],
        );
        let owned = list_owned_dids(&mock, &[creator.owner_puzzle_hash]).await?;
        assert_eq!(owned.len(), 1, "enumeration should find the created DID");
        assert_eq!(owned[0].launcher_id, did.info.launcher_id);
        assert_eq!(owned[0].p2_puzzle_hash, creator.owner_puzzle_hash);

        // Transfer the DID to index 1, no fee (reconstruct from chain, build in context).
        let (xfer_spends, child) = build_did_transfer(
            &mock,
            &creator,
            did.coin,
            recipient.owner_puzzle_hash,
            0,
            None,
        )
        .await?;
        assert_eq!(child.info.p2_puzzle_hash, recipient.owner_puzzle_hash);

        let sig = sign_did_spends(
            &xfer_spends,
            std::slice::from_ref(&creator.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(xfer_spends, sig))?;

        // After transfer, the DID is hinted to the recipient's owner puzzle hash.
        assert!(
            !sim.hinted_coins(recipient.owner_puzzle_hash).is_empty(),
            "the transferred DID should land at the recipient"
        );
        Ok(())
    }

    /// The DID→NFT attribution condition carries the DID launcher id + inner puzzle hash,
    /// and the DID-side acknowledgement spend (a no-op `update`) recreates the DID. This
    /// drives `nft_attribution_condition`, `did_inner_puzzle_hash`, and
    /// `build_did_attribute_nft` against a real DID on the simulator (the NFT half lives
    /// in `crate::nft`, which already round-trips a DID-attributed mint).
    #[tokio::test]
    async fn did_attribution_condition_and_update_spend() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let creator = derive_indexed_keys(ABANDON, 0..1)?[0].clone();
        let funding = sim.new_coin(creator.owner_puzzle_hash, 2);
        let (create_spends, did) = create_simple_did(&creator, funding)?;
        sim.spend_coins(create_spends, std::slice::from_ref(&creator.synthetic_sk))?;

        // The attribution condition points at this DID.
        let inner_ph = did_inner_puzzle_hash(&did);
        let cond = nft_attribution_condition(did.info.launcher_id, inner_ph);
        assert_eq!(cond.launcher_id, Some(did.info.launcher_id));
        assert_eq!(cond.singleton_inner_puzzle_hash, Some(inner_ph));

        // The DID-side acknowledgement spend recreates the DID unchanged (a no-op update,
        // here with empty extra conditions just to prove the spend builds + validates).
        let mock = seed_mock_from_sim(&sim, did.coin);
        let (did_spends, child) =
            build_did_attribute_nft(&mock, &creator, did.coin, Conditions::new()).await?;
        assert_eq!(child.info.launcher_id, did.info.launcher_id);
        assert_eq!(child.info.p2_puzzle_hash, creator.owner_puzzle_hash);

        let sig = sign_did_spends(
            &did_spends,
            std::slice::from_ref(&creator.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(did_spends, sig))?;
        Ok(())
    }
}
