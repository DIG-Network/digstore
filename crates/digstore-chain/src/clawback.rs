//! Clawback payments — the wallet's Sage-parity "claw-back-able send": a payment
//! that the recipient can CLAIM only after a timelock, and that the sender can
//! CLAW BACK (recover) until the recipient claims it.
//!
//! Like [`crate::nft`] and [`crate::offer`], this module is **pure build (+ sign)**:
//! every builder returns UNSIGNED `Vec<CoinSpend>` (the claim/recover signers return
//! a signed [`SpendBundle`] only because they have all the keys), and NOTHING here
//! broadcasts. Signing the assembled bundle and pushing it stay the caller's gated
//! decision (the `dig-wallet` `DIG_WALLET_ALLOW_BROADCAST` gate).
//!
//! ## Shape
//! A clawback coin is a `P2OneOfMany` (1-of-2 merkle) puzzle over two paths,
//! exactly the canonical [`Clawback`] primitive:
//!   * the **receiver path** — an `AugmentedCondition` that asserts
//!     `ASSERT_SECONDS_RELATIVE timelock` then runs the receiver's inner puzzle, so
//!     the recipient can only claim after `timelock` seconds; and
//!   * the **sender path** — a `P2Curried` puzzle of the sender's puzzle hash, so the
//!     original sender can recover the coin at any time before it is claimed.
//!
//! [`build_clawback_send`] funds a clawback coin from an XCH coin the wallet controls
//! and emits the `Remark` hint condition the primitive uses to make the coin
//! reconstructable from its parent spend ([`parse_clawbacks`]). [`build_clawback_claim`]
//! takes the receiver path (after the timelock); [`build_clawback_recover`] takes the
//! sender path (recover before claim).

use crate::error::{ChainError, Result};
use crate::keys::IndexedKeys;
use chia::puzzles::Memos;
use chia_protocol::{Bytes32, Coin, CoinSpend, SpendBundle};
use chia_wallet_sdk::driver::{
    Clawback, Puzzle, Spend, SpendContext, SpendWithConditions, StandardLayer,
};
use chia_wallet_sdk::prelude::ToTreeHash;
use chia_wallet_sdk::types::Conditions;
use datalayer_driver::{sign_coin_spends, SecretKey, Signature};

/// The on-chain coin id and puzzle hash of a freshly-built clawback coin, returned
/// alongside the funding spends so the caller can later [`build_clawback_claim`] /
/// [`build_clawback_recover`] it (both need the coin, which only exists once the
/// funding spend is confirmed).
#[derive(Clone, Copy, Debug)]
pub struct ClawbackCoin {
    /// The clawback coin itself (parent = funding coin id, puzzle hash = the 1-of-2
    /// merkle root, amount = the sent amount).
    pub coin: Coin,
    /// The decoded clawback terms (timelock + sender/receiver puzzle hashes), needed
    /// to build the claim or recover spend.
    pub terms: Clawback,
}

/// Build the (UNSIGNED) coin spends that send `amount` mojos into a clawback coin
/// from `funding_coin` (an XCH coin the wallet controls at `sender.owner_puzzle_hash`),
/// claw-back-able by the sender and claimable by `receiver_ph` after `timelock`
/// seconds. Returns the spends and the resulting [`ClawbackCoin`].
///
/// The funding coin is spent through the standard layer to create the clawback coin at
/// the 1-of-2 merkle root, plus the `Remark` hint condition the [`Clawback`] primitive
/// uses so the coin is reconstructable from this parent spend (see [`parse_clawbacks`]).
/// Any excess of `funding_coin.amount` over `amount` is left to the consensus as an
/// implicit fee — size the funding coin to `amount` if no fee is intended.
///
/// **Pure: does NOT sign or broadcast.** `sender`'s synthetic key authorizes the
/// funding-coin spend; the caller signs the assembled bundle (see [`sign_clawback_spends`]).
pub fn build_clawback_send(
    sender: &IndexedKeys,
    funding_coin: Coin,
    receiver_ph: Bytes32,
    amount: u64,
    timelock: u64,
) -> Result<(Vec<CoinSpend>, ClawbackCoin)> {
    if amount == 0 {
        return Err(ChainError::Chain(
            "clawback send amount must be greater than zero".into(),
        ));
    }
    if amount > funding_coin.amount {
        return Err(ChainError::Chain(format!(
            "clawback amount {amount} exceeds funding coin amount {}",
            funding_coin.amount
        )));
    }

    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(sender.synthetic_pk);

    let clawback = Clawback {
        timelock,
        sender_puzzle_hash: sender.owner_puzzle_hash,
        receiver_puzzle_hash: receiver_ph,
    };
    let clawback_ph: Bytes32 = clawback.to_layer().tree_hash().into();

    // Create the clawback coin AND emit the Remark hint the primitive uses to make
    // the coin reconstructable from this parent spend.
    let remark = clawback
        .get_remark_condition(&mut ctx)
        .map_err(|e| ChainError::Chain(format!("clawback remark condition: {e}")))?;
    let conditions = Conditions::new()
        .create_coin(clawback_ph, amount, Memos::None)
        .with(remark);

    p2.spend(&mut ctx, funding_coin, conditions)
        .map_err(|e| ChainError::Chain(format!("spend funding coin: {e}")))?;

    let coin = Coin::new(funding_coin.coin_id(), clawback_ph, amount);
    Ok((
        ctx.take(),
        ClawbackCoin {
            coin,
            terms: clawback,
        },
    ))
}

