# Operations

## Build and run

Prerequisites: a current stable Rust toolchain and macOS or Linux development environment.

```sh
cargo build --locked
cargo run --locked
```

The default database is `.aster/state.db`. Start from the repository root. `cargo run -- --help` is the non-interactive smoke check. The live Pi/Codex adapter is not available, so routine operation uses implemented deterministic boundaries only.

## Quality and release procedure

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
./scripts/scan-secrets.sh
```

After committing the candidate, run `./scripts/validate-clean-checkout.sh`. Push only after the `quality` and `macOS arm64 release gate` jobs succeed. Preserve the Actions run URL, logs, commit SHA, runner `uname`, and uploaded binary digest under `docs/evidence/` or the release record. Never substitute a local arm64 run for hosted gate evidence.

## Runtime care

* Restrict access to `.aster/`; it may contain task content and audit metadata.
* Do not put API key values in configuration. Use environment references and grant only the destination that needs them.
* Back up the database only while the process is stopped, or use SQLite's supported online backup API.
* Treat plugin diagnostics and provider output as untrusted.
* On startup after interruption, inspect reconciliation-required operations before retrying.

## Troubleshooting

Run with a fresh disposable state directory when isolating corruption, preserve the original first, and do not delete it to hide unknown outcomes. Provider errors should retain their normalized category and safe message, not credentials or raw authentication headers. Recovery is detailed in `recovery.md`.
