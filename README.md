<h1 align="center">digstore</h1>

<p align="center">
  <strong>A Git-shaped, encrypted, content-addressable project that compiles to a single self-defending WebAssembly module.</strong>
</p>

<p align="center">
  <a href="https://github.com/DIG-Network/digstore/actions/workflows/ci.yml"><img src="https://github.com/DIG-Network/digstore/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/DIG-Network/digstore/releases"><img src="https://img.shields.io/github/v/release/DIG-Network/digstore?sort=semver" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--2.0-blue.svg" alt="License: GPL-2.0"></a>
  <img src="https://img.shields.io/badge/platforms-macOS%20%C2%B7%20Linux%20%C2%B7%20Windows-555" alt="Platforms">
  <img src="https://img.shields.io/badge/rust-1.94.1-orange.svg" alt="Rust 1.94.1">
</p>

---

`digstore` gives you Git-style commands — `init`, `add`, `commit`, `log`, `clone`,
`push`, `pull` — for a project that is **encrypted at rest** and compiles into a
**single `.wasm` file**. That one file is both your data and the server that gates
access to it. A host that stores or relays it sees only ciphertext addressed by
hashes; it cannot read what it carries.

You address content with a URN, and the URN *is* the key: it both locates and
decrypts. Hand someone a URN and they can read that resource; without it they
can't, and there's no separate password or access list to manage.

Unlike Git, digstore is built for **build output**, not repo source — you point a
project at a directory like `dist/` and it captures what's there.

> New here? The full design is in the whitepaper:
> [`docs/whitepaper/digstore-whitepaper.pdf`](docs/whitepaper/digstore-whitepaper.pdf).

---

## Install

### Universal installer (recommended)

The DIG installer downloads the right `digstore` binary for your OS and adds it
to your `PATH`. It lives in its own repo,
[**DIG-Network/dig-installer**](https://github.com/DIG-Network/dig-installer)
(the GUI desktop installer — the single-file `DigStore-Setup-*` — lives there
too, and it can optionally also install the `dig-node` local node).

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/DIG-Network/dig-installer/main/install.sh | sh
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/DIG-Network/dig-installer/main/install.ps1 | iex
```

Then open a **new** terminal and check it works:

```sh
digstore --version
```

You can also grab the raw per-OS `digstore` binary directly from this repo's
[Releases](https://github.com/DIG-Network/digstore/releases) page
(`digstore-<ver>-<os_arch>`) and drop it on your `PATH`.

### Build from source (any platform)

You need [Rust](https://rustup.rs) (pinned to 1.94.1 via `rust-toolchain.toml`).
The CLI embeds a WebAssembly guest, so build that first:

```sh
rustup target add wasm32-unknown-unknown
cargo build -p digstore-guest --target wasm32-unknown-unknown --release
cargo build -p digstore-cli --release
```

The binary is at `target/release/digstore` (`digstore.exe` on Windows). Copy it
somewhere on your `PATH`.

---

## Quick start

```sh
mkdir my-project && cd my-project
digstore init                      # create a .dig workspace + a "default" project

echo "hello" > readme.txt
digstore add readme.txt --key readme
digstore commit -m "first deployment"

digstore log                       # list deployments (each root hash = a commit)
digstore urn readme.txt            # preview the URN a file will have — no guessing

# read a resource back (store id + root come from `digstore log --json`):
digstore cat urn:dig:chia:<storeID>:<rootHash>/readme
```

Commands discover the `.dig/` workspace by walking up from wherever you are (like
Git). `add`/`urn` operate on the project's **content root** (the current directory
by default; commonly a build dir — see below), and resource keys are always
relative to that root, so URNs are stable no matter which subdirectory you run
from.

---

## Multiple projects per workspace

A single `.dig/` workspace can hold many projects, each with its own content,
keys, and history (a project accrues a series of **deployments** as you commit).

```sh
digstore init site --dir dist      # a project named "site" that captures ./dist
digstore init docs --dir build/docs
digstore stores                    # list projects; * marks the active one + capacity
digstore use site                  # switch the active project

digstore --store site add -A       # stage everything under dist/ into "site"
digstore staged                    # what's staged + size + remaining headroom
digstore unstage                   # clear staging
digstore commit -m "v1"            # seal a deployment; writes a local urns.json index
```

- **Project selection:** `--store <name>` (alias: `--project`) > the active
  project (`use`) > the single project if there's only one.
- **Content root:** each project captures a build directory (default: the current
  dir; set with `--dir` at `init` or `digstore dir <path>`). `-C/--cwd <path>`
  overrides it for one command.
- **Per-project cap:** each project is capped at **128 MB** of staged content,
  enforced at `add` (and defensively at `commit`); remaining capacity is shown by
  `add`, `status`, `staged`, and `digstore stores`.
- **URN manifest:** `commit` writes a local `urns.json` / `urns.txt` — the
  publisher's index of every shareable URN for that deployment.

---

## How content is addressed: URNs

Every resource is named by a URN. The URN alone locates **and** decrypts it:

```
urn:dig:<chain>:<storeID>[:<rootHash>][/<resourceKey>]
```

| Part | Meaning |
|---|---|
| `<chain>` | Chain identifier, e.g. `chia` |
| `<storeID>` | Your 64-hex store id (required) |
| `<rootHash>` | Optional: pin a specific deployment root; omit for the current one |
| `<resourceKey>` | Optional: which resource (content-root-relative path) |

`digstore urn [PATHS]` previews the exact URN (and retrieval key) a file *will*
have against the active project — so you can check before you commit instead of
guessing.

---

## Public vs private projects

```sh
digstore init             # public:  anyone with the URN can read
digstore init --private   # private: URN locates, but reading also needs a secret salt
```

- **Public** — the URN is sufficient to decrypt.
- **Private** — decryption also requires a secret salt the publisher holds and
  shares out-of-band. Pass it with `--salt <hex>` on `cat`/`checkout`.

---

## Sharing over a remote

A remote is an HTTPS endpoint that hosts and serves your `.wasm` module.

```sh
# publisher
digstore remote add origin https://example.com/stores/<storeID>
digstore push origin

# consumer (fresh directory)
digstore clone https://example.com/stores/<storeID>
digstore cat   urn:dig:chia:<storeID>:<rootHash>/readme
digstore pull  origin          # later: fetch the publisher's newer deployment
```

`clone`/`pull` **verify** what they download before installing it: the module must
match the store id you asked for, and the served root must carry the publisher's
signature. A malicious or broken server cannot feed you fabricated content — the
command fails instead. Remotes must be `https://` (plain `http://` is allowed only
for `localhost`).

