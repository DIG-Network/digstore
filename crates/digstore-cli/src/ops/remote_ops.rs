//! Remote operations: clone, push, pull over the `digstore-remote` `DigClient`.
//!
//! Error mapping matches the TYPED `ClientError` enum, never Display strings
//! (CONVENTIONS C7 push-signing delegated to `digstore_crypto`).

use std::fs;

use digstore_core::{Bytes32, Bytes96, GenerationState, StoreConfig, Visibility};
use digstore_remote::{ClientError, DigClient, PullResult, PushResult};

use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

#[derive(Debug)]
pub struct CloneSummary {
    pub store_id_hex: String,
    pub root_hex: String,
    pub module_size: u64,
}

/// Map the TYPED remote client error enum to a CliError.
pub(crate) fn map_remote_err(e: ClientError) -> CliError {
    match e {
        ClientError::NonFastForward => CliError::NonFastForward,
        ClientError::Unauthorized(_) => {
            CliError::Unauthorized("remote rejected credentials".into())
        }
        ClientError::Status(404) => CliError::NotFound("remote resource".into()),
        ClientError::Status(code) => CliError::Network(format!("remote status {code}")),
        ClientError::Transport(msg) => CliError::Network(msg),
        ClientError::Verification(msg) => CliError::VerificationFailed(msg),
        ClientError::Decode(msg) => CliError::Network(format!("decode: {msg}")),
    }
}

/// The canonical push-auth message (CONVENTIONS C7): SHA-256(root || store_id).
pub(crate) fn push_auth_message(root: &Bytes32, store_id: &Bytes32) -> [u8; 32] {
    digstore_crypto::push_signing_message(root, store_id)
}

/// Parse a `urn:dig:` or raw `…/stores/{id}` URL into (base_url, store_id_hex).
fn parse_store_url(url: &str) -> Result<(String, String), CliError> {
    // Accept `http(s)://host/stores/{hex}`.
    if let Some(idx) = url.find("/stores/") {
        let base = url[..idx].to_string();
        let id = url[idx + "/stores/".len()..]
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        if Bytes32::from_hex(&id).is_ok() {
            return Ok((base, id));
        }
    }
    Err(CliError::InvalidArgument(format!(
        "expected a store URL like http://host/stores/<store_id_hex>, got {url}"
    )))
}

pub async fn clone_from(ctx: &CliContext, store_url: &str) -> Result<CloneSummary, CliError> {
    if ctx.config_path().exists() {
        return Err(CliError::InvalidArgument(
            "dig dir already has a store; clone into an empty dir".into(),
        ));
    }
    let (base, store_id_hex) = parse_store_url(store_url)?;
    let store_id = Bytes32::from_hex(&store_id_hex)
        .map_err(|_| CliError::InvalidArgument("bad store id hex".into()))?;
    let client = DigClient::new(base);

    // Descriptor + roots.
    let info = client.fetch(&store_id).await.map_err(map_remote_err)?;
    let remote_root = Bytes32::from_hex(&info.descriptor.current_root)
        .map_err(|_| CliError::VerificationFailed("bad descriptor root".into()))?;

    // Download + verify (clone_store checks the ETag=root agrees with the body).
    let (etag_root, module) = client
        .clone_store(&store_id, |_bytes, served_root| {
            if *served_root == remote_root {
                Ok(())
            } else {
                Err("module ETag root != descriptor root".to_string())
            }
        })
        .await
        .map_err(map_remote_err)?;
    if etag_root != remote_root {
        return Err(CliError::VerificationFailed(
            "descriptor root and module ETag disagree".into(),
        ));
    }

    // Real generation id/timestamp from /roots.
    let gen = info
        .roots
        .roots
        .iter()
        .find(|r| Bytes32::from_hex(&r.root).ok() == Some(remote_root))
        .ok_or_else(|| CliError::VerificationFailed("root not present in remote /roots".into()))?;
    let timestamp = gen.timestamp;

    // Install the cloned layout.
    fs::create_dir_all(&ctx.dig_dir).map_err(|e| CliError::Other(e.into()))?;
    fs::create_dir_all(ctx.modules_dir()).map_err(|e| CliError::Other(e.into()))?;
    fs::create_dir_all(ctx.generations_dir()).map_err(|e| CliError::Other(e.into()))?;
    let cfg = StoreConfig {
        store_id,
        data_dir: ctx.dig_dir.display().to_string(),
        max_size: 1024 * 1024 * 1024,
        visibility: Visibility::Public,
    };
    digstore_store::save_config(ctx.config_path(), &cfg)
        .map_err(|e| CliError::Other(anyhow::anyhow!("save config: {e}")))?;

    let module_path = ctx.modules_dir().join(format!(
        "{}-{}.wasm",
        store_id.to_hex(),
        remote_root.to_hex()
    ));
    fs::write(&module_path, &module).map_err(|e| CliError::Other(e.into()))?;

    store_ops::append_history(
        ctx,
        GenerationState {
            id: gen.generation,
            root: remote_root,
            timestamp,
        },
    )?;

    Ok(CloneSummary {
        store_id_hex: store_id.to_hex(),
        root_hex: remote_root.to_hex(),
        module_size: module.len() as u64,
    })
}

