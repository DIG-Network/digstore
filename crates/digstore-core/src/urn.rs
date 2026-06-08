//! URN parsing, canonicalization and retrieval-key derivation (paper 6.1, 6.5).
//!
//! Format: `urn:dig:<chain>:<storeID>[:<rootHash>][/<resourceKey>]`
//! - `retrieval_key = SHA-256(canonical())`

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::error::CoreError;
use crate::hash::sha256;
use alloc::format;
use alloc::string::{String, ToString};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Urn {
    pub chain: String,
    pub store_id: Bytes32,
    pub root_hash: Option<Bytes32>,
    pub resource_key: Option<String>,
}

impl Urn {
    /// Parse a URN string. Accepts omitted rootHash and/or resourceKey.
    pub fn parse(input: &str) -> Result<Urn, CoreError> {
        let rest = input
            .strip_prefix("urn:dig:")
            .ok_or_else(|| CoreError::Parse("missing 'urn:dig:' prefix".to_string()))?;

        // Split off the optional resource path at the FIRST '/'.
        let (head, resource_key) = match rest.split_once('/') {
            Some((h, r)) => (h, Some(r.to_string())),
            None => (rest, None),
        };

        // head = <chain>:<storeID>[:<rootHash>]
        let mut parts = head.split(':');
        let chain = parts
            .next()
            .filter(|c| !c.is_empty())
            .ok_or_else(|| CoreError::Parse("missing chain".to_string()))?
            .to_string();
        let store_id_hex = parts
            .next()
            .ok_or_else(|| CoreError::Parse("missing store id".to_string()))?;
        let store_id = Bytes32::from_hex(store_id_hex)?;
        let root_hash = match parts.next() {
            Some(rh) => Some(Bytes32::from_hex(rh)?),
            None => None,
        };
        if parts.next().is_some() {
            return Err(CoreError::Parse("too many ':' segments".to_string()));
        }

        Ok(Urn {
            chain,
            store_id,
            root_hash,
            resource_key,
        })
    }

    /// Render the canonical URN string.
    pub fn canonical(&self) -> String {
        let mut s = format!("urn:dig:{}:{}", self.chain, self.store_id.to_hex());
        if let Some(rh) = &self.root_hash {
            s.push(':');
            s.push_str(&rh.to_hex());
        }
        if let Some(rk) = &self.resource_key {
            s.push('/');
            s.push_str(rk);
        }
        s
    }

    /// `retrieval_key = SHA-256(canonical())`.
    pub fn retrieval_key(&self) -> Bytes32 {
        sha256(self.canonical().as_bytes())
    }
}

impl Encode for Urn {
    fn encode(&self, enc: &mut Encoder) {
        self.chain.encode(enc);
        self.store_id.encode(enc);
        self.root_hash.encode(enc);
        self.resource_key.encode(enc);
    }
}

impl Decode for Urn {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Urn {
            chain: String::decode(dec)?,
            store_id: Bytes32::decode(dec)?,
            root_hash: Option::<Bytes32>::decode(dec)?,
            resource_key: Option::<String>::decode(dec)?,
        })
    }
}
