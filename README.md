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

Routing uses stable typed roles (`orchestrator`, `implementer`, `reviewer`, `researcher`, and `tester`) and keeps effort, context/output budgets, latency, capabilities, isolation, and verification independent. Deterministic policy derives task requirements; the profile selector then chooses the cheapest reliable model. Every decision includes candidate evidence, rejected reasons, a stable decision ID, and rationale. Callers may explicitly override role/model/effort/context and constrain cost or latency. Failed verification raises the required quality, while three verified successes permit de-escalation. Delegation decisions expose expected quality, cost, latency, and context deltas.

The versioned deterministic comparison with the fixed strong-model baseline is in [`docs/routing-benchmark-v1.md`](docs/routing-benchmark-v1.md). Profile numbers are fixtures and must not be interpreted as live provider measurements.

## Status

This is an initial vertical slice, **not a completed v0.1**. Live Pi and Codex bridge integration, lifecycle controls, enforced isolation, full configuration, extensibility, compatibility fixtures, and release evidence remain open and are tracked in `docs/acceptance-matrix.md`.
