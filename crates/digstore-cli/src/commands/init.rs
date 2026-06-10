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
    let res = store_ops::init_store(&store_ctx, private, None)?;

    let first = ws.stores.is_empty();
    ws.register(&name, &res.store_id.to_hex(), content_root.clone())?;
    if first {
        ws.set_active(&name)?;
    }
    ws.save()?;

    let content_root_display = content_root.clone().unwrap_or_else(|| ".".to_string());
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "store": name,
            "store_id": res.store_id.to_hex(),
            "host_public_key": res.host_public_key.to_hex(),
            "content_root": content_root,
            "private": private,
            "active": first,
        }));
    } else {
        ui.success(format!(
            "Initialized store '{}' ({})",
            name,
            res.store_id.to_hex()
        ));
        ui.line(format!("  content root: {content_root_display}"));
        if first {
            ui.line("  set as active store");
        }
        ui.line(format!(
            "  trusted host key: {}",
            res.host_public_key.to_hex()
        ));
        ui.hint("digstore add -A");
    }
    Ok(())
}
