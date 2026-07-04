//! `digstore authorize-origin-as-writer <origin>` (#24): discover an origin's (a
//! website/hub, e.g. `hub.dig.net`) DIG pubkey via its well-known endpoint
//! ([`crate::ops::well_known`]) and add it as a CHIP-0035 WRITER delegate on the active
//! store's on-chain singleton. This is the CLI surface over the on-chain
//! writer-delegation primitive `digstore_chain::singleton::{writer_delegated_puzzle,
//! build_update_ownership}` — the same primitive DIGHUb deploy keys and Teams roles are
//! built on (see `deploy-keys.md`), never a hand-rolled puzzle.
//!
//! `--pubkey <hex>` skips well-known discovery for a caller that already has the pubkey —
//! useful before the origin's own well-known endpoint exists (the hub side is a separate,
//! tracked surface: issue #24) or when authorizing a non-web origin's key directly.
//!
//! Delegation is a REPLACE, not an append, at the chain-driver level
//! (`build_update_ownership`'s `new_delegated_puzzles` replaces the whole set), so this
//! command reads the store's CURRENT delegated puzzles first and re-sends every existing
//! delegate plus the new writer — an existing admin/writer/oracle is never dropped.

use chia_protocol::Bytes32;

use digstore_chain::keys::WalletKeys;
use digstore_chain::singleton::{
    build_update_ownership, sync_datastore, writer_delegated_puzzle, DelegatedPuzzle,
};

use crate::cli::AuthorizeOriginArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::anchor_state::AnchorState;
use crate::ops::assets;
use crate::ops::well_known;
use crate::runtime::block_on;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, args: AuthorizeOriginArgs) -> Result<(), CliError> {
    // 1. Resolve the writer pubkey: an explicit `--pubkey` wins over well-known discovery.
    let pubkey_hex = match &args.pubkey {
        Some(hex) => hex.clone(),
        None => block_on(well_known::fetch_origin_pubkey(&args.origin))??,
    };
    let writer_pk = well_known::parse_pubkey_hex(&pubkey_hex)?;

    // 2. Load the active store's on-chain identity (the launcher id == store id).
    let state = AnchorState::load(&ctx.dig_dir)?
        .ok_or_else(|| CliError::NoStore(ctx.dig_dir.display().to_string()))?;
    let launcher_id = assets::parse_launcher_id(&state.store_id)?;

    // 3. Unlock the OWNER wallet + read the store's current on-chain singleton.
    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);

    let store = block_on(sync_datastore(chain.as_ref(), launcher_id))??;
    let owner_ph = store.info.owner_puzzle_hash;

    // 4. Merge the new writer delegate into the EXISTING delegated set (a replace at the
    //    chain-driver level, so every current delegate must be re-sent to survive).
    let writer_dp = writer_delegated_puzzle(writer_pk);
    let (new_delegates, already_authorized) =
        merge_writer_delegate(&store.info.delegated_puzzles, writer_dp);

    if already_authorized {
        emit_result(ui, &args.origin, &pubkey_hex, None, false, true, mocked);
        return Ok(());
    }

    if args.dry_run {
        emit_result(ui, &args.origin, &pubkey_hex, None, true, false, mocked);
        return Ok(());
    }

    // 5. The OWNER key must match the store's current owner puzzle hash — find its HD index
    //    (the store's owner may not be the wallet's first/default address).
    let keys = find_owner_keys(&mnemonic, owner_ph)?;

    // 6. Build + sign + push the ownership/delegation update (byte-mirror of chip35's
    //    `updateStoreOwnership`; owner puzzle hash is UNCHANGED, only the delegated set moves).
    let fee_coins = if args.fee > 0 {
        let (_, coin) = block_on(assets::scan_and_select_funding(
            chain.as_ref(),
            &mnemonic,
            args.fee,
        ))??;
        vec![coin]
    } else {
        Vec::new()
    };
    let build =
        build_update_ownership(&keys, store, owner_ph, new_delegates, &fee_coins, args.fee)?;
    let tx_id = block_on(assets::push_signed(chain.as_ref(), build.bundle))??;

    emit_result(
        ui,
        &args.origin,
        &pubkey_hex,
        Some(tx_id),
        false,
        false,
        mocked,
    );
    Ok(())
}

