//! THROWAWAY Phase-0 prototype (Plan 2). Proves the coinset-only anchoring
//! recipe end to end on Chia MAINNET. Reads the wallet mnemonic from a file
//! (default `.testcredentials` at the repo root) at runtime — never hardcoded.
//!
//! Usage:
//!   cargo run -p digstore-chain --example anchor_prototype -- balance
//!   cargo run -p digstore-chain --example anchor_prototype -- mint
//!
//! `balance` only derives the address and queries coinset (no spend).
//! `mint` builds + signs + push_tx's a real empty-store mint (spends XCH).

use bip39::Mnemonic;
use chia_sdk_coinset::{ChiaRpcClient, CoinsetClient};
use datalayer_driver::{
    master_public_key_to_first_puzzle_hash, master_secret_key_to_wallet_synthetic_secret_key,
    mint_store, puzzle_hash_to_address, secret_key_to_public_key, select_coins, sign_coin_spends,
    SecretKey,
};
use chia_protocol::{Bytes32, Coin, SpendBundle};

const MNEMONIC_FILE: &str = ".testcredentials";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "balance".to_string());

    // --- Key derivation (pure, no network) ---
    let phrase = std::fs::read_to_string(MNEMONIC_FILE)?.trim().to_string();
    let mnemonic = Mnemonic::parse_normalized(&phrase)?;
    let seed = mnemonic.to_seed("");
    let master_sk = SecretKey::from_seed(&seed);
    let master_pk = master_sk.public_key();
    let owner_puzzle_hash = master_public_key_to_first_puzzle_hash(&master_pk);
    let synthetic_sk = master_secret_key_to_wallet_synthetic_secret_key(&master_sk);
    let synthetic_pk = secret_key_to_public_key(&synthetic_sk);
    let address = puzzle_hash_to_address(owner_puzzle_hash, "xch")?;

    println!("owner puzzle hash : {}", hex::encode(owner_puzzle_hash));
    println!("receive address   : {address}");

    // --- Chain reads via coinset mainnet ---
    let client = CoinsetClient::mainnet();
    let resp = client
        .get_coin_records_by_puzzle_hashes(vec![owner_puzzle_hash], None, None, Some(false))
        .await?;
    if !resp.success {
        return Err(format!("coinset error: {:?}", resp.error).into());
    }
    let unspent: Vec<Coin> = resp
        .coin_records
        .unwrap_or_default()
        .into_iter()
        .filter(|cr| !cr.spent)
        .map(|cr| cr.coin)
        .collect();
    let balance: u64 = unspent.iter().map(|c| c.amount).sum();
    println!("unspent coins     : {}", unspent.len());
    println!("balance (mojos)   : {balance}");
    println!("balance (XCH)     : {}", balance as f64 / 1e12);

    if mode == "balance" {
        println!("\n[balance-only mode; no spend]");
        return Ok(());
    }
    if mode != "mint" {
        return Err(format!("unknown mode '{mode}' (use 'balance' or 'mint')").into());
    }

    // --- MINT an empty store (spends real XCH on mainnet) ---
    if balance == 0 {
        return Err("wallet has no funds; cannot mint".into());
    }
    let fee: u64 = 1_000; // tiny mainnet fee
    let selected = select_coins(&unspent, fee + 1)?;
    println!("\nselected {} coin(s) to fund mint (fee {fee} mojos)", selected.len());

    let success = mint_store(
        synthetic_pk,
        selected,
        Bytes32::default(), // empty root for a fresh store
        None,
        None,
        None,
        None,
        owner_puzzle_hash,
        vec![], // owner-only, no delegated puzzles
        fee,
    )?;
    let launcher_id = success.new_datastore.info.launcher_id;
    println!("built mint; launcher id (store_id) = {}", hex::encode(launcher_id));

    let signature = sign_coin_spends(&success.coin_spends, &[synthetic_sk], false)?;
    let bundle = SpendBundle::new(success.coin_spends, signature);

    println!("broadcasting via coinset push_tx ...");
    let push = client.push_tx(bundle).await?;
    println!("push_tx success={} status={} error={:?}", push.success, push.status, push.error);
    if !push.success {
        return Err(format!("push_tx rejected: {:?}", push.error).into());
    }

    // --- Poll for confirmation ---
    println!("polling for confirmation (launcher coin) ...");
    for i in 0..60 {
        let rec = client.get_coin_record_by_name(launcher_id).await?;
        if let Some(cr) = rec.coin_record {
            println!(
                "confirmed at block {} (spent={})",
                cr.confirmed_block_index, cr.spent
            );
            println!("\nSTORE MINTED. store_id = {}", hex::encode(launcher_id));
            return Ok(());
        }
        println!("  not yet in a block ({}s)...", i * 10);
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
    println!("\nsubmitted but not confirmed within timeout; store_id = {}", hex::encode(launcher_id));
    Ok(())
}
