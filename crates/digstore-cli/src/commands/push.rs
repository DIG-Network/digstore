use crate::cli::PushArgs;
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::{dighub, remote_ops, store_ops};

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: PushArgs) -> Result<(), CliError> {
    // Product gate: require a dighub account (does NOT change the store-key/§21.9
    // push owner-auth, which is unchanged below).
    dighub::ensure_logged_in(ui)?;
    let base = config::resolve_remote_url(ctx, &args.remote)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    let root = rt.block_on(remote_ops::push_to(ctx, ui, &base))?;

    // Link the store to the logged-in dighub account so it appears in the owner's dashboard.
    // The hub adopts the (otherwise owner-less) pushed record; the anchor-watcher then verifies
    // on-chain ownership before it goes live. Best-effort: a claim failure NEVER fails the push.
    let store_pubkey = store_ops::load_signing_key(ctx)
        .ok()
        .map(|sk| hex::encode(sk.public_key().to_bytes().0))
        .unwrap_or_default();
    let claimed = rt
        .block_on(dighub::claim_pushed_store(
            &ctx.load_config()?.store_id.to_hex(),
            &store_pubkey,
        ))
        .unwrap_or(false);

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "pushed_root": root.to_hex(),
            "claimed": claimed,
        }));
    } else {
        ui.success(format!("pushed root {} to {}", root.to_hex(), args.remote));
        if claimed {
            ui.line("linked to your dighub account (pending on-chain owner verification)");
        }
    }
    Ok(())
}
