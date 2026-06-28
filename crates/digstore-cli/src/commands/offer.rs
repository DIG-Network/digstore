//! `digstore offer make|take|show` — Chia offers (XCH/DIG trades).
//!
//! * `make`  — OFFER and REQUEST fungible assets (XCH/DIG), printing a bech32 `offer1…` string;
//!   built + signed by [`digstore_chain::offer::build_make_offer`].
//! * `take`  — fund + take an `offer1…` string via [`digstore_chain::offer::build_take_offer`]
//!   (scans the wallet for XCH + DIG, builds the combined bundle, pushes it). `--dry-run` decodes +
//!   prices the offer WITHOUT signing/pushing.
//! * `show`  — decode an `offer1…` into a summary (offered/requested/cost/royalties); no spend.
//!
//! Offer legs are written `<amount><asset>` where `asset` is `xch` (mojos) or `dig` (DIG base units),
//! e.g. `1000xch`, `100dig`. DIG amounts are base units (1 DIG = 1000 base units); see `format_dig`.

use crate::cli::{OfferAction, OfferArgs, OfferMakeArgs, OfferShowArgs, OfferTakeArgs};
use crate::error::CliError;
use crate::ops::assets;
use crate::runtime::block_on;
use crate::ui::Ui;

use digstore_chain::cat::dig_cats_for;
use digstore_chain::dig::{format_dig, DIG_ASSET_ID};
use digstore_chain::keys::{derive_indexed_keys, IndexedKeys};
use digstore_chain::offer::{
    build_make_offer, build_take_offer, decode_offer_summary, MakerFunds, OfferAsset,
};
use digstore_chain::wallet::scan_wallet;

pub fn run(ui: &Ui, args: OfferArgs) -> Result<(), CliError> {
    match args.action {
        OfferAction::Make(a) => make(ui, a),
        OfferAction::Take(a) => take(ui, a),
        OfferAction::Show(a) => show(ui, a),
    }
}

/// Parse one `<amount><asset>` leg, e.g. `1000xch` or `100dig`, into an [`OfferAsset`].
fn parse_leg(s: &str) -> Result<OfferAsset, CliError> {
    let s = s.trim().to_ascii_lowercase();
    let (num, asset) = if let Some(rest) = s.strip_suffix("xch") {
        (rest, "xch")
    } else if let Some(rest) = s.strip_suffix("dig") {
        (rest, "dig")
    } else {
        return Err(CliError::InvalidArgument(format!(
            "offer leg `{s}` must end in `xch` or `dig` (e.g. 1000xch, 100dig)"
        )));
    };
    let amount: u64 = num
        .trim()
        .parse()
        .map_err(|_| CliError::InvalidArgument(format!("offer leg `{s}` has no integer amount")))?;
    Ok(match asset {
        "xch" => OfferAsset::Xch(amount),
        _ => OfferAsset::Cat {
            asset_id: DIG_ASSET_ID,
            amount,
        },
    })
}

fn parse_legs(legs: &[String]) -> Result<Vec<OfferAsset>, CliError> {
    legs.iter().map(|l| parse_leg(l)).collect()
}

fn make(ui: &Ui, args: OfferMakeArgs) -> Result<(), CliError> {
    let offered = parse_legs(&args.offer)?;
    let requested = parse_legs(&args.request)?;
    if offered.is_empty() {
        return Err(CliError::InvalidArgument(
            "make-offer needs at least one --offer leg".into(),
        ));
    }
    if requested.is_empty() {
        return Err(CliError::InvalidArgument(
            "make-offer needs at least one --request leg".into(),
        ));
    }

    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);

    // Scan the wallet; collect XCH coins and reconstructed DIG CATs (with lineage proofs).
    let scanned = block_on(scan_wallet(chain.as_ref(), &mnemonic))??;
    let primary = derive_indexed_keys(&mnemonic, 0..1)
        .map_err(CliError::from)?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::Chain("could not derive wallet key".into()))?;

    let mut xch: Vec<(chia_protocol::Coin, &IndexedKeys)> = Vec::new();
    for a in &scanned.addrs {
        for c in &a.xch {
            xch.push((*c, &a.keys));
        }
    }
    let mut cats = Vec::new();
    for a in &scanned.addrs {
        if a.dig.is_empty() {
            continue;
        }
        let reconstructed = block_on(dig_cats_for(chain.as_ref(), a.keys.owner_puzzle_hash))??;
        for cat in reconstructed {
            cats.push((cat, &a.keys));
        }
    }

    let funds = MakerFunds { xch, cats };
    // mainnet signing (for_testnet = false).
    let offer_str = build_make_offer(&primary, funds, &offered, &requested, args.fee, false)
        .map_err(CliError::from)?;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "offer.make",
            "offer": offer_str,
        }));
    } else {
        ui.success("offer created");
        ui.line(offer_str);
    }
    Ok(())
}

