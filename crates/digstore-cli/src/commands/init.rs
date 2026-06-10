use crate::cli::InitArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

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

    // Per-store context for init_store (dig_dir = .dig/stores/<name>/).
    let store_dir = ws.store_dir(&name);
    std::fs::create_dir_all(&store_dir).map_err(|e| CliError::Other(e.into()))?;
    let store_ctx = CliContext {
        dig_dir: store_dir,
        workspace_dir: ctx.workspace_dir.clone(),
        op_dir: ctx.op_dir.clone(),
        store_name: Some(name.clone()),
        json: ctx.json,
        verbose: ctx.verbose,
    };
    let res = store_ops::init_store(&store_ctx, args.private, None)?;

    let first = ws.stores.is_empty();
    ws.register(&name, &res.store_id.to_hex(), args.dir.clone())?;
    if first {
        ws.set_active(&name)?;
    }
    ws.save()?;

    let content_root = args.dir.clone().unwrap_or_else(|| ".".to_string());
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "store": name,
            "store_id": res.store_id.to_hex(),
            "host_public_key": res.host_public_key.to_hex(),
            "content_root": args.dir,
            "active": first,
        }));
    } else {
        ui.success(format!(
            "Initialized store '{}' ({})",
            name,
            res.store_id.to_hex()
        ));
        ui.line(format!("  content root: {content_root}"));
        if first {
            ui.line("  set as active store");
        }
        ui.line(format!("  trusted host key: {}", res.host_public_key.to_hex()));
        ui.hint("digstore add -A");
    }
    Ok(())
}
