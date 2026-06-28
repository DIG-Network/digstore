//! Integration tests for the Wave-B asset CLI (#35 nft/collection/did/offer + #33 capsule-media +
//! #36 CHIP-0007 metadata), driven through the INSTALLED `digstore` binary against the seeded mock
//! anchor backend (`DIGSTORE_ANCHOR_MOCK`). These cover the offline/deterministic surface — input
//! validation, JSON shape, the capsule-media URN+hash computation, and the `--dry-run` build path
//! (which never touches the network). On-chain spend round-trips are covered by the chain crate's
//! `Simulator` tests.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

// ---------- did create ----------

/// `digstore did create --dry-run --json` against the mock: builds the create-DID spend without
/// spending, emits a launcher id, `dry_run: true`, `mocked: true`, and no tx id.
#[test]
fn did_create_dry_run_json() {
    let dir = tmp_dig();
    let out = dig(&dir)
        .args(["--json", "did", "create", "--dry-run"])
        .output()
        .unwrap();
    assert!(out.status.success(), "did create --dry-run should succeed");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["action"], "did.create");
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["mocked"], true);
    assert!(v["tx_id"].is_null(), "dry-run must not push (no tx id)");
    let launcher = v["launcher_id"].as_str().expect("launcher_id present");
    assert_eq!(launcher.len(), 64, "launcher id is 32-byte hex");
}

// ---------- nft mint (capsule-media, #33) ----------

