use std::path::Path;

use digstore_core::MAX_STORE_BYTES;
use digstore_store::{RootHistory, StagingArea};

use crate::context::CliContext;
use crate::error::CliError;
use crate::ui::Ui;
use crate::workspace::Workspace;

pub fn run(
    _ctx: &CliContext,
    ui: &Ui,
    ws: &Workspace,
    _args: crate::cli::StoresArgs,
) -> Result<(), CliError> {
    #[derive(serde::Serialize)]
    struct Row<'a> {
        name: &'a str,
        store_id: &'a str,
        /// On-chain project name (singleton metadata label); None until set.
        label: Option<String>,
        description: Option<String>,
        active: bool,
        content_root: Option<String>,
        current_root: Option<String>,
        staged_bytes: u64,
        limit_bytes: u64,
    }
    let mut rows = Vec::new();
    for (name, entry) in &ws.stores {
        let store_dir = ws.store_dir(name);
        let staged_bytes = staged_total_for_dir(&store_dir, &entry.id);
        let current_root = current_root_for_dir(&store_dir);
        let (label, description) = label_and_description_for_dir(&store_dir);
        rows.push(Row {
            name,
            store_id: &entry.id,
            label,
            description,
            active: ws.active.as_deref() == Some(name.as_str()),
            content_root: entry.content_root.clone(),
            current_root,
            staged_bytes,
            limit_bytes: limit_for_dir(&store_dir),
        });
    }
    if ui.json() {
        ui.emit_json(&rows);
        return Ok(());
    }
    if rows.is_empty() {
        ui.line("no stores; create one with `digstore init`");
        return Ok(());
    }
    for r in &rows {
        let star = if r.active { "*" } else { " " };
        let root = r
            .current_root
            .as_deref()
            .map(|h| &h[..h.len().min(12)])
            .unwrap_or("(empty)");
        let cr = r.content_root.clone().unwrap_or_else(|| ".".into());
        // Display name = the on-chain project name (label) when set, else the store id prefix.
        let display_name = match r.label.as_deref() {
            Some(l) if !l.is_empty() => l.to_string(),
            _ => format!("{}…", &r.store_id[..r.store_id.len().min(8)]),
        };
        ui.line(format!(
            "{star} {:<20} [{}]  root {}  dir {}  {}",
            display_name,
            r.name,
            root,
            cr,
            crate::ui::human_capacity(r.staged_bytes, r.limit_bytes),
        ));
    }
    Ok(())
}

/// Total staged bytes for the store rooted at `store_dir` whose id is `id_hex`.
/// Best-effort: returns 0 if the staging file is absent or unreadable.
fn staged_total_for_dir(store_dir: &Path, id_hex: &str) -> u64 {
    let staging_path = store_dir.join(format!("{id_hex}.staging.bin"));
    if !staging_path.exists() {
        return 0;
    }
    match StagingArea::open(&staging_path).and_then(|s| s.records()) {
        Ok(records) => records.iter().map(|r| r.content.len() as u64).sum(),
        Err(_) => 0,
    }
}

/// The per-store stage cap (`StoreConfig.max_size`) for the store rooted at
/// `store_dir`, read from its persisted `config.toml` so legacy/migrated/cloned
/// stores show their real limit. Falls back to the `MAX_STORE_BYTES` default if
/// the config cannot be loaded or records an unset (`0`) cap.
fn limit_for_dir(store_dir: &Path) -> u64 {
    match digstore_store::load_config(store_dir.join("config.toml")) {
        Ok(cfg) if cfg.max_size != 0 => cfg.max_size,
        _ => MAX_STORE_BYTES,
    }
}

/// The on-chain project name (label) + description for the store rooted at
/// `store_dir`, read from its `config.toml`. Best-effort; `(None, None)` if the
/// config is missing/unreadable or the fields are unset.
fn label_and_description_for_dir(store_dir: &Path) -> (Option<String>, Option<String>) {
    match digstore_store::load_config(store_dir.join("config.toml")) {
        Ok(cfg) => (cfg.label, cfg.description),
        Err(_) => (None, None),
    }
}

/// The current generation root (hex) for the store rooted at `store_dir`, or
/// `None` if the store has no committed generation. Best-effort.
fn current_root_for_dir(store_dir: &Path) -> Option<String> {
    let history_path = store_dir.join("roots.log");
    if !history_path.exists() {
        return None;
    }
    RootHistory::open(&history_path)
        .and_then(|h| h.head())
        .ok()
        .flatten()
        .map(|g| g.root.to_hex())
}
