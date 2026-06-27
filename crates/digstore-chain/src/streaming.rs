//! Streamed (vesting) payments — the wallet's Sage-parity streaming surface: lock an
//! XCH amount that vests LINEARLY to a recipient between a start and an end time, let
//! the recipient CLAIM the vested portion at any point, and let the payer CLAW BACK
//! the unvested remainder.
//!
//! Built on the chia-wallet-sdk 0.30 streaming primitive
//! ([`StreamedAsset`]/[`StreamingPuzzleInfo`]/the stream layer). Like [`crate::nft`]
//! and [`crate::option`], this is **pure build (+ sign)**: builders return UNSIGNED
//! `Vec<CoinSpend>`; NOTHING here broadcasts. Signing + pushing stay the caller's gated
//! decision (the dig-wallet `DIG_WALLET_ALLOW_BROADCAST` gate).
//!
//! ## Shape
//! A streaming coin is created at the stream puzzle hash with four launch-hint memos
//! (recipient, clawback ph, last-payment-time = start, end-time). The amount vests
//! linearly from `start` to `end`; at time `t` the claimable amount is
//! `amount * (t - last_payment_time) / (end_time - last_payment_time)`. Each CLAIM
//! pays out the newly-vested portion, advances `last_payment_time` to `t`, and re-creates
//! the streaming coin with the remainder for the next claim.
//!
//! ## Claim authorization (recipient message)
//! The stream puzzle only releases funds when it receives a `send_message` (mode 23)
//! carrying the chosen `payment_time` from the RECIPIENT (or, for a clawback, from the
//! clawback-puzzle coin). [`build_stream_claim`] / [`build_stream_clawback`] build BOTH
//! the message-coin spend and the streaming-coin spend into one bundle, so the caller
//! has a complete, signable transaction.
//!
//! ## Scope of this module
//! This implements **XCH streaming** (the wallet-common case). The same primitive
//! supports a streamed CAT via [`StreamedAsset::cat`]; a CAT stream would reconstruct
//! the CAT lineage proof exactly as [`crate::cat`] does and is documented rather than
//! built here.

use crate::error::{ChainError, Result};
use crate::keys::IndexedKeys;
use chia::consensus::make_aggsig_final_message::u64_to_bytes;
use chia::puzzles::Memos;
use chia_protocol::{Bytes, Bytes32, Coin, CoinSpend};
use chia_wallet_sdk::driver::{SpendContext, StandardLayer, StreamedAsset, StreamingPuzzleInfo};
use chia_wallet_sdk::types::Conditions;
use datalayer_driver::{sign_coin_spends, SecretKey, Signature};

/// A created streaming payment, returned by [`build_stream_create`] so the caller can
/// later claim the vested portion (which needs the streaming coin + its terms, both of
/// which only exist once the create spend is confirmed).
#[derive(Clone, Debug)]
pub struct StreamedPayment {
    /// The spendable streamed asset (the streaming coin + its vesting terms).
    pub asset: StreamedAsset,
}

impl StreamedPayment {
    /// The vested (claimable) amount of this stream at `payment_time`, in mojos. Clamped
    /// at the full coin amount once `payment_time >= end_time`.
    pub fn vested_at(&self, payment_time: u64) -> u64 {
        if payment_time <= self.asset.info.last_payment_time {
            return 0;
        }
        if payment_time >= self.asset.info.end_time {
            return self.asset.coin.amount;
        }
        self.asset
            .info
            .amount_to_be_paid(self.asset.coin.amount, payment_time)
    }
}

