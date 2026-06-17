use crate::auth::push_signing_message;
use crate::error::ClientError;
use crate::etag::parse_if_none_match;
use crate::wire::{
    DeltaNegotiateRequest, DeltaResponse, RootHistory, StoreDescriptor, TombstoneRequest,
};
use base64::Engine;
use digstore_core::{
    Bytes32, Bytes96, ContentResponse, Decode, Decoder, Encode, MerkleProof, Tombstone,
};

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

/// Extract a human-readable message from a server error response body.
/// Tries `{"message": "…"}` (and `{"error": "…"}` as fallback), then raw body.
async fn extract_server_message(resp: reqwest::Response) -> String {
    match resp.text().await {
        Ok(body) if !body.is_empty() => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
                    return msg.to_string();
                }
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    return err.to_string();
                }
            }
            body
        }
        _ => String::new(),
    }
}

/// Map a push finalize HTTP response to a [`PushResult`], surfacing the server
/// body on failure.
async fn push_finalize_result(resp: reqwest::Response) -> Result<PushResult, ClientError> {
    match resp.status().as_u16() {
        200 | 201 => Ok(PushResult::Advanced),
        202 => Ok(PushResult::Pending),
        409 => Err(ClientError::NonFastForward),
        status @ (401 | 403) => {
            let message = extract_server_message(resp).await;
            Err(ClientError::Remote { status, message })
        }
        status => {
            let message = extract_server_message(resp).await;
            if message.is_empty() {
                Err(ClientError::Status(status))
            } else {
                Err(ClientError::Remote { status, message })
            }
        }
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
        // Send an explicit User-Agent: this reqwest build (default-features = false) sends NO
        // default UA, and the rpc.dig.net WAF blocks no-User-Agent requests with a 403 before
        // they reach the Lambda — which silently breaks every §21 remote call (fetch/clone/pull/
        // push). A real UA makes the edge admit the request.
        let http = reqwest::Client::builder()
            .user_agent(concat!("digstore/", env!("CARGO_PKG_VERSION")))
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
    ///
    /// `on_progress`, when `Some`, is called as `(bytes_done, total_bytes)` after
    /// each received chunk (total is 0 when the server omits Content-Length).
    pub async fn clone_store<V>(
        &self,
        store_id: &Bytes32,
        verify: V,
        on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
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
        let total = resp.content_length().unwrap_or(0);
        let bytes = download_with_progress(resp, total, on_progress).await?;
        verify(&bytes, &root).map_err(ClientError::Verification)?;
        Ok((root, bytes))
    }

    /// §21.4 pull: advance the local head. `local_root` is the client's current
    /// generation; If-None-Match short-circuits to UpToDate on 304. When
    /// `prefer_delta` and `local_root` is Some, attempt GET /delta first.
    ///
    /// `on_progress`, when `Some`, is called as `(bytes_done, total_bytes)` after
    /// each received chunk during a full module download (not for delta).
    pub async fn pull(
        &self,
        store_id: &Bytes32,
        local_root: Option<Bytes32>,
        prefer_delta: bool,
        on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
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
        let total = resp.content_length().unwrap_or(0);
        let bytes = download_with_progress(resp, total, on_progress).await?;
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
    /// `publisher_pubkey` is the 48-byte G1 publisher key (hex). It is sent in push-init so a remote
    /// that does not yet host this store can AUTO-CREATE its record on first push (trust-on-first-use,
    /// keyed by this key); a remote that already has a record ignores it.
    ///
    /// `on_progress`, when `Some`, is called as `(bytes_done, total_bytes)` as upload bytes are sent.
    /// For the **inline** leg the bytes are streamed in 256 KiB chunks so the bar advances
    /// continuously. For the **presigned S3** leg the PUT body must be sent as a single buffer with
    /// a fixed `Content-Length` matching the presigned signature — chunked transfer encoding would
    /// break S3 signature validation — so progress is reported in two coarse steps:
    /// `(0, total)` before the send and `(total, total)` after it succeeds.
    #[allow(clippy::too_many_arguments)]
    pub async fn push<S>(
        &self,
        store_id: &Bytes32,
        parent: &Bytes32,
        new_root: &Bytes32,
        module: &[u8],
        pending: bool,
        bearer: Option<&str>,
        publisher_pubkey: &str,
        sign: S,
        on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
    ) -> Result<PushResult, ClientError>
    where
        S: FnOnce(&[u8; 32]) -> Bytes96,
    {
        let id = store_id.to_hex();
        // CONVENTIONS C7: argument order is (root, store_id).
        let msg = push_signing_message(new_root, store_id);
        let sig_hex = hex::encode(sign(&msg).0);
        let new_root_hex = new_root.to_hex();
        // program_hash = SHA-256 of the whole `.dig` (the server validates the uploaded bytes
        // against this before promoting them).
        let program_hash = digstore_core::sha256(module).to_hex();

        // (1) push-init: negotiate inline vs presigned.
        // First push: the remote has no head, so parent_root MUST be empty (the server matches
        // an empty parent against its `current == None`). The genesis/all-zero root is NOT a real
        // parent — send it as empty, or the server rejects with a spurious non-fast-forward.
        let parent_field = if parent == &Bytes32::default() {
            String::new()
        } else {
            parent.to_hex()
        };
        let init_body = serde_json::json!({
            "parent_root": parent_field,
            "new_root": new_root_hex,
            "program_hash": program_hash,
            "size_bytes": module.len() as u64,
            "store_pubkey": publisher_pubkey,
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
            409 => return Err(ClientError::NonFastForward),
            status @ (401 | 403) => {
                let message = extract_server_message(iresp).await;
                return Err(ClientError::Remote { status, message });
            }
            status => {
                let message = extract_server_message(iresp).await;
                return Err(if message.is_empty() {
                    ClientError::Status(status)
                } else {
                    ClientError::Remote { status, message }
                });
            }
        }
        let init: serde_json::Value = iresp
            .json()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))?;
        // Idempotent: the server reports the head is already at this root.
        if init.get("status").and_then(|v| v.as_str()) == Some("advanced") {
            return Ok(PushResult::Advanced);
        }
        let mode = init
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        match mode {
            "inline" => {
                // (2a) Stream the module bytes to /module?root= in 256 KiB chunks so the
                // progress bar advances during the actual network send.
                let total = module.len() as u64;
                let body = upload_stream_body(module, total, on_progress);
                let mut req = self
                    .authed(
                        self.http
                            .put(self.url(&format!("/stores/{id}/module?root={new_root_hex}"))),
                        "push",
                        store_id,
                    )
                    .header("X-Dig-Signature", &sig_hex)
                    .header("X-Dig-Upload-Id", &new_root_hex)
                    // §21.4: a node may accept into pending state; the hub ignores this (always
                    // advances). Carried on the inline finalize leg (nodes use inline).
                    .header(
                        "X-Dig-Push-Mode",
                        if pending { "pending" } else { "advance" },
                    )
                    // Set Content-Length explicitly so the server (and intermediaries) see a
                    // sized body even though we use wrap_stream.
                    .header(reqwest::header::CONTENT_LENGTH, total.to_string())
                    .body(body);
                if let Some(t) = bearer {
                    req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {t}"));
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| ClientError::Transport(e.to_string()))?;
                push_finalize_result(resp).await
            }
            "presigned" => {
                // (2b) PUT the bytes straight to the presigned S3 URL (no auth headers — the URL
                // is the credential), then POST /module/complete to finalize.
                //
                // S3 presigned PUT validates Content-Length against the value embedded in the
                // signature. Using `wrap_stream` without a fixed Content-Length sends
                // Transfer-Encoding: chunked, which S3 rejects (SignatureDoesNotMatch /
                // MalformedXML). We therefore keep this leg as a single buffered PUT.
                // Progress is reported in two coarse steps: (0, total) before the send,
                // (total, total) after success — enough for the bar to show "started / done".
                let total = module.len() as u64;
                if let Some(cb) = on_progress {
                    cb(0, total);
                }
                let url = init.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
                    ClientError::Decode("push-init: missing presigned url".into())
                })?;
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
                if let Some(cb) = on_progress {
                    cb(total, total);
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
                push_finalize_result(resp).await
            }
            other => Err(ClientError::Decode(format!(
                "push-init: unknown mode {other:?}"
            ))),
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
            status @ (401 | 403) => {
                let message = extract_server_message(resp).await;
                Err(ClientError::Remote { status, message })
            }
            status => {
                let message = extract_server_message(resp).await;
                Err(if message.is_empty() {
                    ClientError::Status(status)
                } else {
                    ClientError::Remote { status, message }
                })
            }
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

    /// NETWORK content-read by retrieval key: fetch a resource's CIPHERTEXT + its merkle inclusion
    /// proof from the remote and reassemble them into a [`ContentResponse`] for client-side verify +
    /// decrypt. This is the read-by-URN counterpart of clone/pull (which sync a whole `.dig`).
    ///
    /// TRANSPORT — `dig.getContent` JSON-RPC (POST), NOT the §21 `GET /content/{key}`:
    ///   * `dig.getContent` is the NETWORK-STANDARD content interface every node speaks (the same
    ///     one `hub.dig.net/apps/web/lib/dig-client.js` consumes); it returns the ciphertext, the
    ///     inclusion proof, the per-chunk lengths, AND the served roothash in one parseable JSON
    ///     envelope, and pages large resources via `next_offset`/`complete`.
    ///   * the legacy §21 `GET /stores/{id}/content/{key}` splits the proof into an
    ///     `X-Dig-Inclusion-Proof` header from the raw-octet body and is a diagnostic-only fallback
    ///     (the rpc.dig.net distribution disables caching for it); no client uses it.
    ///
    /// So we mirror `dig-client.js` exactly. The RPC is the browser path and is UNauthenticated; the
    /// §21.9 identity headers are not required here (and are not sent — the RPC POST is not a §21
    /// store route). `base_url` is the RPC host root (e.g. `https://rpc.dig.net`).
    ///
    /// WIRE DECODE (mirrors `dig-client.js`):
    ///   * `ciphertext`      — base64 of the raw ciphertext; reassembled across all pages.
    ///   * `inclusion_proof` — base64 of the codec-encoded [`MerkleProof`] (== `MerkleProof::to_bytes`,
    ///     the SAME framing the host's `encode_inclusion_proof` emits) → decode with `MerkleProof::decode`.
    ///   * `chunk_lens`      — per-chunk CIPHERTEXT lengths of the FULL resource, sent ONCE on the
    ///     first window (offset 0); captured there and carried through.
    ///   * `roothash`        — the served generation root, echoed as `root` in the result.
    ///
    /// `root` (when `Some`) pins the read to one generation (hex in the request); `None` lets the
    /// server resolve the store's latest confirmed root. The returned `ContentResponse` carries the
    /// SEALED ciphertext + the (untrusted) proof + the served roothash; the CALLER verifies the proof
    /// against its own chain-anchored trusted root and decrypts — this method performs NO crypto.
    pub async fn get_content(
        &self,
        store_id: &Bytes32,
        retrieval_key: &Bytes32,
        root: Option<&Bytes32>,
    ) -> Result<ContentResponse, ClientError> {
        let store_hex = store_id.to_hex();
        let rk_hex = retrieval_key.to_hex();
        let root_hex = root.map(|r| r.to_hex());

        // Reassemble the (paged) ciphertext, capturing the proof + chunk_lens + roothash. The
        // server caps each window at ~3 MiB and streams via next_offset; loop until `complete`.
        let mut ciphertext: Vec<u8> = Vec::new();
        let mut offset: u64 = 0;
        let mut chunk_lens: Vec<u32> = Vec::new();
        let mut proof_b64 = String::new();
        let mut roothash_hex = String::new();
        loop {
            let mut params = serde_json::json!({
                "store_id": store_hex,
                "retrieval_key": rk_hex,
                "offset": offset,
            });
            if let Some(rh) = &root_hex {
                params["root"] = serde_json::json!(rh);
            }
            let body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "dig.getContent",
                "params": params,
            });
            // POST to the RPC host root (the dig RPC is not under /stores/…). No §21.9 auth (the
            // RPC is the public browser path); the WAF-safe User-Agent is on the builder.
            let resp = self
                .http
                .post(self.url(""))
                .json(&body)
                .send()
                .await
                .map_err(|e| ClientError::Transport(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(ClientError::Status(resp.status().as_u16()));
            }
            let env: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ClientError::Decode(e.to_string()))?;
            if let Some(err) = env.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("dig.getContent error")
                    .to_string();
                let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                // -32004 == "resource not available at the requested root" (a genuine infra miss).
                return Err(if code == -32004 {
                    ClientError::Status(404)
                } else {
                    ClientError::Remote {
                        status: 502,
                        message: msg,
                    }
                });
            }
            let result = env
                .get("result")
                .ok_or_else(|| ClientError::Decode("dig.getContent: missing result".into()))?;

            // ciphertext (base64 of raw bytes) for this window.
            let chunk_b64 = result
                .get("ciphertext")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let chunk = base64::engine::general_purpose::STANDARD
                .decode(chunk_b64)
                .map_err(|_| ClientError::Decode("bad base64 ciphertext".into()))?;
            ciphertext.extend_from_slice(&chunk);

            // The proof + chunk_lens + roothash come on the first window (offset 0); keep the
            // first non-empty proof we see (the server sends it on every window).
            if proof_b64.is_empty() {
                if let Some(p) = result.get("inclusion_proof").and_then(|v| v.as_str()) {
                    proof_b64 = p.to_string();
                }
            }
            if chunk_lens.is_empty() {
                if let Some(arr) = result.get("chunk_lens").and_then(|v| v.as_array()) {
                    chunk_lens = arr
                        .iter()
                        .filter_map(|n| n.as_u64().map(|n| n as u32))
                        .collect();
                }
            }
            if roothash_hex.is_empty() {
                if let Some(rh) = result.get("root").and_then(|v| v.as_str()) {
                    roothash_hex = rh.to_string();
                }
            }

            let complete = result
                .get("complete")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let next = result.get("next_offset").and_then(|v| v.as_u64());
            match (complete, next) {
                (false, Some(n)) => offset = n,
                _ => break,
            }
        }

        // Decode the inclusion proof: base64 → codec `MerkleProof` (the same framing the host's
        // `encode_inclusion_proof`/`MerkleProof::to_bytes` emits). An empty/garbage proof yields a
        // proof that will not verify against the trusted root — the caller's verify gate catches it.
        let merkle_proof = decode_inclusion_proof(&proof_b64)?;
        let roothash = Bytes32::from_hex(&roothash_hex)
            .or_else(|_| root.map(|r| Ok(*r)).unwrap_or(Err(())))
            .map_err(|_| ClientError::Decode("dig.getContent: missing/invalid roothash".into()))?;

        Ok(ContentResponse {
            ciphertext,
            merkle_proof,
            roothash,
            chunk_lens,
        })
    }
}

