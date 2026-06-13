//! `digstore serve` — run a dig:// remote NODE for the active store.
//!
//! Serves the §21 remote protocol (clone / pull / push — the same protocol
//! rpc.dig.net speaks and that `DigClient` drives) over an `axum` server backed by
//! the store's real on-disk layout. Anyone with the digstore binary can host an
//! origin: `digstore serve --bind 0.0.0.0:8443`, then others clone it with
//! `digstore clone dig://<thisHost>:8443/<storeId>`.
//!
//! Every request must be authenticated by a signed message from the caller's
//! identity key (§21.9); `--anonymous` opts into a fully-public read mirror.

use std::sync::Arc;

use digstore_core::MAX_STORE_BYTES;
use digstore_remote::{RemoteServer, StoreBackend};

use crate::cli::ServeArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: ServeArgs) -> Result<(), CliError> {
    let store_id = ctx.find_store_id()?;

    // The node serves the store's current confirmed head. Read the module for that
    // root and recover the store's BLS publisher key from the module's embedded
    // identity — that key is what the descriptor advertises (and clients verify the
    // head signature against), so it must come from the module, not a local secret.
    let root = store_ops::current_root(ctx)?.ok_or_else(|| {
        CliError::NotFound("no committed root to serve; run `digstore commit` first".into())
    })?;
    let module_path = store_ops::module_path_for(ctx, &store_id, Some(root))?;
    let module = std::fs::read(&module_path).map_err(|e| CliError::Other(e.into()))?;
    let identity = digstore_compiler::verify_module_root(&module, &store_id)
        .map_err(|e| CliError::VerificationFailed(format!("module verify: {e:?}")))?;

    let backend = StoreBackend::open(
        ctx.dig_dir.display().to_string(),
        store_id,
        identity.public_key,
        MAX_STORE_BYTES,
    );
    let mut server = RemoteServer::new(Arc::new(backend));
    if args.anonymous {
        server = server.allow_anonymous();
    }

    let auth = if args.anonymous {
        "anonymous (public read mirror)"
    } else {
        "required (§21.9 signed requests)"
    };
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "serving": true,
            "bind": args.bind,
            "store_id": store_id.to_hex(),
            "root": root.to_hex(),
            "auth": if args.anonymous { "anonymous" } else { "required" },
        }));
    } else {
        ui.line(format!(
            "serving store {} at http://{} (clone via dig://{}/{})\nauth: {}\nPress Ctrl-C to stop.",
            store_id.to_hex(),
            args.bind,
            args.bind,
            store_id.to_hex(),
            auth,
        ));
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    rt.block_on(server.serve(&args.bind))
        .map_err(|e| CliError::Other(anyhow::anyhow!("server error: {e}")))?;
    Ok(())
}
