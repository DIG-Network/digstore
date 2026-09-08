//! Seed management and (later) Chia anchoring for digstore.

mod continuation_guard;

pub mod anchor;
pub mod cat;
pub mod chip0002;
pub mod clawback;
pub mod coinset;
pub mod collection;
pub mod collection_index;
pub mod config;
pub mod did;
pub mod dig;
pub mod error;
mod fs_util;
pub mod keys;
pub mod metadata;
pub mod nft;
pub mod offer;
pub mod option;
pub mod pricing;
pub mod seed;
pub mod selection;
pub mod send;
pub mod singleton;
pub mod streaming;
pub mod unlock;
pub mod vault;
// NOTE: there is deliberately no `vc` module. It wrapped the chia-wallet-sdk verification
// layer (`Verification` / `VerifiedData` / `VerificationAsserter`), which upstream REMOVED
// in 0.36 with no replacement — `chia-sdk-driver`'s `primitives/action_layer/verification*`
// files are gone while the rest of the action layer survives, so the removal is deliberate
// and the on-chain primitive the module attested against no longer ships. Re-deriving those
// puzzles here would be a rival implementation of removed upstream CLVM (Appendix B), so the
// surface is withdrawn rather than faked. It had no callers anywhere in the ecosystem.
pub mod wallet;

pub use error::{ChainError, Result};
