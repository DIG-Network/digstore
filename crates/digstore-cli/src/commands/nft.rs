//! `digstore nft mint|bulk|transfer|list` — NFTs with capsule-stored media (#33/#35).
//!
//! `mint` is the headline path (#33, "truly permanent NFTs"): it WRITES the art + the generated
//! CHIP-0007 metadata JSON into a real DIG capsule, COMPUTES `data_hash`/`metadata_hash` from the
//! REAL bytes (via [`digstore_chain::metadata`]), and sets the on-chain `data_uris`/`metadata_uris`
//! to the capsule's `dig://` URN (primary) + an optional https gateway URI (fallback) BEFORE building
//! the mint spend ([`digstore_chain::nft::build_nft_mint`]). The media lives on DIG, not a
//! centralized host, and the on-chain hashes are pinned to what the URIs actually serve.
//!
//! `bulk`/`transfer`/`list` surface the matching `digstore-chain` builders. `--dry-run` (on
//! mint/bulk/transfer) builds the spend + prints the plan WITHOUT signing/pushing (no spend), so the
//! offline suite exercises the build + capsule-media path end-to-end without touching the chain.

use std::path::Path;

use chia_protocol::{Bytes32, SpendBundle};
use digstore_core::Bytes32 as CoreBytes32;

use crate::cli::{NftAction, NftArgs, NftBulkArgs, NftListArgs, NftMintArgs, NftTransferArgs};
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::assets;
use crate::runtime::block_on;
use crate::ui::Ui;

use digstore_chain::collection::{ManifestItem, ManifestMedia};
use digstore_chain::metadata::{sha256, Chip0007Metadata};
use digstore_chain::nft::{
    build_bulk_mint, build_nft_mint, build_nft_transfer, list_owned_nfts, sign_nft_spends, MintSpec,
};

/// 1 mojo funds the NFT singleton launcher (the mint draws this from the funding coin).
const SINGLETON_MOJO: u64 = 1;
/// The canonical resource key for the NFT's media inside its capsule.
const ART_RESOURCE: &str = "art";

pub fn run(ctx: &CliContext, ui: &Ui, args: NftArgs) -> Result<(), CliError> {
    match args.action {
        NftAction::Mint(a) => mint(ctx, ui, a),
        NftAction::Bulk(a) => bulk(ui, a),
        NftAction::Transfer(a) => transfer(ui, a),
        NftAction::List(a) => list(ui, a),
    }
}

/// The result of writing the NFT media into a capsule (#33): the capsule identity + the hashes and
/// URIs the on-chain mint pins.
///
/// `store_id`/`root_hash` are the capsule's digstore-core identity; `data_hash`/`metadata_hash` are
/// the chain-protocol hashes pinned in the on-chain NFT metadata (computed from the real bytes).
struct CapsuleMedia {
    store_id: CoreBytes32,
    root_hash: CoreBytes32,
    data_hash: Bytes32,
    metadata_hash: Bytes32,
    metadata_json: String,
    data_uris: Vec<String>,
    metadata_uris: Vec<String>,
}

