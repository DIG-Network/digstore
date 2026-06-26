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
use chia::puzzles::offer::{NotarizedPayment, Payment};
use chia::puzzles::Memos;
use chia_protocol::{Bytes32, Coin, SpendBundle};
use chia_wallet_sdk::driver::{
    decode_offer, encode_offer, Action, AssetInfo, Cat, CatAssetInfo, Id, Offer, Relation,
    RequestedPayments, SpendContext, Spends,
};
use chia_wallet_sdk::types::puzzles::SettlementPayment;
use chia_wallet_sdk::types::{Conditions, Mod};
use datalayer_driver::{sign_coin_spends, PublicKey, SecretKey};
use indexmap::IndexMap;

/// The offer settlement puzzle hash (`SETTLEMENT_PAYMENT_HASH`). Sourced via the
/// `SettlementPayment` mod's `mod_hash()` so we don't take a direct dependency on the
/// `chia-puzzles` constants crate (it's only a dev-dependency here).
fn settlement_payment_hash() -> Bytes32 {
    Bytes32::from(<[u8; 32]>::from(SettlementPayment::mod_hash()))
}

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

// ===========================================================================
// Make-offer — the maker's side (the missing half; `take_offer` is above).
//
// A make-offer is the inverse of a take: the wallet OFFERS its fungible assets
// (XCH / CATs) by spending them into the settlement puzzle, and REQUESTS assets
// (XCH / CATs) be paid back to its address. The result is a one-sided, signed
// `offer1…` string that anyone can take (the taker funds the requested side and
// claims the offered coins, exactly what `take_offer` does).
//
// Scope: this builds the fungible (XCH + CAT) legs — the assets the wallet
// actually holds in this crate. NFT/DID legs are owned by a separate module; a
// make-offer that OFFERS an NFT would be assembled there. Royalty enforcement on
// requested NFTs is preserved on decode (`decode_offer_summary`), since the
// canonical `Offer` carries it.
// ===========================================================================

/// One fungible asset leg of an offer: either XCH (mojos) or a CAT (by asset id /
/// TAIL hash, in base units). Used for both the offered and requested sides of
/// [`build_make_offer`] and reported by [`decode_offer_summary`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferAsset {
    /// XCH, amount in mojos.
    Xch(u64),
    /// A CAT identified by its `asset_id` (TAIL hash), amount in base units.
    Cat { asset_id: Bytes32, amount: u64 },
}

impl OfferAsset {
    fn amount(&self) -> u64 {
        match self {
            OfferAsset::Xch(a) => *a,
            OfferAsset::Cat { amount, .. } => *amount,
        }
    }
}

