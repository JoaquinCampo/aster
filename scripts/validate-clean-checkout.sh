#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/aster-clean.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

# Validate the staged candidate when an index exists; in CI this equals HEAD.
# git archive excludes ignored and other local-only state by construction.
tree=$(git -C "$root" write-tree)
git -C "$root" archive --format=tar "$tree" | tar -xf - -C "$tmp"
cd "$tmp"

test ! -e .aster
npm ci --ignore-scripts
cargo build --locked
cargo test --locked --all-targets --all-features
./target/debug/aster --help >/dev/null

echo "Clean-checkout build, test, and smoke validation passed."
