# Configuration

Configuration is TOML parsed into schema-validated canonical types. Layering is defaults → user → project; later layers override known scalar fields. Instruction discovery separately supports `.agents` and `.claude` compatibility with `.claude` preferred for equivalent conflicts. See `config-context-memory.md` for implemented details and fixtures.

## Safety contract

* Persist secret **references**, never resolved values.
* Reject unknown schema versions and invalid capability combinations.
* Preserve unknown fields during semantic file/TUI round trips where supported; comments and whitespace are not contractual.
* Write atomically and detect concurrent modification before replacement.
* Keep roles independent from model and reasoning effort.
* Treat filesystem, process, network, credentials, external services, and workspace/worktree isolation as separate dimensions.

## Current coverage

Validated typed configuration and precedence tests exist for the implemented slice. The Config TUI screen offers representative keyboard edits for context budget and routing, verification, and lifecycle enablement. Each action reloads the document, applies a schema-validated semantic edit, preserves unknown fields, detects a stale baseline, and atomically replaces the file. Full editors for every BRIEF domain, migration UX, and comment/whitespace preservation are not yet complete. Do not hand-author settings based on aspirational fields in the BRIEF; only fields accepted by current types are operational.

Before changing configuration, back up the file, validate with tests, and inspect the effective configuration and provenance. Never paste a resolved secret into bug reports or evidence.
