//! Vaults — the wallet's Sage-parity MIPS multi-key wallet primitive (CHIP-0043):
//! a singleton whose custody is an m-of-n set of secp256k1 (K1) member keys. Create a
//! vault under a key configuration and spend from it by satisfying that configuration.
//!
//! Built on the chia-wallet-sdk 0.30 vault primitive ([`Vault`]) and the MIPS spend
//! machinery ([`MipsSpend`]/[`MofN`]/[`InnerPuzzleSpend`]/[`mips_puzzle_hash`]). Like
//! the other chain modules this is **pure build (+ sign)** — builders return UNSIGNED
//! `Vec<CoinSpend>` (the vault spend carries its K1 signatures inline, so it needs no
//! aggregated BLS signature). NOTHING here broadcasts.
//!
//! ## Key model — why K1, not the wallet's BLS keys
//! The MIPS member exercised end-to-end by the 0.30 simulator surface is the **K1
//! (secp256k1) member**: a member curries a K1 public key and its solution carries a K1
//! signature over `sha256(delegated_puzzle_hash || vault_coin_id)`. The primitive ALSO
//! ships a `BlsMember` (which would let a vault be guarded by the wallet's BLS synthetic
//! keys), but 0.30's tested/verifiable spend path is K1, so this module builds and
//! verifies the K1 configuration. A vault's K1 keys are an independent custody set the
//! wallet manages alongside (not derived from) its BLS spend key.
//!
//! ## What is implemented vs. documented as a gap
//! Implemented + simulator-verified:
//!   * **create** a vault under a **1-of-1** or **m-of-n** K1 member set, and
//!   * **spend** the vault by satisfying that set (re-creating the vault, the canonical
//!     "move funds under custody" shape).
//!
//! Documented gaps (present in the 0.30 API but NOT exercised by its test surface, so
//! NOT built here to avoid shipping unverified spend code — see [`VaultKeyConfig`]):
//!   * **Restrictions** (`Restriction`/`RestrictionKind`) — member-condition validators,
//!     delegated-puzzle-hash validators, and delegated-puzzle wrappers. The API accepts a
//!     `Vec<Restriction>`, but the canonical tests use only the empty set; a working spend
//!     needs validator puzzle/solution pairs that 0.30 does not demonstrate.
//!   * **Timelocks / recovery** (the memo-layer timelock + force-1-of-2 wrappers).
//!   * **Non-K1 members** — `BlsMember`, `R1Member`, `PasskeyMember`, `SingletonMember`,
//!     `FixedPuzzleMember` exist but have no canonical vault-spend test in 0.30.
//!   * **Nested m-of-n** (an m-of-n whose members are themselves m-of-n).

use crate::error::{ChainError, Result};
use crate::keys::IndexedKeys;
use chia::clvm_utils::TreeHash;
use chia::secp::{K1PublicKey, K1SecretKey, K1Signature};
use chia_protocol::{Bytes32, Coin, CoinSpend};
use chia_sha2::Sha256;
use chia_wallet_sdk::driver::{
    mips_puzzle_hash, InnerPuzzleSpend, Launcher, MipsSpend, MofN, SpendContext, StandardLayer,
    Vault,
};
use chia_wallet_sdk::types::puzzles::{K1Member, K1MemberSolution};
use chia_wallet_sdk::types::{Conditions, Mod};
use datalayer_driver::{sign_coin_spends, SecretKey, Signature};

/// A vault's custody configuration: `required` of the given K1 member public keys must
/// sign to spend. `required == keys.len()` is an n-of-n; `required == 1` is a 1-of-n;
/// a single key is the simple 1-of-1 case.
///
/// Restrictions, timelocks, recovery, and non-K1 members are intentionally NOT part of
/// this config — see the module-level gap notes; they are present in the 0.30 API but
/// unverified, so this crate does not construct spends that rely on them.
#[derive(Clone, Debug)]
pub struct VaultKeyConfig {
    /// The K1 member public keys that make up the custody set.
    pub members: Vec<K1PublicKey>,
    /// How many of `members` must sign to spend (1 ..= members.len()).
    pub required: usize,
}

