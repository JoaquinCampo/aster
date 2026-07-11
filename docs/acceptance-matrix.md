# Acceptance matrix

| Requirement | Milestone | Status | Evidence | Limitation |
|---|---|---|---|---|
| Rust TUI submits a task | M1 | Implemented | `src/tui.rs`; manual PTY validation pending | Minimal task-list UI |
| Explicit route and rationale | M1 | Implemented | `routing.rs`, integration test | Static policy only |
| Pi execution adapter | M1 | Partial | `PiAdapter` + deterministic fake | Live Pi adapter pending inspection |
| Durable task and event history | M1 | Implemented | restart integration test | Single-node only |
| Inspectable output and verification | M1 | Partial | persisted task fields | Detail pane pending |
| Full v0.1 integrated workflow | M2–M8 | Not started | — | See `BRIEF.md` |