/// Reconstruct the clawback coins created by a parent spend, by running the parent
/// puzzle+solution and matching its `CreateCoin` outputs against the `Remark`-hinted
/// clawback terms (exactly [`Clawback::parse_children`]).
///
/// Returns each decoded [`ClawbackCoin`] (its coin id is derived from the parent coin
/// id and the clawback merkle-root puzzle hash). Used to recover the spendable terms
/// from chain data when the caller only has the parent [`CoinSpend`] (e.g. from a
/// coinset scan), without having retained the [`ClawbackCoin`] from the send.
pub fn parse_clawbacks(parent_spend: &CoinSpend) -> Result<Vec<ClawbackCoin>> {
    let mut ctx = SpendContext::new();
    let puzzle_ptr = ctx
        .alloc(&parent_spend.puzzle_reveal)
        .map_err(|e| ChainError::Chain(format!("alloc parent puzzle: {e}")))?;
    let puzzle = Puzzle::parse(&ctx, puzzle_ptr);
    let solution = ctx
        .alloc(&parent_spend.solution)
        .map_err(|e| ChainError::Chain(format!("alloc parent solution: {e}")))?;

    let children = Clawback::parse_children(&mut ctx, puzzle, solution)
        .map_err(|e| ChainError::Chain(format!("parse clawback children: {e}")))?
        .unwrap_or_default();

    let parent_id = parent_spend.coin.coin_id();
    Ok(children
        .into_iter()
        .map(|terms| {
            let ph: Bytes32 = terms.to_layer().tree_hash().into();
            // The clawback coin's amount is not carried by the Remark; callers that
            // need it should pair this with the coinset coin record. We reconstruct
            // the puzzle-hash identity here; the amount is filled from the known coin.
            let coin = Coin::new(parent_id, ph, 0);
            ClawbackCoin { coin, terms }
        })
        .collect())
}

/// Build AND sign the spend that CLAIMS a clawback coin to `receiver` (the recipient
/// path), creating `clawback.coin.amount` mojos at `receiver.owner_puzzle_hash`.
///
/// Valid only after the clawback's `timelock` seconds have elapsed (the receiver path
/// asserts `ASSERT_SECONDS_RELATIVE timelock`); broadcasting earlier is rejected by the
/// consensus. The receiver's standard layer is wrapped by the augmented-condition
/// receiver path via [`Clawback::receiver_spend`]. `for_testnet` selects the signing
/// network (the simulator tests pass `true`).
///
/// `receiver` must be the address whose puzzle hash matches `clawback.terms.receiver_puzzle_hash`.
/// **Pure: does NOT broadcast.**
pub fn build_clawback_claim(
    clawback: &ClawbackCoin,
    receiver: &IndexedKeys,
    fee: u64,
    for_testnet: bool,
) -> Result<SpendBundle> {
    if receiver.owner_puzzle_hash != clawback.terms.receiver_puzzle_hash {
        return Err(ChainError::Chain(
            "claim key does not match the clawback's receiver puzzle hash".into(),
        ));
    }
    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(receiver.synthetic_pk);

    // The receiver re-creates the coin at its own address (and optionally reserves a fee).
    let mut inner = Conditions::new().create_coin(
        receiver.owner_puzzle_hash,
        clawback.coin.amount,
        Memos::None,
    );
    if fee > 0 {
        inner = inner.reserve_fee(fee);
    }
    let inner_spend = p2
        .spend_with_conditions(&mut ctx, inner)
        .map_err(|e| ChainError::Chain(format!("receiver inner spend: {e}")))?;

    let receiver_spend = clawback
        .terms
        .receiver_spend(&mut ctx, inner_spend)
        .map_err(|e| ChainError::Chain(format!("clawback receiver spend: {e}")))?;
    spend_clawback(&mut ctx, clawback.coin, receiver_spend)?;

    sign_bundle(
        &ctx.take(),
        std::slice::from_ref(&receiver.synthetic_sk),
        for_testnet,
    )
}

