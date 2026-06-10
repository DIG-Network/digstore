use digstore_core::Urn;

use crate::cli::CloneArgs;
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::remote_ops;

pub fn run(ws_ctx: &CliContext, ui: &crate::ui::Ui, args: CloneArgs) -> Result<(), CliError> {
    // `clone` CREATES a store, like `init`: install it as the workspace's
    // `default` store under `<workspace>/stores/default/` and register it in
    // workspace.toml. The incoming context is workspace-only.
    let name = "default";
    let mut workspace = crate::workspace::Workspace::load_or_migrate(&ws_ctx.workspace_dir)?;
    if workspace.stores.contains_key(name) {
        return Err(CliError::InvalidArgument(format!(
            "store '{name}' already exists in this workspace"
        )));
    }
    let store_dir = workspace.store_dir(name);
    std::fs::create_dir_all(&store_dir).map_err(|e| CliError::Other(e.into()))?;
    let ctx = &CliContext {
        dig_dir: store_dir,
        workspace_dir: ws_ctx.workspace_dir.clone(),
        op_dir: ws_ctx.op_dir.clone(),
        store_name: Some(name.to_string()),
        json: ws_ctx.json,
        verbose: ws_ctx.verbose,
    };

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

    // Register the cloned store and make it active if it is the first one.
    let first = workspace.stores.is_empty();
    workspace.register(name, &summary.store_id_hex, None)?;
    if first {
        workspace.set_active(name)?;
    }
    workspace.save()?;

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
