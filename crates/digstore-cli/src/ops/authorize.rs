//! Writer-authorization for a store's remote RPC (#172).
//!
//! An RPC (rpc.dig.net, a `digstore serve` node, a local dig-node) publishes its
//! §21.9 IDENTITY pubkey at `GET /.well-known/dig-rpc`. To let that RPC advance a
//! store's root on the owner's behalf, the OWNER authorizes the RPC's pubkey as a
//! WRITER: an owner-signed CHIP-0035 delegation update that adds
//! `writer_delegated_puzzle(pubkey)` to the store's delegated set (ownership
//! unchanged). Deauthorizing removes it. This module is the single source of truth
//! for that flow, used by both `digstore remote authorize`/`deauthorize` AND the
//! `digstore push` auto-prompt.
//!
//! The on-chain writer transform + the idempotent add/remove live in
//! `digstore_chain::singleton`; the build+sign+broadcast lives in
//! `digstore_chain::anchor::ChainAnchor::set_writer_authorization`. This module wires
//! discovery + wallet + UI around them.

use digstore_chain::singleton::PublicKey;
use digstore_remote::DigClient;

use crate::error::CliError;

/// Parse a 48-byte BLS G1 identity pubkey from its 96-hex string into the on-chain
/// `PublicKey` used to build the writer delegate. Rejects any non-96-hex / non-BLS
/// value — a caller MUST NOT build an authorization spend for a malformed key. The
/// SAME parse the CLI applies to a discovered well-known pubkey AND an explicit
/// `--pubkey` argument, so both paths validate identically.
pub fn parse_identity_pubkey(hex: &str) -> Result<PublicKey, CliError> {
    let s = hex.trim().to_ascii_lowercase();
    if s.len() != 96 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CliError::InvalidArgument(format!(
            "identity pubkey must be 48-byte (96-hex) BLS G1, got {} chars",
            s.len()
        )));
    }
    let bytes = digstore_core::Bytes48::from_hex(&s)
        .map_err(|_| CliError::InvalidArgument("identity pubkey is not valid hex".into()))?;
    PublicKey::from_bytes(&bytes.0).map_err(|_| {
        CliError::InvalidArgument("identity pubkey is not a valid BLS G1 point".into())
    })
}

/// Resolve the identity pubkey to authorize for `remote_url`: an explicit
/// `--pubkey <hex>` wins; otherwise discover it from the remote's
/// `/.well-known/dig-rpc`. Returns the validated on-chain `PublicKey` + its canonical
/// 96-hex form. A remote that advertises no discoverable identity (empty pubkey or a
/// 404) surfaces a clear, actionable error — the origin cannot be auto-authorized.
pub async fn resolve_writer_pubkey(
    remote_url: &str,
    explicit_pubkey: Option<&str>,
) -> Result<(PublicKey, String), CliError> {
    if let Some(hex) = explicit_pubkey {
        let pk = parse_identity_pubkey(hex)?;
        return Ok((pk, hex.trim().to_ascii_lowercase()));
    }
    let client = DigClient::new(remote_url.to_string());
    let discovered = client.discover_pubkey().await.map_err(|e| {
        CliError::Network(format!(
            "could not discover the RPC identity at {remote_url}/.well-known/dig-rpc: {e}. \
             Pass --pubkey <96-hex> to authorize a specific key."
        ))
    })?;
    let hex = discovered.ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "{remote_url} advertises no discoverable identity pubkey (empty well-known). \
             Pass --pubkey <96-hex> to authorize a specific key."
        ))
    })?;
    let pk = parse_identity_pubkey(&hex)?;
    Ok((pk, hex))
}

