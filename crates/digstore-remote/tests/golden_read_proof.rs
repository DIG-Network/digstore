//! READ-leg golden fixture + rpc.dig.net read proof (super-repo #843).
//!
//! This is the SHARED acceptance input for the single-node "established" read
//! bar: #843 (the rpc-tier read proof) AND #1062 (the multi-node P2P e2e). Both
//! reference ONE committed golden — never a forked second copy.
//!
//! ## What the golden is
//!
//! A single-resource capsule described entirely by three inputs — a fixed
//! `store_id`, a `resource_key`, and a plaintext body — from which EVERY other
//! value is DERIVED deterministically through the one-and-only read-crypto in
//! [`digstore_core`] (the same code the producer, the host, and the browser
//! verifier all run):
//!
//! ```text
//! rootless_urn    = urn:dig:chia:<store_id>/<resource_key>
//! decryption_key  = HKDF(rootless_urn)                          (digstore_core)
//! ciphertext      = AES-256-GCM-SIV(decryption_key, plaintext)  (deterministic)
//! leaf            = resource_leaf(ciphertext) = SHA-256(ciphertext)
//! root            = MerkleTree::from_leaves([leaf]).root        (single leaf)
//! retrieval_key   = SHA-256(rootless_urn)
//! ```
//!
//! Because the KDF is a pure function and AES-256-GCM-SIV is deterministic, the
//! ciphertext / proof / root are byte-stable forever. The committed fixture
//! files under `tests/fixtures/golden/` are the §5.1 anti-regression ANCHOR: if
//! any layer of the read crypto ever drifts, [`golden_bytes_are_byte_stable`]
//! fails, and a published `.dig` under this identity would become unreadable.
//!
//! ## Regenerating (only ever to ADD a field, never to change crypto)
//!
//! `cargo test -p digstore-remote --test golden_read_proof regen -- --ignored`
//! rewrites the committed files from the inputs. Regeneration must NEVER change
//! an existing byte — that would be a §5.1 format break.

use std::path::{Path, PathBuf};

use base64::Engine;
use digstore_core::{
    decrypt_chunk, derive_decryption_key, encrypt_chunk, resource_leaf, Bytes32, Decode, Decoder,
    Encode, MerkleProof, MerkleTree, Urn,
};

// ---------------------------------------------------------------------------
// The golden identity (documented constants — the canonical acceptance input)
// ---------------------------------------------------------------------------

/// The golden `store_id`. A fixed, documented 32-byte constant standing in for a
/// CHIP-0035 DataStore launcher id. It is NOT a live mainnet store yet (see the
/// rpc-tier test's `DIG_RPC_LIVE` gate); publishing a real store under a captured
/// launcher id and re-pointing this constant is the follow-up that flips the
/// rpc-tier read proof from gated to live. Value: `SHA-256("dig-golden-read-proof-843-store")`
/// — a documented, reproducible 32-byte constant (not a hash-of-key; a launcher-id stand-in).
const GOLDEN_STORE_ID_HEX: &str =
    "d9c8ae4b6006b5d2d82ecf53f84d74e4bfe7ec4e9fde2a0d71f058b14216ff9a";

/// The LIVE mainnet golden store's `store_id` — the CHIP-0035 DataStore launcher
/// id assigned by Chia when the store was minted (super-repo #843). Unlike
/// [`GOLDEN_STORE_ID_HEX`] (a documented offline stand-in), this is a REAL,
/// on-chain, published store whose single `index.html` capsule is readable from
/// `rpc.dig.net`. Every live value (decryption key, ciphertext, root,
/// retrieval_key) derives from THIS id through the exact same [`digstore_core`]
/// read-crypto — so the rpc read proof re-derives independently and byte-compares
/// against what the gateway serves. Recorded alongside the offline anchor in
/// `tests/fixtures/golden/live_store.json` (the offline anchor is never touched).
const LIVE_STORE_ID_HEX: &str = "8c4b47f6d685e170ea663656d5cd2bdc8a1880efe5af285975e185974a7eded5";

/// The golden resource key (verbatim, not lowercased — §5.1 URN rule).
const GOLDEN_RESOURCE_KEY: &str = "index.html";

/// The golden plaintext body. Small (< 1 KiB), fixed, human-legible so a
/// byte-comparison failure is obvious.
const GOLDEN_PLAINTEXT: &[u8] =
    b"<!doctype html>\n<title>DIG golden read fixture</title>\n<h1>hello from the DIG Network</h1>\n\
      <p>This is the #843 golden capsule: a single index.html resource whose ciphertext, \
      inclusion proof, and root are derived deterministically from digstore_core.</p>\n";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("golden")
}

