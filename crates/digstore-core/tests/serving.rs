use digstore_core::serving::concat_output;

#[test]
fn concat_output_preserves_order() {
    let a = vec![1u8, 2, 3];
    let b = vec![4u8, 5];
    let c = vec![6u8];
    let out = concat_output(&[&a, &b, &c]);
    assert_eq!(out, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn concat_output_empty_is_empty() {
    let out = concat_output(&[]);
    assert!(out.is_empty());
}

#[test]
fn concat_output_handles_empty_chunks() {
    let a: Vec<u8> = vec![];
    let b = vec![9u8, 9];
    let out = concat_output(&[&a, &b, &a]);
    assert_eq!(out, vec![9, 9]);
}
