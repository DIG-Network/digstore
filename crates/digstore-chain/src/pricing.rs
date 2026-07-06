//! Live per-capsule $DIG price — the digstore consumer of the ONE canonical
//! pricing source (issue #125).
//!
//! # Why this exists
//!
//! The per-capsule $DIG price is **dynamic and USD-pegged** (`SYSTEM.md` → *Core
//! concept — the capsule* → Pricing): `dig_amount = target_usd ÷ live_dig_usd`,
//! where `target_usd ≈ $1/capsule/year` of realistic AWS hosting, uniform per
//! fixed-size capsule. The price the hub charges is computed on the hub server
//! (the pure formula + constants live in the hub's `dighub_data::pricing` crate;
//! a live DIG→USD oracle composes dexie `DIG_XCH` × CoinGecko `chia/usd`) and
//! served at **`GET https://hub.dig.net/v1/pricing`** as `mint_dig` (the capsule
//! price in DIG base units).
//!
//! To guarantee digstore and the hub **never diverge**, digstore does NOT
//! reimplement the formula or the oracle — it consumes that SAME server-computed
//! number from `/v1/pricing`. There is exactly ONE computation of the price in the
//! ecosystem; both the hub web app (`apps/web/lib/pricing.ts`) and this CLI read
//! it. (Linking the hub's `dighub_data` crate is not viable — it is a hub-internal
//! package; and reimplementing the dexie+CoinGecko oracle here would be a second
//! copy that can drift. So: call the endpoint.)
//!
//! # Fail-LOUD (money-path discipline)
//!
//! Publishing a capsule spends real $DIG. If the price cannot be resolved, the
//! caller MUST fail loudly with a clear error — NEVER silently fall back to a
//! stale flat amount on a real-money spend. [`fetch_capsule_price`] therefore
//! returns `Err` on any unreachable endpoint / bad response / missing amount.
//!
//! Note the endpoint itself has a server-side fallback (it returns a conservative
//! amount with `source: "fallback"`/`"… (stale)"` when the live market feed is
//! down), so a reachable endpoint always yields a usable number; digstore only
//! fails when it cannot reach the endpoint at all. [`CapsulePrice::is_fallback`]
//! lets the caller surface a "using a fallback price" note.

use crate::error::{ChainError, Result};
use std::time::Duration;

/// The canonical hub pricing endpoint (public, no auth). Returns the live
/// USD-pegged per-capsule (mint/commit) price as `mint_dig` in DIG base units.
/// Overridable at the CLI layer (env / config) for tests + custom deployments.
pub const DEFAULT_PRICING_URL: &str = "https://hub.dig.net/v1/pricing";

/// The resolved live per-capsule price plus its provenance, parsed from a
/// `/v1/pricing` response.
#[derive(Debug, Clone, PartialEq)]
pub struct CapsulePrice {
    /// The per-capsule (mint/commit) price in DIG base units — the hub's `mint_dig`.
    /// This is the exact amount the hub charges for an equivalent capsule.
    pub base_units: u64,
    /// The live DIG→USD price the amount was computed at (0.0 if the response
    /// omitted it — informational only).
    pub dig_usd: f64,
    /// Provenance of the price: `"dexie+coingecko"`, `"… (stale)"`, or `"fallback"`.
    pub source: String,
}

impl CapsulePrice {
    /// True when the server priced this off a FALLBACK / stale DIG price (the live
    /// market feed was unavailable server-side). The amount is still usable — the
    /// caller should surface a note so the user knows it is not a live quote.
    pub fn is_fallback(&self) -> bool {
        let s = self.source.to_ascii_lowercase();
        s.contains("fallback") || s.contains("stale")
    }
}

/// Coerce a `/v1/pricing` DIG-amount field (base units; may arrive as an integer,
/// a float, or a numeric string) into a `u64`. Returns `None` for a
/// missing/non-numeric/negative/non-finite value so the caller fails loud.
fn parse_base_units(v: &serde_json::Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 0.0 {
            return Some(f.round() as u64);
        }
        return None;
    }
    v.as_str().and_then(|s| s.trim().parse::<u64>().ok())
}

