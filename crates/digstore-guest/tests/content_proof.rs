use digstore_core::merkle::MerkleTree;
use digstore_core::Bytes32;
use digstore_guest::content::emit_merkle_proof;

#[test]
fn emitted_proof_verifies_against_core() {
    // Four chunks -> leaves = SHA-256(chunk). Build the core tree, then emit a
    // proof for chunk index 2 inside the guest and verify it with core rules.
    let chunks: Vec<Vec<u8>> = vec![
        b"alpha".to_vec(),
        b"beta".to_vec(),
        b"gamma".to_vec(),
        b"delta".to_vec(),
    ];
    let tree = MerkleTree::build(&chunks);
    let root: Bytes32 = tree.root();

    let proof = emit_merkle_proof(&tree, 2);
    assert_eq!(proof.root, root);
    assert!(proof.verify(), "guest-emitted proof must verify under core rules");
}