impl VaultKeyConfig {
    /// A simple single-key vault (1-of-1).
    pub fn single(key: K1PublicKey) -> Self {
        Self {
            members: vec![key],
            required: 1,
        }
    }

    /// An m-of-n vault: `required` of `members` must sign.
    pub fn m_of_n(required: usize, members: Vec<K1PublicKey>) -> Result<Self> {
        if members.is_empty() {
            return Err(ChainError::Chain(
                "vault must have at least one member".into(),
            ));
        }
        if required == 0 || required > members.len() {
            return Err(ChainError::Chain(format!(
                "vault required ({required}) must be between 1 and the member count ({})",
                members.len()
            )));
        }
        Ok(Self { members, required })
    }

    /// The per-member MIPS leaf hashes (each a K1 member at nonce 0, no restrictions).
    /// `top_level` is false for leaves of an m-of-n; a single-member vault treats its one
    /// member as the top-level custody (see [`custody_hash`](Self::custody_hash)).
    fn member_hashes(&self, top_level: bool) -> Vec<TreeHash> {
        self.members
            .iter()
            .map(|pk| {
                mips_puzzle_hash(
                    0,
                    Vec::new(),
                    K1Member::new(*pk).curry_tree_hash(),
                    top_level,
                )
            })
            .collect()
    }

    /// The vault's top-level custody hash (the puzzle hash committed at mint), for either
    /// a single-member vault (the member is the top-level custody) or an m-of-n (the MofN
    /// node is the top-level custody, over per-member leaves).
    fn custody_hash(&self) -> TreeHash {
        if self.members.len() == 1 {
            mips_puzzle_hash(
                0,
                Vec::new(),
                K1Member::new(self.members[0]).curry_tree_hash(),
                true,
            )
        } else {
            let leaves = self.member_hashes(false);
            let m_of_n = MofN::new(self.required, leaves);
            mips_puzzle_hash(0, Vec::new(), m_of_n.inner_puzzle_hash(), true)
        }
    }
}

/// A created vault, returned by [`build_vault_create`] so the caller can later spend it
/// (which needs the vault coin + its key config, both of which only exist once the create
/// spend is confirmed).
#[derive(Clone, Debug)]
pub struct CreatedVault {
    /// The vault singleton.
    pub vault: Vault,
    /// The custody configuration the vault was created under (needed to spend it).
    pub config: VaultKeyConfig,
}

/// Build the (UNSIGNED) coin spends that CREATE a vault under `config`, funded by a
/// 1-mojo singleton launched from `funding_coin` (a wallet XCH coin at
/// `funder.owner_puzzle_hash`). Returns the spends and the resulting [`CreatedVault`].
///
/// The funding coin is spent through the standard layer to launch the vault singleton at
/// the config's top-level custody hash. `funding_coin` must hold at least 1 mojo (the
/// singleton); excess is left as an implicit fee.
///
/// **Pure: does NOT sign or broadcast.** `funder`'s BLS synthetic key authorizes the
/// funding-coin spend (sign with [`sign_vault_create_spends`]); the vault's own K1 keys
/// are not needed to CREATE it (only to spend it later).
pub fn build_vault_create(
    funder: &IndexedKeys,
    funding_coin: Coin,
    config: VaultKeyConfig,
) -> Result<(Vec<CoinSpend>, CreatedVault)> {
    if config.members.is_empty() {
        return Err(ChainError::Chain(
            "vault must have at least one member".into(),
        ));
    }
    if config.required == 0 || config.required > config.members.len() {
        return Err(ChainError::Chain(format!(
            "vault required ({}) must be between 1 and the member count ({})",
            config.required,
            config.members.len()
        )));
    }

    let mut ctx = SpendContext::new();
    let p2 = StandardLayer::new(funder.synthetic_pk);

    let custody_hash = config.custody_hash();
    let (mint_conditions, vault) = Launcher::new(funding_coin.coin_id(), 1)
        .mint_vault(&mut ctx, custody_hash, ())
        .map_err(|e| ChainError::Chain(format!("mint vault: {e}")))?;

    p2.spend(&mut ctx, funding_coin, mint_conditions)
        .map_err(|e| ChainError::Chain(format!("spend funding coin: {e}")))?;

    Ok((ctx.take(), CreatedVault { vault, config }))
}

