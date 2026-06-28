//! `digstore did create` — create a creator-identity DID (decentralized identifier).
//!
//! Builds the create-DID spend via [`digstore_chain::did::create_simple_did`], signs with the wallet
//! seed, and pushes via coinset. `--dry-run` builds the spend and prints the plan WITHOUT signing or
//! pushing (no spend), so the offline suite can exercise the build path.

use crate::cli::{DidAction, DidArgs, DidCreateArgs};
use crate::error::CliError;
use crate::ops::assets;
use crate::runtime::block_on;
use crate::ui::Ui;

use digstore_chain::did::{create_simple_did, sign_did_spends};

/// One mojo funds the DID singleton launcher; no protocol fee is charged for a DID create (only the
/// implicit network fee, which the caller can leave to the consensus).
const SINGLETON_MOJO: u64 = 1;

pub fn run(ui: &Ui, args: DidArgs) -> Result<(), CliError> {
    match args.action {
        DidAction::Create(a) => create(ui, a),
    }
}

fn create(ui: &Ui, args: DidCreateArgs) -> Result<(), CliError> {
    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);

    // Select a funding coin for the 1-mojo launcher.
    let (keys, funding) = block_on(assets::scan_and_select_funding(
        chain.as_ref(),
        &mnemonic,
        SINGLETON_MOJO,
    ))??;

    let (spends, did) = create_simple_did(&keys, funding).map_err(CliError::from)?;
    let launcher_id = did.info.launcher_id;

    if args.dry_run {
        emit(ui, launcher_id, None, true, mocked);
        return Ok(());
    }

    // Sign with the wallet's synthetic key and push (mainnet agg_sig).
    let sig = sign_did_spends(&spends, std::slice::from_ref(&keys.synthetic_sk), false)
        .map_err(CliError::from)?;
    let bundle = chia_protocol::SpendBundle::new(spends, sig);
    let tx_id = block_on(assets::push_signed(chain.as_ref(), bundle))??;
    emit(ui, launcher_id, Some(tx_id), false, mocked);
    Ok(())
}

fn emit(
    ui: &Ui,
    launcher_id: chia_protocol::Bytes32,
    tx_id: Option<chia_protocol::Bytes32>,
    dry: bool,
    mocked: bool,
) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "did.create",
            "launcher_id": hex::encode(launcher_id),
            "tx_id": tx_id.map(hex::encode),
            "dry_run": dry,
            "mocked": mocked,
        }));
    } else if dry {
        ui.line(format!(
            "would create DID {} (dry-run; nothing spent)",
            hex::encode(launcher_id)
        ));
    } else {
        ui.success(format!("created DID {}", hex::encode(launcher_id)));
        if let Some(t) = tx_id {
            ui.line(format!("tx {}", hex::encode(t)));
        }
    }
}
