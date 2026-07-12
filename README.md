# Aster

Aster is a provisional codename for a durable, observable agent harness built around Pi. This repository implements the coherent v0.1 baseline from [`BRIEF.md`](BRIEF.md): a custom Rust TUI and control plane, pinned Pi sidecar integration, dynamic auditable routing, durable orchestration and recovery, enforced effect brokering, context and memory systems, verification workflows, and mediated extensions.

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

Routing uses nine stable typed built-in roles (`orchestrator`, `explorer`, `planner`, `implementer`, `reviewer`, `verifier`, `fixer`, `advisor`, and `learning-capture`) with declarative contracts, and keeps roles independent from model and effort. A schema-validated, reviewed, versioned policy is loaded from `config/routing-policy-v1.toml`. Effort, context/output budgets, latency, capabilities, tools, isolation, lifecycle, and verification are selected and serialized for audit. Persisted outcome aggregates produce advisory recommendations only; activating one requires an explicit reviewed, monotonic policy revision, so learning never silently mutates policy.

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

The v0.1 local acceptance baseline is complete. Deterministic contract fixtures cover xAI/Grok and generic OpenAI-compatible providers as expressly permitted when external credentials are unavailable; live-interoperability breadth remains accurately disclosed in `docs/acceptance-matrix.md`. Release status is tied to the candidate commit's hosted Linux and native macOS arm64 gates.
