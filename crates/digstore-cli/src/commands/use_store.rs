use crate::context::CliContext;
use crate::error::CliError;
use crate::ui::Ui;
use crate::workspace::Workspace;

pub fn run(
    _ctx: &CliContext,
    ui: &Ui,
    ws: &mut Workspace,
    args: crate::cli::UseArgs,
) -> Result<(), CliError> {
    ws.set_active(&args.name)?;
    ws.save()?;
    let cr = ws.content_root(&args.name).unwrap_or_else(|| ".".into());
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "active": args.name, "content_root": ws.content_root(&args.name) }));
    } else {
        ui.success(format!("active project is now '{}'", args.name));
        ui.line(format!("  content root: {cr}"));
    }
    Ok(())
}
