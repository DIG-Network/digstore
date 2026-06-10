use std::io::Write;
use std::path::Path;

use digstore_core::{Bytes32, Urn};

use crate::cli::CatArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::{client_crypto, serve, store_ops};

pub fn run(ctx: &CliContext, _ui: &crate::ui::Ui, args: CatArgs) -> Result<(), CliError> {
    let target = args.urn.trim();

    // Two retrieval modes:
    //   * URN (`urn:dig:…`)        → fetch + DECRYPT, so the streamed-out bytes
    //                                 are the final plaintext.
    //   * 64-char hex retrieval key → fetch the RAW ENCRYPTED bytes within the
    //                                 active store; no decryption is performed.
    let bytes = if target.starts_with("urn:") {
        cat_by_urn(ctx, &args, target)?
    } else if let Ok(rk) = Bytes32::from_hex(target) {
        cat_by_retrieval_key(ctx, rk)?
    } else {
        return Err(CliError::InvalidArgument(
            "expected a 'urn:dig:…' URN or a 64-character hex retrieval key".into(),
        ));
    };

    write_out(args.out.as_deref(), &bytes)
}

/// URN path: resolve, serve, decrypt, return plaintext.
fn cat_by_urn(ctx: &CliContext, args: &CatArgs, target: &str) -> Result<Vec<u8>, CliError> {
    let urn = Urn::parse(target).map_err(|e| CliError::InvalidArgument(format!("bad urn: {e}")))?;

    // Trusted root: prefer the URN's root, else the current local root.
    let trusted_root: Bytes32 = match urn.root_hash {
        Some(r) => r,
        None => store_ops::current_root(ctx)?
            .ok_or_else(|| CliError::NotFound("no committed root".into()))?,
    };

    let module_path = store_ops::module_path_for(ctx, &urn.store_id, Some(trusted_root))?;

    // §8.5 social conventions: a URN with no resource key resolves to the store's
    // landing resource `index.html` (its default view) when that key exists in the
    // generation manifest; otherwise it falls back to the store-level empty key.
    // Bind the resolution into an effective URN so EVERY downstream step — the
    // module's retrieval-key lookup, the per-chunk lengths, and the client
    // decryption key — derives from the same key (C9/C10).
    let urn = if urn.resource_key.is_none() {
        Urn {
            resource_key: Some(store_ops::resolve_resource_key(ctx, &trusted_root, &urn)),
            ..urn
        }
    } else {
        urn
    };

    let resp = serve::serve_content(ctx, &module_path, &urn, trusted_root)?;

    if args.verify_proof {
        let (proof, root) = serve::serve_proof(ctx, &module_path, &urn, trusted_root)?;
        if root != trusted_root {
            return Err(CliError::VerificationFailed("proof root mismatch".into()));
        }
        // program_hash is over the REAL guest module the compiler used as its
        // template (deviation #3 / D6).
        let expected = digstore_crypto::sha256(serve::embedded_guest_wasm());
        if proof.program_hash != expected {
            return Err(CliError::VerificationFailed("program hash mismatch".into()));
        }
    }

    let salt: Option<[u8; 32]> = match &args.salt {
        Some(hex) => Some(
            Bytes32::from_hex(hex)
                .map_err(|_| CliError::InvalidArgument("salt must be 32-byte hex".into()))?
                .0,
        ),
        None => None,
    };

    // Per-chunk ciphertext lengths (from the local generation manifest) let the
    // client split the module's plain-concatenated served ciphertext (D5/C9).
    let resource_key = urn.resource_key.clone().unwrap_or_default();
    let chunk_lens =
        store_ops::resource_chunk_lens(ctx, &trusted_root, &resource_key).unwrap_or_default();

    client_crypto::decrypt_and_verify(&resp, &urn, salt.as_ref(), &trusted_root, &chunk_lens)
}

/// Retrieval-key path: look the key up in the active store's current generation
/// and return the RAW ENCRYPTED bytes (the served ciphertext, undecrypted).
fn cat_by_retrieval_key(ctx: &CliContext, retrieval_key: Bytes32) -> Result<Vec<u8>, CliError> {
    let cfg = ctx.load_config()?;
    let store_id = cfg.store_id;
    let trusted_root = store_ops::current_root(ctx)?
        .ok_or_else(|| CliError::NotFound("no committed root".into()))?;

    let resource_key =
        store_ops::resource_key_for_retrieval_key(ctx, &trusted_root, &retrieval_key)?;

    // Serve with the rootless canonical URN (its retrieval key == the manifest's
    // static key), then hand back the ciphertext verbatim — no decryption.
    let urn = store_ops::canonical_resource_urn(store_id, &resource_key);
    let module_path = store_ops::module_path_for(ctx, &store_id, Some(trusted_root))?;
    let resp = serve::serve_content(ctx, &module_path, &urn, trusted_root)?;
    Ok(resp.ciphertext)
}

/// Stream bytes to `out` (a file) or stdout.
fn write_out(out: Option<&Path>, bytes: &[u8]) -> Result<(), CliError> {
    match out {
        Some(path) => std::fs::write(path, bytes)
            .map_err(|e| CliError::Other(anyhow::anyhow!("write {}: {e}", path.display()))),
        None => std::io::stdout()
            .write_all(bytes)
            .map_err(|e| CliError::Other(e.into())),
    }
}
