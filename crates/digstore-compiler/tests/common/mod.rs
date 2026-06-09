#![allow(dead_code)]

use digstore_compiler::{GenerationView, ResourceView};
use digstore_core::{Bytes32, Bytes48, MetadataManifest, TrustedHostKey};
use sha2::{Digest, Sha256};

/// A single resource's contribution to a synthetic generation.
pub struct ResourceSpec {
    pub resource_key: Bytes32,
    /// (chunk_hash, chunk_body) in resource order.
    pub chunks: Vec<(Bytes32, Vec<u8>)>,
}

/// Minimal in-memory stand-in for a loaded generation consumed by the compiler
/// via the `GenerationView` trait.
pub struct FakeGeneration {
    pub root: Bytes32,
    pub generation_id: u64,
    pub resources: Vec<ResourceSpec>,
}

pub fn chunk(body: &[u8]) -> (Bytes32, Vec<u8>) {
    let mut h = Sha256::new();
    h.update(body);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    (Bytes32(out), body.to_vec())
}

pub fn resource_key(name: &str) -> Bytes32 {
    let mut h = Sha256::new();
    h.update(name.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    Bytes32(out)
}

/// Two generations sharing one chunk, for dedup + key-table tests.
pub fn sample_generations() -> Vec<FakeGeneration> {
    let shared = chunk(b"shared-chunk-body-0000");
    let a = chunk(b"alpha-body-1111");
    let b = chunk(b"beta-body-2222");
    vec![
        FakeGeneration {
            root: Bytes32([0x11; 32]),
            generation_id: 1,
            resources: vec![ResourceSpec {
                resource_key: resource_key("index.html"),
                chunks: vec![shared.clone(), a.clone()],
            }],
        },
        FakeGeneration {
            root: Bytes32([0x22; 32]),
            generation_id: 2,
            resources: vec![ResourceSpec {
                resource_key: resource_key("about.html"),
                chunks: vec![shared, b],
            }],
        },
    ]
}

/// The trusted host key embedded in fixture modules. §12.2: the guest verifies
/// the host's attestation signature against this set, so the embedded key MUST
/// be the public half of the key the test host signs with
/// (`BlsSecretKey::from_seed(&[42u8; 32])`, see `self_serving.rs::host_deps`).
/// A placeholder key here would make every real hit (correctly) serve a decoy.
pub fn trusted_keys() -> Vec<TrustedHostKey> {
    let sk = digstore_crypto::bls::BlsSecretKey::from_seed(&[42u8; 32]);
    let pk = sk.public_key().to_bytes().0;
    vec![TrustedHostKey {
        public_key: pk,
        label: format!("dig-host-key-v1:{}", hex::encode(pk)),
    }]
}

pub fn sample_manifest() -> MetadataManifest {
    MetadataManifest {
        schema_version: 1,
        name: "sample-store".to_string(),
        version: Some("1.0.0".to_string()),
        description: Some("fixture".to_string()),
        authors: vec![],
        license: None,
        homepage: None,
        repository: None,
        keywords: vec![],
        categories: vec![],
        icon: None,
        content_type: None,
        links: Default::default(),
        custom: Default::default(),
    }
}

pub fn store_id() -> Bytes32 {
    Bytes32([0xAB; 32])
}

pub fn store_pubkey() -> Bytes48 {
    Bytes48([0xCD; 48])
}

// ---- trait impls so fixtures plug into the compiler pipeline ----

impl ResourceView for ResourceSpec {
    fn resource_key(&self) -> Bytes32 {
        self.resource_key
    }
    fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)> {
        self.chunks.clone()
    }
}

/// Borrowing adapter so `GenerationView::resources` can hand out trait objects.
pub struct ResourceSpecRef<'a>(pub &'a ResourceSpec);

impl<'a> ResourceView for ResourceSpecRef<'a> {
    fn resource_key(&self) -> Bytes32 {
        self.0.resource_key
    }
    fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)> {
        self.0.chunks.clone()
    }
}

impl GenerationView for FakeGeneration {
    fn root(&self) -> Bytes32 {
        self.root
    }
    fn resources(&self) -> Vec<Box<dyn ResourceView + '_>> {
        self.resources
            .iter()
            .map(|r| Box::new(ResourceSpecRef(r)) as Box<dyn ResourceView + '_>)
            .collect()
    }
}
