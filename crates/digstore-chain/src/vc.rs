//! Verifiable Credentials — the wallet's Sage-parity VC surface, built on the
//! chia-wallet-sdk 0.30 **verification** primitive (the CHIP verification layer):
//! an issuer (a DID singleton) MINTS a `Verification` singleton that attests
//! [`VerifiedData`] about an asset (an on-chain credential / attestation), anyone can
//! PROVE a payment carries that verified data via a [`VerificationAsserter`], and the
//! issuing DID can REVOKE (melt) the verification.
//!
//! Built on [`Verification`]/[`VerifiedData`]/[`VerificationAsserter`]. Like the other
//! chain modules this is **pure build (+ sign)** — builders return UNSIGNED
//! `Vec<CoinSpend>`; NOTHING here broadcasts.
//!
//! ## VC model in 0.30 (what "mint / verify / revoke" map to)
//! chia-wallet-sdk 0.30 does NOT expose a CHIP-0042/CR-CAT "credential coin with
//! transfer". What it DOES expose — and what this module wires up — is the
//! **verification layer**, the canonical on-chain attestation:
//!   * **issue (mint)** — a DID launches a `Verification` singleton carrying
//!     [`VerifiedData`] (a version, the subject `asset_id`, a `data_hash` commitment,
//!     and a free-text `comment`), revocable by that DID. This is the credential.
//!   * **prove (verify)** — a [`VerificationAsserter`] lets a coin REQUIRE that a valid
//!     verification exists for `(issuer_did, version, asset_id, data_hash)` before it can
//!     be spent; [`vc_asserter`] / [`vc_asserter_puzzle_hash`] build that gate so a
//!     relying party can verify a credential on-chain.
//!   * **revoke** — the issuing DID melts the verification singleton (sending it the
//!     revocation message), invalidating the credential.
//!
//! There is no separate "transfer": a verification is an issuer-anchored attestation,
//! not a bearer token, so re-attribution is re-issuance by the (same or a new) DID. That
//! is an honest mapping of VC operations onto the primitive 0.30 actually ships — see the
//! note returned by [`vc_transfer_unsupported`].

use crate::error::{ChainError, Result};
use chia::clvm_utils::TreeHash;
use chia_protocol::{Bytes32, Coin, CoinSpend};
use chia_wallet_sdk::driver::{
    Launcher, SpendContext, Verification, VerificationAsserter, VerifiedData,
};
use chia_wallet_sdk::prelude::ToTreeHash;

/// The credential payload an issuer attests: a version, the subject `asset_id` (what the
/// credential is ABOUT — e.g. a CAT/NFT/store launcher id), a `data_hash` committing to
/// the off-chain credential document, and a human-readable `comment`. Mirrors the SDK
/// [`VerifiedData`] one-to-one; surfaced here so callers construct credentials without
/// importing the driver type directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialData {
    /// Schema/format version of this credential (1 for the current verification layer).
    pub version: u32,
    /// The subject the credential is about (e.g. an asset / store launcher id).
    pub asset_id: Bytes32,
    /// A commitment (tree/hash) to the off-chain credential document.
    pub data_hash: Bytes32,
    /// Free-text note carried on-chain with the attestation.
    pub comment: String,
}

impl CredentialData {
    /// Convert to the SDK [`VerifiedData`] used by the verification layer.
    pub fn to_verified_data(&self) -> VerifiedData {
        VerifiedData {
            version: self.version,
            asset_id: self.asset_id,
            data_hash: self.data_hash,
            comment: self.comment.clone(),
        }
    }

    /// Construct from the SDK [`VerifiedData`].
    pub fn from_verified_data(v: &VerifiedData) -> Self {
        Self {
            version: v.version,
            asset_id: v.asset_id,
            data_hash: v.data_hash,
            comment: v.comment.clone(),
        }
    }
}

