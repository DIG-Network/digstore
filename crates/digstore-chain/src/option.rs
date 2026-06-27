//! Option contracts — the wallet's Sage-parity option surface: create a covered
//! option over an underlying the wallet locks, let the holder EXERCISE it (pay the
//! strike, claim the underlying) before expiry, and let the creator CLAW BACK
//! (cancel / reclaim on expiry) the underlying afterward.
//!
//! Built on the chia-wallet-sdk 0.30 option primitive
//! ([`OptionLauncher`]/[`OptionContract`]/[`OptionUnderlying`]). Like [`crate::nft`]
//! and [`crate::clawback`], this is **pure build (+ sign)**: builders return UNSIGNED
//! `Vec<CoinSpend>` (or, where they hold all keys, a signed [`SpendBundle`]); NOTHING
//! here broadcasts. Signing + pushing stay the caller's gated decision (the dig-wallet
//! `DIG_WALLET_ALLOW_BROADCAST` gate).
//!
//! ## Shape (CHIP-0042 options)
//! An option is a singleton (the option "ticket") plus a separate `OptionUnderlying`
//! coin that holds the locked asset under a 1-of-2 path:
//!   * **exercise** — the option singleton's holder pays the `strike_type` asset into
//!     the settlement puzzle and unlocks the underlying to itself, valid until
//!     `expiration_seconds`; and
//!   * **clawback** — after expiry, the creator reclaims the locked underlying.
//!
//! ## Scope of this module
//! The primitive supports XCH / CAT / revocable-CAT / NFT for BOTH the underlying and
//! the strike. This module implements the fully-verifiable, wallet-common case:
//! **an XCH-locked underlying** with a configurable **strike type** (the asset the
//! holder must pay to exercise). The create→exercise round-trip and the
//! create→clawback (cancel/expire) flow are proven on the simulator for an XCH strike.
//! CAT/NFT *underlying* legs use the same `OptionUnderlying::exercise_spend` /
//! `clawback_spend` primitives but require the caller to supply the locked CAT/NFT coin
//! and wrap the spend accordingly (see the gap note on [`build_option_exercise`]).

use crate::error::{ChainError, Result};
use crate::keys::IndexedKeys;
use chia::puzzles::Memos;
use chia_protocol::{Coin, CoinSpend};
use chia_wallet_sdk::driver::{
    OptionContract, OptionLauncher, OptionLauncherInfo, OptionType, OptionUnderlying,
    SingletonInfo, SpendContext, StandardLayer,
};
use chia_wallet_sdk::types::Conditions;
use datalayer_driver::{sign_coin_spends, SecretKey, Signature};

/// A created option, returned by [`build_option_create`] so the caller can later
/// exercise or claw it back (both need the option singleton, the underlying terms, and
/// the locked-underlying coin, which only exist once the create spend is confirmed).
#[derive(Clone, Debug)]
pub struct CreatedOption {
    /// The option singleton (the transferable "ticket"); its holder may exercise it.
    pub option: OptionContract,
    /// The underlying terms (launcher id, creator ph, expiry seconds, locked amount,
    /// strike type) — needed to build the exercise / clawback spend.
    pub underlying: OptionUnderlying,
    /// The locked-underlying XCH coin (parent = the funding coin, puzzle hash = the
    /// underlying's 1-of-2 path, amount = the locked amount).
    pub underlying_coin: Coin,
}

