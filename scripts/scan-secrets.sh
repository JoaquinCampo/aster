#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

# Scan tracked content only; ignored state databases and build output are excluded.
# These are high-confidence credential forms, not generic words such as "token".
pattern='(-----BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9_]{30,}|github_pat_[A-Za-z0-9_]{30,}|sk-[A-Za-z0-9_-]{20,}|xai-[A-Za-z0-9_-]{20,}|(api[_-]?key|secret|password)[[:space:]]*[:=][[:space:]]*["'"'][^"'"']{8,}["'"'])'

matches=$(git grep -nIE "$pattern" -- . ':(exclude)Cargo.lock' ':(exclude)scripts/scan-secrets.sh' || true)
if [[ -n "$matches" ]]; then
  echo "Potential secret material found in tracked files:" >&2
  echo "$matches" >&2
  exit 1
fi

echo "Tracked-file secret scan passed."