/// A minted verifiable credential: the [`Verification`] singleton (the on-chain
/// attestation) plus the issuing DID launcher id (the revocation authority). Returned by
/// [`build_vc_issue`] so the caller can later prove or revoke it.
#[derive(Clone, Debug)]
pub struct MintedCredential {
    /// The verification singleton (carries the [`VerifiedData`]); its launcher id is the
    /// credential's stable identity.
    pub verification: Verification,
    /// The issuing DID launcher id — the singleton allowed to revoke this credential.
    pub issuer_did: Bytes32,
}

/// Build the (UNSIGNED) coin spends that ISSUE a verifiable credential carrying
/// `credential`, launched off `launcher_parent` (a coin the issuer DID spend creates at
/// the singleton-launcher puzzle hash) and revocable by `issuer_did`. Returns the spends
/// and the [`MintedCredential`].
///
/// The verification singleton is created via [`Verification::after_mint`] and the
/// launcher coin is spent to its inner puzzle hash; the resulting verification is then
/// spent in **oracle mode** (no revocation) so it stands as a live attestation. The
/// caller MUST also spend `issuer_did` in the SAME bundle to create the
/// `launcher_parent` coin (a `CREATE_COIN(SINGLETON_LAUNCHER_HASH, 0)` from the DID) and
/// to fund/announce as needed — that DID spend holds the issuer's keys and is the
/// caller's responsibility, mirroring how [`crate::nft`] leaves the attributing DID spend
/// to the caller. `launcher_parent` is the DID coin's id (the launcher coin's parent).
///
/// **Pure: does NOT sign or broadcast.**
pub fn build_vc_issue(
    ctx: &mut SpendContext,
    launcher_parent: Bytes32,
    issuer_did: Bytes32,
    credential: &CredentialData,
) -> Result<MintedCredential> {
    let verified_data = credential.to_verified_data();

    // Launch the verification singleton off the launcher coin the DID will create.
    let launcher = Launcher::new(launcher_parent, 0).with_singleton_amount(1);
    let verification = Verification::after_mint(launcher_parent, issuer_did, verified_data.clone());

    let inner_ph: Bytes32 = Verification::inner_puzzle_hash(issuer_did, &verified_data).into();
    let (_conds, new_coin) = launcher
        .spend(ctx, inner_ph, ())
        .map_err(|e| ChainError::Chain(format!("spend verification launcher: {e}")))?;

    if new_coin != verification.coin {
        return Err(ChainError::Chain(
            "verification launcher produced an unexpected coin".into(),
        ));
    }

    // Spend the verification in oracle mode (no revocation) so it stands as a live
    // attestation that a relying party can assert against.
    verification
        .clone()
        .spend(ctx, None)
        .map_err(|e| ChainError::Chain(format!("oracle-spend verification: {e}")))?;

    Ok(MintedCredential {
        verification,
        issuer_did,
    })
}

/// Build the (UNSIGNED) spend that REVOKES `verification` — melts the verification
/// singleton — authorized by `revocation_singleton_inner_puzzle_hash`, the CURRENT inner
/// puzzle hash of the issuing DID. The DID must, in the SAME bundle, send the revocation
/// message to the verification coin (a `send_message(18, …)` carrying the verification
/// coin's puzzle hash); that DID spend holds the issuer's keys and is the caller's
/// responsibility.
///
/// **Pure: does NOT sign or broadcast.**
pub fn build_vc_revoke(
    ctx: &mut SpendContext,
    verification: Verification,
    revocation_singleton_inner_puzzle_hash: Bytes32,
) -> Result<()> {
    verification
        .spend(ctx, Some(revocation_singleton_inner_puzzle_hash))
        .map_err(|e| ChainError::Chain(format!("revoke verification: {e}")))
}

