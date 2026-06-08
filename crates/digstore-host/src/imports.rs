//! Registration of the eight `dig_host` import functions (§6.3, §12, §18.3).

use crate::error::HostError;
use crate::runtime::RuntimeState;
use wasmtime::Linker;

pub fn register(_linker: &mut Linker<RuntimeState>) -> Result<(), HostError> {
    // Imports added in Task 11. The echo fixture imports nothing, so an empty
    // linker instantiates it successfully.
    Ok(())
}
