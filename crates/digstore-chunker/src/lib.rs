//! Deterministic gear-based content-defined chunking (FastCDC line) for Digstore.
//!
//! # Design notes (documented deviations & decisions)
//! - The gear hash is HAND-ROLLED (not delegated to the `fastcdc` crate) so we
//!   retain byte-exact control over the gear table and boundary algorithm. This
//!   guarantees identical chunk boundaries across platforms and crate versions,
//!   which is required for content-addressed dedup (design §4.2; paper §8.1, §3
//!   CDC heritage). We borrow only the *approach* (a 256-entry gear table + the
//!   `(hash & mask) == 0` cut rule) from FastCDC, not its code.
//! - The gear table is GENERATED AT COMPILE TIME by a `const fn` SplitMix64
//!   stream and is therefore exactly 256 entries by construction — no
//!   hand-authored literals that could miscount or contain malformed hex.
//! - Boundaries use the FastCDC rule `(hash & mask) == 0`, bounded by
//!   `ChunkerConfig::min_size` (no cut below it) and `ChunkerConfig::max_size`
//!   (forced cut at it).

mod boundary;
mod chunk;
mod chunker;
mod config;
mod gear;

pub use chunk::{hash_data, Chunk};
pub use chunker::{chunk_slice, chunk_stream, Chunker};
pub use config::{default_config, mask_for_target};
pub use gear::GEAR_TABLE;