/// Build a [`VerificationAsserter`] for the credential `(issuer_did, version, asset_id,
/// data_hash)` — the on-chain gate a relying party curries into a coin so that coin can
/// only be spent if a matching valid verification exists. This is the "verify a
/// credential" primitive: prove (on-chain) that a verification was issued by `issuer_did`
/// for that asset + data, at that version.
pub fn vc_asserter(
    issuer_did: Bytes32,
    version: u32,
    asset_id: Bytes32,
    data_hash: Bytes32,
) -> VerificationAsserter {
    VerificationAsserter::from(
        issuer_did,
        version,
        asset_id.tree_hash(),
        data_hash.tree_hash(),
    )
}

/// The puzzle hash of the [`vc_asserter`] for a credential — the address a relying party
/// sends a coin to so spending it REQUIRES proof of the credential. A convenience over
/// `vc_asserter(...).tree_hash()`.
pub fn vc_asserter_puzzle_hash(
    issuer_did: Bytes32,
    version: u32,
    asset_id: Bytes32,
    data_hash: Bytes32,
) -> Bytes32 {
    let th: TreeHash = vc_asserter(issuer_did, version, asset_id, data_hash).tree_hash();
    th.into()
}

/// The launcher id (stable identity) of a verification, suitable as the credential's id.
pub fn credential_id(minted: &MintedCredential) -> Bytes32 {
    minted.verification.info.launcher_id
}

/// VC "transfer" is intentionally unsupported: a verification is an issuer-anchored
/// attestation, not a bearer token, so it cannot be reassigned to a new holder. Callers
/// asking to transfer a credential should RE-ISSUE it (a new [`build_vc_issue`] under the
/// desired issuing DID). Returns a clear error so the wallet surfaces an honest message
/// rather than silently doing nothing.
pub fn vc_transfer_unsupported() -> ChainError {
    ChainError::Chain(
        "verifiable credentials are issuer-anchored attestations and cannot be transferred; \
         re-issue the credential under the desired DID instead"
            .into(),
    )
}

/// Sign assembled VC `coin_spends` with `keys`, returning the aggregated signature.
/// A thin convenience over [`datalayer_driver::sign_coin_spends`], mirroring
/// [`crate::nft::sign_nft_spends`] (the issuing DID spend, bundled with the VC spends, is
/// what carries the signature requirement).
pub fn sign_vc_spends(
    coin_spends: &[CoinSpend],
    keys: &[datalayer_driver::SecretKey],
    for_testnet: bool,
) -> Result<datalayer_driver::Signature> {
    datalayer_driver::sign_coin_spends(coin_spends, keys, for_testnet)
        .map_err(|e| ChainError::Chain(format!("sign vc spends: {e}")))
}

