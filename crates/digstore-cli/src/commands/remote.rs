use crate::cli::{RemoteAction, RemoteArgs};
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: RemoteArgs) -> Result<(), CliError> {
    match args.action {
        RemoteAction::Add { name, url } => {
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
