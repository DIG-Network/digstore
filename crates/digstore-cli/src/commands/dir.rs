use crate::context::CliContext;
use crate::error::CliError;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, args: crate::cli::DirArgs) -> Result<(), CliError> {
    let name = ctx
        .store_name
        .clone()
        .ok_or_else(|| CliError::InvalidArgument("no store selected".into()))?;
    let mut ws = crate::workspace::Workspace::load_or_migrate(&ctx.workspace_dir)?;
    match args.path {
        None => {
            let cr = ws.content_root(&name).unwrap_or_else(|| ".".into());
            if ui.json() {
                ui.emit_json(&serde_json::json!({ "store": name, "content_root": ws.content_root(&name), "operating_dir": ctx.op_dir.display().to_string() }));
            } else {
                ui.line(format!("content root: {cr}"));
            }
        }
        Some(p) => {
            let value = p.to_string_lossy().replace('\\', "/");
            ws.set_content_root(&name, Some(value.clone()))?;
            ws.save()?;
            let project_root = ctx
                .workspace_dir
                .parent()
                .map(|x| x.to_path_buf())
                .unwrap_or_default();
            if !project_root.join(&value).exists() {
                ui.line(format!(
                    "note: '{value}' does not exist yet (build output dirs are often created later)"
                ));
            }
            if ui.json() {
                ui.emit_json(&serde_json::json!({ "store": name, "content_root": value }));
            } else {
                ui.success(format!("content root for '{name}' set to '{value}'"));
            }
        }
    }
    Ok(())
}
