//! The **single normative grammar** for the Digstore URN, plus the contract every
//! implementation's URN parser must conform to.
//!
//! Four parsers historically parsed `urn:dig:…` independently — this crate's
//! [`Urn`](crate::urn::Urn), the `dig-sdk` `URN_RE`, the extension `dig-urn.mjs`,
//! and the browser C++ `ParseDigUrn` — with no shared conformance suite, so they
//! drifted (see the divergence notes below). This module fixes ONE grammar as the
//! source of truth and pins it with a frozen vector file
//! (`tests/fixtures/urn_conformance.json`) that the Rust parser is tested against in
//! [`tests/urn_conformance.rs`]; the other ports are expected to run the same frozen
//! vectors so every implementation conforms to one definition.
//!
//! This module deliberately adds **no parsing code** — [`Urn::parse`] /
//! [`Urn::canonical`] / [`Urn::retrieval_key`] remain the implementation. The
//! grammar here DESCRIBES that implementation; the conformance test PROVES the two
//! agree (and is the guard against future drift).
//!
//! # Normative grammar (ABNF, RFC 5234)
//!
//! ```abnf
//! dig-urn       = "urn:dig:" chain ":" store-id [ ":" root-hash ] [ "/" resource ]
//!
//! chain         = 1*chain-char        ; non-empty; canonical value is "chia"
//! chain-char    = ALPHA / DIGIT / "-" ; (see "Chain segment" note — the parser is
//!                                     ; permissive; deployed content uses "chia")
//!
//! store-id      = 64HEXDIG            ; the CHIP-0035 singleton launcher id, 32 bytes
//! root-hash     = 64HEXDIG            ; a capsule's on-chain root, 32 bytes (optional)
//! resource      = *pchar             ; the resource path/key, verbatim after the
//!                                     ; FIRST "/", may itself contain "/" (optional)
//!
//! HEXDIG        = DIGIT / "a" / "b" / "c" / "d" / "e" / "f"   ; lowercase, canonical
//! ```
//!
//! Notes that make the grammar *normative* (the parser's actual behaviour):
//!
//! * **Prefix** is the literal `urn:dig:`. Anything else is rejected.
//! * **Resource split is at the FIRST `/`.** Everything before it is
//!   `chain:store-id[:root-hash]`; everything after is the resource (which may
//!   contain further `/`). A trailing-but-empty resource (`…/`) parses as
//!   `resource = ""` (an empty string), distinct from an absent resource.
//! * **Colon arity in the head is exactly 2 or 3 segments**
//!   (`chain:store-id` or `chain:store-id:root-hash`). A 4th `:`-segment is rejected
//!   (`too many ':' segments`).
//! * **`store-id` and `root-hash` are 32-byte lowercase hex.** A non-hex or
//!   wrong-length value is rejected by [`Bytes32::from_hex`](crate::bytes::Bytes32).
//! * **Canonical form** ([`Urn::canonical`](crate::urn::Urn::canonical)) re-emits
//!   `urn:dig:<chain>:<store-id-hex>[:<root-hash-hex>][/<resource>]`, store-id and
//!   root-hash as lowercase hex, omitting absent fields. Parsing then re-canonicalising
//!   is idempotent for any canonical input.
//! * **Retrieval key** is `SHA-256(canonical())` as raw 32 bytes (lowercase hex on the
//!   wire). It is the *only* identifier sent to a CDN/RPC.
//!
//! # The `?salt` query — intentionally NOT part of the URN identity
//!
//! Some edge parsers historically accepted a `?salt=…` query. **The secret salt is a
//! private-store *decryption-key* input, never part of the canonical URN or the
//! retrieval key**: a private store derives its AES key from `canonical_urn + salt`,
//! but the retrieval key (what the host sees) stays `SHA-256(canonical_urn)` — by
//! design the host cannot tell a private store from a public one. So this grammar has
//! **no `?salt` production**: the canonical URN carries no salt, and the salt is a
//! separate parameter to key derivation (see
//! [`derive_decryption_key`](crate::derive_decryption_key) and the wasm
//! `salt_hex` argument). The conformance vectors record that a `?salt` suffix is NOT
//! stripped by the core parser (it would land in the resource), so any edge parser
//! that strips `?salt` must do so OUTSIDE the canonical-URN derivation.
//!
//! # Known cross-implementation divergences (recorded, not "fixed" here)
//!
//! This module is the source of truth; these are the deltas other parsers must
//! reconcile against it (task #128 — establish the vectors + verify the Rust parser;
//! converging the ports is tracked separately so we do not silently rewrite a parser
//! a published artifact depends on):
//!
//! 1. **Chain segment.** The core parser (and these vectors) accept ANY non-empty
//!    chain token — real frozen content uses `chia` ([`crate::CHAIN`]) but the KDF
//!    known-answer vectors and the URN unit tests also exercise `mainnet`/`testnet`,
//!    so the deployed corpus is multi-chain-labelled. `dig-sdk`'s `URN_RE` requires
//!    the literal `chia` AND a mandatory `/resource`, so it REJECTS both a
//!    `mainnet`/`testnet` URN and a bare `urn:dig:chia:<store>` that core accepts.
//!    The normative position: chain is a non-empty token; `chia` is canonical; a
//!    conforming parser SHOULD accept the bare (resourceless) form. Tightening core
//!    to reject non-`chia` chains is deferred because it would break the frozen KDF
//!    KAT corpus — a breaking corpus change, out of scope for "establish the vectors".
//! 2. **Salt.** `dig-sdk` accepted any-length hex salt, the wasm exactly 64 hex,
//!    the extension lowercase-only. Per the section above, salt is NOT in the URN
//!    identity at all in this normative grammar; a conforming validator that surfaces
//!    a salt parameter MUST require exactly 32 bytes / 64 lowercase hex.
//! 3. **Resource optionality.** Core treats the resource as OPTIONAL (a bare
//!    `urn:dig:chia:<store>` is valid and names the store, not a resource); the
//!    extension's tolerant fallback agrees; `dig-sdk` makes `/resource` MANDATORY.
//!    Normative: resource is OPTIONAL.

/// The normative URN grammar as ABNF (RFC 5234) text — the machine-/human-readable
/// single source of truth, embedded so an agent can introspect it without leaving
/// the crate. Kept byte-identical to the module-doc grammar block above.
pub const URN_ABNF: &str = "\
dig-urn       = \"urn:dig:\" chain \":\" store-id [ \":\" root-hash ] [ \"/\" resource ]\n\
chain         = 1*chain-char        ; non-empty; canonical value is \"chia\"\n\
chain-char    = ALPHA / DIGIT / \"-\"\n\
store-id      = 64HEXDIG            ; CHIP-0035 singleton launcher id, 32 bytes\n\
root-hash     = 64HEXDIG            ; a capsule's on-chain root, 32 bytes (optional)\n\
resource      = *pchar             ; verbatim after the FIRST \"/\" (optional)\n\
HEXDIG        = DIGIT / \"a\" / \"b\" / \"c\" / \"d\" / \"e\" / \"f\"\n";

/// The canonical chain tag a conforming URN SHOULD carry (`"chia"`). Re-exported
/// from [`crate::CHAIN`] so the grammar module names the canonical value in one place.
pub const CANONICAL_CHAIN: &str = crate::CHAIN;
