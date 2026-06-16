use crate::cli::InitArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::anchor_state::{AnchorState, AnchorStatus};
use crate::ops::{anchor_backend, anchor_ux, store_ops};
use crate::runtime::block_on;
use digstore_chain::anchor::ConfirmState;
use digstore_chain::dig::{self, format_dig, format_xch};

/// `digstore init` MINTS an empty store singleton on Chia mainnet; the on-chain
/// launcher id becomes the store_id. This is a HARD GATE: no seed, no funds, or
/// a mint failure all fail `init` BEFORE any local store scaffold is created, so
/// there is nothing to roll back. The local scaffold is only written after the
/// mint succeeds; any post-mint confirmation failure (a timeout OR a transient
/// RPC error) KEEPS the (resumable) store on disk so it can be finished later
/// with `digstore anchor`.
pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: InitArgs) -> Result<(), CliError> {
    let name = args.name.clone().unwrap_or_else(|| "default".to_string());
    crate::workspace::validate_store_name(&name)?;

    // Load or create the workspace (migrating a legacy single-store layout first).
    let mut ws = crate::workspace::Workspace::load_or_migrate(&ctx.workspace_dir)?;
    if ws.stores.contains_key(&name) {
        return Err(CliError::InvalidArgument(format!(
            "store '{name}' already exists"
        )));
    }

    // Compute (WITHOUT creating) the on-disk store dir, and reject up front if a
    // store is already scaffolded there. `store_ops::init_store` performs the same
    // check, but only AFTER the mint — by which point we'd already have spent XCH
    // on a singleton we then orphan. The workspace registry and the on-disk layout
    // can disagree (e.g. a prior run minted+scaffolded but died before `ws.save()`,
    // or `stores.toml` was edited/deleted), so this disk-level guard must run
    // PRE-mint, in addition to the registry check above. Fail before spending.
    let store_dir = ws.store_dir(&name);
    if store_dir.join("config.toml").exists() {
        return Err(CliError::InvalidArgument(format!(
            "store already initialized at {}",
            store_dir.display()
        )));
    }

    // Interactive setup. When the relevant flags weren't supplied and we're on a
    // TTY, ask a couple of setup questions so the store is ready to use without a
    // separate `digstore dir` run. Non-interactive (scripts, --json, --quiet)
    // falls straight through to the flag values / defaults.
    let mut content_root = args.dir.clone();
    let mut private = args.private;
    if content_root.is_none() {
        content_root = ui.prompt_line(
            "Relative path to the build/content directory this store captures",
            ".",
        );
    }
    if !private {
        private = ui.confirm("Make this a private (salted) store?", false);
    }

    // --- HARD GATE: seed → balance → mint, all BEFORE any local files. ---

    // 1+2. Unlock the wallet seed (NoSeed → exit 9) and build the anchor backend
    //      (env-gated mock or real coinset mainnet), warning loudly if mocked.
    let (keys, mnemonic, anchor, mocked, fee) = anchor_backend::prepare_anchor(ui)?;

    // 3. Preflight balance for BOTH assets, with up-front cost disclosure. A
    //    mint pays INIT_DIG (100 DIG) embedded in the on-chain bundle PLUS the
    //    singleton amount (1 mojo) + the XCH fee. Block before any spend if the
    //    wallet is short on EITHER asset; no files exist yet, so nothing rolls back.
    let w = block_on(anchor.scan(&mnemonic))??;
    let have_xch = block_on(anchor.balance(&w))??;
    let have_dig = block_on(anchor.dig_balance(&w))??;

    if !ui.json() {
        ui.line(format!(
            "⛓  Minting a store on Chia mainnet costs {} DIG + up to {} XCH (fee).",
            format_dig(dig::INIT_DIG),
            format_xch(fee)
        ));
        ui.line(format!(
            "   you have {} DIG and {} XCH.",
            format_dig(have_dig),
            format_xch(have_xch)
        ));
    }

    let need_xch = fee + 1;
    if have_xch < need_xch {
        return Err(CliError::InsufficientFunds {
            need: need_xch,
            have: have_xch,
            address: digstore_chain::keys::owner_address(&keys),
            asset: "XCH".into(),
        });
    }
    if have_dig < dig::INIT_DIG {
        return Err(CliError::InsufficientFunds {
            need: dig::INIT_DIG,
            have: have_dig,
            address: digstore_chain::keys::owner_address(&keys),
            asset: "DIG".into(),
        });
    }

    // 4. Mint the empty store singleton. The launcher id becomes the store_id.
    //    Pass the full scanned wallet so the mint gathers XCH + DIG from all addresses.
    let mint = block_on(anchor.mint_empty_store(&w, fee))
        .and_then(|r| r.map_err(|e| CliError::MintFailed(e.to_string())))?;
    let store_id = {
        let mut a = [0u8; 32];
        a.copy_from_slice(mint.launcher_id.as_ref());
        digstore_core::Bytes32(a)
    };
    anchor_ux::report_submitted(ui, "mint", &store_id.to_hex(), ui.json());

    // --- Post-mint: create the local scaffold under the new store_id. ---
    // (`store_dir` was computed and existence-checked PRE-mint above; create it
    // only now that the mint has succeeded.)

    std::fs::create_dir_all(&store_dir).map_err(|e| CliError::Other(e.into()))?;
    let store_ctx = CliContext {
        dig_dir: store_dir,
        workspace_dir: ctx.workspace_dir.clone(),
        op_dir: ctx.op_dir.clone(),
        store_name: Some(name.clone()),
        json: ctx.json,
        verbose: ctx.verbose,
    };
    let res = store_ops::init_store(&store_ctx, private, None, Some(store_id), None, None)?;

    // Persist the on-chain anchor state (Pending until confirmed).
    let coin_id_hex = hex::encode(mint.coin_id.as_ref());
    let mut anchor_state = AnchorState {
        network: "mainnet".to_string(),
        store_id: store_id.to_hex(),
        coin_id: coin_id_hex.clone(),
        status: AnchorStatus::Pending,
        last_root: String::new(),
        last_tx_id: hex::encode(mint.tx_id.as_ref()),
        confirmed_height: 0,
    };
    anchor_state.save(&store_ctx.dig_dir)?;

    let first = ws.stores.is_empty();
    ws.register(&name, &store_id.to_hex(), content_root.clone())?;
    if first {
        ws.set_active(&name)?;
    }
    ws.save()?;

    // 5. Wait for the mint to confirm. The store + anchor.toml are ALREADY on disk
    //    (saved above), so it stays recoverable no matter how confirm exits:
    //    - confirmed     → mark Confirmed, exit 0.
    //    - still pending → keep the store Pending, return ConfirmTimeout (exit 14).
    //    - RPC error     → `?` below propagates CliError::Chain (exit 13); the
    //                      store is unchanged and still resumable.
    //    Any non-zero exit here is resumable via `digstore anchor`.
    let state = anchor_ux::confirm_with_ui(
        ui,
        anchor.as_ref(),
        mint.coin_id,
        args.wait_timeout,
        ui.json(),
    )?;
    anchor_state.apply_confirm(&state);
    anchor_state.save(&store_ctx.dig_dir)?;

    let confirmed = matches!(state, ConfirmState::Confirmed { .. });
    let content_root_display = content_root.clone().unwrap_or_else(|| ".".to_string());

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "store": name,
            "store_id": store_id.to_hex(),
            "coin_id": coin_id_hex,
            "anchor_status": if confirmed { "confirmed" } else { "pending" },
            "mocked": mocked,
            "content_root": content_root,
            "private": private,
            "active": first,
            "host_public_key": res.host_public_key.to_hex(),
        }));
    } else {
        ui.success(format!(
            "Initialized store '{}' ({})",
            name,
            store_id.to_hex()
        ));
        ui.line(format!("  content root: {content_root_display}"));
        if first {
            ui.line("  set as active store");
        }
        ui.line(format!(
            "  trusted host key: {}",
            res.host_public_key.to_hex()
        ));
        if confirmed {
            ui.line(format!("  anchored on mainnet (coin {coin_id_hex})"));
        } else {
            ui.line(format!(
                "  ⏳ anchored on mainnet (coin {coin_id_hex}) — not yet confirmed; run `digstore anchor`"
            ));
        }
    }

    if confirmed {
        if !ui.json() {
            ui.hint("digstore add -A");
        }
        Ok(())
    } else {
        // Resumable: the store is KEPT (Pending), but signal a non-zero exit.
        Err(CliError::ConfirmTimeout)
    }
}
