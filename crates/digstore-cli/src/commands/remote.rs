use crate::cli::{RemoteAction, RemoteArgs};
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;

pub fn run(ctx: &CliContext, args: RemoteArgs) -> Result<(), CliError> {
    match args.action {
        RemoteAction::Add { name, url } => {
            config::add_remote(ctx, &name, &url)?;
            if !ctx.json {
                println!("added remote {name} -> {url}");
            }
        }
        RemoteAction::Remove { name } => {
            config::remove_remote(ctx, &name)?;
            if !ctx.json {
                println!("removed remote {name}");
            }
        }
        RemoteAction::List => {
            let remotes = config::list_remotes(ctx)?;
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&remotes).unwrap());
            } else {
                for (name, url) in remotes {
                    println!("{name}\t{url}");
                }
            }
        }
    }
    Ok(())
}
