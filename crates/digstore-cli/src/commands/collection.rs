//! `digstore collection create|mint` — first-class NFT collections (#34).
//!
//! * `create` — define a collection (shared id/name/royalty) and write its definition JSON. No chain,
//!   no spend; the definition is the input to `mint` (and is itself capsule-storable).
//! * `mint`   — bulk-mint every item in a parsed traits manifest into the collection, attributed to a
//!   creator DID, via [`digstore_chain::collection::build_collection_mint`] (the DID is spent once and
//!   authorizes all the mints). `--dry-run` builds the spend WITHOUT signing/pushing.
//!
//! ## Scaffolded (clear TODO, not faked)
//! - The traits-manifest at SCALE (CSV ingest, generative trait composition, per-item capsule packing)
//!   is the toolkit's job — `mint` consumes an already-parsed items-array JSON (the same shape
//!   `nft bulk` takes), and the chain builder takes parsed items. See the TODO in
//!   `digstore_chain::collection`.
//! - A MULTI-item DID-spent mint needs a separate XCH funding coin for the extra singleton launchers
//!   (the DID singleton alone carries 1 mojo). The single-item attributed mint is validated on-chain;
//!   the multi-item funded path is scaffolded here with a clear error rather than a silent bad spend.

use chia_protocol::SpendBundle;

use crate::cli::{
    CollectionAction, CollectionArgs, CollectionCreateArgs, CollectionListArgs, CollectionMintArgs,
    CollectionShowArgs,
};
use crate::error::CliError;
use crate::ops::assets;
use crate::runtime::block_on;
use crate::ui::Ui;

use digstore_chain::collection::{
    build_collection_mint, Collection, Drop, DropPhase, ManifestItem,
};
use digstore_chain::did::list_owned_dids;
use digstore_chain::nft::{list_collections, read_collection, sign_nft_spends, CollectionView};

pub fn run(ui: &Ui, args: CollectionArgs) -> Result<(), CliError> {
    match args.action {
        CollectionAction::Create(a) => create(ui, a),
        CollectionAction::Mint(a) => mint(ui, a),
        CollectionAction::Show(a) => show(ui, a),
        CollectionAction::List(a) => list(ui, a),
    }
}

/// The wallet's owner puzzle hashes to enumerate NFTs against (the first 20 HD indices).
fn scan_owner_phs(mnemonic: &str) -> Result<Vec<chia_protocol::Bytes32>, CliError> {
    let keys =
        digstore_chain::keys::derive_indexed_keys(mnemonic, 0..20).map_err(CliError::from)?;
    Ok(keys.iter().map(|k| k.owner_puzzle_hash).collect())
}

/// #39 `collection show --did <did>`: read one collection's items, owners, and royalty
/// from coinset (NO third-party indexer). Enumerates the wallet's NFTs attributed to
/// the DID and reports each item's launcher id + current owner + the shared royalty.
fn show(ui: &Ui, args: CollectionShowArgs) -> Result<(), CliError> {
    let did_launcher = assets::parse_launcher_id(&args.did)?;
    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);
    let phs = scan_owner_phs(&mnemonic)?;
    let view = block_on(read_collection(chain.as_ref(), &phs, did_launcher))??;

    if ui.json() {
        ui.emit_json(&collection_view_json("collection.show", &view, mocked));
    } else {
        ui.line(format!(
            "collection {}  ({} item(s){})",
            hex::encode(view.did_launcher),
            view.items.len(),
            view.royalty_basis_points
                .map(|bp| format!(", royalty {bp}bp"))
                .unwrap_or_default()
        ));
        for n in &view.items {
            ui.line(format!(
                "  {}  owner {}",
                hex::encode(n.launcher_id),
                hex::encode(n.p2_puzzle_hash)
            ));
        }
        if view.items.is_empty() {
            ui.line("  (no items held by this wallet for that DID)");
        }
    }
    Ok(())
}

