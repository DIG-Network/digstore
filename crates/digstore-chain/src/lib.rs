//! Seed management and (later) Chia anchoring for digstore.

pub mod anchor;
pub mod cat;
pub mod chip0002;
pub mod clawback;
pub mod coinset;
pub mod config;
pub mod did;
pub mod dig;
pub mod error;
mod fs_util;
pub mod keys;
pub mod nft;
pub mod offer;
pub mod option;
pub mod seed;
pub mod send;
pub mod singleton;
pub mod unlock;
pub mod wallet;

pub use error::{ChainError, Result};