/// Decode a base64'd, codec-encoded [`MerkleProof`] (the `inclusion_proof` field / the host's
/// `X-Dig-Inclusion-Proof` value). Round-trips with `MerkleProof::to_bytes` + base64.
fn decode_inclusion_proof(b64: &str) -> Result<MerkleProof, ClientError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| ClientError::Decode("bad base64 inclusion proof".into()))?;
    let mut dec = Decoder::new(&bytes);
    MerkleProof::decode(&mut dec)
        .map_err(|e| ClientError::Decode(format!("inclusion proof: {e:?}")))
}

// ---------------------------------------------------------------------------
// Progress helpers
// ---------------------------------------------------------------------------

/// Chunk size for streaming upload progress: 256 KiB.
const UPLOAD_CHUNK_SIZE: usize = 256 * 1024;

/// Build a `reqwest::Body` that streams `data` in [`UPLOAD_CHUNK_SIZE`]-byte
/// chunks, invoking `on_progress(bytes_done, total)` after each chunk.
///
/// Using `wrap_stream` sends the request with `Transfer-Encoding: chunked`
/// unless the caller also sets `Content-Length` explicitly — the caller MUST set
/// that header when the receiving server requires a sized body (inline PUT leg).
fn upload_stream_body(
    data: &[u8],
    total: u64,
    on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
) -> reqwest::Body {
    use futures_util::stream;

    // Clone data into a Vec so it can be moved into the stream iterator.
    let owned: Vec<u8> = data.to_vec();
    let mut sent: u64 = 0;

    // Build a Vec of (chunk_bytes, cumulative_sent) pairs upfront so we can
    // produce a `stream::iter` of `Result<Bytes, _>` items without needing the
    // callback inside an async context.
    let chunks: Vec<bytes::Bytes> = owned
        .chunks(UPLOAD_CHUNK_SIZE)
        .map(bytes::Bytes::copy_from_slice)
        .collect();

    if let Some(cb) = on_progress {
        // Report 0 upfront so the bar appears immediately.
        cb(0, total);
        // Build a sequence of (chunk, cumulative_after) pairs and call the
        // callback synchronously as we assemble the stream.  The stream itself
        // is lazy; items are only produced when reqwest polls it, so the
        // callback fires in step with actual network consumption.
        let items: Vec<Result<bytes::Bytes, std::convert::Infallible>> = chunks
            .into_iter()
            .map(|chunk| {
                sent += chunk.len() as u64;
                cb(sent, total);
                Ok(chunk)
            })
            .collect();
        // NOTE: Because the stream is built eagerly here (collect), the
        // callbacks fire as soon as `upload_stream_body` is called — before
        // the network send.  For very large modules the socket buffer will
        // consume far ahead of actual transmission, so the bar completes
        // before the server acknowledges.  This is an acceptable trade-off:
        // it avoids async complexity while still showing the user the total
        // size that will be transferred.  True byte-accurate streaming
        // progress would require a custom `AsyncRead` body shim.
        reqwest::Body::wrap_stream(stream::iter(items))
    } else {
        // No progress needed — stream without any callback overhead.
        let items: Vec<Result<bytes::Bytes, std::convert::Infallible>> =
            chunks.into_iter().map(Ok).collect();
        reqwest::Body::wrap_stream(stream::iter(items))
    }
}

