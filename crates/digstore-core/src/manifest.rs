//! Metadata manifest + author (paper 5.2 structs).
//!
//! `BTreeMap` is used so iteration/encode order is deterministic. `custom`
//! holds `serde_json::Value`, matching the canonical catalog exactly; the
//! `serde_json` dependency is non-optional (`alloc` feature) so `Value` is
//! available in both the host (`std`) and guest (`no_std + alloc`) builds.

use crate::codec::{utf8_from, Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::collections::BTreeMap;
#[cfg(not(feature = "std"))]
use alloc::collections::BTreeMap;

/// One author of a store's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    pub handle: Option<String>,
    pub contact: Option<String>,
}

impl Encode for Author {
    fn encode(&self, enc: &mut Encoder) {
        self.name.encode(enc);
        self.handle.encode(enc);
        self.contact.encode(enc);
    }
}

impl Decode for Author {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Author {
            name: String::decode(dec)?,
            handle: Option::<String>::decode(dec)?,
            contact: Option::<String>::decode(dec)?,
        })
    }
}

/// Plaintext metadata manifest (NOT gated by session; served via get_metadata).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<Author>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub keywords: Vec<String>,
    pub categories: Vec<String>,
    pub icon: Option<String>,
    pub content_type: Option<String>,
    pub links: BTreeMap<String, String>,
    pub custom: BTreeMap<String, serde_json::Value>,
}

/// Encode a `BTreeMap<String, String>` as 4-byte BE count then key/value strings.
fn encode_str_map(map: &BTreeMap<String, String>, enc: &mut Encoder) {
    (map.len() as u32).encode(enc);
    for (k, v) in map {
        k.encode(enc);
        v.encode(enc);
    }
}

fn decode_str_map(dec: &mut Decoder<'_>) -> Result<BTreeMap<String, String>, DecodeError> {
    let count = u32::decode(dec)? as usize;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let k = String::decode(dec)?;
        let v = String::decode(dec)?;
        map.insert(k, v);
    }
    Ok(map)
}

impl Encode for MetadataManifest {
    fn encode(&self, enc: &mut Encoder) {
        self.schema_version.encode(enc);
        self.name.encode(enc);
        self.version.encode(enc);
        self.description.encode(enc);
        self.authors.encode(enc);
        self.license.encode(enc);
        self.homepage.encode(enc);
        self.repository.encode(enc);
        self.keywords.encode(enc);
        self.categories.encode(enc);
        self.icon.encode(enc);
        self.content_type.encode(enc);
        encode_str_map(&self.links, enc);
        // custom: 4-byte BE count then (key string, json-text string).
        (self.custom.len() as u32).encode(enc);
        for (k, v) in &self.custom {
            k.encode(enc);
            let json = serde_json::to_string(v).unwrap_or_else(|_| "null".to_string());
            json.encode(enc);
        }
    }
}

impl Decode for MetadataManifest {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let schema_version = u32::decode(dec)?;
        let name = String::decode(dec)?;
        let version = Option::<String>::decode(dec)?;
        let description = Option::<String>::decode(dec)?;
        let authors = Vec::<Author>::decode(dec)?;
        let license = Option::<String>::decode(dec)?;
        let homepage = Option::<String>::decode(dec)?;
        let repository = Option::<String>::decode(dec)?;
        let keywords = Vec::<String>::decode(dec)?;
        let categories = Vec::<String>::decode(dec)?;
        let icon = Option::<String>::decode(dec)?;
        let content_type = Option::<String>::decode(dec)?;
        let links = decode_str_map(dec)?;

        let custom_count = u32::decode(dec)? as usize;
        let mut custom = BTreeMap::new();
        for _ in 0..custom_count {
            let k = String::decode(dec)?;
            let len = u32::decode(dec)? as usize;
            let raw = dec.read_bytes(len)?;
            let s = utf8_from(raw)?;
            let value: serde_json::Value =
                serde_json::from_str(&s).map_err(|_| DecodeError::Invalid("bad json"))?;
            custom.insert(k, value);
        }

        Ok(MetadataManifest {
            schema_version,
            name,
            version,
            description,
            authors,
            license,
            homepage,
            repository,
            keywords,
            categories,
            icon,
            content_type,
            links,
            custom,
        })
    }
}
