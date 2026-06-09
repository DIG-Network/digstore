use crate::cli::LogArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::output;

pub fn run(ctx: &CliContext, _ui: &crate::ui::Ui, args: LogArgs) -> Result<(), CliError> {
    let entries = store_ops::log(ctx, args.limit)?;
    print!("{}", output::render_log(&entries, ctx.json));
    Ok(())
}
