//! DIG token (CAT) constants + amount helpers (see the DIG-CAT-payment design).
//!
//! # Per-capsule $DIG payment — enforcement model (#130)
//!
//! A capsule (a commit / root-advance) MUST pay the per-capsule $DIG price to the
//! DIG treasury; minting a store is free of $DIG (only the XCH network fee). How
//! that invariant is enforced today, and the explicit boundary of that
//! enforcement, so it is never mistaken for more than it is:
//!
//! - **Builder/validation gate (this crate).** Every commit bundle the anchor
//!   builds is checked by [`verify_commit_pays_dig_treasury`] before it is signed
//!   and returned — it FAILS CLOSED if the bundle does not pay the treasury. This
//!   prevents the digstore builder from ever emitting a silent FREE root-advance
//!   (a builder bug that dropped the payment is a hard error, not a free commit).
//! - **Client-side atomicity.** The DIG payment and the singleton update are one
//!   co-signed spend bundle, admitted all-or-nothing by the mempool — so an
//!   accepted root-advance carried its payment.
//! - **Off-chain / network gate.** The off-chain anchor-watcher (hub side) is the
//!   network-level gate that observes the treasury coin; per-capsule *pricing* (the
//!   dynamic, USD-pegged amount) is a business-layer policy resolved by the caller
//!   (the hub computes the live amount; the CLI takes it as input — see
//!   [`COMMIT_DIG`]).
//! - **NOT a protocol-level on-chain invariant (yet).** There is no on-chain CLVM
//!   coupling that *forces* a singleton update to assert the DIG payment (e.g. an
//!   announcement only the treasury payment can emit). A direct caller bypassing
//!   this builder could still hand-roll a bundle that advances a root without
//!   paying. Adding that on-chain coupling is a separate, explicitly-versioned
//!   protocol event (it would change the chip35 spend puzzle + require a
//!   release-first), tracked as such — not silently assumed here.
use crate::error::{ChainError, Result};
use chia_protocol::Bytes32;

/// DIG CAT asset id (mainnet). Matches DataLayer-Driver `DIG_ASSET_ID`.
pub const DIG_ASSET_ID: Bytes32 = Bytes32::new(hex_literal::hex!(
    "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81"
));

/// DIG treasury recipient (bech32 `xch1…`); DIG is sent to this address's CAT ph.
pub const TREASURY_ADDRESS: &str = "xch1a37rq3cgcl2ecpudttsf35x75qzdan68lgw2l6ajvmqs44jxdn5qv6pk3y";

/// DIG has 3 decimals: 1 DIG = 1000 base units.
pub const DIG_DECIMALS: u32 = 3;
/// DEFAULT base units per root update (`commit`/`deploy`) — a CAPSULE — 100 DIG.
///
/// **Minting a store is FREE of $DIG** (#111, SYSTEM.md → "DIG CAT payment"): the mint
/// only launches the on-chain singleton (empty/initial root) + the XCH network fee, so
/// there is no `INIT_DIG` mint cost — the DIG payment is attached ONLY to a commit /
/// root-advance (= a capsule). This is a DEFAULT, not a hard constant: the per-capsule
/// price is **dynamic and USD-pegged** (SYSTEM.md — `dig_amount = target_usd ÷ live DIG
/// price`, where `target_usd ≈ $1/capsule/year` of realistic AWS hosting; uniform per
/// fixed-size capsule → same USD target → obfuscation preserved). The hub computes that
/// live amount in the browser and the CLI accepts it explicitly (`--dig-amount` /
/// `DIGSTORE_DIG_AMOUNT` / `dig.toml`'s `dig-amount`). The CLI stays DETERMINISTIC: it
/// never fetches a live price itself — it takes the amount as input and falls back to
/// this default when none is given. See [`resolve_dig_amount`]. Matches the hub web
/// app's `lib/dig.js` default and chip35's per-capsule payment.
pub const COMMIT_DIG: u64 = 100_000;

/// Resolve the DIG amount (base units) to spend for a mint/commit, DETERMINISTICALLY.
///
/// `explicit` carries the amount the CLI already resolved from its own precedence
/// (flag > env `DIGSTORE_DIG_AMOUNT` > `dig.toml`'s `dig-amount`); when `None`, fall
/// back to `default_units` (the [`COMMIT_DIG`] protocol default). This
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

/// True if `needle` appears contiguously in `haystack`.
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// True if any spend in `coin_spends` pays $DIG to the DIG treasury — the DIG
/// payment's CAT spend commits to the treasury INNER puzzle hash (it appears in
/// the spend's serialized `puzzle_reveal || solution` as the CREATE_COIN
/// recipient / hint memo), while mint/update singleton spends never reference the
/// treasury. A keyless byte-signal, byte-mirror of chip35's `bundle_pays_dig_treasury`
/// — it needs no on-chain CLVM/lineage, matching chip35.
pub fn bundle_pays_dig_treasury(coin_spends: &[chia_protocol::CoinSpend]) -> bool {
    let needle = treasury_inner_puzzle_hash().to_bytes();
    coin_spends.iter().any(|cs| {
        contains_subslice(cs.puzzle_reveal.as_ref(), &needle)
            || contains_subslice(cs.solution.as_ref(), &needle)
    })
}

