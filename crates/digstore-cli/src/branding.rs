//! Canonical user-facing branding constants (SYSTEM.md "Canonical terminology &
//! branding"). Centralized so the wordmark, the content-open scheme, the Get-$DIG
//! venues, and the support link cannot re-drift across the CLI's many help/output
//! surfaces.
//!
//! WHY a single module: these strings were previously hand-typed in dozens of
//! places, which let casing/scheme drift creep in (e.g. `DIGHub` vs the canonical
//! `DIGHUb`). One source of truth here means a sweep can never leave a straggler.
//!
//! EXEMPT — these constants do NOT cover the wire/transport surfaces, which stay
//! on `dig://`/`urn:dig:` by design: the §21 remote locator `dig://<host>/<id>`,
//! the `urn:dig:` namespace, and the on-chain NFT `data_uris`/`metadata_uris`.

/// The hub wordmark in user-facing prose: capital `U`, lowercase `b`. The domain
/// (`hub.dig.net`) and the code identifier (`dighub`) stay lowercase — this const
/// is for human-readable copy only.
pub const DIGHUB: &str = "DIGHUb";

/// The token sigil for the first user-facing reference in prose/help.
pub const DIG_TOKEN: &str = "$DIG";

/// Canonical community/support link (SYSTEM.md "Canonical Discord").
pub const DISCORD_URL: &str = "https://discord.gg/dignetwork";

/// The user-facing scheme a person types/clicks to OPEN verified DIG content —
/// what the DIG Browser/extension register. Distinct from the §21 remote scheme
/// (`dig://<host>/<id>`) and the `urn:dig:` namespace, which are NOT this.
pub const CONTENT_SCHEME: &str = "chia://";

/// The DIG CAT asset id (TAIL), for users who need to add the token by id (and to
/// build the venue deep-links below).
pub const DIG_ASSET_ID_HEX: &str =
    "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81";

/// Where to acquire `$DIG`, in the canonical order (SYSTEM.md "Get $DIG"; mirrors
/// the hub `apps/web/lib/links.js` `GET_DIG_SOURCES` — TibetSwap leads, then the
/// dexie and 9mm.pro per-CAT deep links). Each is `(label, url)`.
pub fn get_dig_sources() -> Vec<(&'static str, String)> {
    vec![
        ("TibetSwap", "https://v2.tibetswap.io/".to_string()),
        (
            "dexie.space",
            format!("https://dexie.space/offers/{DIG_ASSET_ID_HEX}/XCH"),
        ),
        (
            "xch.9mm.pro",
            format!("https://xch.9mm.pro/token/{DIG_ASSET_ID_HEX}"),
        ),
    ]
}

/// The canonical content-open address for a capsule resource: `chia://<storeId>:
/// <rootHash>/[<resource>]`. This is the address a user opens in the DIG Browser /
/// extension. It mirrors the on-chain NFT primary-URI path layout exactly EXCEPT
/// the scheme — the on-chain bytes stay `dig://` (wire contract); this displayed
/// form is `chia://`. Downstream (dig-sdk, deploy-action) match this form.
pub fn content_url(store_hex: &str, root_hex: &str, resource: &str) -> String {
    format!("{CONTENT_SCHEME}{store_hex}:{root_hex}/{resource}")
}

/// A one-line "Get $DIG" hint naming the three canonical venues in order. Shown
/// where the CLI dead-ends a user on DIG funds (insufficient-funds, doctor, setup).
pub fn get_dig_hint() -> String {
    let venues = get_dig_sources()
        .iter()
        .map(|(label, url)| format!("{label} ({url})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Get {DIG_TOKEN}: {venues}. DIG CAT id: {DIG_ASSET_ID_HEX}.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordmark_is_canonical_casing() {
        assert_eq!(DIGHUB, "DIGHUb");
    }

    #[test]
    fn content_scheme_is_chia() {
        assert_eq!(CONTENT_SCHEME, "chia://");
        assert_eq!(
            content_url("aa", "bb", "index.html"),
            "chia://aa:bb/index.html"
        );
    }

    #[test]
    fn get_dig_hint_names_three_venues_in_order() {
        let h = get_dig_hint();
        let tibet = h.find("TibetSwap").expect("TibetSwap present");
        let dexie = h.find("dexie.space").expect("dexie.space present");
        let nine = h.find("xch.9mm.pro").expect("xch.9mm.pro present");
        assert!(tibet < dexie && dexie < nine, "venues in canonical order");
        assert!(h.contains(DIG_TOKEN), "names the $DIG sigil");
        assert!(h.contains(DIG_ASSET_ID_HEX), "includes the DIG CAT id");
    }
}
