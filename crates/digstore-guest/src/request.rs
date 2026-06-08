//! Wire request structs parsed inside the guest. Big-endian Chia streamable
//! framing (DOC DEVIATION: big-endian, not the paper's little-endian note —
//! Chia compatibility wins). Optional<T> = 1 tag byte; range = Optional<(u64,u64)>.

use alloc::vec::Vec;
use digstore_core::Bytes32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidityWindow {
    pub not_before: u64,
    pub not_after: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentRequest {
    pub retrieval_key: Bytes32,
    pub root_hash: Option<Bytes32>,
    pub range: Option<(u64, u64)>,
    pub jwt: Option<Vec<u8>>,
    pub window: Option<ValidityWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRequest {
    pub retrieval_key: Bytes32,
    pub root_hash: Option<Bytes32>,
    pub client_nonce: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError;

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.pos + n > self.b.len() {
            return Err(DecodeError);
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_be_bytes(a))
    }
    fn bytes32(&mut self) -> Result<Bytes32, DecodeError> {
        let s = self.take(32)?;
        let mut a = [0u8; 32];
        a.copy_from_slice(s);
        Ok(Bytes32(a))
    }
}

impl ContentRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.retrieval_key.0);
        match &self.root_hash {
            Some(r) => {
                out.push(1);
                out.extend_from_slice(&r.0);
            }
            None => out.push(0),
        }
        match &self.range {
            Some((a, b)) => {
                out.push(1);
                put_u64(&mut out, *a);
                put_u64(&mut out, *b);
            }
            None => out.push(0),
        }
        match &self.jwt {
            Some(j) => {
                out.push(1);
                put_u32(&mut out, j.len() as u32);
                out.extend_from_slice(j);
            }
            None => out.push(0),
        }
        match &self.window {
            Some(w) => {
                out.push(1);
                put_u64(&mut out, w.not_before);
                put_u64(&mut out, w.not_after);
            }
            None => out.push(0),
        }
        out
    }

    pub fn decode(b: &[u8]) -> Result<(Self, usize), DecodeError> {
        let mut r = Reader::new(b);
        let retrieval_key = r.bytes32()?;
        let root_hash = if r.u8()? == 1 { Some(r.bytes32()?) } else { None };
        let range = if r.u8()? == 1 { Some((r.u64()?, r.u64()?)) } else { None };
        let jwt = if r.u8()? == 1 {
            let n = r.u32()? as usize;
            Some(r.take(n)?.to_vec())
        } else {
            None
        };
        let window = if r.u8()? == 1 {
            Some(ValidityWindow { not_before: r.u64()?, not_after: r.u64()? })
        } else {
            None
        };
        Ok((
            ContentRequest { retrieval_key, root_hash, range, jwt, window },
            r.pos,
        ))
    }
}

impl ProofRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.retrieval_key.0);
        match &self.root_hash {
            Some(r) => {
                out.push(1);
                out.extend_from_slice(&r.0);
            }
            None => out.push(0),
        }
        out.extend_from_slice(&self.client_nonce);
        out
    }

    pub fn decode(b: &[u8]) -> Result<(Self, usize), DecodeError> {
        let mut r = Reader::new(b);
        let retrieval_key = r.bytes32()?;
        let root_hash = if r.u8()? == 1 { Some(r.bytes32()?) } else { None };
        let mut client_nonce = [0u8; 32];
        client_nonce.copy_from_slice(r.take(32)?);
        Ok((ProofRequest { retrieval_key, root_hash, client_nonce }, r.pos))
    }
}
