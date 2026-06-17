use crate::cli::PushArgs;
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::{dighub, remote_ops};

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
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "pushed_root": root.to_hex() }));
    } else {
        ui.success(format!("pushed root {} to {}", root.to_hex(), args.remote));
    }
    Ok(())
}
