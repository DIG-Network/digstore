//! `digstore config` — get/set/clear global CLI configuration. Currently a
//! single key, `node.url` (`CLAUDE.md` §5.3): the persisted, lowest-precedence
//! override for the client→node resolution ladder.

use crate::cli::{ConfigAction, ConfigArgs};
use crate::config;
use crate::error::CliError;

pub fn run(
    _ctx: &crate::context::CliContext,
    ui: &crate::ui::Ui,
    args: ConfigArgs,
) -> Result<(), CliError> {
    match args.action {
        ConfigAction::NodeUrl { url, show, unset } => node_url(ui, url, show, unset),
    }
}

fn node_url(
    ui: &crate::ui::Ui,
    url: Option<String>,
    show: bool,
    unset: bool,
) -> Result<(), CliError> {
    if unset {
        config::unset_node_url()?;
        if ui.json() {
            ui.emit_json(&serde_json::json!({ "node_url": null }));
        } else {
            ui.success("cleared node.url (falling back to the dig.local -> localhost -> rpc.dig.net ladder)");
        }
        return Ok(());
    }

    if show || url.is_none() {
        let current = config::get_node_url()?;
        if ui.json() {
            ui.emit_json(&serde_json::json!({ "node_url": current }));
        } else {
            match current {
                Some(u) => ui.line(u),
                None => ui.line("(unset — using the dig.local -> localhost -> rpc.dig.net ladder)"),
            }
        }
        return Ok(());
    }

    // Safe: reached only when `url.is_some()` (the `show || url.is_none()` branch above returns).
    let url = url.expect("url is Some in the set path");
    config::set_node_url(&url)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "node_url": url }));
    } else {
        ui.success(format!("node.url = {url}"));
    }
    Ok(())
}
