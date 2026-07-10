//! `digstore collection create|mint` — first-class NFT collections (#34).
//!
//! * `create` — define a collection (shared id/name/royalty) and write its definition JSON. No chain,
//!   no spend; the definition is the input to `mint` (and is itself capsule-storable).
//! * `mint`   — bulk-mint every item in a parsed traits manifest into the collection, attributed to a
//!   creator DID, via [`digstore_chain::collection::build_collection_mint`] (single item — the DID's
//!   own value funds the one launcher) or
//!   [`digstore_chain::collection::build_collection_mint_funded`] (N>1 items — a separate XCH funding
//!   coin, selected via [`assets::scan_and_select_funding`], supplies the extra launchers; #199). The
//!   DID is spent once either way and authorizes every mint. `--dry-run` builds the spend WITHOUT
//!   signing/pushing. `--did` accepts a 64-hex launcher id OR a `did:chia:1…` bech32m address (#198).
//!
//! ## Scaffolded (clear TODO, not faked)
//! - The traits-manifest at SCALE (CSV ingest, generative trait composition, per-item capsule packing)
//!   is the toolkit's job — `mint` consumes an already-parsed items-array JSON (the same shape
//!   `nft bulk` takes), and the chain builder takes parsed items. See the TODO in
//!   `digstore_chain::collection`.

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
    build_collection_batch, build_collection_mint, default_batch_size, manifest_fingerprint,
    plan_batches, Collection, Drop, DropPhase, ManifestItem, MintProgress,
};
use digstore_chain::did::{list_owned_dids, OwnedDid};
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
    let did_launcher = assets::parse_did_arg(&args.did)?;
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
        // Plain-language warning — no internal tracker numbers in user-facing output.
        ui.line(
            "⚠ drop mechanics are SCAFFOLDED: recorded in the definition, NOT yet enforced at mint",
        );
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
    let did_launcher = assets::parse_did_arg(&args.did)?;
    let did_hex = hex::encode(did_launcher);

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

    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);

    // The minter key is the wallet's primary (index 0); TODO(#35) map the DID p2 to its exact index.
    let keys = digstore_chain::keys::derive_indexed_keys(&mnemonic, 0..1)
        .map_err(CliError::from)?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::Chain("could not derive wallet key".into()))?;
    let recipient = keys.owner_puzzle_hash;

    // SINGLE item: the DID's own coin funds the one launcher — no separate funding, no batching,
    // no resume state (a single bundle can never be oversized). Preserves the original path (#199).
    if items.len() == 1 {
        let owned = resolve_owned_did(chain.as_ref(), &mnemonic, did_launcher)?;
        let out = build_collection_mint(&keys, owned.did, &collection, &items, recipient)
            .map_err(CliError::from)?;
        let launcher_ids: Vec<String> = out.launcher_ids.iter().map(hex::encode).collect();
        if args.dry_run {
            emit_single(ui, &collection.id, &launcher_ids, None);
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
        emit_single(ui, &collection.id, &launcher_ids, Some(tx_id));
        return Ok(());
    }

    // MULTI item: auto-split into cost-bounded batches so no single spend bundle exceeds Chia's
    // per-block CLVM cost limit (#231 — dkackman's 200-item mint died as one oversized bundle). The
    // batch size is COMPUTED from the cost model unless `--batch-size` overrides it; a too-large
    // override is a terminal `TooLarge` error (never a coinset-connectivity misread).
    let effective_size = args.batch_size.unwrap_or_else(default_batch_size);
    let plan = plan_batches(items.len(), args.batch_size).map_err(CliError::from)?;

    // Resolve the creator DID the wallet owns. (Kept before the dry-run preview so an unowned DID is
    // reported here, as it always has been.)
    let _owned = resolve_owned_did(chain.as_ref(), &mnemonic, did_launcher)?;

    if args.dry_run {
        emit_plan(
            ui,
            &collection.id,
            items.len(),
            effective_size,
            &plan,
            mocked,
        );
        return Ok(());
    }

    // --- real run: resume-aware, one confirmed batch at a time ---
    let manifest_hash = manifest_fingerprint(items_raw.as_bytes());
    let progress_path = progress_file_path(&manifest_hash)?;
    let mut progress = load_progress(&progress_path)
        .filter(|p| {
            p.matches(&collection.id, &did_hex, &manifest_hash)
                && p.batch_size == effective_size
                && p.total_items == items.len()
        })
        .unwrap_or_else(|| {
            MintProgress::new(
                &collection.id,
                &did_hex,
                &manifest_hash,
                items.len(),
                effective_size,
            )
        });

    // Reconcile a pushed-but-unconfirmed tail against chain: if its recreated DID coin already
    // exists, the batch landed (the DID advanced) even though we never recorded its confirmation —
    // mark it confirmed so we never re-mint it. (Correctness also rests on the DID being single-use:
    // at most one mint can confirm per DID generation, so a re-run can never double-mint.)
    reconcile_tail(chain.as_ref(), &mut progress)?;

    let batch_count = plan.len();
    let mut minted_total = 0usize;
    for (bi, range) in plan.iter().enumerate() {
        let n = range.len();
        if progress.is_confirmed(range.start, range.end) {
            ui.line(format!(
                "batch {}/{batch_count}: already minted ({n} item(s)) — resuming past it",
                bi + 1
            ));
            minted_total += n;
            continue;
        }

        // Re-fetch the CURRENT unspent DID (reflects every confirmed batch so far) and select an XCH
        // coin covering this batch's launchers (1 mojo/item). Both are re-selected per batch because
        // each batch spends fresh coins (the DID's next generation + a fresh/change funding coin).
        let owned = resolve_owned_did(chain.as_ref(), &mnemonic, did_launcher)?;
        let did_coin_id = owned.coin_id;
        let (funding_keys, funding_coin) = block_on(assets::scan_and_select_funding(
            chain.as_ref(),
            &mnemonic,
            n as u64,
        ))??;

        let batch = build_collection_batch(
            &keys,
            owned.did,
            &collection,
            &items[range.clone()],
            recipient,
            funding_coin,
            &funding_keys,
        )
        .map_err(CliError::from)?;
        let launcher_ids: Vec<String> = batch.launcher_ids.iter().map(hex::encode).collect();
        let next_did_coin = batch.next_did.coin.coin_id();

        // Sign with the minter key + the funding coin's key (dedup by pubkey when identical).
        let signing_keys = vec![keys.synthetic_sk.clone(), funding_keys.synthetic_sk.clone()];
        let sig =
            sign_nft_spends(&batch.coin_spends, &signing_keys, false).map_err(CliError::from)?;
        let bundle = SpendBundle::new(batch.coin_spends, sig);

        // WRITE-AHEAD the attempt BEFORE broadcasting. The tx id and the recreated-DID coin are both
        // deterministic from the bundle, so they are known pre-broadcast. If the process died in the
        // window between a successful broadcast and this write, the batch could land yet be left
        // unrecorded — and a resume, re-fetching the now-advanced DID, would re-mint this batch's
        // items against the next generation (a real double-spend). Recording first lets
        // `reconcile_tail` detect a landed-but-unrecorded batch (its recreated DID coin on chain) and
        // skip it. Re-broadcasting the identical bundle on resume is idempotent at the mempool.
        let tx_id = bundle.name();
        progress.record_pending(
            range.start,
            range.end,
            hex::encode(did_coin_id),
            hex::encode(next_did_coin),
            hex::encode(tx_id),
            launcher_ids,
        );
        save_progress(&progress_path, &progress)?;

        block_on(assets::push_signed(chain.as_ref(), bundle))??;
        ui.line(format!(
            "batch {}/{batch_count}: broadcast {n} item(s) — tx {}",
            bi + 1,
            hex::encode(tx_id)
        ));

        // Wait for the recreated DID coin to appear on chain (proves the batch landed) before the
        // next batch, which must spend that recreated DID.
        let confirmed = block_on(assets::confirm_coin(chain.as_ref(), next_did_coin, 600))??;
        if !confirmed {
            save_progress(&progress_path, &progress)?;
            ui.line(format!(
                "batch {}/{batch_count} broadcast but not yet confirmed — re-run `digstore collection \
                 mint` to resume from here (already-minted batches are skipped)",
                bi + 1
            ));
            return Err(CliError::ConfirmTimeout);
        }
        progress.confirm(range.start, range.end);
        save_progress(&progress_path, &progress)?;
        minted_total += n;
        ui.line(format!("batch {}/{batch_count}: confirmed", bi + 1));
    }

    // Mint complete — remove the resume file and report every launcher id.
    let _ = std::fs::remove_file(&progress_path);
    let all_launchers: Vec<String> = progress
        .batches
        .iter()
        .flat_map(|b| b.launcher_ids.clone())
        .collect();
    emit_multi(
        ui,
        &collection.id,
        &all_launchers,
        minted_total,
        batch_count,
        mocked,
    );
    Ok(())
}

