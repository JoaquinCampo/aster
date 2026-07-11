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

Provide manifest/parser tests, denied-capability tests, timeout/crash/malformed-output tests, compatibility tests, and an end-to-end deterministic fixture. Do not call a fixture a live external integration. Current plugin hosting is an early out-of-process slice; complete lifecycle hooks, MCP client/server operation, installation UX, version migration, and full diagnostics remain incomplete.