pub async fn push_to(ctx: &CliContext, store_url: &str) -> Result<Bytes32, CliError> {
    let cfg = ctx.load_config()?;
    let root = store_ops::current_root(ctx)?
        .ok_or_else(|| CliError::NotFound("no committed root to push".into()))?;
    let module_path = store_ops::module_path_for(ctx, &cfg.store_id, Some(root))?;
    let module = fs::read(&module_path).map_err(|e| CliError::Other(e.into()))?;

    let (base, _id) = parse_store_url(store_url)?;
    let client = DigClient::new(base);

    // Parent = the remote's current served root (genesis if fresh).
    let info = client.fetch(&cfg.store_id).await.map_err(map_remote_err)?;
    let parent = Bytes32::from_hex(&info.descriptor.current_root)
        .map_err(|_| CliError::VerificationFailed("bad descriptor root".into()))?;

    let sk = store_ops::load_signing_key(ctx)?;
    let store_id = cfg.store_id;
    let result = client
        .push(
            &store_id,
            &parent,
            &root,
            &module,
            false,
            None,
            |msg: &[u8; 32]| -> Bytes96 {
                // The client computes msg = SHA-256(root || store_id); sign it.
                debug_assert_eq!(*msg, push_auth_message(&root, &store_id));
                digstore_crypto::bls::bls_sign(&sk, msg)
            },
        )
        .await
        .map_err(map_remote_err)?;
    match result {
        PushResult::Advanced | PushResult::Pending => Ok(root),
    }
}

pub async fn pull_from(ctx: &CliContext, store_url: &str) -> Result<Bytes32, CliError> {
    let cfg = ctx.load_config()?;
    let (base, _id) = parse_store_url(store_url)?;
    let client = DigClient::new(base);

    let local_root = store_ops::current_root(ctx)?;
    let result = client
        .pull(&cfg.store_id, local_root, false)
        .await
        .map_err(map_remote_err)?;
    match result {
        PullResult::UpToDate => Ok(local_root.unwrap_or(Bytes32([0u8; 32]))),
        PullResult::Module { root, bytes } => {
            // Real generation id/timestamp from /roots.
            let info = client.fetch(&cfg.store_id).await.map_err(map_remote_err)?;
            let gen = info
                .roots
                .roots
                .iter()
                .find(|r| Bytes32::from_hex(&r.root).ok() == Some(root))
                .ok_or_else(|| CliError::VerificationFailed("root not in remote /roots".into()))?;
            let module_path =
                ctx.modules_dir()
                    .join(format!("{}-{}.wasm", cfg.store_id.to_hex(), root.to_hex()));
            fs::write(&module_path, &bytes).map_err(|e| CliError::Other(e.into()))?;
            store_ops::append_history(
                ctx,
                GenerationState {
                    id: gen.generation,
                    root,
                    timestamp: gen.timestamp,
                },
            )?;
            Ok(root)
        }
        PullResult::Delta { root, .. } => Ok(root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_auth_message_is_sha256_of_root_concat_store_id() {
        let root = Bytes32([2u8; 32]);
        let sid = Bytes32([1u8; 32]);
        // Canonical message is SHA-256(root || store_id) (C7 argument order).
        assert_eq!(
            push_auth_message(&root, &sid),
            digstore_crypto::push_signing_message(&root, &sid)
        );
    }

    #[test]
    fn parse_store_url_extracts_base_and_id() {
        let id = "ab".repeat(32);
        let (base, got) = parse_store_url(&format!("http://127.0.0.1:9000/stores/{id}")).unwrap();
        assert_eq!(base, "http://127.0.0.1:9000");
        assert_eq!(got, id);
    }
}