/// Reconstruct the creator DID the wallet owns; errors [`CliError::NotFound`] (exit 4) when the
/// wallet does not own `did_launcher`.
fn resolve_owned_did(
    chain: &dyn digstore_chain::coinset::ChainReads,
    mnemonic: &str,
    did_launcher: chia_protocol::Bytes32,
) -> Result<OwnedDid, CliError> {
    let owner_phs = scan_owner_phs(mnemonic)?;
    let dids = block_on(list_owned_dids(chain, &owner_phs))??;
    dids.into_iter()
        .find(|d| d.launcher_id == did_launcher)
        .ok_or_else(|| {
            CliError::NotFound(format!(
                "the wallet does not own DID {}",
                hex::encode(did_launcher)
            ))
        })
}

/// Path of the resume-progress file for a mint, content-addressed by the manifest fingerprint under
/// `~/.dig/collection-mints/`.
fn progress_file_path(manifest_hash: &str) -> Result<std::path::PathBuf, CliError> {
    let home = digstore_chain::config::dig_home().map_err(CliError::from)?;
    Ok(home
        .join("collection-mints")
        .join(format!("{manifest_hash}.json")))
}

/// Load a saved [`MintProgress`], or `None` if the file is absent/unreadable/corrupt (a corrupt or
/// stale record is simply ignored — the caller starts fresh).
fn load_progress(path: &std::path::Path) -> Option<MintProgress> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Persist [`MintProgress`] atomically enough for a CLI (create the dir, write the JSON).
fn save_progress(path: &std::path::Path, progress: &MintProgress) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CliError::Other(e.into()))?;
    }
    let json = serde_json::to_string_pretty(progress)
        .map_err(|e| CliError::Other(anyhow::anyhow!("serialize mint progress: {e}")))?;
    std::fs::write(path, json.as_bytes()).map_err(|e| CliError::Other(e.into()))?;
    Ok(())
}

