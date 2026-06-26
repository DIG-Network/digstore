//! Native offer acceptance — the wallet's side of taking a Chia offer
//! (`chia_takeOffer`), so the hub's supporter-badge mint flow works against the
//! DIG Browser's built-in wallet.
//!
//! A MintGarden badge offer is a bech32 `offer1…` string in which the maker
//! **offers an NFT** (the badge) and **requests a DIG-CAT payment** (the badge
//! price). Accepting it is a single atomic settlement: the wallet funds the
//! requested DIG (and any fee), the offered NFT routes to the wallet, and the
//! combined spend bundle is broadcast as one transaction.
//!
//! This module is **pure build + sign** — exactly like [`crate::send`] and
//! [`crate::cat`]. It NEVER broadcasts. The decoded maker side already carries the
//! maker's signature; we build the taker's side over the canonical
//! `chia-wallet-sdk` action/settlement system, sign only the taker's own coins
//! (AugScheme, the same path as the canonical send/CHIP-0002 signer), and
//! aggregate the two into one ready-to-broadcast [`SpendBundle`] via
//! [`Offer::take`]. Pushing it is the caller's gated decision.
//!
//! The taker's funding coins (DIG CATs with lineage proofs, XCH coins) come from a
//! live coinset scan — the same reconstruction [`crate::cat::dig_cats`] does — so
//! the full end-to-end build over real chain data is validated live, while the
//! decode/parse/validation core is unit-tested offline (and the full settlement
//! round-trip is proven against the in-crate simulator).

use crate::error::{ChainError, Result};
use crate::keys::IndexedKeys;
use chia_protocol::{Bytes32, Coin, SpendBundle};
use chia_wallet_sdk::driver::{
    decode_offer, encode_offer, Action, Cat, Offer, Relation, SpendContext, Spends,
};
use datalayer_driver::{sign_coin_spends, PublicKey, SecretKey};
use indexmap::IndexMap;

/// What the wallet must pay to take an offer, derived from the offer's arbitrage
/// (requested minus offered). All amounts are the asset's base units (mojos for
/// XCH, base units for CATs). Used to validate the wallet can fund the take and to
/// surface a clear, specific shortfall before any spend is built.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OfferCost {
    /// XCH (mojos) the wallet must pay (the requested-over-offered surplus).
    pub xch: u64,
    /// Per-asset-id CAT base units the wallet must pay (e.g. the DIG badge price).
    pub cats: Vec<(Bytes32, u64)>,
}

