use std::path::PathBuf;

use digstore_core::{Bytes32, Urn};

use crate::cli::PullArgs;
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::{client_crypto, dighub, identity, remote_ops};
use digstore_remote::{DigClient, RequestIdentity};

/// How a `pull` argument routes (factored out as a pure helper so the routing — URN-with-resource
/// vs URN-without-resource vs remote-name — is unit-testable without any network).
#[derive(Debug, PartialEq, Eq)]
enum PullRoute {
    /// `urn:dig:…/<resource>` → NETWORK content-read by retrieval key (fetch ciphertext + proof,
    /// verify against the trusted root, decrypt, write plaintext).
    UrnResource(Urn),
    /// `urn:dig:…` with NO resource key → whole-store sync (needs a local store matching the id).
    UrnWholeStore(Urn),
    /// A configured remote name (default `origin`) → existing whole-store sync, unchanged.
    RemoteName(String),
}

/// Classify a `pull` argument. A `urn:`-prefixed value is parsed as a URN (a parse error is
/// surfaced); everything else is a remote name. A URN WITH a non-empty `resource_key` is the new
/// network content-read; a URN without one falls back to whole-store sync.
fn route(arg: &str) -> Result<PullRoute, CliError> {
    if arg.starts_with("urn:") {
        let urn =
            Urn::parse(arg).map_err(|e| CliError::InvalidArgument(format!("bad urn: {e}")))?;
        match urn.resource_key.as_deref() {
            Some(rk) if !rk.is_empty() => Ok(PullRoute::UrnResource(urn)),
            _ => Ok(PullRoute::UrnWholeStore(urn)),
        }
    } else {
        Ok(PullRoute::RemoteName(arg.to_string()))
    }
}

/// Reduce a resolved remote URL (which may carry a `/stores/<id>` path) to its RPC host base
/// (`scheme://host[:port]`) — the dig RPC `dig.getContent` POST is rooted at the host, not under
/// `/stores/…`. Any path/userinfo is dropped. A non-URL value passes through unchanged.
fn rpc_base(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.trim_end_matches('/').to_string();
    };
    let authority = rest.split('/').next().unwrap_or("");
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    format!("{scheme}://{host_port}")
}

/// The last path segment of a resource key, used to name the output file when `--out` is omitted.
/// Falls back to the whole key (sanitized) then to "resource".
fn out_file_name(resource_key: &str) -> String {
    let last = resource_key
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(resource_key);
    let name = last.trim();
    if name.is_empty() {
        "resource".to_string()
    } else {
        name.to_string()
    }
}

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: PullArgs) -> Result<(), CliError> {
    // Product gate: require a dighub account only for a DIGHUB remote (*.dig.net). A URN pull or a
    // remote name resolving to a self-hosted / loopback node needs no dighub account.
    let gate_base = if args.remote.starts_with("urn:") {
        config::resolve_remote_url(ctx, "origin").unwrap_or_else(|_| "https://rpc.dig.net".into())
    } else {
        config::resolve_remote_url(ctx, &args.remote).unwrap_or_default()
    };
    if dighub::is_dighub_remote(&gate_base) {
        dighub::ensure_logged_in(ui)?;
    }
    match route(&args.remote)? {
        PullRoute::UrnResource(urn) => pull_urn_resource(ctx, ui, &args, urn),
        PullRoute::UrnWholeStore(urn) => {
            // A bare-store URN can only be synced into a LOCAL store with the same id. The whole-
            // store sync (remote_ops::pull_from) operates on the store in the current `.dig` dir.
            let cfg = ctx.load_config()?;
            if urn.store_id != cfg.store_id {
                return Err(CliError::InvalidArgument(format!(
                    "URN store id {} does not match the local store {}; pull a resource URN \
                     (urn:dig:…/<path>) for a network read, or run pull from the store's own dir",
                    urn.store_id.to_hex(),
                    cfg.store_id.to_hex()
                )));
            }
            pull_whole_store(ctx, ui, "origin")
        }
        PullRoute::RemoteName(name) => pull_whole_store(ctx, ui, &name),
    }
}

/// Existing behavior: whole-store `.dig` sync over the §21 protocol (unchanged).
fn pull_whole_store(ctx: &CliContext, ui: &crate::ui::Ui, remote: &str) -> Result<(), CliError> {
    let base = config::resolve_remote_url(ctx, remote)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    let root = rt.block_on(remote_ops::pull_from(ctx, ui, &base))?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "root": root.to_hex() }));
    } else {
        ui.success(format!("pulled; local root is now {}", root.to_hex()));
    }
    Ok(())
}