/// Reconcile a pushed-but-unconfirmed tail batch: if its recreated DID coin is already on chain, the
/// batch landed — mark it confirmed so a resume never re-mints it.
fn reconcile_tail(
    chain: &dyn digstore_chain::coinset::ChainReads,
    progress: &mut MintProgress,
) -> Result<(), CliError> {
    let Some(tail) = progress.pending_tail() else {
        return Ok(());
    };
    let (start, end) = (tail.start, tail.end);
    let next_coin = match hex::decode(&tail.next_did_coin_id)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
    {
        Some(arr) => chia_protocol::Bytes32::new(arr),
        None => return Ok(()),
    };
    if block_on(chain.coin_record(next_coin))??.is_some() {
        progress.confirm(start, end);
    }
    Ok(())
}

/// JSON/pretty output for a single-item mint (the DID-funded, non-batched path).
fn emit_single(
    ui: &Ui,
    collection_id: &str,
    launcher_ids: &[String],
    tx_id: Option<chia_protocol::Bytes32>,
) {
    let dry = tx_id.is_none();
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

/// Dry-run preview of the batch plan for a large mint (no chain spend).
fn emit_plan(
    ui: &Ui,
    collection_id: &str,
    total_items: usize,
    batch_size: usize,
    plan: &[std::ops::Range<usize>],
    mocked: bool,
) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "collection.mint",
            "collection_id": collection_id,
            "dry_run": true,
            "mocked": mocked,
            "total_items": total_items,
            "batch_size": batch_size,
            "batch_count": plan.len(),
            "batches": plan.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>(),
        }));
    } else {
        ui.line(format!(
            "would mint {total_items} item(s) into collection `{collection_id}` in {} cost-bounded \
             batch(es) of up to {batch_size} (dry-run; nothing spent)",
            plan.len()
        ));
    }
}

/// Final summary for a completed multi-batch mint.
fn emit_multi(
    ui: &Ui,
    collection_id: &str,
    launcher_ids: &[String],
    minted: usize,
    batch_count: usize,
    mocked: bool,
) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "collection.mint",
            "collection_id": collection_id,
            "dry_run": false,
            "mocked": mocked,
            "minted": minted,
            "batch_count": batch_count,
            "launcher_ids": launcher_ids,
        }));
    } else {
        ui.success(format!(
            "minted {minted} item(s) into collection `{collection_id}` across {batch_count} batch(es)"
        ));
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