/// #39 `collection list`: list every collection (creator DID) the wallet holds items
/// for, with each collection's item count + royalty — from coinset, no indexer.
fn list(ui: &Ui, _args: CollectionListArgs) -> Result<(), CliError> {
    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);
    let phs = scan_owner_phs(&mnemonic)?;
    let cols = block_on(list_collections(chain.as_ref(), &phs))??;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "collection.list",
            "mocked": mocked,
            "collections": cols.iter().map(|v| serde_json::json!({
                "did_launcher": hex::encode(v.did_launcher),
                "items": v.items.len(),
                "royalty_basis_points": v.royalty_basis_points,
            })).collect::<Vec<_>>(),
        }));
    } else if cols.is_empty() {
        ui.line("no collections held by this wallet");
    } else {
        for v in &cols {
            ui.line(format!(
                "{}  {} item(s){}",
                hex::encode(v.did_launcher),
                v.items.len(),
                v.royalty_basis_points
                    .map(|bp| format!("  royalty {bp}bp"))
                    .unwrap_or_default()
            ));
        }
    }
    Ok(())
}

/// JSON for a single [`CollectionView`] (shared by `show`).
fn collection_view_json(action: &str, view: &CollectionView, mocked: bool) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "mocked": mocked,
        "did_launcher": hex::encode(view.did_launcher),
        "royalty_basis_points": view.royalty_basis_points,
        "items": view.items.iter().map(|n| serde_json::json!({
            "launcher_id": hex::encode(n.launcher_id),
            "coin_id": hex::encode(n.coin_id),
            "owner_puzzle_hash": hex::encode(n.p2_puzzle_hash),
            "royalty_basis_points": n.royalty_basis_points,
        })).collect::<Vec<_>>(),
    })
}

/// Slugify a name into a stable default collection id (lowercase, non-alnum → `-`, collapsed).
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Parse one `--phase name[:start_unix[:supply]]` spec into a [`DropPhase`] (#40).
/// `name` is required; `start_unix`/`supply` are optional positional, colon-separated.
fn parse_phase(spec: &str) -> Result<DropPhase, CliError> {
    let mut parts = spec.splitn(3, ':');
    let name = parts.next().unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err(CliError::InvalidArgument(
            "--phase needs a name: name[:start_unix[:supply]]".into(),
        ));
    }
    let start_unix =
        match parts.next() {
            Some(s) if !s.trim().is_empty() => Some(s.trim().parse::<u64>().map_err(|_| {
                CliError::InvalidArgument(format!("--phase start not a number: {s}"))
            })?),
            _ => None,
        };
    let supply =
        match parts.next() {
            Some(s) if !s.trim().is_empty() => Some(s.trim().parse::<u64>().map_err(|_| {
                CliError::InvalidArgument(format!("--phase supply not a number: {s}"))
            })?),
            _ => None,
        };
    Ok(DropPhase {
        // A phase with an allowlist (collection-level `--allow`) is allowlist-only by
        // convention when named "allowlist"; finer per-phase gating is a #40 follow-up.
        allowlist_only: name.eq_ignore_ascii_case("allowlist"),
        name,
        start_unix,
        supply,
    })
}

/// Assemble the optional drop config from `create` flags (#40, scaffolded). Returns
/// `None` when no drop flag is set (an ordinary open collection).
fn build_drop(args: &CollectionCreateArgs) -> Result<Option<Drop>, CliError> {
    let phases = args
        .phase
        .iter()
        .map(|p| parse_phase(p))
        .collect::<Result<Vec<_>, _>>()?;
    let drop = Drop {
        reveal_unix: args.reveal_at,
        allowlist: args.allow.clone(),
        phases,
        lazy_mint: args.lazy_mint,
    };
    Ok(drop.is_configured().then_some(drop))
}

