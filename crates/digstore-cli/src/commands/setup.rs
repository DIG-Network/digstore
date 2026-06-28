//! `digstore setup` (alias `auth`) — one guided first-run (roadmap #21).
//!
//! The CLI has historically had TWO identities with no unified onboarding:
//!   - the **seed** — a BIP-39 wallet that SIGNS every on-chain action and pays
//!     the 100 DIG + XCH per publish; and
//!   - the **dighub login** — an account that GATES the push to the public hub and
//!     surfaces your projects in the dashboard, with NO on-chain authority.
//!
//! `setup` collapses both into one flow and explains the difference in one place:
//!   1. Seed — import an existing mnemonic, or generate a new one.
//!   2. Funds — does the wallet hold enough DIG + XCH for a publish? (Pointer to
//!      where to get more if not.)
//!   3. Login (optional) — pair a dighub account.
//!
//! Everything except generating a brand-new seed is safe to re-run. The seed +
//! login steps REUSE the exact `seed`/`login` command paths (no duplicate logic),
//! so behaviour stays identical to running them individually.

use digstore_chain::dig::{self, format_dig, format_xch};
use digstore_chain::{config as chain_config, seed as chain_seed, unlock};

use crate::cli::{SeedAction, SeedArgs, SetupArgs};
use crate::error::CliError;
use crate::ops::dighub;
use crate::ui::Ui;

/// Where to acquire XCH/DIG — shown when the wallet is short on funds. A pointer,
/// not a transaction: funding is the user's call, off-CLI.
const FUNDING_HINT: &str = "Get XCH/DIG: see https://dig.net (and any Chia exchange for XCH).";

pub fn run(ui: &Ui, args: SetupArgs) -> Result<(), CliError> {
    let home = chain_config::dig_home().map_err(CliError::from)?;
    let seed_path = chain_config::seed_path(&home);
    let session_path = chain_config::session_path(&home);
    // A live unlock session means a usable wallet even without an on-disk
    // `seed.enc` (a session-only setup), so treat either as "seed present" — the
    // same rule `doctor` uses, so the two commands agree on what "set up" means.
    let have_seed = chain_seed::seed_exists(&seed_path) || unlock::is_unlocked(&session_path);

    // Collect a small status record so `--json` callers get a structured result.
    let mut seed_action = "kept";

    // --- Step 1: seed ---------------------------------------------------------
    section(ui, "1/3  Wallet seed");
    if args.generate {
        run_seed(ui, SeedAction::Generate { words: 24 })?;
        seed_action = "generated";
    } else if args.import {
        run_seed(ui, SeedAction::Import { mnemonic: None })?;
        seed_action = "imported";
    } else if have_seed {
        // A seed already exists and the user didn't force a (re)create — keep it.
        if !ui.json() {
            ui.line("  a wallet seed is already set up — keeping it.");
        }
    } else if ui.can_prompt() {
        // No seed, interactive, no flag: ask whether to import or generate.
        if ui.confirm("No wallet seed yet. Import an existing mnemonic?", false) {
            run_seed(ui, SeedAction::Import { mnemonic: None })?;
            seed_action = "imported";
        } else {
            run_seed(ui, SeedAction::Generate { words: 24 })?;
            seed_action = "generated";
        }
    } else {
        // No seed and can't prompt (CI/--json): be explicit instead of hanging.
        return Err(CliError::InvalidArgument(
            "no wallet seed and nothing to do non-interactively; pass --generate or --import"
                .into(),
        ));
    }

    // --- Step 2: funds --------------------------------------------------------
    section(ui, "2/3  Funds");
    let funds = fund_check(ui);

    // --- Step 3: optional dighub login ---------------------------------------
    section(ui, "3/3  Dighub account (optional)");
    let logged_in = if args.no_login {
        if !ui.json() {
            ui.line("  skipped (--no-login).");
        }
        dighub::valid_session().is_some()
    } else if let Some(s) = dighub::valid_session() {
        if !ui.json() {
            let who = s.handle.clone().unwrap_or_else(|| "logged in".to_string());
            ui.line(format!("  already logged in as {who}."));
        }
        true
    } else if ui.can_prompt() {
        if ui.confirm(
            "Log in to dighub now (so your stores show in your dashboard)?",
            true,
        ) {
            // Reuse the exact login path; a login failure here is non-fatal — the
            // seed is what publishing needs, login only gates the public hub push.
            match crate::commands::login::run(ui, crate::cli::LoginArgs {}) {
                Ok(()) => true,
                Err(e) => {
                    ui.error(&e);
                    false
                }
            }
        } else {
            ui.line("  skipped — run `digstore login` anytime.");
            false
        }
    } else {
        false
    };

    // --- Summary --------------------------------------------------------------
    // The one-place explanation of the two-identity model (the whole point of #21).
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "seed": seed_action,
            "seed_unlocked": unlock::is_unlocked(&session_path),
            "funds_ok": funds,
            "logged_in": logged_in,
        }));
    } else {
        ui.line("");
        ui.success("Setup complete.");
        ui.line("  Your SEED signs the chain and pays per publish (it never leaves this machine).");
        ui.line("  Your LOGIN only gates the push to the public hub — no on-chain authority.");
        ui.line("");
        ui.line("Next:");
        ui.line("  digstore new <template>   # start a store (free)");
        ui.line("  digstore doctor           # re-check you're ready to publish");
    }
    Ok(())
}

