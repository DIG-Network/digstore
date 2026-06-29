//! `digstore new <template>` — scaffold a working LOCAL project, free of any
//! wallet, chain, or spend.
//!
//! This closes the #1 journey gap: today the only way to start is `init`, which
//! MINTS on mainnet (spends the per-capsule $DIG price) into an empty directory. `new` instead
//! writes a runnable project — a `dig.toml`, a starter app, and (for the dapp /
//! NFT templates) a `window.chia` usage example — that the developer can preview
//! for free with `digstore dev` and only publish (spending DIG) once it's ready.
//!
//! It is deliberately separate from `init`: `new` never touches the chain, never
//! creates a `.dig` workspace, and never mints. The flow is:
//!   `digstore new <template>` → edit → `digstore dev` (free) → `digstore deploy`.

use std::path::Path;

use crate::cli::NewArgs;
use crate::error::CliError;
use crate::templates::{self, Template};
use crate::ui::Ui;

pub fn run(ui: &Ui, args: NewArgs) -> Result<(), CliError> {
    // `--list` (or an explicit request) just prints the catalog and exits 0.
    if args.list {
        return list_templates(ui);
    }

    let template = templates::find(&args.template).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "unknown template '{}'. Available: {}",
            args.template,
            templates::names()
        ))
    })?;

    // Default to the current directory so `digstore new static-site` scaffolds in
    // place (the common case); an explicit dir creates/uses that directory.
    let target = match &args.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir().map_err(|e| CliError::Other(e.into()))?,
    };

    write_template(ui, template, &target, args.force)
}

/// Print the template catalog (name + one-line description).
fn list_templates(ui: &Ui) -> Result<(), CliError> {
    if ui.json() {
        let arr: Vec<_> = templates::TEMPLATES
            .iter()
            .map(|t| serde_json::json!({ "name": t.name, "description": t.description }))
            .collect();
        ui.emit_json(&serde_json::json!({ "templates": arr }));
    } else {
        ui.line("Available templates:");
        for t in templates::TEMPLATES {
            ui.line(format!("  {:<18} {}", t.name, t.description));
        }
        ui.hint("digstore new <template> [dir]");
    }
    Ok(())
}

/// Materialize `template` into `target`, refusing to clobber an existing,
/// non-empty directory unless `--force` was given. A scaffold writes only the
/// template's own files; `--force` overwrites same-named files but leaves others.
fn write_template(
    ui: &Ui,
    template: &Template,
    target: &Path,
    force: bool,
) -> Result<(), CliError> {
    // Guard against silently overwriting a project: a non-empty target without
    // --force is an error (so we never stomp a developer's existing work).
    if target.exists() && dir_is_nonempty(target)? && !force {
        return Err(CliError::InvalidArgument(format!(
            "target directory '{}' is not empty; choose an empty/new directory or pass --force",
            target.display()
        )));
    }
    std::fs::create_dir_all(target).map_err(|e| CliError::Other(e.into()))?;

    let mut written = Vec::with_capacity(template.files.len());
    for file in template.files {
        let dest = target.join(file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CliError::Other(e.into()))?;
        }
        std::fs::write(&dest, file.contents)
            .map_err(|e| CliError::Other(anyhow::anyhow!("write {}: {e}", dest.display())))?;
        written.push(file.path);
    }

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "template": template.name,
            "dir": target.display().to_string(),
            "files": written,
            "minted": false,
            "spent": false,
        }));
    } else {
        ui.success(format!(
            "Created a {} store in {}",
            template.name,
            target.display()
        ));
        ui.line(format!(
            "  {} files · no wallet, no chain, no spend",
            written.len()
        ));
        ui.line("");
        ui.line("Next: preview it for free, then publish when it's ready.");
        // `digstore dev` runs from inside the project dir, so point the user there
        // first when they scaffolded into a named directory.
        if needs_cd(target) {
            ui.line(format!("  cd {}", display_cd(target)));
        }
        ui.line("  digstore dev      # free local preview (no spend)");
        ui.line("  digstore deploy   # publish on Chia (the capsule price in $DIG + an XCH fee)");
    }
    Ok(())
}

/// True when `dir` exists and contains at least one entry.
fn dir_is_nonempty(dir: &Path) -> Result<bool, CliError> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let mut entries = std::fs::read_dir(dir).map_err(|e| CliError::Other(e.into()))?;
    Ok(entries.next().is_some())
}

/// Whether we should suggest a `cd` (i.e. the target is not the current dir).
fn needs_cd(target: &Path) -> bool {
    match std::env::current_dir() {
        Ok(cwd) => {
            let canon_target = std::fs::canonicalize(target).ok();
            let canon_cwd = std::fs::canonicalize(&cwd).ok();
            canon_target != canon_cwd
        }
        Err(_) => true,
    }
}

/// A display path for the `cd` hint: the relative arg if it is one, else absolute.
fn display_cd(target: &Path) -> String {
    if target.is_relative() {
        target.display().to_string()
    } else {
        // Prefer a path relative to CWD when possible for a tidy hint.
        match std::env::current_dir() {
            Ok(cwd) => target
                .strip_prefix(&cwd)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| target.display().to_string()),
            Err(_) => target.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ui() -> Ui {
        Ui::resolve(
            crate::ui::ColorChoice::Never,
            false,
            true,
            false,
            false,
            false,
        )
    }

    #[test]
    fn scaffolds_static_site_with_no_dig_workspace() {
        let td = TempDir::new().unwrap();
        let dir = td.path().join("site");
        let t = templates::find("static-site").unwrap();
        write_template(&ui(), t, &dir, false).unwrap();

        // The starter files exist...
        assert!(dir.join("dig.toml").exists());
        assert!(dir.join("index.html").exists());
        // ...and NOTHING on-chain/local-workspace was created (no mint, no .dig).
        assert!(
            !dir.join(".dig").exists(),
            "new must not create a workspace"
        );
    }

    #[test]
    fn refuses_nonempty_dir_without_force() {
        let td = TempDir::new().unwrap();
        std::fs::write(td.path().join("keep.txt"), b"hi").unwrap();
        let t = templates::find("static-site").unwrap();
        let err = write_template(&ui(), t, td.path(), false).unwrap_err();
        assert!(matches!(err, CliError::InvalidArgument(ref m) if m.contains("not empty")));
        // The pre-existing file is untouched.
        assert!(td.path().join("keep.txt").exists());
    }

    #[test]
    fn force_writes_into_nonempty_dir() {
        let td = TempDir::new().unwrap();
        std::fs::write(td.path().join("keep.txt"), b"hi").unwrap();
        let t = templates::find("static-site").unwrap();
        write_template(&ui(), t, td.path(), true).unwrap();
        assert!(td.path().join("index.html").exists());
        assert!(
            td.path().join("keep.txt").exists(),
            "force keeps other files"
        );
    }

    #[test]
    fn nested_template_paths_are_created() {
        let td = TempDir::new().unwrap();
        let t = templates::find("vite-react").unwrap();
        write_template(&ui(), t, td.path(), true).unwrap();
        assert!(td.path().join("src").join("App.jsx").exists());
    }
}
