use std::io::Write;
use std::path::Path;

use digstore_core::{Bytes32, Urn};
use digstore_remote::{ClientError, DigClient, RequestIdentity};

use crate::cli::CatArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::{client_crypto, identity, node, remote_ops, serve, store_ops};

pub fn run(
    ctx: &CliContext,
    _ui: &crate::ui::Ui,
    args: CatArgs,
    node_flag: Option<&str>,
) -> Result<(), CliError> {
    let target = args.urn.trim();

    // Two retrieval modes:
    //   * URN (`urn:dig:…`)        → fetch + DECRYPT, so the streamed-out bytes
    //                                 are the final plaintext. A full URN is
    //                                 SELF-CONTAINED (§5.3, #227): it is read
    //                                 LOCALLY when `ctx` already backs the SAME
    //                                 store (fast, offline — the common
    //                                 "cat my own just-committed content"
    //                                 case), and via the client->node ladder
    //                                 otherwise. No local store is ever
    //                                 REQUIRED for this form.
    //   * 64-char hex retrieval key → fetch the RAW ENCRYPTED bytes within the
    //                                 active store; no decryption is performed.
    //                                 Legitimately requires an active local
    //                                 store (unchanged by #227).
    let bytes = if target.starts_with("urn:") {
        cat_by_urn(ctx, &args, target, node_flag)?
    } else if let Ok(rk) = Bytes32::from_hex(target) {
        cat_by_retrieval_key(ctx, rk)?
    } else {
        return Err(CliError::InvalidArgument(
            "expected a 'urn:dig:…' URN or a 64-character hex retrieval key".into(),
        ));
    };

    write_out(args.out.as_deref(), &bytes)
}

/// URN path: LOCAL-first, NODE-LADDER-fallback (#227). Attempts the fully
/// offline local read only when `ctx` backs the SAME store the URN names;
/// otherwise (no local store at all, or a DIFFERENT local store, or simply no
/// cached module for this generation) resolves via the network.
fn cat_by_urn(
    ctx: &CliContext,
    args: &CatArgs,
    target: &str,
    node_flag: Option<&str>,
) -> Result<Vec<u8>, CliError> {
    let urn = Urn::parse(target).map_err(|e| CliError::InvalidArgument(format!("bad urn: {e}")))?;

    let is_local_match = ctx
        .load_config()
        .map(|cfg| cfg.store_id == urn.store_id)
        .unwrap_or(false);

    if is_local_match {
        if let Some(bytes) = cat_by_urn_local(ctx, args, &urn)? {
            return Ok(bytes);
        }
        // Local store matches the URN's store id but doesn't have this
        // generation/module cached — fall through to the network below.
    }

    cat_by_urn_network(args, &urn, node_flag)
}

