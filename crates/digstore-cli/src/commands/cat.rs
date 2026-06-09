use std::io::Write;

use digstore_core::{Bytes32, Urn};

use crate::cli::CatArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::{client_crypto, serve, store_ops};

pub fn run(ctx: &CliContext, args: CatArgs) -> Result<(), CliError> {
    let urn =
        Urn::parse(&args.urn).map_err(|e| CliError::InvalidArgument(format!("bad urn: {e}")))?;

    // Trusted root: prefer the URN's root, else the current local root.
    let trusted_root: Bytes32 = match urn.root_hash {
        Some(r) => r,
        None => store_ops::current_root(ctx)?
            .ok_or_else(|| CliError::NotFound("no committed root".into()))?,
    };

    let module_path = store_ops::module_path_for(ctx, &urn.store_id, Some(trusted_root))?;

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
    let chunk_lens = store_ops::resource_chunk_lens(ctx, &trusted_root, &resource_key)
        .unwrap_or_default();

    let plaintext =
        client_crypto::decrypt_and_verify(&resp, &urn, salt.as_ref(), &trusted_root, &chunk_lens)?;

    std::io::stdout()
        .write_all(&plaintext)
        .map_err(|e| CliError::Other(e.into()))?;
    Ok(())
}
