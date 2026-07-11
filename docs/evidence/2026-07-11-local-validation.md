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

Built the release binary and operated it with `tui-use`:

- `120×30`: launched, submitted `implement durable routing test`, observed the durable queued task.
- Quit and restarted against the same SQLite database.
- `60×16`: opened the Tasks screen and observed the same task after restart.
- `30×8` degraded rendering is covered by a deterministic Ratatui backend test.

The initial PTY run exposed a real startup defect caused by `terminal.clear()` querying cursor position. The defect was removed in commit `271cf90`; the release binary then launched and operated successfully. Broader failure/cancellation/permission/recovery PTY workflows remain required.

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
- Exercised the actual installed package import/version-discovery boundary through the Aster sidecar integration test.
- Deterministic Pi fixture execution validates normalized event and tool-preflight behavior without paid provider traffic.
