//! The host's accepted wasm language is pinned, not inherited from the engine.
//!
//! `wasmtime::Config`'s default proposal set is NOT stable across engine majors:
//! wasmtime 45 subtracted GC, exceptions and function-references from its
//! defaults, wasmtime 47 does not. Inheriting the default therefore silently
//! WIDENS what an untrusted serving module may do whenever the engine is
//! upgraded — the opposite of a sandbox.
//!
//! The GC proposal is the sharp one: a GC heap is a SECOND memory that the
//! store's resource limiter does not account for (`bump_resource_counts` counts
//! only `num_defined_memories()`), so a module that enables it gets a whole
//! extra `memory_size` allowance on top of its linear-memory ceiling. These
//! tests pin the accepted language explicitly so an engine upgrade can never
//! re-widen it unnoticed.

use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};

mod common;
use common::test_deps;

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    }
}

/// Assemble `wat` and assert the host refuses the module. Returns the rendered
/// error so a caller can check it is a *validation* refusal and not some
/// unrelated failure that would make the test vacuous.
fn reject(wat_src: &str, proposal: &str) -> String {
    let module_bytes = wat::parse_str(wat_src).unwrap_or_else(|e| {
        panic!("{proposal} fixture must assemble (else the test is vacuous): {e}")
    });
    let outcome = HostRuntime::new(
        &module_bytes,
        cfg(),
        ExecutionLimits::default(),
        test_deps(FixedClock::new(1_700_000_000)),
    );
    match outcome {
        Ok(_) => panic!("host accepted a module using the {proposal} proposal"),
        Err(e) => format!("{e:?}"),
    }
}

#[test]
fn gc_proposal_is_rejected() {
    // A GC array allocation — the primitive that would open an unaccounted
    // second heap alongside linear memory.
    let err = reject(
        r#"(module
             (type $bytes (array i8))
             (memory (export "memory") 1 256)
             (func $s (drop (array.new_default $bytes (i32.const 1))))
             (start $s)
             (func (export "alloc") (param i32) (result i32) (i32.const 1024))
             (func (export "dealloc") (param i32) (param i32))
             (func (export "init") (result i32) (i32.const 0)))"#,
        "GC",
    );
    assert!(
        err.contains("Validation"),
        "expected a validation refusal, got {err}"
    );
}

#[test]
fn exceptions_proposal_is_rejected() {
    let err = reject(
        r#"(module
             (tag $e)
             (memory (export "memory") 1 256)
             (func (export "alloc") (param i32) (result i32) (i32.const 1024))
             (func (export "dealloc") (param i32) (param i32))
             (func (export "init") (result i32) (i32.const 0)))"#,
        "exceptions",
    );
    assert!(
        err.contains("Validation"),
        "expected a validation refusal, got {err}"
    );
}

#[test]
fn function_references_proposal_is_rejected() {
    // A non-nullable typed function reference: function-references only.
    let err = reject(
        r#"(module
             (type $ft (func))
             (memory (export "memory") 1 256)
             (func $callee)
             (func $s (local $r (ref $ft)) (local.set $r (ref.func $callee)))
             (start $s)
             (elem declare func $callee)
             (func (export "alloc") (param i32) (result i32) (i32.const 1024))
             (func (export "dealloc") (param i32) (param i32))
             (func (export "init") (result i32) (i32.const 0)))"#,
        "function-references",
    );
    assert!(
        err.contains("Validation"),
        "expected a validation refusal, got {err}"
    );
}
