//! Obfuscation hooks (§17). The guest does NOT obfuscate itself; the
//! digstore-compiler applies WASM-level passes (instruction substitution,
//! opaque predicates, bogus code, control-flow nops). This module exposes a
//! stable, no-op "opaque predicate" seam the compiler can recognize and expand,
//! plus a marker so the pass can locate hookable functions. Keeping the seam
//! here (rather than ad hoc) makes the obfuscation pass deterministic (§19.3).

/// An opaque-predicate seam: always returns true, but is shaped so the compiler
/// pass can replace it with a non-trivially-true predicate over injected state.
#[inline(never)]
pub fn opaque_true() -> bool {
    // The compiler pass rewrites this body; the default must be semantically true.
    core::hint::black_box(true)
}

/// Marker the obfuscation pass scans for to find hookable control points.
#[inline(never)]
pub fn obfuscation_anchor() {
    core::hint::black_box(0u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_true_is_true_by_default() {
        assert!(opaque_true(), "default seam must be semantically true");
    }

    #[test]
    fn anchor_is_callable() {
        obfuscation_anchor(); // must not panic; presence is the contract
    }
}
