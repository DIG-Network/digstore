use crate::cli::LogArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::output;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: LogArgs) -> Result<(), CliError> {
    let entries = store_ops::log(ctx, args.limit)?;
    if ui.json() {
        ui.emit_json(&entries);
    } else {
        let text = output::render_log(&entries, false);
        let trimmed = text.trim_end_matches('\n');
        if !trimmed.is_empty() {
            ui.line(trimmed);
        }
    }
    Ok(())
}
