use crate::cli::PullArgs;
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::remote_ops;

pub fn run(ctx: &CliContext, args: PullArgs) -> Result<(), CliError> {
    let base = config::resolve_remote_url(ctx, &args.remote)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    let root = rt.block_on(remote_ops::pull_from(ctx, &base))?;
    if ctx.json {
        println!("{}", serde_json::json!({ "root": root.to_hex() }));
    } else {
        println!("pulled; local root is now {}", root.to_hex());
    }
    Ok(())
}
