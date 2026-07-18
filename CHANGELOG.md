# Changelog

All notable changes to this project are documented here.
This project adheres to [Semantic Versioning](https://semver.org) and
[Conventional Commits](https://www.conventionalcommits.org).

## [0.15.0] - 2026-07-18

### Features
- **nft:** Emit canonical URN alongside https url in NFT mint uris (#24)

## [0.14.0] - 2026-07-17

### Refactor
- **cli:** Rename binary digstore -> dig-store with transitional dual-publish (#23)

### CI
- **digstore-core:** Publish the library crate to crates.io (#22)- Add flaky-test management (#489) (#17)

## [0.13.3] - 2026-07-15

### CI
- **release:** Nightlies system (cron + dispatch, nightly channel) (#592) (#19)- **release:** Nightlies polish (#20)

## [0.13.1] - 2026-07-12

### Bug Fixes
- **digstore:** Correct Discord invite (imposter link -> official) (#18)

## [0.13.0] - 2026-07-12

### Features
- **cli:** Add first-class digs alias binary for digstore (#16)

## [0.12.0] - 2026-07-11

### Features
- **chain:** Cap-50 high-value-first coin selection + consolidate on init/commit/deploy (#15)

## [0.11.2] - 2026-07-11

### Documentation
- Add BACKERS.md (public sponsors) (#14)

## [0.11.1] - 2026-07-11

### Bug Fixes
- **cli:** Resolve raw binary asset for Windows self-update, not *-setup (#13)

## [0.11.0] - 2026-07-10

### Features
- **collection:** Cost-bounded auto-batching for large mints, resumable, terminal oversize error (#12)

## [0.10.1] - 2026-07-10

### Bug Fixes
- **cli:** Cat resolves a full urn:dig:... via the node ladder without a local store (#11)

## [0.10.0] - 2026-07-10

### Features
- **collection:** Multi-item DID-attributed mint + bech32 DID decode (#10)

## [0.9.1] - 2026-07-07

### Bug Fixes
- **nft:** Collection attributes use CHIP-0007 `type`, not `trait_type` (#9)

## [0.9.0] - 2026-07-06

### Features
- **update:** Real self-updater on macOS/Linux + macOS install docs + README quickstart (#6)

## [0.8.0] - 2026-07-06

### Features
- **pricing:** Consume the canonical hub /v1/pricing for dynamic per-capsule $DIG (#5)

## [0.7.2] - 2026-07-06

### Bug Fixes
- **remote:** Clear error when a remote returns non-JSON/CloudFront HTML (#4)

## [0.7.1] - 2026-07-06

### Bug Fixes
- **chain:** Retry transient coinset failures + resume pending mint (#84) (#3)

## [0.7.0] - 2026-07-04

### #209
- Remove the dig-node/dig-runtime/dig-wallet crates — digstore is store libs + CLI only

### Features
- **cli:** Authorize-origin-as-writer via well-known pubkey discovery (#2)

### Security
- Allow the DIG-Network git sources + accept two transitive unmaintained advisories- Comprehensive normative SPEC.md — full function set + onion participation (#197)- Bump dig-download to the #179-hardened verifier + adopt with_proof_verifier

### CI
- Add commitlint + version-increment gate + git-cliff changelog config (#230 pipeline lockdown)- Changelog + tag on merge feeding the existing tag-driven binary release (#230)

### CI
- Strip debuginfo + reclaim more disk (wasmtime/cranelift ENOSPC)

### Deny.toml
- Ignore RUSTSEC-2026-0190 (anyhow downcast_mut unsoundness; path unreachable)- Allow the dig-pex git source (#166 supply-chain audit)- Allow the dig-peer-selector git source (#178)

### Dig-node
- Fix flaky Windows config lost-update (race-free probe + in-process RMW lock)- Enforce mandatory + uniform anchored-root pinning on reads (#127)- Pin + test the honest read-path proof contract (#126)- Maintain a persistent relay connection (NAT reachability)- Integrate dig-nat + dig-gossip — the L7 peer network (#162 PHASE-2b)- Integrate the dig-dht content-location DHT (#163 PHASE-B)- Node<->node PEX peer-sharing over the mTLS peer streams (#166)- Add the dig-download dependency (#164/#165)- Dig-download content-fetch path + redirect-on-miss (#164/#165)- IPv6-first peer listener/advertise + dig-nat candidate-list dial (#180)- Rustfmt the network_info IPv6-first test assert- Integrate dig-peer-selector into the P2P content-fetch path (#178)- Gate peer JSON-RPC surface behind a method allowlist (#179 CRITICAL auth-bypass)- Bound the dig.stage directory walk against memory exhaustion (#179 HIGH)- Cap launcher_ids on peer-reachable collection reads (#179 HIGH)- Bound peer connection + per-connection stream concurrency (#179 HIGH)- Snapshot inventory once + cap items in availability_batch (#179 optimization)- Memoize decoded content + move decode off the async runtime (#179 optimization)- Rustfmt + clippy cleanup for the #179 audit fixes- Fix CI — rustfmt dht.rs comments + ignore new quick-xml RUSTSEC advisories- Chain-watch + subscriptions + generation gap-fill (#198)- Announce a freshly cache.fetchAndCache'd capsule to the DHT (#198, §6.2)- Address review — decouple chain-watch from the DHT + fix control-echo + docs (#198)- Decouple chain-watch from DHT bring-up + normalize subscription echo (#198 follow-up)- Background capsule backfill on a read from another node (#198, SPEC §5.6)

### Digstore-chain/compiler
- Enforce per-capsule $DIG payment + pin canonical capsule size (#130)

## [0.6.1] - 2026-06-29

### Features
- **chain,cli:** Configurable per-capsule DIG amount (dynamic, USD-pegged)- **cli:** Agent-friendly completion — JSON errors, full --help-json, exit-code table- **dig-node:** Add additive dig.stage RPC for in-process capsule staging (#95 Pass C)

### Bug Fixes
- **cli:** Rename commit --deploy-key to --writer-key (footgun)- **cli,docs:** Stop leaking internal tracker numbers; lead README with free new/dev

### Refactor
- **cli:** Purge user-facing "project" → store/capsule; cross-link create-dig-app- **stage:** Extract stage→compile engine into digstore-stage; CLI delegates (#95 Pass C)

### CI
- **release:** Attach apt .tar.gz CLI assets + cross-compile mac-x64 on macos-14

### Dig-node
- Make the shared .dig cache safe for two processes (#95/#96 Pass A)

## [0.6.0] - 2026-06-28

### Documentation
- README — writer deploy tokens + free `deploy --preview` (#17/#18)

### Chain
- Wave-C primitives — writer deploy token, DID-attributed mint, collection reads, drop model (#17/#38/#39/#40)

### Cli
- Free pre-publish DX — `new`, `dev`, `doctor`, `commit --dry-run` (Wave-1 #5/#6/#13/#14)- Deploy --if-changed/--dry-run, dig.toml manifest, setup/link/completion (Wave-1/2 #8/#19/#20/#21/#27)- Deploy --dry-run leaves the source tree untouched; dev survives a poisoned lock- Fix flaky dev test — bind the port first, announce the real one (CI green)- Keep the dev test's stdout drained so the child never blocks/EPIPEs- Make the dev test read by Content-Length, not EOF (CI determinism)- Wave-B asset CLI — nft/collection/did/offer + capsule-media + CHIP-0007 (#35/#33/#36)- Harden flaky cli_dev asset fetch — 30s http retry deadline (CI determinism)- Wave-C commands — deploy token, free preview, DID mint, collection show/list (#17/#18/#38/#39/#40)

## [0.5.29] - 2026-06-27

### Features
- **cli:** Align user-facing terminology with hub.dig.net (project/deployment)- Write+read project name/description in the CHIP-0035 singleton metadata- **cli:** Offer to publish a confirmed deployment to DIGHub; indeterminate transfer bars- **dig-resolver:** Resolve a store's current singleton-tip root via coinset.org- **dig-wallet:** Built-in Chia wallet sidecar (v0 — keys, balance, receive)- **dig-wallet:** Encrypted-at-rest seed + password lock/unlock

### Bug Fixes
- **deps:** Move off yanked bitcoin_hashes 0.14.100

### Security
- Per-origin dapp consent gate + Connections UI

### Refactor
- Adopt digstore_core::CHAIN on the producer side; drop dead Ui.verbose

### Documentation
- **readme:** Align user-facing terminology with hub (project/deployment)

### Testing
- Add scripts/local-push-test.sh — exercise push/pull/clone locally

### CI
- Free ~25 GB on ubuntu runner before the workspace build

### Chain
- FOUNDATION coin-query + mempool-submit layer for wallet Sage-parity- **cat:** Generic CAT support over any TAIL (reconstruct/balance/send)- **offer:** Make-offer builder + decode-summary + cancel (Sage parity)- NFT module — list/mint/bulk-mint/transfer (Sage parity)- DID module — create/list/transfer/NFT-attribute (Sage parity)- Store discovery — enum/discover user stores + capsule history- Clawback payments — timelocked claw-back-able send (claim + recover)- Transaction history — wallet-side coin add/remove aggregation- Option contracts — create / exercise / clawback (Sage parity)- Streamed (vesting) payments — create / claim-vested / clawback (Sage parity)- Vault (MIPS multi-key wallet) — create + K1 member spend (Sage parity)- Verifiable credentials — issue / verify / revoke (Sage parity)

### Chip0002
- Native CHIP-0002 signer + dig-wallet WalletConnect dispatcher

### Cli
- Surface the capsule identity (storeId:rootHash) in commit + help (#33)

### Core
- Add canonical Capsule type (storeId:rootHash) as ecosystem identity

### Deploy
- CI auto-deploy to an existing store (#1 roadmap)

### Dig-node
- Move test module to EOF (clippy items_after_test_module)- Rustfmt (CI format check)- Native Chia §21.9 identity signer + authenticated whole-store sync- Dig.getAnchoredRoot — chain-anchored trusted root for dig:// pinning- Log §21 whole-store sync outcome (success root / failure status)- Tag dig.getContent results local/remote (for the browser "local" chip)- Cache.getConfig/setCapBytes/clear RPC methods- Regression test for cache.* RPC (getConfig/setCapBytes/clear)- Cached-store management RPCs + public fns (capsule list/remove/fetch)

### Dig-runtime
- Run the DIG node NATIVE in-process (no sidecar)- Direct FFI (dig_rpc) — the browser process IS the node, no server- Host the built-in Chia wallet in-process (no sidecar)

### Dig-wallet
- Native XCH send/sign flow (gated broadcast)- DIG protocol settings page + native cache-threshold API- Add chia_signMessageByAddress + chip0002_getAssetBalance (#24)- Add chip0002_getAssetCoins (XCH + DIG CAT) — the hub spend path (#24)- Implement chia_takeOffer (native badge-mint offer accept)- WalletConnect relay responder + DIG-settings wallet features (#24)- Generalize tokens to any CAT + chia_send (XCH + CAT)- Offers — make / summary / cancel (Sage parity)- NFTs — list / transfer / mint / bulk-mint (Sage parity)- DIDs — list / create / transfer (Sage parity)- Transactions history + My Stores (capsules)- Advanced wallet UI (tokens/NFTs/offers/DIDs/activity/stores)- Lock new deps (chia, chia-wallet-sdk, digstore-core)- Cached-store manager (#32) — capsule cache list/remove/fetch- Advanced coin-type endpoints + Advanced UI (clawback/options/streaming/vault/VC)- "DIG Wallet" luxury redesign of the embedded wallet UI- Delegate the embedded wallet to Sage over WalletConnect (#34)

### Dig-wallet/dig-runtime
- Native FFI entrypoint for the in-process wallet

### Fmt
- Rustfmt dig-wallet + chip0002 (fix red CI Format check)

### Offer
- Pass change keys explicitly to take_offer (fix change-at-funded-index assumption)

## [0.5.28] - 2026-06-17

### Bug Fixes
- **ci:** Clear clippy -D warnings + cargo-deny advisory (green CI)- Login gate only for dighub remotes + first-push to self-hosted nodes

### Testing
- **cli:** Adv_delegated_host_key now asserts any-node serve

## [0.5.27] - 2026-06-17

### Bug Fixes
- **guest:** Serve content to ANY node — drop host-key attestation gate

## [0.5.26] - 2026-06-17

### Reverts
- **cli:** Drop push-time re-key (it changed the anchored program_hash)

## [0.5.25] - 2026-06-17

### Bug Fixes
- **cli:** Re-key the module to the serving node's host key on push

## [0.5.24] - 2026-06-17

### Bug Fixes
- **cli:** Record real session token expiry; detect dead tokens

## [0.5.23] - 2026-06-17

### Features
- **cli:** Claim the store for the logged-in account after push

## [0.5.22] - 2026-06-17

### Bug Fixes
- **cli:** Origin remote defaults to https://rpc.dig.net when unconfigured

## [0.5.21] - 2026-06-17

### Bug Fixes
- **cli:** Device pairing wire field is device_token, not device_code

## [0.5.20] - 2026-06-17

### Features
- **cli:** Pull <urn> with no generation defaults to the on-chain singleton tip

## [0.5.19] - 2026-06-17

### Features
- **cli:** Digstore pull <urn> — network read by retrieval key, verify merkle proof, auto-decrypt

## [0.5.18] - 2026-06-17

### Features
- **cli:** Origin defaults to bare https://rpc.dig.net (drop username)

## [0.5.17] - 2026-06-17

### Features
- **cli:** Digstore login/whoami/logout (device pairing) + login gate + origin auto-fill

## [0.5.16] - 2026-06-17

### Features
- **cli:** Origin = https://<username>@rpc.dig.net; store id from local store

## [0.5.15] - 2026-06-17

### Features
- **cli:** Estimate XCH fee via coinset get_fee_estimate (fee=0 → estimated; config.fee overrides; fail-open)

## [0.5.14] - 2026-06-17

### Features
- **cli:** Upload/download progress bars + confirm spinners

## [0.5.13] - 2026-06-17

### Bug Fixes
- **push:** Send empty parent_root on first push (genesis is not a real parent) — fixes spurious 409 + bump v0.5.13

## [0.5.12] - 2026-06-17

### Bug Fixes
- **push:** First push to a fresh store treats descriptor 404 as genesis parent (was aborting) + bump v0.5.12

## [0.5.11] - 2026-06-17

### Bug Fixes
- **remote:** Set User-Agent on the dig client — rpc.dig.net WAF 403s no-UA requests, silently breaking all §21 fetch/clone/pull/push

## [0.5.10] - 2026-06-16

### Bug Fixes
- **cli:** Surface server error message on push failures + bump v0.5.10

## [0.5.9] - 2026-06-16

### Features
- **cli:** Build/scan/confirm spinners + skip empty-passphrase prompt

### Bug Fixes
- **cli:** Commit/update cost is 100 DIG (was 10) + bump v0.5.9

## [0.5.8] - 2026-06-16

### Features
- Client-side pre-encryption — compile --pre-encrypted + encryptResource- **remote:** Client speaks dig RPC push protocol v1 (inline + presigned negotiation)- **remote:** Server speaks dig RPC push protocol v1 (inline negotiation, §21.4 preserved)- **cli:** Interactive + non-interactive modes across all commands- **remote:** Push sends the publisher pubkey so a remote can auto-create the store on first push- **wallet:** Derive_indexed_keys (unhardened HD range; index 0 matches legacy)- **wallet:** Adaptive HD scan + aggregate XCH/DIG balance- **wallet:** Aggregate anchor balance over scanned wallet + TibetSwap link on shortfall- **wallet:** Mint spends DIG+XCH across HD addresses, signs with all keys- **wallet:** Init shows aggregate HD balance + read-only live HD-scan verification test

### Refactor
- Conciseness cleanups (dead code / dup removal, no behavior change)- **core:** Consolidate read-crypto + resource leaf into digstore-core (SP1)- **core:** Centralize CHAIN + DEFAULT_RESOURCE_KEY in digstore-core (SP2)

### Documentation
- **wallet:** Design — HD-wallet support (fix single-address balance/spend in digstore init)- **wallet:** HD-wallet support implementation plan

### CI
- Publish the digstore compile binary to the artifacts bucket- Build guest wasm before digstore-cli (build.rs embeds it, BINDING D6)

### Styling
- Rustfmt import order for the sha256 re-export

### Compile
- Add --host-key to embed a delegated serving node's trusted key

### Serve
- Carry per-chunk lengths so multi-chunk resources decrypt in-browser

## [0.5.7] - 2026-06-13

### Features
- Dig-client-wasm — browser read-crypto for dighub content viewing- **remote:** Per-request CLI identity auth (§21.9) + dig:// user@host- **remote:** §21.9 auth-enforcing server + `digstore serve` runnable node

### Bug Fixes
- **cli:** Commit() 3-arg arity in all callers + dig:// remote scheme- **clippy:** Type alias for the request signer + doc list spacing

### Styling
- Cargo fmt (compile --metadata threading)

### Chores
- Declare GPL-2.0-only license on all crates (cargo-deny in downstream consumers)

### Compile
- --metadata embeds the store manifest in the .dig

## [0.5.6] - 2026-06-12

### Features
- **cli:** Chainless `digstore compile` + publish binary to S3 on release

### Styling
- Cargo fmt (compile command + store_ops)

### Chores
- Release v0.5.6 (chainless compile + S3 binary publish)

## [0.5.5] - 2026-06-12

### Bug Fixes
- DIG-feature followups — coin_spend not-found is pending; tx_id from bundle name; format_xch; commit --resubmit; dig_cats+coin_spend tests

### Documentation
- DIG cost (init 100, commit 10) + digstore balance in README

## [0.5.4] - 2026-06-12

### Features
- **chain:** Emit digstore-scoped owner discovery hint at mint (chip35 parity)- **chain:** DIG constants + dig_cat_puzzle_hash + dig_balance- **cli:** Digstore balance (XCH + DIG) + InsufficientFunds asset variant- **chain:** DIG CAT payment embedded in mint/update bundle- **cli:** Init/commit DIG+XCH preflight + up-front DIG cost disclosure

### Documentation
- **spec:** DIG CAT payment in mint/update bundle + balance gating- **plan:** DIG CAT payment + balance gating implementation plan- **chain:** Note single-bundle co-signing is the sole DIG/anchor atomicity guarantee

## [0.5.3] - 2026-06-12

### Bug Fixes
- **cli:** Clear error on commit-before-init-confirmed; clean 'not a digstore module' for anchor inspect

## [0.5.2] - 2026-06-12

### Bug Fixes
- **cli:** Refuse no-op commit (unchanged content) before anchoring; avoids duplicate-root on-chain re-anchor

## [0.5.1] - 2026-06-12

### Bug Fixes
- **cli:** Clone validates URL before creating local store (no stray .dig on rejected clone)- **chain:** Coinset not-found coin record is pending, not an error (confirm polling)

## [0.5.0] - 2026-06-12

### Features
- **chain:** Scaffold digstore-chain crate- **chain:** Global config + ~/.dig path resolution- **chain:** BIP-39 mnemonic validate + generate- **chain:** Argon2id+AES-256-GCM seed encryption- **chain:** Persist encrypted seed owner-only- **chain:** Cached-unlock session with TTL- **cli:** Seed error variants + ChainError mapping- **cli:** Hidden passphrase prompt- **cli:** Define seed + lock commands- **cli:** Seed import/generate/status + lock commands- **cli:** Clearer prerequisite guidance (error hints + self-guiding messages)- **cli:** Guide 'no store selected' in dir command- **chain:** Phase-0 anchoring prototype — coinset-only mint PROVEN on mainnet- **chain:** Coinset access via ChainReads trait + mock- **chain:** Wallet key derivation from mnemonic- **chain:** Build+sign store mint spend- **chain:** Sync datastore singleton from launcher id over coinset- **chain:** Build+sign store root update spend- **chain:** ChainAnchor trait + CoinsetAnchor (mint/update/confirm)- **cli:** Anchoring error variants + async runtime bridge- **cli:** Anchoring foundation — anchor.toml state, seed-unlock, mock backend, confirm UX- **cli:** Init mints store singleton; store_id := launcher id (relax §20.1 self-cert)- **cli:** Commit anchors new root on mainnet (blocks until confirmed)- **cli:** Digstore anchor / anchor status commands- **core:** ChainState data section + read_chain_state- **compiler:** Emit + preserve ChainState data section- **cli:** Embed ChainState in the module at commit finalize- **cli:** Anchor status reads module chain pointer; anchor inspect <module>- **chain:** Current_root — read a launcher's live on-chain root- **cli:** Clone/pull verify served root against the on-chain singleton

### Bug Fixes
- **chain:** Zeroize derived AES key; test tampered ciphertext- **cli:** Reject non-utf8 passphrase, zeroize entered mnemonic, validate --words, status without config- **chain:** Atomic secret-file write; resolve passphrase before showing mnemonic; drop unused hex dep- **chain:** Surface error on missing coin_records; clarify mock- **chain:** Cap singleton sync depth; drop needless key clone in mint- **cli:** Check store-exists before minting in init (avoid orphaning a mint)- **cli:** Anchor resume hints to finalize commit when local head is behind- **cli:** Warn when chain-root check is mocked; rename misleading current_root test

### Refactor
- **chain:** Extract shared write_secret_file to fs_util

### Documentation
- **spec:** Onchain anchoring + seed management design- **spec:** Mainnet-only anchoring, broadcast via coinset.org- **plan:** Seed management implementation plan (subsystem 1/2)- Document seed commands- **spec:** Record anchoring verification spike — coinset-only needs datalayer-driver low-level build (dig-store-coin is P2P-bound)- **plan:** Onchain anchoring implementation plan (subsystem 2/2)- **plan:** Fold in chia-sdk-coinset CoinsetClient discovery- Handoff prompt for onchain anchoring Phase 5 (CLI integration)- **cli:** README anchoring section + plan notes + gated mainnet e2e; Phase 5 verification- **spec:** Chainstate-in-wasm design (embed chain pointer + chain-verified clone/pull)- **plan:** Chainstate-in-wasm Phase A implementation plan- README + spec note for embedded module chain pointer (Phase A)- **plan:** Chainstate-in-wasm Phase B (chain-verified clone/pull)

### Testing
- **chain:** Make DIGSTORE_HOME override test panic-safe- **cli:** Cover new seed exit codes in distinctness test- **cli:** Seed/lock command integration tests- Chain-verified clone/pull closes SECURITY residual #6

### Styling
- Cargo fmt the branch + remove throwaway anchor prototype

### Chores
- **chain:** Add datalayer-driver + chia-protocol + reqwest deps for anchoring

## [0.4.5] - 2026-06-11

### Bug Fixes
- **update:** Match release installer asset name; bump 0.4.5

## [0.4.4] - 2026-06-11

### Bug Fixes
- **installer:** Point 'Open Documentation' at docs.dig.net/digstore

## [0.4.3] - 2026-06-10

### Bug Fixes
- **installer:** Stop install freezing at 2% + never persist wizard step

### CI
- **release:** Pass --repo in publish job (no checkout = no git context)

## [0.4.2] - 2026-06-10

### Bug Fixes
- **installer:** Stop UAC auto-elevation + never hang on install error

### CI
- **release:** Publish in a single job after the build matrix (fix race)

## [0.4.1] - 2026-06-10

### Features
- **cli:** Describe every command + interactive init setup- **installer:** Single self-contained run-once installer per OS + .dig icon- **cli:** Stream by urn (decrypt) or retrieval key (encrypted) + keys command

### Bug Fixes
- **installer:** Resolve .win class collision hiding Welcome content

### Chores
- Release v0.4.1 — installer single-exe, .dig icon, cat/keys streaming

## [0.4.0] - 2026-06-10

### Features
- **guest:** Enforce JWT signature verification in content gate (residual #4/#5)- **cli:** Add `digstore update` command + throttled release beacon- **security:** Merkle leaf/node + per-role BLS domain separation (residual #2)- Signed root-revocation tombstones (residual #1 Layer 1)- **host:** Injectable proof backend/chain/clock in blind serve (residual #3)

### Documentation
- **spec:** Key rotation & root revocation design (residual #1) — layered tombstone/succession/recovery-key model

### Chores
- Release v0.4.0 — security residuals + release beacon

## [0.3.0] - 2026-06-10

### Features
- **host:** Raise module memory ceiling 256→2048 pages (128 MiB)- **compiler:** Raise memory ceiling to 2048 pages; unify CEILING_PAGES into template::MAX_MEMORY_PAGES- **guest:** Dynamic heap base above the data section; remove fixed 16 MiB cap (D2 for any blob size)- **cli:** Workspace.toml registry with store selection + legacy migration- **cli:** --store/-C global flags, new subcommand variants, workspace-aware dispatch + migration- **cli:** Init [name] [--dir] registers store in workspace.toml; 100 MB default cap- **cli:** `digstore stores` lists stores with active marker, root, content root, capacity- **cli:** `digstore use <name>` sets the active store- **cli:** `digstore dir [path]` shows/sets the per-store content root- Uniform-size module filler + 384 MiB ceiling; MAX_STORE_BYTES=128MB in digstore-core- **cli:** Walk scopes to operating dir, skips workspace dir, rejects escapes- **cli:** Enforce + surface the 128 MB cap in add/status/commit- **cli:** Ui::capacity (used/free/limit numbers, bar on color TTY)- **cli:** Status shows the capacity header- **cli:** Add prints capacity + emits staged/limit bytes in JSON- **cli:** `digstore unstage` clears the staging area- **cli:** `digstore staged` lists entries + sizes + capacity- **cli:** `digstore urn [PATHS]` previews content-root-relative URNs- **installer:** Branded Tauri 2 desktop installer + tauri-installer release job- **installer:** Badge bundled digstore version + premium motion layer

### Bug Fixes
- **cli:** Complete multi-store dispatch + green the full workspace test+lint gate- **cli:** Cap arithmetic must not double-count a re-staged modified key- **cli:** Reject out-of-build-dir paths; keys always build-relative- **installer:** User-PATH via HKCU registry (no machine-PATH duplication / setx truncation); call the integrity gate a checksum, not a signature- **installer:** License copy -> GPL-2.0 (same as Git)

### Performance
- **guest:** Single-copy serve — a full ~122 MB resource serves within the 384 MiB ceiling

### Refactor
- **cli:** CliContext carries workspace_dir, op_dir, store_name (dig_dir stays = store dir)- **cli:** Use digstore_core::MAX_STORE_BYTES as the single cap source

### Documentation
- **spec:** Multi-store workspaces, content roots, 100MB cap, staging mgmt- **plan:** Multi-store + limits implementation plan (24 tasks, A–F)- **plan:** Finalize memory model — uniform-size filler, 128MB cap, 384MiB ceiling- **plan:** Add Task A6 — single-copy serve (122 MB resource within 384 MiB)- **whitepaper:** Multi-store workspaces, content roots, uniform module size, 384 MiB ceiling- **security:** Note per-store 128 MB cap + configurable 384 MiB memory bound + uniform module size- Professional repo — branded README + badges, GPL-2.0 LICENSE, CONTRIBUTING, issue/PR templates

### Testing
- **compiler:** Serve a >8 MiB data section + near-cap ceiling-headroom stress test- **cli:** Multi-store integration coverage + cap/urn/cleanup fixups (Phase F)- **cli:** Align harness op_dir with the build dir

### CI
- Pin toolchain to 1.94.1 for deterministic clippy

### Chores
- Cargo fmt across multi-store work- **installer:** Sync design-package token annotations + build brief- Release v0.3.0

## [0.2.0] - 2026-06-09

### Features
- Use the .dig extension for compiled store modules- **cli:** Init ignores the .dig/ store dir in .gitignore- **cli:** Ui theme (styles, markers, verb formatting)- **cli:** Ui value with color/TTY/json/quiet resolution- **cli:** --color/--quiet flags; build and thread Ui through dispatch- **cli:** Directory/glob resolution honoring .digignore/.gitignore- **cli:** Git-parity add (-A/./globs/multiple/--dry-run, store-root keys)- **cli:** Directory-aware status (staged/modified/untracked)- **cli:** Cargo-style errors with actionable hints- **cli:** Per-command help with EXAMPLES- **cli:** Route all command output through the Ui layer

### Documentation
- **spec:** World-class CLI UX design- **plan:** CLI UX Phase 1 implementation plan

### Build
- **cli:** Add anstream, anstyle, ignore, globset

### CI
- Fix CI suite (toolchain components, deny config)

### Chores
- Release v0.2.0

## [0.1.0] - 2026-06-09

### Features
- **core:** Add ErrorCode enum and CoreError- **core:** Add ABI pack/unpack/is_error helpers- **core:** Add Encode/Decode codec traits and cursors- **core:** Add big-endian codec primitive impls- **core:** Add Bytes32/48/96 newtypes, hex, codec, sha256- **core:** Add URN parse/canonical/retrieval_key (paper 6.1,6.5)- **core:** Add DIGS data-section header + offset table- **core:** Add Merkle tree build + inclusion proof verify (paper 7.1-7.3)- **core:** Add KeyTableEntry and PathWalk (paper 8.4)- **core:** Add MetadataManifest and Author (paper 5.2)- **core:** Add ChiaBlockRef/ExecutionProof/ProofResponse wire structs (paper 9.1-9.2)- **core:** Add ContentResponse/attestation/AuthenticationInfo wire structs (paper 9.3,9.5)- **core:** Add store/generation/compiler config types (paper 5.2)- **core:** Add serving::concat_output helper (CONVENTIONS C9)- **chunker:** Generate frozen 256-entry gear table + rolling-hash helper- **chunker:** Derive boundary mask from target size (log2) + default config- **chunker:** Add Chunk type with SHA-256 content addressing- **chunker:** Add gear-hash boundary detector with min/mask/max bounds- **chunker:** Add slice chunking API (chunk_slice + Chunker)- **chunker:** True incremental chunk_stream, proven equal to chunk_slice- **crypto:** Crate root, error types, and sha256 Bytes32 wrapper- **crypto:** Derive_decryption_key via HKDF-SHA256 from canonical URN- **crypto:** Freeze HKDF KAT fixtures via gen_fixtures example- **crypto:** AES-256-GCM encrypt_chunk/decrypt_chunk with fixed nonce- **crypto:** Public bls module (SecretKey/PublicKey/Signature) per CONVENTIONS C1- **crypto:** Emit cross-impl BLS parity fixtures consumed by digstore-guest- **crypto:** Sign_push/verify_push over canonical SHA-256(root||store_id) (C7,§21.6)- **crypto:** Sign_node binds program_hash/output/anchor/input (§13.7,§16)- **crypto:** Sign_attestation over nonce||store_id||timestamp_be (§12)- **crypto:** Decrypt_and_unwrap returns unified CryptoError (AEAD+BLS)- **store:** Scaffold crate with StoreError enum and Result alias- **store:** Add Clock trait with SystemClock and FixedClock- **store:** Add StorePaths builder for the on-disk layout (4.4)- **store:** Config.toml round-trip for StoreConfig and Visibility (4.1)- **store:** Big-endian framed staging area with last-write-wins read-back- **store:** Per-directory content-addressed write-once chunk store- **store:** Generation manifest with KeyTableEntry projection (8.2)- **store:** Append-only monotonic root history (roots.log, 4.3)- **store:** GenerationDiff over chunk sets and resource keys (20.4)- **store:** Store::init/open creating the 4.4 on-disk layout (20.1)- **store:** Store::add and stage_file (path becomes resource key, 20.2)- **store:** Store::commit - chunk, merkle root, generation, history (20.3)- **store:** Global cross-generation chunk dedup (8.2)- **store:** Store::resolve_chunk for global content-addressed chunk resolution (8.2)- **store:** Store::log and Store::diff over generations (20.4)- **store:** Current_root, roothash_history, and module_path accessors- **guest:** Crate skeleton, no_std bump allocator, native-test feature- **guest:** DigHost trait + deterministic MockHost test double- **guest:** Ptr/len packing parity + big-endian request codec round-trip- **guest:** DataSection DIGS view + key-table encode/lookup- **guest:** Deterministic decoys, log-size distribution + real-shape ContentResponse (14.2)- **guest:** Oblivious padded-count bucketing + per-call shuffle/cover plan (14.3,14.4)- **guest:** Pure-Rust bls12_381 AugScheme verify + attestation gate, C8 parity (12.1)- **guest:** Session-gated jwks_fetch + JWT decode/claims + RS256/ES256 verify (6.3,12.4)- **guest:** Temporal validity-window check (16)- **guest:** Emit_merkle_proof verifiable under core rules (9)- **guest:** Serve_content gate chain + oblivious gather (C9 concat_output) + obfuscation seam (7,8,14,16,17)- **guest:** Serve_proof returns ProofPrelude (C3), nonce-bound serving_digest, decoy on miss (13)- **guest:** Metadata exports logic, get_metadata ungated (6.1,6.2)- **guest:** Wasm ABI exports + dig_host imports + data_stub; get_proof emits ProofPrelude (C3) (5.1,6.1,6.2,6.3)- **prover:** Scaffold digstore-prover crate, final manifest, error type- **prover:** ServingInputs with roothash-bound public_output (§13.4, deviation #3)- **prover:** Strict public_input codec + node signing message helpers- **prover:** Prover/Verifier/ChainSource trait definitions- **prover:** Deterministic MockChainSource with freshness window (§13.8)- **prover:** MockProver/MockVerifier commitment-chain round-trip (§13.1-13.4, 13.7-13.8)- **prover:** CoinsetChainSource with tx-block walk-down + fixture parse (§13.8)- **prover:** Hardware-attest alternative behind same trait (§13.6)- **prover:** Risc0 host backend + prove/verify smoke (§13.1-13.4, deviation #3)- **host:** Add injectable Clock with FixedClock and SystemClock- **host:** Add ExecutionLimits with 16MiB ceiling and fuel defaults- **host:** Add HostError with guest ErrorCode mapping- **host:** Add capped CSPRNG wrapper for host_random_bytes- **host:** Add session table with expiry for jwks gating- **host:** Add swappable AttestationBackend (13.6 TEE hook)- **host:** Add HostState and growable shared return buffer- **host:** Add guest linear-memory read/write helpers- **host:** Instantiate validated module and call data exports- **host:** Wire host_get_current_time and host_random_bytes imports- **host:** Implement host_read_return_buffer memcpy into guest memory- **host:** Implement host_get_public_key import- **host:** Implement attestation signing and session establish/verify imports- **host:** Session-gate jwks_fetch (NoSession before session)- **host:** Enforce wall-clock timeout via epoch interruption- **host:** Map fuel exhaustion trap to OutOfFuel- **host:** Enforce outer memory ceiling via StoreLimits- **host:** Jwks_fetch HTTP success path via blocking reqwest (mock-tested)- **host:** Implement serve_content/serve_proof flow per 18.4- **host:** Expose remaining data-export wrappers- **compiler:** Scaffold crate with CompilerError, CompilerConfig, CompilerStats- **compiler:** ChunkIndex global dedup with sequential indices- **compiler:** GenerationView/ResourceView traits and KeyTable build with dedup, ordered entries, lookup, integrity check- **compiler:** Deterministic ChaCha20 pool filler keyed by store_id+roothash- **compiler:** Interleaved pool with power-of-two bucketing, filler gaps, ChunkLoc core codec- **compiler:** Data-section codec via core Encode with offset table and store-header decode- **compiler:** Pinned guest template fixture, build.rs assembly, ABI/memory validation (5.1)- **compiler:** Inject data section via whole-section passthrough with memory bump (5.1)- **compiler:** Atomic temp-then-rename write with exact output filename (19.4)- **compiler:** Deterministic behavior-preserving obfuscation pass (17.1)- **compiler:** Pipeline orchestration with CompileOutcome (core stats + CompilerStats detail, C6), NoTrustedKeys, bucketed pool- **remote:** Scaffold digstore-remote crate with §21.8 error/status mapping- **remote:** ETag = root, If-None-Match parsing/matching (§21.7)- **remote:** JSON wire DTOs for descriptor/roots/content/proof/delta (§21.2)- **remote:** RemoteBackend trait with served/pending head + delta (§21.4/§21.5)- **remote:** InMemoryBackend reference (decoy-safe content, pending head, delta)- **remote:** BLS push auth delegating to digstore_crypto verify_push (§21.6, C7)- **remote:** Per-store token-bucket rate limiter (§21.8 429)- **remote:** Axum server skeleton + descriptor/roots handlers (§21.2/§21.3)- **remote:** HEAD/GET module with application/wasm + ETag/304 (§21.2/§21.7)- **remote:** PUT module push — BLS auth (C7), fast-forward, pending head, 413/422 (§21.4/§21.6/§21.8)- **remote:** POST content (200 decoy never 404) + POST proof (§21.2/§14.2)- **remote:** GET/POST delta — new chunks + key-table changes (§21.5)- **remote:** Per-store rate-limit middleware (§21.8 429)- **remote:** DigClient fetch/clone/pull/push over reqwest (§21.3-§21.6)- **remote:** StoreBackend adapter over digstore-store (§18/§21)- **cli:** Full CLI (init/add/commit/status/log/diff/checkout/cat/remote/clone/push/pull)- **core:** Add canonical datasection module per BINDING contract D1/D3/D4/D5- **guest:** Make served merkle proof genuinely verify (D5)- **compiler:** Real compiled module serves itself with a verifying proof (D6)- **store:** Per-resource ciphertext merkle leaves match compiler CurrentRoot (D5)- **cli:** Route cat/checkout through the self-serving module's serve_content (D6)- **cli:** Resolve key-less URN to index.html default view (D-DEFAULT-RESOURCE, section 8.5)- **cli:** §8.5 /.well-known/dig/manifest.json discovery convention- **prover,cli:** Bind execution-proof node key to the §12 attestation trusted-key set (§13.7)- **compiler:** Carry spec §5 compiler version 1.0.0 in the artifact- **host:** Add S3-compatible dighost binary (Artifact 3)- **compiler:** Real deterministic WASM obfuscation for §17.1 (no longer a no-op)- **compiler:** Thread per-store AuthenticationInfo into Compiler::compile (D-AUTH-01)- **compiler:** Real instruction-substitution obfuscation pass (§17.1)- **security:** Audit-driven hardening + authenticated remote head- **cli:** Operate on the current working directory (Git-style discovery)

### Bug Fixes
- **cli:** Read served data via canonical core datasection codec- **core,guest:** Move DIGS data section above guest static data + heap (D2)- **guest:** Enforce §12.2 host BLS attestation verification in the serving gate (D-ATTEST-VERIFY)- **guest:** Enforce §12.2 attestation timestamp freshness in serving gate (D-ATTEST-FRESHNESS)- **host:** Distinguish SessionExpired(-101) from NoSession(-100) (D-SESSION-EXPIRED)- **guest:** Gate JWT auth on an active session before releasing content (D-SESSION-JWT-GATE, §12.4)- **compiler:** Enforce 5.1 16 MiB memory ceiling on emitted module (D-MEM-MAX)- **compiler:** Reject memory64 templates in load_template (D-MEM-MEMORY64)- **compiler:** Reject shared-memory templates in load_template (D-MEM-SHARED §5.1)- **cli:** Make §18.4 host/client boundary explicit (D-HOST-INSPECT)- **compiler,guest:** Emit §5.1 Import section with all 8 dig_host imports (D-IMPORT-SECTION)- **compiler:** Template memory minimum matches §5.1 nominal 1 page (D-MEMORY-MIN-PAGES)- **guest:** Decoy octave uses top 3 bits, eliminating dead seed bits (14.2)

### Refactor
- **prover:** Make CHIA_BLOCK_REF_LEN public + debug-assert public_input length- **guest:** Delegate data-section parsing to digstore_core::datasection (D1/D2)- **compiler:** Emit data section in canonical core format (D1-D5)

### Documentation
- Digstore design spec + 10-crate implementation plans- **guest:** Record deviations (BE codec, deterministic filler, verify-only, C3/C9, sha2-0.9 alias, C8 fixture path); clippy clean lib- Pin canonical data-section contract (compiler<->guest unification)- Drift analysis vs whitepaper — faithful + 8 documented deviations + 2 action items- **core:** Document merkle proof size as <= ceil(log2 n) under carry-up (D-PROOF-PATHLEN)- Spec for artifact 3 — S3-compatible dighost (serve by retrieval key)- Second drift analysis — 5 new drifts found+fixed, attestation re-verified genuine, zero accidental drift- Third drift analysis — zero accidental drift achieved; obfuscation 4/4 genuine- **whitepaper:** Reconcile spec with hardened implementation (v2.0)- **whitepaper:** Project-local .dig layout (§4.4)- Add end-user README

### Testing
- **core:** Add aggregate struct round-trip + golden frame fixtures- **core:** Guard CONVENTIONS C2/C3/C9 module paths- **chunker:** Pin golden boundary sequence + content address (determinism contract)- **chunker:** Freeze front-insert dedup-locality vector (CDC heritage)- **chunker:** Property tests for determinism, bounds, locality, stream parity- **chunker:** Public Chunker round-trip; clippy-clean full suite- **crypto:** Lock SHA-256 wrapper with FIPS vectors and crate version const- **crypto:** Assert sha256(canonical urn) matches Urn::retrieval_key- **crypto:** Enforce unique-key-per-URN invariant for fixed-nonce safety- **crypto:** AES-256-GCM rejects tampered ciphertext, tag, wrong key, truncation- **crypto:** BLS round-trip plus wrong-key/message and malformed-bytes rejection- **crypto:** Freeze real Chia AugScheme known-answer vector (pubkey + signature)- **guest:** JWT gate -> expired/missing token yields decoy (6.3,14.2)- **guest:** Wasm32 build smoke test validates module + all ABI exports (5.1,6.2)- **prover:** Roothash binding enforced in verify_response (§13.4)- **prover:** Nonce binding rejection in verify_with_nonce (§13.5)- **prover:** GUARD chain freshness window accept/reject (§13.8)- **prover:** GUARD node BLS attribution accept/reject (§13.7)- **prover:** GUARD program_hash + public_output mismatch rejection (§13.4)- **prover:** GUARD object-safe traits; full default + feature sweep green- **host:** Return-buffer round-trip and grow-on-demand- **host:** Serve flow propagates guest error sentinels- **host:** Clock injection is deterministic and observed by the guest- **host:** Gated end-to-end serve against real guest fixture- **compiler:** Independently-validated golden data-section vector- **compiler:** Double-compile byte-identical determinism, plain and obfuscated (19.3)- **compiler:** Wasmtime harness proves obfuscation preserves export behavior (17.1)- **remote:** Wire indistinguishability of content hit vs decoy + clippy clean (§14.2/§15)- **cli:** Full e2e suite (round-trip, private salt, decoy, tamper, remote)- **properties:** Add §17.2 secretless scan + §9.4 store root==tree-root- **cli:** Adversarial self-serving e2e — real module serves itself with verifying proof- **guest:** Pin §12.3 fail-closed when no embedded TrustedKeys section (D-ATTEST-TRUSTSET)- **guest:** Pin §12.1/§12.2 gate signs real AttestationChallenge, not a literal (D-ATTEST-NONCE)

### CI
- Secure CI suite on PR + Windows installer release on tag

### Chores
- **core:** Scaffold digstore-core no_std crate skeleton- **core:** Clippy clean + no_std build gate- **chunker:** Scaffold digstore-chunker crate with compiling module stubs- **crypto:** Pin chia-bls=0.45.0 and scaffold digstore-crypto manifest- **crypto:** Satisfy clippy -D warnings (io::Error::other) and clean doc build- **store:** Pass clippy and fmt; finalize public re-exports- **host:** Scaffold digstore-host crate skeleton- **host:** Clippy clean and full-suite green- **cli:** Scaffold digstore-cli crate manifest and entry points- **cli:** Clippy + fmt clean; workspace integration green