/// The wallet's owner keys for the HD index whose puzzle hash matches `owner_ph` — the
/// store's owner may live at any address the wallet scanned, not necessarily index 0.
/// Scans the same first-20-index width the asset commands use elsewhere.
fn find_owner_keys(mnemonic: &str, owner_ph: Bytes32) -> Result<WalletKeys, CliError> {
    let indexed = digstore_chain::keys::derive_indexed_keys(mnemonic, 0..20)?;
    indexed
        .into_iter()
        .find(|k| k.owner_puzzle_hash == owner_ph)
        .map(|k| WalletKeys {
            synthetic_sk: k.synthetic_sk,
            synthetic_pk: k.synthetic_pk,
            owner_puzzle_hash: k.owner_puzzle_hash,
        })
        .ok_or_else(|| {
            CliError::Unauthorized(
                "this wallet does not own the store (no HD index within the first 20 matches \
                 the store's on-chain owner key)"
                    .into(),
            )
        })
}

/// Merge `writer_dp` into `existing`, returning the new full delegated-puzzle set (owner +
/// every prior delegate + the new writer) and whether `writer_dp` was ALREADY present
/// (derivation is deterministic — see `writer_delegated_puzzle`'s stability guarantee in
/// `digstore-chain`'s own tests — so equality is a reliable "already authorized" check).
fn merge_writer_delegate(
    existing: &[DelegatedPuzzle],
    writer_dp: DelegatedPuzzle,
) -> (Vec<DelegatedPuzzle>, bool) {
    if existing.contains(&writer_dp) {
        (existing.to_vec(), true)
    } else {
        let mut merged = existing.to_vec();
        merged.push(writer_dp);
        (merged, false)
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_result(
    ui: &Ui,
    origin: &str,
    pubkey_hex: &str,
    tx_id: Option<Bytes32>,
    dry_run: bool,
    already_authorized: bool,
    mocked: bool,
) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "authorize-origin-as-writer",
            "origin": origin,
            "pubkey": pubkey_hex,
            "already_authorized": already_authorized,
            "tx_id": tx_id.map(hex::encode),
            "dry_run": dry_run,
            "mocked": mocked,
        }));
        return;
    }
    if already_authorized {
        ui.line(format!(
            "{origin} (pubkey {pubkey_hex}) is already an authorized writer; nothing to do"
        ));
    } else if dry_run {
        ui.line(format!(
            "would authorize {origin} (pubkey {pubkey_hex}) as a writer (dry-run; nothing spent)"
        ));
    } else {
        ui.success(format!("authorized {origin} as a writer"));
        ui.line(format!("pubkey {pubkey_hex}"));
        if let Some(t) = tx_id {
            ui.line(format!("tx {}", hex::encode(t)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon art";

    #[test]
    fn merge_writer_delegate_appends_when_absent() {
        let keys = digstore_chain::keys::derive_indexed_keys(ABANDON, 0..2).unwrap();
        let dp = writer_delegated_puzzle(keys[0].synthetic_pk);
        let (merged, already) = merge_writer_delegate(&[], dp);
        assert!(!already);
        assert_eq!(merged, vec![dp]);
    }

    #[test]
    fn merge_writer_delegate_is_idempotent() {
        let keys = digstore_chain::keys::derive_indexed_keys(ABANDON, 0..2).unwrap();
        let dp = writer_delegated_puzzle(keys[0].synthetic_pk);
        let (merged, already) = merge_writer_delegate(std::slice::from_ref(&dp), dp);
        assert!(already);
        assert_eq!(merged, vec![dp]);
    }

    #[test]
    fn merge_writer_delegate_preserves_existing_other_delegates() {
        let keys = digstore_chain::keys::derive_indexed_keys(ABANDON, 0..2).unwrap();
        let existing_dp = writer_delegated_puzzle(keys[0].synthetic_pk);
        let new_dp = writer_delegated_puzzle(keys[1].synthetic_pk);
        let (merged, already) = merge_writer_delegate(std::slice::from_ref(&existing_dp), new_dp);
        assert!(!already);
        assert_eq!(merged, vec![existing_dp, new_dp]);
    }

    #[test]
    fn find_owner_keys_finds_the_matching_hd_index() {
        let indexed = digstore_chain::keys::derive_indexed_keys(ABANDON, 0..3).unwrap();
        let target_ph = indexed[2].owner_puzzle_hash;
        let found = find_owner_keys(ABANDON, target_ph).unwrap();
        assert_eq!(found.owner_puzzle_hash, target_ph);
        assert_eq!(
            found.synthetic_pk.to_bytes(),
            indexed[2].synthetic_pk.to_bytes()
        );
    }

    #[test]
    fn find_owner_keys_errors_when_wallet_does_not_own_it() {
        let unrelated_ph = Bytes32::new([0xABu8; 32]);
        let err = find_owner_keys(ABANDON, unrelated_ph).err().unwrap();
        assert!(matches!(err, CliError::Unauthorized(_)));
    }
}