/// Write the art + generated CHIP-0007 metadata into a fresh capsule and compute the on-chain media
/// fields (#33). The capsule is a real DIG store built in `ctx`'s (ephemeral) dig dir: it stages the
/// art under `ART_RESOURCE` and the metadata under `metadata.json`, commits a generation, and returns
/// the resulting `storeId:rootHash` plus the byte-computed hashes + the dig://(+gateway) URIs.
fn build_media_capsule(
    ctx: &CliContext,
    art_path: &Path,
    name: &str,
    description: Option<&str>,
    gateway: Option<&str>,
) -> Result<CapsuleMedia, CliError> {
    let art_bytes = std::fs::read(art_path).map_err(|e| {
        CliError::InvalidArgument(format!("read --art {}: {e}", art_path.display()))
    })?;
    if art_bytes.is_empty() {
        return Err(CliError::InvalidArgument(format!(
            "--art file is empty: {}",
            art_path.display()
        )));
    }
    let data_hash = sha256(&art_bytes);

    // Generate the CHIP-0007 metadata document for this single item.
    let mut md = Chip0007Metadata::new(name);
    md.description = description.map(|s| s.to_string());
    md.minting_tool = Some("DIG".to_string());
    md.validate_schema()
        .map_err(|e| CliError::InvalidArgument(e.to_string()))?;
    let metadata_json = md
        .to_canonical_json()
        .map_err(|e| CliError::Chain(e.to_string()))?;
    let metadata_hash = sha256(metadata_json.as_bytes());

    // Build the capsule: init an ephemeral store, stage art + metadata.json, commit one generation.
    // The op_dir (ctx.op_dir) is the content root the staging walk reads, so write the two files
    // there. We use a content-addressed store id (None override => SHA-256 of the fresh host key),
    // which is this media capsule's own store id.
    crate::ops::store_ops::init_store(ctx, false, None, None, None, None, None, None)?;
    let content_root = &ctx.op_dir;
    std::fs::create_dir_all(content_root).map_err(|e| CliError::Other(e.into()))?;
    std::fs::write(content_root.join(ART_RESOURCE), &art_bytes)
        .map_err(|e| CliError::Other(e.into()))?;
    std::fs::write(content_root.join("metadata.json"), metadata_json.as_bytes())
        .map_err(|e| CliError::Other(e.into()))?;

    let staged = crate::ops::store_ops::add_files(ctx, &[], true, false, None)?;
    if staged.staged.is_empty() {
        return Err(CliError::Chain("capsule build staged no files".into()));
    }
    let metadata = crate::ops::serve::empty_manifest();
    let outcome = crate::ops::store_ops::commit(ctx, Some("NFT media".to_string()), metadata)?;
    let root_hash = outcome.roothash;
    let store_id = ctx.find_store_id()?;

    // The dig:// URN is the PRIMARY URI; the https gateway (if given) is the fallback.
    let mut data_uris = vec![assets::dig_uri(store_id, root_hash, ART_RESOURCE)];
    let mut metadata_uris = vec![assets::dig_uri(store_id, root_hash, "metadata.json")];
    if let Some(gw) = gateway {
        data_uris.push(assets::gateway_uri(gw, store_id, root_hash, ART_RESOURCE));
        metadata_uris.push(assets::gateway_uri(
            gw,
            store_id,
            root_hash,
            "metadata.json",
        ));
    }

    Ok(CapsuleMedia {
        store_id,
        root_hash,
        data_hash,
        metadata_hash,
        metadata_json,
        data_uris,
        metadata_uris,
    })
}

