# Architecture

Aster is a Rust control plane with an Elm-style terminal UI. Pi is represented by an adapter boundary; the repository currently uses deterministic adapters and must not be described as live Pi integration.

## Components

* `domain` defines task, route, lifecycle, event, and evidence types.
* `routing` selects an explicit route and rationale; current policy breadth is limited.
* `runtime` coordinates routing, provider execution, durable transitions, and verification.
* `provider` normalizes streaming provider events and includes fixture-backed OpenAI-compatible behavior.
* `effects` is the mandatory broker for filesystem/process/network/secret/external effects in trusted core code.
* `store` persists tasks/events in SQLite and marks interrupted work for reconciliation.
* `config`, `context`, and `memory` provide validated configuration, instruction/context manifests, and scoped provenance-aware memory.
* `plugin` runs declared plugins out of process through brokered capabilities.
* `tui` renders state and translates key events into commands without blocking rendering.

## Boundaries and data flow

TUI command → runtime → route/context/capability decision → durable intent/event → adapter/broker operation → streamed events → durable outcome/evidence → TUI projection. Adapter output, repository content, plugins, hooks, MCP servers, and model output are untrusted. Parsing does not confer trust.

SQLite is a single-node durability boundary, not a distributed scheduler. An append-only event record explains transitions; mutable/deletable content must stay outside immutable audit payloads. Unknown effect outcomes require reconciliation rather than blind replay.

## Decisions and gaps

The provisional name is not encoded into protocol boundaries. Provider and effect traits preserve replacement points. OS-level sandboxing, live Pi/Codex operation, complete DAG scheduling, full lifecycle controls, and the complete observability workspace remain open; see the acceptance matrix. Architecture rationale begins in `docs/adr/0001-architecture.md`.
