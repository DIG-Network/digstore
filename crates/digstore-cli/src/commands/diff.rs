use digstore_core::Bytes32;

use crate::cli::DiffArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::output;

pub fn run(ctx: &CliContext, _ui: &crate::ui::Ui, args: DiffArgs) -> Result<(), CliError> {
    let from = Bytes32::from_hex(&args.from)
        .map_err(|_| CliError::InvalidArgument("from must be 32-byte hex".into()))?;
    let to = Bytes32::from_hex(&args.to)
        .map_err(|_| CliError::InvalidArgument("to must be 32-byte hex".into()))?;
    let entries = store_ops::diff(ctx, &from, &to)?;
    print!("{}", output::render_diff(&entries, ctx.json));
    Ok(())
}
