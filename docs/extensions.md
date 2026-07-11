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

Provide manifest/parser tests, denied-capability tests, timeout/crash/malformed-output tests, compatibility tests, and an end-to-end deterministic fixture. Do not call a fixture a live external integration. Typed lifecycle hooks and MCP stdio and Streamable HTTP client/server transports are exercised entirely against local deterministic fixtures; routine tests require no external MCP endpoint.

## Lifecycle hooks

`HookSpec` binds an executable to one typed trigger (`before_task`, `after_task`, `before_tool`, `after_tool`, `on_failure`, or `on_checkpoint`), a non-zero timeout, an explicit failure policy, and declared capabilities. `HookSet` is installed on `Runtime` and invokes these triggers at actual attempt start/end, provider execution boundaries, failure handling, and the durable post-operation checkpoint. Each invocation is appended to task audit evidence. Hook processes inherit no environment. They receive one JSON invocation on stdin and return one JSON response on stdout. `continue` records a non-fatal outcome; `fail_execution` propagates failure. Effects are rejected unless declared and, when declared, are passed to `EffectBroker`; declarations never bypass broker authorization or audit.

## MCP stdio

`StdioTransport::spawn_authorized` launches an MCP process only through the core `EffectBroker`, using an exact `EffectRequest::Exec` that binds the scoped process grant and explicit approval to the executable, arguments, scrubbed environment, and working directory. The broker persists intent, start, authorization, and launch outcome before the transport exchanges newline-delimited JSON-RPC messages; denied or mutated requests never reach process creation. `serve_stdio` exposes an existing `Server` over any buffered input/output pair and flushes every response. The deterministic fixture proves initialize, tool discovery, and tool invocation locally.

## MCP Streamable HTTP

`StreamableHttpTransport` uses MCP's JSON/SSE content negotiation, retains the server-issued `Mcp-Session-Id`, correlates JSON-RPC responses, converts protocol/HTTP failures into bounded errors, enforces a request timeout, and supports cancellation both before authorization and while a request is in flight. Before opening a socket it presents the endpoint destination, operation, and context classes to `NetworkMediator`; `EffectBrokerMediator` applies the runtime grant's network allowlist. `serve_streamable_http` is the deterministic local conformance server: POST initializes and invokes MCP, DELETE cancels a session, unknown sessions fail closed, and no external service is needed.

## Installation UX

The Plugins TUI accepts `install|PATH`, `upgrade|PATH`, `uninstall|ID`, `enable|ID`, `disable|ID`, and `diagnostics`. Install and upgrade stage and validate a complete copy, atomically swap it into place, and restore the previous installation on activation or post-validation failure. Uninstall quarantines before deletion and restores on failure. Enablement is explicit persistent state and never grants capabilities. The pane shows compatibility diagnostics plus every declared MCP destination and context class before use.