/// Build the (UNSIGNED) coin spends that CREATE an option: lock `underlying_amount`
/// mojos of XCH (from `funding_coin`) as the underlying, and mint the option singleton
/// to `creator` (the holder/owner), exercisable for `strike_type` until `expiry_seconds`.
///
/// Two coins are funded from `funding_coin` in one bundle: the locked-underlying coin
/// (at the option's `p2_puzzle_hash` 1-of-2 path) and the option singleton launcher.
/// `funding_coin` must hold at least `underlying_amount + 1` mojos (the underlying plus
/// the 1-mojo singleton); any excess is left as an implicit fee.
///
/// **Pure: does NOT sign or broadcast.** `creator`'s synthetic key authorizes the
/// funding-coin spend; the caller signs the assembled bundle (see [`sign_option_spends`]).
pub fn build_option_create(
    creator: &IndexedKeys,
    funding_coin: Coin,
    underlying_amount: u64,
    strike_type: OptionType,
    expiry_seconds: u64,
) -> Result<(Vec<CoinSpend>, CreatedOption)> {
    if underlying_amount == 0 {
        return Err(ChainError::Chain(
            "option underlying amount must be greater than zero".into(),
        ));
    }
    let needed = underlying_amount.checked_add(1).ok_or_else(|| {
        ChainError::Chain("underlying amount overflows the singleton mojo".into())
    })?;
    if funding_coin.amount < needed {
        return Err(ChainError::Chain(format!(
            "funding coin amount {} is too small: need {needed} (underlying {underlying_amount} + 1 mojo singleton)",
            funding_coin.amount
        )));
    }

    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(creator.synthetic_pk);

    // Build the launcher off the funding coin. The launcher amount is the 1-mojo
    // singleton; the creator is both the option creator (clawback path) and owner.
    let launcher = OptionLauncher::new(
        &mut ctx,
        funding_coin.coin_id(),
        OptionLauncherInfo::new(
            creator.owner_puzzle_hash,
            creator.owner_puzzle_hash,
            expiry_seconds,
            underlying_amount,
            strike_type,
        ),
        1,
    )
    .map_err(|e| ChainError::Chain(format!("create option launcher: {e}")))?;

    let underlying = launcher.underlying();
    let p2_option = launcher.p2_puzzle_hash();

    // Lock the underlying XCH at the option's 1-of-2 path AND create the launcher coin,
    // both funded by the single funding-coin spend.
    let underlying_coin = Coin::new(funding_coin.coin_id(), p2_option, underlying_amount);
    let launcher = launcher.with_underlying(underlying_coin.coin_id());
    let (mint_conditions, option) = launcher
        .mint(&mut ctx)
        .map_err(|e| ChainError::Chain(format!("mint option: {e}")))?;

    let conditions = mint_conditions.create_coin(p2_option, underlying_amount, Memos::None);
    p2.spend(&mut ctx, funding_coin, conditions)
        .map_err(|e| ChainError::Chain(format!("spend funding coin: {e}")))?;

    Ok((
        ctx.take(),
        CreatedOption {
            option,
            underlying,
            underlying_coin,
        },
    ))
}

/// Build the (UNSIGNED) coin spends that EXERCISE `created` by its holder `owner`:
/// spend the option singleton through its exercise path and unlock the underlying XCH
/// coin to the option's inner puzzle (the holder). Returns the spends.
///
/// This builds ONLY the option-singleton + underlying-unlock legs. The holder must
/// ALSO pay the strike (the `strike_type` asset) into the settlement puzzle in the SAME
/// bundle for the exercise to be valid — that strike payment is the caller's
/// responsibility (it depends on which strike asset the holder funds with; for an XCH
/// strike, send `underlying.requested_payment(...)` into the settlement coin). The
/// exercise is rejected by the consensus after `expiry_seconds`.
///
/// **Gap (honest):** this builder covers an **XCH-locked underlying** (unlocked via
/// [`OptionUnderlying::exercise_coin_spend`]). A CAT/NFT underlying uses
/// [`OptionUnderlying::exercise_spend`] wrapped in a `CatSpend`/`Nft::spend`, which
/// needs the caller to pass the locked CAT/NFT coin; that wrapping is not built here.
///
/// **Pure: does NOT sign or broadcast.** `owner`'s synthetic key authorizes the option
/// singleton spend.
pub fn build_option_exercise(
    created: &CreatedOption,
    owner: &IndexedKeys,
) -> Result<Vec<CoinSpend>> {
    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(owner.synthetic_pk);

    // Spend the option singleton through its exercise path (the holder authorizes it).
    // `OptionContract` is `Copy`, so passing `created.option` copies rather than moves.
    created
        .option
        .exercise(&mut ctx, &p2, Conditions::new())
        .map_err(|e| ChainError::Chain(format!("exercise option singleton: {e}")))?;

    // Unlock the locked underlying XCH to the option's inner puzzle hash.
    created
        .underlying
        .exercise_coin_spend(
            &mut ctx,
            created.underlying_coin,
            created.option.info.inner_puzzle_hash().into(),
            created.option.coin.amount,
        )
        .map_err(|e| ChainError::Chain(format!("exercise underlying coin: {e}")))?;

    Ok(ctx.take())
}

