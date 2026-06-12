//! DIG token (CAT) constants + amount helpers (see the DIG-CAT-payment design).
use chia_protocol::Bytes32;

/// DIG CAT asset id (mainnet). Matches DataLayer-Driver `DIG_ASSET_ID`.
pub const DIG_ASSET_ID: Bytes32 = Bytes32::new(hex_literal::hex!(
    "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81"
));

/// DIG treasury recipient (bech32 `xch1…`); DIG is sent to this address's CAT ph.
pub const TREASURY_ADDRESS: &str =
    "xch1a37rq3cgcl2ecpudttsf35x75qzdan68lgw2l6ajvmqs44jxdn5qv6pk3y";

/// DIG has 3 decimals: 1 DIG = 1000 base units.
pub const DIG_DECIMALS: u32 = 3;
/// Base units to mint a store (`init`): 100 DIG.
pub const INIT_DIG: u64 = 100_000;
/// Base units per root update (`commit`): 10 DIG.
pub const COMMIT_DIG: u64 = 10_000;

/// The treasury's inner (standard) puzzle hash, decoded from `TREASURY_ADDRESS`.
pub fn treasury_inner_puzzle_hash() -> Bytes32 {
    datalayer_driver::address_to_puzzle_hash(TREASURY_ADDRESS)
        .expect("TREASURY_ADDRESS is a valid xch address")
}

/// Format base units as a human DIG string (÷1000, 3 dp).
pub fn format_dig(base_units: u64) -> String {
    format!("{}.{:03}", base_units / 1000, base_units % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn treasury_ph_decodes_and_is_32_bytes() {
        let ph = treasury_inner_puzzle_hash();
        assert_eq!(ph.to_bytes().len(), 32);
    }
    #[test]
    fn format_dig_renders_three_decimals() {
        assert_eq!(format_dig(100_000), "100.000");
        assert_eq!(format_dig(10_500), "10.500");
        assert_eq!(format_dig(1), "0.001");
    }
}
