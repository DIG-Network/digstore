<!-- Thanks for contributing to digstore! -->

## What this changes
A clear description of the change and why.

## Related issue
Closes #

## Checklist
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::default_constructed_unit_structs -A clippy::field_reassign_with_default` is clean
- [ ] `cargo test --workspace --locked` passes (guest WASM rebuilt if the guest changed)
- [ ] `cargo deny check advisories bans sources` is clean
- [ ] Commits are signed; no `Co-Authored-By` trailers
- [ ] Docs / `SECURITY.md` updated if behavior or threat model changed
