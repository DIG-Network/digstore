use crate::context::CliContext;
use crate::error::CliError;
use crate::ui::theme::Marker;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, _args: crate::cli::StagedArgs) -> Result<(), CliError> {
    let (entries, total, limit) = crate::ops::store_ops::list_staged(ctx)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "staged": entries.iter().map(|(k, s)| serde_json::json!({ "key": k, "size": s })).collect::<Vec<_>>(),
            "total_bytes": total,
            "limit_bytes": limit,
        }));
        return Ok(());
    }
    if entries.is_empty() {
        ui.line("nothing staged");
        ui.capacity(0, limit);
        return Ok(());
    }
    for (k, s) in &entries {
        ui.item(
            Marker::Staged,
            format!("{k}  ({:.1} MB)", *s as f64 / 1_000_000.0),
        );
    }
    ui.capacity(total, limit);
    Ok(())
}
