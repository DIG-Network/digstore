//! Build script: compile the branded DIG icon into `dig-resolver`.
//!
//! `embed_resource::compile` links the compiled `.res` with
//! `cargo:rustc-link-arg-bins`, which reaches every bin this crate ships —
//! here, just `dig-resolver` — so no `compile_for` scoping is needed.
//!
//! The result is checked rather than discarded: an environment that cannot
//! compile a resource would otherwise silently produce an unbranded binary,
//! which is precisely the failure this build step exists to prevent.
//!
//! No-op on non-Windows.

fn main() {
    #[cfg(windows)]
    embed_icon();
}

#[cfg(windows)]
fn embed_icon() {
    embed_resource::compile("../../assets/dig.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile assets/dig.rc — no usable Windows resource compiler?");

    println!("cargo:rerun-if-changed=../../assets/dig.rc");
    println!("cargo:rerun-if-changed=../../assets/dig.ico");
}