/// Build the (UNSIGNED) coin spends that CLAW BACK (cancel / reclaim on expiry) the
/// locked underlying XCH of `created` to `creator`, recovering `underlying_amount` mojos
/// to `creator.owner_puzzle_hash`.
///
/// The creator spends the underlying's clawback path, which is valid only AFTER
/// `expiry_seconds` (the option holder had until expiry to exercise). This recovers an
/// **XCH-locked underlying**; a CAT/NFT underlying uses the same
/// [`OptionUnderlying::clawback_spend`] wrapped for that asset (see the exercise gap note).
///
/// **Pure: does NOT sign or broadcast.** `creator`'s synthetic key authorizes the
/// clawback inner spend.
pub fn build_option_clawback(
    created: &CreatedOption,
    creator: &IndexedKeys,
) -> Result<Vec<CoinSpend>> {
    use chia_wallet_sdk::driver::SpendWithConditions;

    if creator.owner_puzzle_hash != created.underlying.creator_puzzle_hash {
        return Err(ChainError::Chain(
            "clawback key does not match the option's creator puzzle hash".into(),
        ));
    }

    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(creator.synthetic_pk);

    // The creator recovers the locked underlying to its own address.
    let inner = p2
        .spend_with_conditions(
            &mut ctx,
            Conditions::new().create_coin(
                creator.owner_puzzle_hash,
                created.underlying_coin.amount,
                Memos::None,
            ),
        )
        .map_err(|e| ChainError::Chain(format!("creator inner spend: {e}")))?;

    created
        .underlying
        .clawback_coin_spend(&mut ctx, created.underlying_coin, inner)
        .map_err(|e| ChainError::Chain(format!("clawback underlying coin: {e}")))?;

    Ok(ctx.take())
}

