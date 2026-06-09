use digstore_core::merkle::MerkleTree;
use digstore_core::sha256;
use digstore_core::Bytes32;

fn chunks(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| vec![i as u8; 8]).collect()
}

#[test]
fn single_leaf_root_is_leaf_hash() {
    let data = vec![vec![1u8, 2, 3]];
    let tree = MerkleTree::build(&data);
    assert_eq!(tree.root(), sha256(&[1u8, 2, 3]));
}

#[test]
fn two_leaves_root_is_parent_hash() {
    let a = vec![0xAAu8];
    let b = vec![0xBBu8];
    let tree = MerkleTree::build(&[a.clone(), b.clone()]);
    let la = sha256(&a);
    let lb = sha256(&b);
    let mut cat = Vec::new();
    cat.extend_from_slice(&la.0);
    cat.extend_from_slice(&lb.0);
    assert_eq!(tree.root(), sha256(&cat));
}

#[test]
fn odd_leaf_is_carried_up() {
    // 3 leaves: level0 = [l0,l1,l2]; level1 = [h(l0||l1), l2]; root = h(level1_0 || l2).
    let data = chunks(3);
    let tree = MerkleTree::build(&data);
    let l: Vec<Bytes32> = data.iter().map(|c| sha256(c)).collect();
    let mut p01 = Vec::new();
    p01.extend_from_slice(&l[0].0);
    p01.extend_from_slice(&l[1].0);
    let n01 = sha256(&p01);
    let mut top = Vec::new();
    top.extend_from_slice(&n01.0);
    top.extend_from_slice(&l[2].0); // odd carried up unchanged
    assert_eq!(tree.root(), sha256(&top));
}

#[test]
fn inclusion_proof_accepts_each_leaf() {
    let data = chunks(8);
    let tree = MerkleTree::build(&data);
    for (i, c) in data.iter().enumerate() {
        let proof = tree.prove(i).unwrap();
        assert_eq!(proof.leaf, sha256(c));
        assert_eq!(proof.root, tree.root());
        assert!(proof.verify());
    }
}

#[test]
fn inclusion_proof_rejects_tampered_leaf() {
    let data = chunks(8);
    let tree = MerkleTree::build(&data);
    let mut proof = tree.prove(3).unwrap();
    proof.leaf = Bytes32([0xFF; 32]);
    assert!(!proof.verify());
}

#[test]
fn inclusion_proof_rejects_tampered_path() {
    let data = chunks(8);
    let tree = MerkleTree::build(&data);
    let mut proof = tree.prove(3).unwrap();
    proof.path[0].hash = Bytes32([0x00; 32]);
    assert!(!proof.verify());
}

#[test]
fn full_spine_proof_size_is_exactly_ceil_log2_n() {
    // Leaf index 0 is never the carried-up node at any level, so it traverses
    // the full left spine: its path length is the worst case and equals
    // ceil(log2 n) for every n. This is §9.5's stated sibling count, realized
    // by the leaf that attains the bound.
    for n in [1usize, 2, 3, 4, 5, 8, 16, 17, 1000] {
        let data = chunks(n);
        let tree = MerkleTree::build(&data);
        let proof = tree.prove(0).unwrap();
        assert_eq!(proof.path.len(), ceil_log2(n), "n={n}");
    }
}

#[test]
fn every_proof_path_is_at_most_ceil_log2_n_and_verifies() {
    // §9.1 carries an odd node up unchanged, so a leaf that is carried up skips
    // a level and its path is SHORTER than the full-spine worst case. §9.5's
    // ceil(log2 n) is therefore the upper bound on sibling count; every leaf's
    // path is <= ceil(log2 n) and still verifies against the root. (Approved
    // deviation: carry-up paths, documented in 00-DATASECTION-CONTRACT.md D8.)
    for n in [1usize, 2, 3, 4, 5, 6, 7, 8, 9, 16, 17, 31, 1000] {
        let data = chunks(n);
        let tree = MerkleTree::build(&data);
        let bound = ceil_log2(n);
        for i in 0..n {
            let proof = tree.prove(i).unwrap();
            assert!(
                proof.path.len() <= bound,
                "n={n} i={i}: path.len()={} exceeds ceil(log2 n)={bound}",
                proof.path.len()
            );
            assert!(proof.verify(), "n={n} i={i}: proof must verify");
        }
    }
}

#[test]
fn carried_up_leaf_has_shorter_path_than_full_spine() {
    // Documents the carry-up deviation concretely: for n=3, the carried-up leaf
    // (index 2) ascends in 1 step while the full spine (index 0) takes 2.
    let tree = MerkleTree::build(&chunks(3));
    assert_eq!(tree.prove(0).unwrap().path.len(), 2);
    assert_eq!(tree.prove(2).unwrap().path.len(), 1);
    assert!(tree.prove(2).unwrap().verify());
}

#[test]
fn thousand_leaf_all_proofs_verify() {
    let data = chunks(1000);
    let tree = MerkleTree::build(&data);
    for i in (0..1000).step_by(37) {
        assert!(tree.prove(i).unwrap().verify());
    }
}

#[test]
fn prove_out_of_range_is_none() {
    let tree = MerkleTree::build(&chunks(4));
    assert!(tree.prove(4).is_none());
}

fn ceil_log2(n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let mut levels = 0;
    let mut count = n;
    while count > 1 {
        count = count.div_ceil(2);
        levels += 1;
    }
    levels
}
