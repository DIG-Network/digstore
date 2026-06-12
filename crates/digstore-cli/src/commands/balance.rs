//! `digstore balance` — show the wallet's XCH + DIG balance.
//!
//! READ-ONLY: this unlocks the seed only to derive the owner keys, queries the
//! anchor backend for spendable XCH (mojos) and DIG (base units), and prints
//! them. It never builds, signs, or broadcasts a spend.

use crate::context::CliContext;
use crate::error::CliError;
use crate::runtime::block_on;
use digstore_chain::dig::{format_dig, format_xch};

pub fn run(_ctx: &CliContext, ui: &crate::ui::Ui) -> Result<(), CliError> {
    // Unlock the wallet seed (NoSeed → exit 9) to derive the owner keys.
    let (keys, _gcfg) = crate::ops::wallet::unlock_wallet_keys(ui)?;

    // Build the (mock or real) anchor backend; warn loudly if mocked.
    let (anchor, mocked) = crate::ops::anchor_backend::build_anchor();
    crate::ops::anchor_backend::warn_if_mocked(ui, mocked);

    // Query spendable balances. `block_on` → Result<ChainResult<u64>, CliError>;
    // the inner ChainError maps to CliError via `?`.
    let xch = block_on(anchor.balance(&keys))??;
    let dig = block_on(anchor.dig_balance(&keys))??;
    let addr = digstore_chain::keys::owner_address(&keys);

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "address": addr,
            "xch_mojos": xch,
            "dig": format_dig(dig),
            "dig_base_units": dig,
            "mocked": mocked,
        }));
    } else {
        ui.line(format!("address: {addr}"));
        ui.line(format!("XCH: {} ({xch} mojos)", format_xch(xch)));
        ui.line(format!("DIG: {} ({dig} base units)", format_dig(dig)));
    }

    Ok(())
}
