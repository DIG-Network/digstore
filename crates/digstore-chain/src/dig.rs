//! DIG token (CAT) constants + amount helpers (see the DIG-CAT-payment design).
use chia_protocol::Bytes32;

/// DIG CAT asset id (mainnet). Matches DataLayer-Driver `DIG_ASSET_ID`.
pub const DIG_ASSET_ID: Bytes32 = Bytes32::new(hex_literal::hex!(
    "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81"
));

/// DIG treasury recipient (bech32 `xch1…`); DIG is sent to this address's CAT ph.
pub const TREASURY_ADDRESS: &str = "xch1a37rq3cgcl2ecpudttsf35x75qzdan68lgw2l6ajvmqs44jxdn5qv6pk3y";

/// DIG has 3 decimals: 1 DIG = 1000 base units.
pub const DIG_DECIMALS: u32 = 3;
/// DEFAULT base units to mint a store (`init`): 100 DIG.
///
/// This is a DEFAULT, not a hard constant: the per-capsule price is **dynamic and
/// USD-pegged** (SYSTEM.md — `dig_amount = target_usd ÷ live DIG price`, where
/// `target_usd ≈ $1/capsule/year` of realistic AWS hosting). The hub computes that
/// live amount in the browser and the CLI accepts it explicitly (`--dig-amount` /
/// `DIGSTORE_DIG_AMOUNT` / `dig.toml`'s `dig-amount`). The CLI stays DETERMINISTIC:
/// it never fetches a live price itself — it takes the amount as input and falls
/// back to this default when none is given. See [`resolve_dig_amount`].
pub const INIT_DIG: u64 = 100_000;
/// DEFAULT base units per root update (`commit`/`deploy`): 100 DIG. Same dynamic,
/// USD-pegged pricing model as [`INIT_DIG`] — uniform per capsule (fixed-size
/// capsule → same USD target → obfuscation preserved). Matches the hub web app's
/// `lib/dig.js` default.
pub const COMMIT_DIG: u64 = 100_000;

/// Resolve the DIG amount (base units) to spend for a mint/commit, DETERMINISTICALLY.
///
/// `explicit` carries the amount the CLI already resolved from its own precedence
/// (flag > env `DIGSTORE_DIG_AMOUNT` > `dig.toml`'s `dig-amount`); when `None`, fall
/// back to `default_units` (the [`INIT_DIG`]/[`COMMIT_DIG`] protocol default). This
/// function NEVER performs network I/O or a live price fetch — the dynamic,
/// USD-pegged amount is computed by the caller (the hub) and passed in, so the CLI's
/// spend is reproducible for a given input. An explicit `0` is rejected by the CLI
/// layer (a capsule must pay the protocol fee), so this just selects the source.
pub fn resolve_dig_amount(explicit: Option<u64>, default_units: u64) -> u64 {
    explicit.unwrap_or(default_units)
}

/// The treasury's inner (standard) puzzle hash, decoded from `TREASURY_ADDRESS`.
pub fn treasury_inner_puzzle_hash() -> Bytes32 {
    datalayer_driver::address_to_puzzle_hash(TREASURY_ADDRESS)
        .expect("TREASURY_ADDRESS is a valid xch address")
}

/// Format base units as a human DIG string (÷1000, 3 dp).
pub fn format_dig(base_units: u64) -> String {
    format!("{}.{:03}", base_units / 1000, base_units % 1000)
}

/// Parse a human DIG amount (a decimal string, max 3 dp — DIG has 3 decimals) into
/// base units. Accepts `"100"`, `"100.0"`, `"100.000"`, `"87.5"`; rejects negatives,
/// more than 3 decimal places, and non-numeric input. Used to parse the configurable
/// per-capsule DIG amount (`--dig-amount` / `DIGSTORE_DIG_AMOUNT` / dig.toml
/// `dig-amount`): the dynamic, USD-pegged amount the hub computes is expressed here as
/// DIG, then converted deterministically to base units for the spend.
pub fn parse_dig(s: &str) -> std::result::Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty DIG amount".to_string());
    }
    let (whole_str, frac_str) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if frac_str.len() > DIG_DECIMALS as usize {
        return Err(format!("DIG has at most {DIG_DECIMALS} decimal places"));
    }
    let whole: u64 = whole_str
        .parse()
        .map_err(|_| format!("`{s}` is not a valid DIG amount"))?;
    // Right-pad the fraction to exactly DIG_DECIMALS digits (e.g. "5" → "500").
    let frac: u64 = if frac_str.is_empty() {
        0
    } else {
        let padded = format!("{frac_str:0<width$}", width = DIG_DECIMALS as usize);
        padded
            .parse()
            .map_err(|_| format!("`{s}` is not a valid DIG amount"))?
    };
    let scale = 10u64.pow(DIG_DECIMALS);
    whole
        .checked_mul(scale)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(|| "DIG amount overflows".to_string())
}

/// Mojos per XCH: 1 XCH = 1_000_000_000_000 mojos (12 decimals).
pub const MOJOS_PER_XCH: u64 = 1_000_000_000_000;

/// Format mojos as a human XCH string (÷1e12, 12 dp), e.g.
/// `format_xch(903_384) == "0.000000903384"`.
pub fn format_xch(mojos: u64) -> String {
    format!("{}.{:012}", mojos / MOJOS_PER_XCH, mojos % MOJOS_PER_XCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn treasury_ph_decodes_and_is_32_bytes() {
        let ph = treasury_inner_puzzle_hash();
        assert_eq!(ph.to_bytes().len(), 32);
    }
    #[test]
    fn format_dig_renders_three_decimals() {
        assert_eq!(format_dig(100_000), "100.000");
        assert_eq!(format_dig(10_500), "10.500");
        assert_eq!(format_dig(1), "0.001");
    }
    #[test]
    fn parse_dig_roundtrips_and_validates() {
        assert_eq!(parse_dig("100").unwrap(), 100_000);
        assert_eq!(parse_dig("100.000").unwrap(), 100_000);
        assert_eq!(parse_dig("87.5").unwrap(), 87_500);
        assert_eq!(parse_dig("0.001").unwrap(), 1);
        assert_eq!(parse_dig(" 50 ").unwrap(), 50_000);
        // round-trips with format_dig
        assert_eq!(format_dig(parse_dig("12.345").unwrap()), "12.345");
        // rejects bad input
        assert!(parse_dig("").is_err());
        assert!(parse_dig("1.2345").is_err(), "more than 3 dp rejected");
        assert!(parse_dig("abc").is_err());
        assert!(parse_dig("-5").is_err());
    }

    #[test]
    fn resolve_dig_amount_uses_explicit_then_default() {
        // Explicit wins (the hub's dynamic, USD-pegged amount passed through).
        assert_eq!(resolve_dig_amount(Some(42_000), INIT_DIG), 42_000);
        // None → the protocol default (deterministic; no live fetch).
        assert_eq!(resolve_dig_amount(None, INIT_DIG), INIT_DIG);
        assert_eq!(resolve_dig_amount(None, COMMIT_DIG), 100_000);
    }

    #[test]
    fn format_xch_renders_twelve_decimals() {
        assert_eq!(format_xch(MOJOS_PER_XCH), "1.000000000000");
        assert_eq!(format_xch(903_384), "0.000000903384");
        assert_eq!(format_xch(1), "0.000000000001");
        assert_eq!(format_xch(0), "0.000000000000");
        assert_eq!(format_xch(1_500_000_000_000), "1.500000000000");
    }
}