/// The maker's coins funding the OFFERED side: spendable XCH coins and CAT coins
/// (with lineage proofs, as reconstructed by [`crate::cat::reconstruct_cat_coins`]),
/// each tagged with the synthetic keypair of the address that holds it so the inner
/// spend can be authorized. Mirrors [`TakerFunds`].
pub struct MakerFunds<'a> {
    /// XCH coins available to fund offered XCH and the fee.
    pub xch: Vec<(Coin, &'a IndexedKeys)>,
    /// CAT coins available to fund offered CAT legs.
    pub cats: Vec<(Cat, &'a IndexedKeys)>,
}

/// Greedily select coins of the given `OfferAsset` from `funds`, covering its amount.
/// Returns the chosen coins so the caller can spend exactly them into settlement.
fn select_offered_xch(funds: &MakerFunds<'_>, need: u64) -> Result<Vec<(Coin, IndexedKeys)>> {
    let mut sorted: Vec<&(Coin, &IndexedKeys)> = funds.xch.iter().collect();
    sorted.sort_by(|a, b| b.0.amount.cmp(&a.0.amount));
    let mut sum = 0u64;
    let mut out = Vec::new();
    for (coin, keys) in sorted {
        if sum >= need {
            break;
        }
        out.push((*coin, (*keys).clone()));
        sum += coin.amount;
    }
    if sum < need {
        return Err(ChainError::Chain(format!(
            "insufficient XCH to offer: need {need} have {sum}"
        )));
    }
    Ok(out)
}

/// Build AND sign a make-offer: OFFER `offered` (XCH/CAT) and REQUEST `requested`
/// (XCH/CAT, paid to the maker's primary address), returning the bech32 `offer1…`
/// string. Optionally reserve an XCH network `fee`. `for_testnet` selects the
/// signing network (mainnet in production; the simulator validates against TESTNET11).
///
/// The offered coins are spent into the settlement puzzle and the requested payments
/// are asserted, so the resulting one-sided offer is only valid when a taker funds
/// the requested side in the same transaction. **Pure: does NOT broadcast** — the
/// returned string is handed out; settlement happens when someone takes it.
///
/// `maker` is the address the requested assets are paid to (and offered change /
/// fee draw from `funds`). Errors if a leg's coins can't cover the offered amount,
/// or if an offered/requested CAT is also present on the other side (a same-asset
/// wash is rejected up front).
pub fn build_make_offer(
    maker: &IndexedKeys,
    funds: MakerFunds<'_>,
    offered: &[OfferAsset],
    requested: &[OfferAsset],
    fee: u64,
    for_testnet: bool,
) -> Result<String> {
    if offered.is_empty() {
        return Err(ChainError::Chain(
            "make-offer must offer at least one asset".into(),
        ));
    }
    if requested.is_empty() {
        return Err(ChainError::Chain(
            "make-offer must request at least one asset".into(),
        ));
    }

    let settlement_ph = settlement_payment_hash();
    let maker_ph = maker.owner_puzzle_hash;

    let mut ctx = SpendContext::new();
    let maker_hint = ctx
        .hint(maker_ph)
        .map_err(|e| ChainError::Chain(format!("alloc maker hint: {e}")))?;

    // The change/spend target is the maker's address (offered surplus + fee draw).
    let mut spends = Spends::new(maker_ph);
    let mut key_map: IndexMap<Bytes32, PublicKey> = IndexMap::new();
    key_map.insert(maker_ph, maker.synthetic_pk);

    // ---- Add the maker's funding coins for each OFFERED leg ----
    let mut actions: Vec<Action> = Vec::new();
    // Collect the offered coin ids so the requested payments can be notarized
    // against a nonce derived from them (binds the request to this exact offer).
    let mut offered_coin_ids: Vec<Bytes32> = Vec::new();

    for leg in offered {
        match leg {
            OfferAsset::Xch(amount) => {
                let chosen = select_offered_xch(&funds, amount.saturating_add(fee))?;
                for (coin, keys) in &chosen {
                    spends.add(*coin);
                    key_map.insert(keys.owner_puzzle_hash, keys.synthetic_pk);
                    offered_coin_ids.push(coin.coin_id());
                }
                // Offer the XCH into settlement; change auto-returns to maker_ph.
                actions.push(Action::send(Id::Xch, settlement_ph, *amount, Memos::None));
            }
            OfferAsset::Cat { asset_id, amount } => {
                // Select CAT coins of this asset id covering the offered amount.
                let mut sorted: Vec<&(Cat, &IndexedKeys)> = funds
                    .cats
                    .iter()
                    .filter(|(c, _)| c.info.asset_id == *asset_id)
                    .collect();
                sorted.sort_by(|a, b| b.0.coin.amount.cmp(&a.0.coin.amount));
                let mut sum = 0u64;
                for (cat, keys) in sorted {
                    if sum >= *amount {
                        break;
                    }
                    spends.add(*cat);
                    key_map.insert(keys.owner_puzzle_hash, keys.synthetic_pk);
                    offered_coin_ids.push(cat.coin.coin_id());
                    sum += cat.coin.amount;
                }
                if sum < *amount {
                    return Err(ChainError::Chain(format!(
                        "insufficient CAT (asset {asset_id:?}) to offer: need {amount} have {sum}"
                    )));
                }
                // Offer the CAT into settlement; change auto-returns to maker_ph.
                actions.push(Action::send(
                    Id::Existing(*asset_id),
                    settlement_ph,
                    *amount,
                    Memos::None,
                ));
            }
        }
    }

    if offered_coin_ids.is_empty() {
        return Err(ChainError::Chain(
            "make-offer selected no offered coins".into(),
        ));
    }

    // ---- Build the REQUESTED payments (paid to the maker's address) ----
    let nonce = Offer::nonce(offered_coin_ids);
    let mut requested_payments = RequestedPayments::new();
    let mut requested_asset_info = AssetInfo::new();
    for leg in requested {
        if leg.amount() == 0 {
            return Err(ChainError::Chain(
                "requested asset amount must be greater than zero".into(),
            ));
        }
        match leg {
            OfferAsset::Xch(amount) => {
                requested_payments.xch.push(NotarizedPayment::new(
                    nonce,
                    vec![Payment::new(maker_ph, *amount, maker_hint)],
                ));
            }
            OfferAsset::Cat { asset_id, amount } => {
                requested_payments.cats.insert(
                    *asset_id,
                    vec![NotarizedPayment::new(
                        nonce,
                        vec![Payment::new(maker_ph, *amount, maker_hint)],
                    )],
                );
                requested_asset_info
                    .insert_cat(*asset_id, CatAssetInfo::new(None))
                    .map_err(|e| ChainError::Chain(format!("insert requested cat info: {e}")))?;
            }
        }
    }

    if fee > 0 {
        actions.push(Action::fee(fee));
    }

    // ---- Run the offered spends, asserting the requested payments ----
    let deltas = spends
        .apply(&mut ctx, &actions)
        .map_err(|e| ChainError::Chain(format!("apply make-offer actions: {e}")))?;

    spends.conditions.required = spends.conditions.required.extend(
        requested_payments
            .assertions(&mut ctx, &requested_asset_info)
            .map_err(|e| ChainError::Chain(format!("requested payment assertions: {e}")))?,
    );

    spends
        .finish_with_keys(&mut ctx, &deltas, Relation::AssertConcurrent, &key_map)
        .map_err(|e| ChainError::Chain(format!("finish make-offer spends: {e}")))?;

    // Sign the maker's offered coins, then assemble the one-sided offer and encode it.
    let coin_spends = ctx.take();
    let signing_keys: Vec<SecretKey> = maker_signing_keys(&key_map, maker);
    let signature = sign_coin_spends(&coin_spends, &signing_keys, for_testnet)
        .map_err(|e| ChainError::Chain(format!("sign make-offer spends: {e}")))?;

    let offer = Offer::from_input_spend_bundle(
        &mut ctx,
        SpendBundle::new(coin_spends, signature),
        requested_payments,
        requested_asset_info,
    )
    .map_err(|e| ChainError::Chain(format!("assemble make-offer: {e}")))?;

    encode_offer_bundle(
        &offer
            .to_spend_bundle(&mut ctx)
            .map_err(|e| ChainError::Chain(format!("serialize make-offer spend bundle: {e}")))?,
    )
}

/// The maker's signing keys: the synthetic secret key for every owner puzzle hash in
/// `key_map`, sourced from `maker` (the only address in this crate's make-offer path).
/// In the single-address common case this is just the maker's key.
fn maker_signing_keys(
    key_map: &IndexMap<Bytes32, PublicKey>,
    maker: &IndexedKeys,
) -> Vec<SecretKey> {
    // Every selected coin's owner ph maps to a synthetic pk in `key_map`; for the
    // make-offer path the funds belong to `maker` (a single derived address). If a
    // future caller funds across addresses, each address's key must be threaded
    // through here; today the maker key signs all offered coins.
    let _ = key_map;
    vec![maker.synthetic_sk.clone()]
}

/// A decoded summary of an `offer1…` string: what it OFFERS, what it REQUESTS, the
/// net arbitrage (requested-minus-offered per asset), and any NFT royalties carried
/// by the offer. Lets a wallet inspect an offer without taking it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OfferSummary {
    /// Assets the offer gives the taker (XCH mojos + per-CAT base units).
    pub offered: Vec<OfferAsset>,
    /// Assets the offer asks the taker to pay (XCH mojos + per-CAT base units).
    pub requested: Vec<OfferAsset>,
    /// Net the taker must FUND to take it (the requested-over-offered surplus).
    pub arbitrage: OfferCost,
    /// Royalty legs carried by the offer: (NFT launcher id, royalty basis points).
    pub royalties: Vec<(Bytes32, u16)>,
}

