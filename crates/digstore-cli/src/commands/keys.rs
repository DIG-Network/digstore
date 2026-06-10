use digstore_core::Bytes32;

use crate::cli::KeysArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::ui::Ui;

/// List the retrieval key (and canonical URN) for every committed resource in a
/// generation. The retrieval key streams out the RAW ENCRYPTED bytes via
/// `digstore cat <retrieval-key>`; the URN streams them out decrypted.
pub fn run(ctx: &CliContext, ui: &Ui, args: KeysArgs) -> Result<(), CliError> {
    let cfg = ctx.load_config()?;

    let root: Bytes32 = match &args.root {
        Some(hex) => Bytes32::from_hex(hex)
            .map_err(|_| CliError::InvalidArgument(format!("bad root hex: {hex}")))?,
        None => store_ops::current_root(ctx)?
            .ok_or_else(|| CliError::NotFound("no committed root".into()))?,
    };

    let entries = store_ops::list_resource_keys(ctx, cfg.store_id, &root)?;

    if ui.json() {
        ui.emit_json(
            &entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "key": e.resource_key,
                        "urn": e.urn,
                        "retrieval_key": e.retrieval_key,
                    })
                })
                .collect::<Vec<_>>(),
        );
        return Ok(());
    }

    if entries.is_empty() {
        ui.line("(no resources in this generation)");
        return Ok(());
    }
    for e in &entries {
        ui.line(format!("{}  {}", e.retrieval_key, e.urn));
    }
    Ok(())
}
