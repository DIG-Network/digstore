//! digstore-prover — execution proofs (§13) and Chia chain anchoring.
//!
//! # Documented deviation #3 (paper §13)
//! risc0 proves a faithful re-execution of the *deterministic serving
//! computation* (resolve retrieval key -> key-table lookup -> gather +
//! concatenate chunk ciphertext -> commit output), NOT wasmtime opcodes.
//! `program_hash = SHA-256(module_bytes)`. The [`mock::MockProver`] is the
//! default backend so the rest of the system is fully functional while the
//! real risc0 circuit matures.

pub mod chain;
pub mod coinset;
pub mod commitment;
pub mod error;
pub mod mock;
pub mod mock_chain;
pub mod prover;
pub mod serving_inputs;

#[cfg(feature = "risc0")]
pub mod risc0_backend;

#[cfg(feature = "hardware-attest")]
pub mod hardware;

pub use chain::ChainSource;
pub use coinset::CoinsetChainSource;
pub use commitment::{build_public_input, parse_public_input, signing_message, NONCE_LEN};
pub use error::{ProverError, Result};
pub use mock::{MockProver, MockVerifier, DEFAULT_FRESHNESS_WINDOW_SECS};
pub use mock_chain::MockChainSource;
pub use prover::{bound_public_output, Prover, Verifier};
pub use serving_inputs::ServingInputs;
