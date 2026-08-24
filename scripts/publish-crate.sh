#!/usr/bin/env bash
# Publish ONE workspace crate to crates.io, idempotently.
#
#   ./scripts/publish-crate.sh <crate-name>
#
# WHY a script rather than inline workflow YAML: the publish is identical for every
# publishable crate in this workspace, and the ordering between them (release-first,
# bottom-up) is the only thing that differs. Keeping the logic in one place means a
# fix to the skip-if-exists probe cannot land for one crate and be forgotten for another.
#
# The publish is IDEMPOTENT: crates.io rejects a duplicate version, so a re-run or a
# stray tag must NO-OP rather than fail red. `$CARGO_REGISTRY_TOKEN` is read from the
# environment and never echoed.
set -euo pipefail

CRATE="${1:?usage: publish-crate.sh <crate-name>}"

# crates.io REQUIRES a descriptive User-Agent; without one it answers in a way that is
# indistinguishable from "this crate does not exist", which would make every probe read
# as "not published" and turn the skip-if-exists guard into a no-op.
UA="dig-ecosystem-ci (help@dig.net)"

# Package first: proves the manifest is complete and that no `path`/`git` dependency
# leaks into the published form, WITHOUT publishing anything.
echo "::group::cargo package -p $CRATE"
cargo package -p "$CRATE" --locked
echo "::endgroup::"

VERSION=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import sys,json; d=json.load(sys.stdin); print(next(p["version"] for p in d["packages"] if p["name"]==sys.argv[1]))' "$CRATE")
echo "$CRATE version to publish: $VERSION"

BODY=$(curl -fsS -A "$UA" "https://crates.io/api/v1/crates/$CRATE/versions" || echo '')
if [ -n "$BODY" ] && printf '%s' "$BODY" | python3 -c '
import sys, json
doc = json.load(sys.stdin)
sys.exit(0 if sys.argv[1] in [v["num"] for v in doc.get("versions", [])] else 1)
' "$VERSION"; then
  echo "$CRATE@$VERSION is already on crates.io — skipping publish (no-op)."
  exit 0
fi

if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
  echo "::error::CARGO_REGISTRY_TOKEN is not set (repo or org secret)."
  exit 1
fi

cargo publish -p "$CRATE" --locked --token "$CARGO_REGISTRY_TOKEN"

# Read the version back from the sparse index rather than trusting the publish's exit
# code — a green publish step is not a published crate. The index is eventually
# consistent, so poll briefly.
PREFIX=$(python3 -c '
import sys
n = sys.argv[1]
print(n if len(n) < 4 else f"{n[:2]}/{n[2:4]}" if len(n) > 3 else n)
' "$CRATE")
for _ in $(seq 1 30); do
  if curl -fsS -A "$UA" "https://index.crates.io/$PREFIX/$CRATE" \
      | grep -q "\"vers\":\"$VERSION\""; then
    echo "VERIFIED on index.crates.io: $CRATE@$VERSION"
    exit 0
  fi
  sleep 10
done
echo "::error::$CRATE@$VERSION did not appear on index.crates.io within 5 minutes."
exit 1
