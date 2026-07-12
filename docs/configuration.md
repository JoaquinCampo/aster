# Configuration

Configuration is TOML parsed into schema-validated canonical types. Schema version 2 is current. Version 1 documents are migrated sequentially and idempotently to version 2 before validation; missing, malformed, obsolete, and future versions are rejected for complete documents. Layering is defaults → user → project; later layers override known scalar fields. Instruction discovery separately supports `.agents` and `.claude` compatibility with `.claude` preferred for equivalent conflicts. See `config-context-memory.md` for implemented details and fixtures.

## Safety contract

* Persist secret **references**, never resolved values.
* Reject unknown schema versions and invalid capability combinations.
* Preserve unknown fields during semantic file/TUI round trips where supported; comments and whitespace are not contractual.
* Write atomically and detect concurrent modification before replacement.
* Keep roles independent from model and reasoning effort.
* Treat filesystem, process, network, credentials, external services, and workspace/worktree isolation as separate dimensions.

## Operational coverage

Every required domain has a typed schema: providers/models, roles, routing, budgets, permissions, tools/MCP, skills/rules, hooks/plugins, persistence paths, TUI, verification, and lifecycle/concurrency. Unknown keys are retained only in each schema's flattened `extensions` map. Nested layers merge recursively in defaults → user → project → local order, so a later scalar does not erase sibling values or unknown extensions.

Start the application with `aster --state PATH --config PATH`. Configuration is validated before the store or runtime starts. The runtime consumes routing policy paths, token/time budgets, retry limits and concurrency; TUI startup consumes persistence database/memory paths and refresh timing. Provider endpoints must be HTTPS except loopback. Provider credentials are references of the form `env:NAME`, `keychain:NAME`, or `file:PATH`; literal credentials are rejected.

The Config screen lists every editable schema leaf. Type `field=TOML_VALUE` and press Enter—for example `lifecycle.concurrency=4`, `models.allow=["grok-4"]`, or `providers.auth_ref="env:XAI_API_KEY"`. `e` enables a selected boolean domain. Each edit reloads the document, validates the complete candidate, preserves unknown fields, detects a stale baseline, and atomically replaces the file. Comments and whitespace are not contractual.
