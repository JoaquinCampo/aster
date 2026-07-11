# Security model

## Trust

Trusted computing base: Rust control plane, validated configuration loader, durable store, effect broker, and explicitly selected adapters. Untrusted: model/provider output, repository files, retrieved text, MCP servers, skills, hooks, plugins, subprocess output, and external services.

Every effect from trusted core code must pass through `EffectBroker` or an independently enforced isolation boundary. A manifest is only a request. Deny when a capability cannot be safely mediated. Grants are scoped to task and operation and, as applicable, canonical workspace/worktree roots, executable argv, environment names, network destination, secret reference, and service.

## Isolation dimensions

Filesystem, process, network, credential, external-service, and workspace/worktree isolation are independent and must be reported independently. For every runtime operation, the selected adapter reports the concrete launch outcome for all six dimensions. The runtime stores these normalized records with task, attempt, and operation ownership before provider execution. Each record states whether the control was active, whether it was enforced, its mechanism, and its limitation. The Permissions/Approvals TUI pane queries those durable records for the selected execution, including after restart; it does not infer enforcement from route intent.

Current macOS enforcement is broker policy, not a kernel sandbox: there is no container/namespace boundary, syscall filter, complete TOCTOU defense, or protection from trusted code that bypasses the broker. Network/external transports are deny-by-default until implemented. The deterministic in-process adapter therefore reports all six controls inactive and unenforced rather than presenting fixture policy as isolation. The trusted runtime constructs a task-scoped `ProcessExec` grant and an exact request-bound approval before every Pi discovery or execution launch; the request binds the canonical Node executable, canonical sidecar argument, cleared environment containing only `PATH` and optional `ASTER_PI_NODE_MODULES`, and canonical sidecar-directory cwd. The core broker persists intent and authorization before spawning the dedicated child. Pi still explicitly reports inherited host-filesystem, host-network, host-credential-file/service, and external-service limitations.

Processes receive argv rather than shell strings and a cleared environment populated only by grants. Filesystem checks canonicalize roots and targets, reject traversal, and prevent known symlink escapes. Production hardening still requires sandboxed workers and descriptor-relative access.

## Secrets and data

Configuration stores environment/keychain references only. Never log values, authentication headers, prompts containing credentials, or raw provider errors that may echo them. Audit events may retain nonsensitive operation metadata and digests but not deletable payloads or reconstructable derivatives. `.aster/` is local sensitive state and is excluded from Git.

## No product telemetry and outbound inventory

Aster has no analytics, crash-reporting, usage-beacon, or product-telemetry transport. Logs, audit history, token accounting, benchmarks, and health state remain local. The locked graph has one HTTP client (`reqwest`), and its sole application call site is `src/provider.rs`. MCP uses an explicitly configured local stdio child; external tools may communicate only under their declared grants. No background updater or health beacon exists.

Task communication is not telemetry: when the user invokes a configured provider, the destination receives the model identifier, selected task prompt/context, and reasoning effort. MCP/tool calls receive the method and explicitly selected arguments. The Providers and Plugin/MCP TUI panes disclose destination, purpose, context classes, and the `task_communication_not_product_telemetry` distinction; `NetworkDisclosure::audit_detail` provides the same structured record for audit integration.

`scripts/check-no-telemetry.sh` fails if outbound call sites escape the reviewed provider adapter, an MCP network transport appears, or common telemetry dependencies enter either locked graph. `tests/no_product_telemetry.rs` proves routine local store/fake-adapter operations do not connect and that an explicit provider call reaches only its configured and disclosed destination.

## Validation and reporting

`effect_security` tests exercise positive and negative broker boundaries. `pi_gateway` additionally proves denial before spawn, exact argument/environment approval invalidation, and durable Pi launch operation/authorization evidence after restart. `scan-secrets.sh` scans tracked files for high-confidence forms; it does not replace a dedicated history scanner or human review. Dependency advisories are gated by `cargo audit` in CI. Report vulnerabilities privately to the maintainer until a public security policy/channel exists; include minimal reproduction and no live secret.
