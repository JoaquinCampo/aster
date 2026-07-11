# Out-of-process plugins

A plugin is an executable plus `plugin.toml`. Configure discovery roots explicitly and call `discover`; nothing is auto-enabled. Manifests declare a bounded `aster-plugin` protocol range, capabilities, tool JSON-schema contracts, endpoint descriptors, and optional `SKILL.md`/rule files. Duplicate IDs, tools, and endpoint names are rejected.

The host exchanges one JSON object per line over stdin/stdout. Requests contain `id`, `method`, and `params`; responses contain the same `id` and exactly one of `result`, `error`, or `effect`. Initialization negotiates host protocol version 1. Lifecycle methods are `initialize`, `lifecycle.stop`, and `health`. Calls are bounded by the manifest timeout; timeout, EOF/crash, malformed responses, and ID mismatches fail closed and terminate/mark the child unhealthy.

Plugins are untrusted. Their environment is cleared and they receive no in-process API. Effects must be returned as an `effect` carrying a declared capability and are executed only by the embedding application's `EffectBroker`, where policy, authorization, path confinement, auditing, and user approval belong. A capability declaration is not authorization.

`McpEndpoint` and `ToolContract` are registry contracts for names, descriptions, and input schemas. This implementation does **not** claim MCP wire interoperability: it does not implement or exercise MCP JSON-RPC initialization, transports, sessions, notifications, or conformance tests. An adapter may expose these contracts to an MCP implementation after separate interoperability testing.

See `fixtures/plugins/echo` for an installable fixture. Production installers should verify provenance/signatures and copy into an administrator-selected discovery root; installation and trust policy are intentionally outside the process host.
