use crate::cli::{RemoteAction, RemoteArgs};
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::dighub;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: RemoteArgs) -> Result<(), CliError> {
    match args.action {
        RemoteAction::Add { name, url } => {
            // Interactive: prompt for either positional when omitted. Non-interactive: a clear
            // "pass <name>/<url>" error rather than clap's terse usage.
            let name = match name {
                Some(n) => n,
                None => ui.require_input("Remote name (e.g. origin)", "<name>")?,
            };
            // Origin auto-fill (cosmetic, never fails): when adding `origin` with a
            // logged-in handle, default the URL to `https://<handle>@rpc.dig.net`
            // instead of prompting; and if a URL is given without `userinfo@`, inject
            // the handle. Falls back to the normal prompt when no handle is known.
            let handle = if name == "origin" {
                dighub::current_handle()
            } else {
                None
            };
            let url = match url {
                Some(u) => match &handle {
                    Some(h) => dighub::inject_handle(&u, h),
                    None => u,
                },
                None => match &handle {
                    Some(h) => dighub::default_origin_url(h),
                    None => ui.require_input(
                        "Remote URL (e.g. https://<username>@rpc.dig.net)",
                        "<url>",
                    )?,
                },
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