// ---------------------------------------------------------------------------
// Deterministic derivation (the single source of truth for every golden value)
// ---------------------------------------------------------------------------

/// The rootless canonical URN for a given store id — the one the retrieval/
/// decryption keys derive from (the root is dropped so the keys survive
/// generations, per the format skill).
fn rootless_urn_for(store_id_hex: &str) -> Urn {
    Urn {
        chain: "chia".to_string(),
        store_id: Bytes32::from_hex(store_id_hex).expect("valid store_id hex"),
        root_hash: None,
        resource_key: Some(GOLDEN_RESOURCE_KEY.to_string()),
    }
}

/// The offline stand-in's rootless URN (the deterministic crypto anchor).
fn rootless_urn() -> Urn {
    rootless_urn_for(GOLDEN_STORE_ID_HEX)
}

/// Everything a reader needs to verify + decrypt the golden, derived purely from
/// the documented inputs. This is the exact wire shape `dig.getContent` returns
/// (`ciphertext` + `inclusion_proof` + `root`) plus the `retrieval_key` the
/// caller looks the resource up by.
struct Golden {
    store_id: Bytes32,
    retrieval_key: Bytes32,
    ciphertext: Vec<u8>,
    proof: MerkleProof,
    root: Bytes32,
}

/// Derive the full golden verification tuple for an arbitrary store id from the
/// documented inputs — the single source of truth shared by the offline anchor
/// (with [`GOLDEN_STORE_ID_HEX`]) and the live rpc proof (with
/// [`LIVE_STORE_ID_HEX`]).
fn derive_for(store_id_hex: &str) -> Golden {
    let urn = rootless_urn_for(store_id_hex);
    let canonical = urn.canonical();

    let key = derive_decryption_key(&canonical, None);
    let ciphertext = encrypt_chunk(&key, GOLDEN_PLAINTEXT);

    // Single-resource capsule: the merkle leaf layer is one D5 resource leaf.
    let leaf = resource_leaf(&ciphertext);
    let tree = MerkleTree::from_leaves(vec![leaf]);
    let proof = tree.prove(0).expect("single-leaf proof");
    let root = proof.root;

    Golden {
        store_id: urn.store_id,
        retrieval_key: urn.retrieval_key(),
        ciphertext,
        proof,
        root,
    }
}

/// The offline stand-in golden (the deterministic §5.1 crypto anchor).
fn derive_golden() -> Golden {
    derive_for(GOLDEN_STORE_ID_HEX)
}

fn proof_to_b64(proof: &MerkleProof) -> String {
    base64::engine::general_purpose::STANDARD.encode(proof.to_bytes())
}

fn proof_from_b64(b64: &str) -> MerkleProof {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .expect("valid base64 proof");
    let mut dec = Decoder::new(&bytes);
    MerkleProof::decode(&mut dec).expect("decodable proof")
}

