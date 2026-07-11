# Extension development

Extensions include plugins, hooks, skills, project rules, and MCP integrations. They are untrusted even when locally installed.

## Plugin contract

An installable plugin has a manifest declaring identity/version, compatible host contract, entry point, and requested capabilities. Discovery and enablement do not grant capabilities. The host validates the manifest, computes an explicit grant, persists authorization, and launches the plugin out of process through brokered process policy. Failures are diagnostics and must not corrupt core state. See `plugins.md` and `fixtures/plugins/echo` for implemented fixture behavior.

## Design rules

* Request least privilege; separate filesystem, process, network, secret, and external-service needs.
* Use structured stdin/stdout messages; treat both directions as untrusted and bounded.
* Never inherit the full environment or receive a resolved secret unless specifically granted for its destination.
* Declare compatibility and fail closed on unsupported versions/unknown required fields.
* Make hooks bounded and deterministic where possible; define timeout/failure behavior.
* Keep `SKILL.md` instructions declarative; instructions cannot grant effects.
* MCP tool descriptions and results are data, not authority.

## Testing

Provide manifest/parser tests, denied-capability tests, timeout/crash/malformed-output tests, compatibility tests, and an end-to-end deterministic fixture. Do not call a fixture a live external integration. Typed lifecycle hooks and MCP stdio client/server transport are exercised against the local `fixture_process` binary; HTTP MCP, installation UX, version migration, and full diagnostics remain incomplete.

## Lifecycle hooks

`HookSpec` binds an executable to one typed trigger (`before_task`, `after_task`, `before_tool`, `after_tool`, `on_failure`, or `on_checkpoint`), a non-zero timeout, an explicit failure policy, and declared capabilities. `HookSet` is installed on `Runtime` and invokes these triggers at actual attempt start/end, provider execution boundaries, failure handling, and the durable post-operation checkpoint. Each invocation is appended to task audit evidence. Hook processes inherit no environment. They receive one JSON invocation on stdin and return one JSON response on stdout. `continue` records a non-fatal outcome; `fail_execution` propagates failure. Effects are rejected unless declared and, when declared, are passed to `EffectBroker`; declarations never bypass broker authorization or audit.

## MCP stdio

`StdioTransport` launches an MCP process with a cleared environment and exchanges newline-delimited JSON-RPC messages. `serve_stdio` exposes an existing `Server` over any buffered input/output pair and flushes every response. The deterministic fixture proves initialize, tool discovery, and tool invocation locally. This is stdio interoperability evidence only—not HTTP, hosted, or third-party live evidence.