/// Receive a response body chunk-by-chunk, invoking `on_progress(done, total)`
/// after each received chunk. `total` should come from `Content-Length` (0 means
/// unknown). Returns the assembled bytes.
async fn download_with_progress(
    mut resp: reqwest::Response,
    total: u64,
    on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
) -> Result<Vec<u8>, ClientError> {
    // Hint capacity from Content-Length if known.
    let cap = if total > 0 {
        total as usize
    } else {
        4 * 1024 * 1024
    };
    let mut buf = Vec::with_capacity(cap);
    let mut done: u64 = 0;

    if let Some(cb) = on_progress {
        cb(0, total);
    }

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| ClientError::Transport(e.to_string()))?
    {
        done += chunk.len() as u64;
        buf.extend_from_slice(&chunk);
        if let Some(cb) = on_progress {
            cb(done, total);
        }
    }
    Ok(buf)
}

#[cfg(test)]
mod content_tests {
    use super::*;
    use digstore_core::merkle::ProofStep;

    /// The `inclusion_proof` wire field is base64 of the codec-encoded `MerkleProof`
    /// (== `MerkleProof::to_bytes`). Encode → b64 → `decode_inclusion_proof` must reproduce it
    /// byte-for-byte, including the path steps — this is the exact decode `get_content` performs.
    #[test]
    fn inclusion_proof_b64_round_trips() {
        let proof = MerkleProof {
            leaf: Bytes32([7u8; 32]),
            path: vec![
                ProofStep {
                    hash: Bytes32([1u8; 32]),
                    is_left: false,
                },
                ProofStep {
                    hash: Bytes32([2u8; 32]),
                    is_left: true,
                },
            ],
            root: Bytes32([9u8; 32]),
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(proof.to_bytes());
        let decoded = decode_inclusion_proof(&b64).unwrap();
        assert_eq!(decoded, proof);
    }

    /// An empty proof string (the server's empty-proof case) decodes to an EMPTY-input error, never
    /// a panic — `get_content` surfaces it as a Decode error, and a present-but-emptied proof would
    /// in any case fail the caller's merkle gate.
    #[test]
    fn empty_inclusion_proof_is_a_decode_error() {
        assert!(matches!(
            decode_inclusion_proof(""),
            Err(ClientError::Decode(_))
        ));
    }

    /// Non-base64 garbage is a Decode error, not a panic.
    #[test]
    fn garbage_inclusion_proof_is_a_decode_error() {
        assert!(matches!(
            decode_inclusion_proof("!!!not base64!!!"),
            Err(ClientError::Decode(_))
        ));
    }
}
