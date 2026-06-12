mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

/// `digstore balance` against the seeded mock: exits 0 and the human output
/// shows the address line plus XCH and DIG balances. No store/init needed —
/// balance is wallet-only.
#[test]
fn balance_human_shows_xch_dig_and_address() {
    let dir = tmp_dig();
    dig(&dir).arg("balance").assert().success().stdout(
        predicate::str::contains("address")
            .and(predicate::str::contains("XCH"))
            .and(predicate::str::contains("DIG")),
    );
}

/// `digstore balance --json` carries xch_mojos, dig_base_units, address, and the
/// mocked flag (true under the in-memory mock).
#[test]
fn balance_json_has_fields_and_mocked_flag() {
    let dir = tmp_dig();
    let out = dig(&dir).args(["--json", "balance"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["xch_mojos"].is_u64());
    assert!(v["dig_base_units"].is_u64());
    assert!(v["address"].as_str().is_some());
    assert_eq!(v["mocked"], true);

    // The `dig` field is the human-formatted DIG string (÷1000, 3 dp) and must
    // equal format_dig of the raw base units the JSON also carries.
    let dig_units = v["dig_base_units"].as_u64().unwrap();
    let dig_str = v["dig"].as_str().expect("dig is a formatted string");
    assert_eq!(dig_str, digstore_chain::dig::format_dig(dig_units));
    // N.NNN shape: one dot, exactly 3 fractional digits, all-digit parts.
    let (whole, frac) = dig_str.split_once('.').expect("dig has a decimal point");
    assert_eq!(frac.len(), 3, "dig `{dig_str}` must have 3 decimals");
    assert!(
        whole.chars().all(|c| c.is_ascii_digit()) && frac.chars().all(|c| c.is_ascii_digit()),
        "dig `{dig_str}` must be all digits"
    );
}

/// With `DIGSTORE_ANCHOR_MOCK_DIG=0` the reported DIG balance is zero.
#[test]
fn balance_json_dig_zero_when_env_zero() {
    let dir = tmp_dig();
    let out = dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_DIG", "0")
        .args(["--json", "balance"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["dig_base_units"], 0);
}
