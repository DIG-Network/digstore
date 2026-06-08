//! `dig-host`: wasmtime runtime for serving compiled Digstore WASM modules.
//!
//! Implements the host side of the `dig_host` import module (paper §6.3, §12),
//! the shared return buffer (§6.4), execution bounds (§18.2), the import
//! dispatch / state threading (§18.1, §18.3), the serve flow (§18.4), and the
//! swappable TEE-alternative attestation hook (§13.6). The host NEVER decrypts
//! or inspects served payloads.

mod clock;
mod config;
mod error;
mod imports;
mod memory;
mod random;
mod runtime;
mod session;
mod state;
mod teehook;

pub use clock::{Clock, FixedClock, SystemClock};
pub use config::{ExecutionLimits, MAX_MEMORY_BYTES, WASM_PAGE_SIZE};
pub use error::HostError;
pub use random::HostRng;
pub use session::{Session, SessionTable};
pub use teehook::{AttestationBackend, BlsAttestationBackend, SharedBackend};
