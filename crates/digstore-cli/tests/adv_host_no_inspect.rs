//! ADVERSARIAL verification of §18.4: "The runtime returns to the client exactly
//! what the module produced: it neither decrypts nor inspects the payload."
//!
//! Drift D-HOST-INSPECT asked whether the CLI serve path, which DECODES the
//! `ContentResponse` envelope host-side, is in tension with §18.4. It is not, and
//! this test pins the spec-exact boundary:
//!
//!   * The HOST RUNTIME (`digstore_host::HostRuntime::serve_content`) returns the
//!     module's output bytes VERBATIM — it performs no decode and no decrypt. The
//!     bytes it hands back are still the encrypted, encoded envelope.
//!   * The DECODE that `digstore_cli::ops::serve::serve_content` performs is a
//!     CLIENT-SIDE step (the `digstore` reader holds the URN and the keys, §11.3).
//!     Decoding the envelope frames is NOT decryption: the recovered
//!     `ContentResponse.ciphertext` is still AES-256-GCM ciphertext that the
//!     plaintext cannot be read from without the client key.
//!
//! Concretely we PROVE: (a) the raw host bytes contain no plaintext; (b) the
//! CLI-decoded `ContentResponse.ciphertext` still contains no plaintext (so the
//! CLI "decode" did not decrypt); (c) the plaintext is only recoverable AFTER a
//! client-side GCM open with the URN-derived key. If any future change made the
//! host or the CLI serve step decrypt/inspect content, (a) or (b) would fail.

use digstore_cli::context::CliContext;
use digstore_cli::ops::{serve, store_ops};
use digstore_core::Urn;

/// Does `haystack` contain `needle` as a contiguous byte run?
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn host_runtime_neither_decrypts_nor_inspects_the_payload() {
    // ---- Build a REAL store with a DISTINCTIVE plaintext we can scan for.
    let td = tempfile::tempdir().unwrap();
    let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
    store_ops::init_store(&ctx, false, None, None, None).unwrap();

    // A long, unique, high-entropy-looking ASCII marker so an accidental match is
    // implausible. This exact run must NEVER appear in served (ciphertext) bytes.
    let marker: &[u8] =
        b"SENTINEL-PLAINTEXT-MUST-NOT-LEAK-THROUGH-THE-HOST-RUNTIME-0xC0FFEE-9z9z9z";
    let mut original = Vec::new();
    original.extend_from_slice(b"prefix bytes; ");
    original.extend_from_slice(marker);
    original.extend_from_slice(b"; suffix bytes");

    let f = td.path().join("secret.bin");
    std::fs::write(&f, &original).unwrap();
    store_ops::add_path(&ctx, &f, Some("secret".into())).unwrap();

    let res = store_ops::commit(&ctx, None, serve::empty_manifest()).unwrap();
    let store_id = ctx.find_store_id().unwrap();
    let trusted_root = res.roothash;

    let urn = Urn {
        chain: "chia".into(),
        store_id,
        root_hash: None,
        resource_key: Some("secret".into()),
    };

    // ---- (a) RAW host bytes: drive the host runtime exactly like the CLI does and
    // confirm the bytes it returns contain no plaintext. The runtime returns the
    // module's output verbatim (§18.4) — encrypted + encoded.
    let module_path = store_ops::module_path_for(&ctx, &store_id, Some(trusted_root)).unwrap();
    let raw = serve::serve_content_raw(&ctx, &module_path, &urn).unwrap();
    assert!(
        !raw.is_empty(),
        "host must return the module's non-empty output"
    );
    assert!(
        !contains_subslice(&raw, marker),
        "SPEC §18.4 VIOLATION: plaintext marker leaked in RAW host output (host inspected/decrypted)"
    );

    // ---- (b) CLI-decoded envelope: the CLI decode is client-side framing, NOT
    // decryption. The recovered ciphertext must STILL contain no plaintext.
    let resp = serve::serve_content(&ctx, &module_path, &urn, trusted_root).unwrap();
    assert!(
        !contains_subslice(&resp.ciphertext, marker),
        "SPEC §18.4 VIOLATION: plaintext marker present after CLI decode (decode decrypted the payload)"
    );

    // ---- (c) Only a CLIENT-SIDE GCM open with the URN-derived key recovers the
    // plaintext, proving the decryption is a distinct client step (§11.3).
    let key = digstore_cli::ops::client_crypto::derive_decryption_key(&urn, None);
    let lens = store_ops::resource_chunk_lens(&ctx, &trusted_root, "secret").unwrap();
    let plan: Vec<usize> = if lens.is_empty() {
        vec![resp.ciphertext.len()]
    } else {
        lens
    };
    let mut recovered = Vec::new();
    let mut p = 0usize;
    for len in plan {
        let ct = &resp.ciphertext[p..p + len];
        p += len;
        let pt = digstore_crypto::decrypt_chunk(&key, ct)
            .expect("client GCM open must succeed with the URN key");
        recovered.extend_from_slice(&pt);
    }
    assert_eq!(
        recovered, original,
        "client-side decrypt must recover the original plaintext"
    );
    assert!(
        contains_subslice(&recovered, marker),
        "sanity: the recovered plaintext must contain the marker"
    );
}
