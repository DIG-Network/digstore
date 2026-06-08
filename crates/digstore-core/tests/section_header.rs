use digstore_core::codec::section::{SectionEntry, SectionHeader, DIGS_MAGIC, FORMAT_VERSION};
use digstore_core::codec::{Decode, Encode};

#[test]
fn header_starts_with_magic_and_version() {
    let header = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![
            SectionEntry { id: 1, offset: 100, length: 50 },
            SectionEntry { id: 2, offset: 150, length: 25 },
        ],
    };
    let bytes = header.to_bytes();
    assert_eq!(&bytes[0..4], DIGS_MAGIC);
    assert_eq!(bytes[4], FORMAT_VERSION);
}

#[test]
fn header_offset_table_roundtrip() {
    let header = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![
            SectionEntry { id: 7, offset: 0, length: 4096 },
            SectionEntry { id: 9, offset: 4096, length: 1024 },
        ],
    };
    let bytes = header.to_bytes();
    let decoded = SectionHeader::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, header);
}

#[test]
fn header_rejects_bad_magic() {
    let mut bytes = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![],
    }
    .to_bytes();
    bytes[0] = b'X';
    assert!(SectionHeader::from_bytes(&bytes).is_err());
}

#[test]
fn header_rejects_unknown_version() {
    let mut bytes = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![],
    }
    .to_bytes();
    bytes[4] = 99;
    assert!(SectionHeader::from_bytes(&bytes).is_err());
}

#[test]
fn lookup_finds_section_by_id() {
    let header = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![
            SectionEntry { id: 3, offset: 10, length: 20 },
            SectionEntry { id: 5, offset: 30, length: 40 },
        ],
    };
    assert_eq!(header.find(5), Some((30, 40)));
    assert_eq!(header.find(99), None);
}