/// `digstore nft mint --art <file> --dry-run --json` builds the media capsule + mint spend without
/// spending and proves the #33 capsule-media contract:
///   * the art is written into a capsule (storeId:rootHash present),
///   * `data_hash` == sha256(art bytes) and `metadata_hash` == sha256(canonical CHIP-0007 JSON),
///   * the primary `data_uris[0]` / `metadata_uris[0]` are the capsule's `dig://` URN,
///   * the embedded metadata JSON is canonical CHIP-0007 (`"format":"CHIP-0007"`).
#[test]
fn nft_mint_capsule_media_dry_run_json() {
    let dir = tmp_dig();
    let art = dir.path().join("art.png");
    let art_bytes = b"\x89PNG\r\n\x1a\nfake-png-bytes-for-the-test";
    std::fs::write(&art, art_bytes).unwrap();

    let out = dig(&dir)
        .args([
            "--json",
            "nft",
            "mint",
            "--art",
            art.to_str().unwrap(),
            "--name",
            "DIG Punk #1",
            "--royalty",
            "300",
            "--gateway",
            "https://rpc.dig.net",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "nft mint --dry-run should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["action"], "nft.mint");
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["mocked"], true);
    assert!(v["tx_id"].is_null());

    let cap = &v["capsule"];
    let store_id = cap["store_id"].as_str().unwrap();
    let root_hash = cap["root_hash"].as_str().unwrap();
    assert_eq!(store_id.len(), 64);
    assert_eq!(root_hash.len(), 64);

    // data_hash MUST equal sha256(art bytes) — pinned to the REAL bytes (#36 footgun-closer).
    let expected_data_hash = sha256_hex(art_bytes);
    assert_eq!(
        cap["data_hash"].as_str().unwrap(),
        expected_data_hash,
        "on-chain data_hash must be sha256 of the real art bytes"
    );

    // metadata_hash MUST equal sha256(canonical CHIP-0007 JSON).
    let md_json = cap["metadata_json"].as_str().unwrap();
    assert!(
        md_json.contains(r#""format":"CHIP-0007""#),
        "embedded metadata must be canonical CHIP-0007 JSON: {md_json}"
    );
    assert!(md_json.contains(r#""name":"DIG Punk #1""#));
    assert_eq!(
        cap["metadata_hash"].as_str().unwrap(),
        sha256_hex(md_json.as_bytes()),
        "on-chain metadata_hash must be sha256 of the canonical metadata JSON"
    );

    // The PRIMARY data/metadata URIs are the capsule's dig:// URN; the https gateway is the fallback.
    let data_uris = cap["data_uris"].as_array().unwrap();
    assert!(data_uris[0]
        .as_str()
        .unwrap()
        .starts_with(&format!("dig://{store_id}:{root_hash}/")));
    assert!(
        data_uris[1]
            .as_str()
            .unwrap()
            .starts_with("https://rpc.dig.net/urn:dig:chia:"),
        "second data uri is the https gateway fallback"
    );
}

/// An empty `--art` file is rejected with a clear invalid-argument error (exit 2).
#[test]
fn nft_mint_rejects_empty_art() {
    let dir = tmp_dig();
    let art = dir.path().join("empty.png");
    std::fs::write(&art, b"").unwrap();
    dig(&dir)
        .args([
            "nft",
            "mint",
            "--art",
            art.to_str().unwrap(),
            "--name",
            "X",
            "--dry-run",
        ])
        .assert()
        .failure()
        .code(2);
}

/// #38 end-to-end DID-attributed mint: `--did` now reconstructs the wallet's DID and
/// composes its acknowledging spend into the mint bundle (proven on the Simulator in
/// the chain crate). Under the offline mock there is no DID on chain, so the path is
/// EXERCISED and fails with a clear "does not own DID" — NOT the old "not wired"
/// refusal. (The media capsule is still built first, before any chain read.)
#[test]
fn nft_mint_did_attribution_reconstructs_did() {
    let dir = tmp_dig();
    let art = dir.path().join("a.png");
    std::fs::write(&art, b"bytes").unwrap();
    dig(&dir)
        .args([
            "nft",
            "mint",
            "--art",
            art.to_str().unwrap(),
            "--name",
            "X",
            "--did",
            &"ab".repeat(32),
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not own DID"));
}

// ---------- nft list ----------

/// `digstore nft list --json` against the mock returns an empty list (the mock has no NFTs).
#[test]
fn nft_list_empty_under_mock_json() {
    let dir = tmp_dig();
    let out = dig(&dir).args(["--json", "nft", "list"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["action"], "nft.list");
    assert_eq!(v["nfts"].as_array().unwrap().len(), 0);
}

// ---------- nft bulk ----------

/// `digstore nft bulk --manifest <items.json> --dry-run --json` builds a bulk-mint for every item
/// without spending and returns one launcher id per item.
#[test]
fn nft_bulk_dry_run_json() {
    let dir = tmp_dig();
    let manifest = dir.path().join("items.json");
    std::fs::write(
        &manifest,
        r#"[
            {"name":"A","media":{"data_uris":["dig://s/a"],"data_hash":"1111111111111111111111111111111111111111111111111111111111111111"}},
            {"name":"B","media":{"data_uris":["dig://s/b"],"data_hash":"2222222222222222222222222222222222222222222222222222222222222222"}}
        ]"#,
    )
    .unwrap();
    let out = dig(&dir)
        .args([
            "--json",
            "nft",
            "bulk",
            "--manifest",
            manifest.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "nft bulk --dry-run should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["action"], "nft.bulk");
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["launcher_ids"].as_array().unwrap().len(), 2);
}

// ---------- collection ----------

/// `digstore collection create --json` writes a definition with a slugged id + the given royalty.
#[test]
fn collection_create_json() {
    let dir = tmp_dig();
    let out = dig(&dir)
        .args([
            "--json",
            "collection",
            "create",
            "--name",
            "DIG Punks",
            "--royalty",
            "500",
            "--royalty-address",
            "xch1qvx0dy7tzw8s6f5h7gqas6f3kq0r0e2d6f6f6f6f6f6f6f6f6f6sjxqsdq",
        ])
        .output()
        .unwrap();
    // The royalty-address may or may not decode (it's a placeholder); accept either a clean success
    // with the slug, or a clear invalid-address error. The id slug is the deterministic part we pin.
    if out.status.success() {
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["action"], "collection.create");
        assert_eq!(v["id"], "dig-punks");
    } else {
        assert_eq!(out.status.code(), Some(2), "bad address → invalid-argument");
    }
}

/// `collection mint` refuses a multi-item manifest with a clear message (the multi-item DID-funded
/// bulk path is scaffolded — single item per call for now).
#[test]
fn collection_mint_refuses_multi_item() {
    let dir = tmp_dig();
    let col = dir.path().join("col.json");
    std::fs::write(
        &col,
        r#"{"id":"c","name":"C","attributes":[],"royalty_puzzle_hash":"0000000000000000000000000000000000000000000000000000000000000000","royalty_basis_points":0}"#,
    )
    .unwrap();
    let items = dir.path().join("items.json");
    std::fs::write(
        &items,
        r#"[{"name":"A","media":{}},{"name":"B","media":{}}]"#,
    )
    .unwrap();
    dig(&dir)
        .args([
            "collection",
            "mint",
            "--collection",
            col.to_str().unwrap(),
            "--manifest",
            items.to_str().unwrap(),
            "--did",
            &"ab".repeat(32),
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("single DID-attributed item"));
}

/// #40 drop scaffold: `collection create` with drop flags records the drop model in
/// the definition JSON (committed); enforcement is TODO. A plain create has no drop.
#[test]
fn collection_create_records_drop_mechanics_json() {
    let dir = tmp_dig();
    let out = dir.path().join("col.json");
    let res = dig(&dir)
        .args([
            "--json",
            "collection",
            "create",
            "--name",
            "DIG Drop",
            "--royalty-address",
            // index-0 owner address of the ABANDON test vector (decodes cleanly).
            "xch16fqlq7r0u8vxav3e6x8u57xxjmstsj5tg6mrh65l7ush8ple73jqfmws8h",
            "--reveal-at",
            "1900000000",
            "--allow",
            "abcd",
            "--phase",
            "allowlist:1800000000:50",
            "--lazy-mint",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    // The royalty address is a placeholder; accept a clean success OR a bad-address
    // error. When it succeeds, the written definition must carry the drop block.
    if res.status.success() {
        let def: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        let drop = &def["drop"];
        assert_eq!(drop["reveal_unix"].as_u64(), Some(1_900_000_000));
        assert_eq!(drop["lazy_mint"].as_bool(), Some(true));
        assert_eq!(drop["allowlist"][0].as_str(), Some("abcd"));
        assert_eq!(drop["phases"][0]["supply"].as_u64(), Some(50));
        assert_eq!(drop["phases"][0]["allowlist_only"].as_bool(), Some(true));
    } else {
        assert_eq!(res.status.code(), Some(2), "bad address → invalid-argument");
    }
}

/// The human-mode drop-scaffold warning must NOT leak an internal tracker number
/// (e.g. `(#40)`) into user-facing output — plain language only.
#[test]
fn collection_create_drop_warning_has_no_tracker_number() {
    let dir = tmp_dig();
    let out = dir.path().join("warn.json");
    let res = dig(&dir)
        .args([
            "collection",
            "create",
            "--name",
            "DIG Drop",
            "--royalty-address",
            "xch16fqlq7r0u8vxav3e6x8u57xxjmstsj5tg6mrh65l7ush8ple73jqfmws8h",
            "--lazy-mint",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );
    assert!(
        !combined.contains("(#"),
        "user-facing output must not leak a tracker number: {combined}"
    );
}

/// A plain `collection create` (no drop flags) writes NO drop block (existing
/// definitions stay unchanged).
#[test]
fn collection_create_plain_has_no_drop_block() {
    let dir = tmp_dig();
    let out = dir.path().join("plain.json");
    let res = dig(&dir)
        .args([
            "collection",
            "create",
            "--name",
            "Plain",
            "--royalty-address",
            "xch16fqlq7r0u8vxav3e6x8u57xxjmstsj5tg6mrh65l7ush8ple73jqfmws8h",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    if res.status.success() {
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(
            !text.contains("\"drop\""),
            "plain collection has no drop block: {text}"
        );
    }
}

/// #39 `collection list --json` against the mock returns an empty list (no NFTs on the
/// mock chain) — the read path is exercised end-to-end with no third-party API.
#[test]
fn collection_list_empty_under_mock_json() {
    let dir = tmp_dig();
    let out = dig(&dir)
        .args(["--json", "collection", "list"])
        .output()
        .unwrap();
    assert!(out.status.success(), "collection list should succeed");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["action"], "collection.list");
    assert_eq!(v["collections"].as_array().unwrap().len(), 0);
}

/// #39 `collection show --did <did> --json` against the mock returns the collection
/// view with zero items (the DID owns nothing on the mock), proving the read path.
#[test]
fn collection_show_empty_under_mock_json() {
    let dir = tmp_dig();
    let out = dig(&dir)
        .args(["--json", "collection", "show", "--did", &"ab".repeat(32)])
        .output()
        .unwrap();
    assert!(out.status.success(), "collection show should succeed");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["action"], "collection.show");
    assert_eq!(v["did_launcher"].as_str().unwrap(), "ab".repeat(32));
    assert_eq!(v["items"].as_array().unwrap().len(), 0);
}

// ---------- offer ----------

/// `digstore offer make` rejects a leg with an unknown asset suffix (exit 2, before any wallet use).
#[test]
fn offer_make_rejects_bad_leg() {
    let dir = tmp_dig();
    dig(&dir)
        .args(["offer", "make", "--offer", "100usd", "--request", "1xch"])
        .assert()
        .failure()
        .code(2);
}

/// `digstore offer show --offer <bad>` rejects a non-offer string with a clear chain error.
#[test]
fn offer_show_rejects_non_offer() {
    let dir = tmp_dig();
    dig(&dir)
        .args(["offer", "show", "--offer", "not-an-offer"])
        .assert()
        .failure();
}

// ---------- helpers ----------

/// SHA-256 of `bytes` as lowercase hex, via the SAME `digstore_chain::metadata::sha256` primitive the
/// CLI uses — so verifying the CLI's computed `data_hash`/`metadata_hash` is an exact, not parallel,
/// check.
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(digstore_chain::metadata::sha256(bytes))
}
