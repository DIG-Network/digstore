use crate::cli::StatusArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::output;

pub fn run(ctx: &CliContext, _args: StatusArgs) -> Result<(), CliError> {
    let view = store_ops::status(ctx)?;
    print!("{}", output::render_status(&view, ctx.json));
    Ok(())
}
