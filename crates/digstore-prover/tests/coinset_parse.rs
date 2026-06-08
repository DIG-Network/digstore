use digstore_prover::coinset::{
    parse_block_record_resolved, parse_blockchain_state, BlockRecord, BlockRecordResponse,
    BlockchainStateResponse,
};

const STATE_JSON: &str = include_str!("fixtures/get_blockchain_state.json");
const NOTX_JSON: &str = include_str!("fixtures/get_block_record_by_height_notx.json");
const PREVTX_JSON: &str = include_str!("fixtures/get_block_record_prev_tx.json");

#[test]
fn parses_blockchain_state_peak_into_chia_block_ref() {
    let resp: BlockchainStateResponse = serde_json::from_str(STATE_JSON).unwrap();
    let block = parse_blockchain_state(resp).unwrap();
    assert_eq!(block.height, 5_421_337);
    assert_eq!(block.timestamp, 1_717_804_800);
    assert_eq!(block.header_hash.0[0], 0xB5);
    assert_eq!(block.header_hash.0[31], 0xEE);
}

#[test]
fn non_transaction_block_walks_down_to_prev_tx_for_timestamp() {
    // The fetched record (height 5421335) has timestamp == null; the resolver
    // returns the prev transaction block (5421330) which carries a timestamp.
    let notx: BlockRecordResponse = serde_json::from_str(NOTX_JSON).unwrap();
    let prevtx: BlockRecordResponse = serde_json::from_str(PREVTX_JSON).unwrap();
    let prev_record: BlockRecord = prevtx.block_record.clone().unwrap();

    let block = parse_block_record_resolved(notx, &mut |height| {
        assert_eq!(height, 5_421_330); // walk-down follows prev_transaction_block_height
        Ok(prev_record.clone())
    })
    .unwrap();

    // Header hash/height stay the originally-requested record's; timestamp is
    // inherited from the nearest previous transaction block.
    assert_eq!(block.height, 5_421_335);
    assert_eq!(block.header_hash.0[0], 0xC6);
    assert_eq!(block.timestamp, 1_717_804_620);
}

#[test]
fn transaction_block_uses_its_own_timestamp_without_walking() {
    let prevtx: BlockRecordResponse = serde_json::from_str(PREVTX_JSON).unwrap();
    // Resolver must NOT be called for a block that already has a timestamp.
    let block = parse_block_record_resolved(prevtx, &mut |_h| {
        panic!("resolver must not be called for a transaction block");
    })
    .unwrap();
    assert_eq!(block.height, 5_421_330);
    assert_eq!(block.timestamp, 1_717_804_620);
}

#[test]
fn rejects_unsuccessful_response() {
    let bad = r#"{"success": false, "error": "boom"}"#;
    let resp: BlockchainStateResponse = serde_json::from_str(bad).unwrap();
    assert!(parse_blockchain_state(resp).is_err());
}
