//! digstore-store: the host-side Store entity, on-disk layout, staging, and generations.
//!
//! Implements paper sections 4.1–4.4 (store structure), 8.2 (generations), and
//! 20.1–20.3 store mechanics (init / add / commit). Generation directories
//! produced here are consumed by `digstore-compiler` (which owns §8.3 pool
//! ordering and §19.3 byte-identical compilation) and `digstore-guest`.

mod chunkstore;
mod clock;
mod config;
mod diff;
mod error;
mod generation;
mod history;
mod paths;
mod staging;

pub use chunkstore::ChunkStore;
pub use diff::GenerationDiff;
pub use clock::{Clock, FixedClock, SystemClock};
pub use config::{load_config, save_config};
pub use error::{Result, StoreError};
pub use generation::{ChunkRef, GenerationManifest, KeyTableRecord};
pub use history::RootHistory;
pub use paths::StorePaths;
pub use staging::{StagedRecord, StagingArea};