/// Build AND sign the spend that RECOVERS (claws back) a clawback coin to `sender` (the
/// sender path), returning `clawback.coin.amount` mojos to `sender.owner_puzzle_hash`.
///
/// Valid at any time before the recipient claims it (the sender path has no timelock).
/// The sender's standard layer is wrapped by the curried sender path via
/// [`Clawback::sender_spend`]. `for_testnet` selects the signing network.
///
/// `sender` must be the address whose puzzle hash matches `clawback.terms.sender_puzzle_hash`.
/// **Pure: does NOT broadcast.**
pub fn build_clawback_recover(
    clawback: &ClawbackCoin,
    sender: &IndexedKeys,
    fee: u64,
    for_testnet: bool,
) -> Result<SpendBundle> {
    if sender.owner_puzzle_hash != clawback.terms.sender_puzzle_hash {
        return Err(ChainError::Chain(
            "recover key does not match the clawback's sender puzzle hash".into(),
        ));
    }
    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(sender.synthetic_pk);

    let mut inner =
        Conditions::new().create_coin(sender.owner_puzzle_hash, clawback.coin.amount, Memos::None);
    if fee > 0 {
        inner = inner.reserve_fee(fee);
    }
    let inner_spend = p2
        .spend_with_conditions(&mut ctx, inner)
        .map_err(|e| ChainError::Chain(format!("sender inner spend: {e}")))?;

    let sender_spend = clawback
        .terms
        .sender_spend(&mut ctx, inner_spend)
        .map_err(|e| ChainError::Chain(format!("clawback sender spend: {e}")))?;
    spend_clawback(&mut ctx, clawback.coin, sender_spend)?;

    sign_bundle(
        &ctx.take(),
        std::slice::from_ref(&sender.synthetic_sk),
        for_testnet,
    )
}

/// Insert the assembled `spend` of the clawback `coin` into `ctx`.
fn spend_clawback(ctx: &mut SpendContext, coin: Coin, spend: Spend) -> Result<()> {
    ctx.spend(coin, spend)
        .map_err(|e| ChainError::Chain(format!("spend clawback coin: {e}")))
}

/// Sign assembled clawback `coin_spends` with `keys` (the wallet's synthetic secret
/// keys), returning the aggregated signature. A thin convenience over
/// [`datalayer_driver::sign_coin_spends`] so callers building a clawback send bundle
/// sign it the same way [`crate::nft::sign_nft_spends`] does.
pub fn sign_clawback_spends(
    coin_spends: &[CoinSpend],
    keys: &[SecretKey],
    for_testnet: bool,
) -> Result<Signature> {
    sign_coin_spends(coin_spends, keys, for_testnet)
        .map_err(|e| ChainError::Chain(format!("sign clawback spends: {e}")))
}

