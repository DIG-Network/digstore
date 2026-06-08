use crate::cli::InitArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

pub fn run(ctx: &CliContext, args: InitArgs) -> Result<(), CliError> {
    let res = store_ops::init_store(ctx, args.private, args.data_dir)?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "store_id": res.store_id.to_hex(),
                "host_public_key": res.host_public_key.to_hex(),
            })
        );
    } else {
        println!("Initialized digstore {}", res.store_id.to_hex());
        println!("  dig dir: {}", ctx.dig_dir.display());
        println!("  trusted host key: {}", res.host_public_key.to_hex());
    }
    Ok(())
}
