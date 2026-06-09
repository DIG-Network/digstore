# Artifact 3 — S3-Compatible Host (`dighost`)

The three deliverable artifacts:
1. **`digstore`** — the git-like developer binary (already built; `crates/digstore-cli`).
2. **The `.wasm` datastore** — `{storeID}-{rootHash}.wasm`, produced by `digstore commit` (already built; `crates/digstore-compiler`).
3. **`dighost`** — the runnable host that instantiates a module and **streams content out by retrieval key**, reading the module/objects from an **S3-compatible object store** so it can run on AWS S3. *(This doc.)*

## Role (§15, §18)
`dighost` is the DIG Node / neutral pipe. It:
- loads a compiled module from a **storage source** (local path, `s3://bucket/key`, or any S3-compatible endpoint),
- instantiates it via `digstore_host::HostRuntime` (real wasmtime, ABI §6),
- on a request carrying a **32-byte retrieval key**, calls `serve_content` and **streams the served bytes** (`ContentResponse` = ciphertext + merkle proof, or an indistinguishable decoy) to the output,
- **never decrypts** — it holds no URN and no decryption key (provider blindness is structural).

## S3 compatibility
Storage is abstracted with the **`object_store`** crate, which supports:
- `object_store::aws::AmazonS3` — **AWS S3** and any **S3-compatible endpoint** (MinIO, Cloudflare R2, Wasabi) via `--endpoint` + `--region` + `--allow-http`,
- `object_store::local::LocalFileSystem` — local files,
- `object_store::memory::InMemory` — tests.

CLI surface:
```
dighost serve --module <local-path | s3://bucket/key> [--endpoint URL] [--region R] \
              --host-key <hex|file> --retrieval-key <64-hex> [--root <64-hex>] [--out <file>]
dighost serve --module s3://bucket/key --urn <urn:dig:...>   # derive retrieval key from URN (host still never stores the URN)
```
- `--module s3://…` reads the module bytes from the bucket (AWS creds via the standard env/instance chain; `--endpoint` for S3-compatible).
- Output: raw `ContentResponse` bytes to stdout (or `--out`). With `--urn`, the retrieval key is derived locally for convenience; the host itself only ever uses the hash.

## Attestation key (§12)
The compiled module embeds trusted host BLS keys. To serve **real** content (not a decoy), `dighost`'s BLS key must be in the module's trusted set. `--host-key` accepts the host signing secret (the `signing_key.bin` written by `digstore init`, or raw hex). Without a trusted key the gate fails closed → decoys (still streamable, indistinguishable on the wire).

## Deployment: "running on AWS S3"
- **Bucket-backed serving (this binary):** point `dighost` at `s3://bucket/{storeID}-{root}.wasm`; it serves content GET-by-retrieval-key from the bucket. Run it on EC2/ECS/Lambda in front of the bucket.
- **S3 Object Lambda (in-S3 execution):** the same `serve_content` path is invokable from an S3 Object Lambda handler so a `GET` of a retrieval key transforms into the served `ContentResponse`. The core logic is the binary's `serve` function; a thin Lambda adapter is the only delta (documented as the deployment wrapper, not re-implemented here).

## Tests (must stay green)
- Local `object_store` (LocalFileSystem + InMemory) loads a REAL compiled fixture module and `serve`-by-retrieval-key returns a non-empty `ContentResponse` whose proof verifies to the trusted root (reuse the `adv_self_serve` fixture path).
- A miss retrieval key → decoy (proof does not verify); identical wire shape.
- The S3 code path is exercised against `object_store::memory::InMemory` (and, if a `DIGHOST_S3_TEST_ENDPOINT` env is set, a live MinIO) so S3 wiring is covered without requiring AWS in CI.
- Host never decrypts: assert the served bytes are ciphertext (do not equal plaintext) and decryption only succeeds in a separate client step.