/// Build the (UNSIGNED) coin spends that CREATE an XCH streaming payment of `amount`
/// mojos to `recipient_ph`, vesting linearly from `start_time` to `end_time`, with the
/// unvested remainder claw-back-able to `clawback_ph` (typically the payer's address).
/// Returns the spends and the resulting [`StreamedPayment`].
///
/// The full `amount` is spent from `funding_coin` (a wallet XCH coin at
/// `payer.owner_puzzle_hash`) into a single streaming coin at the stream puzzle hash,
/// carrying the four launch-hint memos the primitive uses to reconstruct the terms. Any
/// excess of `funding_coin.amount` over `amount` is left as an implicit fee.
///
/// **Pure: does NOT sign or broadcast.** `payer`'s synthetic key authorizes the
/// funding-coin spend; the caller signs the assembled bundle (see [`sign_stream_spends`]).
pub fn build_stream_create(
    payer: &IndexedKeys,
    funding_coin: Coin,
    recipient_ph: Bytes32,
    clawback_ph: Bytes32,
    amount: u64,
    start_time: u64,
    end_time: u64,
) -> Result<(Vec<CoinSpend>, StreamedPayment)> {
    if amount == 0 {
        return Err(ChainError::Chain(
            "stream amount must be greater than zero".into(),
        ));
    }
    if end_time <= start_time {
        return Err(ChainError::Chain(
            "stream end_time must be greater than start_time".into(),
        ));
    }
    if amount > funding_coin.amount {
        return Err(ChainError::Chain(format!(
            "stream amount {amount} exceeds funding coin amount {}",
            funding_coin.amount
        )));
    }

    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(payer.synthetic_pk);

    // The vesting terms: linear from start_time (last_payment_time) to end_time.
    let info = StreamingPuzzleInfo::new(recipient_ph, Some(clawback_ph), end_time, start_time);
    let stream_ph: Bytes32 = info.inner_puzzle_hash().into();

    // Launch hints let the primitive reconstruct the stream from its parent spend.
    let launch_hints = ctx
        .alloc(&info.get_launch_hints())
        .map_err(|e| ChainError::Chain(format!("alloc stream launch hints: {e}")))?;

    p2.spend(
        &mut ctx,
        funding_coin,
        Conditions::new().create_coin(stream_ph, amount, Memos::Some(launch_hints)),
    )
    .map_err(|e| ChainError::Chain(format!("spend funding coin: {e}")))?;

    let coin = Coin::new(funding_coin.coin_id(), stream_ph, amount);
    let asset = StreamedAsset::xch(coin, info);
    Ok((ctx.take(), StreamedPayment { asset }))
}

/// Reconstruct a [`StreamedPayment`] from the parent spend that created or last updated
/// it (a coinset/simulator [`CoinSpend`]), via [`StreamedAsset::from_parent_spend`].
///
/// Returns `Ok(None)` if the parent spend was a CLAWBACK (the stream was fully reclaimed
/// and no child exists) or if the parent is not a streaming coin. Used to pick up the
/// live streaming coin after a create or a prior partial claim, so the next claim spends
/// the correct child.
pub fn reconstruct_stream(parent_spend: &CoinSpend) -> Result<Option<StreamedPayment>> {
    let mut ctx = SpendContext::new();
    let (asset, _clawback, _paid) = StreamedAsset::from_parent_spend(&mut ctx, parent_spend)
        .map_err(|e| ChainError::Chain(format!("reconstruct stream: {e}")))?;
    Ok(asset.map(|asset| StreamedPayment { asset }))
}

