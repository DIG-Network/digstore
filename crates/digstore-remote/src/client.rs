use crate::auth::push_signing_message;
use crate::error::ClientError;
use crate::etag::parse_if_none_match;
use crate::wire::{
    DeltaNegotiateRequest, DeltaResponse, RootHistory, StoreDescriptor, TombstoneRequest,
};
use base64::Engine;
use digstore_core::{Bytes32, Bytes96, Encode, Tombstone};

/// Verify that every chunk in a server-supplied delta actually hashes to the
/// content address it is advertised under. Chunks are content-addressed by
/// `SHA-256(ciphertext)`, so a server (or MITM) cannot substitute chunk bytes
/// without detection. Returns a verification error on the first mismatch.
fn verify_delta_integrity(delta: &DeltaResponse) -> Result<(), ClientError> {
    for c in &delta.chunks {
        let data = base64::engine::general_purpose::STANDARD
            .decode(&c.data_b64)
            .map_err(|_| ClientError::Decode("bad base64 in delta chunk".into()))?;
        let want = Bytes32::from_hex(&c.hash)
            .map_err(|_| ClientError::Decode("bad delta chunk hash hex".into()))?;
        if digstore_core::sha256(&data) != want {
            return Err(ClientError::Verification(
                "delta chunk does not hash to its advertised content id".into(),
            ));
        }
    }
    Ok(())
}

/// Map a push finalize HTTP status (inline PUT or POST /module/complete) to a [`PushResult`].
fn push_finalize_result(status: u16) -> Result<PushResult, ClientError> {
    match status {
        200 | 201 => Ok(PushResult::Advanced),
        202 => Ok(PushResult::Pending),
        401 | 403 => Err(ClientError::Unauthorized(status)),
        409 => Err(ClientError::NonFastForward),
        other => Err(ClientError::Status(other)),
    }
}

/// Result of `fetch`: descriptor + root history (§21.3).
#[derive(Debug, Clone)]
pub struct FetchInfo {
    pub descriptor: StoreDescriptor,
    pub roots: RootHistory,
}

/// Result of `pull` (§21.4).
#[derive(Debug, Clone)]
pub enum PullResult {
    /// Local head already matched remote head (304).
    UpToDate,
    /// Downloaded a fresh module for `root` (size bytes).
    Module { root: Bytes32, bytes: Vec<u8> },
    /// Downloaded a delta to `root`.
    Delta { root: Bytes32, delta: DeltaResponse },
}

/// Result of `push` (§21.6 / §21.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushResult {
    /// 201: served head advanced.
    Advanced,
    /// 202: accepted into pending state (§21.4).
    Pending,
}

/// A BLS signer over the 32-byte canonical request message (paper §21.9). Boxed +
/// `Send + Sync` so it can live on the client and be called per request.
pub type RequestSignFn = Box<dyn Fn(&[u8; 32]) -> Bytes96 + Send + Sync>;

/// The caller's per-request authentication identity (paper §21.9). `pubkey_hex` is
/// the 48-byte BLS G1 identity public key (the `<user>` in a `dig://<user>@host/…`
/// origin); `sign` produces a BLS signature over the 32-byte canonical request
/// message. Stored on the client so EVERY request is signed.
pub struct RequestIdentity {
    pub pubkey_hex: String,
    pub sign: RequestSignFn,
}

/// HTTPS remote client: clone/fetch/pull/push (§21). When constructed with an
/// identity, every request carries the §21.9 auth headers (`X-Dig-Identity`,
/// `X-Dig-Timestamp`, `X-Dig-Nonce`, `X-Dig-Auth`).
pub struct DigClient {
    base_url: String,
    http: reqwest::Client,
    identity: Option<RequestIdentity>,
}

