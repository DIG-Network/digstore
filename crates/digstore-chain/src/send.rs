//! Native standard-XCH send: build + sign a Chia spend bundle that pays `amount`
//! mojos to a recipient address with `fee` mojos, drawing XCH coins from across the
//! HD wallet. This module is **pure build + sign** — it NEVER broadcasts. Pushing
//! the returned bundle to mainnet is the caller's explicit decision (the dig-wallet
//! sidecar gates it behind a confirmation + env flag), because a push spends real
//! funds.
//!
//! Coin model (mirrors the multi-address fee pattern in `singleton.rs`): one
//! selected coin is the LEAD — it carries every output (the payment, and the change
//! back to the wallet) and reserves the fee. Every other selected coin is spent
//! under its own synthetic key with a single `ASSERT_CONCURRENT_SPEND(lead)`, so the
//! whole set is atomic: the lead alone is invalid (its outputs exceed its own
//! value), and each extra coin requires the lead in the same block. The bundle
//! balances exactly: `sum(inputs) == amount + change + fee`.

use std::collections::HashSet;

use crate::error::{ChainError, Result};
use crate::wallet::ScannedWallet;
use chia::puzzles::Memos;
use chia_wallet_sdk::driver::{SpendContext, StandardLayer};
use chia_wallet_sdk::types::Conditions;
use datalayer_driver::{
    address_to_puzzle_hash, select_coins, sign_coin_spends, Bytes32, Coin, CoinSpend, PublicKey,
    SecretKey, SpendBundle,
};

/// An XCH coin tagged with the synthetic keypair (and owner puzzle hash) of the
/// wallet address that holds it, so it can be spent under the correct key.
struct KeyedCoin {
    coin: Coin,
    sk: SecretKey,
    pk: PublicKey,
    owner_ph: Bytes32,
}

/// Decode a bech32m mainnet `xch1…` address to its 32-byte puzzle hash.
pub fn decode_xch_address(address: &str) -> Result<Bytes32> {
    address_to_puzzle_hash(address)
        .map_err(|e| ChainError::Chain(format!("invalid recipient address: {e}")))
}

/// The unsigned plan for an XCH send: the coin spends plus the value breakdown.
/// `inputs == amount + change + fee` always holds.
pub struct SendPlan {
    pub coin_spends: Vec<CoinSpend>,
    /// Total mojos of the selected input coins.
    pub inputs: u64,
    /// Mojos paid to the recipient.
    pub amount: u64,
    /// Mojos reserved as the network fee.
    pub fee: u64,
    /// Mojos returned to the wallet (to the lead coin's own address).
    pub change: u64,
    /// Puzzle hash the change is returned to (the lead coin's owner address).
    pub change_ph: Bytes32,
    /// Synthetic secret keys of exactly the chosen input coins (the signing set).
    signing_keys: Vec<SecretKey>,
}

