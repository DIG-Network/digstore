#!/usr/bin/env bash
# Test `digstore push` (and clone/pull) locally — NO mainnet money, NO hub account,
# NO device-pairing login.
#
# Why this wraps the integration suite instead of poking a live `digstore serve`:
# the §21 push fast-forwards a remote that ALREADY hosts the store with a known
# (signed) head. In production rpc.dig.net provisions that server-side; a bare
# `digstore serve` of a never-pushed store can't bootstrap a first push (its head
# carries no publisher signature yet, so a peer's clone fails closed). The
# integration suite stands up a real in-process §21 server (`TestServer`, the same
# axum `RemoteServer` digstore serve runs) pre-provisioned for the store, then drives
# the REAL `digstore push`/`pull`/`clone` commands against it over loopback HTTP —
# init/commit are anchored against the in-memory mock so nothing touches Chia.
#
# Coverage (crates/digstore-cli/tests/cli_remote_clone_push_pull.rs):
#   - push_fast_forward_then_pull_advances  — push v1, pull; commit v2, push, pull advances
#   - clone_then_cat_round_trips_from_remote — clone a served store + read it back (verified)
#   - clone/push rejection + tamper cases
#
# Usage:  bash scripts/local-push-test.sh   (from the digstore repo root)
#
# Manual interactive variant (host a store locally for clone/pull):
#   digstore serve --anonymous --bind 127.0.0.1:8612        # terminal A (after a commit)
#   digstore clone http://127.0.0.1:8612/stores/<storeId>   # terminal B
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Building the guest wasm (digstore-cli build.rs prerequisite)…"
cargo build -p digstore-guest --target wasm32-unknown-unknown --release >/dev/null

echo "==> Running the local §21 push/pull/clone integration suite…"
cargo test -p digstore-cli --test cli_remote_clone_push_pull -- --nocapture
echo "LOCAL PUSH TEST: PASS (push/pull/clone exercised over a real in-process §21 server, offline)"