/// Fully-offline LOCAL read (the pre-#227 behavior, unchanged in outcome):
/// resolve, serve via the locally-compiled `.dig` module, decrypt.
///
/// Returns `Ok(None)` when the local store simply doesn't have the requested
/// generation/module cached (module_path_for / current_root miss) — this is
/// NOT an error, it just tells the caller to fall through to the node ladder.
/// Any OTHER error (a bad salt, a verification failure on an unknown/decoy
/// resource, a program-hash mismatch, …) is a REAL failure and is returned as
/// `Err` so it can never be silently masked by a network retry.
fn cat_by_urn_local(
    ctx: &CliContext,
    args: &CatArgs,
    urn: &Urn,
) -> Result<Option<Vec<u8>>, CliError> {
    // Trusted root: prefer the URN's root, else the current local root.
    let trusted_root: Bytes32 = match urn.root_hash {
        Some(r) => r,
        None => match store_ops::current_root(ctx)? {
            Some(r) => r,
            None => return Ok(None),
        },
    };

    let module_path = match store_ops::module_path_for(ctx, &urn.store_id, Some(trusted_root)) {
        Ok(p) => p,
        Err(CliError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };

    // §8.5 social conventions: a URN with no resource key resolves to the store's
    // landing resource `index.html` (its default view) when that key exists in the
    // generation manifest; otherwise it falls back to the store-level empty key.
    // Bind the resolution into an effective URN so EVERY downstream step — the
    // module's retrieval-key lookup, the per-chunk lengths, and the client
    // decryption key — derives from the same key (C9/C10).
    let urn = if urn.resource_key.is_none() {
        Urn {
            resource_key: Some(store_ops::resolve_resource_key(ctx, &trusted_root, urn)),
            ..urn.clone()
        }
    } else {
        urn.clone()
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

    let salt = parse_salt(args.salt.as_deref())?;

    // Per-chunk ciphertext lengths (from the local generation manifest) let the
    // client split the module's plain-concatenated served ciphertext (D5/C9).
    let resource_key = urn.resource_key.clone().unwrap_or_default();
    let chunk_lens =
        store_ops::resource_chunk_lens(ctx, &trusted_root, &resource_key).unwrap_or_default();

    let bytes =
        client_crypto::decrypt_and_verify(&resp, &urn, salt.as_ref(), &trusted_root, &chunk_lens)?;
    Ok(Some(bytes))
}

/// NODE-LADDER network read (`CLAUDE.md` §5.3, #227): resolve the node endpoint
/// (override `--node`/`$DIG_NODE_URL`/`node.url` > `dig.local` > `localhost` >
/// `rpc.dig.net`), fetch the resource's ciphertext + merkle inclusion proof
/// (`dig.getContent`), verify against the trusted root, and decrypt — mirroring
/// `pull`'s `pull_urn_resource`. Used whenever no matching local copy is
/// available, so a full `urn:dig:…` argument NEVER requires owning a local
/// store.
fn cat_by_urn_network(
    args: &CatArgs,
    urn: &Urn,
    node_flag: Option<&str>,
) -> Result<Vec<u8>, CliError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;

    let base =
        crate::commands::pull::rpc_base(&rt.block_on(node::resolve_node(node_flag))?.base_url);

    // Carry the §21.9 identity (harmless for the unauthenticated RPC POST; future-proofs node
    // hosts that gate the read) — same as `pull`'s `remote_ops::authed_client`.
    let (pubkey_hex, sign) = identity::request_signer()?;
    let client = DigClient::new(base).with_identity(RequestIdentity { pubkey_hex, sign });

    // Trusted root: the URN's pinned generation, else the TIP of the store singleton's
    // on-chain lineage — never a server-reported root, so the proof is verified against
    // what the chain actually anchors.
    let trusted_root: Bytes32 = match urn.root_hash {
        Some(r) => r,
        None => rt.block_on(remote_ops::onchain_tip_root(&urn.store_id))?,
    };

    // The retrieval key (like the decryption key, in `client_crypto::derive_decryption_key`) is
    // always derived from the ROOT-INDEPENDENT canonical URN (`store_ops::canonical_resource_urn`)
    // — matching what the generation manifest records at commit time — regardless of whether
    // THIS urn happens to carry a root_hash.
    let (resource_key, resp) = match &urn.resource_key {
        Some(rk) => {
            let retrieval_key = store_ops::canonical_resource_urn(urn.store_id, rk).retrieval_key();
            let resp = rt
                .block_on(client.get_content(&urn.store_id, &retrieval_key, Some(&trusted_root)))
                .map_err(remote_ops::map_remote_err)?;
            (rk.clone(), resp)
        }
        None => {
            // §8.5 social convention, resolved over the network (no local manifest to
            // consult): try the default landing resource first, falling back to the
            // store-level empty key on a genuine miss.
            let default_key =
                store_ops::canonical_resource_urn(urn.store_id, store_ops::DEFAULT_RESOURCE_KEY)
                    .retrieval_key();
            match rt.block_on(client.get_content(&urn.store_id, &default_key, Some(&trusted_root)))
            {
                Ok(resp) => (store_ops::DEFAULT_RESOURCE_KEY.to_string(), resp),
                Err(ClientError::Status(404)) => {
                    let empty_key =
                        store_ops::canonical_resource_urn(urn.store_id, "").retrieval_key();
                    let resp = rt
                        .block_on(client.get_content(
                            &urn.store_id,
                            &empty_key,
                            Some(&trusted_root),
                        ))
                        .map_err(remote_ops::map_remote_err)?;
                    (String::new(), resp)
                }
                Err(e) => return Err(remote_ops::map_remote_err(e)),
            }
        }
    };

    // `--verify-proof`'s extra program-hash attestation is a LOCAL-execution concept (it
    // checks the guest wasm the module ran was the real compiler template); a network read
    // has no local module execution to attest, but ALWAYS gets the merkle-inclusion-against-
    // trusted-root check below regardless of the flag, so no content is trusted unverified.
    let resolved_urn = Urn {
        resource_key: Some(resource_key),
        ..urn.clone()
    };
    let salt = parse_salt(args.salt.as_deref())?;
    let chunk_lens: Vec<usize> = resp.chunk_lens.iter().map(|&n| n as usize).collect();

    client_crypto::decrypt_and_verify(
        &resp,
        &resolved_urn,
        salt.as_ref(),
        &trusted_root,
        &chunk_lens,
    )
}

/// Retrieval-key path: look the key up in the active store's current generation
/// and return the RAW ENCRYPTED bytes (the served ciphertext, undecrypted).
/// Unchanged by #227 — this form legitimately requires the active local store.
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

/// Parse the `--salt` hex flag shared by both the local and network URN paths.
fn parse_salt(hex: Option<&str>) -> Result<Option<[u8; 32]>, CliError> {
    match hex {
        Some(hex) => Ok(Some(
            Bytes32::from_hex(hex)
                .map_err(|_| CliError::InvalidArgument("salt must be 32-byte hex".into()))?
                .0,
        )),
        None => Ok(None),
    }
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