fn mint(ctx: &CliContext, ui: &Ui, args: NftMintArgs) -> Result<(), CliError> {
    // 1. Write the media into a capsule and compute the on-chain media fields FIRST (#33). This is
    //    free (no chain), so it runs before any wallet unlock — `--dry-run` stops right after the
    //    mint spend is built.
    let media = build_media_capsule(
        ctx,
        &args.art,
        &args.name,
        args.description.as_deref(),
        args.gateway.as_deref(),
    )?;

    // 2. Build the on-chain NFT metadata program from the capsule media (dig:// + hashes).
    let item = ManifestItem {
        name: args.name.clone(),
        description: args.description.clone(),
        attributes: Vec::new(),
        media: ManifestMedia {
            data_uris: media.data_uris.clone(),
            data_hash: Some(media.data_hash),
            metadata_uris: media.metadata_uris.clone(),
            metadata_hash: Some(media.metadata_hash),
            ..Default::default()
        },
    };
    let metadata_program = digstore_chain::collection::item_to_metadata_program(&item, 1, 1)
        .map_err(CliError::from)?;

    // 3. Unlock the wallet + chain backend.
    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);

    // 4. Two mint paths:
    //    (a) DID-ATTRIBUTED (#38): reconstruct the owned DID, build the mint with the
    //        DID's acknowledging spend composed into the SAME bundle (the launcher is
    //        created off the DID coin — no separate XCH funding coin needed, the DID
    //        singleton carries the mojo), so the NFT is verifiably DID-attributed.
    //    (b) PLAIN: fund the 1-mojo launcher from an XCH coin (no attribution).
    let (spends, launcher_id, signer_sk) = match &args.did {
        Some(did_hex) => {
            let did_launcher = assets::parse_launcher_id(did_hex)?;
            // The minter key is the wallet's primary (index 0); the DID is reconstructed
            // over chain (its current coin is what the mint spends to acknowledge).
            // TODO(#35): map the DID's p2 puzzle hash back to its exact HD index.
            let owner_phs = scan_owner_phs(&mnemonic)?;
            let dids = block_on(digstore_chain::did::list_owned_dids(
                chain.as_ref(),
                &owner_phs,
            ))??;
            let owned = dids
                .into_iter()
                .find(|d| d.launcher_id == did_launcher)
                .ok_or_else(|| {
                    CliError::NotFound(format!(
                        "the wallet does not own DID {} (create one with `digstore did create`, \
                         or mint without --did)",
                        hex::encode(did_launcher)
                    ))
                })?;
            let keys = digstore_chain::keys::derive_indexed_keys(&mnemonic, 0..1)
                .map_err(CliError::from)?
                .into_iter()
                .next()
                .ok_or_else(|| CliError::Chain("could not derive wallet key".into()))?;
            let (spends, nft) = digstore_chain::nft::build_nft_mint_with_did(
                &keys,
                owned.did,
                metadata_program,
                keys.owner_puzzle_hash,
                args.royalty,
            )
            .map_err(CliError::from)?;
            let launcher_id = nft.info.launcher_id;
            (spends, launcher_id, keys.synthetic_sk)
        }
        None => {
            let (keys, funding) = block_on(assets::scan_and_select_funding(
                chain.as_ref(),
                &mnemonic,
                SINGLETON_MOJO,
            ))??;
            let spec = MintSpec {
                metadata: metadata_program,
                owner_ph: keys.owner_puzzle_hash,
                royalty_basis_points: args.royalty,
                did: None,
            };
            let (spends, nft) = build_nft_mint(&keys, funding, &spec).map_err(CliError::from)?;
            let launcher_id = nft.info.launcher_id;
            (spends, launcher_id, keys.synthetic_sk)
        }
    };

    if args.dry_run {
        emit_mint(ui, &media, launcher_id, None, true, mocked);
        return Ok(());
    }

    let sig = sign_nft_spends(&spends, std::slice::from_ref(&signer_sk), false)
        .map_err(CliError::from)?;
    let tx_id = block_on(assets::push_signed(
        chain.as_ref(),
        SpendBundle::new(spends, sig),
    ))??;
    emit_mint(ui, &media, launcher_id, Some(tx_id), false, mocked);
    Ok(())
}

