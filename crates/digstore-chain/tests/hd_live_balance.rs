//! Read-only live HD-wallet scan against Chia mainnet (coinset.org).
//!
//! This test is `#[ignore]`d so it never runs in CI (which has no network).
//! Run it manually:
//!
//! ```text
//! TEST_CREDENTIALS=C:\path\to\.test-credentials \
//!   cargo test -p digstore-chain --test hd_live_balance -- --ignored --nocapture
//! ```
//!
//! The test reads the mnemonic from the credentials file (never from an
//! inline literal and never printed to stdout/stderr).  It then:
//!
//!   1. Scans only index 0 of the HD wallet and reports XCH + DIG there.
//!   2. Runs the full adaptive `scan_wallet` and reports the aggregate.
//!   3. Asserts the scan succeeds and aggregate >= index-0-only values.
//!
//! NO spend is built, signed, or broadcast.

use digstore_chain::anchor::{ChainAnchor, CoinsetAnchor};
use digstore_chain::coinset::{ChainReads, Coinset};
use digstore_chain::keys::{derive_indexed_keys, derive_wallet_keys, owner_address};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read the mnemonic from the `.test-credentials` file.
///
/// The file format has lines like:
///   `# Mnemonic: word1 word2 … word24`
///
/// We take the FIRST such comment (the primary / funding wallet).
/// The path is read from the `TEST_CREDENTIALS` env var; falls back to the
/// sibling repo path used in the project's CI scripts.
fn read_mnemonic() -> Result<String, String> {
    let path = std::env::var("TEST_CREDENTIALS").unwrap_or_else(|_| {
        // Default: sibling repo relative to a typical workspace checkout.
        "C:\\Users\\micha\\workspace\\dig_network\\chia-scaled-parallel-voting\\.test-credentials"
            .to_string()
    });

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot open credentials file at {path}: {e}"))?;

    for line in content.lines() {
        let trimmed = line.trim();
        // Lines starting with `# Mnemonic:` hold the BIP-39 phrase.
        if let Some(phrase) = trimmed
            .strip_prefix("# Mnemonic:")
            .or_else(|| trimmed.strip_prefix("#Mnemonic:"))
        {
            let phrase = phrase.trim().to_string();
            if !phrase.is_empty() {
                return Ok(phrase);
            }
        }
    }

    Err(format!("no '# Mnemonic:' line found in {path}"))
}

fn format_xch(mojos: u64) -> String {
    // 1 XCH = 1_000_000_000_000 mojos
    let xch = mojos as f64 / 1_000_000_000_000.0;
    format!("{xch:.6} XCH ({mojos} mojos)")
}

fn format_dig(base: u64) -> String {
    // 1 DIG = 1_000 base units (same as CHIP-0029 1e3)
    let dig = base as f64 / 1_000.0;
    format!("{dig:.3} DIG ({base} base units)")
}

// ---------------------------------------------------------------------------
// Live test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires network + TEST_CREDENTIALS file; run manually with --ignored"]
async fn hd_live_balance_aggregates_across_addresses() {
    // 1. Load mnemonic — aborts test if unavailable, never prints the phrase.
    let mnemonic = read_mnemonic().expect("test-credentials not available");

    // 2. Derive index-0 keys and report the single-address balance (the OLD
    //    behaviour that HD scan replaces).
    let chain = Coinset::mainnet();

    let idx0 = derive_indexed_keys(&mnemonic, 0..1)
        .expect("derive index 0")
        .into_iter()
        .next()
        .expect("index 0 present");

    let xch0: u64 = chain
        .unspent_coins(idx0.owner_puzzle_hash)
        .await
        .expect("fetch XCH index 0")
        .iter()
        .map(|c| c.amount)
        .sum();

    let dig_ph0 = digstore_chain::cat::dig_cat_puzzle_hash(idx0.owner_puzzle_hash);
    let dig0: u64 = chain
        .unspent_coins(dig_ph0)
        .await
        .expect("fetch DIG index 0")
        .iter()
        .map(|c| c.amount)
        .sum();

    println!("--- index-0 only (OLD behaviour) ---");
    println!("  address : {}", owner_address(&derive_wallet_keys(&mnemonic).expect("derive")));
    println!("  XCH     : {}", format_xch(xch0));
    println!("  DIG     : {}", format_dig(dig0));

    // 3. Run the full HD scan via the production anchor path.
    let anchor = CoinsetAnchor::mainnet();
    let w = anchor
        .scan(&mnemonic)
        .await
        .expect("HD scan should succeed against mainnet");

    let agg_xch = w.xch_balance();
    let agg_dig = w.dig_balance();
    let num_addrs = w.addrs.len();
    let signing_key_count = w.signing_keys().len();

    println!("--- aggregate HD scan (NEW behaviour) ---");
    println!("  addresses scanned + kept : {num_addrs}");
    println!("  signing keys             : {signing_key_count}");
    println!("  XCH aggregate            : {}", format_xch(agg_xch));
    println!("  DIG aggregate            : {}", format_dig(agg_dig));

    if agg_xch > xch0 || agg_dig > dig0 {
        println!("*** HD scan found funds beyond index 0 — fix validated ***");
    } else {
        println!("(all funds are at index 0; multi-address path structurally correct but not exercised by this wallet)");
    }

    // 4. Correctness assertions (read-only — no spend).
    assert!(
        agg_xch >= xch0,
        "aggregate XCH ({agg_xch}) must be >= index-0 XCH ({xch0})"
    );
    assert!(
        agg_dig >= dig0,
        "aggregate DIG ({agg_dig}) must be >= index-0 DIG ({dig0})"
    );
    assert!(
        signing_key_count == num_addrs,
        "signing_keys() count ({signing_key_count}) must equal kept-address count ({num_addrs})"
    );
    assert!(num_addrs >= 1, "scan must always keep at least index 0");
}
