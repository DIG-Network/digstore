use digstore_core::Bytes32;

/// The module's ETag is its generation root (§21.7), rendered as a strong,
/// quoted hex string: `"<64-hex>"`.
pub fn etag_for_root(root: &Bytes32) -> String {
    format!("\"{}\"", root.to_hex())
}

/// Parse a single `If-None-Match` header value into a root, if it is a
/// well-formed quoted 64-hex strong tag. Returns None for `*`, weak tags,
/// or malformed values.
pub fn parse_if_none_match(header: &str) -> Option<Bytes32> {
    let trimmed = header.trim();
    let inner = trimmed.strip_prefix('"')?.strip_suffix('"')?;
    Bytes32::from_hex(inner).ok()
}

/// Does the client's `If-None-Match` value match the current root? (=> 304)
pub fn matches_current(header: &str, current_root: &Bytes32) -> bool {
    parse_if_none_match(header).as_ref() == Some(current_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;

    fn root(b: u8) -> Bytes32 {
        Bytes32([b; 32])
    }

    #[test]
    fn etag_is_quoted_hex_of_root() {
        let e = etag_for_root(&root(0xAB));
        assert_eq!(e, format!("\"{}\"", "ab".repeat(32)));
    }

    #[test]
    fn parse_round_trips_etag() {
        let r = root(0x07);
        let e = etag_for_root(&r);
        assert_eq!(parse_if_none_match(&e), Some(r));
    }

    #[test]
    fn matches_current_true_when_equal_false_when_not() {
        let r = root(0x10);
        assert!(matches_current(&etag_for_root(&r), &r));
        assert!(!matches_current(&etag_for_root(&root(0x11)), &r));
    }

    #[test]
    fn star_and_garbage_do_not_match() {
        let r = root(0x20);
        assert!(!matches_current("*", &r));
        assert!(!matches_current("\"nothex\"", &r));
        assert!(!matches_current("W/\"weak\"", &r));
    }
}
