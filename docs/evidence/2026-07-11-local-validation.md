# Local validation evidence — 2026-07-11

Commit under validation: `19188bb` and ancestors (record the final release SHA separately after remaining changes).

## Rust quality gates

Executed locally on macOS arm64:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
git diff --check
```

All checks passed. The suite covered configuration/context/memory, durable runtime and recovery, effect security, plugins, provider contracts, routing, verification workflows, the first vertical slice, and the concrete Pi gateway.

## Real TUI PTY

Built the release binary and ran `scripts/tui-acceptance.sh` through `tui-use`. The reproducible run preserved semantic text and JSON snapshots under `docs/evidence/tui-pty/`:

- `120×30`: submitted a deterministic successful task, applied a validated route preset, observed an illegal pause rejection for a succeeded task, and traversed query panes.
- Restarted against the same SQLite database at `60×16` and observed the persisted succeeded task plus compact query panes.
- Restarted at degraded `30×8` dimensions and retained semantic task status.

The script uses semantic waits rather than timing sleeps and retains terminal dimensions, cursor/status metadata, and screen content in JSON alongside human-readable snapshots. Timeout, permission-denial, cancellation-in-flight, and injected recovery-failure PTY cases remain required.

## Local Codex bridge

Discovered running source-backed bridge version `0.1.0` at `127.0.0.1:18474`.

- `/healthz`: healthy.
- `/readyz`: authenticated and ready.
- `/v1/models`: advertised `gpt-5.6-sol` and `gpt-5.6-terra`.
- `/status`: bridge and subscription quota reporting available.
- Live `gpt-5.6-sol`, reasoning effort `low`: completed successfully and returned exactly `ASTER_CODEX_OK`.
- Live usage: 29 input tokens, 8 output tokens, 37 total tokens.

No credential values were read, printed, stored, or sent anywhere except through the bridge's internally managed authenticated destination. Response and account identifiers are intentionally omitted from this evidence.

## Pi runtime

- Inspected upstream `badlogic/pi-mono` commit `8479bd84743e8889f728acb21a62794102db0529`.
- Confirmed MIT license and local installed package versions 0.73.0.
- Exercised installed-package discovery/import and deterministic execution through the actual sidecar boundary.
- `PiGateway` implements the runtime `PiAdapter`; the integrated 13-step acceptance workflow consumes normalized output and usage through that adapter.
- Deterministic mode imports Pi 0.73.0 but never constructs a provider agent, inherits no provider credentials, and makes no paid call.
- Capability preflight denial remains covered at the same normalized event boundary.
