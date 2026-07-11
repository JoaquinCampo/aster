# Durable autonomous preflight

Re-checked 2026-07-11 from `main` at `d1a3131`. The reproducible repository checks are in `scripts/validate-autonomous-preflight.sh`; captured output is preserved in `docs/evidence/2026-07-11-autonomous-preflight.txt` and the credential-free assertions run in CI. No paid-provider request was made during this refresh.

| Check | Re-checked evidence | Acceptance / remaining boundary |
|---|---|---|
| Path and Git | `/Users/joaquincamponario/Documents/Personal/my-harness`; clean `main`; HEAD and `origin/main` both `d1a31310f9ca415be27b262cb5ee84869cce9f17` before this refresh | Isolated repository is correctly based on the requested commit; `.aster/` and `target/` remain ignored |
| GitHub CLI and publication | `gh` 2.83.2 authenticated via keyring; `gh repo view` reports `JoaquinCampo/aster`, public, default `main`; SSH origin is configured | Auth was checked with `gh auth status`; no token value was requested or preserved |
| Rust and platform | Darwin arm64; Rust/Cargo 1.96.1; Git 2.50.1; Node 26.0.0/npm 11.12.1 | Native local Apple Silicon and locked Rust/Node gates are available |
| Pi source and license | `badlogic/pi-mono` commit `8479bd84743e8889f728acb21a62794102db0529`; MIT obligations in `THIRD_PARTY_NOTICES.md`; exact 0.73.0 packages pinned in `package-lock.json` | Source-backed discovery/import, deterministic sidecar execution, normalized usage, capability denial, and integrated workflow are tested without constructing a provider agent or inheriting credentials |
| Pi build/test/integration | `npm ci --ignore-scripts` installs pinned packages; Rust tests exercise `PiGateway` → Node sidecar → actual package import | Deterministic mode makes no provider request; live Pi/provider interoperability is not inferred |
| Codex bridge | Source-backed local bridge contract and previously preserved authenticated Sol streaming/usage evidence remain in `docs/provider-contract.md` and local-validation evidence | This refresh re-used preserved evidence and made no paid call; Luna/Terra, tool-loop, cancellation, and full live error matrix remain partial |
| TUI / `tui-use` | Reproducible `scripts/tui-acceptance.sh`; semantic text/JSON snapshots cover success, route override, panes, timeout, permission denial, cancellation, crash/recovery/reconciliation, restart, and three dimensions | Deterministic scenarios make no paid calls |
| CI arm64 | Public Actions workflow uses `macos-14`, asserts `Darwin-arm64`, builds/tests/smokes release and uploads the binary; run `29170135437` passed both jobs on predecessor `5a6013e` | The pushed refresh commit must obtain its own green hosted run before release |
| Secret paths | Pi child clears its environment and restores only `PATH` plus explicit package location; provider auth uses references; tracked-file scanner and adversarial redaction/deletion tests are gated | Environment names/auth status only were inspected; credential values were neither read nor persisted |
| Persistence | Default `.aster/state.db`; `.aster/` ignored; normalized SQLite persistence has restart, recovery, reconciliation, migration, integrity, deletion, WAL/free-page cleanup, and clean-checkout coverage | Operators remain responsible for local file permissions and backups |

## Safe reproduction

Run `./scripts/validate-autonomous-preflight.sh` for credential-free repository assertions. Run with `--report` only in an authenticated local environment to append path, HEAD, branch, origin, platform/toolchain, public-repository metadata, and auth availability. The script calls `gh auth status` but never `gh auth token`. Do not probe a hosted provider merely to refresh this record.

## Blocker classification

- **External live breadth:** Codex Luna/Terra and the live tool/error/cancellation matrix, xAI/Grok, and generic OpenAI-compatible operation remain outside deterministic acceptance. Existing evidence is not promoted beyond what it proves.
- **Release evidence:** the refresh commit needs a green Linux quality/security/preflight/clean-checkout job and native macOS arm64 release job.
- **No current preflight blocker:** path, Git, public repository, authenticated CLI capability, Rust/platform, pinned source-licensed Pi boundary, preserved Codex evidence, `tui-use`, CI shape, secret paths, and persistence are all evidenced and reproducibly checked.
