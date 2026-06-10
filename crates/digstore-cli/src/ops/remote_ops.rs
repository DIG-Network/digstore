//! Remote operations: clone, push, pull over the `digstore-remote` `DigClient`.
//!
//! Error mapping matches the TYPED `ClientError` enum, never Display strings
//! (CONVENTIONS C7 push-signing delegated to `digstore_crypto`).

use std::fs;

use digstore_core::tombstone::TombstoneScope;
use digstore_core::{
    Bytes32, Bytes48, Bytes96, Decode, GenerationState, StoreConfig, Tombstone, Visibility,
    MAX_STORE_BYTES,
};
use digstore_remote::wire::TombstoneEntry;
use digstore_remote::{verify_push_signature, ClientError, DigClient, PullResult, PushResult};

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

/// Verify the remote's served-head authorization: a publisher BLS signature over
/// `SHA-256(root || store_id)` under `pubkey` (§21.6). `pubkey` is the store key
/// embedded in the verified module, already proven by `verify_module_root` to
/// satisfy `SHA-256(pubkey) == store_id`, so a valid signature here proves the
/// served root was authorized by the store's private key — not merely that the
/// module is self-consistent. Fails closed on an absent signature: a missing sig
/// would otherwise let a malicious origin strip authorization (downgrade attack).
fn verify_head_signature(
    pubkey: &Bytes48,
    root: &Bytes32,
    store_id: &Bytes32,
    push_sig_hex: &str,
) -> Result<(), CliError> {
    if push_sig_hex.is_empty() {
        return Err(CliError::VerificationFailed(
            "remote served head carries no publisher signature (unauthenticated head)".into(),
        ));
    }
    let raw = hex::decode(push_sig_hex)
        .map_err(|_| CliError::VerificationFailed("malformed push signature hex".into()))?;
    let arr: [u8; 96] = raw
        .try_into()
        .map_err(|_| CliError::VerificationFailed("push signature must be 96 bytes".into()))?;
    if !verify_push_signature(pubkey, root, store_id, &Bytes96(arr)) {
        return Err(CliError::VerificationFailed(
            "served-root signature does not verify against the store key".into(),
        ));
    }
    Ok(())
}

/// Fail-closed revocation check (SECURITY.md residual #1 Layer 1). After fetching
/// the descriptor, verify each served tombstone's signature against the
/// store-id-bound module key (`pubkey`, already proven by `verify_module_root` to
/// hash to `store_id`), and REFUSE to install/advance to `served_root` if:
///   - any VALID `Store`-scoped tombstone is present (the whole store is revoked), or
///   - a VALID `Root`-scoped tombstone names exactly `served_root`.
///
/// An unsigned / wrong-key / malformed tombstone is IGNORED (it does not revoke):
/// a malicious origin cannot fabricate a revocation it has no key to sign, and a
/// stray bad entry must not deny service. Returns `Ok(())` when the served root
/// is not revoked.
fn check_not_revoked(
    pubkey: &Bytes48,
    served_root: &Bytes32,
    store_id: &Bytes32,
    tombstones: &[TombstoneEntry],
) -> Result<(), CliError> {
    let pk = match digstore_crypto::bls::PublicKey::from_bytes(pubkey) {
        Ok(p) => p,
        // No valid module key to verify against: ignore all tombstones (the head
        // signature check is the authoritative gate; a key that cannot even be
        // parsed cannot have signed a revocation).
        Err(_) => return Ok(()),
    };
    for entry in tombstones {
        // Decode the canonical record + signature; skip malformed entries.
        let record = match hex::decode(&entry.record) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let tombstone = match Tombstone::from_bytes(&record) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let sig = match hex::decode(&entry.signature)
            .ok()
            .and_then(|b| <[u8; 96]>::try_from(b).ok())
        {
            Some(arr) => Bytes96(arr),
            None => continue,
        };
        // A tombstone for a DIFFERENT store does not apply here.
        if tombstone.store_id != *store_id {
            continue;
        }
        // Unsigned / wrong-key tombstone is ignored (does not revoke).
        if !digstore_crypto::verify_tombstone(&pk, &tombstone, &sig) {
            continue;
        }
        match tombstone.scope {
            TombstoneScope::Store => {
                return Err(CliError::VerificationFailed(format!(
                    "store {} has been revoked by a signed tombstone (reason {})",
                    store_id.to_hex(),
                    tombstone.reason
                )));
            }
            TombstoneScope::Root(r) if r == *served_root => {
                return Err(CliError::VerificationFailed(format!(
                    "served root {} has been revoked by a signed tombstone (reason {})",
                    served_root.to_hex(),
                    tombstone.reason
                )));
            }
            TombstoneScope::Root(_) => {}
        }
    }
    Ok(())
}

