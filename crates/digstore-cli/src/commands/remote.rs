use crate::cli::{RemoteAction, RemoteArgs};
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: RemoteArgs) -> Result<(), CliError> {
    match args.action {
        RemoteAction::Add { name, url } => {
            // Interactive: prompt for either positional when omitted. Non-interactive: a clear
            // "pass <name>/<url>" error rather than clap's terse usage.
            let name = match name {
                Some(n) => n,
                None => ui.require_input("Remote name (e.g. origin)", "<name>")?,
            };
            // No URL given: `origin` defaults to the public RPC. Identity is the owner
            // puzzle hash, authenticated by the keys on push — the origin needs no
            // username and no store id (push/pull take the store id from the local
            // store). Any other remote name is prompted for.
            let url = match url {
                Some(u) => u,
                None if name == "origin" => "https://rpc.dig.net".to_string(),
                None => ui.require_input("Remote URL (e.g. https://rpc.dig.net)", "<url>")?,
            };
            config::add_remote(ctx, &name, &url)?;
            ui.success(format!("added remote {name} -> {url}"));
        }
        RemoteAction::Remove { name } => {
            config::remove_remote(ctx, &name)?;
            ui.success(format!("removed remote {name}"));
        }
        RemoteAction::List => {
            let remotes = config::list_remotes(ctx)?;
            if ui.json() {
                ui.emit_json(&remotes);
            } else {
                for (name, url) in remotes {
                    ui.line(format!("{name}\t{url}"));
                }
            }
        }
    }
    Ok(())
}
