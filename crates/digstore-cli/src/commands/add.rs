use crate::cli::AddArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

pub fn run(ctx: &CliContext, args: AddArgs) -> Result<(), CliError> {
    let res = store_ops::add_path(ctx, &args.path, args.key)?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({ "resource_key": res.resource_key, "chunks": res.chunk_count, "size": res.total_size })
        );
    } else {
        println!(
            "staged {} ({} bytes, {} chunks)",
            res.resource_key, res.total_size, res.chunk_count
        );
    }
    Ok(())
}
