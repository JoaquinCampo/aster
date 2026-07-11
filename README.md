# Aster

Aster is a provisional codename for a durable, observable agent harness built around Pi. This repository currently implements the **required first architectural slice** from [`BRIEF.md`](BRIEF.md): a Rust TUI, explicit auditable routing, a Pi adapter boundary with deterministic fake, durable SQLite task/event history, and deterministic verification evidence.

## Run

```sh
cargo run
```

Type a task and press Enter. Quit with `q` while the input is empty. State is stored at `.aster/state.db` and survives restart.

## Verify

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Status

This is an initial vertical slice, **not a completed v0.1**. Live Pi and Codex bridge integration, lifecycle controls, enforced isolation, full configuration, extensibility, compatibility fixtures, and release evidence remain open and are tracked in `docs/acceptance-matrix.md`.
