//! Wasm ABI exports (§6.2). Thin wrappers: parse request -> call pure logic ->
//! encode response -> pack ptr/len. Wasm-only.
//!
//! CONVENTIONS C3: `get_content` returns a serialized `ContentResponse`;
//! `get_proof` returns a serialized `ProofPrelude` (NOT an `ExecutionProof` — the
//! guest cannot produce ZK proofs in wasm; the host wraps the prelude later).

use crate::content::{serve_content, ContentOutcome, GateConfig};
use crate::datasection::embedded;
use crate::host::WasmHost;
use crate::metadata;
use crate::packing::guest_pack;
use crate::proof::{serve_proof, ProofOutcome};
use crate::request::{ContentRequest, ProofRequest};
use alloc::vec::Vec;
use digstore_core::codec::{Encode, Encoder};

/// Encode any core wire struct to a byte vec via the shared big-endian Encoder.
fn encode_to_vec<T: Encode>(value: &T) -> Vec<u8> {
    let mut enc = Encoder::new();
    value.encode(&mut enc);
    enc.finish()
}

/// Leak a Vec into linear memory and return its packed ptr/len.
fn ret(bytes: Vec<u8>) -> i64 {
    let len = bytes.len() as u32;
    let boxed = bytes.into_boxed_slice();
    let ptr = boxed.as_ptr() as u32;
    core::mem::forget(boxed);
    guest_pack(ptr, len)
}

#[no_mangle]
pub extern "C" fn init() -> i32 {
    // §5.1: keep all eight dig_host import declarations alive in the Import
    // section. The anchor never calls the host at runtime (its body is behind a
    // never-taken, optimizer-opaque branch) but ties the import declarations
    // into the reachable call graph from an export so the linker emits them.
    #[cfg(target_arch = "wasm32")]
    {
        return crate::imports::retain_dig_host_imports();
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

#[no_mangle]
pub extern "C" fn alloc(size: i32) -> i32 {
    let v: Vec<u8> = Vec::with_capacity(size as usize);
    let ptr = v.as_ptr() as i32;
    core::mem::forget(v);
    ptr
}

#[no_mangle]
pub extern "C" fn dealloc(_ptr: i32, _size: i32) {
    // Bump allocator never frees; intentional no-op.
}

#[no_mangle]
pub extern "C" fn get_store_id() -> i64 {
    ret(metadata::store_id(&embedded()).0.to_vec())
}

#[no_mangle]
pub extern "C" fn get_current_roothash() -> i64 {
    ret(metadata::current_roothash(&embedded()).0.to_vec())
}

#[no_mangle]
pub extern "C" fn get_roothash_history() -> i64 {
    ret(metadata::roothash_history(&embedded()))
}

#[no_mangle]
pub extern "C" fn get_public_key() -> i64 {
    ret(metadata::public_key(&embedded()).0.to_vec())
}

#[no_mangle]
pub extern "C" fn get_metadata() -> i64 {
    ret(metadata::metadata_bytes(&embedded()))
}

#[no_mangle]
pub extern "C" fn get_authentication_info() -> i64 {
    ret(metadata::authentication_info(&embedded()))
}

fn read_req(ptr: i32, len: i32) -> Vec<u8> {
    unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec() }
}

#[no_mangle]
pub extern "C" fn get_content(req_ptr: i32, req_len: i32) -> i64 {
    let raw = read_req(req_ptr, req_len);
    let req = match ContentRequest::decode(&raw) {
        Ok((r, _)) => r,
        Err(_) => return guest_pack(0xFFFF_FFFF, 0), // error sentinel
    };
    let cfg = GateConfig {
        require_attestation: true,
        require_jwt: false,
        expected_iss: None,
        expected_aud: None,
    };
    let resp = match serve_content(&WasmHost, &embedded(), &req, &cfg) {
        ContentOutcome::Real(r) | ContentOutcome::Decoy(r) => r,
    };
    ret(encode_to_vec(&resp))
}

#[no_mangle]
pub extern "C" fn get_proof(req_ptr: i32, req_len: i32) -> i64 {
    let raw = read_req(req_ptr, req_len);
    let req = match ProofRequest::decode(&raw) {
        Ok((r, _)) => r,
        Err(_) => return guest_pack(0xFFFF_FFFF, 0),
    };
    let cfg = GateConfig {
        require_attestation: true,
        require_jwt: false,
        expected_iss: None,
        expected_aud: None,
    };
    // CONVENTIONS C3: serialize a ProofPrelude, not an ExecutionProof.
    let prelude = match serve_proof(&WasmHost, &embedded(), &req, &cfg) {
        ProofOutcome::Real(p) | ProofOutcome::Decoy(p) => p,
    };
    ret(encode_to_vec(&prelude))
}