fn create(ui: &Ui, args: CollectionCreateArgs) -> Result<(), CliError> {
    // #40 (scaffolded): assemble the optional drop mechanics from the flags FIRST
    // (while `args` is fully intact). The model is committed into the definition;
    // enforcement in the mint path is TODO (see digstore_chain::collection::Drop).
    let drop = build_drop(&args)?;

    let id = args.id.clone().unwrap_or_else(|| slug(&args.name));
    if id.is_empty() {
        return Err(CliError::InvalidArgument(
            "--name produced an empty id; pass --id explicitly".into(),
        ));
    }

    // The royalty recipient defaults to the wallet's own address (so the creator collects royalties).
    let royalty_puzzle_hash = match &args.royalty_address {
        Some(addr) => assets::parse_xch_address(addr)?,
        None => {
            let mnemonic = assets::unlock_mnemonic(ui)?;
            let keys = digstore_chain::keys::derive_indexed_keys(&mnemonic, 0..1)
                .map_err(CliError::from)?
                .into_iter()
                .next()
                .ok_or_else(|| CliError::Chain("could not derive wallet key".into()))?;
            keys.owner_puzzle_hash
        }
    };

    if drop.is_some() && !ui.json() {
        ui.line("⚠ drop mechanics are SCAFFOLDED: recorded in the definition, NOT yet enforced at mint (#40)");
    }

    let collection = Collection {
        id: id.clone(),
        name: args.name.clone(),
        attributes: Vec::new(),
        royalty_puzzle_hash,
        royalty_basis_points: args.royalty,
        drop,
    };
    let json = serde_json::to_string_pretty(&collection)
        .map_err(|e| CliError::Other(anyhow::anyhow!("serialize collection: {e}")))?;

    match &args.out {
        Some(path) => {
            std::fs::write(path, json.as_bytes()).map_err(|e| CliError::Other(e.into()))?;
            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "action": "collection.create",
                    "id": id,
                    "out": path.display().to_string(),
                }));
            } else {
                ui.success(format!("collection `{id}` written to {}", path.display()));
            }
        }
        None => {
            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "action": "collection.create",
                    "id": id,
                    "collection": collection,
                }));
            } else {
                ui.line(json);
            }
        }
    }
    Ok(())
}

fn mint(ui: &Ui, args: CollectionMintArgs) -> Result<(), CliError> {
    let did_launcher = assets::parse_launcher_id(&args.did)?;

    let col_raw = std::fs::read_to_string(&args.collection).map_err(|e| {
        CliError::InvalidArgument(format!(
            "read --collection {}: {e}",
            args.collection.display()
        ))
    })?;
    let collection: Collection = serde_json::from_str(&col_raw).map_err(|e| {
        CliError::InvalidArgument(format!("--collection is not a valid definition: {e}"))
    })?;

    let items_raw = std::fs::read_to_string(&args.manifest).map_err(|e| {
        CliError::InvalidArgument(format!("read --manifest {}: {e}", args.manifest.display()))
    })?;
    let items: Vec<ManifestItem> = serde_json::from_str(&items_raw).map_err(|e| {
        CliError::InvalidArgument(format!("--manifest is not a valid items array: {e}"))
    })?;
    if items.is_empty() {
        return Err(CliError::InvalidArgument(
            "--manifest must contain at least one item".into(),
        ));
    }
    // TODO(#34 at scale): a MULTI-item DID-spent mint needs a separate XCH funding coin for the extra
    // launchers (the DID singleton carries 1 mojo). Until that funding path is wired, refuse >1 item
    // with a clear message rather than build an underfunded (failing) spend.
    if items.len() > 1 {
        return Err(CliError::InvalidArgument(format!(
            "collection mint currently supports a single DID-attributed item per call ({} given); \
             multi-item DID-funded bulk mint is scaffolded (needs a separate XCH funding coin for the \
             extra launchers — roadmap #34). Mint items individually for now.",
            items.len()
        )));
    }

    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);

    // Reconstruct the creator DID the wallet owns (its current coin is what the mint spends).
    let owner_phs = scan_owner_phs(&mnemonic)?;
    let dids = block_on(list_owned_dids(chain.as_ref(), &owner_phs))??;
    let owned = dids
        .into_iter()
        .find(|d| d.launcher_id == did_launcher)
        .ok_or_else(|| {
            CliError::NotFound(format!(
                "the wallet does not own DID {}",
                hex::encode(did_launcher)
            ))
        })?;

    // The minter key is the wallet's primary (index 0); TODO(#35) map the DID p2 to its exact index.
    let keys = digstore_chain::keys::derive_indexed_keys(&mnemonic, 0..1)
        .map_err(CliError::from)?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::Chain("could not derive wallet key".into()))?;

    let recipient = keys.owner_puzzle_hash;
    let out = build_collection_mint(&keys, owned.did, &collection, &items, recipient)
        .map_err(CliError::from)?;
    let launcher_ids: Vec<String> = out.launcher_ids.iter().map(hex::encode).collect();

    if args.dry_run {
        emit(ui, &collection.id, &launcher_ids, None, true);
        return Ok(());
    }
    let sig = sign_nft_spends(
        &out.coin_spends,
        std::slice::from_ref(&keys.synthetic_sk),
        false,
    )
    .map_err(CliError::from)?;
    let tx_id = block_on(assets::push_signed(
        chain.as_ref(),
        SpendBundle::new(out.coin_spends, sig),
    ))??;
    emit(ui, &collection.id, &launcher_ids, Some(tx_id), false);
    Ok(())
}