/// Build AND sign the spend that CLAIMS the vested portion of `stream` to its recipient
/// at `payment_time`, returning a ready [`SpendBundle`].
///
/// Builds BOTH legs the claim needs in one bundle: the `message_coin` (a real spendable
/// 0-mojo coin the recipient controls at its own address) `send_message`s (mode 23) the
/// `payment_time` to the streaming coin (authorizing the release), and the streaming-coin
/// spend itself pays the newly-vested amount to the recipient and re-creates the stream
/// with the remainder. `payment_time` must be a block timestamp the consensus will accept
/// (>= the stream's `last_payment_time`, and the claim block's timestamp must reach it).
///
/// `message_coin` is supplied by the caller (e.g. a fresh 0-mojo coin at the recipient's
/// address) — the stream puzzle releases funds ONLY in response to this authorizing
/// message, so it cannot be a synthetic placeholder. `recipient` must be the address
/// whose puzzle hash matches the stream's recipient AND the `message_coin`'s puzzle hash.
/// `for_testnet` selects the signing network. **Pure: does NOT broadcast.**
pub fn build_stream_claim(
    stream: &StreamedPayment,
    recipient: &IndexedKeys,
    message_coin: Coin,
    payment_time: u64,
    for_testnet: bool,
) -> Result<chia_protocol::SpendBundle> {
    if recipient.owner_puzzle_hash != stream.asset.info.recipient {
        return Err(ChainError::Chain(
            "claim key does not match the stream's recipient puzzle hash".into(),
        ));
    }
    if message_coin.puzzle_hash != recipient.owner_puzzle_hash {
        return Err(ChainError::Chain(
            "message coin must be held at the recipient's address".into(),
        ));
    }
    if payment_time <= stream.asset.info.last_payment_time {
        return Err(ChainError::Chain(
            "claim payment_time must be after the stream's last payment time".into(),
        ));
    }

    let mut ctx = SpendContext::new();

    // The recipient sends a message authorizing the release at `payment_time`.
    let p2 = StandardLayer::new(recipient.synthetic_pk);
    let stream_coin_id = ctx
        .alloc(&stream.asset.coin.coin_id())
        .map_err(|e| ChainError::Chain(format!("alloc stream coin id: {e}")))?;
    let message: Bytes = Bytes::new(u64_to_bytes(payment_time));
    p2.spend(
        &mut ctx,
        message_coin,
        Conditions::new().send_message(23, message, vec![stream_coin_id]),
    )
    .map_err(|e| ChainError::Chain(format!("recipient message spend: {e}")))?;

    // Spend the streaming coin, paying the vested portion and re-creating the remainder.
    stream
        .asset
        .spend(&mut ctx, payment_time, false)
        .map_err(|e| ChainError::Chain(format!("spend streaming coin: {e}")))?;

    sign_bundle(
        &ctx.take(),
        std::slice::from_ref(&recipient.synthetic_sk),
        for_testnet,
    )
}

/// Build AND sign the spend that CLAWS BACK the unvested remainder of `stream` to the
/// payer (the address behind the stream's `clawback_ph`), returning a ready
/// [`SpendBundle`]. Cancels the stream: the full coin is reclaimed.
///
/// Builds BOTH legs: the `message_coin` (a real spendable 0-mojo coin the payer controls
/// at the clawback address) `send_message`s the `payment_time` to the streaming coin, and
/// the streaming-coin spend with the clawback flag set routes the remainder back to the
/// clawback puzzle hash.
///
/// `payer` must be the address whose puzzle hash matches the stream's `clawback_ph` AND
/// the `message_coin`'s puzzle hash. `for_testnet` selects the signing network.
/// **Pure: does NOT broadcast.**
pub fn build_stream_clawback(
    stream: &StreamedPayment,
    payer: &IndexedKeys,
    message_coin: Coin,
    payment_time: u64,
    for_testnet: bool,
) -> Result<chia_protocol::SpendBundle> {
    let clawback_ph = stream.asset.info.clawback_ph.ok_or_else(|| {
        ChainError::Chain("this stream has no clawback path (clawback_ph is None)".into())
    })?;
    if payer.owner_puzzle_hash != clawback_ph {
        return Err(ChainError::Chain(
            "clawback key does not match the stream's clawback puzzle hash".into(),
        ));
    }
    if message_coin.puzzle_hash != payer.owner_puzzle_hash {
        return Err(ChainError::Chain(
            "message coin must be held at the clawback address".into(),
        ));
    }

    let mut ctx = SpendContext::new();

    let p2 = StandardLayer::new(payer.synthetic_pk);
    let stream_coin_id = ctx
        .alloc(&stream.asset.coin.coin_id())
        .map_err(|e| ChainError::Chain(format!("alloc stream coin id: {e}")))?;
    let message: Bytes = Bytes::new(u64_to_bytes(payment_time));
    p2.spend(
        &mut ctx,
        message_coin,
        Conditions::new().send_message(23, message, vec![stream_coin_id]),
    )
    .map_err(|e| ChainError::Chain(format!("clawback message spend: {e}")))?;

    stream
        .asset
        .spend(&mut ctx, payment_time, true)
        .map_err(|e| ChainError::Chain(format!("clawback streaming coin: {e}")))?;

    sign_bundle(
        &ctx.take(),
        std::slice::from_ref(&payer.synthetic_sk),
        for_testnet,
    )
}

