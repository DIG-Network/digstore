use crate::context::CliContext;
use crate::error::CliError;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, args: crate::cli::UrnArgs) -> Result<(), CliError> {
    if args.paths.is_empty() && !args.all {
        return Err(CliError::InvalidArgument(
            "nothing to preview: pass paths or -A".into(),
        ));
    }
    let previews =
        crate::ops::store_ops::preview_urns(ctx, &args.paths, args.all, args.root.as_deref())?;
    if ui.json() {
        ui.emit_json(
            &previews
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "path": p.path, "key": p.key, "urn": p.urn, "retrieval_key": p.retrieval_key,
                    })
                })
                .collect::<Vec<_>>(),
        );
        return Ok(());
    }
    for p in &previews {
        ui.line(format!("{}\t{}", p.key, p.urn));
    }
    Ok(())
}
