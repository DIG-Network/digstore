use crate::cli::AddArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

pub fn run(ctx: &CliContext, _ui: &crate::ui::Ui, args: AddArgs) -> Result<(), CliError> {
    // §8.5 social conventions: `--discovery` stages the
    // `/.well-known/dig/manifest.json` discovery manifest instead of a file.
    if args.discovery {
        let manifest = store_ops::stage_discovery_manifest(ctx)?;
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({
                    "resource_key": crate::ops::discovery::DISCOVERY_RESOURCE_KEY,
                    "resources": manifest.resources,
                })
            );
        } else {
            println!(
                "staged discovery manifest {} ({} resources)",
                crate::ops::discovery::DISCOVERY_RESOURCE_KEY,
                manifest.resources.len()
            );
        }
        return Ok(());
    }

    let path = args.path.ok_or_else(|| {
        CliError::InvalidArgument("add requires a path (or use --discovery)".into())
    })?;
    let res = store_ops::add_path(ctx, &path, args.key)?;
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