/// The store-owner p2 puzzle hash a `PublicKey`-derived origin authorization targets:
/// the WRITER delegate's inner-puzzle TreeHash is derived from the pubkey; this helper
/// is exposed so the CLI can print the exact delegate being added/removed.
pub fn writer_delegate_label(pubkey: PublicKey) -> String {
    // `DelegatedPuzzle::Writer(TreeHash)` — Debug renders the inner hash, giving a
    // stable, greppable identifier for the delegate in CLI/agent output.
    format!(
        "{:?}",
        digstore_chain::singleton::writer_delegated_puzzle(pubkey)
    )
}

/// Whether `pubkey` is currently an authorized writer of the store at `launcher_id`,
/// read from the live chain (syncs the singleton). Used by the push auto-prompt to
/// decide whether to offer authorization, and by `remote authorize` to short-circuit
/// a redundant spend. `chain` is the coinset reader.
pub async fn is_authorized_writer_onchain(
    chain: &dyn digstore_chain::coinset::ChainReads,
    launcher_id: chia_protocol::Bytes32,
    pubkey: PublicKey,
) -> Result<bool, CliError> {
    let store = digstore_chain::singleton::sync_datastore(chain, launcher_id)
        .await
        .map_err(|e| CliError::Chain(format!("sync store to read writers: {e}")))?;
    Ok(digstore_chain::singleton::is_authorized_writer(
        &store, pubkey,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real BLS G1 pubkey (48-byte) derived from a fixed seed, as 96-hex. Uses the
    // digstore-crypto BLS key (byte-identical G1 encoding to the chain `PublicKey`),
    // so it parses via `parse_identity_pubkey` into the chain type.
    fn sample_pubkey_hex() -> String {
        digstore_crypto::bls::SecretKey::from_seed(&[7u8; 32])
            .public_key()
            .to_bytes()
            .to_hex()
    }

    /// **Proves:** a valid 96-hex BLS pubkey parses; case + surrounding whitespace are
    /// normalized. **Catches:** a parser that rejects upper-case or padded input the
    /// well-known endpoint / a user might supply.
    #[test]
    fn parses_valid_pubkey_case_insensitive_and_trimmed() {
        let hex = sample_pubkey_hex();
        let padded = format!("  {}  ", hex.to_ascii_uppercase());
        let pk = parse_identity_pubkey(&padded).unwrap();
        // Round-trips to the same canonical lower-case hex (chain PublicKey → 48 bytes).
        assert_eq!(hex::encode(pk.to_bytes()), hex);
    }

    /// **Proves:** a wrong-length / non-hex value is REJECTED (no spend can be built
    /// for garbage). **Catches:** a silent accept of a malformed key.
    #[test]
    fn rejects_malformed_pubkey() {
        assert!(parse_identity_pubkey("").is_err());
        assert!(parse_identity_pubkey("aa").is_err()); // too short
        assert!(parse_identity_pubkey(&"zz".repeat(48)).is_err()); // non-hex
        assert!(parse_identity_pubkey(&"ab".repeat(50)).is_err()); // too long
    }

    /// **Proves:** an explicit `--pubkey` resolves WITHOUT any network call (no
    /// discovery), returning the parsed key + canonical hex.
    /// **Catches:** the explicit path accidentally hitting the well-known endpoint.
    #[tokio::test]
    async fn explicit_pubkey_bypasses_discovery() {
        let hex = sample_pubkey_hex();
        // An unreachable URL — if discovery were attempted this would error.
        let (pk, got_hex) = resolve_writer_pubkey("http://127.0.0.1:1/never", Some(&hex))
            .await
            .unwrap();
        assert_eq!(got_hex, hex);
        assert_eq!(hex::encode(pk.to_bytes()), hex);
    }

    /// **Proves:** the writer-delegate label is stable + non-empty (a greppable id for
    /// CLI output). **Catches:** an empty/placeholder label.
    #[test]
    fn writer_delegate_label_is_stable() {
        let pk = parse_identity_pubkey(&sample_pubkey_hex()).unwrap();
        let a = writer_delegate_label(pk);
        let b = writer_delegate_label(pk);
        assert_eq!(a, b);
        assert!(a.contains("Writer"));
    }
}