/// NETWORK content-read by retrieval key. Fetch the resource's ciphertext + merkle inclusion proof
/// from the remote (`dig.getContent`), VERIFY the proof against the trusted root, AUTO-DECRYPT with
/// the URN-derived key, and write the plaintext.
fn pull_urn_resource(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    args: &PullArgs,
    urn: Urn,
) -> Result<(), CliError> {
    // The RPC host base: the configured `origin` if present, else the public RPC. Reduce either to
    // the scheme://host root the dig RPC POST is addressed at.
    let base = rpc_base(
        &config::resolve_remote_url(ctx, "origin").unwrap_or_else(|_| "https://rpc.dig.net".into()),
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;

    // Carry the §21.9 identity (harmless for the unauthenticated RPC POST; future-proofs node hosts
    // that gate the read). Built the same way as remote_ops::authed_client.
    let (pubkey_hex, sign) = identity::request_signer()?;
    let client = DigClient::new(base).with_identity(RequestIdentity { pubkey_hex, sign });

    // Trusted root: the URN's pinned generation, else the TIP of the store singleton's
    // on-chain lineage. When the URN names no generation we default to the chain tip — the
    // authority — NOT a server-reported root, so the proof is verified against what the
    // chain actually anchors.
    let trusted_root: Bytes32 = match urn.root_hash {
        Some(r) => r,
        None => rt.block_on(remote_ops::onchain_tip_root(&urn.store_id))?,
    };

    let retrieval_key = urn.retrieval_key();
    let resp = rt
        .block_on(client.get_content(&urn.store_id, &retrieval_key, Some(&trusted_root)))
        .map_err(remote_ops::map_remote_err)?;

    // Verify the merkle inclusion proof against the trusted root AND decrypt (one call). A proof
    // that does not fold to `trusted_root`, or a wrong key/salt, fails here.
    let chunk_lens: Vec<usize> = resp.chunk_lens.iter().map(|&n| n as usize).collect();
    let plain = client_crypto::decrypt_and_verify(&resp, &urn, None, &trusted_root, &chunk_lens)?;

    // Write to --out, else a file named after the resource key's last path segment in cwd.
    let resource_key = urn.resource_key.clone().unwrap_or_default();
    let out_path: PathBuf = match &args.out {
        Some(p) => p.clone(),
        None => PathBuf::from(out_file_name(&resource_key)),
    };
    std::fs::write(&out_path, &plain)
        .map_err(|e| CliError::Other(anyhow::anyhow!("write {}: {e}", out_path.display())))?;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "urn": urn.canonical(),
            "bytes": plain.len(),
            "out": out_path.display().to_string(),
        }));
    } else {
        ui.success(format!(
            "pulled {} bytes → {}",
            plain.len(),
            out_path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urn_with_resource() -> String {
        format!("urn:dig:chia:{}/docs/readme.md", "ab".repeat(32))
    }
    fn urn_no_resource() -> String {
        format!("urn:dig:chia:{}", "ab".repeat(32))
    }

    #[test]
    fn routes_urn_with_resource_to_network_read() {
        match route(&urn_with_resource()).unwrap() {
            PullRoute::UrnResource(u) => {
                assert_eq!(u.resource_key.as_deref(), Some("docs/readme.md"))
            }
            other => panic!("expected UrnResource, got {other:?}"),
        }
    }

    #[test]
    fn routes_urn_without_resource_to_whole_store() {
        assert!(matches!(
            route(&urn_no_resource()).unwrap(),
            PullRoute::UrnWholeStore(_)
        ));
    }

    #[test]
    fn routes_urn_with_root_and_resource_to_network_read() {
        let arg = format!(
            "urn:dig:chia:{}:{}/index.html",
            "ab".repeat(32),
            "cd".repeat(32)
        );
        assert!(matches!(route(&arg).unwrap(), PullRoute::UrnResource(_)));
    }

    #[test]
    fn routes_remote_name() {
        assert_eq!(
            route("origin").unwrap(),
            PullRoute::RemoteName("origin".into())
        );
        assert_eq!(
            route("my-node").unwrap(),
            PullRoute::RemoteName("my-node".into())
        );
    }

    #[test]
    fn malformed_urn_is_an_argument_error() {
        let err = route("urn:dig:not-a-urn").unwrap_err();
        assert!(matches!(err, CliError::InvalidArgument(_)));
    }

    #[test]
    fn rpc_base_strips_store_path_and_userinfo() {
        let id = "ab".repeat(32);
        assert_eq!(
            rpc_base(&format!("https://rpc.dig.net/stores/{id}")),
            "https://rpc.dig.net"
        );
        assert_eq!(
            rpc_base("https://alice@rpc.dig.net/stores/x"),
            "https://rpc.dig.net"
        );
        assert_eq!(rpc_base("https://rpc.dig.net"), "https://rpc.dig.net");
        assert_eq!(
            rpc_base("http://127.0.0.1:9000/stores/x"),
            "http://127.0.0.1:9000"
        );
    }

    #[test]
    fn out_file_name_uses_last_segment() {
        assert_eq!(out_file_name("docs/readme.md"), "readme.md");
        assert_eq!(out_file_name("index.html"), "index.html");
        assert_eq!(out_file_name("a/b/c/"), "c");
        assert_eq!(out_file_name(""), "resource");
    }
}
