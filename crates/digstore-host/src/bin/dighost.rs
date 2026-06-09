//! `dighost` — the runnable, S3-compatible Digstore host (Artifact 3).
//!
//! `dighost` is the neutral DIG-Node pipe (paper §15, §18). It loads a compiled
//! `{storeID}-{root}.wasm` module from a **storage source** (a local path, an
//! `s3://bucket/key` URL, or any S3-compatible endpoint), instantiates it via
//! `digstore_host::HostRuntime`, and on a request carrying a **32-byte retrieval
//! key** calls `serve_content` and **streams the served bytes**
//! (`ContentResponse` = ciphertext + merkle proof, or an indistinguishable
//! decoy) to stdout or `--out`. The host NEVER decrypts and never holds a URN —
//! provider blindness is structural.
//!
//! Storage is abstracted with the `object_store` crate so the same code path
//! runs against AWS S3, any S3-compatible endpoint (MinIO/R2/Wasabi), the local
//! filesystem, or an in-memory store (tests).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use digstore_core::types::{Bytes32, Bytes48};
use digstore_core::Urn;
use digstore_crypto::bls::BlsSecretKey;
use digstore_host::{serve_blind, BlindServeConfig};
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;

/// A resolved `object_store` source plus the key (path) to fetch.
pub struct ResolvedStore {
    pub store: Arc<dyn ObjectStore>,
    pub key: ObjPath,
}

/// Parse an `s3://bucket/key` URL into `(bucket, key)`.
///
/// The host only ever uses the bucket + key to fetch module bytes; it routes
/// through the [`AmazonS3Builder`] so AWS and any S3-compatible endpoint share
/// one path.
pub fn parse_s3_url(url: &str) -> Result<(String, String)> {
    let rest = url
        .strip_prefix("s3://")
        .ok_or_else(|| anyhow!("not an s3:// URL: {url}"))?;
    let (bucket, key) = rest
        .split_once('/')
        .ok_or_else(|| anyhow!("s3 URL missing key: {url} (expected s3://bucket/key)"))?;
    if bucket.is_empty() {
        bail!("s3 URL missing bucket: {url}");
    }
    if key.is_empty() {
        bail!("s3 URL missing key: {url}");
    }
    Ok((bucket.to_string(), key.to_string()))
}

/// Build an [`AmazonS3Builder`] for an `s3://bucket/key` URL, applying optional
/// S3-compatible endpoint / region / allow-http overrides. Credentials come from
/// the standard AWS env/instance chain unless an endpoint is supplied.
///
/// Returns the configured builder and the object key, so the URL parsing +
/// builder construction are testable without a live bucket.
pub fn build_s3(
    url: &str,
    endpoint: Option<&str>,
    region: Option<&str>,
    allow_http: bool,
) -> Result<(AmazonS3Builder, String)> {
    let (bucket, key) = parse_s3_url(url)?;
    let mut builder = AmazonS3Builder::from_env().with_bucket_name(&bucket);
    if let Some(ep) = endpoint {
        builder = builder.with_endpoint(ep).with_virtual_hosted_style_request(false);
    }
    if let Some(r) = region {
        builder = builder.with_region(r);
    }
    if allow_http {
        builder = builder.with_allow_http(true);
    }
    Ok((builder, key))
}

/// Resolve the `--module` argument into an [`ObjectStore`] + key. Supports
/// `s3://bucket/key` (AWS or any S3-compatible endpoint) and local filesystem
/// paths.
pub fn resolve_module_source(
    module: &str,
    endpoint: Option<&str>,
    region: Option<&str>,
    allow_http: bool,
) -> Result<ResolvedStore> {
    if module.starts_with("s3://") {
        let (builder, key) = build_s3(module, endpoint, region, allow_http)?;
        let store = builder
            .build()
            .context("building AmazonS3 object store from --module")?;
        Ok(ResolvedStore {
            store: Arc::new(store),
            key: ObjPath::from(key),
        })
    } else {
        // Local filesystem: split into a parent root + the file name so the
        // object_store key is relative to a real directory root.
        let path = PathBuf::from(module);
        let abs = std::fs::canonicalize(&path)
            .with_context(|| format!("module path not found: {module}"))?;
        let parent = abs
            .parent()
            .ok_or_else(|| anyhow!("module path has no parent dir: {module}"))?;
        let file_name = abs
            .file_name()
            .ok_or_else(|| anyhow!("module path has no file name: {module}"))?
            .to_string_lossy()
            .to_string();
        let store = LocalFileSystem::new_with_prefix(parent)
            .context("building LocalFileSystem object store")?;
        Ok(ResolvedStore {
            store: Arc::new(store),
            key: ObjPath::from(file_name),
        })
    }
}