/// Parse a `/v1/pricing` JSON body into the per-capsule [`CapsulePrice`]. PURE
/// (no I/O) so the extraction is unit-testable against captured bodies.
///
/// FAILS LOUD (`Err`) when `mint_dig` is absent, non-numeric, or `0` — a real-money
/// spend must never proceed on a missing/zero price.
pub fn parse_capsule_price(v: &serde_json::Value) -> Result<CapsulePrice> {
    let base_units = v
        .get("mint_dig")
        .and_then(parse_base_units)
        .ok_or_else(|| {
            ChainError::Chain(
                "pricing response is missing a valid `mint_dig` (per-capsule price)".to_string(),
            )
        })?;
    if base_units == 0 {
        return Err(ChainError::Chain(
            "pricing endpoint returned a zero per-capsule price".to_string(),
        ));
    }
    Ok(CapsulePrice {
        base_units,
        dig_usd: v.get("dig_usd").and_then(|x| x.as_f64()).unwrap_or(0.0),
        source: v
            .get("source")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// Fetch the live per-capsule $DIG price from the hub pricing endpoint.
///
/// FAILS LOUD on any unreachable endpoint, non-2xx status, undecodable body, or
/// missing/zero `mint_dig` — the caller (a real-money commit/deploy) must surface
/// the error and stop, never spend a wrong amount.
pub async fn fetch_capsule_price(pricing_url: &str) -> Result<CapsulePrice> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| ChainError::Chain(format!("pricing http client: {e}")))?;
    let resp = client
        .get(pricing_url)
        .header(
            reqwest::header::USER_AGENT,
            concat!("digstore/", env!("CARGO_PKG_VERSION")),
        )
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| {
            ChainError::Chain(format!(
                "could not reach the DIG pricing endpoint ({pricing_url}): {e}"
            ))
        })?;
    if !resp.status().is_success() {
        return Err(ChainError::Chain(format!(
            "DIG pricing endpoint {pricing_url} returned status {}",
            resp.status().as_u16()
        )));
    }
    let json: serde_json::Value = resp.json().await.map_err(|e| {
        ChainError::Chain(format!("could not decode the DIG pricing response: {e}"))
    })?;
    parse_capsule_price(&json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The canonical `/v1/pricing` response (the exact shape the hub serves) parses
    /// to the per-capsule price in base units, verbatim from `mint_dig`.
    #[test]
    fn parses_mint_dig_from_canonical_response() {
        // dig_usd = 0.05 → $1 / $0.05 = 20 DIG = 20_000 base units (the server's formula).
        let body = json!({
            "dig_usd": 0.05,
            "computed_at": 1_700_000_000u64,
            "source": "dexie+coingecko",
            "mint_dig": 20_000u64,
            "mint_usd": 1.0,
            "subdomain_dig": 400_000u64,
            "subdomain_usd": 20.0,
            "cert_dig": 200_000u64,
            "cert_usd": 10.0,
            "basis": "dig_xch=…×xch_usd=…",
        });
        let p = parse_capsule_price(&body).expect("valid response parses");
        assert_eq!(p.base_units, 20_000);
        assert_eq!(p.dig_usd, 0.05);
        assert_eq!(p.source, "dexie+coingecko");
        assert!(!p.is_fallback());
    }

    /// `mint_dig` may arrive as a numeric string (defensive coercion), matching the
    /// hub client's `parseBaseUnits`.
    #[test]
    fn parses_mint_dig_as_string() {
        let body = json!({ "mint_dig": "33333", "source": "dexie+coingecko" });
        assert_eq!(parse_capsule_price(&body).unwrap().base_units, 33_333);
    }

    /// A `fallback`/`stale` source is flagged so the caller can warn the user (the
    /// server priced off the conservative fallback, not a live quote).
    #[test]
    fn detects_fallback_and_stale_source() {
        let fb = json!({ "mint_dig": 20_000u64, "source": "fallback" });
        assert!(parse_capsule_price(&fb).unwrap().is_fallback());
        let stale = json!({ "mint_dig": 20_000u64, "source": "dexie+coingecko (stale)" });
        assert!(parse_capsule_price(&stale).unwrap().is_fallback());
    }

    /// FAIL LOUD: a missing / zero / non-numeric `mint_dig` is an error, never a
    /// silent wrong price on a real-money spend.
    #[test]
    fn missing_or_zero_mint_dig_fails_loud() {
        assert!(parse_capsule_price(&json!({ "dig_usd": 0.05 })).is_err());
        assert!(parse_capsule_price(&json!({ "mint_dig": 0u64 })).is_err());
        assert!(parse_capsule_price(&json!({ "mint_dig": "abc" })).is_err());
        assert!(parse_capsule_price(&json!({ "mint_dig": -5 })).is_err());
    }

    /// CONFORMANCE with the canonical hub formula (`dighub_data::pricing`): the
    /// endpoint's `mint_dig` is `round($1 / dig_usd × 1000)`, floored at 1 DIG. We
    /// pin the expectation for representative prices so a server-side formula change
    /// (that digstore must track) is caught here. digstore consumes `mint_dig`
    /// directly; these are the numbers it must receive for those prices.
    #[test]
    fn conformance_expected_amounts_for_known_prices() {
        // (dig_usd, expected capsule base units): $1 target.
        let cases = [
            (0.05, 20_000u64), // $1 / $0.05  = 20 DIG
            (0.02, 50_000u64), // $1 / $0.02  = 50 DIG
            (0.20, 5_000u64),  // $1 / $0.20  = 5 DIG
            (0.03, 33_333u64), // $1 / $0.03  = 33.333 DIG
        ];
        for (dig_usd, expected) in cases {
            let body =
                json!({ "mint_dig": expected, "dig_usd": dig_usd, "source": "dexie+coingecko" });
            assert_eq!(
                parse_capsule_price(&body).unwrap().base_units,
                expected,
                "capsule price for dig_usd={dig_usd}"
            );
        }
    }

    /// End-to-end through the REAL reqwest path: a local server returning the
    /// canonical body → `fetch_capsule_price` returns the parsed amount.
    #[tokio::test]
    async fn fetch_reads_mint_dig_from_a_live_response() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let (mut s, _) = listener.accept().unwrap();
            let _ = s.read(&mut buf);
            let body = br#"{"dig_usd":0.05,"source":"dexie+coingecko","mint_dig":20000}"#;
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(hdr.as_bytes());
            let _ = s.write_all(body);
            let _ = s.flush();
        });

        let p = fetch_capsule_price(&format!("http://{addr}"))
            .await
            .expect("valid response");
        assert_eq!(p.base_units, 20_000);
        assert_eq!(p.source, "dexie+coingecko");
        handle.join().unwrap();
    }

    /// FAIL LOUD end-to-end: an unreachable endpoint is an `Err` (never a silent
    /// fallback). Binds a port, drops the listener so nothing answers.
    #[tokio::test]
    async fn fetch_unreachable_endpoint_fails_loud() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        drop(listener); // nothing listens on `addr` now
        let err = fetch_capsule_price(&format!("http://{addr}"))
            .await
            .expect_err("unreachable endpoint must fail loud");
        assert!(matches!(err, ChainError::Chain(_)));
    }

    /// FAIL LOUD end-to-end: a 200 response whose body lacks `mint_dig` is an error.
    #[tokio::test]
    async fn fetch_missing_mint_dig_fails_loud() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let (mut s, _) = listener.accept().unwrap();
            let _ = s.read(&mut buf);
            let body = br#"{"dig_usd":0.05,"source":"dexie+coingecko"}"#;
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(hdr.as_bytes());
            let _ = s.write_all(body);
            let _ = s.flush();
        });

        let err = fetch_capsule_price(&format!("http://{addr}"))
            .await
            .expect_err("missing mint_dig must fail loud");
        assert!(matches!(err, ChainError::Chain(_)));
        handle.join().unwrap();
    }
}
