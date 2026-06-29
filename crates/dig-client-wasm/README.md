# dig-client-wasm

Browser **read-crypto** for dighub. The visitor's browser does ALL the crypto, so
dighub and the `cdn.dig.net` serving CDN stay blind even for public website stores
(API §17; Frontend Decision Q6). This crate compiles the canonical Digstore
read-path crypto to WebAssembly and exposes it to the frontend as the
`globalThis.digClient` object (and as a plain ES module).

## What it does

Three primitives, matching the on-chain / format crypto exactly (parity with
`digstore-crypto` is enforced by `tests/parity.rs`):

1. **URN reconstruction** — `reconstructUrn`, `reconstructUrnWithRoot`,
   `retrievalKey`. Builds the canonical `urn:dig:chia:<store_id>[/<resource_key>]`
   and the `retrieval_key = SHA-256(canonical_urn)` the CDN is addressed by
   (Digstore §6.1/§7.3; API §17). The URN itself is never sent to the server.
2. **AES-256-GCM-SIV decryption** — `deriveKey`, `decryptChunk`,
   `decryptResource`, `decryptResourceToText`. Derives the per-URN AES-256 key via
   HKDF-SHA256 (public stores: URN alone; private stores: URN + 32-byte secret
   salt) and opens the served ciphertext (Digstore §11.1/§11.2; RFC 8452).
3. **Inclusion-proof verification** — `verifyInclusion`. Verifies served
   ciphertext against a root the client trusts **from the chain** (read directly
   from coinset.org, never from the serving response): `leaf = SHA-256(ciphertext)`
   must equal the proof leaf, the merkle path must fold to `proof.root`, and
   `proof.root` must equal the chain-anchored root (Digstore §9.3; API §18). A
   decoy / wrong-store response returns `false` (treat as "not found in this
   store"); tampered bytes return `false`.

`decryptResource` runs the full gate-then-decrypt pipeline: it FIRST verifies the
inclusion proof, then derives the key and AES-256-GCM-SIV-opens the resource,
splitting a multi-chunk resource by the per-chunk ciphertext lengths
(`chunk_lens`, in order; pass `null`/empty for the common single-chunk case).

## Why a separate crate (not `digstore-crypto` directly)

`digstore-crypto` depends on `chia-bls` -> `blst` (a C/asm library that does not
compile to `wasm32-unknown-unknown`) and on `getrandom`. BLS and randomness are
on the *write/serve* path, **not** the read path. This crate is therefore
excluded from the `digstore_wasm` workspace and resolved independently: it depends
only on `digstore-core` (pure, wasm-clean) for `Urn` / `MerkleProof` / `Bytes32` /
`sha256`, plus the **same** `aes-gcm-siv` / `hkdf` / `sha2` versions and
byte-identical domain constants reproduced in `src/crypto.rs`. The native
`tests/parity.rs` cross-checks every formula against the real `digstore-crypto`.

## Published npm package — `@dignetwork/dig-client`

This crate is published to npm as **`@dignetwork/dig-client`** (roadmap #16) so
consumers `npm i` it instead of vendoring the `.wasm` with a hand-copied SHA. One
package ships BOTH wasm-bindgen targets behind conditional exports:

- `import` / `browser` / bundlers → the `--target web` async-init ES module,
- `require` / Node → the `--target nodejs` CommonJS entry (`.cjs`),

sharing ONE byte-identical `dig_client_bg.wasm` (so there is one SRI anchor).

```sh
npm i @dignetwork/dig-client
```

```js
// browser / bundler (Vite, webpack, Next):
import init, { retrievalKey, verifyInclusion, decryptResourceToText } from "@dignetwork/dig-client";
await init();

// Node:
import { verifyInclusion, version } from "@dignetwork/dig-client";        // resolves the .cjs entry
import integrity from "@dignetwork/dig-client/integrity.json" assert { type: "json" };
```

### Integrity / SRI

The wasm binary is byte-identical across targets, so there is ONE digest to pin —
the canonical trust anchor (pin it regardless of the npm semver, so a tampered
artifact fails closed even if the version looks right). The published package ships
it machine-readably:

- runtime `version()` == the package `version`,
- `@dignetwork/dig-client/integrity.json` → `{ version, sha256, sri }`,
- `package.json` mirrors `digIntegrity.{sha256,sri}`.

The current `dig_client_bg.wasm` SHA-256 is:

```
309d68af80e95e0df515ae46a8bfc049bd730de71d092d95e00275019f553e40
```

(also emitted by `npm run build:pkg`; it changes only when the crate/deps change).
**Consumers that vendor today** — `hub.dig.net`, `dig-embed.js`, `dig-companion`,
`dig-sdk` (each hand-copies the wasm + glue and re-asserts a SHA) — can switch to
`npm i @dignetwork/dig-client` and pin this digest from `integrity.json`, then drop
their vendored copies. (Cross-repo follow-on; not done here.)

## Build (locally)

```sh
# Assemble the publishable @dignetwork/dig-client package into ./pkg (both targets):
npm run build:pkg
# Verify it (exports + .d.ts + integrity + a Node runtime read-path smoke):
cargo run --example gen_smoke_fixture > smoke_fixture.json && node scripts/verify-pkg.mjs

# Or build a single target by hand:
wasm-pack build --release --target web    --out-dir pkg-web  --out-name dig_client
wasm-pack build --release --target nodejs --out-dir pkg-node --out-name dig_client
```

Plain `cargo build --target wasm32-unknown-unknown --release` also produces a raw
`.wasm` at `target/wasm32-unknown-unknown/release/dig_client_bg.wasm` if you only
want the module without wasm-bindgen JS glue.

### Publishing

`@dignetwork/dig-client` is published by `.github/workflows/publish-npm.yml` (gated
on a GitHub Release or `workflow_dispatch`), which builds + verifies the package and
runs `npm publish ./pkg --access public` with the org `NPM_TOKEN`. Bump the crate
version in `Cargo.toml` (the npm version tracks it) before cutting a release.

## Use in the browser (ES module)

```js
import init, { verifyInclusion, decryptResourceToText, retrievalKey } from "./pkg/dig_client.js";
await init();                       // also installs globalThis.digClient
const rk = retrievalKey(storeId, resourceKey);
const resp = await fetch(`${CDN}/stores/${storeId}/content/${rk}?root=<root>`);
const proofB64 = resp.headers.get("X-Dig-Inclusion-Proof");
const ciphertext = new Uint8Array(await resp.arrayBuffer());
const trustedRoot = /* read from coinset.org singleton state */;
if (!verifyInclusion(ciphertext, proofB64, trustedRoot)) { /* not found / decoy */ }
const html = decryptResourceToText(storeId, resourceKey, ciphertext, proofB64, trustedRoot, /*salt*/ null, /*chunkLens*/ null);
```

The standalone usercontent loader can instead call `globalThis.digClient.*` after
`await init()`.

## Tests

```sh
cargo test --test parity                       # native: byte-parity vs digstore-crypto
cargo run --example gen_smoke_fixture > smoke_fixture.json   # host-produced fixture
node smoke.mjs                                 # runtime: load wasm, verify + decrypt
```
