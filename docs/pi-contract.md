# Pi runtime contract

## Inspected evidence

This boundary targets `badlogic/pi-mono` commit `8479bd84743e8889f728acb21a62794102db0529` and the locally installed npm packages `@mariozechner/pi-agent-core` and `@mariozechner/pi-ai` 0.73.0. The package manifests identify the repository, require Node >=20, and declare MIT; upstream `LICENSE` is MIT, copyright 2025 Mario Zechner. No Pi source or binary is copied into Aster. `THIRD_PARTY_NOTICES.md` records redistribution obligations.

The inspected 0.73.0 declarations establish `Agent`, `Agent.subscribe`, `Agent.prompt`, `Agent.abort`, `AgentEvent`, model lookup through `pi-ai.getModel`, message usage, thinking level, system prompt, tools, and abort signals. The implementation imports those installed package entry points during discovery and live runs; discovery rejects versions other than exactly 0.73.0.

## Protocol v1

`scripts/pi-sidecar.mjs` is a newline-delimited JSON stdio sidecar launched by Rust. Every frame has `v: 1`, a request `id`, and `type`. Requests are `discover`, `run`, and `abort`. Run input carries canonical `provider/model-id`, effort, prompt, and optional bounded context. Events are normalized as `ready`, `agent_start`, `message_delta`, `tool_preflight`, `usage`, `agent_end`, `aborted`, or `error {code,message}`. Unknown or malformed versions fail closed.

The Rust launcher clears the child environment, restoring only `PATH` and, when explicitly configured, `ASTER_PI_NODE_MODULES`. Consequently provider credentials are not inherited. Live mode gives Pi an empty tool list: the sidecar cannot perform ambient filesystem, process, network, secret, or external effects. A future exposed tool must first emit `tool_preflight`; Rust checks its requested capability before representing the call. Denial terminates before execution. Dropping the Rust request kills the child (`kill_on_drop`); protocol `abort` maps to the active run's abort controller.

## Validation and deterministic mode

`run` with `mode: "deterministic"` first imports the installed Pi package entry points and then deterministically emits agent/message/tool/usage/end frames without constructing an agent or contacting a provider. `PiGateway` implements the runtime `PiAdapter` interface by consuming those normalized events into `ExecutionResult` output and usage. Contract and integrated workflow tests verify import/discovery, normalization, capability denial, runtime output, and runtime token accounting through the actual sidecar boundary. No provider credentials are inherited and no paid call occurs. The installed 0.73.0 packages are a required local acceptance prerequisite rather than a silently skipped optional test. Set `ASTER_PI_NODE_MODULES` (or `PiGateway::with_node_modules`) to the directory containing the scoped packages.