/// Print a step header (suppressed in JSON mode).
fn section(ui: &Ui, title: &str) {
    if !ui.json() {
        ui.line("");
        ui.line(title);
    }
}

/// Run the shared `seed` command path for one action (no duplicate seed logic).
fn run_seed(ui: &Ui, action: SeedAction) -> Result<(), CliError> {
    crate::commands::seed::run(ui, SeedArgs { action })
}

/// Check the wallet has enough DIG + XCH for a publish (100 DIG + the fee). Only
/// scans when the seed is unlocked (so `setup --json`/CI never blocks on a
/// passphrase). Returns `Some(true)`/`Some(false)` when checked, `None` when
/// skipped. Mirrors `doctor`'s funds check; kept best-effort (a scan error is a
/// soft note, never a setup failure).
fn fund_check(ui: &Ui) -> Option<bool> {
    let home = chain_config::dig_home().ok()?;
    if !unlock::is_unlocked(&chain_config::session_path(&home)) {
        if !ui.json() {
            ui.line("  seed is locked — unlock it to check funds (it'll prompt at publish).");
        }
        return None;
    }
    match scan_balances(ui) {
        Ok((have_dig, have_xch, fee)) => {
            let need_dig = dig::COMMIT_DIG;
            let ok = have_dig >= need_dig && have_xch >= fee;
            if !ui.json() {
                ui.line(format!(
                    "  you have {} DIG and {} XCH (need {} DIG + ~{} XCH per publish).",
                    format_dig(have_dig),
                    format_xch(have_xch),
                    format_dig(need_dig),
                    format_xch(fee),
                ));
                if !ok {
                    ui.line(format!("  not enough yet. {FUNDING_HINT}"));
                }
            }
            Some(ok)
        }
        Err(e) => {
            if !ui.json() {
                ui.line(format!("  could not scan wallet: {e}"));
            }
            None
        }
    }
}

/// Scan the wallet once (unlocked seed) → `(dig, xch, fee)`. Reuses the shared
/// anchor gate so the mock backend is honored in tests/CI.
fn scan_balances(ui: &Ui) -> Result<(u64, u64, u64), CliError> {
    use crate::runtime::block_on;
    let (_keys, mnemonic, anchor, _mocked, fee) = crate::ops::anchor_backend::prepare_anchor(ui)?;
    let w = block_on(anchor.scan(&mnemonic))??;
    let have_xch = block_on(anchor.balance(&w))??;
    let have_dig = block_on(anchor.dig_balance(&w))??;
    Ok((have_dig, have_xch, fee))
}
