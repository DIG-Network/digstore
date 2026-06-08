# Digstore Security Properties & Doc-Only Sections

These paper sections are **properties / threat-model statements**, not standalone code units. Each is realized by behavior owned elsewhere and verified by named tests. Tracked here so coverage is explicit.

## §8.5 Social Conventions (well-known resource keys)
Implementable but needs **no special engine code** — conventions are ordinary resource keys.
- `index.html` — default landing resource. CLI `cat <urn-without-resourceKey>` resolves to `index.html` when present.
- `/.well-known/dig/manifest.json` — machine-readable discovery list. CLI `digstore add` MAY write one; reading it is a normal `cat`.
- **Owner:** `digstore-cli` (default-resource fallback + optional discovery-manifest helper). **Verify:** cli test `cat` on a store whose only resource is `index.html` returns it when resourceKey omitted; a store with a `/.well-known/dig/manifest.json` resource returns it verbatim.
- Privacy preserved: secret-keyed resources remain unlisted because nothing maps a public name to them.

## §9.4 Generation root = Merkle tree root
Not separate code — it is the invariant `Generation.state.root == Generation.tree.root()`.
- **Owner:** `digstore-core::merkle` (root computation) + `digstore-store` (commit sets `state.root = tree.root()`). **Verify:** core merkle test asserts `MerkleTree::root()` equals the known root for fixed leaves; store commit test asserts the persisted `GenerationState.root` equals the freshly recomputed tree root.

## §10 Threat Model
Adversaries: curious/malicious provider, network observer, enumerating client. Guarantees map to mechanisms:
| Guarantee | Mechanism | Owner / verifying test |
|-----------|-----------|------------------------|
| Confidentiality | URN-derived key, client-only decrypt | crypto §11, cli §11.3 |
| Integrity | Merkle proof + GCM tag + execution proof | core §9, crypto §11.2, prover §13 |
| Blindness | retrieval/decryption keys are URN functions | guest/host serve hashes+ciphertext only |
| Indistinguishability | deterministic decoys + oblivious access | guest §14 |
No standalone code. Each row's test lives in the named crate.

## §15 Provider Blindness (structural property)
Consequence of: provider receives only `retrieval_key` (a hash) + returns ciphertext; never holds the URN or decryption key.
- **Verify (integration):** a host-side test that serves `get_content` for a real URN and asserts the host never receives URN/plaintext — the response is ciphertext + proof; decryption only succeeds in a separate client step holding the URN. Owner: `digstore-host` integration test + `digstore-cli` round-trip.

## §17.2 Secretless Module
Property: the compiled `.wasm` embeds no decryption key, no signing key, no proving secret.
- **Verify:** `digstore-compiler` test scans the emitted module's data section and asserts it contains only ciphertext, public metadata, public keys (trusted host keys, store public key) — and that no `SecretSalt`, no BLS secret, no decryption key byte-pattern is present. Owner: `digstore-compiler` (`test_module_is_secretless`).

## §17.1 Obfuscation (this IS code)
Owned by `digstore-compiler` (deterministic instruction substitution, opaque predicates, bogus code, control-flow nops at WASM level). Listed here only to note: obfuscation must stay deterministic (deviation: byte-identical recompile) and must preserve `get_content` behavior (verified by running obf vs non-obf modules through the host and asserting identical output).
