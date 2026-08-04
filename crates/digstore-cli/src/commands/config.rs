//! `digstore config` — get/set/clear CLI configuration. Currently a single key,
//! `node.url` (`CLAUDE.md` §5.3): the persisted override for the client→node
//! resolution ladder, in either of two scopes.
//!
//! **Machine scope** (no `--local`) is the historical behaviour: one value in
//! the global dig config dir, applying wherever you run digs.
//!
//! **Project scope** (`--local`, #2099) writes `.dig/node.toml` in the current
//! project, so one project can point at the public gateway while another uses
//! your own node. It beats the machine-wide value because it is the narrower
//! scope. Setting it here also records your approval of it — see
//! `ops::node::trusted_project_node` for why a project value needs one.

use crate::cli::{ConfigAction, ConfigArgs};
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: ConfigArgs) -> Result<(), CliError> {
    match args.action {
        ConfigAction::NodeUrl {
            url,
            show,
            unset,
            local,
        } => node_url(ctx, ui, url, show, unset, local),
    }
}

/// What digs falls back to when no override is set — shown so the user can see
/// what will happen instead.
const LADDER_HINT: &str = "using the dig.local -> localhost -> rpc.dig.net ladder";

fn node_url(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    url: Option<String>,
    show: bool,
    unset: bool,
    local: bool,
) -> Result<(), CliError> {
    let scope = if local { "project" } else { "machine" };

    if unset {
        if local {
            config::unset_project_node_url_in(&ctx.workspace_dir)?;
        } else {
            config::unset_node_url()?;
        }
        if ui.json() {
            ui.emit_json(&serde_json::json!({ "node_url": null, "scope": scope }));
        } else {
            ui.success(format!("cleared the {scope} node.url ({LADDER_HINT})"));
        }
        return Ok(());
    }

    if show || url.is_none() {
        let current = if local {
            config::get_project_node_url_in(&ctx.workspace_dir)?
        } else {
            config::get_node_url()?
        };

        // A PROJECT value that has not been approved on this machine is NOT the
        // node digs will use — it is a proposal the resolver ignores (§14.3).
        // Printing it bare reads as "this is your node", which is precisely the
        // wrong impression to give about a value that arrived inside a
        // repository: a user checking why a clone behaves oddly would see the
        // attacker's URL presented as their configuration.
        let approved = match (local, current.as_deref()) {
            (true, Some(u)) => config::is_project_node_trusted_in(
                &config::global_config_dir()?,
                &ctx.workspace_dir,
                u,
            )?,
            _ => true,
        };

        if ui.json() {
            ui.emit_json(&serde_json::json!({
                "node_url": current,
                "scope": scope,
                // Machine-readable for the same reason: a script must be able to
                // tell a value in force from one being ignored.
                "in_effect": current.is_some() && approved,
                "approved": approved,
            }));
        } else {
            match current {
                Some(u) if approved => ui.line(u),
                Some(u) => {
                    ui.line(format!("{u}  (NOT APPROVED — ignored)"));
                    ui.hint(format!(
                        "this project declares {u}, but it has not been approved on this \
                         machine, so digs is using the ladder instead ({LADDER_HINT}). \
                         Approve it with `digstore config node.url --local {u}`."
                    ));
                }
                None => ui.line(format!("(unset — {LADDER_HINT})")),
            }
        }
        return Ok(());
    }

    // Safe: reached only when `url.is_some()` (the branch above returns).
    let url = url.expect("url is Some in the set path");
    if local {
        config::set_project_node_url_in(&ctx.workspace_dir, &url)?;
        // Typing the URL IS the approval: this value did not arrive inside a
        // repository, it came from this user on this machine just now. Without
        // recording it, `--local` would write a value its own next invocation
        // would then refuse to use.
        config::trust_project_node_in(&config::global_config_dir()?, &ctx.workspace_dir, &url)?;
    } else {
        config::set_node_url(&url)?;
    }
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "node_url": url, "scope": scope }));
    } else {
        ui.success(format!("{scope} node.url = {url}"));
    }
    Ok(())
}
