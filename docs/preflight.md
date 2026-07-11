# Preflight record

Recorded 2026-07-11. This is a local discovery record, not evidence of hosted CI, repository publication, or live provider operation.

| Check | Safely discovered fact | Consequence / follow-up |
|---|---|---|
| Path | Assigned isolated Git worktree; `.aster/` and `target/` are ignored | Local state/build output must never enter release archives |
| Platform | macOS 26.5.1, Darwin arm64 | Suitable for local Apple Silicon checks |
| Toolchain | Git 2.50.1; Rust/Cargo 1.96.1; `gh` 2.83.2 | Local Rust quality gates are available |
| Git identity | Name/email configured | No assertion that the identity is correct for publication |
| GitHub auth | `gh auth status` reports active keychain login with repository scope | Authentication discovered; no repository creation or push was attempted |
| Repository remote | No remote configured in this checkout | Public repository and branch protection are unverified blockers |
| Pi | No `pi-mono` source found in the bounded `~/Documents/Personal` search | License, build, tests, fork obligations, and live integration remain blocked |
| Codex bridge | No bridge path found and no Codex-named environment variable found; values were never inspected | Contract is fixture-only; live startup/auth/models/streaming/tools/usage/errors are not validated |
| Other credentials | Only environment variable names were searched; no provider credential is assumed | xAI/OpenAI live checks remain explicit opt-in work |
| TUI tooling | `tui-use` executable and specified skill file exist | Release-critical PTY evidence has not yet been produced |
| Security tools | `cargo-audit` and `gitleaks` absent locally | CI installs `cargo-audit`; repository script provides a deterministic high-confidence tracked-file scan |
| CI feasibility | GitHub API is reachable; workflow requests `macos-14` and asserts `Darwin-arm64` | A successful hosted run is required evidence; local feasibility is not that evidence |
| Persistence | Default `.aster/state.db`; directory ignored | Operators must protect file permissions and backups; secrets must not be stored there |

## Safe reproduction

Run version/platform commands individually, `gh auth status` (never `gh auth token`), inspect environment **names only**, and use bounded searches. Redact token-bearing output before preservation. Do not probe authenticated providers unless the operator explicitly selects that destination and accepts possible usage.

## Blocker classification

* **Core/integration:** Pi source/runtime and the local Codex bridge were not discoverable in the bounded search. Deterministic adapters validate internal boundaries but do not establish live compatibility.
* **Environmental:** no remote, no preserved Actions run, no local `cargo-audit`, and no hosted Apple Silicon artifact. These do not block documentation or local tests, but block release claims.
* **Integration:** xAI and generic OpenAI credentials were not assumed. Contract fixtures may be tested without claiming live operation.