/// Fetch the full object bytes for `key` from `store` (async object_store GET).
pub async fn fetch_bytes(store: &dyn ObjectStore, key: &ObjPath) -> Result<Vec<u8>> {
    let get = store
        .get(key)
        .await
        .with_context(|| format!("object_store GET failed for key {key}"))?;
    let bytes = get
        .bytes()
        .await
        .context("reading object bytes")?;
    Ok(bytes.to_vec())
}

/// Parse a `--host-key` argument: either raw 64-hex (32-byte seed) or a path to a
/// `signing_key.bin` file (32 raw bytes, as written by `digstore init`).
pub fn parse_host_key_seed(host_key: &str) -> Result<[u8; 32]> {
    // Try hex first.
    if host_key.len() == 64 {
        if let Ok(bytes) = hex::decode(host_key) {
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&bytes);
            return Ok(seed);
        }
    }
    // Otherwise treat it as a file path (e.g. .dig/signing_key.bin).
    let bytes = std::fs::read(host_key)
        .with_context(|| format!("--host-key is neither 64-hex nor a readable file: {host_key}"))?;
    if bytes.len() != 32 {
        bail!(
            "--host-key file {host_key} must contain a 32-byte seed (got {})",
            bytes.len()
        );
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    Ok(seed)
}

/// Parse a 64-hex retrieval key into 32 bytes.
pub fn parse_retrieval_key(hexstr: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hexstr).context("--retrieval-key must be 64-hex")?;
    if bytes.len() != 32 {
        bail!("--retrieval-key must be 32 bytes (64 hex chars), got {}", bytes.len());
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&bytes);
    Ok(k)
}

/// Inputs needed to perform a blind serve, resolved from the CLI args.
pub struct ServePlan {
    pub module_bytes: Vec<u8>,
    pub store_id: Bytes32,
    pub seed: [u8; 32],
    pub retrieval_key: [u8; 32],
}

impl ServePlan {
    /// Run the blind serve: instantiate the module and stream its self-served
    /// bytes. The host derives nothing about the content — it only forwards a
    /// 32-byte retrieval key and returns the module's verbatim output.
    pub fn serve(self) -> Result<Vec<u8>> {
        let cfg = BlindServeConfig::from_seed(self.store_id, &self.seed);
        // Sanity: surface the host's trusted public key in stderr for operators.
        let pk: Bytes48 = BlsSecretKey::from_seed(&self.seed).public_key().to_bytes();
        eprintln!("[dighost] host BLS pubkey = {}", pk.to_hex());
        let out = serve_blind(&self.module_bytes, &self.retrieval_key, cfg)
            .map_err(|e| anyhow!("serve_content failed: {e:?}"))?;
        if out.is_empty() {
            bail!("module returned empty bytes (not self-serving)");
        }
        Ok(out)
    }
}

#[derive(Parser, Debug)]
#[command(name = "dighost", about = "S3-compatible Digstore host (Artifact 3)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
enum Cmd {
    /// Serve content by retrieval key from a module in an object store.
    Serve(ServeArgs),
}

#[derive(clap::Args, Debug)]
struct ServeArgs {
    /// Module location: a local path or `s3://bucket/key`.
    #[arg(long)]
    module: String,

    /// S3-compatible endpoint URL (MinIO/R2/Wasabi). Implies S3 routing.
    #[arg(long)]
    endpoint: Option<String>,

    /// S3 region.
    #[arg(long)]
    region: Option<String>,

    /// Allow plain HTTP to the S3 endpoint (e.g. local MinIO).
    #[arg(long, default_value_t = false)]
    allow_http: bool,

    /// Host signing key: 64-hex seed OR a path to `signing_key.bin`.
    #[arg(long)]
    host_key: String,

    /// Retrieval key (64-hex, 32 bytes). Mutually exclusive with --urn.
    #[arg(long)]
    retrieval_key: Option<String>,

    /// URN (`urn:dig:...`); the retrieval key is derived LOCALLY for convenience.
    /// The host still only ever uses the 32-byte hash.
    #[arg(long)]
    urn: Option<String>,

    /// Trusted root (64-hex) — informational; verification is a client step.
    #[arg(long)]
    root: Option<String>,

    /// Write served bytes here instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
}

