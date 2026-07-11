# Security model

## Trust

Trusted computing base: Rust control plane, validated configuration loader, durable store, effect broker, and explicitly selected adapters. Untrusted: model/provider output, repository files, retrieved text, MCP servers, skills, hooks, plugins, subprocess output, and external services.

Every effect from trusted core code must pass through `EffectBroker` or an independently enforced isolation boundary. A manifest is only a request. Deny when a capability cannot be safely mediated. Grants are scoped to task and operation and, as applicable, canonical workspace/worktree roots, executable argv, environment names, network destination, secret reference, and service.

## Isolation dimensions

Filesystem, process, network, credential, external-service, and workspace/worktree isolation are independent and must be reported independently. Current macOS enforcement is broker policy, not a kernel sandbox: there is no container/namespace boundary, syscall filter, complete TOCTOU defense, or protection from trusted code that bypasses the broker. Network/external transports are deny-by-default until implemented.

Processes receive argv rather than shell strings and a cleared environment populated only by grants. Filesystem checks canonicalize roots and targets, reject traversal, and prevent known symlink escapes. Production hardening still requires sandboxed workers and descriptor-relative access.

## Secrets and data

Configuration stores environment/keychain references only. Never log values, authentication headers, prompts containing credentials, or raw provider errors that may echo them. Audit events may retain nonsensitive operation metadata and digests but not deletable payloads or reconstructable derivatives. `.aster/` is local sensitive state and is excluded from Git.

## Validation and reporting

`effect_security` tests exercise positive and negative broker boundaries. `scan-secrets.sh` scans tracked files for high-confidence forms; it does not replace a dedicated history scanner or human review. Dependency advisories are gated by `cargo audit` in CI. Report vulnerabilities privately to the maintainer until a public security policy/channel exists; include minimal reproduction and no live secret.
