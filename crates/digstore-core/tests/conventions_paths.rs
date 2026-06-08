//! Guards CONVENTIONS C2/C3/C9 module paths that downstream crates depend on.

// C2: types alias module.
use digstore_core::types::{Bytes32, Bytes48, Bytes96};
// C2: abi::ErrorCode must resolve (re-exported inside abi.rs).
use digstore_core::abi::ErrorCode;
// C2: submodule paths are public.
use digstore_core::config::HostImportsConfig;
use digstore_core::merkle::MerkleTree;
// C3: ProofPrelude in wire.
use digstore_core::wire::ProofPrelude;
// C9: serving::concat_output.
use digstore_core::serving::concat_output;

#[test]
fn convention_paths_resolve() {
    let _b32 = Bytes32([0; 32]);
    let _b48 = Bytes48([0; 48]);
    let _b96 = Bytes96([0; 96]);
    assert_eq!(ErrorCode::GeneralError as i32, -1);
    let _h = HostImportsConfig::default();
    let _t = MerkleTree::build(&[vec![1u8]]);
    let _p = ProofPrelude {
        roothash: Bytes32([1; 32]),
        output_commitment: Bytes32([2; 32]),
        serving_digest: Bytes32([3; 32]),
    };
    assert_eq!(concat_output(&[&[1u8, 2][..], &[3u8][..]]), vec![1, 2, 3]);
}