/// The 32-byte message a K1 vault member signs to authorize a spend: the SHA-256 of the
/// delegated-puzzle tree hash followed by the vault coin id (the canonical MIPS K1
/// signing message). Exposed so a caller holding the K1 secret keys can sign without
/// re-deriving the hash.
pub fn vault_signing_message(delegated_puzzle_hash: TreeHash, vault_coin_id: Bytes32) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(<[u8; 32]>::from(delegated_puzzle_hash));
    hasher.update(vault_coin_id);
    hasher.finalize()
}

/// Build the (UNSIGNED) coin spends that SPEND `created` under its K1 custody, emitting
/// `conditions` as the vault's delegated output (e.g. re-create the vault, send a
/// payment). `signers` are the K1 SECRET keys that will satisfy the custody set — exactly
/// `config.required` of them for an m-of-n, or the single key for a 1-of-1; each MUST
/// correspond to one of `config.members`.
///
/// The vault's delegated spend commits `conditions`; each provided signer signs
/// [`vault_signing_message`] and its K1 member solution is inserted into the
/// [`MipsSpend`]. The resulting coin spend carries the K1 signatures inline, so it needs
/// NO aggregated BLS signature to validate (sign the bundle with an empty key set).
///
/// **Pure: does NOT broadcast.** Errors if too few signers are provided, or if a signer
/// is not a member of the vault's custody set.
pub fn build_vault_spend(
    created: &CreatedVault,
    signers: &[K1SecretKey],
    conditions: Conditions,
) -> Result<Vec<CoinSpend>> {
    let config = &created.config;
    if signers.len() < config.required {
        return Err(ChainError::Chain(format!(
            "vault spend needs at least {} signer(s), got {}",
            config.required,
            signers.len()
        )));
    }

    let mut ctx = SpendContext::new();
    let vault = &created.vault;

    // The delegated spend that the custody set authorizes (the vault's output conditions).
    let delegated = ctx
        .delegated_spend(conditions)
        .map_err(|e| ChainError::Chain(format!("vault delegated spend: {e}")))?;
    let mut spend = MipsSpend::new(delegated);

    // The signing message is over the delegated puzzle hash + the vault coin id.
    let delegated_ph = ctx.tree_hash(spend.delegated.puzzle);
    let vault_coin_id = vault.coin.coin_id();
    let message = vault_signing_message(delegated_ph, vault_coin_id);

    // Map each signer to its member public key and verify membership.
    let member_index = |sk: &K1SecretKey| -> Option<usize> {
        let pk = sk.public_key();
        config
            .members
            .iter()
            .position(|m| m.to_bytes() == pk.to_bytes())
    };

    if config.members.len() == 1 {
        // 1-of-1: the single member IS the top-level custody.
        let sk = signers
            .iter()
            .find(|sk| member_index(sk).is_some())
            .ok_or_else(|| ChainError::Chain("no provided signer is the vault member".into()))?;
        let signature = k1_sign(sk, &message)?;
        let member = K1Member::new(sk.public_key());
        insert_k1_member(&mut ctx, &mut spend, member, vault_coin_id, signature, true)?;
    } else {
        // m-of-n: insert the MofN node at the top level, then `required` member leaves.
        let leaves = config.member_hashes(false);
        let m_of_n = MofN::new(config.required, leaves);
        let custody_hash = mips_puzzle_hash(0, Vec::new(), m_of_n.inner_puzzle_hash(), true);
        spend.members.insert(
            custody_hash,
            InnerPuzzleSpend::m_of_n(0, Vec::new(), config.required, m_of_n.items.clone()),
        );

        let mut used: Vec<usize> = Vec::new();
        for sk in signers {
            if used.len() == config.required {
                break;
            }
            let Some(idx) = member_index(sk) else {
                continue;
            };
            if used.contains(&idx) {
                continue;
            }
            let signature = k1_sign(sk, &message)?;
            let member = K1Member::new(config.members[idx]);
            insert_k1_member(
                &mut ctx,
                &mut spend,
                member,
                vault_coin_id,
                signature,
                false,
            )?;
            used.push(idx);
        }
        if used.len() < config.required {
            return Err(ChainError::Chain(format!(
                "vault spend got {} valid member signature(s), needs {}",
                used.len(),
                config.required
            )));
        }
    }

    vault
        .spend(&mut ctx, &spend)
        .map_err(|e| ChainError::Chain(format!("spend vault: {e}")))?;
    Ok(ctx.take())
}