fn emit(
    ui: &Ui,
    collection_id: &str,
    launcher_ids: &[String],
    tx_id: Option<chia_protocol::Bytes32>,
    dry: bool,
) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "collection.mint",
            "collection_id": collection_id,
            "launcher_ids": launcher_ids,
            "tx_id": tx_id.map(hex::encode),
            "dry_run": dry,
        }));
    } else if dry {
        ui.line(format!(
            "would mint {} item(s) into collection `{collection_id}` (dry-run; nothing spent)",
            launcher_ids.len()
        ));
    } else {
        ui.success(format!(
            "minted {} item(s) into collection `{collection_id}`",
            launcher_ids.len()
        ));
        if let Some(t) = tx_id {
            ui.line(format!("tx {}", hex::encode(t)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_makes_stable_ids() {
        assert_eq!(slug("DIG Punks"), "dig-punks");
        assert_eq!(slug("  Hello, World! "), "hello-world");
        assert_eq!(slug("a___b"), "a-b");
        assert_eq!(slug("!!!"), "");
    }

    #[test]
    fn parse_phase_parses_name_start_supply() {
        // #40: name only.
        let p = parse_phase("public").unwrap();
        assert_eq!(p.name, "public");
        assert_eq!(p.start_unix, None);
        assert_eq!(p.supply, None);
        assert!(!p.allowlist_only);
        // name:start.
        let p = parse_phase("early:1800000000").unwrap();
        assert_eq!(p.start_unix, Some(1_800_000_000));
        assert_eq!(p.supply, None);
        // name:start:supply, allowlist named phase is allowlist_only.
        let p = parse_phase("allowlist:1800000000:100").unwrap();
        assert_eq!(p.start_unix, Some(1_800_000_000));
        assert_eq!(p.supply, Some(100));
        assert!(p.allowlist_only);
        // empty name / bad number error.
        assert!(parse_phase("").is_err());
        assert!(parse_phase("p:notanumber").is_err());
    }

    fn create_args() -> CollectionCreateArgs {
        CollectionCreateArgs {
            name: "C".into(),
            id: None,
            royalty: 0,
            royalty_address: None,
            out: None,
            reveal_at: None,
            allow: vec![],
            phase: vec![],
            lazy_mint: false,
        }
    }

    #[test]
    fn build_drop_is_none_without_flags() {
        // #40: no drop flags → no drop block.
        assert!(build_drop(&create_args()).unwrap().is_none());
    }

    #[test]
    fn build_drop_collects_all_mechanics() {
        let mut a = create_args();
        a.reveal_at = Some(1_900_000_000);
        a.allow = vec!["abcd".into()];
        a.phase = vec!["allowlist:1800000000:50".into(), "public".into()];
        a.lazy_mint = true;
        let drop = build_drop(&a).unwrap().expect("drop configured");
        assert_eq!(drop.reveal_unix, Some(1_900_000_000));
        assert_eq!(drop.allowlist, vec!["abcd".to_string()]);
        assert_eq!(drop.phases.len(), 2);
        assert!(drop.phases[0].allowlist_only);
        assert!(drop.lazy_mint);
    }
}
