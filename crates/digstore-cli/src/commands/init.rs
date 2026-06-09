use crate::cli::InitArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: InitArgs) -> Result<(), CliError> {
    let res = store_ops::init_store(ctx, args.private, args.data_dir)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "store_id": res.store_id.to_hex(),
            "host_public_key": res.host_public_key.to_hex(),
        }));
    } else {
        ui.success(format!("Initialized digstore {}", res.store_id.to_hex()));
        ui.line(format!("  dig dir: {}", ctx.dig_dir.display()));
        ui.line(format!(
            "  trusted host key: {}",
            res.host_public_key.to_hex()
        ));
        ui.hint("digstore add -A");
    }
    Ok(())
}
