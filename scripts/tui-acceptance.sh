#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
EVIDENCE=${1:-"$ROOT/docs/evidence/tui-pty"}
DB=${ASTER_TUI_DB:-"${TMPDIR:-/tmp}/aster-tui-acceptance.db"}
BIN="$ROOT/target/release/aster"
mkdir -p "$EVIDENCE"
rm -f "$DB" "$DB.toml"

capture() {
  local name=$1
  tui-use wait --debounce 250 >/dev/null
  tui-use snapshot >"$EVIDENCE/$name.txt"
  tui-use snapshot --format json >"$EVIDENCE/$name.json"
}

expect() {
  tui-use wait --text "$1" >/dev/null
}

cargo build --locked --release --manifest-path "$ROOT/Cargo.toml"

# Wide: successful execution, validated route override, lifecycle legality, and query panes.
tui-use start --label aster-wide --cols 120 --rows 30 --cwd "$ROOT" "$BIN --state $DB" >/dev/null
expect "Submit task"
capture 120x30-initial
tui-use type "implement deterministic acceptance evidence"
tui-use press enter
expect "Succeeded"
capture 120x30-success
tui-use type "o"
expect "route override editor"
tui-use press arrow_down
tui-use press enter
tui-use wait --debounce 250 >/dev/null
for _ in 1 2 3 4; do tui-use press tab; done
expect "Routing/Overrides"
capture 120x30-route-override
for _ in 1 2 3 4; do tui-use press arrow_left; done
tui-use type "p"
expect "pause unavailable for Succeeded"
capture 120x30-lifecycle-legality
for pane in tasks dag routing audit context artifacts usage diagnostics; do
  tui-use press tab
  capture "120x30-pane-$pane"
done
tui-use type "q"
tui-use wait >/dev/null || true

# Deterministic release-critical failures are driven through the real PTY.
tui-use start --label aster-failures --cols 120 --rows 30 --cwd "$ROOT" "$BIN --state $DB" >/dev/null
expect "Succeeded"
tui-use type "scenario:timeout"
tui-use press enter
expect "TimedOut"
capture 120x30-timeout
tui-use type "scenario:permission-denied"
tui-use press enter
expect "Failed"
capture 120x30-permission-denied
tui-use type "scenario:in-flight-cancellation"
tui-use press enter
expect "press x to cancel"
capture 120x30-cancellation-in-flight
tui-use type "x"
expect "Cancelled"
capture 120x30-cancelled
tui-use type "q"
tui-use wait >/dev/null || true

# The provider exits after a durable operation start. Restart invokes recovery,
# exposes OutcomeUnknown, and requires an explicit reconciled outcome.
set +e
tui-use start --label aster-crash --cols 120 --rows 30 --cwd "$ROOT" "$BIN --state $DB" >/dev/null
tui-use wait --text "Submit task" >/dev/null
tui-use type "scenario:injected-crash"
tui-use press enter
tui-use wait >/dev/null || true
set -e
tui-use start --label aster-recovery --cols 120 --rows 30 --cwd "$ROOT" "$BIN --state $DB" >/dev/null
expect "OutcomeUnknown"
capture 120x30-injected-crash-recovery
tui-use type "y"
expect "Succeeded"
capture 120x30-outcome-reconciled
tui-use type "q"
tui-use wait >/dev/null || true

# Compact restart: prove durable task recovery and inspect additional query panes.
tui-use start --label aster-compact --cols 60 --rows 16 --cwd "$ROOT" "$BIN --state $DB" >/dev/null
expect "Succeeded"
capture 60x16-restart
for pane in next-1 next-2 next-3 next-4; do
  tui-use press tab
  capture "60x16-$pane"
done
tui-use type "q"
tui-use wait >/dev/null || true

# Degraded dimensions remain operable and retain semantic status evidence.
tui-use start --label aster-degraded --cols 30 --rows 8 --cwd "$ROOT" "$BIN --state $DB" >/dev/null
expect "Succeeded"
capture 30x8-restart
tui-use type "q"
tui-use wait >/dev/null || true

printf 'PTY acceptance evidence: %s\n' "$EVIDENCE"