// ---------------------------------------------------------------------------
// Regeneration (ignored; run explicitly to ADD a field — never to change crypto)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "regeneration helper — run with --ignored to (re)write the committed golden files"]
fn regen_golden_fixtures() {
    let g = derive_golden();
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(dir.join("plaintext.bin"), GOLDEN_PLAINTEXT).unwrap();
    std::fs::write(dir.join("ciphertext.bin"), &g.ciphertext).unwrap();
    std::fs::write(dir.join("inclusion_proof.b64"), proof_to_b64(&g.proof)).unwrap();

    let pinned_urn = {
        let mut u = rootless_urn();
        u.root_hash = Some(g.root);
        u.canonical()
    };
    let manifest = serde_json::json!({
        "description": "DIG Network #843 golden read fixture — one index.html capsule, \
                        derived deterministically from digstore_core. Shared by #843 (rpc read \
                        proof) and #1062 (P2P e2e). Do not fork a second copy.",
        "store_id": g.store_id.to_hex(),
        "resource_key": GOLDEN_RESOURCE_KEY,
        "root": g.root.to_hex(),
        "retrieval_key": g.retrieval_key.to_hex(),
        "canonical_urn_rootless": rootless_urn().canonical(),
        "canonical_urn_pinned": pinned_urn,
        "plaintext_len": GOLDEN_PLAINTEXT.len(),
        "ciphertext_len": g.ciphertext.len(),
        "live_at_rpc_dig_net": false,
        "publish_followup": "Publish a real store under a captured launcher id, re-point \
                             GOLDEN_STORE_ID_HEX, then un-gate rpc_tier_read_proof (set DIG_RPC_LIVE).",
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap() + "\n",
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// §5.1 anchor: the committed golden decodes byte-identically, forever
// ---------------------------------------------------------------------------

/// The committed ciphertext + proof MUST equal what today's crypto derives from
/// the documented inputs. A drift in the KDF, the AEAD, the leaf binding, or the
/// merkle fold breaks this — which is exactly the §5.1 regression we must catch,
/// because a published `.dig` under this identity would stop decoding.
#[test]
fn golden_bytes_are_byte_stable() {
    let g = derive_golden();
    let dir = fixtures_dir();

    let committed_ct = std::fs::read(dir.join("ciphertext.bin")).expect("committed ciphertext");
    assert_eq!(
        committed_ct, g.ciphertext,
        "golden ciphertext drifted — a read-crypto change would brick published .dig content (§5.1)"
    );

    let committed_proof =
        std::fs::read_to_string(dir.join("inclusion_proof.b64")).expect("committed proof");
    assert_eq!(
        proof_from_b64(&committed_proof),
        g.proof,
        "golden inclusion proof drifted (§5.1)"
    );

    let committed_pt = std::fs::read(dir.join("plaintext.bin")).expect("committed plaintext");
    assert_eq!(committed_pt, GOLDEN_PLAINTEXT, "golden plaintext drifted");
}

/// The manifest's documented `store_id` + `root` MUST match the derivation, so
/// #1431 `verify_pinned_root` and any per-range merkle path can assert against
/// the same identity the manifest advertises to #1062.
#[test]
fn golden_manifest_identity_matches_derivation() {
    let g = derive_golden();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures_dir().join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["store_id"], g.store_id.to_hex());
    assert_eq!(manifest["root"], g.root.to_hex());
    assert_eq!(manifest["retrieval_key"], g.retrieval_key.to_hex());
}

// ---------------------------------------------------------------------------
// The verify-then-decrypt contract every read path (rpc / local / P2P) applies
// ---------------------------------------------------------------------------

/// The full read gate over the COMMITTED golden bytes, exactly as a client does
/// after fetching from any tier: (1) the served ciphertext's resource leaf equals
/// the proof leaf, (2) the proof recomputes the trusted root, (3) decryption
/// yields the golden plaintext byte-for-byte. Fail-closed at every step.
#[test]
fn committed_golden_verifies_then_decrypts() {
    let dir = fixtures_dir();
    let ciphertext = std::fs::read(dir.join("ciphertext.bin")).unwrap();
    let proof = proof_from_b64(&std::fs::read_to_string(dir.join("inclusion_proof.b64")).unwrap());
    let plaintext = std::fs::read(dir.join("plaintext.bin")).unwrap();

    // (1) content binds to the leaf the proof commits to.
    assert_eq!(
        resource_leaf(&ciphertext),
        proof.leaf,
        "served ciphertext must hash to the proof leaf"
    );
    // (2) the proof recomputes the (trusted) root.
    assert!(proof.verify(), "inclusion proof must verify to its root");

    // (3) decrypt with the URN-derived key → the golden plaintext.
    let key = derive_decryption_key(&rootless_urn().canonical(), None);
    let opened = decrypt_chunk(&key, &ciphertext).expect("GCM-SIV tag must verify");
    assert_eq!(
        opened, plaintext,
        "decrypted golden must equal the plaintext"
    );
}

/// A tampered ciphertext MUST fail the read gate — proving the golden's proof is
/// a real integrity check, not a rubber stamp.
#[test]
fn tampered_golden_fails_the_gate() {
    let mut ciphertext = std::fs::read(fixtures_dir().join("ciphertext.bin")).unwrap();
    ciphertext[0] ^= 0xFF;
    let proof = proof_from_b64(
        &std::fs::read_to_string(fixtures_dir().join("inclusion_proof.b64")).unwrap(),
    );
    assert_ne!(
        resource_leaf(&ciphertext),
        proof.leaf,
        "tampered ciphertext must not match the committed proof leaf"
    );
}

// ---------------------------------------------------------------------------
// The rpc-tier read proof (#843) — live against rpc.dig.net when a store exists
// ---------------------------------------------------------------------------

/// Read the LIVE golden URN from `rpc.dig.net` via `dig.getContent`, verify the
/// returned inclusion proof against the derived root, decrypt, and byte-compare to
/// the golden plaintext — the definition of "a dig-node can read content" over
/// the public gateway (the §5.3 final fallback tier).
///
/// This runs against the REAL mainnet-published store [`LIVE_STORE_ID_HEX`]
/// (super-repo #843): a public single-`index.html` capsule minted + committed +
/// pushed to rpc.dig.net. Every value the read is checked against
/// ([`derive_for`]`(LIVE_STORE_ID_HEX)`) is re-derived independently through the
/// same [`digstore_core`] crypto, so a PASS proves the gateway serves exactly the
/// bytes the deterministic contract predicts.
///
/// GATED behind `DIG_RPC_LIVE` so CI (which has no network) skips it; run it with
/// `DIG_RPC_LIVE=1 cargo test -p digstore-remote --test golden_read_proof
/// rpc_tier_read_proof`. The offline anchor tests above cover the crypto in CI.
#[tokio::test]
async fn rpc_tier_read_proof() {
    if std::env::var("DIG_RPC_LIVE").is_err() {
        eprintln!(
            "skipping rpc_tier_read_proof: set DIG_RPC_LIVE=1 to read the live golden store \
             from rpc.dig.net (network-gated; the offline anchor tests cover the crypto)"
        );
        return;
    }

    let base = std::env::var("DIG_RPC_URL").unwrap_or_else(|_| "https://rpc.dig.net".to_string());
    let client = digstore_remote::DigClient::new(base);

    // The live store: every value derived independently from LIVE_STORE_ID_HEX.
    let g = derive_for(LIVE_STORE_ID_HEX);
    let resp = client
        .get_content(&g.store_id, &g.retrieval_key, Some(&g.root))
        .await
        .expect("dig.getContent must succeed for the published live golden store");

    // Verify-then-decrypt against the derived (chain-anchored) root — the read must
    // fail closed if the gateway served a wrong tree.
    assert_eq!(
        resp.roothash, g.root,
        "served root must equal the derived live root"
    );
    let mut proof = resp.merkle_proof;
    assert_eq!(
        resource_leaf(&resp.ciphertext),
        proof.leaf,
        "served ciphertext must hash to the proof leaf"
    );
    proof.root = g.root; // pin verification to the trusted root
    assert!(proof.verify(), "served proof must verify to the live root");

    let key = derive_decryption_key(&rootless_urn_for(LIVE_STORE_ID_HEX).canonical(), None);
    let opened = decrypt_chunk(&key, &resp.ciphertext).expect("GCM-SIV tag must verify");
    assert_eq!(
        opened, GOLDEN_PLAINTEXT,
        "rpc.dig.net read must byte-match the golden plaintext"
    );
}

/// The live store's identity recorded in `live_store.json` MUST match what the
/// deterministic crypto derives from [`LIVE_STORE_ID_HEX`] — so the manifest the
/// #1062 e2e / #1473 pinned-root work reads advertises the same store_id / root /
/// retrieval_key the read path recomputes. Runs offline (no network), always.
#[test]
fn live_store_manifest_matches_derivation() {
    let g = derive_for(LIVE_STORE_ID_HEX);
    let live: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures_dir().join("live_store.json")).unwrap())
            .unwrap();
    assert_eq!(live["status"], "live");
    assert_eq!(live["store_id"], g.store_id.to_hex());
    assert_eq!(live["root"], g.root.to_hex());
    assert_eq!(live["retrieval_key"], g.retrieval_key.to_hex());
}

// ---------------------------------------------------------------------------
// STUB — local-tier / rootless-resolve read leg (connect-lane)
// ---------------------------------------------------------------------------

/// PENDING SEAM (do not delete): the local-tier read proof — `dign open` against
/// the local node's `/s/<store>:<root>/<key>` GET plus the rootless `X-Dig-Root`
/// resolve path — belongs here alongside the rpc-tier proof, but it depends on
/// the connect-lane dig-node work consuming #1431 `verify_pinned_root` (dig-node
/// #1439 / #747 / #852). It is intentionally left as a documented stub so the
/// seam is visible and #1062's e2e harness knows where the local-tier assertion
/// lands. Track: super-repo #843 (local-tier leg) + #1062.
#[test]
#[ignore = "pending connect-lane dig-node #1439/#747/#852 (verify_pinned_root consumption)"]
fn local_tier_read_proof_stub() {
    // Intentionally empty: the local /s GET + rootless X-Dig-Root resolve proof
    // is implemented once the local node exposes the pinned-root read path.
}