/// Enforce the per-capsule $DIG payment INVARIANT at commit-bundle validation
/// (#130): a commit / root-advance MUST carry the DIG treasury payment, so this
/// FAILS CLOSED (`ChainError::Chain`) if `coin_spends` does not pay the treasury.
///
/// This is the protocol's enforcement point for per-capsule pricing on the
/// builder/validation path: every commit-bundle the anchor produces is checked
/// to actually pay the treasury before it is signed/returned, so a code path that
/// ever dropped or mis-built the payment yields a hard error rather than a silent
/// FREE root-advance. It does not replace an on-chain coupling (see the note on
/// [`COMMIT_DIG`] / SYSTEM.md): the DIG-payment+singleton-update atomicity is
/// still a client-side co-signed-bundle convention, and the off-chain
/// anchor-watcher remains the network-side gate; full on-chain enforcement
/// (a singleton-update announcement only the DIG payment can assert) is a
/// separate, explicitly-versioned protocol event. What this guarantees is that
/// the digstore builder never emits a commit bundle that silently omits the
/// payment.
pub fn verify_commit_pays_dig_treasury(coin_spends: &[chia_protocol::CoinSpend]) -> Result<()> {
    if bundle_pays_dig_treasury(coin_spends) {
        Ok(())
    } else {
        Err(ChainError::Chain(
            "commit bundle does not pay the per-capsule $DIG price to the treasury \
             (a root-advance must carry the DIG payment)"
                .to_string(),
        ))
    }
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

    /// The DIG asset id + treasury inner puzzle hash are a cross-system shared contract
    /// and MUST be byte-identical to chip35's `DIG_ASSET_ID` /
    /// `DIG_TREASURY_INNER_PUZZLE_HASH` (the canonical spend builder, #111). A drift here
    /// breaks payment atomicity / anchor-watcher gating, so pin the exact bytes.
    #[test]
    fn dig_constants_match_chip35_cross_system_contract() {
        assert_eq!(
            hex::encode(DIG_ASSET_ID),
            "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81"
        );
        assert_eq!(
            hex::encode(treasury_inner_puzzle_hash()),
            "ec7c304708c7d59c078d5ae098d0dea004decf47fa1cafebb266c10ad6466ce8"
        );
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
        assert_eq!(resolve_dig_amount(Some(42_000), COMMIT_DIG), 42_000);
        // None → the protocol default (deterministic; no live fetch).
        assert_eq!(resolve_dig_amount(None, COMMIT_DIG), COMMIT_DIG);
        assert_eq!(resolve_dig_amount(None, COMMIT_DIG), 100_000);
    }

    // -- #130 per-capsule $DIG payment enforcement at commit validation --------

    /// A commit bundle that does NOT pay the treasury fails closed — the
    /// enforcement point that prevents a silent FREE root-advance.
    #[test]
    fn verify_commit_rejects_a_bundle_with_no_treasury_payment() {
        use chia_protocol::{Coin, CoinSpend, Program};
        // A spend whose puzzle/solution does not contain the treasury inner ph.
        let spend = CoinSpend::new(
            Coin::new(Bytes32::default(), Bytes32::default(), 1),
            Program::from(vec![0x01u8, 0x02, 0x03]),
            Program::from(vec![0x04u8, 0x05]),
        );
        let err = verify_commit_pays_dig_treasury(&[spend]).unwrap_err();
        assert!(
            format!("{err}").contains("does not pay the per-capsule $DIG price"),
            "no-payment bundle must fail closed: {err}"
        );
    }

    /// A spend that commits to the treasury inner puzzle hash (the DIG payment's
    /// CREATE_COIN recipient/hint) passes the gate.
    #[test]
    fn verify_commit_accepts_a_bundle_paying_the_treasury() {
        use chia_protocol::{Coin, CoinSpend, Program};
        // Embed the treasury inner ph bytes in the solution (the keyless signal a
        // real DIG payment carries as the CREATE_COIN recipient + hint memo).
        let needle = treasury_inner_puzzle_hash().to_bytes().to_vec();
        let mut solution = vec![0xAAu8, 0xBB];
        solution.extend_from_slice(&needle);
        let spend = CoinSpend::new(
            Coin::new(Bytes32::default(), Bytes32::default(), 1),
            Program::from(vec![0x01u8]),
            Program::from(solution),
        );
        assert!(verify_commit_pays_dig_treasury(&[spend]).is_ok());
        assert!(!bundle_pays_dig_treasury(&[]), "empty bundle pays nobody");
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
