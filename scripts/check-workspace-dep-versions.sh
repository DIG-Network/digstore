#!/usr/bin/env bash
# Assert every in-repo `[workspace.dependencies]` entry declares the SAME version as
# `[workspace.package].version`.
#
# WHY this exists: the publishable crates (`digstore-core`, `digstore-chain`) inherit their
# own version from `[workspace.package]`, but a downstream consumer resolves the version
# written in the *dependency* declaration. Those two numbers are edited in different places
# and nothing else couples them, so a release bump can silently leave a published crate
# depending on the PREVIOUS version of its sibling — which either fails the publish or, worse,
# ships a manifest pointing at stale code. Cargo has no built-in check for this.
set -euo pipefail

MANIFEST="${1:-Cargo.toml}"

read -r -d '' PY <<'PYEOF' || true
import pathlib, sys, tomllib

manifest = sys.argv[1]
with open(manifest, "rb") as fh:
    doc = tomllib.load(fh)

workspace = doc.get("workspace", {})
expected = workspace.get("package", {}).get("version")
if expected is None:
    sys.exit(f"{manifest}: [workspace.package].version is missing")

failures = []
for name, spec in workspace.get("dependencies", {}).items():
    # Only in-repo path deps are coupled to the workspace version; external crates are not.
    if not isinstance(spec, dict) or "path" not in spec:
        continue
    declared = spec.get("version")
    if declared is None:
        failures.append(f"  {name}: declares `path` with no `version` (unpublishable)")
    elif declared != expected:
        failures.append(f"  {name}: declares {declared!r}, workspace version is {expected!r}")

if failures:
    print(f"{manifest}: workspace dependency versions are out of step:", file=sys.stderr)
    print("\n".join(failures), file=sys.stderr)
    sys.exit(1)

print(f"{manifest}: all in-repo workspace dependencies declare version {expected}")

# ── Member-declared in-repo path deps must name the target crate's ACTUAL package version. ──
root = pathlib.Path(manifest).parent
member_failures = []
checked = 0
for member in workspace.get("members", []):
    member_manifest = root / member / "Cargo.toml"
    if not member_manifest.is_file():
        continue
    with open(member_manifest, "rb") as fh:
        member_doc = tomllib.load(fh)
    # A member that never publishes may declare siblings however it likes; only a crate that
    # ships to crates.io needs a version on every in-repo dependency.
    if member_doc.get("package", {}).get("publish", True) is False:
        continue
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for name, spec in member_doc.get(section, {}).items():
            if not isinstance(spec, dict) or "path" not in spec:
                continue
            target_manifest = (member_manifest.parent / spec["path"] / "Cargo.toml").resolve()
            if not target_manifest.is_file():
                continue
            with open(target_manifest, "rb") as fh:
                target_doc = tomllib.load(fh)
            target_pkg = target_doc.get("package", {})
            actual = target_pkg.get("version")
            # A member inheriting `version.workspace = true` carries the workspace version.
            if not isinstance(actual, str):
                actual = expected
            declared = spec.get("version")
            checked += 1
            if declared is None:
                # A version-less DEV-dependency is legal in a published crate: cargo drops
                # it from the published manifest entirely rather than refusing the package.
                # That is load-bearing here — digstore-host dev-depends on digstore-cli,
                # which depends back on digstore-host, so a versioned dev-dep would be a
                # cyclic registry dependency that can never resolve on a first publish.
                # Only a normal or build dependency must carry a version.
                if section == "dev-dependencies":
                    continue
                member_failures.append(
                    f"  {member} [{section}] {name}: bare `path` with no `version`, but "
                    f"{member} publishes — cargo refuses such a dependency")
            elif declared != actual:
                member_failures.append(
                    f"  {member} [{section}] {name}: declares {declared!r}, "
                    f"but {name} carries {actual!r}")

if member_failures:
    print(f"{manifest}: member-declared in-repo dependency versions are out of step:",
          file=sys.stderr)
    print(chr(10).join(member_failures), file=sys.stderr)
    sys.exit(1)

print(f"{manifest}: {checked} member-declared in-repo path deps all name a real version")
PYEOF

python3 -c "$PY" "$MANIFEST"