fn take(ui: &Ui, args: OfferTakeArgs) -> Result<(), CliError> {
    // Always decode for the cost summary (and to validate the string up front).
    let summary = decode_offer_summary(&args.offer).map_err(CliError::from)?;

    if args.dry_run {
        emit_take(ui, &summary, None, true);
        return Ok(());
    }

    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);
    // build_take_offer scans the wallet, builds + signs the combined bundle (mainnet).
    let taken = block_on(build_take_offer(
        chain.as_ref(),
        &mnemonic,
        &args.offer,
        args.fee,
        false,
    ))??;
    let tx_id = block_on(assets::push_signed(chain.as_ref(), taken.bundle))??;
    emit_take(ui, &summary, Some(tx_id), false);
    Ok(())
}

fn show(ui: &Ui, args: OfferShowArgs) -> Result<(), CliError> {
    let summary = decode_offer_summary(&args.offer).map_err(CliError::from)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "offer.show",
            "offered": legs_json(&summary.offered),
            "requested": legs_json(&summary.requested),
            "cost": cost_json(&summary.arbitrage),
            "royalties": summary
                .royalties
                .iter()
                .map(|(id, bp)| serde_json::json!({"launcher_id": hex::encode(id), "basis_points": bp}))
                .collect::<Vec<_>>(),
        }));
    } else {
        ui.line("OFFERED:");
        for a in &summary.offered {
            ui.line(format!("  {}", fmt_asset(a)));
        }
        ui.line("REQUESTED:");
        for a in &summary.requested {
            ui.line(format!("  {}", fmt_asset(a)));
        }
        ui.line(format!(
            "cost to take: {} XCH-mojos{}",
            summary.arbitrage.xch,
            summary
                .arbitrage
                .cats
                .iter()
                .map(|(_, v)| format!(" + {} DIG", format_dig(*v)))
                .collect::<String>()
        ));
    }
    Ok(())
}

fn emit_take(
    ui: &Ui,
    summary: &digstore_chain::offer::OfferSummary,
    tx_id: Option<chia_protocol::Bytes32>,
    dry: bool,
) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "offer.take",
            "cost": cost_json(&summary.arbitrage),
            "tx_id": tx_id.map(hex::encode),
            "dry_run": dry,
        }));
    } else if dry {
        ui.line(format!(
            "would take this offer for {} XCH-mojos{} (dry-run; nothing spent)",
            summary.arbitrage.xch,
            summary
                .arbitrage
                .cats
                .iter()
                .map(|(_, v)| format!(" + {} DIG", format_dig(*v)))
                .collect::<String>()
        ));
    } else {
        ui.success("offer taken");
        if let Some(t) = tx_id {
            ui.line(format!("tx {}", hex::encode(t)));
        }
    }
}

fn fmt_asset(a: &OfferAsset) -> String {
    match a {
        OfferAsset::Xch(m) => format!("{m} XCH-mojos"),
        OfferAsset::Cat { asset_id, amount } if *asset_id == DIG_ASSET_ID => {
            format!("{} DIG", format_dig(*amount))
        }
        OfferAsset::Cat { asset_id, amount } => {
            format!("{amount} of CAT {}", hex::encode(asset_id))
        }
    }
}

fn legs_json(legs: &[OfferAsset]) -> Vec<serde_json::Value> {
    legs.iter()
        .map(|a| match a {
            OfferAsset::Xch(m) => serde_json::json!({"asset": "xch", "amount": m}),
            OfferAsset::Cat { asset_id, amount } => serde_json::json!({
                "asset": if *asset_id == DIG_ASSET_ID { "dig" } else { "cat" },
                "asset_id": hex::encode(asset_id),
                "amount": amount,
            }),
        })
        .collect()
}

fn cost_json(cost: &digstore_chain::offer::OfferCost) -> serde_json::Value {
    serde_json::json!({
        "xch_mojos": cost.xch,
        "cats": cost.cats.iter().map(|(id, v)| serde_json::json!({"asset_id": hex::encode(id), "amount": v})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_leg_xch_and_dig() {
        assert_eq!(parse_leg("1000xch").unwrap(), OfferAsset::Xch(1000));
        match parse_leg("100dig").unwrap() {
            OfferAsset::Cat { asset_id, amount } => {
                assert_eq!(asset_id, DIG_ASSET_ID);
                assert_eq!(amount, 100);
            }
            _ => panic!("expected DIG cat leg"),
        }
        // case-insensitive
        assert_eq!(parse_leg("5XCH").unwrap(), OfferAsset::Xch(5));
    }

    #[test]
    fn parse_leg_rejects_bad_input() {
        assert!(parse_leg("100usd").is_err());
        assert!(parse_leg("xch").is_err());
        assert!(parse_leg("abcxch").is_err());
    }
}