/// The wallet's funding coins for taking an offer: its spendable XCH coins and its
/// DIG/CAT coins (with lineage proofs, as reconstructed by [`crate::cat::dig_cats`]),
/// each tagged with the synthetic keypair of the address that holds it so the inner
/// spend can be authorized.
pub struct TakerFunds<'a> {
    /// XCH coins available to fund the requested XCH and the fee, each tagged with
    /// its address's keys.
    pub xch: Vec<(Coin, &'a IndexedKeys)>,
    /// CAT coins available to fund requested CAT payments (e.g. DIG), each tagged
    /// with its address's keys. `Cat` carries its own lineage proof + asset id.
    pub cats: Vec<(Cat, &'a IndexedKeys)>,
}

/// Decode a bech32 `offer1…` string into the maker's [`Offer`].
///
/// Errors (with a specific message) if the string is not a valid, current-format
/// Chia offer — so the hub surfaces an honest "this isn't an offer" rather than a
/// cryptic decode failure deep in a spend build.
pub fn decode_offer_string(offer: &str) -> Result<Offer> {
    let trimmed = offer.trim();
    if !trimmed.starts_with("offer1") {
        return Err(ChainError::Chain(
            "not a Chia offer: expected a bech32 string starting with 'offer1'".into(),
        ));
    }
    let spend_bundle =
        decode_offer(trimmed).map_err(|e| ChainError::Chain(format!("invalid offer: {e}")))?;
    // SpendContext derefs to Allocator, which is what `from_spend_bundle` needs.
    let mut ctx = SpendContext::new();
    Offer::from_spend_bundle(&mut ctx, &spend_bundle)
        .map_err(|e| ChainError::Chain(format!("could not parse offer: {e}")))
}

/// Re-encode an [`Offer`]'s combined spend bundle back to a bech32 `offer1…` string.
/// Used by callers that want to hand the assembled take back out as an offer.
pub fn encode_offer_bundle(spend_bundle: &SpendBundle) -> Result<String> {
    encode_offer(spend_bundle)
        .map_err(|e| ChainError::Chain(format!("could not encode offer: {e}")))
}

/// The wallet's cost to take `offer`: the requested-over-offered surplus the taker
/// must fund (XCH mojos + per-CAT base units). NFTs/options the wallet receives are
/// not a cost; NFTs/options the wallet would have to give up are not expressible by
/// the badge flow and are surfaced as an error by [`take_offer`].
pub fn offer_cost(offer: &Offer) -> OfferCost {
    let arb = offer.arbitrage();
    OfferCost {
        xch: arb.offered.xch,
        cats: arb.offered.cats.iter().map(|(k, v)| (*k, *v)).collect(),
    }
}

/// Build AND sign the wallet's side of taking the bech32 `offer1…` string `offer_str`,
/// returning the combined, ready-to-broadcast [`SpendBundle`] (maker + taker spends,
/// aggregated signature).
///
/// `funds` are the wallet's spendable XCH + CAT coins (from a live scan).
/// `change_keys` is the address that surplus/change AND the received NFT return to
/// (the wallet's primary address) — passed explicitly (not searched in `funds`) so
/// the take still works when the change address itself holds no funding coins. `fee`
/// is an optional network fee in mojos. `for_testnet` selects the `agg_sig_me`
/// network for signing (mainnet in production; the simulator test sets it true).
///
/// The offer is decoded and consumed in a single [`SpendContext`]: the parsed offer
/// holds allocator-relative pointers (the NFT metadata `HashedPtr`), so reconstructing
/// the offered NFT during the take MUST share that allocator — hence this takes the
/// string and decodes internally rather than a pre-decoded [`Offer`].
///
/// **Pure: does NOT broadcast.** Errors — before building any spend — if the string
/// is not a valid offer, if it asks the wallet to give up an NFT/option (not the
/// badge flow), or if the wallet's funds can't cover the requested payment.
pub fn take_offer(
    offer_str: &str,
    funds: TakerFunds<'_>,
    change_keys: &IndexedKeys,
    fee: u64,
    for_testnet: bool,
) -> Result<SpendBundle> {
    let trimmed = offer_str.trim();
    if !trimmed.starts_with("offer1") {
        return Err(ChainError::Chain(
            "not a Chia offer: expected a bech32 string starting with 'offer1'".into(),
        ));
    }
    let spend_bundle =
        decode_offer(trimmed).map_err(|e| ChainError::Chain(format!("invalid offer: {e}")))?;

    // One SpendContext (= one allocator) for decode AND the take build, so the
    // offer's parsed NodePtrs stay valid when we reconstruct the offered NFT.
    let mut ctx = SpendContext::new();
    let offer = Offer::from_spend_bundle(&mut ctx, &spend_bundle)
        .map_err(|e| ChainError::Chain(format!("could not parse offer: {e}")))?;

    // The badge flow only ever asks the wallet to PAY (XCH/CAT) for received
    // assets. If the offer would make the wallet give up an NFT/option, that is not
    // this flow — refuse before building anything rather than produce a spend the
    // wallet can't sign.
    let arb = offer.arbitrage();
    if !arb.offered.nfts.is_empty() || !arb.offered.options.is_empty() {
        return Err(ChainError::Chain(
            "this offer asks the wallet to give up an NFT/option, which the native wallet \
             cannot take (only paying XCH/CAT for received assets is supported)"
                .into(),
        ));
    }

    // Build the taker side: change/surplus returns to the wallet, and the offered
    // coins (the badge NFT) are claimed by spending the maker's settlement coins.
    let change_ph = change_keys.owner_puzzle_hash;
    let mut spends = Spends::new(change_ph);
    spends.add(offer.offered_coins().clone());

    // The wallet's funding coins. `finish_with_keys` looks each input's inner
    // (p2) puzzle hash up in the key map, so record every participating address's
    // synthetic key as we add its coins. The change address's key is seeded first
    // so change/NFT routing to it always has a key, even if it funds nothing.
    let mut key_map: IndexMap<Bytes32, PublicKey> = IndexMap::new();
    key_map.insert(change_ph, change_keys.synthetic_pk);
    for (coin, keys) in &funds.xch {
        spends.add(*coin);
        key_map.insert(keys.owner_puzzle_hash, keys.synthetic_pk);
    }
    for (cat, keys) in &funds.cats {
        spends.add(*cat);
        key_map.insert(keys.owner_puzzle_hash, keys.synthetic_pk);
    }

    // Settle exactly what the maker requested (the DIG price, plus any XCH leg),
    // funded from the wallet's coins added above; add the network fee if any.
    let mut actions = offer.requested_payments().actions();
    if fee > 0 {
        actions.push(Action::fee(fee));
    }

    let deltas = spends
        .apply(&mut ctx, &actions)
        .map_err(|e| ChainError::Chain(format!("apply take-offer actions: {e}")))?;

    spends
        .finish_with_keys(&mut ctx, &deltas, Relation::AssertConcurrent, &key_map)
        .map_err(|e| ChainError::Chain(format!("finish take-offer spends: {e}")))?;

    // Sign ONLY the taker's coins (the maker's half is already signed inside the
    // decoded offer). datalayer_driver maps each raw + synthetic key, so the
    // wallet's standard-puzzle coins are covered. mainnet agg_sig in production.
    let taker_coin_spends = ctx.take();
    let signing_keys: Vec<SecretKey> = signing_keys(&funds);
    let taker_signature = sign_coin_spends(&taker_coin_spends, &signing_keys, for_testnet)
        .map_err(|e| ChainError::Chain(format!("sign take-offer spends: {e}")))?;
    let taker_bundle = SpendBundle::new(taker_coin_spends, taker_signature);

    // Aggregate the maker's signed half with the taker's into one bundle.
    Ok(offer.take(taker_bundle))
}

/// The distinct synthetic secret keys for all funding coins (the taker's signing
/// set). De-duplicated by owner puzzle hash so a key is not added twice when an
/// address funds with both XCH and CAT coins.
fn signing_keys(funds: &TakerFunds<'_>) -> Vec<SecretKey> {
    let mut seen: IndexMap<Bytes32, SecretKey> = IndexMap::new();
    for (_, k) in &funds.xch {
        seen.entry(k.owner_puzzle_hash)
            .or_insert_with(|| k.synthetic_sk.clone());
    }
    for (_, k) in &funds.cats {
        seen.entry(k.owner_puzzle_hash)
            .or_insert_with(|| k.synthetic_sk.clone());
    }
    seen.into_values().collect()
}

/// The result of building (and, if requested, broadcasting) a take-offer.
#[derive(Debug)]
pub struct TakenOffer {
    /// The combined, signed maker+taker spend bundle (ready to broadcast).
    pub bundle: SpendBundle,
    /// What the wallet paid to take it (the requested-over-offered surplus).
    pub cost: OfferCost,
}

/// High-level orchestration for the wallet: scan the HD wallet over `chain`,
/// reconstruct its funding coins (XCH + DIG CATs with lineage proofs), and build +
/// sign the taker side of `offer_str`. **Pure: does NOT broadcast** — returns the
/// signed bundle (+ cost) for the caller to push under its own safety gate, exactly
/// like [`crate::send::build_xch_send`].
///
/// `for_testnet` selects the signing network (mainnet in production).
pub async fn build_take_offer(
    chain: &dyn crate::coinset::ChainReads,
    mnemonic: &str,
    offer_str: &str,
    fee: u64,
    for_testnet: bool,
) -> Result<TakenOffer> {
    // The wallet's primary owner address is the change/receive target.
    let primary = crate::keys::derive_indexed_keys(mnemonic, 0..1)?
        .into_iter()
        .next()
        .ok_or_else(|| ChainError::Chain("could not derive wallet key".into()))?;

    // Scan the HD wallet for spendable XCH + DIG, then reconstruct each DIG-bearing
    // address's CATs WITH lineage proofs (raw scan coins lack the proof needed to
    // spend). Every funding coin is tagged with its address's key for signing.
    let scanned = crate::wallet::scan_wallet(chain, mnemonic).await?;

    let mut xch: Vec<(Coin, &IndexedKeys)> = Vec::new();
    for a in &scanned.addrs {
        for c in &a.xch {
            xch.push((*c, &a.keys));
        }
    }

    // Reconstruct DIG CATs (lineage proofs) per DIG-bearing address, tagging each
    // with the key of the address whose DIG CAT puzzle hash it lives at.
    let mut cats: Vec<(Cat, &IndexedKeys)> = Vec::new();
    for a in &scanned.addrs {
        if a.dig.is_empty() {
            continue;
        }
        let reconstructed = crate::cat::dig_cats_for(chain, a.keys.owner_puzzle_hash).await?;
        for cat in reconstructed {
            cats.push((cat, &a.keys));
        }
    }

    let funds = TakerFunds { xch, cats };
    let bundle = take_offer(offer_str, funds, &primary, fee, for_testnet)?;

    // Recompute the cost from the offer for the caller's response (cheap re-decode).
    let cost = offer_cost(&decode_offer_string(offer_str)?);

    Ok(TakenOffer { bundle, cost })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::derive_indexed_keys;
    use chia::puzzles::offer::{NotarizedPayment, Payment};
    use chia::puzzles::Memos;
    use chia_puzzles::SETTLEMENT_PAYMENT_HASH;
    use chia_sdk_test::Simulator;
    use chia_wallet_sdk::driver::{
        Action, AssetInfo, Cat, CatAssetInfo, Id, RequestedPayments, StandardLayer,
    };
    use chia_wallet_sdk::types::Conditions;
    use indexmap::indexmap;

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    // ----- offline: decode + validation core -----

    #[test]
    fn decode_rejects_non_offer_strings() {
        // A bare non-offer string is rejected up front with a clear message.
        let err = decode_offer_string("hello world").unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("not a Chia offer")),
            "got: {err}"
        );
        // The empty string is not an offer either.
        assert!(decode_offer_string("   ").is_err());
        // A well-prefixed but malformed offer fails at decode (not a panic).
        let err = decode_offer_string("offer1qqzh3w"); // bad bech32 payload
        assert!(err.is_err());
    }

    /// Build a real offer in the simulator: `maker` mints a royalty NFT and OFFERS
    /// it, REQUESTING `price` units of the CAT identified by `cat_asset_id`, paid to
    /// the maker. Returns the bech32 `offer1…` string.
    ///
    /// This mirrors the canonical NFT-for-NFT take test in chia-sdk-driver, but the
    /// requested side is a CAT payment (the badge price), which is exactly the
    /// MintGarden badge shape (NFT offered, DIG-CAT requested).
    #[allow(clippy::type_complexity)]
    fn make_nft_for_cat_offer(
        sim: &mut Simulator,
        ctx: &mut SpendContext,
        cat_asset_id: Bytes32,
        price: u64,
    ) -> anyhow::Result<String> {
        let maker = sim.bls(1);
        let maker_hint = ctx.hint(maker.puzzle_hash)?;

        // Mint a royalty NFT owned by the maker (the "badge").
        let mut spends = Spends::new(maker.puzzle_hash);
        spends.add(maker.coin);
        let deltas = spends.apply(
            ctx,
            &[Action::mint_empty_royalty_nft(maker.puzzle_hash, 300)],
        )?;
        let outputs = spends.finish_with_keys(
            ctx,
            &deltas,
            Relation::AssertConcurrent,
            &indexmap! { maker.puzzle_hash => maker.pk },
        )?;
        let nft = outputs.nfts[&Id::New(0)];
        sim.spend_coins(ctx.take(), std::slice::from_ref(&maker.sk))?;

        // The maker offers the NFT, requesting `price` of the CAT paid to itself.
        let mut requested_payments = RequestedPayments::new();
        let mut requested_asset_info = AssetInfo::new();
        requested_payments.cats.insert(
            cat_asset_id,
            vec![NotarizedPayment::new(
                Offer::nonce(vec![nft.coin.coin_id()]),
                vec![Payment::new(maker.puzzle_hash, price, maker_hint)],
            )],
        );
        requested_asset_info.insert_cat(cat_asset_id, CatAssetInfo::new(None))?;

        // Spend the NFT into the settlement puzzle, asserting the requested payment.
        let mut spends = Spends::new(maker.puzzle_hash);
        spends.add(nft);
        let deltas = spends.apply(
            ctx,
            &[Action::send(
                Id::Existing(nft.info.launcher_id),
                SETTLEMENT_PAYMENT_HASH.into(),
                1,
                Memos::None,
            )],
        )?;
        spends.conditions.required = spends
            .conditions
            .required
            .extend(requested_payments.assertions(ctx, &requested_asset_info)?);
        spends.finish_with_keys(
            ctx,
            &deltas,
            Relation::AssertConcurrent,
            &indexmap! { maker.puzzle_hash => maker.pk },
        )?;

        let coin_spends = ctx.take();
        let signature = chia_sdk_test::sign_transaction(&coin_spends, &[maker.sk])?;
        let offer = Offer::from_input_spend_bundle(
            ctx,
            SpendBundle::new(coin_spends, signature),
            requested_payments,
            requested_asset_info,
        )?;
        Ok(encode_offer_bundle(&offer.to_spend_bundle(ctx)?)?)
    }

    /// Issue `amount` of a fresh CAT to the taker's owner puzzle hash in the
    /// simulator, returning the spendable [`Cat`] (with lineage proof) and its
    /// asset id. The taker funds the offer's requested CAT payment with this.
    fn issue_cat_to(
        sim: &mut Simulator,
        ctx: &mut SpendContext,
        owner_ph: Bytes32,
        owner_pk: PublicKey,
        owner_sk: &SecretKey,
        amount: u64,
    ) -> anyhow::Result<(Cat, Bytes32)> {
        // Fund issuance from an XCH coin the taker controls (standard puzzle of pk).
        let xch = sim.new_coin(owner_ph, amount);
        let p2 = StandardLayer::new(owner_pk);
        let hint = ctx.hint(owner_ph)?;
        let (issue_cat, cats) = Cat::issue_with_coin(
            ctx,
            xch.coin_id(),
            amount,
            Conditions::new().create_coin(owner_ph, amount, hint),
        )?;
        p2.spend(ctx, xch, issue_cat)?;
        let asset_id = cats[0].info.asset_id;
        sim.spend_coins(ctx.take(), std::slice::from_ref(owner_sk))?;
        Ok((cats[0], asset_id))
    }

    #[test]
    fn take_nft_for_cat_offer_assembles_a_valid_bundle() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let mut ctx = SpendContext::new();

        // The taker is the native wallet, keyed from the ABANDON vector (index 0).
        let taker = derive_indexed_keys(ABANDON, 0..1)?[0].clone();
        let price: u64 = 100_000; // a DIG-like CAT badge price (base units)

        // Give the taker a CAT to pay with, then make a maker offer requesting it.
        let (taker_cat, cat_asset_id) = issue_cat_to(
            &mut sim,
            &mut ctx,
            taker.owner_puzzle_hash,
            taker.synthetic_pk,
            &taker.synthetic_sk,
            price,
        )?;
        let offer_str = make_nft_for_cat_offer(&mut sim, &mut ctx, cat_asset_id, price)?;

        // Decode + inspect: the wallet's cost is exactly the requested CAT price.
        let preview = decode_offer_string(&offer_str)?;
        let cost = offer_cost(&preview);
        assert_eq!(cost.xch, 0, "no XCH leg in a pure NFT-for-CAT badge offer");
        assert_eq!(
            cost.cats,
            vec![(cat_asset_id, price)],
            "the wallet must pay exactly the badge price in the requested CAT"
        );

        // Take it: fund the requested CAT from the taker's cat, no fee, testnet sig
        // (the simulator validates against TESTNET11 agg_sig data).
        let funds = TakerFunds {
            xch: vec![],
            cats: vec![(taker_cat, &taker)],
        };
        let bundle = take_offer(&offer_str, funds, &taker, 0, true)?;

        // The assembled bundle must be a single valid transaction on the simulator:
        // the maker's signed NFT spend + the taker's signed CAT payment, atomic.
        assert!(
            bundle.coin_spends.len() >= 2,
            "bundle must include both the maker NFT settlement and the taker CAT spend"
        );
        sim.new_transaction(bundle)?;

        // After the transaction, the badge NFT must be owned by the taker.
        let taker_nfts = sim.hinted_coins(taker.owner_puzzle_hash);
        assert!(
            !taker_nfts.is_empty(),
            "the badge NFT (and CAT change) should land at the taker's address"
        );
        Ok(())
    }

    #[test]
    fn take_offer_rejects_non_offer_string_before_touching_funds() {
        // A non-offer string is rejected up front (no decode, no spend build), so the
        // hub gets an honest "not a Chia offer" rather than a deep failure. Pure unit:
        // no chain, no funds needed.
        let taker = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let funds = TakerFunds {
            xch: vec![],
            cats: vec![],
        };
        let err = take_offer("definitely not an offer", funds, &taker, 0, true).unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("not a Chia offer")),
            "got: {err}"
        );
    }

    #[test]
    fn take_succeeds_when_the_change_address_holds_no_funding_coins() -> anyhow::Result<()> {
        // The change/receive address (index 0) may itself hold no funding coins —
        // the wallet's DIG can live at a later HD index. The take must still work,
        // routing the NFT + change to index 0 using its explicitly-passed key. This
        // guards the regression where the change key was searched only among funds.
        let mut sim = Simulator::new();
        let mut ctx = SpendContext::new();

        let keys = derive_indexed_keys(ABANDON, 0..2)?;
        let change = keys[0].clone(); // primary/receive address — funds NOTHING here
        let payer = keys[1].clone(); // a later address that actually holds the DIG CAT
        let price: u64 = 100_000;

        let (payer_cat, cat_asset_id) = issue_cat_to(
            &mut sim,
            &mut ctx,
            payer.owner_puzzle_hash,
            payer.synthetic_pk,
            &payer.synthetic_sk,
            price,
        )?;
        let offer_str = make_nft_for_cat_offer(&mut sim, &mut ctx, cat_asset_id, price)?;

        let funds = TakerFunds {
            xch: vec![],
            cats: vec![(payer_cat, &payer)],
        };
        // change_keys = index 0 (holds no coins); the CAT is paid from index 1.
        let bundle = take_offer(&offer_str, funds, &change, 0, true)?;
        sim.new_transaction(bundle)?;

        // The NFT routes to the change address even though it funded nothing.
        assert!(
            !sim.hinted_coins(change.owner_puzzle_hash).is_empty(),
            "the badge NFT should land at the change/receive address"
        );
        Ok(())
    }
}
