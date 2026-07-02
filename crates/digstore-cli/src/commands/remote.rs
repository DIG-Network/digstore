use crate::cli::{RemoteAction, RemoteArgs};
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::anchor_backend;
use crate::ops::anchor_ux;
use crate::ops::authorize;
use crate::runtime::block_on;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: RemoteArgs) -> Result<(), CliError> {
    match args.action {
        RemoteAction::Add { name, url } => {
            // Interactive: prompt for either positional when omitted. Non-interactive: a clear
            // "pass <name>/<url>" error rather than clap's terse usage.
            let name = match name {
                Some(n) => n,
                None => ui.require_input("Remote name (e.g. origin)", "<name>")?,
            };
            // No URL given: `origin` defaults to the public RPC. Identity is the owner
            // puzzle hash, authenticated by the keys on push — the origin needs no
            // username and no store id (push/pull take the store id from the local
            // store). Any other remote name is prompted for.
            let url = match url {
                Some(u) => u,
                None if name == "origin" => "https://rpc.dig.net".to_string(),
                None => ui.require_input("Remote URL (e.g. https://rpc.dig.net)", "<url>")?,
            };
            config::add_remote(ctx, &name, &url)?;
            ui.success(format!("added remote {name} -> {url}"));
        }
        RemoteAction::Remove { name } => {
            config::remove_remote(ctx, &name)?;
            ui.success(format!("removed remote {name}"));
        }
        RemoteAction::List => {
            let remotes = config::list_remotes(ctx)?;
            if ui.json() {
                ui.emit_json(&remotes);
            } else {
                for (name, url) in remotes {
                    ui.line(format!("{name}\t{url}"));
                }
            }
        }
        RemoteAction::Authorize { name, pubkey } => {
            set_authorization(ctx, ui, &name, pubkey.as_deref(), true)?;
        }
        RemoteAction::Deauthorize { name, pubkey } => {
            set_authorization(ctx, ui, &name, pubkey.as_deref(), false)?;
        }
    }
    Ok(())
}

/// Authorize (`add = true`) or deauthorize (`add = false`) the remote `name`'s RPC as
/// an on-chain WRITER for the active store: resolve the RPC's identity pubkey (explicit
/// `--pubkey` or discover it from the well-known endpoint), then owner-sign + broadcast
/// a delegation update. Idempotent — a no-op is reported, not an error. Shared by the
/// `remote authorize`/`deauthorize` subcommands AND (via [`ensure_origin_authorized`])
/// the `push` auto-prompt.
pub fn set_authorization(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    name: &str,
    explicit_pubkey: Option<&str>,
    add: bool,
) -> Result<(), CliError> {
    let store_id = ctx.find_store_id()?;
    // The anchor API speaks `chia_protocol::Bytes32`; `find_store_id` returns the
    // `digstore_core` twin — convert by raw bytes (identical 32-byte value).
    let launcher_id = chia_protocol::Bytes32::new(store_id.0);
    let remote_url = config::resolve_remote_url(ctx, name)?;
    let (writer_pk, pubkey_hex) = block_on(authorize::resolve_writer_pubkey(
        &remote_url,
        explicit_pubkey,
    ))??;

    let verb = if add { "authorize" } else { "deauthorize" };
    // Wallet + anchor gate (unlock seed, build backend, surface fee) — identical to
    // `commit`/`init`, so a mocked run warns loudly and a real run spends owner XCH.
    let (_keys, mnemonic, anchor, _mocked, fee) = anchor_backend::prepare_anchor(ui)?;
    let w = block_on(anchor.scan(&mnemonic))??;

    let outcome = block_on(anchor.set_writer_authorization(launcher_id, writer_pk, add, &w, fee))??;

    if !outcome.changed {
        // Already in the desired state — no spend, no fee. Report as success (idempotent).
        let state = if add {
            "already authorized"
        } else {
            "not authorized"
        };
        if ui.json() {
            ui.emit_json(&serde_json::json!({
                "changed": false,
                "authorized": add,
                "pubkey": pubkey_hex,
                "remote": name,
                "store_id": store_id.to_hex(),
            }));
        } else {
            ui.success(format!("{name} ({pubkey_hex}) is {state} — nothing to do"));
        }
        return Ok(());
    }

    // A spend was broadcast — wait for confirmation (like commit/init).
    if let Some(coin_id) = outcome.new_coin_id {
        anchor_ux::confirm_with_ui(ui, anchor.as_ref(), coin_id, 120, ui.json())?;
    }

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "changed": true,
            "authorized": add,
            "pubkey": pubkey_hex,
            "remote": name,
            "store_id": store_id.to_hex(),
            "tx_id": outcome.tx_id.map(|t| hex::encode(t.as_ref())),
        }));
    } else {
        ui.success(format!(
            "{}d {name} ({pubkey_hex}) as a writer for store {}",
            verb,
            store_id.to_hex()
        ));
        if add {
            ui.line("That RPC can now advance this store's root on your behalf.");
        }
    }
    Ok(())
}

