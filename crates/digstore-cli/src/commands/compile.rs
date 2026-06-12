//! `digstore compile` — headless build of a hostable module from a directory.
//!
//! This is the chainless half of `commit`: it stages a content directory, computes
//! the generation merkle root, and compiles the `.dig` module — with NO wallet, NO
//! chain, NO signing. The caller (e.g. the dighub compile worker) anchors the printed
//! `root` on-chain separately (via a wallet/Sage). It reuses the exact local pipeline
//! `commit` uses (`stage_to_root` + `finalize_commit(None)`), so the root and module
//! are byte-identical to what an on-chain commit of the same files would produce.

use digstore_core::config::SecretSalt;
use digstore_core::Bytes32;

use crate::cli::CompileArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;
use crate::ui::Ui;

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
            let b = Bytes32::from_hex(hex.trim_start_matches("0x"))
                .map_err(|e| CliError::InvalidArgument(format!("--salt is not 32-byte hex: {e}")))?;
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

    // 3. Scaffold the ephemeral store for THIS store id — config + staging + roots.log
    //    under the (temp) dig dir. NO mint, NO chain. data_dir defaults to the dig dir;
    //    the content walk is driven by ctx.op_dir (== --in), set by the dispatcher.
    store_ops::init_store(ctx, private, None, Some(store_id), salt_override)?;

    // 4. Stage every file under the content root.
    let staged = store_ops::add_files(ctx, &[], true, false, None)?;
    if staged.staged.is_empty() {
        return Err(CliError::InvalidArgument(format!(
            "no files to compile under {}",
            args.r#in.display()
        )));
    }

    // 5. Compute the root and compile the module locally — NO chain pointer (None).
    //    finalize_commit writes the module to the store's modules/ dir; we copy it out.
    let outcome = store_ops::commit(ctx, None)?;

    // 6. Place the module at --out and hash it (program_hash = SHA-256 of the module
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

    // 7. Emit the build result. JSON is the contract the worker parses.
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "root": outcome.roothash.to_hex(),
            "program_hash": program_hash.to_hex(),
            "size": outcome.output_size,
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
