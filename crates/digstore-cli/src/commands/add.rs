use crate::cli::AddArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::ui::theme::Marker;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, args: AddArgs) -> Result<(), CliError> {
    if args.discovery {
        return run_discovery(ctx, ui);
    }
    if args.paths.is_empty() && !args.all {
        return Err(CliError::InvalidArgument(
            "nothing to add: pass paths, or -A to stage everything".into(),
        ));
    }
    let outcome = store_ops::add_files(ctx, &args.paths, args.all, args.dry_run, args.key)?;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "staged": outcome.staged.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            "unchanged": outcome.unchanged,
            "dry_run": outcome.dry_run,
            "staged_bytes": outcome.staged_bytes,
            "limit_bytes": outcome.limit_bytes,
        }));
        return Ok(());
    }
    let verb = if outcome.dry_run {
        "Would stage"
    } else {
        "Staged"
    };
    ui.verb(verb, format!("{} file(s)", outcome.staged.len()));
    for (k, _size) in &outcome.staged {
        ui.item(Marker::Staged, k);
    }
    if outcome.unchanged > 0 {
        ui.line(format!("  {} unchanged", outcome.unchanged));
    }
    ui.capacity(outcome.staged_bytes, outcome.limit_bytes);
    if !outcome.dry_run && !outcome.staged.is_empty() {
        ui.hint("digstore commit -m \"...\"");
    }
    Ok(())
}

fn run_discovery(ctx: &CliContext, ui: &Ui) -> Result<(), CliError> {
    let manifest = store_ops::stage_discovery_manifest(ctx)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "resource_key": crate::ops::discovery::DISCOVERY_RESOURCE_KEY,
            "resources": manifest.resources,
        }));
    } else {
        println!(
            "staged discovery manifest {} ({} resources)",
            crate::ops::discovery::DISCOVERY_RESOURCE_KEY,
            manifest.resources.len()
        );
    }
    Ok(())
}