fn bulk(ui: &Ui, args: NftBulkArgs) -> Result<(), CliError> {
    let raw = std::fs::read_to_string(&args.manifest).map_err(|e| {
        CliError::InvalidArgument(format!("read --manifest {}: {e}", args.manifest.display()))
    })?;
    let items: Vec<ManifestItem> = serde_json::from_str(&raw).map_err(|e| {
        CliError::InvalidArgument(format!("--manifest is not a valid items array: {e}"))
    })?;
    if items.is_empty() {
        return Err(CliError::InvalidArgument(
            "--manifest must contain at least one item".into(),
        ));
    }
    if args.did.is_some() {
        return Err(CliError::InvalidArgument(
            "DID-attributed bulk mint needs the DID spend composed into the bundle — use \
             `collection mint --did <did>` for the attributed path"
                .into(),
        ));
    }

    // Build a MintSpec per item from its (already-computed) media hashes/URIs.
    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);
    let (keys, funding) = block_on(assets::scan_and_select_funding(
        chain.as_ref(),
        &mnemonic,
        items.len() as u64,
    ))??;

    let total = items.len();
    let mut specs = Vec::with_capacity(total);
    for (i, item) in items.iter().enumerate() {
        let metadata =
            digstore_chain::collection::item_to_metadata_program(item, i as u64 + 1, total as u64)
                .map_err(CliError::from)?;
        specs.push(MintSpec {
            metadata,
            owner_ph: keys.owner_puzzle_hash,
            royalty_basis_points: 0,
            did: None,
        });
    }

    let (spends, nfts) = build_bulk_mint(&keys, funding, &specs).map_err(CliError::from)?;
    let launcher_ids: Vec<String> = nfts
        .iter()
        .map(|n| hex::encode(n.info.launcher_id))
        .collect();

    if args.dry_run {
        emit_bulk(ui, &launcher_ids, None, true, mocked);
        return Ok(());
    }
    let sig = sign_nft_spends(&spends, std::slice::from_ref(&keys.synthetic_sk), false)
        .map_err(CliError::from)?;
    let tx_id = block_on(assets::push_signed(
        chain.as_ref(),
        SpendBundle::new(spends, sig),
    ))??;
    emit_bulk(ui, &launcher_ids, Some(tx_id), false, mocked);
    Ok(())
}

fn transfer(ui: &Ui, args: NftTransferArgs) -> Result<(), CliError> {
    let launcher_id = assets::parse_launcher_id(&args.nft)?;
    let to_ph = assets::parse_xch_address(&args.to)?;

    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);

    // Find the owned NFT whose launcher id matches (its current coin is what the transfer spends).
    let primary_phs = scan_owner_phs(&mnemonic)?;
    let owned = block_on(list_owned_nfts(chain.as_ref(), &primary_phs))??;
    let target = owned
        .into_iter()
        .find(|n| n.launcher_id == launcher_id)
        .ok_or_else(|| {
            CliError::NotFound(format!(
                "the wallet does not own NFT {}",
                hex::encode(launcher_id)
            ))
        })?;

    // The signing key is the address index holding the NFT; for the common case it is index 0.
    // TODO(#35): map the NFT's p2 puzzle hash back to its exact HD index (currently uses index 0).
    let keys = digstore_chain::keys::derive_indexed_keys(&mnemonic, 0..1)
        .map_err(CliError::from)?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::Chain("could not derive wallet key".into()))?;

    let (spends, child) = block_on(build_nft_transfer(
        chain.as_ref(),
        &keys,
        target.nft.coin,
        to_ph,
        0,
        None,
    ))??;
    let new_owner = child.info.p2_puzzle_hash;

    if args.dry_run {
        emit_transfer(ui, launcher_id, new_owner, None, true, mocked);
        return Ok(());
    }
    let sig = sign_nft_spends(&spends, std::slice::from_ref(&keys.synthetic_sk), false)
        .map_err(CliError::from)?;
    let tx_id = block_on(assets::push_signed(
        chain.as_ref(),
        SpendBundle::new(spends, sig),
    ))??;
    emit_transfer(ui, launcher_id, new_owner, Some(tx_id), false, mocked);
    Ok(())
}

fn list(ui: &Ui, _args: NftListArgs) -> Result<(), CliError> {
    let mnemonic = assets::unlock_mnemonic(ui)?;
    let (chain, mocked) = assets::chain_reads();
    assets::warn_if_mocked(ui, mocked);
    let phs = scan_owner_phs(&mnemonic)?;
    let owned = block_on(list_owned_nfts(chain.as_ref(), &phs))??;

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "nft.list",
            "nfts": owned.iter().map(|n| serde_json::json!({
                "launcher_id": hex::encode(n.launcher_id),
                "coin_id": hex::encode(n.coin_id),
                "owner_did": n.owner_did.map(hex::encode),
                "royalty_basis_points": n.royalty_basis_points,
                "p2_puzzle_hash": hex::encode(n.p2_puzzle_hash),
            })).collect::<Vec<_>>(),
        }));
    } else if owned.is_empty() {
        ui.line("no NFTs owned by this wallet");
    } else {
        for n in &owned {
            ui.line(format!(
                "{}  royalty {}bp{}",
                hex::encode(n.launcher_id),
                n.royalty_basis_points,
                n.owner_did
                    .map(|d| format!("  did {}", hex::encode(d)))
                    .unwrap_or_default()
            ));
        }
    }
    Ok(())
}

