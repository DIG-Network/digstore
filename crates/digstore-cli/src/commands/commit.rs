use crate::cli::CommitArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

pub fn run(ctx: &CliContext, _ui: &crate::ui::Ui, args: CommitArgs) -> Result<(), CliError> {
    let res = store_ops::commit(ctx, args.message)?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({ "root": res.roothash.to_hex(), "module": res.output_path.display().to_string(), "size": res.output_size })
        );
    } else {
        println!("committed root {}", res.roothash.to_hex());
        println!(
            "  module: {} ({} bytes)",
            res.output_path.display(),
            res.output_size
        );
    }
    Ok(())
}