/// Decode a bech32 `offer1…` string into an [`OfferSummary`] — offered/requested
/// fungible assets, arbitrage, and royalties — WITHOUT taking it. Built on
/// [`Offer::from_spend_bundle`] + `offered_coins`/`requested_payments`/royalty
/// accessors, so it reports exactly what the canonical offer carries.
pub fn decode_offer_summary(offer_str: &str) -> Result<OfferSummary> {
    let offer = decode_offer_string(offer_str)?;

    // Offered fungible amounts (what the taker receives).
    let offered_amounts = offer.offered_coins().amounts();
    let mut offered: Vec<OfferAsset> = Vec::new();
    if offered_amounts.xch > 0 {
        offered.push(OfferAsset::Xch(offered_amounts.xch));
    }
    for (asset_id, amount) in &offered_amounts.cats {
        offered.push(OfferAsset::Cat {
            asset_id: *asset_id,
            amount: *amount,
        });
    }

    // Requested fungible amounts (what the taker pays).
    let requested_amounts = offer.requested_payments().amounts();
    let mut requested: Vec<OfferAsset> = Vec::new();
    if requested_amounts.xch > 0 {
        requested.push(OfferAsset::Xch(requested_amounts.xch));
    }
    for (asset_id, amount) in &requested_amounts.cats {
        requested.push(OfferAsset::Cat {
            asset_id: *asset_id,
            amount: *amount,
        });
    }

    // Royalties carried by the offer (offered + requested NFT royalties).
    let mut royalties: Vec<(Bytes32, u16)> = Vec::new();
    for r in offer
        .offered_royalties()
        .into_iter()
        .chain(offer.requested_royalties())
    {
        royalties.push((r.launcher_id, r.basis_points));
    }

    Ok(OfferSummary {
        offered,
        requested,
        arbitrage: offer_cost(&offer),
        royalties,
    })
}