impl DigClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        // Disable automatic redirect following: a malicious/compromised server
        // must not be able to bounce a push (which carries the X-Dig-Signature and
        // the module body, and optionally a bearer token) to an attacker-chosen
        // host, nor use redirects as an SSRF primitive. Redirects are a protocol
        // error here, surfaced as a non-success status.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        DigClient {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            identity: None,
        }
    }

    pub fn with_client(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        DigClient {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            identity: None,
        }
    }

    /// Attach a per-request signing identity (§21.9). Builder-style; chains after
    /// `new`/`with_client`. The CLI sets this from its persistent identity key so
    /// the whole remote protocol is authenticated.
    pub fn with_identity(mut self, identity: RequestIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Stamp the §21.9 per-request auth headers onto a request, if an identity is
    /// configured. `method` is the logical operation bound into the signed message
    /// (`fetch`/`roots`/`module`/`content`/`proof`/`push`/`tombstone`/`delta`), so a
    /// signature for one operation can never be replayed as another. A fresh
    /// timestamp + random nonce make every signed request unique (server-side
    /// freshness window + nonce defeat replay). No-op when no identity is set
    /// (anonymous client / tests against an open server).
    fn authed(
        &self,
        req: reqwest::RequestBuilder,
        method: &str,
        store_id: &Bytes32,
    ) -> reqwest::RequestBuilder {
        let Some(identity) = &self.identity else {
            return req;
        };
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut nonce = [0u8; 32];
        let _ = getrandom::getrandom(&mut nonce);
        let msg = digstore_crypto::request_signing_message(method, store_id, timestamp, &nonce);
        let sig = (identity.sign)(&msg);
        req.header("X-Dig-Identity", &identity.pubkey_hex)
            .header("X-Dig-Timestamp", timestamp.to_string())
            .header("X-Dig-Nonce", hex::encode(nonce))
            .header("X-Dig-Auth", hex::encode(sig.0))
    }

    /// §21.3 fetch: descriptor + root history only.
    pub async fn fetch(&self, store_id: &Bytes32) -> Result<FetchInfo, ClientError> {
        let id = store_id.to_hex();
        let d: StoreDescriptor = self
            .authed(
                self.http.get(self.url(&format!("/stores/{id}"))),
                "fetch",
                store_id,
            )
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?
            .error_for_status()
            .map_err(|e| ClientError::Status(e.status().map(|s| s.as_u16()).unwrap_or(0)))?
            .json()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))?;
        let roots: RootHistory = self
            .authed(
                self.http.get(self.url(&format!("/stores/{id}/roots"))),
                "roots",
                store_id,
            )
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?
            .json()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))?;
        Ok(FetchInfo {
            descriptor: d,
            roots,
        })
    }

    /// §21.3 clone: download + verify the module. `verify` is called with
    /// (module_bytes, served_root) and must return Ok(()) when the module
    /// validates to that root (full merkle verification lives in the caller).
    pub async fn clone_store<V>(
        &self,
        store_id: &Bytes32,
        verify: V,
    ) -> Result<(Bytes32, Vec<u8>), ClientError>
    where
        V: FnOnce(&[u8], &Bytes32) -> Result<(), String>,
    {
        let id = store_id.to_hex();
        let resp = self
            .authed(
                self.http.get(self.url(&format!("/stores/{id}/module"))),
                "module",
                store_id,
            )
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        let etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let root = etag
            .as_deref()
            .and_then(parse_if_none_match)
            .ok_or_else(|| ClientError::Verification("missing/invalid ETag".into()))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?
            .to_vec();
        verify(&bytes, &root).map_err(ClientError::Verification)?;
        Ok((root, bytes))
    }

    /// §21.4 pull: advance the local head. `local_root` is the client's current
    /// generation; If-None-Match short-circuits to UpToDate on 304. When
    /// `prefer_delta` and `local_root` is Some, attempt GET /delta first.
    pub async fn pull(
        &self,
        store_id: &Bytes32,
        local_root: Option<Bytes32>,
        prefer_delta: bool,
    ) -> Result<PullResult, ClientError> {
        let id = store_id.to_hex();
        // determine remote head.
        let desc = self.fetch(store_id).await?;
        let remote_root = Bytes32::from_hex(&desc.descriptor.current_root)
            .map_err(|_| ClientError::Decode("bad current_root".into()))?;
        if local_root == Some(remote_root) {
            return Ok(PullResult::UpToDate);
        }
        if prefer_delta {
            if let Some(from) = local_root {
                let from_h = from.to_hex();
                let to_h = remote_root.to_hex();
                let resp = self
                    .authed(
                        self.http
                            .get(self.url(&format!("/stores/{id}/delta?from={from_h}&to={to_h}"))),
                        "delta",
                        store_id,
                    )
                    .send()
                    .await
                    .map_err(|e| ClientError::Transport(e.to_string()))?;
                if resp.status().is_success() {
                    let delta: DeltaResponse = resp
                        .json()
                        .await
                        .map_err(|e| ClientError::Decode(e.to_string()))?;
                    verify_delta_integrity(&delta)?;
                    return Ok(PullResult::Delta {
                        root: remote_root,
                        delta,
                    });
                }
                // fall through to full module on non-success delta.
            }
        }
        // full module download with conditional request.
        let mut req = self.authed(
            self.http.get(self.url(&format!("/stores/{id}/module"))),
            "module",
            store_id,
        );
        if let Some(lr) = local_root {
            req = req.header(
                reqwest::header::IF_NONE_MATCH,
                format!("\"{}\"", lr.to_hex()),
            );
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        if resp.status().as_u16() == 304 {
            return Ok(PullResult::UpToDate);
        }
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?
            .to_vec();
        Ok(PullResult::Module {
            root: remote_root,
            bytes,
        })
    }

    /// §21.6 push (dig RPC push protocol v1): the dig RPC is the STANDARD push protocol, and the
    /// spec carries the module EITHER inline OR via a presigned upload URL — negotiated at init —
    /// because an HTTPS edge (e.g. a Lambda) caps request bodies well below a real capsule's size.
    ///
    /// Flow: (1) POST `/module/upload` with `{parent_root,new_root,program_hash,size_bytes}` +
    /// the C7 push signature → server replies `{mode:"inline"|"presigned", upload_id, url?}`.
    /// (2a) inline → PUT the bytes to `/module?root=` (the server finalizes). (2b) presigned → PUT
    /// the bytes straight to the presigned URL, then POST `/module/complete`. One C7 signature
    /// authorizes the whole push; each leg re-sends it and the server re-verifies against the
    /// store's registered publisher key. `sign` is the caller's BLS signer over the 32-byte push
    /// message.
    #[allow(clippy::too_many_arguments)]
    pub async fn push<S>(
        &self,
        store_id: &Bytes32,
        parent: &Bytes32,
        new_root: &Bytes32,
        module: &[u8],
        pending: bool,
        bearer: Option<&str>,
        sign: S,
    ) -> Result<PushResult, ClientError>
    where
        S: FnOnce(&[u8; 32]) -> Bytes96,
    {
        let _ = pending; // the rpc §21 push protocol finalizes on the server (always advances).
        let id = store_id.to_hex();
        // CONVENTIONS C7: argument order is (root, store_id).
        let msg = push_signing_message(new_root, store_id);
        let sig_hex = hex::encode(sign(&msg).0);
        let new_root_hex = new_root.to_hex();
        // program_hash = SHA-256 of the whole `.dig` (the server validates the uploaded bytes
        // against this before promoting them).
        let program_hash = digstore_core::sha256(module).to_hex();

        // (1) push-init: negotiate inline vs presigned.
        let init_body = serde_json::json!({
            "parent_root": parent.to_hex(),
            "new_root": new_root_hex,
            "program_hash": program_hash,
            "size_bytes": module.len() as u64,
        });
        let mut ireq = self
            .authed(
                self.http
                    .post(self.url(&format!("/stores/{id}/module/upload"))),
                "push-init",
                store_id,
            )
            .header("X-Dig-Signature", &sig_hex)
            .json(&init_body);
        if let Some(t) = bearer {
            ireq = ireq.header(reqwest::header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let iresp = ireq
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        match iresp.status().as_u16() {
            200 => {}
            401 | 403 => return Err(ClientError::Unauthorized(iresp.status().as_u16())),
            409 => return Err(ClientError::NonFastForward),
            other => return Err(ClientError::Status(other)),
        }
        let init: serde_json::Value = iresp
            .json()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))?;
        // Idempotent: the server reports the head is already at this root.
        if init.get("status").and_then(|v| v.as_str()) == Some("advanced") {
            return Ok(PushResult::Advanced);
        }
        let mode = init.get("mode").and_then(|v| v.as_str()).unwrap_or_default();

        match mode {
            "inline" => {
                // (2a) PUT the body to /module?root=; the server validates + finalizes.
                let mut req = self
                    .authed(
                        self.http
                            .put(self.url(&format!("/stores/{id}/module?root={new_root_hex}"))),
                        "push",
                        store_id,
                    )
                    .header("X-Dig-Signature", &sig_hex)
                    .header("X-Dig-Upload-Id", &new_root_hex)
                    .body(module.to_vec());
                if let Some(t) = bearer {
                    req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {t}"));
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| ClientError::Transport(e.to_string()))?;
                push_finalize_result(resp.status().as_u16())
            }
            "presigned" => {
                // (2b) PUT the bytes straight to the presigned S3 URL (no auth headers — the URL
                // is the credential), then POST /module/complete to finalize.
                let url = init
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ClientError::Decode("push-init: missing presigned url".into()))?;
                let s3 = self
                    .http
                    .put(url)
                    .body(module.to_vec())
                    .send()
                    .await
                    .map_err(|e| ClientError::Transport(e.to_string()))?;
                if !s3.status().is_success() {
                    return Err(ClientError::Status(s3.status().as_u16()));
                }
                let complete_body = serde_json::json!({
                    "upload_id": new_root_hex,
                    "new_root": new_root_hex,
                });
                let mut req = self
                    .authed(
                        self.http
                            .post(self.url(&format!("/stores/{id}/module/complete"))),
                        "push-complete",
                        store_id,
                    )
                    .header("X-Dig-Signature", &sig_hex)
                    .json(&complete_body);
                if let Some(t) = bearer {
                    req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {t}"));
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| ClientError::Transport(e.to_string()))?;
                push_finalize_result(resp.status().as_u16())
            }
            other => Err(ClientError::Decode(format!("push-init: unknown mode {other:?}"))),
        }
    }

    /// SECURITY.md residual #1 Layer 1: publish a signed revocation tombstone.
    /// `sign` is the caller's BLS signer over the 32-byte tombstone message
    /// (`digstore_crypto::tombstone_signing_message`). The remote re-verifies the
    /// signature against the store's published key before persisting (403 on a bad
    /// signature). Returns Ok(()) on 201.
    pub async fn post_tombstone<S>(
        &self,
        store_id: &Bytes32,
        tombstone: &Tombstone,
        sign: S,
    ) -> Result<(), ClientError>
    where
        S: FnOnce(&[u8; 32]) -> Bytes96,
    {
        let id = store_id.to_hex();
        let msg = digstore_crypto::tombstone_signing_message(tombstone);
        let sig = sign(&msg);
        let body = TombstoneRequest {
            record: hex::encode(tombstone.to_bytes()),
            signature: hex::encode(sig.0),
        };
        let resp = self
            .authed(
                self.http.post(self.url(&format!("/stores/{id}/tombstone"))),
                "tombstone",
                store_id,
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        match resp.status().as_u16() {
            201 => Ok(()),
            401 | 403 => Err(ClientError::Unauthorized(resp.status().as_u16())),
            other => Err(ClientError::Status(other)),
        }
    }

    /// §21.5 negotiated delta from a have-summary.
    pub async fn negotiate_delta(
        &self,
        store_id: &Bytes32,
        to: &Bytes32,
        have: &[Bytes32],
    ) -> Result<DeltaResponse, ClientError> {
        let id = store_id.to_hex();
        let body = DeltaNegotiateRequest {
            to: to.to_hex(),
            have: have.iter().map(|h| h.to_hex()).collect(),
        };
        let resp = self
            .authed(
                self.http.post(self.url(&format!("/stores/{id}/delta"))),
                "delta",
                store_id,
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        let delta: DeltaResponse = resp
            .json()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))?;
        verify_delta_integrity(&delta)?;
        Ok(delta)
    }
}
