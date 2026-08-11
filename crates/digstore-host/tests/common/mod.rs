//! Shared test helpers for digstore-host integration tests.

use digstore_core::types::Bytes32;
use digstore_core::ChiaBlockRef;
use digstore_crypto::bls::BlsSecretKey;
use digstore_host::{FixedClock, HostDeps, HostIdentity};
use digstore_prover::{MockChainSource, MockProver};
use std::sync::Arc;

/// Build HostDeps with a deterministic BLS key, mock chain, and mock prover.
/// `clock` is shared (FixedClock clones share their counter) so tests can advance it.
pub fn test_deps(clock: FixedClock) -> HostDeps {
    anonymous_test_deps(clock).with_identity(HostIdentity::from_seed(&[42u8; 32]))
}

/// The same deps with NO host identity: the shape the CLI's read path builds.
///
/// Everything except the identity is identical to [`test_deps`], so a test that
/// swaps one for the other varies exactly one thing.
pub fn anonymous_test_deps(clock: FixedClock) -> HostDeps {
    // A separate (deterministic) key + a known chain block back the mock prover.
    let prover_sk = BlsSecretKey::from_seed(&[7u8; 32]);
    let prover_pk = prover_sk.public_key();
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 100,
        timestamp: 1_700_000_000,
    };
    let chain = MockChainSource::new(vec![block.clone()], 1_700_000_000);
    let prover = MockProver::new(prover_sk, prover_pk, block);

    HostDeps::new(
        Bytes32([0u8; 32]),
        Arc::new(clock),
        Arc::new(chain),
        Arc::new(prover),
        Bytes32([1u8; 32]),
    )
    .with_rng_seed([99u8; 32])
}
