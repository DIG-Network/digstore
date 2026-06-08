//! Helpers for reading/writing the guest's linear memory (§6.4).

use crate::error::HostError;
use wasmtime::{AsContext, AsContextMut, Memory};

/// Read `len` bytes at `ptr` from guest memory.
pub fn read_bytes(
    store: impl AsContext,
    mem: &Memory,
    ptr: u32,
    len: u32,
) -> Result<Vec<u8>, HostError> {
    let data = mem.data(&store);
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or(HostError::OutOfBounds)?;
    if end > data.len() {
        return Err(HostError::OutOfBounds);
    }
    Ok(data[start..end].to_vec())
}

/// Write `bytes` at `ptr` into guest memory.
pub fn write_bytes(
    mut store: impl AsContextMut,
    mem: &Memory,
    ptr: u32,
    bytes: &[u8],
) -> Result<(), HostError> {
    let data = mem.data_mut(&mut store);
    let start = ptr as usize;
    let end = start
        .checked_add(bytes.len())
        .ok_or(HostError::OutOfBounds)?;
    if end > data.len() {
        return Err(HostError::OutOfBounds);
    }
    data[start..end].copy_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Engine, Instance, Module, Store};

    fn fixture() -> (Store<()>, Memory) {
        let engine = Engine::default();
        let wat = include_str!("../tests/fixtures/wat/echo.wat");
        let bytes = wat::parse_str(wat).unwrap();
        let module = Module::new(&engine, &bytes).unwrap();
        let mut store = Store::new(&engine, ());
        let instance = Instance::new(&mut store, &module, &[]).unwrap();
        let mem = instance.get_memory(&mut store, "memory").unwrap();
        (store, mem)
    }

    #[test]
    fn write_and_read_round_trip() {
        let (mut store, mem) = fixture();
        write_bytes(&mut store, &mem, 512, &[1, 2, 3, 4, 5]).unwrap();
        let got = read_bytes(&store, &mem, 512, 5).unwrap();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn read_out_of_bounds_errors() {
        let (store, mem) = fixture();
        let err = read_bytes(&store, &mem, u32::MAX, 16).unwrap_err();
        assert!(matches!(err, crate::error::HostError::OutOfBounds));
    }
}
