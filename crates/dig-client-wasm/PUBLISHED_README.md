# @dignetwork/dig-client

DIG **read-crypto** for the browser and Node, compiled to WebAssembly. This is the
published, installable form of digstore's `dig-client-wasm` crate — so consumers
`npm i` it instead of vendoring the `.wasm` with a hand-copied SHA.

The visitor's runtime does ALL the crypto, so dighub and the serving CDN stay
**blind**: they never see plaintext, the URN, or any key. The three read-path
primitives let you VIEW DIG content without trusting the host:

1. **URN reconstruction** — `reconstructUrn`, `reconstructUrnWithRoot`,
   `retrievalKey`. The CDN is addressed by `retrieval_key = SHA-256(canonical_urn)`;
   the URN itself is never sent.
2. **AES-256-GCM-SIV decryption** — `deriveKey`, `decryptChunk`, `decryptResource`,
   `decryptResourceToText`. Per-URN key via HKDF-SHA256 (public: URN alone; private:
   URN + 32-byte secret salt).
3. **Inclusion-proof verification** — `verifyInclusion`. Verifies served ciphertext
   against a root the client trusts **from the chain** (read from coinset.org, never
   from the serving response). A decoy / wrong-store / tampered response returns
   `false`.

Plus `encryptResource` (pre-encrypt a file before upload so the server compiles the
`.dig` from ciphertext alone) and `version()` (the crate version, for compat/SRI
checks).

## Install

```sh
npm i @dignetwork/dig-client
```

## Use — browser / bundler (Vite, webpack, Next)

The default and `browser`/`import` conditions resolve to the `--target web` build,
which initializes asynchronously and installs `globalThis.digClient`:

```js
import init, { retrievalKey, verifyInclusion, decryptResourceToText } from "@dignetwork/dig-client";

await init();                                   // also installs globalThis.digClient
const rk = retrievalKey(storeId, resourceKey);
const resp = await fetch(`${CDN}/stores/${storeId}/content/${rk}?root=${root}`);
const proofB64 = resp.headers.get("X-Dig-Inclusion-Proof");
const ciphertext = new Uint8Array(await resp.arrayBuffer());
const trustedRoot = /* read from coinset.org singleton state */;
if (!verifyInclusion(ciphertext, proofB64, trustedRoot)) { /* not found / decoy */ }
const html = decryptResourceToText(storeId, resourceKey, ciphertext, proofB64, trustedRoot, /*salt*/ null, /*chunkLens*/ null);
```

## Use — Node

The `node` condition resolves to the `--target nodejs` build, which initializes
synchronously (no `init()`):

```js
import { retrievalKey, verifyInclusion, decryptResourceToText, version } from "@dignetwork/dig-client";
// or: import * as dig from "@dignetwork/dig-client/node";

const ok = verifyInclusion(ciphertext, proofB64, trustedRoot);
```

Need a specific target regardless of environment? Import the explicit subpath:
`@dignetwork/dig-client/web` or `@dignetwork/dig-client/node`. The raw binary is at
`@dignetwork/dig-client/dig_client_bg.wasm`.

## Integrity (SRI / provenance)

The wasm binary is byte-identical across the web and node targets, so there is ONE
digest to pin. It is the canonical trust anchor — pin and verify it regardless of
the npm semver, so a wrong/tampered artifact fails closed even if the version looks
right.

- **`version()`** (runtime) and the package `version` are the same string.
- **`@dignetwork/dig-client/integrity.json`** ships `{ version, sha256, sri }`
  (machine-readable), and `package.json` mirrors `digIntegrity.sha256` + `.sri`.

```js
import integrity from "@dignetwork/dig-client/integrity.json" assert { type: "json" };
// integrity.sha256 — lowercase hex SHA-256 of dig_client_bg.wasm
// integrity.sri    — "sha384-…" for a <script integrity> / fetch SRI check
```

Verify the installed binary:

```sh
sha256sum node_modules/@dignetwork/dig-client/dig_client_bg.wasm
# must equal the sha256 in integrity.json
```

The current `dig_client_bg.wasm` SHA-256 is recorded in `integrity.json` and printed
by the build (`assemble-pkg`). It is the same digest for every target in a release.

## Replaces vendoring

`hub.dig.net`, `dig-embed.js`, `dig-companion`, and `dig-sdk` currently VENDOR this
wasm (hand-copied `.wasm` + glue + a re-asserted SHA). They can switch to
`npm i @dignetwork/dig-client` and pin `integrity.json`'s `sha256` instead of
maintaining a private copy. The vendored fallbacks can be removed once switched.

## License

GPL-2.0-only. Source: https://github.com/DIG-Network/digstore (crate
`crates/dig-client-wasm`).
