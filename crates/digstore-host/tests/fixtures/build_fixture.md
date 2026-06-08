# Building the `e2e_guest` integration fixture

The `tests/e2e_guest.rs` suite is `#[ignore]`-marked and drives the host against
the **real** compiled `digstore-guest` serving module. It needs two artifacts
that are produced from the other crates (not checked in):

- `tests/fixtures/sample.wasm` — a compiled Digstore serving module
- `tests/fixtures/hello_request.bin` — a content request for a known resource

## Steps

1. Build the guest template to wasm:

   ```sh
   cargo build -p digstore-guest --target wasm32-unknown-unknown --release
   # -> target/wasm32-unknown-unknown/release/digstore_guest.wasm
   ```

2. Seed a tiny store and compile it with `digstore-compiler` (once that crate
   lands). Use a deterministic store:
   - `store_id` = all-`0x00`
   - one resource `hello.txt` with a known plaintext (e.g. `b"hello digstore"`)

   The compiler injects the data/key-table/pool sections into the guest
   template above and writes the finished module to
   `crates/digstore-host/tests/fixtures/sample.wasm`.

3. Produce the matching content request and write it to
   `crates/digstore-host/tests/fixtures/hello_request.bin`. The request bytes
   are the retrieval key + trusted root + (optional) byte range, encoded per the
   guest's `get_content` ABI.

## Expected values (record after the first successful build)

| field      | value |
|------------|-------|
| `store_id` | `0000…0000` (32 bytes) |
| `roothash` | _record the compiled root here_ |
| request    | _record the hex of `hello_request.bin` here_ |

## Running the gated suite

```sh
cargo test -p digstore-host --test e2e_guest -- --ignored
```

If the fixture is missing, `load()` panics with these build instructions rather
than silently passing — the default suite leaves these four tests `ignored`.
