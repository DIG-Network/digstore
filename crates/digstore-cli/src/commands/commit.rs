use crate::cli::CommitArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: CommitArgs) -> Result<(), CliError> {
    let res = store_ops::commit(ctx, args.message)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "root": res.roothash.to_hex(),
            "module": res.output_path.display().to_string(),
            "size": res.output_size
        }));
    } else {
        ui.success(format!("committed root {}", res.roothash.to_hex()));
        ui.line(format!(
            "  module: {} ({} bytes)",
            res.output_path.display(),
            res.output_size
        ));
        ui.hint("digstore push origin");
    }
    Ok(())
}
