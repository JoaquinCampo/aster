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

Validated typed configuration and precedence tests exist for the implemented slice. A complete TUI editor, all BRIEF domains (providers, roles, routing, budgets, permissions, MCP/tools, skills/rules, hooks/plugins, persistence, TUI, verification, concurrency/lifecycle), migration UX, and comprehensive unknown-field round-trip behavior are not yet complete. Do not hand-author settings based on aspirational fields in the BRIEF; only fields accepted by current types are operational.

Before changing configuration, back up the file, validate with tests, and inspect the effective configuration and provenance. Never paste a resolved secret into bug reports or evidence.