/// Sign assembled stream `coin_spends` with `keys`, returning the aggregated signature.
/// A thin convenience over [`datalayer_driver::sign_coin_spends`], mirroring
/// [`crate::nft::sign_nft_spends`].
pub fn sign_stream_spends(
    coin_spends: &[CoinSpend],
    keys: &[SecretKey],
    for_testnet: bool,
) -> Result<Signature> {
    sign_coin_spends(coin_spends, keys, for_testnet)
        .map_err(|e| ChainError::Chain(format!("sign stream spends: {e}")))
}

/// Sign `coin_spends` and wrap into a ready [`SpendBundle`].
fn sign_bundle(
    coin_spends: &[CoinSpend],
    keys: &[SecretKey],
    for_testnet: bool,
) -> Result<chia_protocol::SpendBundle> {
    let sig = sign_stream_spends(coin_spends, keys, for_testnet)?;
    Ok(chia_protocol::SpendBundle::new(coin_spends.to_vec(), sig))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::derive_indexed_keys;
    use chia_protocol::SpendBundle;
    use chia_sdk_test::Simulator;

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    // ----- offline: input validation + vesting math -----

    #[test]
    fn create_rejects_zero_amount() {
        let payer = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let funding = Coin::new(Bytes32::default(), payer.owner_puzzle_hash, 10);
        let err = build_stream_create(
            &payer,
            funding,
            Bytes32::default(),
            payer.owner_puzzle_hash,
            0,
            10,
            20,
        )
        .unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("greater than zero")),
            "got: {err}"
        );
    }

    #[test]
    fn create_rejects_bad_time_window() {
        let payer = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let funding = Coin::new(Bytes32::default(), payer.owner_puzzle_hash, 100);
        let err = build_stream_create(
            &payer,
            funding,
            Bytes32::default(),
            payer.owner_puzzle_hash,
            100,
            20,
            10, // end <= start
        )
        .unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("end_time must be greater")),
            "got: {err}"
        );
    }

    #[test]
    fn vested_amount_is_linear() {
        let payer = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let funding = Coin::new(Bytes32::from([1; 32]), payer.owner_puzzle_hash, 1_000);
        // 1000 mojos vesting from t=1000 to t=2000.
        let (_spends, stream) = build_stream_create(
            &payer,
            funding,
            Bytes32::from([2; 32]),
            payer.owner_puzzle_hash,
            1_000,
            1_000,
            2_000,
        )
        .unwrap();
        assert_eq!(stream.vested_at(1_000), 0, "nothing vested at start");
        assert_eq!(stream.vested_at(1_500), 500, "half vested at midpoint");
        assert_eq!(stream.vested_at(2_000), 1_000, "fully vested at end");
        assert_eq!(stream.vested_at(3_000), 1_000, "clamped after end");
    }

    // ----- Simulator: create -> claim vested -----

    /// Create an XCH stream from the payer (index 0) to the recipient (index 1), confirm
    /// it, advance the clock to the vesting midpoint, then have the recipient CLAIM the
    /// vested portion. The vested mojos must land at the recipient and the stream must
    /// re-create with the remainder. Drives `build_stream_create` + `build_stream_claim`
    /// end-to-end on the in-process Chia simulator.
    #[test]
    fn create_then_claim_vested() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let keys = derive_indexed_keys(ABANDON, 0..2)?;
        let payer = keys[0].clone();
        let recipient = keys[1].clone();

        // Vesting window. Start the stream at a future timestamp so the create block
        // (timestamp 0) precedes the window, then advance the clock to claim.
        let amount = 10_000u64;
        let start = 1_000u64;
        let end = 2_000u64;

        // The stream coin is created at the START timestamp so its last_payment_time
        // aligns with the block timestamp; set the create block timestamp to `start`.
        sim.set_next_timestamp(start)?;
        let funding = sim.new_coin(payer.owner_puzzle_hash, amount);
        let (create_spends, stream) = build_stream_create(
            &payer,
            funding,
            recipient.owner_puzzle_hash,
            payer.owner_puzzle_hash,
            amount,
            start,
            end,
        )?;
        let sig = sign_stream_spends(
            &create_spends,
            std::slice::from_ref(&payer.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(create_spends, sig))?;
        assert!(
            sim.coin_state(stream.asset.coin.coin_id()).is_some(),
            "the streaming coin should exist after create"
        );

        // Advance to the midpoint and claim the half-vested amount. The claim block's
        // timestamp must reach `payment_time`.
        let claim_time = 1_500u64;
        sim.set_next_timestamp(claim_time)?;
        let expected_vested = stream.vested_at(claim_time);
        assert_eq!(
            expected_vested,
            amount / 2,
            "half should be vested at midpoint"
        );

        // The recipient supplies a real 0-mojo coin at its address to carry the
        // authorizing message.
        let message_coin = sim.new_coin(recipient.owner_puzzle_hash, 0);
        let claim = build_stream_claim(&stream, &recipient, message_coin, claim_time, true)?;
        sim.new_transaction(claim)?;

        // The vested portion lands at the recipient (hinted to its address).
        let recipient_got = sim
            .unspent_coins(recipient.owner_puzzle_hash, true)
            .iter()
            .map(|c| c.amount)
            .sum::<u64>();
        assert_eq!(
            recipient_got, expected_vested,
            "the recipient should receive the vested portion"
        );
        Ok(())
    }

    // ----- Simulator: create -> clawback the remainder -----

    /// Create an XCH stream, advance partway, then have the PAYER claw back the unvested
    /// remainder (cancel). Drives `build_stream_create` + `build_stream_clawback` on the
    /// simulator and asserts the stream coin is consumed (fully reclaimed).
    #[test]
    fn create_then_clawback_remainder() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let keys = derive_indexed_keys(ABANDON, 0..2)?;
        let payer = keys[0].clone();
        let recipient = keys[1].clone();

        let amount = 8_000u64;
        let start = 1_000u64;
        let end = 2_000u64;

        sim.set_next_timestamp(start)?;
        let funding = sim.new_coin(payer.owner_puzzle_hash, amount);
        let (create_spends, stream) = build_stream_create(
            &payer,
            funding,
            recipient.owner_puzzle_hash,
            payer.owner_puzzle_hash, // payer is the clawback address
            amount,
            start,
            end,
        )?;
        let sig = sign_stream_spends(
            &create_spends,
            std::slice::from_ref(&payer.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(create_spends, sig))?;

        // Claw back at the quarter point. The payer supplies a real 0-mojo coin at the
        // clawback address to carry the authorizing message.
        let claw_time = 1_250u64;
        sim.set_next_timestamp(claw_time)?;
        let message_coin = sim.new_coin(payer.owner_puzzle_hash, 0);
        let clawback = build_stream_clawback(&stream, &payer, message_coin, claw_time, true)?;
        sim.new_transaction(clawback)?;

        // The streaming coin has been spent (clawed back); it is no longer unspent.
        let still_unspent = sim
            .unspent_coins(stream.asset.coin.puzzle_hash, false)
            .iter()
            .any(|c| c.coin_id() == stream.asset.coin.coin_id());
        assert!(
            !still_unspent,
            "the streaming coin should be consumed by the clawback"
        );
        Ok(())
    }
}
