use crate::cli::StatusArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::output;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, _args: StatusArgs) -> Result<(), CliError> {
    let view = store_ops::compute_status(ctx)?;
    output::render_status(&view, ui);
    Ok(())
}
