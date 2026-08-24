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
import re, sys, tomllib

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
PYEOF

python3 -c "$PY" "$MANIFEST"
