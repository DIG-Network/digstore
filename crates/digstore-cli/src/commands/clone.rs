use digstore_core::Urn;

use crate::cli::CloneArgs;
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::remote_ops;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: CloneArgs) -> Result<(), CliError> {
    let store_url = if args.source.starts_with("urn:dig:") {
        let urn = Urn::parse(&args.source)
            .map_err(|e| CliError::InvalidArgument(format!("bad urn: {e}")))?;
        let base = config::resolve_remote_url(ctx, "origin").map_err(|_| {
            CliError::InvalidArgument("cloning a URN requires a configured `origin` remote".into())
        })?;
        // base is already a `…/stores/{id}` URL; rebuild from the URN's store id.
        let host = base.split("/stores/").next().unwrap_or(&base);
        format!(
            "{}/stores/{}",
            host.trim_end_matches('/'),
            urn.store_id.to_hex()
        )
    } else {
        args.source.clone()
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    let summary = rt.block_on(remote_ops::clone_from(ctx, &store_url))?;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "store_id": summary.store_id_hex,
            "root": summary.root_hex,
            "module_size": summary.module_size
        }));
    } else {
        ui.success(format!(
            "cloned {} at root {} ({} bytes)",
            summary.store_id_hex, summary.root_hex, summary.module_size
        ));
        ui.hint("digstore cat <urn>");
    }
    Ok(())
}
