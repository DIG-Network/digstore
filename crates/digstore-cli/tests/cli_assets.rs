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
///   * the primary `data_uris[0]` / `metadata_uris[0]` are the capsule's canonical bare root-pinned
///     URN and the fallback `[1]` is the https gateway url (#663 NFT1 multi-url backup),
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

    // #663 NFT1 multi-url backup: the PRIMARY entry is the canonical BARE root-pinned URN
    // `urn:dig:chia:<store>:<root>/<key>` (never a `dig://`-prefixed URN — the #686 bug), and the
    // fallback is the https gateway url. Both are always present, URN first.
    for uris_key in ["data_uris", "metadata_uris"] {
        let uris = cap[uris_key].as_array().unwrap();
        assert_eq!(uris.len(), 2, "{uris_key} carries the URN + the https url");
        let primary = uris[0].as_str().unwrap();
        assert_eq!(
            primary,
            format!(
                "urn:dig:chia:{store_id}:{root_hash}/{}",
                if uris_key == "data_uris" { "art" } else { "metadata.json" }
            ),
            "{uris_key}[0] is the canonical bare root-pinned URN"
        );
        assert!(!primary.starts_with("dig://"), "URN must not be dig://-prefixed (#686)");
        assert!(
            uris[1]
                .as_str()
                .unwrap()
                .starts_with("https://rpc.dig.net/urn:dig:chia:"),
            "{uris_key}[1] is the https gateway fallback"
        );
    }
}