/// Insert one K1 member's puzzle+solution into the `MipsSpend` at its leaf hash.
/// `top_level` is true only for a 1-of-1 vault (its member is the top-level custody).
fn insert_k1_member(
    ctx: &mut SpendContext,
    spend: &mut MipsSpend,
    member: K1Member,
    vault_coin_id: Bytes32,
    signature: K1Signature,
    top_level: bool,
) -> Result<()> {
    let leaf = mips_puzzle_hash(0, Vec::new(), member.curry_tree_hash(), top_level);
    let puzzle = ctx
        .curry(member)
        .map_err(|e| ChainError::Chain(format!("curry k1 member: {e}")))?;
    let solution = ctx
        .alloc(&K1MemberSolution::new(vault_coin_id, signature))
        .map_err(|e| ChainError::Chain(format!("alloc k1 member solution: {e}")))?;
    spend.members.insert(
        leaf,
        InnerPuzzleSpend::new(
            0,
            Vec::new(),
            chia_wallet_sdk::driver::Spend::new(puzzle, solution),
        ),
    );
    Ok(())
}

/// Sign the K1 `message` with `sk`, returning the recoverable K1 signature.
fn k1_sign(sk: &K1SecretKey, message: &[u8; 32]) -> Result<K1Signature> {
    sk.sign_prehashed(message)
        .map_err(|e| ChainError::Chain(format!("k1 sign: {e}")))
}

