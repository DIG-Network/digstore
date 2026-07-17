# digstore-core

Core read-crypto and format primitives for the [DIG Network](https://dig.net) store
(`.dig`) format — the `no_std`-friendly, wasm-clean foundation every DIG layer
(producer, host, and in-browser verifier) shares.

It defines the on-the-wire building blocks of a `.dig` capsule and the symmetric
read-crypto used to seal and open its chunks:

- **Capsules & manifests** (`capsule`, `manifest`, `public_manifest`) — the
  content-addressed store layout and its serialized shape.
- **Merkle proofs** (`merkle`) — inclusion proofs over capsule content.
- **URN grammar** (`urn`, `urn_grammar`) — parsing and canonicalizing DIG URNs.
- **Read-crypto** (`crypto`) — the AES-256-GCM-SIV chunk seal and HKDF content-key
  derivation, with `aes-gcm-siv`'s RNG-pulling defaults disabled so the crate stays
  wasm-clean (encryption runs under a fixed nonce, no randomness required).
- **Codec, hashing, and error types** (`codec`, `hash`, `error`, `bytes`).

## Format stability

The `.dig` format is a permanent, on-chain-anchored artifact: published content stays
readable forever. Changes to this crate's format types are **additive and backwards
compatible** — a newer reader decodes every older `.dig` byte-identically.

## Features

- `std` (default) — enable `std` on the underlying dependencies.
- `serde` (default) — derive `serde` on the public types.

Disable default features (`default-features = false`) for a `no_std`, wasm-clean build.

## License

GPL-2.0-only. See [LICENSE](https://github.com/DIG-Network/dig-store/blob/main/LICENSE).
