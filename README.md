# digstore

**Content-addressable, encrypted store format where the store is a self-serving, provider-blind WebAssembly module.**

`digstore` gives you Git-style commands — `init`, `add`, `commit`, `log`,
`clone`, `push`, `pull` — for a store that is **encrypted at rest** and compiles
into a **single `.wasm` file**. That one file is both your data and the server
that gates access to it. A host that stores or relays it sees only ciphertext
addressed by hashes; it cannot read what it carries.

You address content with a URN, and the URN is the key: it both locates and
decrypts. Hand someone a URN and they can read that resource; without it they
can't, and there's no separate password or access list to manage.

> New here? The full design is in the whitepaper:
> [`docs/whitepaper/digstore-whitepaper.pdf`](docs/whitepaper/digstore-whitepaper.pdf).

---

## Install

### Windows (installer)

1. Download `digstore-<version>-setup.exe` from the
   [Releases](../../releases) page.
2. Run it. It installs per-user (no admin prompt) and adds `digstore` to your
   `PATH`.
3. Open a **new** terminal and check it works:

   ```sh
   digstore --version
   ```

### Build from source (any platform)

You need [Rust](https://rustup.rs). The CLI embeds a WebAssembly guest, so build
that first:

```sh
rustup target add wasm32-unknown-unknown
cargo build -p digstore-guest --target wasm32-unknown-unknown --release
cargo build -p digstore-cli --release
```

The binary is at `target/release/digstore` (`digstore.exe` on Windows). Copy it
somewhere on your `PATH`.

---

## Quick start

`digstore` works on the directory you run it in — like `git`. `init` creates a
`.dig` store in the current folder; other commands find it by walking up from
wherever you are.

```sh
mkdir my-store && cd my-store
digstore init                      # create a store here (.dig/)

echo "hello" > readme.txt
digstore add readme.txt --key readme
digstore commit -m "first generation"

digstore log                       # list generations (each root hash = a commit)

# read a resource back. Get <storeID> and <rootHash> from `digstore log --json`:
digstore cat urn:dig:chia:<storeID>:<rootHash>/readme
```

Get your store ID and the latest root hash any time with `digstore log --json`
(the store ID is also in `.dig/config.toml`).

---

## How content is addressed: URNs

Every resource is named by a URN. The URN alone is what locates **and** decrypts
it:

```
urn:dig:<chain>:<storeID>[:<rootHash>][/<resourceKey>]
```

| Part | Meaning |
|---|---|
| `<chain>` | Chain identifier, e.g. `chia` |
| `<storeID>` | Your 64-hex store id (required) |
| `<rootHash>` | Optional: pin a specific generation; omit for the current one |
| `<resourceKey>` | Optional: which resource (the `--key` you gave on `add`) |

Example: `urn:dig:chia:ab12…ef/readme` reads the `readme` resource at the current
root. Share that string and the holder can read that resource — nothing else.

---

## Public vs private stores

```sh
digstore init             # public:  anyone with the URN can read
digstore init --private   # private: URN locates, but reading also needs a secret salt
```

- **Public** — the URN is sufficient to decrypt. Good for content you want any
  URN-holder to read.
- **Private** — decryption also requires a secret salt the publisher holds and
  shares out-of-band. A URN-holder without the salt can locate a resource but
  not read it. Pass it with `--salt <hex>` on `cat`/`checkout`.

---

## Sharing over a remote

A remote is an HTTPS endpoint that stores and serves your `.wasm` module.

```sh
# publisher side
digstore remote add origin https://example.com/stores/<storeID>
digstore push origin

# consumer side (fresh directory)
digstore clone https://example.com/stores/<storeID>
digstore cat   urn:dig:chia:<storeID>:<rootHash>/readme
digstore pull  origin          # later: fetch the publisher's newer generation
```

`clone` and `pull` **verify** what they download before installing it: the module
must match the store id you asked for, and the served root must carry the
publisher's signature. A malicious or broken server cannot feed you fabricated
content — the command fails instead. Remotes must be `https://` (plain `http://`
is allowed only for `localhost`).

---

## Command reference

| Command | What it does |
|---|---|
| `digstore init [--private]` | Create a store in the current directory |
| `digstore add <path> [--key <name>]` | Stage and chunk a file (key defaults to the file name) |
| `digstore add --discovery` | Stage a `/.well-known/dig/manifest.json` listing the resources you choose to expose |
| `digstore commit [-m <msg>]` | Seal a new generation and compile the module |
| `digstore status` | Show staged changes |
| `digstore log [--limit N]` | List generations (root hash = commit id) |
| `digstore diff <a> <b>` | Compare two generations |
| `digstore cat <urn> [--salt <hex>] [--verify-proof]` | Read a resource by URN |
| `digstore checkout <root> --out <dir> [--salt <hex>]` | Write a whole generation's contents to a directory |
| `digstore remote add\|list\|remove …` | Manage remotes |
| `digstore clone <url>` | Download and verify a store from a remote |
| `digstore push [remote]` | Publish your latest generation (default remote: `origin`) |
| `digstore pull [remote]` | Fetch and verify the remote's newer generation |

Global flags: `--dig-dir <path>` (use a specific store dir instead of
discovering one), `--json` (machine-readable output), `--verbose`.

---

## What this gives you

- **Encrypted at rest.** Content is encrypted with a key derived from its URN.
  Lose the URN, lose the read — there is no key stored anywhere to recover.
- **Provider-blind hosting.** Whoever hosts your store holds only ciphertext
  keyed by hashes. They can't scan it or read requests.
- **Verified downloads.** `clone`/`pull` reject content that isn't the genuine,
  publisher-signed store.
- **One portable file.** A store is a single `.wasm`. Copy it to back it up; run
  it to serve it.

---

## Security

Security posture, the hardening that has been applied, and known residual risks
are documented in [`SECURITY.md`](SECURITY.md). Report vulnerabilities privately
to the maintainer rather than opening a public issue.

## License

MIT.
