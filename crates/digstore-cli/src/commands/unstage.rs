use crate::context::CliContext;
use crate::error::CliError;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, _args: crate::cli::UnstageArgs) -> Result<(), CliError> {
    let cleared = crate::ops::store_ops::clear_staging(ctx)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "cleared": cleared }));
    } else {
        ui.success(format!(
            "cleared {cleared} staged entr{}",
            if cleared == 1 { "y" } else { "ies" }
        ));
    }
    Ok(())
}
