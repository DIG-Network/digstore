//! `digstore manifest` — the store's complete public file surface.
//!
//! Prints the normalized public manifest: every public file PATH with its LATEST
//! version's capsule (root) + version index, that version's content SHA-256, and
//! how many versions of the path exist across the store's whole history. Unlike
//! `keys` (which lists a single capsule's resources), this flattens the entire
//! history into one latest-per-path view — including files whose latest version
//! lives in an earlier capsule.
//!
//! `--json` emits the machine surface (the same shape embedded in the `.dig`
//! `PublicManifest` section and returned by the browser reader): `{ schema_version,
//! entries: [ { path, latest_root, generation_index, sha256_latest, version_count } ] }`.

use crate::cli::ManifestArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, _args: ManifestArgs) -> Result<(), CliError> {
    let manifest = store_ops::public_manifest(ctx)?;

    if ui.json() {
        // Serde-serialize the manifest (hashes as hex via `Bytes32`) so the CLI,
        // the embedded `.dig` section, and the browser reader agree on one shape:
        // `{ schema_version, entries: [ { path, latest_root, generation_index,
        // sha256_latest, version_count } ] }`.
        ui.emit_json(&manifest);
        return Ok(());
    }

    if manifest.entries.is_empty() {
        ui.line("(no published files yet; run `digstore commit` to publish a capsule)");
        return Ok(());
    }

    ui.line(format!(
        "{:<40}  {:>4}  {:<12}  {}",
        "PATH", "VERS", "LATEST GEN", "SHA-256 (latest)"
    ));
    for e in &manifest.entries {
        ui.line(format!(
            "{:<40}  {:>4}  gen {:<8}  {}",
            e.path,
            e.version_count,
            e.generation_index,
            e.sha256_latest.to_hex(),
        ));
    }
    Ok(())
}
