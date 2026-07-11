#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

fail() { printf 'FAIL\t%s\n' "$1" >&2; exit 1; }
pass() { printf 'PASS\t%s\n' "$1"; }
has() { grep -Eq "$2" "$1" || fail "$3"; pass "$3"; }

# Repository-backed checks are deterministic, credential-free, and safe in CI.
test "$(git rev-parse --is-inside-work-tree)" = true || fail "git worktree"
pass "git worktree"
test -n "$(git remote get-url origin)" || fail "origin remote"
pass "origin remote configured"
test -f LICENSE && has LICENSE 'MIT License' "project MIT license"
has package-lock.json '"@mariozechner/pi-agent-core": "0.73.0"' "Pi agent package pinned"
has package-lock.json '"@mariozechner/pi-ai": "0.73.0"' "Pi AI package pinned"
has docs/pi-contract.md 'badlogic/pi-mono.*8479bd84743e8889f728acb21a62794102db0529' "Pi source revision recorded"
has docs/pi-contract.md 'MIT' "Pi license recorded"
has scripts/pi-sidecar.mjs 'pi-agent-core' "Pi source-backed sidecar import"
has tests/pi_gateway.rs 'deterministic' "Pi deterministic integration coverage"
has docs/provider-contract.md 'Codex' "Codex bridge contract recorded"
has docs/evidence/2026-07-11-local-validation.md 'Live `gpt-5.6-sol`' "Codex live evidence preserved"
has scripts/tui-acceptance.sh 'tui-use' "tui-use acceptance driver"
test -f docs/evidence/tui-pty/120x30-success.json || fail "TUI PTY evidence"
pass "TUI PTY evidence preserved"
has .github/workflows/quality.yml 'runs-on: macos-14' "macOS arm64 CI runner"
has .github/workflows/quality.yml 'Darwin-arm64' "native arm64 CI assertion"
has .github/workflows/quality.yml 'validate-autonomous-preflight.sh' "autonomous preflight CI gate"
has scripts/scan-secrets.sh 'git grep' "tracked-file secret scan"
has src/pi_gateway.rs 'env_clear' "Pi child environment allowlist"
has .gitignore '^\.aster/' "local persistence ignored"
has src/main.rs '\.aster/state.db' "SQLite persistence default"
has tests/verification_persistence.rs 'restart' "persistence restart coverage"
has scripts/validate-clean-checkout.sh 'git.*archive' "clean-checkout release validation"
has scripts/check-no-telemetry.sh 'telemetry' "no-telemetry release gate"

if [[ ${1:-} == --report ]]; then
  printf '\nDISCOVERED\troot=%s\n' "$root"
  printf 'DISCOVERED\thead=%s\n' "$(git rev-parse HEAD)"
  printf 'DISCOVERED\tbranch=%s\n' "$(git branch --show-current)"
  printf 'DISCOVERED\torigin=%s\n' "$(git remote get-url origin)"
  printf 'DISCOVERED\tplatform=%s-%s\n' "$(uname -s)" "$(uname -m)"
  printf 'DISCOVERED\trustc=%s\n' "$(rustc --version)"
  printf 'DISCOVERED\tcargo=%s\n' "$(cargo --version)"
  printf 'DISCOVERED\tgh=%s\n' "$(gh --version | head -1)"
  gh repo view --json nameWithOwner,isPrivate,url,defaultBranchRef \
    --jq '"DISCOVERED\trepository=\(.nameWithOwner) public=\(.isPrivate | not) default=\(.defaultBranchRef.name) url=\(.url)"'
  gh auth status >/dev/null
  pass "GitHub authentication available (token value not read)"
fi