/// Derive the 32-byte retrieval key + store id from the CLI args. With `--urn`
/// the retrieval key is derived locally via the ROOT-INDEPENDENT canonical URN
/// (matching the compiler's stored key); with `--retrieval-key` the raw hash is
/// used directly. Returns `(store_id, retrieval_key)`.
fn resolve_retrieval(args: &ServeArgs) -> Result<(Bytes32, [u8; 32])> {
    match (&args.urn, &args.retrieval_key) {
        (Some(urn_str), _) => {
            let urn = Urn::parse(urn_str).map_err(|e| anyhow!("parse --urn: {e:?}"))?;
            // Root-INDEPENDENT canonical URN (drop the root) — this is the
            // `static_key` the compiler stored at commit time.
            let canonical = Urn {
                chain: urn.chain.clone(),
                store_id: urn.store_id,
                root_hash: None,
                resource_key: urn.resource_key.clone(),
            };
            Ok((urn.store_id, canonical.retrieval_key().0))
        }
        (None, Some(rk)) => {
            // Need a store_id for HostDeps; --urn is the only carrier of it, so
            // require --root-less callers to also be fine with a derived id from
            // the module is not possible. Use a zeroed store_id: the embedded
            // guest uses its own injected store id, and HostDeps.store_id only
            // feeds get_store_id (not the serve gate). Accept an optional --root
            // does not carry store id either, so zero is correct here.
            let key = parse_retrieval_key(rk)?;
            Ok((Bytes32([0u8; 32]), key))
        }
        (None, None) => bail!("provide either --retrieval-key <64hex> or --urn <urn:dig:...>"),
    }
}

fn run(args: ServeArgs) -> Result<()> {
    let seed = parse_host_key_seed(&args.host_key)?;
    let (store_id, retrieval_key) = resolve_retrieval(&args)?;

    // Async object_store fetch on the tokio runtime, then sync wasmtime serve.
    let resolved = resolve_module_source(
        &args.module,
        args.endpoint.as_deref(),
        args.region.as_deref(),
        args.allow_http,
    )?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    let module_bytes =
        rt.block_on(async { fetch_bytes(resolved.store.as_ref(), &resolved.key).await })?;
    eprintln!(
        "[dighost] loaded module: {} bytes from {}",
        module_bytes.len(),
        args.module
    );
    if module_bytes.len() < 4 || &module_bytes[0..4] != b"\0asm" {
        bail!("fetched object is not a wasm module (bad magic)");
    }

    let plan = ServePlan {
        module_bytes,
        store_id,
        seed,
        retrieval_key,
    };
    // wasmtime is sync; run it on a blocking thread off the async runtime.
    let served = rt.block_on(async {
        tokio::task::spawn_blocking(move || plan.serve())
            .await
            .map_err(|e| anyhow!("serve task join error: {e}"))?
    })?;

    eprintln!("[dighost] served {} bytes (ContentResponse envelope)", served.len());

    match &args.out {
        Some(path) => {
            std::fs::write(path, &served)
                .with_context(|| format!("writing --out {}", path.display()))?;
            eprintln!("[dighost] wrote {} bytes to {}", served.len(), path.display());
        }
        None => {
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            lock.write_all(&served).context("streaming to stdout")?;
            lock.flush().ok();
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve(args) => run(args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_s3_url() {
        let (b, k) = parse_s3_url("s3://my-bucket/path/to/module.wasm").unwrap();
        assert_eq!(b, "my-bucket");
        assert_eq!(k, "path/to/module.wasm");
    }

    #[test]
    fn rejects_non_s3_url() {
        assert!(parse_s3_url("https://x/y").is_err());
        assert!(parse_s3_url("s3://only-bucket").is_err());
        assert!(parse_s3_url("s3:///key-no-bucket").is_err());
    }

    #[test]
    fn s3_url_routes_to_amazon_builder() {
        // URL parsing + builder construction without a live bucket.
        let (builder, key) = build_s3(
            "s3://store-bucket/abc-deadbeef.wasm",
            Some("http://127.0.0.1:9000"),
            Some("us-east-1"),
            true,
        )
        .unwrap();
        assert_eq!(key, "abc-deadbeef.wasm");
        // The builder is configured (build() would need creds; we only assert it
        // constructs and that an endpoint-backed build succeeds offline).
        let store = builder.build().expect("AmazonS3 builds with endpoint override");
        let _ = store; // routing confirmed: an AmazonS3 store was produced.
    }

    #[test]
    fn parses_hex_host_key() {
        let seed = parse_host_key_seed(&"ab".repeat(32)).unwrap();
        assert_eq!(seed, [0xabu8; 32]);
    }

    #[test]
    fn parses_retrieval_key_hex() {
        let k = parse_retrieval_key(&"cd".repeat(32)).unwrap();
        assert_eq!(k, [0xcdu8; 32]);
        assert!(parse_retrieval_key("zz").is_err());
    }
}