/// Build the UNSIGNED coin spends paying `amount` mojos to `recipient_ph` with
/// `fee` mojos, selecting XCH across the scanned wallet. Errors on a zero amount,
/// arithmetic overflow, or insufficient funds.
pub fn build_xch_send_unsigned(
    wallet: &ScannedWallet,
    recipient_ph: Bytes32,
    amount: u64,
    fee: u64,
) -> Result<SendPlan> {
    if amount == 0 {
        return Err(ChainError::Chain(
            "send amount must be greater than zero".into(),
        ));
    }
    let total_needed = amount
        .checked_add(fee)
        .ok_or_else(|| ChainError::Chain("amount + fee overflows u64".into()))?;

    // Flatten every spendable XCH coin, each tagged with its address's keys.
    let keyed: Vec<KeyedCoin> = wallet
        .addrs
        .iter()
        .flat_map(|a| {
            a.xch.iter().map(move |c| KeyedCoin {
                coin: *c,
                sk: a.keys.synthetic_sk.clone(),
                pk: a.keys.synthetic_pk,
                owner_ph: a.keys.owner_puzzle_hash,
            })
        })
        .collect();

    let all_coins: Vec<Coin> = keyed.iter().map(|k| k.coin).collect();
    let selected = select_coins(&all_coins, total_needed)
        .map_err(|e| ChainError::Chain(format!("insufficient XCH for amount + fee: {e}")))?;
    let selected_ids: HashSet<Bytes32> = selected.iter().map(|c| c.coin_id()).collect();
    let chosen: Vec<&KeyedCoin> = keyed
        .iter()
        .filter(|k| selected_ids.contains(&k.coin.coin_id()))
        .collect();
    // `select_coins` guarantees a non-empty selection covering `total_needed`.
    let lead = *chosen
        .first()
        .ok_or_else(|| ChainError::Chain("coin selection returned no coins".into()))?;

    let inputs: u64 = chosen.iter().map(|k| k.coin.amount).sum();
    let change = inputs - amount - fee; // inputs >= total_needed (select_coins invariant)
    let change_ph = lead.owner_ph;

    let mut ctx = SpendContext::new();

    // Lead coin: pay the recipient, return change, reserve the fee.
    let mut lead_conditions = Conditions::new().create_coin(recipient_ph, amount, Memos::None);
    if change > 0 {
        lead_conditions = lead_conditions.create_coin(change_ph, change, Memos::None);
    }
    if fee > 0 {
        lead_conditions = lead_conditions.reserve_fee(fee);
    }
    StandardLayer::new(lead.pk)
        .spend(&mut ctx, lead.coin, lead_conditions)
        .map_err(|e| ChainError::Chain(format!("lead coin spend: {e}")))?;

    // Every other selected coin: spent under its own key, bound to the lead.
    let lead_id = lead.coin.coin_id();
    for k in chosen.iter().skip(1) {
        StandardLayer::new(k.pk)
            .spend(
                &mut ctx,
                k.coin,
                Conditions::new().assert_concurrent_spend(lead_id),
            )
            .map_err(|e| ChainError::Chain(format!("extra coin spend: {e}")))?;
    }

    Ok(SendPlan {
        coin_spends: ctx.take(),
        inputs,
        amount,
        fee,
        change,
        change_ph,
        signing_keys: chosen.iter().map(|k| k.sk.clone()).collect(),
    })
}