/// Sign the vault-CREATE `coin_spends` with `keys` (the funder's BLS synthetic secret
/// keys), returning the aggregated signature. The CREATE is a standard-layer spend of
/// the funding coin, signed exactly like [`crate::nft::sign_nft_spends`]. (The vault
/// SPEND itself carries K1 signatures inline and needs no BLS aggregate.)
pub fn sign_vault_create_spends(
    coin_spends: &[CoinSpend],
    keys: &[SecretKey],
    for_testnet: bool,
) -> Result<Signature> {
    sign_coin_spends(coin_spends, keys, for_testnet)
        .map_err(|e| ChainError::Chain(format!("sign vault create spends: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::derive_indexed_keys;
    use chia::puzzles::Memos;
    use chia_protocol::SpendBundle;
    use chia_sdk_test::{K1Pair, Simulator};

    // Public BIP-39 test vector (NOT a real wallet). Matches the rest of the crate.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    // ----- offline: config validation -----

    #[test]
    fn m_of_n_rejects_bad_threshold() {
        let keys = K1Pair::range_vec(3);
        let pks: Vec<K1PublicKey> = keys.iter().map(|k| k.pk).collect();
        assert!(VaultKeyConfig::m_of_n(0, pks.clone()).is_err());
        assert!(VaultKeyConfig::m_of_n(4, pks.clone()).is_err());
        assert!(VaultKeyConfig::m_of_n(2, pks).is_ok());
        assert!(VaultKeyConfig::m_of_n(1, vec![]).is_err());
    }

    #[test]
    fn spend_rejects_too_few_signers() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let funder = derive_indexed_keys(ABANDON, 0..1)?[0].clone();
        let keys = K1Pair::range_vec(3);
        let pks: Vec<K1PublicKey> = keys.iter().map(|k| k.pk).collect();
        let config = VaultKeyConfig::m_of_n(2, pks)?;
        let funding = sim.new_coin(funder.owner_puzzle_hash, 1);
        let (_spends, created) = build_vault_create(&funder, funding, config)?;
        // Provide only one signer for a 2-of-3 vault.
        let err = build_vault_spend(
            &created,
            std::slice::from_ref(&keys[0].sk),
            Conditions::new(),
        )
        .unwrap_err();
        assert!(
            matches!(&err, ChainError::Chain(m) if m.contains("at least 2 signer")),
            "got: {err}"
        );
        Ok(())
    }

    // ----- Simulator: create -> spend a 1-of-1 vault -----

    /// Create a single-K1-key vault funded by the wallet's BLS coin, confirm it, then
    /// spend it under that key (re-creating the vault). Drives `build_vault_create` +
    /// `build_vault_spend` end-to-end on the in-process Chia simulator (the vault.rs
    /// `test_simple_vault` shape).
    #[test]
    fn create_then_spend_single_key_vault() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let funder = derive_indexed_keys(ABANDON, 0..1)?[0].clone();
        let k1 = K1Pair::default();
        let config = VaultKeyConfig::single(k1.pk);

        // Create the vault.
        let funding = sim.new_coin(funder.owner_puzzle_hash, 1);
        let (create_spends, created) = build_vault_create(&funder, funding, config)?;
        let sig = sign_vault_create_spends(
            &create_spends,
            std::slice::from_ref(&funder.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(create_spends, sig))?;
        assert!(
            sim.coin_state(created.vault.coin.coin_id()).is_some(),
            "the vault singleton should exist after create"
        );

        // Spend the vault under its single K1 key: re-create the vault at the same
        // custody (the canonical "move under custody" output). The K1 signature is inline
        // so no BLS aggregate is needed.
        let conditions =
            Conditions::new().create_coin(created.vault.info.custody_hash.into(), 1, Memos::None);
        let spend_spends = build_vault_spend(&created, std::slice::from_ref(&k1.sk), conditions)?;
        sim.new_transaction(SpendBundle::new(spend_spends, Signature::default()))?;

        // The vault re-created a child at the same custody hash; the original coin is spent.
        assert!(
            sim.coin_state(created.vault.coin.coin_id())
                .and_then(|cs| cs.spent_height)
                .is_some(),
            "the vault coin should be spent after the member spend"
        );
        Ok(())
    }

    // ----- Simulator: create -> spend an m-of-n vault -----

    /// Create a 2-of-3 K1 vault, confirm it, then spend it with two of the three keys.
    /// Drives the m-of-n path of `build_vault_create` + `build_vault_spend` on the
    /// simulator (the vault.rs `test_m_of_n_vault` shape).
    #[test]
    fn create_then_spend_2_of_3_vault() -> anyhow::Result<()> {
        let mut sim = Simulator::new();

        let funder = derive_indexed_keys(ABANDON, 0..1)?[0].clone();
        let keys = K1Pair::range_vec(3);
        let pks: Vec<K1PublicKey> = keys.iter().map(|k| k.pk).collect();
        let config = VaultKeyConfig::m_of_n(2, pks)?;

        let funding = sim.new_coin(funder.owner_puzzle_hash, 1);
        let (create_spends, created) = build_vault_create(&funder, funding, config)?;
        let sig = sign_vault_create_spends(
            &create_spends,
            std::slice::from_ref(&funder.synthetic_sk),
            true,
        )?;
        sim.new_transaction(SpendBundle::new(create_spends, sig))?;

        // Spend with members 0 and 2 (two of three) — re-create the vault.
        let conditions =
            Conditions::new().create_coin(created.vault.info.custody_hash.into(), 1, Memos::None);
        let signers = vec![keys[0].sk.clone(), keys[2].sk.clone()];
        let spend_spends = build_vault_spend(&created, &signers, conditions)?;
        sim.new_transaction(SpendBundle::new(spend_spends, Signature::default()))?;

        assert!(
            sim.coin_state(created.vault.coin.coin_id())
                .and_then(|cs| cs.spent_height)
                .is_some(),
            "the 2-of-3 vault coin should be spent after the member spend"
        );
        Ok(())
    }
}
