# Development Log

Durable realizations from debugging/development — with context, not a change diary. A
dev-log curator agent periodically re-verifies and garbage-collects entries.

## `update_store_metadata` (and every DataLayer/CHIP-0035 metadata-condition builder) REPLACES the whole struct — it never PATCHES

`datalayer_driver::update_store_metadata` (and the lower-level `wallet::update_store_metadata`
it wraps) does not merge its arguments into the store's existing on-chain metadata — it
constructs a brand-new `DatastoreMetadata { root_hash, label, description, bytes, size_proof }`
from exactly the values the caller passes, full stop, and asserts THAT as the new metadata
condition. Any field the caller doesn't explicitly re-supply is asserted as absent, even if the
store already carried a real value for it on chain.

This has now bitten **two** of the five `DatastoreMetadata`/ownership fields, on two separate
occasions:

- `label`/`description` — already handled correctly. Every caller (`build_update_unsigned`,
  `_multi`, `_writer`, and the CLI's `commit.rs`) re-sends the values persisted at init
  specifically because of this replace semantics; the code comments say so explicitly.
- `bytes`/`size_proof` (the store's on-chain size + size-proof) — was NOT handled. All three
  `singleton.rs` update builders hard-coded `None, None` for these two positional arguments,
  so every `digstore commit` (and every writer-delegate update) silently cleared any
  previously-recorded on-chain size — even though the freshly-synced `store: Datastore` passed
  into each builder already carries the CURRENT value at `store.info.metadata.bytes` /
  `.size_proof`. Fixed by reading those two fields into local bindings before `store` is moved
  into the `update_store_metadata` call, and passing them through instead of `None, None`
  (`crates/digstore-chain/src/singleton.rs`, all three of `build_update_unsigned`,
  `build_update_unsigned_multi`, `build_update_unsigned_writer`).

The **same shape** was already known and handled for a *different* on-chain replace: the
delegated-puzzle set updated via `updateStoreOwnership` is also a REPLACE, not an append — the
`dig-store-format` skill already documents "always re-send existing delegates plus the new
one, or you silently drop the admin."

**The lesson: treat this as a known pattern for this crate's builders, not a one-off.** Any
NEW field ever added to `DatastoreMetadata`, or any new caller of an existing
metadata/ownership-replacing builder, must be checked against this same question — "does this
caller have access to the CURRENT on-chain value, and does it re-send it?" — before it ships.
A caller that only threads through the fields it happens to care about will silently erase
every field it doesn't.

## `datalayer-driver`'s public Rust API (`lib.rs`) is a thin wrapper over `wallet.rs`

`update_store_metadata`, `update_store_ownership`, `melt_store`, etc. in `datalayer-driver`'s
top-level `lib.rs` just forward to same-named functions in its internal `wallet` module, one
positional argument at a time — no additional logic. Reading `wallet.rs` directly (rather than
guessing from the wrapper's doc comment) is the fast way to learn what a builder ACTUALLY does
with a `None`, since the wrapper's own doc comments do not describe replace-vs-merge semantics
per field.
