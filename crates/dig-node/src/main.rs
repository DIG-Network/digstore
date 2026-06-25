//! dig-node standalone binary — a thin wrapper over the `dig_node` library's
//! [`dig_node::run`]. The SAME service runs natively in-process inside the DIG
//! browser via the `dig-runtime` cdylib (no sidecar); this binary exists only
//! for standalone use and testing.

fn main() {
    let rt = tokio::runtime::Runtime::new().expect("dig-node: tokio runtime");
    rt.block_on(dig_node::run());
}