/// Sign assembled option `coin_spends` with `keys` (the wallet's synthetic secret
/// keys), returning the aggregated signature. A thin convenience over
/// [`datalayer_driver::sign_coin_spends`], mirroring [`crate::nft::sign_nft_spends`].
pub fn sign_option_spends(
    coin_spends: &[CoinSpend],
    keys: &[SecretKey],
    for_testnet: bool,
) -> Result<Signature> {
    sign_coin_spends(coin_spends, keys, for_testnet)
        .map_err(|e| ChainError::Chain(format!("sign option spends: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::derive_indexed_keys;
    use chia::puzzles::offer::SettlementPaymentsSolution;
    use chia_protocol::{Bytes32, SpendBundle};
    use chia_puzzles::SETTLEMENT_PAYMENT_HASH;
    use chia_sdk_test::Simulator;
    use chia_wallet_sdk::driver::{Layer, SettlementLayer};

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    // ----- offline: input validation -----

    #[test]
    fn create_rejects_zero_underlying() {
        let creator = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let funding = Coin::new(Bytes32::default(), creator.owner_puzzle_hash, 10);
        let err = build_option_create(&creator, funding, 0, OptionType::Xch { amount: 1 }, 10)
            .unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("greater than zero")),
            "got: {err}"
        );
    }

    #[test]
    fn create_rejects_funding_too_small() {
        let creator = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        // need underlying(10) + 1 = 11, provide 10.
        let funding = Coin::new(Bytes32::default(), creator.owner_puzzle_hash, 10);
        let err = build_option_create(&creator, funding, 10, OptionType::Xch { amount: 1 }, 10)
            .unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("too small")),
            "got: {err}"
        );
    }

    #[test]
    fn clawback_rejects_wrong_creator_key() -> anyhow::Result<()> {
        // Create a real option so the CreatedOption is well-formed, then attempt to
        // claw it back with a DIFFERENT key — must be rejected up front.
        let mut sim = Simulator::new();
        let keys = derive_indexed_keys(ABANDON, 0..2)?;
        let creator = keys[0].clone();
        let stranger = keys[1].clone();
        let funding = sim.new_coin(creator.owner_puzzle_hash, 1_001);
        let (_spends, created) =
            build_option_create(&creator, funding, 1_000, OptionType::Xch { amount: 1 }, 10)?;
        let err = build_option_clawback(&created, &stranger).unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("creator puzzle hash")),
            "got: {err}"
        );
        Ok(())
    }

    // ----- Simulator: create -> exercise round-trip -----

    /// Create an XCH-locked option (XCH strike) for the wallet, confirm it, then
    /// EXERCISE it: the holder pays the strike into the settlement puzzle and unlocks
    /// the underlying to itself. Drives `build_option_create` + `build_option_exercise`
    /// end-to-end on the in-process Chia simulator (the option_contract.rs test shape).
    #[test]
    fn create_then_exercise_round_trip() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let creator = derive_indexed_keys(ABANDON, 0..1)?[0].clone();
        let underlying_amount = 1_000u64;
        let strike_amount = 250u64;
        let expiry = 1_000u64;

        // Fund: lock 1000 mojos underlying + 1 mojo singleton, and a separate coin to
        // pay the XCH strike into the settlement puzzle on exercise.
        let funding = sim.new_coin(creator.owner_puzzle_hash, underlying_amount + 1);
        let (create_spends, created) = build_option_create(
            &creator,
            funding,
            underlying_amount,
            OptionType::Xch {
                amount: strike_amount,
            },
            expiry,
        )?;
        assert_eq!(created.underlying.amount, underlying_amount);
        assert_eq!(created.underlying_coin.amount, underlying_amount);

        let sig = sign_option_spends(
            &create_spends,
            std::slice::from_ref(&creator.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(create_spends, sig))?;
        assert!(
            sim.coin_state(created.option.coin.coin_id()).is_some(),
            "the option singleton should exist after create"
        );

        // Exercise: build the option + underlying-unlock legs, and ALSO pay the XCH
        // strike into the settlement puzzle (the caller's responsibility, per docs).
        let mut exercise_spends = build_option_exercise(&created, &creator)?;

        // Fund the XCH strike from a fresh coin -> settlement puzzle, then settle it to
        // the option's requested payment.
        let strike_funding = sim.new_coin(creator.owner_puzzle_hash, strike_amount);
        let ctx = &mut SpendContext::new();
        let p2 = StandardLayer::new(creator.synthetic_pk);
        p2.spend(
            ctx,
            strike_funding,
            Conditions::new().create_coin(
                SETTLEMENT_PAYMENT_HASH.into(),
                strike_amount,
                Memos::None,
            ),
        )?;
        let settlement_coin = Coin::new(
            strike_funding.coin_id(),
            SETTLEMENT_PAYMENT_HASH.into(),
            strike_amount,
        );
        let payment = created.underlying.requested_payment(&mut **ctx)?;
        let coin_spend = SettlementLayer.construct_coin_spend(
            ctx,
            settlement_coin,
            SettlementPaymentsSolution::new(vec![payment]),
        )?;
        ctx.insert(coin_spend);
        exercise_spends.extend(ctx.take());

        let sig = sign_option_spends(
            &exercise_spends,
            std::slice::from_ref(&creator.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(exercise_spends, sig))?;

        // After exercise, the unlocked underlying lands at the holder. The strike
        // payment landed at the creator's requested address (here, the creator IS the
        // holder, so its address gains the underlying + strike change).
        let owned = sim
            .unspent_coins(creator.owner_puzzle_hash, true)
            .iter()
            .map(|c| c.amount)
            .sum::<u64>();
        assert!(owned > 0, "the holder should own coins after exercise");
        Ok(())
    }

    // ----- Simulator: create -> clawback (cancel / expire) -----

    /// Create an option, let it expire (advance past `expiry_seconds`), then have the
    /// CREATOR claw back the locked underlying. The recovered mojos return to the
    /// creator. Drives `build_option_create` + `build_option_clawback` on the simulator.
    #[test]
    fn create_then_clawback_on_expiry() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let creator = derive_indexed_keys(ABANDON, 0..1)?[0].clone();
        let underlying_amount = 2_000u64;
        let expiry = 100u64;

        let funding = sim.new_coin(creator.owner_puzzle_hash, underlying_amount + 1);
        let (create_spends, created) = build_option_create(
            &creator,
            funding,
            underlying_amount,
            OptionType::Xch { amount: 1 },
            expiry,
        )?;
        let sig = sign_option_spends(
            &create_spends,
            std::slice::from_ref(&creator.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(create_spends, sig))?;

        // Advance past expiry so the clawback path's AssertBeforeSeconds boundary lets
        // the creator reclaim the underlying.
        sim.pass_time(expiry + 10);

        let clawback_spends = build_option_clawback(&created, &creator)?;
        let sig = sign_option_spends(
            &clawback_spends,
            std::slice::from_ref(&creator.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(clawback_spends, sig))?;

        // The reclaimed underlying returns to the creator's address.
        let recovered = sim
            .unspent_coins(creator.owner_puzzle_hash, false)
            .iter()
            .map(|c| c.amount)
            .sum::<u64>();
        assert_eq!(
            recovered, underlying_amount,
            "the creator should reclaim the locked underlying on expiry"
        );
        Ok(())
    }
}
