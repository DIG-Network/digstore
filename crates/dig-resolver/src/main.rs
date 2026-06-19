//! dig-resolver — print a DIG store's current (tip) root.
//!
//! Usage: `dig-resolver <store_id_hex>`
//!
//! Walks the store's CHIP-0035 DataStore singleton lineage on mainnet via
//! coinset.org (`digstore_chain::singleton::sync_datastore`) and prints the
//! unspent tip's metadata `root_hash` as 64-char lowercase hex to stdout.
//!
//! The DIG Browser runs this for a *rootless* `dig://` navigation so the root
//! can be omitted by the user and resolved (chain-anchored, from coinset.org —
//! never from the serving node) behind the scenes. Exit codes: 0 = printed the
//! root; 1 = resolution failed (see stderr); 2 = bad usage/argument.

use chia_protocol::Bytes32;
use digstore_chain::coinset::Coinset;
use digstore_chain::singleton::sync_datastore;

#[tokio::main]
async fn main() {
    let store_id_hex = match std::env::args().nth(1) {
        Some(s) => s,
        None => {
            eprintln!("usage: dig-resolver <store_id_hex>");
            std::process::exit(2);
        }
    };

    let bytes = match hex::decode(store_id_hex.trim()) {
        Ok(b) if b.len() == 32 => b,
        _ => {
            eprintln!("store_id must be a 32-byte (64-hex) launcher id");
            std::process::exit(2);
        }
    };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    let launcher = Bytes32::new(arr);

    // Production reads over coinset.org (api.coinset.org), mainnet.
    let chain = Coinset::mainnet();
    match sync_datastore(&chain, launcher).await {
        Ok(store) => {
            // The unspent tip's content root — 64-char lowercase hex.
            println!("{}", hex::encode(store.info.metadata.root_hash));
        }
        Err(e) => {
            eprintln!("resolve failed: {e}");
            std::process::exit(1);
        }
    }
}
