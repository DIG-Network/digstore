//! §8.5 Social Conventions — the `/.well-known/dig/manifest.json` discovery
//! manifest.
//!
//! The whitepaper (§8.5) defines two opt-in conventions whose resource keys are
//! *public by agreement* so a party that knows the store ID can construct their
//! URN, derive the retrieval key, and (on a public store) read them:
//!
//! | Convention        | Resource key                      | Purpose                                              |
//! |-------------------|-----------------------------------|------------------------------------------------------|
//! | Default resource  | `index.html`                      | the store's landing/default view                     |
//! | Discovery manifest | `/.well-known/dig/manifest.json` | machine-readable list of publisher-exposed resources, with labels and types |
//!
//! The discovery manifest is **just a normal resource**: the publisher elects
//! which resource keys to expose (with a human label and a content type), the CLI
//! serializes that list to JSON and stages it under the conventional resource
//! key, and `commit` seals/chunks/merkle-roots it exactly like any other content.
//! Reading it back is an ordinary `cat` of the conventional retrieval key — the
//! engine needs no special code. Secret-keyed resources stay opaque because
//! nothing maps a public name to them; the publisher only lists what they choose.

use serde::{Deserialize, Serialize};

/// The conventional resource key for the discovery manifest (§8.5).
///
/// The whitepaper displays the convention as `/.well-known/dig/manifest.json`,
/// where the leading `/` is the URN path separator between the store/root head
/// and the resource key (`urn:dig:<chain>:<store>[:<root>]/<resourceKey>`). The
/// resource KEY itself — the segment after that separator, and the value used in
/// both retrieval-key derivation and the on-disk key table — is therefore
/// `.well-known/dig/manifest.json` (no leading slash). A discoverer who writes
/// the displayed URN `urn:dig:chia:<store>:<root>/.well-known/dig/manifest.json`
/// parses to exactly this key, so the writer and the reader agree.
pub const DISCOVERY_RESOURCE_KEY: &str = ".well-known/dig/manifest.json";

/// One publisher-elected entry in the discovery manifest: the public resource
/// key, a human-readable label, and a media/content type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryEntry {
    /// The resource key a discoverer constructs a URN for.
    pub key: String,
    /// Human-readable label for the resource.
    pub label: String,
    /// Content/media type (e.g. `text/html`, `application/json`).
    #[serde(rename = "type")]
    pub content_type: String,
}

/// The machine-readable discovery manifest written at
/// `/.well-known/dig/manifest.json` (§8.5): a list of the resources the
/// publisher chooses to expose, each with a label and a type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryManifest {
    pub schema_version: u32,
    /// The resources the publisher elects to expose. Resources NOT listed here
    /// remain opaque (their keys are not advertised).
    pub resources: Vec<DiscoveryEntry>,
}

impl DiscoveryManifest {
    /// Build a manifest from publisher-elected `(key, label, type)` entries.
    pub fn new(entries: Vec<DiscoveryEntry>) -> Self {
        DiscoveryManifest {
            schema_version: 1,
            resources: entries,
        }
    }

    /// Deterministic, pretty JSON bytes (the resource body stored under the
    /// conventional key). Deterministic so recompiles of an unchanged manifest
    /// produce identical bytes (§19.3 spirit).
    pub fn to_json_bytes(&self) -> Vec<u8> {
        // serde_json preserves field/struct order; `resources` order is the
        // publisher's election order. Pretty for human inspection.
        serde_json::to_vec_pretty(self).expect("DiscoveryManifest is always serializable")
    }

    /// Parse manifest bytes fetched back by `cat` of the conventional key.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// Infer a conventional content type from a resource key's extension. Used to
/// auto-populate the `type` field when the publisher does not specify one.
pub fn infer_content_type(resource_key: &str) -> String {
    let lower = resource_key.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let m = DiscoveryManifest::new(vec![
            DiscoveryEntry {
                key: "index.html".into(),
                label: "Home".into(),
                content_type: "text/html".into(),
            },
            DiscoveryEntry {
                key: "data.json".into(),
                label: "Data".into(),
                content_type: "application/json".into(),
            },
        ]);
        let bytes = m.to_json_bytes();
        let back = DiscoveryManifest::from_json_bytes(&bytes).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn type_field_serializes_as_type() {
        let m = DiscoveryManifest::new(vec![DiscoveryEntry {
            key: "a".into(),
            label: "A".into(),
            content_type: "text/plain".into(),
        }]);
        let text = String::from_utf8(m.to_json_bytes()).unwrap();
        assert!(text.contains("\"type\""), "uses `type`, not `content_type`");
        assert!(!text.contains("content_type"));
    }

    #[test]
    fn serialization_is_deterministic() {
        let m = DiscoveryManifest::new(vec![DiscoveryEntry {
            key: "index.html".into(),
            label: "Home".into(),
            content_type: "text/html".into(),
        }]);
        assert_eq!(m.to_json_bytes(), m.to_json_bytes());
    }

    #[test]
    fn infers_common_content_types() {
        assert_eq!(infer_content_type("index.html"), "text/html");
        assert_eq!(infer_content_type("a/b/data.json"), "application/json");
        assert_eq!(infer_content_type("noext"), "application/octet-stream");
    }
}