/// True if `base` is an `http://` URL pointing at the loopback interface.
/// Plaintext transport is permitted ONLY to loopback (local dev/tests); every
/// other host must use TLS so store contents and push credentials are not sent
/// in the clear and cannot be substituted by a network MITM.
fn is_loopback_http(base: &str) -> bool {
    let rest = match base.strip_prefix("http://") {
        Some(r) => r,
        None => return false,
    };
    // host[:port] up to the first '/'.
    let authority = rest.split('/').next().unwrap_or("");
    let host = authority
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(authority);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// Parse a raw `<scheme>://host/stores/{id}` URL into (base_url, store_id_hex).
///
/// Enforces a transport policy: the scheme must be `https`, or `http` to a
/// loopback host (dev/test). Other schemes (`file:`, `ftp:`, `gopher:`, …) and
/// plaintext `http` to a non-loopback host are rejected to prevent SSRF-adjacent
/// scheme abuse and cleartext exposure of content and credentials.
fn parse_store_url(url: &str) -> Result<(String, String), CliError> {
    if let Some(idx) = url.find("/stores/") {
        let base = url[..idx].to_string();
        let id = url[idx + "/stores/".len()..]
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        if Bytes32::from_hex(&id).is_ok() {
            let scheme_ok = base.starts_with("https://") || is_loopback_http(&base);
            if !scheme_ok {
                return Err(CliError::InvalidArgument(format!(
                    "insecure or unsupported remote URL scheme in {base}: use https:// \
                     (http:// is allowed only for localhost)"
                )));
            }
            return Ok((base, id));
        }
    }
    Err(CliError::InvalidArgument(format!(
        "expected a store URL like https://host/stores/<store_id_hex>, got {url}"
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

    // Download + verify. The closure cryptographically validates the downloaded
    // module against the store identity the user asked for: the module's embedded
    // StoreId must equal `store_id`, SHA-256(embedded PublicKey) must equal the
    // store id (§20.1 self-certifying identity), and the merkle root recomputed
    // from the module's own content must equal both the embedded CurrentRoot and
    // the served root. A server returning an arbitrary/foreign/corrupted module
    // therefore fails closed instead of being installed and executed.
    let (etag_root, module) = client
        .clone_store(&store_id, |bytes, served_root| {
            let id = digstore_compiler::verify_module_root(bytes, &store_id)
                .map_err(|e| format!("module identity verification failed: {e:?}"))?;
            if id.root != *served_root {
                return Err(format!(
                    "module content root {} != served root {}",
                    id.root.to_hex(),
                    served_root.to_hex()
                ));
            }
            Ok(())
        })
        .await
        .map_err(map_remote_err)?;
    if etag_root != remote_root {
        return Err(CliError::VerificationFailed(
            "descriptor root and module ETag disagree".into(),
        ));
    }

    // Authenticated head (§21.6): the served root must carry the publisher's BLS
    // signature, verified against the store key embedded in the module (which
    // verify_module_root proved hashes to the store id). This is what upgrades
    // clone from "self-consistent module" to "publisher-authorized content", so a
    // malicious origin holding the public store key cannot serve a fabricated root.
    let identity = digstore_compiler::verify_module_root(&module, &store_id)
        .map_err(|e| CliError::VerificationFailed(format!("module verify: {e:?}")))?;
    verify_head_signature(
        &identity.public_key,
        &remote_root,
        &store_id,
        &info.descriptor.push_sig,
    )?;

    // Fail-closed revocation (§ residual #1 Layer 1): refuse to install a served
    // root that a signed tombstone retracts (or to clone a Store-revoked store),
    // verifying each tombstone against the same store-id-bound module key.
    check_not_revoked(
        &identity.public_key,
        &remote_root,
        &store_id,
        &info.descriptor.tombstones,
    )?;

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
        max_size: MAX_STORE_BYTES,
        visibility: Visibility::Public,
    };
    digstore_store::save_config(ctx.config_path(), &cfg)
        .map_err(|e| CliError::Other(anyhow::anyhow!("save config: {e}")))?;

    // §12.2: the downloaded module trusts the ORIGIN's host key, which this clone
    // does not possess. To serve the module locally (the clone's `dig cat` drives
    // `HostRuntime::serve_content`, attesting with the local host key), generate a
    // local host signing key and RE-KEY the module to trust it. Only the
    // TrustedKeys section changes; chunks, key table, merkle nodes, and the
    // current root are preserved, so served content and proofs are byte-identical.
    let (local_seed, local_pubkey) = store_ops::generate_host_key();
    let module = digstore_compiler::rekey_module_trusted(
        &module,
        &[digstore_core::TrustedHostKey {
            public_key: local_pubkey.0,
            label: format!("dig-host-key-v1:{}", local_pubkey.to_hex()),
        }],
    )
    .map_err(|e| CliError::Other(anyhow::anyhow!("re-key cloned module: {e:?}")))?;
    store_ops::persist_host_identity(ctx, &local_seed, local_pubkey)?;

    let module_path = ctx.modules_dir().join(format!(
        "{}-{}.dig",
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

/// The store's BLS public key as embedded in (and proven self-certifying by) the
/// LOCAL module for `root`, if that module is present on disk. Used to verify
/// served tombstones on a pull/up-to-date path where no fresh module is
/// downloaded. Returns `None` (caller falls back to the served-module key) if the
/// local module is absent or fails verification.
fn local_module_pubkey(ctx: &CliContext, store_id: &Bytes32, root: &Bytes32) -> Option<Bytes48> {
    let path = store_ops::module_path_for(ctx, store_id, Some(*root)).ok()?;
    let bytes = fs::read(&path).ok()?;
    let id = digstore_compiler::verify_module_root(&bytes, store_id).ok()?;
    Some(id.public_key)
}

/// The result of a `revoke`: the scope that was revoked.
#[derive(Debug)]
pub enum RevokeScope {
    Root(Bytes32),
    Store,
}

/// Build, sign, and publish a revocation tombstone (SECURITY.md residual #1
/// Layer 1). `root` is `Some` for a single-root revocation or `None` for a
/// whole-store revocation. The tombstone is signed with the store's own signing
/// key (`signing_key.bin`) and POSTed to the configured remote, which re-verifies
/// the signature against the store's published key before persisting it.
pub async fn revoke_to(
    ctx: &CliContext,
    store_url: &str,
    root: Option<Bytes32>,
    reason: digstore_core::RevocationReason,
) -> Result<RevokeScope, CliError> {
    let cfg = ctx.load_config()?;
    let (base, _id) = parse_store_url(store_url)?;
    let client = DigClient::new(base);

    let not_after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let tombstone = match root {
        Some(r) => Tombstone::root(cfg.store_id, r, not_after, reason),
        None => Tombstone::store(cfg.store_id, not_after, reason),
    };

    let sk = store_ops::load_signing_key(ctx)?;
    client
        .post_tombstone(&cfg.store_id, &tombstone, |msg: &[u8; 32]| -> Bytes96 {
            debug_assert_eq!(*msg, digstore_crypto::tombstone_signing_message(&tombstone));
            digstore_crypto::bls::bls_sign(&sk, msg)
        })
        .await
        .map_err(map_remote_err)?;

    Ok(match root {
        Some(r) => RevokeScope::Root(r),
        None => RevokeScope::Store,
    })
}

pub async fn pull_from(ctx: &CliContext, store_url: &str) -> Result<Bytes32, CliError> {
    let cfg = ctx.load_config()?;
    let (base, _id) = parse_store_url(store_url)?;
    let client = DigClient::new(base);

    let local_root = store_ops::current_root(ctx)?;

    // Fail-closed revocation gate up front (SECURITY.md residual #1 Layer 1):
    // fetch the descriptor and, using the LOCAL module's store-id-bound key,
    // refuse a Store-revoked store (or a remote-root-revoked head) BEFORE any
    // advance — including the up-to-date case where no module is downloaded. The
    // Module branch below re-checks against the freshly downloaded module key too.
    let pre = client.fetch(&cfg.store_id).await.map_err(map_remote_err)?;
    if !pre.descriptor.tombstones.is_empty() {
        if let Some(local_root) = local_root {
            if let Some(pubkey) = local_module_pubkey(ctx, &cfg.store_id, &local_root) {
                let remote_root = Bytes32::from_hex(&pre.descriptor.current_root)
                    .map_err(|_| CliError::VerificationFailed("bad descriptor root".into()))?;
                // Check both the remote head we might advance to and our own local
                // root (a Store tombstone refuses the store regardless of root).
                check_not_revoked(
                    &pubkey,
                    &remote_root,
                    &cfg.store_id,
                    &pre.descriptor.tombstones,
                )?;
                check_not_revoked(
                    &pubkey,
                    &local_root,
                    &cfg.store_id,
                    &pre.descriptor.tombstones,
                )?;
            }
        }
    }

    let result = client
        .pull(&cfg.store_id, local_root, false)
        .await
        .map_err(map_remote_err)?;
    match result {
        PullResult::UpToDate => Ok(local_root.unwrap_or(Bytes32([0u8; 32]))),
        PullResult::Module { root, bytes } => {
            // Verify the downloaded module before persisting/serving it: embedded
            // StoreId must match, SHA-256(PublicKey)==StoreId, and the recomputed
            // content root must equal the claimed root (same gate as `clone`).
            let id = digstore_compiler::verify_module_root(&bytes, &cfg.store_id)
                .map_err(|e| CliError::VerificationFailed(format!("module verify: {e:?}")))?;
            if id.root != root {
                return Err(CliError::VerificationFailed(format!(
                    "pulled module content root {} != claimed root {}",
                    id.root.to_hex(),
                    root.to_hex()
                )));
            }
            // Real generation id/timestamp from /roots.
            let info = client.fetch(&cfg.store_id).await.map_err(map_remote_err)?;
            // Authenticated head (§21.6): require the publisher signature over the
            // pulled root, verified against the store-id-bound module key.
            verify_head_signature(
                &id.public_key,
                &root,
                &cfg.store_id,
                &info.descriptor.push_sig,
            )?;
            // Fail-closed revocation (§ residual #1 Layer 1): refuse to advance to
            // a revoked root (or a Store-revoked store).
            check_not_revoked(
                &id.public_key,
                &root,
                &cfg.store_id,
                &info.descriptor.tombstones,
            )?;
            let gen = info
                .roots
                .roots
                .iter()
                .find(|r| Bytes32::from_hex(&r.root).ok() == Some(root))
                .ok_or_else(|| CliError::VerificationFailed("root not in remote /roots".into()))?;
            let module_path =
                ctx.modules_dir()
                    .join(format!("{}-{}.dig", cfg.store_id.to_hex(), root.to_hex()));
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
