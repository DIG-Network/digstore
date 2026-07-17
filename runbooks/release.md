# Runbook — releasing dig-store (nightly cron + manual dispatch)

How this repo's `dig-store` CLI (+ the `digs` alias) is built and released. The shape is copied from
the ecosystem's **reference nightlies system** (`dig-updater`, dig_ecosystem #590/#592); the
normative contract is `SPEC.md` §12.

## TL;DR

- Releases are **NOT cut on merge to `main`**. They are batched to a **nightly cron at midnight UTC**
  plus **manual dispatch**.
- **Stable** (`vX.Y.Z`): cut automatically when the `[workspace.package].version` was bumped
  (detected as "the `vX.Y.Z` tag doesn't exist yet"), or on demand. Publishes bare per-OS binaries
  (+ `digs`) + apt `.tar.gz`, AND uploads the Linux x86_64 binary to the dighub S3 bucket.
- **Nightly**: built every night from `main` HEAD as a **pre-release** under a dated tag
  `nightly-YYYYMMDD` + a rolling `nightly` tag. `prerelease: true`, never `latest`, no S3 upload.
  Keeps 14.

## Prerequisites / credentials

- **`RELEASE_TOKEN`** — an org-level classic PAT. Both channels no-op with a warning if it is
  absent. Pushes the changelog commit past branch protection + pushes tags that trigger downstream
  workflows (`GITHUB_TOKEN` cannot do either).
- **S3 (stable only, optional):** `AWS_ARTIFACT_ROLE_ARN` (secret) + `AWS_REGION` +
  `DIGSTORE_ARTIFACT_BUCKET` (vars) for the hub compile-worker binary upload. The S3 step no-ops
  (does not fail the release) when unset.

## If nightlies silently stop — check for the 60-day cron auto-disable

GitHub disables a `schedule:` trigger after **60 days of no repo activity** on a public repo, with
**no automatic re-enable** — and since this cron is the *only* automatic release trigger, a quiet
repo can go dark with no error. If nightlies (or a long-overdue stable release) stop appearing:

```bash
gh api repos/DIG-Network/dig-store/actions/workflows/nightly-release.yml --jq .state
# "disabled_inactivity" means GitHub turned it off — re-enable it:
gh workflow enable nightly-release.yml --repo DIG-Network/dig-store
```

Any repo activity (a merged PR, a manual dispatch) resets the 60-day counter.

## Cut a STABLE release (the normal path)

1. In your feature PR, bump `[workspace.package].version` in the root `Cargo.toml` per SemVer and run
   `cargo update --workspace` so `Cargo.lock` matches. Merge the PR (squash).
2. Nothing releases on merge. At the next **midnight UTC** the `nightly-release.yml` cron runs its
   **stable** job: it sees the new version has no `vX.Y.Z` tag, regenerates `CHANGELOG.md`, commits
   `chore(release): vX.Y.Z` to `main`, tags it, and pushes with `RELEASE_TOKEN`.
3. The pushed `v*` tag fires `release.yml`, which builds every OS/arch (guest-wasm prereq first),
   publishes the GitHub Release (bare binaries + `digs` + apt tarballs), and uploads the Linux
   binary to S3.

> TRANSITIONAL (rename epic #703): the primary binary was renamed `digstore` -> `dig-store`. For one
> transition cycle every asset is DUAL-PUBLISHED under BOTH the new `dig-store-*` stem AND the legacy
> `digstore-*` stem (bare binaries + apt `.tar.gz`), and each apt tarball ships a `digstore` ->
> `dig-store` compat symlink at its root, so apt.dig.net + dig-installer stay green until they cut
> over to `dig-store-*`. The dighub S3 hub-worker layout (`digstore/<ver>/digstore`) is UNCHANGED
> (it is the compile-worker's contract). Drop the legacy stem + symlink in a later release once both
> installers have cut over.

### Cut a stable release NOW / re-cut

- Now: Actions → **Nightly + stable release** → **Run workflow** → `channel: stable` (or `both`).
- Re-cut (failed build): same, with **`force: true`**. `force` REFUSES (non-zero exit) when the tag
  already has a PUBLISHED release AND points at a different commit than this run would build — it
  only proceeds for a same-commit retry or a tag with no published release. To ship new code, bump
  `Cargo.toml` instead.

## Cut a NIGHTLY on demand

Actions → **Nightly + stable release** → **Run workflow** → `channel: nightly` (or `both`) → Run.

## Verify a release went live

- **Stable:** `gh release view vX.Y.Z --repo DIG-Network/dig-store` — bare per-OS binaries + `digs` +
  apt `.tar.gz` (x86_64 + aarch64). Watch: `gh run watch <id>`.
- **Nightly:** `gh release view nightly --repo DIG-Network/dig-store` (rolling) or
  `gh release view nightly-YYYYMMDD` — `prerelease: true`.

## Workflows

| File | Trigger | Role |
|---|---|---|
| `nightly-release.yml` | midnight-UTC cron + `workflow_dispatch` | Orchestrator: stable (changelog + tag) + nightly (build + pre-release + prune). |
| `release.yml` | `push: tags: v*` (+ dispatch canary) | Builds + publishes the stable Release + the S3 hub-worker binary for a `vX.Y.Z` tag. |
| `build-binaries.yml` | `workflow_call` | Reusable cross-OS build (guest-wasm prereq + `digs`); both channels call it. |
| `ci.yml` | PR + push to main | fmt/clippy/test/coverage (pre-merge). NOTE: `ubuntu-latest` + `windows-latest` — macOS builds are first exercised by the nightly channel / release, not PR CI (SPEC §12). |

## Local build (dev)

```bash
# Guest-wasm prereq (once per clean checkout — §3.5):
cargo build -p digstore-guest --target wasm32-unknown-unknown --release
cargo build --release --locked
cargo test  --locked        # includes the workflow-shape guard tests
```