---

## Deploy from GitHub Actions (CI)

Auto-publish your built site/dapp to your existing store on every push — a new
capsule, git-push-to-deploy. The store must already exist (you ran
`digstore init` once); CI only **advances** it (it never mints).

One-time setup, on the machine that created the store:

```sh
digstore log --json          # copy the store_id
digstore deploy-key export   # copy the 64-hex publisher deploy key
```

Add two repository **secrets** — `DIG_MNEMONIC` (your funded deploy wallet) and
`DIG_DEPLOY_KEY` (the key above) — commit a `dig.toml` (see [`examples/dig.toml`](examples/dig.toml)),
then add the workflow ([`examples/github-actions-deploy.yml`](examples/github-actions-deploy.yml)):

```yaml
- name: Deploy to DIG
  uses: DIG-Network/digstore@v0.5.29   # pin to a release tag
  with:
    mnemonic: ${{ secrets.DIG_MNEMONIC }}
    deploy-key: ${{ secrets.DIG_DEPLOY_KEY }}
    output-dir: dist
```

> **⚠ Security:** v1 ships the funded wallet mnemonic into CI as a secret — it can
> spend ALL of that wallet's DIG/XCH. Use a **dedicated, low-balance deploy
> wallet** funded with only enough DIG for your expected deploys (each deploy costs
> 100 DIG + a small XCH fee). For the on-chain root advance you can instead use a
> **revocable writer deploy token** (see [Writer deploy tokens](#writer-deploy-tokens--advance-the-root-without-the-owner-seed)
> below) so the owner key never enters CI; the funded wallet is still needed to pay
> the DIG + XCH fee.

`digstore deploy` reconstructs the store locally from the deploy key + the
on-chain root, stages your `output-dir`, advances the root, and pushes the new
capsule — all non-interactively. See `digstore deploy --help`.

### Preview a build without spending (free)

`digstore deploy --preview` builds a **free preview capsule** — it runs the real
compile → verify → decrypt read path on your `output-dir`, writes a local `.dig`
artifact, and prints its content address (`storeId:rootHash` + `dig://` URN).
**No chain, no wallet, no deploy key, nothing spent** — the preview store id is a
fresh ephemeral id, so a preview never touches (or impersonates) your real store.
Use it to verify a build, or to serve a shareable preview from CI:

```sh
digstore deploy --preview                       # → <output-dir>/../.dig-preview/<root>.dig
digstore deploy --preview --preview-out p.dig   # explicit artifact path
```

### Writer deploy tokens — advance the root without the owner seed

The CI flow above ships the funded wallet into CI. To advance a store's root from
CI **without exposing the owner key**, use a **writer deploy token**: a revocable
delegate the owner pre-authorizes (the hub Teams "Deployer" flow / on-chain
`updateStoreOwnership`). A writer can change **only the metadata root** — it can
never change ownership or melt the store, and the owner revokes it at any time.

```sh
digstore commit -m "deploy" --deploy-key $DIGSTORE_WRITER_KEY   # writer-signed root advance
digstore deploy --writer-key $DIGSTORE_WRITER_KEY               # same, in the CI deploy flow
```

Prefer the `DIGSTORE_WRITER_KEY` env var so the key isn't visible in the process
table. The wallet seed still pays the 100 DIG + XCH fee; the writer key only
authorizes the on-chain root advance. (This is distinct from the §21 publisher
`--deploy-key`/`DIGSTORE_DEPLOY_KEY` above, which lets DIGHub accept the capsule.)

---

## On-chain anchoring (Chia mainnet)

Every project is **anchored on Chia mainnet**. `digstore init` mints an empty store
singleton on-chain, and the singleton's **launcher id becomes the store id**.
Every `digstore commit` then pushes the new deployment's root to that singleton
with an on-chain update and **blocks until the update confirms** before finalizing
the deployment locally.

> **This spends real XCH and DIG.** Anchoring is mandatory — there is no offline mode.
> `init` and `commit` will not proceed without an unlocked wallet seed and enough
> funds, and they block on mainnet confirmation. All broadcast and chain reads go
> through [coinset.org](https://coinset.org) over HTTPS (no peer node or TLS cert
> to run).
>
> **DIG token fees (v0.5.4):** every `init` pays **100 DIG** to the DIG treasury,
> and every `commit` pays **10 DIG** — embedded atomically in the same spend bundle
> as the mint/update (memo = store id). Before submitting, each command prints the
> cost and your current balance; if the wallet is short on XCH **or** DIG the
> command blocks and tells you what's missing. Use `digstore balance` to check your
> spendable XCH (mojos) and DIG at any time.

### 1. Set up a wallet seed

digstore keeps an encrypted BIP-39 seed in `~/.dig/seed.enc`.

```sh
digstore seed generate          # create a new mnemonic (shown once — back it up)
# or
digstore seed import            # import an existing mnemonic
digstore seed status            # is a seed present / unlocked?
digstore lock                   # clear the cached-unlock session
```

The seed is encrypted with a passphrase (Argon2id + AES-256-GCM). After unlock it
is cached for a configurable TTL; `DIGSTORE_PASSPHRASE` supplies the passphrase
non-interactively (for CI/scripts). Global settings live in `~/.dig/config.toml`
(`coinset_url`, `unlock_ttl`, `fee`).

### 2. Fund the wallet

Minting and updates cost both XCH (the transaction fee) and DIG (the DIG token, a
Chia CAT). The wallet derived from your seed needs **both**. Run `digstore balance`
to see your current spendable XCH (mojos), DIG (3-decimal display), and the wallet
receive address. If either is short, `init`/`commit` block, disclose the exact cost
up front, and print the **receive address** to fund:

```
insufficient funds: need <N> mojos, have <M>; fund xch1…
```

Both XCH and DIG are received at the same `xch1…` receive address (DIG as a CAT).
Send funds there, wait for them to confirm, then retry.

### 3. Init mints, commit anchors

```sh
digstore init                   # mints the store singleton; store id = launcher id
                                # blocks until the mint confirms on mainnet

digstore add readme.txt --key readme
digstore commit -m "first deployment"
                                # pushes the new root on-chain; blocks until
                                # confirmed, then finalizes the deployment locally
```

Both commands take `--wait-timeout <secs>` (default `300`) for how long to wait on
confirmation. On a confirm-timeout the project is kept **pending** (and the local
deployment is *not* finalized) — it is resumable, not lost.

### 4. Resume / inspect an anchor

```sh
digstore anchor                 # resume a pending anchor: confirm the chain coin
                                # and flip the project to confirmed (idempotent)
digstore anchor status          # read-only: show the project's anchor state
digstore anchor status --json   # machine-readable state
```

Per-project anchor state (network, store id / launcher, coin id, status, last root,
last tx id, confirmed height) is recorded in the project's `anchor.toml`.

The compiled `.dig` module also embeds the on-chain pointer (network, launcher/store id,
current coin id, confirmed height, and a coinset endpoint hint) directly in its data
section. `digstore anchor status` surfaces this alongside the local `anchor.toml` state
(use `--json` for machine-readable output); `digstore anchor inspect <module.dig>` dumps
the pointer from any module file without a local workspace. The embedded coinset URL is
a hint only — local config and flags always take precedence.

> Note: `clone`/`pull` verify the publisher's signature over the served head **and**
> verify that the served root equals the project's current on-chain singleton root —
> read from the chain via the launcher id embedded in the module. They **fail closed**
> on a mismatch or an unreachable chain, making the chain the authority for the current
> root. (A module with no embedded on-chain pointer falls back to the head-signature
> gate.) See [`SECURITY.md`](SECURITY.md).

---

## Command reference

| Command | What it does |
|---|---|
| `digstore init [name] [--dir <path>] [--private] [--wait-timeout <s>]` | Create a project (default name `default`); mints its singleton on mainnet (store id = launcher id); `--dir` sets its content root |
| `digstore stores` (alias: `projects`) | List projects with active marker, root, content root, capacity |
| `digstore use <name>` | Set the active project |
| `digstore dir [<path>]` | Show or set the active project's content root |
| `digstore add <path…> [-A] [--key <name>]` | Stage files (`-A` = the whole content root) |
| `digstore staged` / `digstore unstage` | List the staging area / clear it |
| `digstore commit [-m <msg>] [--wait-timeout <s>] [--deploy-key <writer-seed>]` | Seal a new deployment, anchor its root on mainnet (blocks until confirmed), compile the module, write the URN manifest. `--deploy-key` advances the root with a revocable **writer deploy token** instead of the owner seed |
| `digstore status` | Show staged/modified/untracked + capacity |
| `digstore log [--limit N]` / `digstore diff <a> <b>` | List / compare deployments |
| `digstore urn [PATHS…] [--root <hex>]` | Preview the URN(s) files will have |
| `digstore cat <urn> [--salt <hex>] [--verify-proof]` | Read a resource by URN |
| `digstore checkout <root> --out <dir> [--salt <hex>]` | Write a whole deployment to a directory |
| `digstore remote add\|list\|remove …` | Manage remotes |
| `digstore clone <url>` / `push [remote]` / `pull [remote]` | Sync with a remote (verified) |
| `digstore deploy [--store-id <hex>] [--output-dir <dir>] [--build-command <cmd>] [-m <msg>] [--writer-key <seed>]` | CI auto-deploy: advance an EXISTING store from a fresh checkout (reads `dig.toml`); never mints. `--writer-key` advances the root with a revocable writer deploy token (owner seed stays out of CI) |
| `digstore deploy --preview [--preview-out <file>]` | Build a **free** preview capsule via the real read path (writes a local `.dig` artifact + content address); no chain, no wallet, nothing spent |
| `digstore deploy-key export [--out <file>]` | Export the store's publisher deploy key (for a CI secret) |
| `digstore anchor [--wait-timeout <s>]` | Resume a pending on-chain anchor (confirm the coin, flip to confirmed) |
| `digstore anchor status [--json]` | Show the active project's anchor state + embedded module chain pointer (read-only) |
| `digstore anchor inspect <module.dig> [--json]` | Dump the on-chain pointer embedded in any module file (read-only, no workspace needed) |
| `digstore balance [--json]` | Show spendable XCH (mojos) and DIG (3-decimal) + the wallet receive address (read-only) |
| `digstore seed generate\|import\|status` / `digstore lock` | Manage the encrypted wallet seed used for anchoring |

Global flags: `--store <name>` (target a specific project), `-C/--cwd <path>`
(operating directory for this command), `--dig-dir <path>` (workspace location),
`--json` (machine-readable), `--quiet`, `--verbose`, `--color <auto\|always\|never>`.

### Wallet seed

`digstore seed generate|import|status` and `digstore lock` manage the encrypted
BIP-39 wallet seed used for on-chain anchoring — see
[On-chain anchoring](#on-chain-anchoring-chia-mainnet) above for details.

---

## What this gives you

- **Encrypted at rest.** Content is encrypted with a key derived from its URN.
  There is no key stored anywhere to recover — lose the URN, lose the read.
- **Provider-blind hosting.** Whoever hosts your project holds only ciphertext keyed
  by hashes; they can't scan it or read requests.
- **Verified downloads.** `clone`/`pull` reject content that isn't the genuine,
  publisher-signed project.
- **Uniform, self-contained.** A project is a single `.wasm`, padded to a uniform
  size so its bytes reveal nothing about how much content it holds. Copy it to
  back it up; run it to serve it.

---

## Security

Security posture, the hardening applied, and known residual risks are documented
in [`SECURITY.md`](SECURITY.md). Please report vulnerabilities privately to the
maintainer rather than opening a public issue.

## Contributing

Build, test, and contribution guidelines are in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Licensed under the [GNU General Public License v2.0](LICENSE) — the same license
as Git.