/// The wallet's owner puzzle hashes to enumerate NFTs/DIDs against (the first 20 HD indices, the
/// scan width the wallet uses elsewhere).
fn scan_owner_phs(mnemonic: &str) -> Result<Vec<Bytes32>, CliError> {
    let keys =
        digstore_chain::keys::derive_indexed_keys(mnemonic, 0..20).map_err(CliError::from)?;
    Ok(keys.iter().map(|k| k.owner_puzzle_hash).collect())
}

fn emit_mint(
    ui: &Ui,
    media: &CapsuleMedia,
    launcher_id: Bytes32,
    tx_id: Option<Bytes32>,
    dry: bool,
    mocked: bool,
) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "nft.mint",
            "launcher_id": hex::encode(launcher_id),
            "tx_id": tx_id.map(hex::encode),
            "dry_run": dry,
            "mocked": mocked,
            "capsule": {
                "store_id": media.store_id.to_hex(),
                "root_hash": media.root_hash.to_hex(),
                "data_hash": hex::encode(media.data_hash),
                "metadata_hash": hex::encode(media.metadata_hash),
                "data_uris": media.data_uris,
                "metadata_uris": media.metadata_uris,
                "metadata_json": media.metadata_json,
            }
        }));
    } else {
        if dry {
            ui.line(format!(
                "would mint NFT {} (dry-run; nothing spent)",
                hex::encode(launcher_id)
            ));
        } else {
            ui.success(format!("minted NFT {}", hex::encode(launcher_id)));
            if let Some(t) = tx_id {
                ui.line(format!("tx {}", hex::encode(t)));
            }
        }
        ui.line(format!(
            "media capsule {}:{}",
            media.store_id.to_hex(),
            media.root_hash.to_hex()
        ));
        ui.line(format!("data uri  {}", media.data_uris[0]));
        ui.line(format!("data hash {}", hex::encode(media.data_hash)));
    }
}

fn emit_bulk(ui: &Ui, launcher_ids: &[String], tx_id: Option<Bytes32>, dry: bool, mocked: bool) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "nft.bulk",
            "launcher_ids": launcher_ids,
            "tx_id": tx_id.map(hex::encode),
            "dry_run": dry,
            "mocked": mocked,
        }));
    } else if dry {
        ui.line(format!(
            "would bulk-mint {} NFTs (dry-run; nothing spent)",
            launcher_ids.len()
        ));
    } else {
        ui.success(format!("bulk-minted {} NFTs", launcher_ids.len()));
        if let Some(t) = tx_id {
            ui.line(format!("tx {}", hex::encode(t)));
        }
    }
}

fn emit_transfer(
    ui: &Ui,
    launcher_id: Bytes32,
    new_owner: Bytes32,
    tx_id: Option<Bytes32>,
    dry: bool,
    mocked: bool,
) {
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "action": "nft.transfer",
            "launcher_id": hex::encode(launcher_id),
            "new_owner_puzzle_hash": hex::encode(new_owner),
            "tx_id": tx_id.map(hex::encode),
            "dry_run": dry,
            "mocked": mocked,
        }));
    } else if dry {
        ui.line(format!(
            "would transfer NFT {} (dry-run; nothing spent)",
            hex::encode(launcher_id)
        ));
    } else {
        ui.success(format!("transferred NFT {}", hex::encode(launcher_id)));
        if let Some(t) = tx_id {
            ui.line(format!("tx {}", hex::encode(t)));
        }
    }
}
