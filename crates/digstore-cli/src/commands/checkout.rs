use std::fs;

use digstore_core::{Bytes32, Urn};

use crate::cli::CheckoutArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::{client_crypto, serve, store_ops};

pub fn run(ctx: &CliContext, args: CheckoutArgs) -> Result<(), CliError> {
    let root = Bytes32::from_hex(&args.root)
        .map_err(|_| CliError::InvalidArgument("root must be 32-byte hex".into()))?;
    let store_id = ctx.find_store_id()?;
    let module_path = store_ops::module_path_for(ctx, &store_id, Some(root))?;

    let salt: Option<[u8; 32]> = match &args.salt {
        Some(hex) => Some(
            Bytes32::from_hex(hex)
                .map_err(|_| CliError::InvalidArgument("salt must be 32-byte hex".into()))?
                .0,
        ),
        None => None,
    };

    fs::create_dir_all(&args.out).map_err(|e| CliError::Other(e.into()))?;
    let keys = store_ops::list_generation_resources(ctx, &root)?;
    let mut count = 0usize;
    for key in keys {
        let urn = Urn {
            chain: "chia".into(),
            store_id,
            root_hash: Some(root),
            resource_key: Some(key.clone()),
        };
        let resp = serve::serve_content(ctx, &module_path, &urn, root)?;
        let chunk_lens =
            store_ops::resource_chunk_lens(ctx, &root, &key).unwrap_or_default();
        let plaintext =
            client_crypto::decrypt_and_verify(&resp, &urn, salt.as_ref(), &root, &chunk_lens)?;
        let dest = args.out.join(&key);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| CliError::Other(e.into()))?;
        }
        fs::write(&dest, &plaintext).map_err(|e| CliError::Other(e.into()))?;
        count += 1;
    }
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({ "root": root.to_hex(), "files": count })
        );
    } else {
        println!(
            "checked out {} files from {} into {}",
            count,
            root.to_hex(),
            args.out.display()
        );
    }
    Ok(())
}
