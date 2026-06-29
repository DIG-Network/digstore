//! `digstore link <storeId|urn>` — attach the current folder to an EXISTING
//! project (web↔CLI bridge, roadmap #20).
//!
//! After a developer creates a project in the hub (or on-chain), `link` makes the
//! local folder redeploy-ready by writing a committable `dig.toml` that pins the
//! project's store id (and, from a full URN, its remote) and registering `origin`.
//! It is deliberately CHEAP and OFFLINE: no mint, no spend, no content download,
//! no seed — it only records WHERE to publish. The first redeploy
//! (`digstore deploy`) then reconstructs the store from the publisher deploy key.
//!
//! Accepts either a 64-hex store id or a `urn:dig:chia:<storeID>[:<root>]` URN
//! (the share link the hub hands out), so the user can paste whichever they have.

use digstore_core::{Bytes32, Urn};

use crate::cli::LinkArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ui::Ui;

/// Parse the link target into a store id. Accepts a bare 64-hex id or a
/// `urn:dig:…` URN (the store id is extracted from it).
fn parse_target(target: &str) -> Result<Bytes32, CliError> {
    let t = target.trim();
    if t.starts_with("urn:") {
        let urn = Urn::parse(t)
            .map_err(|e| CliError::InvalidArgument(format!("not a valid dig URN: {e}")))?;
        return Ok(urn.store_id);
    }
    Bytes32::from_hex(t).map_err(|_| {
        CliError::InvalidArgument(
            "target must be a 64-hex store id or a urn:dig:chia:<storeID> URN".into(),
        )
    })
}

/// Render the `dig.toml` for a linked project. Kept as a pure string-builder so it
/// is unit-testable and the generated file stays human-readable + comment-rich.
fn render_dig_toml(store_id: &Bytes32, output_dir: &str, remote: &str) -> String {
    format!(
        "# dig.toml — linked to an existing DIG store. Committed to your repo; NO secrets.\n\
         #\n\
         # `digstore dev`    — preview locally for free (no chain, no spend).\n\
         # `digstore deploy` — publish a new version (costs the capsule price in $DIG + an XCH fee).\n\
         \n\
         # The on-chain store this folder publishes to.\n\
         store-id = \"{store_id}\"\n\
         \n\
         # The folder `deploy` publishes (your build output).\n\
         output-dir = \"{output_dir}\"\n\
         \n\
         # Where to publish. The public DIGHUb by default for this store.\n\
         remote = \"{remote}\"\n\
         \n\
         # Uncomment to run a build before each deploy.\n\
         # build-command = \"npm ci && npm run build\"\n",
        store_id = store_id.to_hex(),
    )
}

pub fn run(ctx: &CliContext, ui: &Ui, args: LinkArgs) -> Result<(), CliError> {
    let store_id = parse_target(&args.target)?;

    // The link folder is the operating directory (where the developer ran the
    // command); `dig.toml` is committed alongside their source.
    let toml_path = ctx.op_dir.join("dig.toml");
    if toml_path.exists() && !args.force {
        return Err(CliError::InvalidArgument(format!(
            "a dig.toml already exists at {}; pass --force to overwrite",
            toml_path.display()
        )));
    }

    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| "dist".to_string());
    // Default remote: the canonical public dig:// remote for this store (the hub
    // resolves it). An explicit --remote overrides for self-hosted nodes.
    let remote = args
        .remote
        .clone()
        .unwrap_or_else(|| format!("dig://{}", store_id.to_hex()));

    let contents = render_dig_toml(&store_id, &output_dir, &remote);
    std::fs::write(&toml_path, &contents)
        .map_err(|e| CliError::Other(anyhow::anyhow!("write {}: {e}", toml_path.display())))?;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "linked": true,
            "store_id": store_id.to_hex(),
            "output_dir": output_dir,
            "remote": remote,
            "dig_toml": toml_path.display().to_string(),
        }));
    } else {
        ui.success(format!("Linked this folder to store {}", store_id.to_hex()));
        ui.line(format!("  wrote {}", toml_path.display()));
        ui.line(format!("  output: {output_dir}    remote: {remote}"));
        ui.line("");
        ui.line("Next:");
        ui.line("  digstore dev      # preview locally for free");
        ui.line("  digstore deploy   # publish a new version (needs your deploy key)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_store_id() {
        let id = "ab".repeat(32);
        let parsed = parse_target(&id).unwrap();
        assert_eq!(parsed.to_hex(), id);
    }

    #[test]
    fn parses_store_id_from_urn() {
        let id = "cd".repeat(32);
        let urn = format!("urn:dig:chia:{id}");
        let parsed = parse_target(&urn).unwrap();
        assert_eq!(parsed.to_hex(), id);
    }

    #[test]
    fn parses_store_id_from_urn_with_root() {
        let id = "ef".repeat(32);
        let root = "12".repeat(32);
        let urn = format!("urn:dig:chia:{id}:{root}");
        let parsed = parse_target(&urn).unwrap();
        assert_eq!(parsed.to_hex(), id);
    }

    #[test]
    fn rejects_garbage_target() {
        assert!(parse_target("not-a-store").is_err());
        assert!(parse_target("urn:dig:chia:nothex").is_err());
    }

    #[test]
    fn rendered_toml_pins_store_and_remote() {
        let id = Bytes32::from_hex(&"ab".repeat(32)).unwrap();
        let t = render_dig_toml(&id, "dist", "dig://abc");
        assert!(t.contains(&format!("store-id = \"{}\"", id.to_hex())));
        assert!(t.contains("output-dir = \"dist\""));
        assert!(t.contains("remote = \"dig://abc\""));
    }
}
