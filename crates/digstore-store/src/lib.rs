//! digstore-store: the host-side Store entity, on-disk layout, staging, and generations.
//!
//! Implements paper sections 4.1–4.4 (store structure), 8.2 (generations), and
//! 20.1–20.3 store mechanics (init / add / commit). Generation directories
//! produced here are consumed by `digstore-compiler` (which owns §8.3 pool
//! ordering and §19.3 byte-identical compilation) and `digstore-guest`.

mod clock;
mod error;
mod paths;

pub use clock::{Clock, FixedClock, SystemClock};
pub use error::{Result, StoreError};
pub use paths::StorePaths;