/// The 0-mojo verification-launcher coin the issuing DID must create to mint a
/// credential: a `CREATE_COIN(SINGLETON_LAUNCHER_HASH, 0)` from the DID coin. Exposed so
/// the caller wires the right launcher coin into [`build_vc_issue`] (`launcher_parent` is
/// the DID coin id; this is the coin that id parents). Derived via [`Launcher`] so the
/// crate does not hard-code the launcher puzzle hash.
pub fn vc_launcher_coin(did_coin_id: Bytes32) -> Coin {
    Launcher::new(did_coin_id, 0).coin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chia::puzzles::Memos;
    use chia_protocol::Bytes;
    use chia_puzzles::SINGLETON_LAUNCHER_HASH;
    use chia_sdk_test::Simulator;
    use chia_wallet_sdk::driver::{
        Launcher as DriverLauncher, Puzzle, SingletonInfo, StandardLayer,
    };
    use chia_wallet_sdk::types::Conditions;

    // ----- offline: credential data + asserter determinism + transfer gap -----

    #[test]
    fn credential_data_round_trips_verified_data() {
        let c = CredentialData {
            version: 1,
            asset_id: Bytes32::from([7; 32]),
            data_hash: Bytes32::from([9; 32]),
            comment: "store ownership attestation".into(),
        };
        let v = c.to_verified_data();
        assert_eq!(CredentialData::from_verified_data(&v), c);
    }

    #[test]
    fn asserter_puzzle_hash_is_deterministic() {
        let did = Bytes32::from([1; 32]);
        let asset = Bytes32::from([2; 32]);
        let data = Bytes32::from([3; 32]);
        let a = vc_asserter_puzzle_hash(did, 1, asset, data);
        let b = vc_asserter_puzzle_hash(did, 1, asset, data);
        assert_eq!(a, b, "the asserter puzzle hash must be deterministic");
        // A different data hash yields a different gate.
        let c = vc_asserter_puzzle_hash(did, 1, asset, Bytes32::from([4; 32]));
        assert_ne!(a, c);
    }

    #[test]
    fn transfer_is_unsupported_with_a_clear_message() {
        let err = vc_transfer_unsupported();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("cannot be transferred")),
            "got: {err}"
        );
    }

    // ----- Simulator: issue (mint) a credential under a DID, then revoke it -----

    /// Issue a verifiable credential under a freshly-created DID (the issuer), prove the
    /// minted verification carries the attested data and is anchored to the DID, then
    /// REVOKE it (the DID melts the verification). Drives `build_vc_issue` +
    /// `build_vc_revoke` end-to-end on the in-process Chia simulator, following the
    /// canonical verification-layer lifecycle.
    #[test]
    fn issue_then_revoke_credential() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();

        // The issuer's DID (BLS-backed, as the verification authority).
        let alice = sim.bls(1);
        let alice_p2 = StandardLayer::new(alice.pk);
        let (create_did, did) =
            DriverLauncher::new(alice.coin.coin_id(), 1).create_simple_did(ctx, &alice_p2)?;
        alice_p2.spend(ctx, alice.coin, create_did)?;

        // The DID spends to create the 0-mojo verification-launcher coin; capture the
        // proof so the asserter could verify later.
        let did = did.update(
            ctx,
            &alice_p2,
            Conditions::new().create_coin(SINGLETON_LAUNCHER_HASH.into(), 0, Memos::None),
        )?;

        // Issue the credential off that launcher coin (parented by the DID's prior coin).
        let credential = CredentialData {
            version: 1,
            asset_id: Bytes32::from([2; 32]),
            data_hash: Bytes32::from([3; 32]),
            comment: "Sage-parity VC test attestation".into(),
        };
        let launcher_parent = did.coin.parent_coin_info;
        let minted = build_vc_issue(ctx, launcher_parent, did.info.launcher_id, &credential)?;

        // The minted verification carries our attested data and is anchored to the DID.
        assert_eq!(
            CredentialData::from_verified_data(&minted.verification.info.verified_data),
            credential
        );
        assert_eq!(minted.issuer_did, did.info.launcher_id);
        assert_eq!(
            minted.verification.info.revocation_singleton_launcher_id,
            did.info.launcher_id
        );

        // Re-parse the verification from its own (oracle) spend, mirroring how a relying
        // party reconstructs it from chain data, then REVOKE it: the DID sends the
        // revocation message and the verification is melted in the same bundle.
        let parent_puzzle = minted.verification.construct_puzzle(ctx)?;
        let parent_puzzle = Puzzle::parse(ctx, parent_puzzle);
        let verification =
            Verification::from_parent_spend(ctx, minted.verification.coin, parent_puzzle)?
                .expect("verification should re-parse from its oracle spend");

        let revocation_inner_ph: Bytes32 = did.info.inner_puzzle_hash().into();
        let msg_data = ctx.alloc(&verification.coin.puzzle_hash)?;
        let _did = did.update(
            ctx,
            &alice_p2,
            Conditions::new().send_message(18, Bytes::default(), vec![msg_data]),
        )?;
        build_vc_revoke(ctx, verification, revocation_inner_ph)?;

        // The whole lifecycle (DID create + update + verification mint + oracle + revoke)
        // settles as valid transactions on the simulator.
        sim.spend_coins(ctx.take(), &[alice.sk])?;
        Ok(())
    }
}
