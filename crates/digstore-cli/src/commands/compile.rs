//! `digstore compile` — headless build of a hostable module from a directory.
//!
//! This is the chainless half of `commit`: it stages a content directory, computes
//! the generation merkle root, and compiles the `.dig` module — with NO wallet, NO
//! chain, NO signing. The caller (e.g. the dighub compile worker) anchors the printed
//! `root` on-chain separately (via a wallet/Sage). It reuses the exact local pipeline
//! `commit` uses (`stage_to_root` + `finalize_commit(None)`), so the root and module
//! are byte-identical to what an on-chain commit of the same files would produce.

use digstore_core::config::SecretSalt;
use digstore_core::{Author, Bytes32, MetadataManifest};
use std::collections::BTreeMap;

use crate::cli::CompileArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::ui::Ui;

/// Build a [`MetadataManifest`] from the dighub `Manifest` JSON shape (14 publisher fields).
/// Tolerant: missing/empty fields collapse to `None`/empty; unknown keys are ignored except
/// `custom`, which is preserved verbatim. This is the inverse of the retrieval Lambda's
/// `manifest_to_json`, so a round-trip (.dig -> RPC JSON -> recompile) is stable.
fn manifest_from_json(v: &serde_json::Value) -> MetadataManifest {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
    let opt = |k: &str| s(k).filter(|t| !t.is_empty());
    let arr_str = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(|t| t.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let authors = v
        .get("authors")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let name = e.get("name").and_then(|n| n.as_str())?.to_string();
                    Some(Author {
                        name,
                        handle: e
                            .get("handle")
                            .and_then(|h| h.as_str())
                            .map(|t| t.to_string()),
                        contact: e
                            .get("contact")
                            .and_then(|h| h.as_str())
                            .map(|t| t.to_string()),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let links = v
        .get("links")
        .and_then(|x| x.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, val)| val.as_str().map(|t| (k.clone(), t.to_string())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let custom = v
        .get("custom")
        .and_then(|x| x.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, val)| (k.clone(), val.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    MetadataManifest {
        schema_version: v
            .get("schema_version")
            .and_then(|x| x.as_u64())
            .unwrap_or(1) as u32,
        name: s("name").unwrap_or_default(),
        version: opt("version"),
        description: opt("description"),
        authors,
        license: opt("license"),
        homepage: opt("homepage"),
        repository: opt("repository"),
        keywords: arr_str("keywords"),
        categories: arr_str("categories"),
        icon: opt("icon"),
        content_type: opt("content_type"),
        links,
        custom,
    }
}

pub fn run(ctx: &CliContext, ui: &Ui, args: CompileArgs) -> Result<(), CliError> {
    // 1. The on-chain store id (launcher id) this generation belongs to. It is curried
    //    into the compiled module so clients can verify the served root against the
    //    singleton. Minting happened earlier (wallet-side); compile never touches chain.
    let store_id = Bytes32::from_hex(args.store_id.trim_start_matches("0x"))
        .map_err(|e| CliError::InvalidArgument(format!("--store-id is not 32-byte hex: {e}")))?;

    // 2. Optional deterministic salt for a private store. With --salt the root is
    //    reproducible; --private without --salt would use fresh randomness (a re-compile
    //    would not match), so a private store should always pass its stored salt.
    let salt_override = match &args.salt {
        Some(hex) => {
            let b = Bytes32::from_hex(hex.trim_start_matches("0x")).map_err(|e| {
                CliError::InvalidArgument(format!("--salt is not 32-byte hex: {e}"))
            })?;
            Some(SecretSalt(b.0))
        }
        None => None,
    };
    let private = args.private || salt_override.is_some();

    if !args.r#in.is_dir() {
        return Err(CliError::InvalidArgument(format!(
            "--in is not a directory: {}",
            args.r#in.display()
        )));
    }

    // 2b. Optional serving-host public key. When set, the compiled module's trusted host-key set
    //     is this key (Digstore §12.2), so the delegated serving node (e.g. the dighub retrieval
    //     host) can attest and `serve_blind` releases real content instead of decoys. Embedded
    //     NATIVELY at init (not a post-hoc re-key), so the 135 MB chunk pool is written once.
    let host_key_override = match &args.host_key {
        Some(hk) => Some(
            digstore_core::Bytes48::from_hex(hk.trim_start_matches("0x")).map_err(|e| {
                CliError::InvalidArgument(format!("--host-key is not 48-byte hex: {e}"))
            })?,
        ),
        None => None,
    };

    // 3. Scaffold the ephemeral store for THIS store id — config + staging + roots.log
    //    under the (temp) dig dir. NO mint, NO chain. data_dir defaults to the dig dir;
    //    the content walk is driven by ctx.op_dir (== --in), set by the dispatcher.
    store_ops::init_store(
        ctx,
        private,
        None,
        Some(store_id),
        salt_override,
        host_key_override,
        // Headless compile: no chain, no on-chain metadata.
        None,
        None,
    )?;

    // 4. Stage every file under the content root.
    let staged = store_ops::add_files(ctx, &[], true, false, None)?;
    if staged.staged.is_empty() {
        return Err(CliError::InvalidArgument(format!(
            "no files to compile under {}",
            args.r#in.display()
        )));
    }

    // 5. Load the metadata manifest to embed in the module (the dighub `Manifest` JSON), or an
    //    empty manifest when --metadata is absent. It is served ungated via `get_metadata` and is
    //    bound to the module's program_hash, so a reader can verify it against the on-chain anchor.
    let metadata = match &args.metadata {
        Some(path) => {
            let raw = std::fs::read_to_string(path).map_err(|e| {
                CliError::InvalidArgument(format!("--metadata read {}: {e}", path.display()))
            })?;
            let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                CliError::InvalidArgument(format!("--metadata is not valid JSON: {e}"))
            })?;
            manifest_from_json(&v)
        }
        None => crate::ops::serve::empty_manifest(),
    };

    // 6. Compute the root and compile the module locally — NO chain pointer (None).
    //    finalize_commit writes the module to the store's modules/ dir; we copy it out.
    //    --pre-encrypted: inputs are already sealed client-side (server stays blind to plaintext).
    let outcome = if args.pre_encrypted {
        store_ops::commit_pre_encrypted(ctx, metadata)?
    } else {
        store_ops::commit(ctx, None, metadata)?
    };

    // 7. Place the module at --out and hash it (program_hash = SHA-256 of the module
    //    bytes — the "size proof" the singleton metadata can carry).
    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| CliError::Other(e.into()))?;
        }
    }
    std::fs::copy(&outcome.output_path, &args.out).map_err(|e| {
        CliError::Other(anyhow::anyhow!(
            "copy module {} -> {}: {e}",
            outcome.output_path.display(),
            args.out.display()
        ))
    })?;
    let module_bytes = std::fs::read(&args.out).map_err(|e| CliError::Other(e.into()))?;
    let program_hash = digstore_crypto::sha256(&module_bytes);

    // 8. Emit the build result. JSON is the contract the worker parses.
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "root": outcome.roothash.to_hex(),
            "program_hash": program_hash.to_hex(),
            "size": module_bytes.len(),
            "module": args.out.display().to_string(),
            "store_id": store_id.to_hex(),
            "files": staged.staged.len(),
        }));
    } else {
        ui.success(format!("compiled root {}", outcome.roothash.to_hex()));
        ui.line(format!(
            "  module: {} ({} bytes)",
            args.out.display(),
            outcome.output_size
        ));
        ui.line(format!("  program hash: {}", program_hash.to_hex()));
        ui.line(format!("  files: {}", staged.staged.len()));
    }
    Ok(())
}
