//! Well-known DIG pubkey discovery (#24): resolve an origin's (a website/hub) DIG BLS
//! pubkey from `https://<origin>/.well-known/dig/pubkey`, so `digstore
//! authorize-origin-as-writer` can add it as a CHIP-0035 writer delegate on the active
//! store — no copy-pasted key, no hub-managed secret. The wire contract (path + JSON
//! shape) is normative in `SPEC.md` and `SYSTEM.md`.
//!
//! The hub-side endpoint that SERVES this well-known resource is a SEPARATE, not-yet-built
//! surface tracked on hub's own issue — this module only fetches + parses it (mocked in
//! tests via a local server). Split into pure helpers (URL build, JSON parse, pubkey
//! decode), tested with no network, plus one async fetch fn integration-tested against a
//! local mock server.

use digstore_chain::singleton::PublicKey;

use crate::error::CliError;

/// The well-known path a DIG-aware origin serves its pubkey at (RFC 8615 `.well-known`).
pub const WELL_KNOWN_PATH: &str = "/.well-known/dig/pubkey";

/// `USER_AGENT` every digstore HTTP call sends (WAF-safe, matches the `dighub`/§21 clients).
const USER_AGENT: &str = concat!("digstore/", env!("CARGO_PKG_VERSION"));

/// Build the well-known pubkey URL for `origin`. `origin` may be a bare host
/// (`hub.dig.net`), a `host:port`, or a full `https://…`/`http://…` URL — any given scheme
/// is discarded, because the well-known lookup is ALWAYS https (a plain-http origin cannot
/// serve an authoritative pubkey). Rejects an empty origin.
pub fn pubkey_url(origin: &str) -> Result<String, CliError> {
    let trimmed = origin.trim();
    if trimmed.is_empty() {
        return Err(CliError::InvalidArgument(
            "--origin must not be empty".into(),
        ));
    }
    let host = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if host.is_empty() {
        return Err(CliError::InvalidArgument(format!(
            "not a valid origin: {origin}"
        )));
    }
    Ok(format!("https://{host}{WELL_KNOWN_PATH}"))
}

/// Extract the `pubkey` hex field from a well-known response body
/// (`{"pubkey": "<96-hex>"}`; see `SPEC.md` for the full contract).
pub fn extract_pubkey_field(body: &serde_json::Value) -> Result<String, CliError> {
    body.get("pubkey")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            CliError::Network(format!(
                "well-known response is missing a string \"pubkey\" field: {body}"
            ))
        })
}

/// Parse a 96-hex (48-byte) BLS12-381 G1 public key, tolerating a leading `0x`.
pub fn parse_pubkey_hex(hex_str: &str) -> Result<PublicKey, CliError> {
    let raw = hex::decode(hex_str.trim().trim_start_matches("0x"))
        .map_err(|e| CliError::InvalidArgument(format!("bad pubkey hex: {e}")))?;
    let arr: [u8; 48] = raw
        .as_slice()
        .try_into()
        .map_err(|_| CliError::InvalidArgument("pubkey must be 48 bytes (96 hex chars)".into()))?;
    PublicKey::from_bytes(&arr)
        .map_err(|e| CliError::InvalidArgument(format!("invalid BLS pubkey: {e}")))
}

fn http_client() -> Result<reqwest::Client, CliError> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| CliError::Network(format!("http client: {e}")))
}