/// On `push`, ensure the origin RPC is an authorized writer of the store, offering to
/// authorize it when it is not (the #172 push auto-prompt). Behavior:
/// - `no_auth = true` → skip entirely (push-only; CI / out-of-band authorization).
/// - the origin advertises no discoverable identity → skip silently (nothing to offer;
///   e.g. an anonymous read mirror). A push still authenticates via §21.9 as before.
/// - already an authorized writer → no-op.
/// - otherwise → prompt (auto-approved by the global `--yes`); on approval, run the
///   owner-signed authorization. Declining is NOT an error — the push proceeds.
///
/// Best-effort + non-fatal: a discovery/chain hiccup here NEVER fails the push; it
/// logs a note and continues, so the auth prompt can never block shipping content.
pub fn ensure_origin_authorized(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    remote_name: &str,
    remote_url: &str,
    no_auth: bool,
) -> Result<(), CliError> {
    if no_auth {
        return Ok(());
    }
    // Discover the origin's identity pubkey; a remote with none (empty/404) has nothing
    // to authorize — skip silently.
    let discovered = match block_on(async {
        digstore_remote::DigClient::new(remote_url.to_string())
            .discover_pubkey()
            .await
    }) {
        Ok(Ok(Some(hex))) => hex,
        // No identity advertised, or the endpoint is absent/unreachable: nothing to do.
        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => return Ok(()),
    };
    let Ok(writer_pk) = authorize::parse_identity_pubkey(&discovered) else {
        return Ok(()); // malformed advertised key — do not offer to authorize garbage.
    };

    // Is it already an authorized writer? (reads the chain). A read failure is non-fatal.
    let chain = digstore_chain::coinset::Coinset::mainnet();
    let launcher_id = chia_protocol::Bytes32::new(ctx.find_store_id()?.0);
    match block_on(authorize::is_authorized_writer_onchain(
        &chain,
        launcher_id,
        writer_pk,
    )) {
        Ok(Ok(true)) => return Ok(()), // already a writer — nothing to do.
        Ok(Ok(false)) => {}            // not a writer — offer to authorize below.
        Ok(Err(_)) | Err(_) => return Ok(()), // chain read failed — do not block the push.
    }

    let prompt = format!(
        "{remote_name} ({discovered}) is not yet an authorized writer for this store. \
         Authorize it now so it can advance the root on your behalf?"
    );
    // `--yes` auto-approves; interactive prompts; non-interactive without --yes declines
    // (default false), so an unattended push never blocks waiting for input.
    if !ui.confirm(&prompt, false) && !ui.assume_yes() {
        ui.line(
            "Skipped writer authorization; the push proceeds. Run \
                 `digstore remote authorize` later to enable it.",
        );
        return Ok(());
    }
    set_authorization(ctx, ui, remote_name, Some(&discovered), true)
}
