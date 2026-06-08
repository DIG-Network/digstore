use crate::auth::push_signing_message;
use crate::error::ClientError;
use crate::etag::parse_if_none_match;
use crate::wire::{DeltaNegotiateRequest, DeltaResponse, RootHistory, StoreDescriptor};
use digstore_core::{Bytes32, Bytes96};

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

/// HTTPS remote client: clone/fetch/pull/push (§21).
pub struct DigClient {
    base_url: String,
    http: reqwest::Client,
}

impl DigClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        DigClient {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_client(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        DigClient {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// §21.3 fetch: descriptor + root history only.
    pub async fn fetch(&self, store_id: &Bytes32) -> Result<FetchInfo, ClientError> {
        let id = store_id.to_hex();
        let d: StoreDescriptor = self
            .http
            .get(self.url(&format!("/stores/{id}")))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?
            .error_for_status()
            .map_err(|e| ClientError::Status(e.status().map(|s| s.as_u16()).unwrap_or(0)))?
            .json()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))?;
        let roots: RootHistory = self
            .http
            .get(self.url(&format!("/stores/{id}/roots")))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?
            .json()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))?;
        Ok(FetchInfo { descriptor: d, roots })
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
            .http
            .get(self.url(&format!("/stores/{id}/module")))
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
                    .http
                    .get(self.url(&format!("/stores/{id}/delta?from={from_h}&to={to_h}")))
                    .send()
                    .await
                    .map_err(|e| ClientError::Transport(e.to_string()))?;
                if resp.status().is_success() {
                    let delta: DeltaResponse = resp
                        .json()
                        .await
                        .map_err(|e| ClientError::Decode(e.to_string()))?;
                    return Ok(PullResult::Delta {
                        root: remote_root,
                        delta,
                    });
                }
                // fall through to full module on non-success delta.
            }
        }
        // full module download with conditional request.
        let mut req = self.http.get(self.url(&format!("/stores/{id}/module")));
        if let Some(lr) = local_root {
            req = req.header(reqwest::header::IF_NONE_MATCH, format!("\"{}\"", lr.to_hex()));
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

    /// §21.6 push: sign the canonical push message (CONVENTIONS C7,
    /// `SHA-256(root || store_id)`) and PUT the module. `sign` is the caller's
    /// BLS signer over the 32-byte push message.
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
        let id = store_id.to_hex();
        // CONVENTIONS C7: argument order is (root, store_id).
        let msg = push_signing_message(new_root, store_id);
        let sig = sign(&msg);
        let mut req = self
            .http
            .put(self.url(&format!("/stores/{id}/module")))
            .header("X-Dig-Parent", parent.to_hex())
            .header("X-Dig-Root", new_root.to_hex())
            .header("X-Dig-Signature", hex::encode(sig.0))
            .header("X-Dig-Push-Mode", if pending { "pending" } else { "advance" })
            .body(module.to_vec());
        if let Some(t) = bearer {
            req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        match resp.status().as_u16() {
            201 => Ok(PushResult::Advanced),
            202 => Ok(PushResult::Pending),
            401 | 403 => Err(ClientError::Unauthorized(resp.status().as_u16())),
            409 => Err(ClientError::NonFastForward),
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
            .http
            .post(self.url(&format!("/stores/{id}/delta")))
            .json(&body)
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        resp.json()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))
    }
}
