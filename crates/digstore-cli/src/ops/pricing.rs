//! Resolve the per-capsule $DIG amount a commit/deploy will DISPLAY and SPEND.
//!
//! The capsule price is dynamic + USD-pegged and computed by ONE canonical source
//! (the hub `/v1/pricing` endpoint — see [`digstore_chain::pricing`]). This module
//! resolves the amount for the CLI money-path:
//!
//! 1. An **explicit override** — `--dig-amount` flag > `DIGSTORE_DIG_AMOUNT` env >
//!    `dig.toml` `dig-amount` — always wins and stays DETERMINISTIC (no fetch). This
//!    is how CI / the hub / a power-user pins an exact amount.
//! 2. Otherwise, **fetch the live dynamic price** from the pricing endpoint and use
//!    it — the SAME `mint_dig` the hub charges.
//!
//! FAIL-LOUD: if no override is set and the price cannot be fetched,
//! [`resolve_capsule_price`] returns an error — a real-money commit never silently
//! spends a stale flat amount.

use crate::error::CliError;
use crate::runtime::block_on;

/// The pricing endpoint URL: `DIGSTORE_PRICING_URL` env override, else the canonical
/// public default. The env override lets a test point at a local mock server and a
/// custom deployment point at its own hub (explicit config always wins, §5.3 spirit).
pub fn pricing_url() -> String {
    if let Ok(u) = std::env::var("DIGSTORE_PRICING_URL") {
        let u = u.trim();
        if !u.is_empty() {
            return u.to_string();
        }
    }
    digstore_chain::pricing::DEFAULT_PRICING_URL.to_string()
}

/// The per-capsule DIG amount to display + spend, plus a human provenance note.
pub struct ResolvedCapsulePrice {
    /// Base units to spend (and disclose) for this capsule.
    pub base_units: u64,
    /// Live DIG→USD price (0.0 when an explicit override was used / unknown).
    pub dig_usd: f64,
    /// Provenance: `"override"` (explicit amount), `"dexie+coingecko"`, `"fallback"`, …
    pub source: String,
    /// True when the price came from the endpoint's conservative fallback (live
    /// market feed unavailable server-side) — surfaced as a warning.
    pub is_fallback: bool,
    /// A short human line describing where the amount came from (for the cost
    /// disclosure); `None` for a plain explicit override.
    pub note: Option<String>,
}

/// Resolve the per-capsule DIG amount for a real-money commit/deploy.
///
/// `explicit` is the already-resolved override (flag > env > `dig.toml`), or `None`.
/// With an override, returns it verbatim (deterministic). Otherwise fetches the live
/// dynamic price from the canonical hub endpoint and FAILS LOUD if it is unreachable
/// — never a silent flat fallback on a spend.
pub fn resolve_capsule_price(explicit: Option<u64>) -> Result<ResolvedCapsulePrice, CliError> {
    if let Some(base_units) = explicit {
        return Ok(ResolvedCapsulePrice {
            base_units,
            dig_usd: 0.0,
            source: "override".to_string(),
            is_fallback: false,
            note: None,
        });
    }
    let url = pricing_url();
    match block_on(digstore_chain::pricing::fetch_capsule_price(&url))? {
        Ok(p) => {
            let is_fallback = p.is_fallback();
            let note = if is_fallback {
                Some(
                    "using a FALLBACK $DIG price (live market feed unavailable) — pass \
                     --dig-amount <DIG> to set it explicitly"
                        .to_string(),
                )
            } else if p.dig_usd > 0.0 {
                Some(format!(
                    "live price: 1 DIG ≈ ${:.4} USD ({})",
                    p.dig_usd, p.source
                ))
            } else {
                None
            };
            Ok(ResolvedCapsulePrice {
                base_units: p.base_units,
                dig_usd: p.dig_usd,
                source: p.source,
                is_fallback,
                note,
            })
        }
        Err(e) => Err(CliError::Network(format!(
            "could not fetch the live per-capsule $DIG price: {e}. The capsule price is \
             dynamic (USD-pegged); re-run when the pricing endpoint is reachable, or pass \
             an explicit amount with --dig-amount <DIG> (or set DIGSTORE_DIG_AMOUNT)"
        ))),
    }
}

/// Best-effort current per-capsule price (base units) for a PREFLIGHT display
/// (`doctor` / `setup`) — NEVER errors: honors an explicit `DIGSTORE_DIG_AMOUNT`
/// override (keeps offline/CI runs deterministic), else a quiet live fetch, else
/// `None` (the caller shows a neutral "dynamic price" message). NOT for the spend
/// path — that uses [`resolve_capsule_price`], which fails loud.
pub fn current_capsule_price_quiet() -> Option<u64> {
    if let Some(u) = env_dig_amount() {
        return Some(u);
    }
    block_on(digstore_chain::pricing::fetch_capsule_price(&pricing_url()))
        .ok()
        .and_then(|r| r.ok())
        .map(|p| p.base_units)
}

/// Parse the `DIGSTORE_DIG_AMOUNT` env override (a human DIG decimal string) into
/// base units, or `None` when unset/empty/invalid.
fn env_dig_amount() -> Option<u64> {
    let s = std::env::var("DIGSTORE_DIG_AMOUNT").ok()?;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    digstore_chain::dig::parse_dig(s).ok()
}