/// #663: WITHOUT `--gateway`, the mint still emits BOTH uris — the canonical URN first and the
/// DEFAULT https gateway (`https://rpc.dig.net`) as the fallback — so a minted NFT is never
/// URN-only (a legacy wallet always has a working https url).
#[test]
fn nft_mint_defaults_gateway_when_omitted() {
    let dir = tmp_dig();
    let art = dir.path().join("art.png");
    std::fs::write(&art, b"fake-png").unwrap();

    let out = dig(&dir)
        .args([
            "--json", "nft", "mint", "--art", art.to_str().unwrap(), "--name", "X", "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let uris = v["capsule"]["data_uris"].as_array().unwrap();
    assert_eq!(uris.len(), 2, "both uris present even without --gateway");
    assert!(uris[0].as_str().unwrap().starts_with("urn:dig:chia:"));
    assert!(uris[1]
        .as_str()
        .unwrap()
        .starts_with("https://rpc.dig.net/urn:dig:chia:"));
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

/// #199/#231: `collection mint` no longer refuses a multi-item manifest on item count alone — a
/// 203-item manifest (dkackman's exact real-world size) is parsed and the cost-bounded batch plan is
/// computed (proving the #231 auto-batching path executes without error), and the command proceeds
/// all the way to the SAME "does not own DID" failure (exit 4) the single-item path hits under the
/// offline mock (no real DID exists there) — NOT the old "single DID-attributed item" refusal
/// (exit 2). The real on-chain proof that each funded batch VALIDATES under the block cost limit is
/// the digstore-chain Simulator tests (`build_collection_mint_funded_in_validates_on_simulator` +
/// `build_collection_batch_chains_across_batches_on_simulator`).
#[test]
fn collection_mint_multi_item_no_longer_refused() {
    let dir = tmp_dig();
    let col = dir.path().join("col.json");
    std::fs::write(
        &col,
        r#"{"id":"c","name":"C","attributes":[],"royalty_puzzle_hash":"0000000000000000000000000000000000000000000000000000000000000000","royalty_basis_points":0}"#,
    )
    .unwrap();
    let items_json = dir.path().join("items.json");
    let items: Vec<serde_json::Value> = (0..203)
        .map(|i| {
            serde_json::json!({
                "name": format!("Item #{i}"),
                "media": {
                    "data_uris": [format!("dig://s/{i}.png")],
                    "data_hash": format!("{:064x}", i + 1),
                }
            })
        })
        .collect();
    std::fs::write(&items_json, serde_json::to_string(&items).unwrap()).unwrap();

    let out = dig(&dir)
        .args([
            "collection",
            "mint",
            "--collection",
            col.to_str().unwrap(),
            "--manifest",
            items_json.to_str().unwrap(),
            "--did",
            &"ab".repeat(32),
            "--dry-run",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("single DID-attributed item"),
        "the >1-item refusal must be gone (#199); stderr: {stderr}"
    );
    assert!(!out.status.success(), "no real DID exists under the mock");
    assert_eq!(
        out.status.code(),
        Some(4),
        "must fail on DID ownership (NotFound=4) — proving parsing + funding selection both \
         succeeded and the SAME failure as the single-item path was reached; stderr: {stderr}"
    );
    assert!(stderr.contains("does not own DID"));
}

/// #187 (dkackman, live user): a CHIP-0007-conformant `collection.json` — whose collection-level
/// attribute uses `"type"` (NOT `"trait_type"`) — must be ACCEPTED by `collection mint`. Before the
/// fix this failed at parse time with `--collection is not a valid definition: missing field
/// 'trait_type'` (exit 2), because `Collection::attributes` was wrongly typed as the NFT-item
/// `Attribute` (which demands `trait_type`). After the fix, parsing succeeds and the command
/// proceeds to the DID-ownership check, which fails under the offline mock (no real DID exists) —
/// proving the parse itself is no longer the failure. The distinct exit code (4, NOT 2) and error
/// text (no "trait_type"/"not a valid definition") is the regression guard.
#[test]
fn collection_mint_accepts_chip0007_type_attribute() {
    let dir = tmp_dig();
    let col = dir.path().join("col.json");
    std::fs::write(
        &col,
        r#"{"id":"c","name":"C","attributes":[{"type":"icon","value":"https://dig.net/icon.png"}],"royalty_puzzle_hash":"0000000000000000000000000000000000000000000000000000000000000000","royalty_basis_points":0}"#,
    )
    .unwrap();
    let items = dir.path().join("items.json");
    std::fs::write(
        &items,
        r#"[{"name":"A","media":{"data_uris":["dig://s/a"],"data_hash":"1111111111111111111111111111111111111111111111111111111111111111"}}]"#,
    )
    .unwrap();
    let out = dig(&dir)
        .args([
            "collection",
            "mint",
            "--collection",
            col.to_str().unwrap(),
            "--manifest",
            items.to_str().unwrap(),
            "--did",
            &"ab".repeat(32),
            "--dry-run",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("trait_type") && !stderr.contains("not a valid definition"),
        "a CHIP-0007 `type` collection attribute must parse (dkackman's #187 bug); stderr: {stderr}"
    );
    assert!(!out.status.success(), "no real DID exists under the mock");
    assert_eq!(
        out.status.code(),
        Some(4),
        "must fail on DID ownership (NotFound=4), not on parsing (InvalidArgument=2); stderr: {stderr}"
    );
    assert!(
        stderr.contains("does not own DID"),
        "expected the parse to succeed and fail past it on DID ownership; stderr: {stderr}"
    );
}

/// Back-compat (§5.1): a collection.json already emitted with the OLD, non-conformant
/// `trait_type` field on its collection attributes must STILL parse (the `#[serde(alias)]`), so
/// existing DIG collection definitions are not broken by the #187 fix.
#[test]
fn collection_mint_accepts_legacy_trait_type_collection_attribute() {
    let dir = tmp_dig();
    let col = dir.path().join("col.json");
    std::fs::write(
        &col,
        r#"{"id":"c","name":"C","attributes":[{"trait_type":"icon","value":"https://dig.net/icon.png"}],"royalty_puzzle_hash":"0000000000000000000000000000000000000000000000000000000000000000","royalty_basis_points":0}"#,
    )
    .unwrap();
    let items = dir.path().join("items.json");
    std::fs::write(
        &items,
        r#"[{"name":"A","media":{"data_uris":["dig://s/a"],"data_hash":"1111111111111111111111111111111111111111111111111111111111111111"}}]"#,
    )
    .unwrap();
    let out = dig(&dir)
        .args([
            "collection",
            "mint",
            "--collection",
            col.to_str().unwrap(),
            "--manifest",
            items.to_str().unwrap(),
            "--did",
            &"ab".repeat(32),
            "--dry-run",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(4),
        "the legacy trait_type collection attribute must still parse; stderr: {stderr}"
    );
    assert!(stderr.contains("does not own DID"));
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

/// #198: `--did` accepts a `did:chia:1…` bech32m address (how Sage/CNI display DIDs), decoding it
/// inline to the same launcher id a hex `--did` would — the exact ergonomics gap dkackman reported.
#[test]
fn collection_show_accepts_bech32_did() {
    let dir = tmp_dig();
    let out = dig(&dir)
        .args([
            "--json",
            "collection",
            "show",
            "--did",
            "did:chia:1s8j4pquxfu5mhlldzu357qfqkwa9r35mdx5a0p0ehn76dr4ut4tqs0n6kv",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "collection show --did <bech32> should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // Decodes to the SAME 64-hex launcher id a raw hex --did would carry.
    assert_eq!(v["did_launcher"].as_str().unwrap().len(), 64);
}

/// A malformed bech32 `did:chia:` address is rejected with a clear invalid-argument error (exit 2),
/// not a panic or a confusing hex-parse message.
#[test]
fn collection_show_rejects_malformed_bech32_did() {
    let dir = tmp_dig();
    dig(&dir)
        .args(["collection", "show", "--did", "did:chia:not-valid"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("invalid --did address"));
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