/// Sign `coin_spends` and wrap them into a ready [`SpendBundle`] (used by the claim /
/// recover builders, which hold all the keys for their single coin).
fn sign_bundle(
    coin_spends: &[CoinSpend],
    keys: &[SecretKey],
    for_testnet: bool,
) -> Result<SpendBundle> {
    let sig = sign_clawback_spends(coin_spends, keys, for_testnet)?;
    Ok(SpendBundle::new(coin_spends.to_vec(), sig))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::derive_indexed_keys;
    use chia_sdk_test::Simulator;

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    // ----- offline: input validation -----

    #[test]
    fn send_rejects_zero_amount() {
        let sender = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let funding = Coin::new(Bytes32::default(), sender.owner_puzzle_hash, 10);
        let err = build_clawback_send(&sender, funding, Bytes32::default(), 0, 1).unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("greater than zero")),
            "got: {err}"
        );
    }

    #[test]
    fn send_rejects_amount_over_funding() {
        let sender = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let funding = Coin::new(Bytes32::default(), sender.owner_puzzle_hash, 5);
        let err = build_clawback_send(&sender, funding, Bytes32::default(), 10, 1).unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("exceeds funding coin amount")),
            "got: {err}"
        );
    }

    #[test]
    fn claim_rejects_wrong_receiver_key() {
        let keys = derive_indexed_keys(ABANDON, 0..2).unwrap();
        let sender = keys[0].clone();
        let stranger = keys[1].clone();
        let clawback = ClawbackCoin {
            coin: Coin::new(Bytes32::default(), Bytes32::default(), 1),
            terms: Clawback {
                timelock: 1,
                sender_puzzle_hash: sender.owner_puzzle_hash,
                // receiver is someone else, not `stranger`.
                receiver_puzzle_hash: Bytes32::from([0x42; 32]),
            },
        };
        let err = build_clawback_claim(&clawback, &stranger, 0, true).unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("receiver puzzle hash")),
            "got: {err}"
        );
    }

    // ----- Simulator: send -> claim (receiver path, after timelock) -----

    /// Send a clawback payment from the wallet (index 0) to the recipient (index 1),
    /// confirm it on the simulator, advance past the timelock, then have the recipient
    /// CLAIM it. The claimed mojos must land at the recipient's address. This drives
    /// `build_clawback_send` + `build_clawback_claim` end-to-end on the in-process
    /// Chia simulator (the nft.rs/offer.rs pattern).
    #[test]
    fn send_then_claim_round_trip() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let keys = derive_indexed_keys(ABANDON, 0..2)?;
        let sender = keys[0].clone();
        let receiver = keys[1].clone();
        let timelock = 100u64;
        let amount = 1_000u64;

        // Fund a clawback coin from a simulator XCH coin at the sender's address.
        let funding = sim.new_coin(sender.owner_puzzle_hash, amount);
        let (send_spends, clawback) = build_clawback_send(
            &sender,
            funding,
            receiver.owner_puzzle_hash,
            amount,
            timelock,
        )?;
        assert_eq!(clawback.coin.amount, amount);
        assert_eq!(clawback.terms.timelock, timelock);
        assert_eq!(
            clawback.terms.receiver_puzzle_hash,
            receiver.owner_puzzle_hash
        );

        // The send bundle reconstructs the clawback terms from its own parent spend.
        let parsed = parse_clawbacks(&send_spends[0])?;
        assert_eq!(parsed.len(), 1, "send must hint exactly one clawback coin");
        assert_eq!(parsed[0].terms, clawback.terms);

        let sig = sign_clawback_spends(
            &send_spends,
            std::slice::from_ref(&sender.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(send_spends, sig))?;

        // The clawback coin now exists on-chain.
        assert!(
            sim.coin_state(clawback.coin.coin_id()).is_some(),
            "the clawback coin should exist after the send"
        );

        // Advance past the timelock so the receiver path's ASSERT_SECONDS_RELATIVE
        // passes: the claim block's timestamp must be >= the clawback coin's creation
        // timestamp + the timelock.
        sim.pass_time(timelock + 10);

        // The recipient claims the clawback coin to its own address.
        let claim = build_clawback_claim(&clawback, &receiver, 0, true)?;
        sim.new_transaction(claim)?;

        // The claimed mojos land at the receiver's address.
        let received = sim
            .unspent_coins(receiver.owner_puzzle_hash, false)
            .iter()
            .map(|c| c.amount)
            .sum::<u64>();
        assert_eq!(
            received, amount,
            "the recipient should receive the full amount"
        );
        Ok(())
    }

    // ----- Simulator: send -> recover (sender path, before claim) -----

    /// Send a clawback payment, then have the SENDER recover (claw back) it before the
    /// recipient claims. The recovered mojos must return to the sender. Drives
    /// `build_clawback_send` + `build_clawback_recover` end-to-end on the simulator.
    #[test]
    fn send_then_recover_round_trip() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let keys = derive_indexed_keys(ABANDON, 0..2)?;
        let sender = keys[0].clone();
        let receiver = keys[1].clone();
        let amount = 2_000u64;

        let funding = sim.new_coin(sender.owner_puzzle_hash, amount);
        let (send_spends, clawback) = build_clawback_send(
            &sender,
            funding,
            receiver.owner_puzzle_hash,
            amount,
            u64::MAX, // a far-future timelock; the recipient could never claim in time
        )?;
        let sig = sign_clawback_spends(
            &send_spends,
            std::slice::from_ref(&sender.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(send_spends, sig))?;

        // The sender recovers the coin (no timelock on the sender path).
        let recover = build_clawback_recover(&clawback, &sender, 0, true)?;
        sim.new_transaction(recover)?;

        // The recovered mojos return to the sender's address.
        let recovered = sim
            .unspent_coins(sender.owner_puzzle_hash, false)
            .iter()
            .map(|c| c.amount)
            .sum::<u64>();
        assert_eq!(
            recovered, amount,
            "the sender should recover the full amount"
        );
        Ok(())
    }
}