/// GET `url` and extract the pubkey hex field. The lower-level fetch primitive: tests hit
/// this directly against a local mock server, since the CLI-facing [`fetch_origin_pubkey`]
/// always forces https, which a local test server can't easily present.
pub async fn fetch_pubkey(url: &str) -> Result<String, CliError> {
    let client = http_client()?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| CliError::Network(format!("GET {url}: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(CliError::Network(format!("GET {url}: HTTP {status}")));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CliError::Network(format!("GET {url}: bad JSON body: {e}")))?;
    extract_pubkey_field(&body)
}

/// Discover `origin`'s DIG pubkey (hex) via its well-known endpoint
/// (`https://<origin>/.well-known/dig/pubkey`).
pub async fn fetch_origin_pubkey(origin: &str) -> Result<String, CliError> {
    let url = pubkey_url(origin)?;
    fetch_pubkey(&url).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon art";

    #[test]
    fn pubkey_url_builds_https_well_known_path() {
        assert_eq!(
            pubkey_url("hub.dig.net").unwrap(),
            "https://hub.dig.net/.well-known/dig/pubkey"
        );
    }

    #[test]
    fn pubkey_url_strips_a_given_scheme_and_trailing_slash() {
        assert_eq!(
            pubkey_url("https://hub.dig.net/").unwrap(),
            "https://hub.dig.net/.well-known/dig/pubkey"
        );
        assert_eq!(
            pubkey_url("http://hub.dig.net").unwrap(),
            "https://hub.dig.net/.well-known/dig/pubkey"
        );
    }

    #[test]
    fn pubkey_url_rejects_empty_origin() {
        assert!(pubkey_url("").is_err());
        assert!(pubkey_url("   ").is_err());
        assert!(pubkey_url("https://").is_err());
    }

    #[test]
    fn extract_pubkey_field_reads_the_pubkey_string() {
        let body = serde_json::json!({ "pubkey": "ab".repeat(48) });
        assert_eq!(extract_pubkey_field(&body).unwrap(), "ab".repeat(48));
    }

    #[test]
    fn extract_pubkey_field_rejects_missing_field() {
        let body = serde_json::json!({ "other": 1 });
        assert!(extract_pubkey_field(&body).is_err());
    }

    #[test]
    fn extract_pubkey_field_rejects_non_string_field() {
        let body = serde_json::json!({ "pubkey": 123 });
        assert!(extract_pubkey_field(&body).is_err());
    }

    #[test]
    fn parse_pubkey_hex_accepts_0x_prefix_and_plain_hex() {
        let keys = digstore_chain::keys::derive_wallet_keys(ABANDON).unwrap();
        let hex_str = hex::encode(keys.synthetic_pk.to_bytes());

        let parsed = parse_pubkey_hex(&hex_str).unwrap();
        assert_eq!(parsed.to_bytes(), keys.synthetic_pk.to_bytes());

        let with_0x = format!("0x{hex_str}");
        let parsed2 = parse_pubkey_hex(&with_0x).unwrap();
        assert_eq!(parsed2.to_bytes(), keys.synthetic_pk.to_bytes());
    }

    #[test]
    fn parse_pubkey_hex_rejects_wrong_length() {
        assert!(parse_pubkey_hex(&"ab".repeat(20)).is_err());
    }

    #[test]
    fn parse_pubkey_hex_rejects_non_hex() {
        assert!(parse_pubkey_hex(&"zz".repeat(48)).is_err());
    }

    /// Integration-level: `fetch_pubkey` against a real (local) HTTP server proves the
    /// GET + JSON-decode + field-extraction wiring end to end. The hub's own
    /// `/.well-known/dig/pubkey` is a separate, not-yet-built surface (deferred to hub's
    /// issue) — this stands in for it with a minimal raw HTTP responder.
    #[tokio::test]
    async fn fetch_pubkey_reads_a_mocked_well_known_response() {
        let keys = digstore_chain::keys::derive_wallet_keys(ABANDON).unwrap();
        let hex_str = hex::encode(keys.synthetic_pk.to_bytes());
        let body = serde_json::json!({ "pubkey": hex_str }).to_string();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(response.as_bytes()).await.unwrap();
            sock.shutdown().await.unwrap();
        });

        let url = format!("http://{addr}{WELL_KNOWN_PATH}");
        let got = fetch_pubkey(&url).await.unwrap();
        server.await.unwrap();

        assert_eq!(got, hex_str);
    }

    /// A non-2xx well-known response surfaces as a clean [`CliError::Network`], not a panic
    /// or a silent empty pubkey.
    #[tokio::test]
    async fn fetch_pubkey_rejects_non_success_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await.unwrap();
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            sock.write_all(response.as_bytes()).await.unwrap();
            sock.shutdown().await.unwrap();
        });

        let url = format!("http://{addr}{WELL_KNOWN_PATH}");
        let err = fetch_pubkey(&url).await.unwrap_err();
        server.await.unwrap();

        assert!(matches!(err, CliError::Network(_)));
    }
}