/// Build AND sign an XCH send into a ready-to-broadcast [`SpendBundle`]. Signs the
/// spends with the synthetic secret keys of exactly the chosen coins (AugScheme,
/// the aggregated §11.3 signature). **Pure: does NOT push** — broadcasting is the
/// caller's decision. Returns the bundle and the value plan.
pub fn build_xch_send(
    wallet: &ScannedWallet,
    recipient_ph: Bytes32,
    amount: u64,
    fee: u64,
) -> Result<(SpendBundle, SendPlan)> {
    let plan = build_xch_send_unsigned(wallet, recipient_ph, amount, fee)?;
    let signature = sign_coin_spends(&plan.coin_spends, &plan.signing_keys, false)
        .map_err(|e| ChainError::Chain(format!("sign coin spends: {e}")))?;
    let bundle = SpendBundle::new(plan.coin_spends.clone(), signature);
    Ok((bundle, plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::derive_indexed_keys;
    use crate::wallet::{AddressCoins, ScannedWallet};
    use chia_protocol::Coin;

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    /// A `Bytes32` filled with `b` — a stand-in parent coin id for fabricated coins.
    fn b32(b: u8) -> Bytes32 {
        Bytes32::new([b; 32])
    }

    /// Build a `ScannedWallet` from the ABANDON vector: address index `i` holds the
    /// XCH coins given by `amounts[i]` (in mojos), at that index's owner puzzle hash.
    fn wallet_with(amounts: &[&[u64]]) -> ScannedWallet {
        let keys = derive_indexed_keys(ABANDON, 0..amounts.len() as u32).unwrap();
        let mut addrs = Vec::new();
        for (i, k) in keys.into_iter().enumerate() {
            let ph = k.owner_puzzle_hash;
            let xch = amounts[i]
                .iter()
                .enumerate()
                .map(|(j, amt)| Coin::new(b32(0x10 + (i * 8 + j) as u8), ph, *amt))
                .collect();
            addrs.push(AddressCoins {
                keys: k,
                xch,
                dig: vec![],
            });
        }
        ScannedWallet { addrs }
    }

    /// The recipient puzzle hash used by the payment tests.
    fn recipient() -> Bytes32 {
        b32(0xAA)
    }

    #[test]
    fn multi_coin_send_balances_across_addresses() {
        // Two 3000-mojo coins at different addresses; neither alone covers 4500, so
        // BOTH are selected and the spend draws across addresses (each under its own
        // key). The whole bundle must balance exactly: inputs == amount + change + fee.
        let wallet = wallet_with(&[&[3_000], &[3_000]]);
        let plan = build_xch_send_unsigned(&wallet, recipient(), 4_000, 500).unwrap();
        assert_eq!(plan.inputs, plan.amount + plan.change + plan.fee);
        assert_eq!(plan.amount, 4_000);
        assert_eq!(plan.fee, 500);
        assert_eq!(plan.inputs, 6_000); // both coins
        assert_eq!(plan.change, 6_000 - 4_000 - 500);
        assert_eq!(plan.coin_spends.len(), 2);
        // Change returns to a wallet-owned address.
        let owned: Vec<Bytes32> = wallet
            .addrs
            .iter()
            .map(|a| a.keys.owner_puzzle_hash)
            .collect();
        assert!(owned.contains(&plan.change_ph));
    }

    #[test]
    fn single_coin_no_change_when_exact() {
        // One coin exactly covers amount + fee → no change output, single spend.
        let wallet = wallet_with(&[&[4_500]]);
        let plan = build_xch_send_unsigned(&wallet, recipient(), 4_000, 500).unwrap();
        assert_eq!(plan.change, 0);
        assert_eq!(plan.coin_spends.len(), 1);
    }

    #[test]
    fn zero_amount_is_rejected() {
        let wallet = wallet_with(&[&[1_000]]);
        assert!(build_xch_send_unsigned(&wallet, recipient(), 0, 0).is_err());
    }

    #[test]
    fn insufficient_funds_is_rejected() {
        let wallet = wallet_with(&[&[1_000]]);
        let r = build_xch_send_unsigned(&wallet, recipient(), 5_000, 0);
        assert!(r.is_err(), "5000 mojos requested from a 1000-mojo wallet");
    }

    #[test]
    fn build_signs_deterministically_and_nonempty() {
        // The signing path is real: it produces a canonical 96-byte AugScheme
        // aggregate, and signing the SAME spends with the SAME keys is deterministic.
        let wallet = wallet_with(&[&[10_000]]);
        let (bundle_a, _plan) = build_xch_send(&wallet, recipient(), 4_000, 500).unwrap();
        let (bundle_b, _plan) = build_xch_send(&wallet, recipient(), 4_000, 500).unwrap();
        let sig_a = bundle_a.aggregated_signature.to_bytes();
        let sig_b = bundle_b.aggregated_signature.to_bytes();
        assert_eq!(sig_a.len(), 96, "G2 aggregate signature is 96 bytes");
        assert_eq!(
            sig_a, sig_b,
            "AugScheme signing over the spends is deterministic"
        );
        // A real signature was produced (not the identity/empty signature).
        assert_ne!(
            sig_a,
            datalayer_driver::Signature::default().to_bytes(),
            "a real signature must be produced over the spends"
        );
        assert_eq!(bundle_a.coin_spends.len(), 1);
    }

    #[test]
    fn lead_spend_pays_recipient_the_exact_amount() {
        // Decode the lead coin's conditions and confirm a CREATE_COIN pays the
        // recipient exactly `amount` — proving the payment output is correct (not
        // just that the value math balances).
        use chia_wallet_sdk::types::{run_puzzle, Condition};
        use clvm_traits::{FromClvm, ToClvm};
        use clvmr::Allocator;

        let wallet = wallet_with(&[&[10_000]]);
        let plan = build_xch_send_unsigned(&wallet, recipient(), 4_000, 500).unwrap();
        let cs = &plan.coin_spends[0];

        let mut a = Allocator::new();
        let puzzle = cs.puzzle_reveal.to_clvm(&mut a).unwrap();
        let solution = cs.solution.to_clvm(&mut a).unwrap();
        let output = run_puzzle(&mut a, puzzle, solution).unwrap();
        let conditions = Vec::<Condition>::from_clvm(&a, output).unwrap();

        let pays_recipient = conditions.iter().any(|c| {
            matches!(c, Condition::CreateCoin(cc)
                if cc.puzzle_hash == recipient() && cc.amount == 4_000)
        });
        assert!(
            pays_recipient,
            "lead spend must CREATE_COIN(recipient, amount)"
        );
    }
}