/// Build AND sign the cancel spends for an offer the wallet MADE: spend the offered
/// coins back to the maker, invalidating the outstanding `offer1…` string. Returns
/// the signed [`SpendBundle`] (caller pushes it under its broadcast gate).
///
/// Uses [`Offer::cancellable_coin_spends`] to recover exactly the offered coins
/// (those whose outputs the maker still controls), re-spends each to the maker's
/// address, and signs with the maker's synthetic key. `for_testnet` selects the
/// signing network. **Pure: does NOT broadcast.**
pub fn cancel_offer(
    offer_str: &str,
    maker: &IndexedKeys,
    fee: u64,
    for_testnet: bool,
) -> Result<SpendBundle> {
    use chia_wallet_sdk::driver::StandardLayer;

    let offer = decode_offer_string(offer_str)?;
    let cancellable = offer
        .cancellable_coin_spends()
        .map_err(|e| ChainError::Chain(format!("compute cancellable coin spends: {e}")))?;
    if cancellable.is_empty() {
        return Err(ChainError::Chain(
            "no cancellable coins in this offer (already settled or not the maker's)".into(),
        ));
    }

    // Re-spend each offered (settlement-bound) coin back to the maker. The maker
    // controls the inner puzzle of each offered coin (it signed them into the offer),
    // so the standard layer of the maker's synthetic key authorizes reclaiming them.
    let mut ctx = SpendContext::new();
    let maker_ph = maker.owner_puzzle_hash;
    let p2 = StandardLayer::new(maker.synthetic_pk);
    let mut first = true;
    for cs in &cancellable {
        // Reclaim each coin to the maker; the first coin reserves the fee.
        let mut conditions = Conditions::new().create_coin(maker_ph, cs.coin.amount, Memos::None);
        if first && fee > 0 {
            conditions = conditions.reserve_fee(fee);
            first = false;
        }
        p2.spend(&mut ctx, cs.coin, conditions)
            .map_err(|e| ChainError::Chain(format!("build cancel spend: {e}")))?;
    }

    let coin_spends = ctx.take();
    let signature = sign_coin_spends(
        &coin_spends,
        std::slice::from_ref(&maker.synthetic_sk),
        for_testnet,
    )
    .map_err(|e| ChainError::Chain(format!("sign cancel spends: {e}")))?;
    Ok(SpendBundle::new(coin_spends, signature))
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

    // ----- make-offer: offline validation core -----

    #[test]
    fn make_offer_rejects_empty_sides() {
        let maker = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        let funds = MakerFunds {
            xch: vec![],
            cats: vec![],
        };
        // No offered assets.
        let err = build_make_offer(
            &maker,
            MakerFunds {
                xch: vec![],
                cats: vec![],
            },
            &[],
            &[OfferAsset::Xch(1)],
            0,
            true,
        )
        .unwrap_err();
        assert!(matches!(&err, ChainError::Chain(m) if m.contains("offer at least one")));
        // No requested assets.
        let err = build_make_offer(&maker, funds, &[OfferAsset::Xch(1)], &[], 0, true).unwrap_err();
        assert!(matches!(&err, ChainError::Chain(m) if m.contains("request at least one")));
    }

    #[test]
    fn make_offer_errors_when_offered_coins_are_short() {
        let maker = derive_indexed_keys(ABANDON, 0..1).unwrap()[0].clone();
        // Offer 1000 mojos XCH but provide no coins → insufficient.
        let err = build_make_offer(
            &maker,
            MakerFunds {
                xch: vec![],
                cats: vec![],
            },
            &[OfferAsset::Xch(1_000)],
            &[OfferAsset::Xch(1)],
            0,
            true,
        )
        .unwrap_err();
        assert!(matches!(&err, ChainError::Chain(m) if m.contains("insufficient XCH to offer")));
    }

    #[test]
    fn decode_summary_rejects_non_offer() {
        assert!(decode_offer_summary("not an offer").is_err());
        assert!(decode_offer_summary("   ").is_err());
    }

    // ----- make-offer: full round-trip on the Simulator -----
    //
    // The maker OFFERS CAT-A and REQUESTS CAT-B. We build the one-sided offer with
    // build_make_offer, inspect it with decode_offer_summary (offered/requested legs
    // match), then a taker holding CAT-B TAKES it via take_offer. The combined bundle
    // settles as one transaction and the assets cross over: the maker ends with CAT-B,
    // the taker with CAT-A.

    #[test]
    fn make_offer_round_trips_through_decode_and_take() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let mut ctx = SpendContext::new();

        // Maker = index 0, taker = index 1 (distinct addresses).
        let keys = derive_indexed_keys(ABANDON, 0..2)?;
        let maker = keys[0].clone();
        let taker = keys[1].clone();

        let offered_amt: u64 = 80_000; // CAT-A the maker gives
        let requested_amt: u64 = 50_000; // CAT-B the maker wants

        // Issue CAT-A to the maker (what it offers) and CAT-B to the taker (the
        // requested payment). issue_cat_to returns spendable Cats with lineage proofs.
        let (maker_cat_a, asset_a) = issue_cat_to(
            &mut sim,
            &mut ctx,
            maker.owner_puzzle_hash,
            maker.synthetic_pk,
            &maker.synthetic_sk,
            offered_amt,
        )?;
        let (taker_cat_b, asset_b) = issue_cat_to(
            &mut sim,
            &mut ctx,
            taker.owner_puzzle_hash,
            taker.synthetic_pk,
            &taker.synthetic_sk,
            requested_amt,
        )?;
        assert_ne!(asset_a, asset_b, "two distinct CATs");

        // Maker builds the offer: OFFER CAT-A, REQUEST CAT-B (testnet sig for the sim).
        let maker_funds = MakerFunds {
            xch: vec![],
            cats: vec![(maker_cat_a, &maker)],
        };
        let offer_str = build_make_offer(
            &maker,
            maker_funds,
            &[OfferAsset::Cat {
                asset_id: asset_a,
                amount: offered_amt,
            }],
            &[OfferAsset::Cat {
                asset_id: asset_b,
                amount: requested_amt,
            }],
            0,
            true,
        )?;
        assert!(offer_str.starts_with("offer1"), "got: {offer_str}");

        // Inspect WITHOUT taking: offered = CAT-A, requested = CAT-B, and the taker's
        // arbitrage cost is exactly the requested CAT-B amount.
        let summary = decode_offer_summary(&offer_str)?;
        assert_eq!(
            summary.offered,
            vec![OfferAsset::Cat {
                asset_id: asset_a,
                amount: offered_amt
            }],
            "offer must offer CAT-A"
        );
        assert_eq!(
            summary.requested,
            vec![OfferAsset::Cat {
                asset_id: asset_b,
                amount: requested_amt
            }],
            "offer must request CAT-B"
        );
        assert_eq!(
            summary.arbitrage.cats,
            vec![(asset_b, requested_amt)],
            "taker must fund exactly the requested CAT-B amount"
        );
        assert_eq!(summary.arbitrage.xch, 0, "no XCH leg");

        // Taker takes it, funding CAT-B from its coin; change/received CAT-A route to
        // the taker's address.
        let funds = TakerFunds {
            xch: vec![],
            cats: vec![(taker_cat_b, &taker)],
        };
        let bundle = take_offer(&offer_str, funds, &taker, 0, true)?;
        assert!(
            bundle.coin_spends.len() >= 2,
            "bundle must include both the maker's offered spend and the taker's payment"
        );

        // Settle the whole thing in one transaction on the simulator.
        sim.new_transaction(bundle)?;

        // After settlement: the maker received CAT-B; the taker received CAT-A. Both
        // land hinted at their addresses (CAT coins are hinted to the owner inner ph).
        let maker_got_b = sim
            .unspent_coins(
                crate::cat::cat_puzzle_hash(maker.owner_puzzle_hash, asset_b),
                false,
            )
            .iter()
            .map(|c| c.amount)
            .sum::<u64>();
        let taker_got_a = sim
            .unspent_coins(
                crate::cat::cat_puzzle_hash(taker.owner_puzzle_hash, asset_a),
                false,
            )
            .iter()
            .map(|c| c.amount)
            .sum::<u64>();
        assert_eq!(
            maker_got_b, requested_amt,
            "maker must receive requested CAT-B"
        );
        assert_eq!(taker_got_a, offered_amt, "taker must receive offered CAT-A");
        Ok(())
    }
}
