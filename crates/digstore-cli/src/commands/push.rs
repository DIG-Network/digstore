use crate::cli::PushArgs;
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::{dighub, remote_ops, store_ops};

/// The outcome of a push: the pushed root and whether the dighub claim succeeded.
pub struct PushOutcome {
    pub root: digstore_core::Bytes32,
    pub claimed: bool,
}

/// Push the current committed root to `remote`, returning the outcome WITHOUT
/// emitting any output. Shared by [`run`] (which prints) and `commit`'s `--push`
/// path (which folds the result into its own single JSON/human output), so there
/// is exactly one push implementation and no double-emitted JSON object.
pub fn push_core(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    remote: &str,
) -> Result<PushOutcome, CliError> {
    let base = config::resolve_remote_url(ctx, remote)?;
    // Product gate: require a dighub account — but ONLY for dighub-managed remotes
    // (*.dig.net). Pushing to a self-hosted / loopback node needs no dighub account.
    // (Does NOT change the store-key/§21.9 push owner-auth, unchanged below.)
    if dighub::is_dighub_remote(&base) {
        dighub::ensure_logged_in(ui)?;
    }
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

    Ok(PushOutcome { root, claimed })
}

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: PushArgs) -> Result<(), CliError> {
    let out = push_core(ctx, ui, &args.remote)?;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "pushed_root": out.root.to_hex(),
            "claimed": out.claimed,
        }));
    } else {
        ui.success(format!(
            "pushed root {} to {}",
            out.root.to_hex(),
            args.remote
        ));
        if out.claimed {
            ui.line("linked to your dighub account (pending on-chain owner verification)");
        }
    }
    Ok(())
}
