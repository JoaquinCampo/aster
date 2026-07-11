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

## Effect isolation and platform limitations

All filesystem, process, network, secret, and external effects must pass through `EffectBroker`. Grants are task-, capability-, workspace/worktree-, executable-, destination-, and service-scoped; authorizations are persisted before adapter invocation. Process execution uses an argv vector (never a shell string) and an environment cleared before explicitly granted variables are added. Filesystem policy canonicalizes roots and targets, rejects `..`, and prevents symlink escapes.

The isolation profile is intentionally multidimensional: filesystem, process, network, and secrets are configured independently. On macOS these controls are broker-level policy enforcement, **not an OS sandbox**: they do not provide container/namespace isolation, syscall filtering, protection from adapters that bypass the broker, or complete TOCTOU protection if another process mutates paths between validation and use. Production adapters should additionally use sandboxed workers and descriptor-relative filesystem APIs. Network and external transports are deny-by-default placeholders until explicitly implemented.

## Typed, auditable routing

Routing uses nine stable typed built-in roles (`orchestrator`, `explorer`, `planner`, `implementer`, `reviewer`, `verifier`, `fixer`, `advisor`, and `learning-capture`) with declarative contracts, and keeps roles independent from model and effort. Effort, context/output budgets, latency, capabilities, isolation, and verification are separate route dimensions. Deterministic policy derives task requirements; the static fixture-profile selector then chooses the cheapest eligible model. Every decision includes hard/soft constraint evidence, rejected reasons, a stable decision ID, and rationale. Unknown model overrides and overrides that violate hard quality, context, cost, or latency constraints return a typed `NoEligibleRoute`; routing never falls back to an ineligible model. No persisted outcome history is currently integrated, so this is deterministic policy routing rather than hybrid/history-based routing.

The versioned deterministic comparison with the fixed strong-model baseline is in [`docs/routing-benchmark-v1.md`](docs/routing-benchmark-v1.md). Profile numbers are fixtures and must not be interpreted as live provider measurements.

## Documentation

- [Preflight and genuine blockers](docs/preflight.md)
- [Architecture](docs/architecture.md)
- [Operations and release gates](docs/operations.md)
- [Configuration](docs/configuration.md)
- [Security model](docs/security.md)
- [Recovery](docs/recovery.md)
- [Extension development](docs/extensions.md)
- [Contributing](docs/contributing.md)
- [BRIEF requirements matrix](docs/acceptance-matrix.md)

## Status

This is an initial vertical slice, **not a completed v0.1**. Live Pi and Codex bridge integration, lifecycle controls, OS-enforced isolation, full configuration, extensibility, compatibility fixtures, benchmarks, hosted CI, TUI PTY validation, and macOS arm64 release evidence remain open and are tracked in `docs/acceptance-matrix.md`.
