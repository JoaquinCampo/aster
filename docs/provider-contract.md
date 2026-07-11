# Provider contract

## Common contract

Providers accept a canonical, full wire model ID, prompt, and normalized reasoning effort (`none`, `low`, `medium`, `high`, `xhigh`, or `max`). They return an asynchronous stream of output, reasoning, tool-call argument, and completion/usage events. HTTP failures, streamed `response.failed` events, malformed/truncated SSE, and transport failures are distinct errors. Dropping a stream is cancellation: the response body and underlying HTTP transfer are dropped; adapters do not claim server-side rollback.

Credentials are constructor inputs only, are never returned, and debug output redacts authorization. The generic adapter implements the OpenAI Responses endpoint and always requests SSE (`stream: true`) with storage disabled. xAI uses the same Responses contract, requires a full `grok-*` model ID, and adds bearer authorization. No live xAI or generic-provider request was exercised by routine tests.

The deterministic fake emits an immutable caller-supplied event sequence without sleeping or network access. `PiProcess` is the narrow Pi child-process boundary; process lifecycle and protocol implementation remain intentionally outside this provider slice. The existing `PiAdapter` remains the runtime-facing interface.

## Discovered local Codex bridge

Source inspected: `~/.grok/codex-bridge-rs` (Rust). The supplied scripts build/start the release binary and discover it at loopback host `GROK_CODEX_BRIDGE_HOST` (default `127.0.0.1`) and port `GROK_CODEX_BRIDGE_PORT` (default `18474`). Startup uses `start.sh`, a PID file, `/healthz`, and authenticated `/readyz`. Routes include `/v1/models` and `/v1/responses` (also unversioned aliases). It rejects non-loopback peers and needs no client-side bridge credential.

Authentication is performed inside the bridge using the Codex session at `CODEX_AUTH_PATH` or `~/.codex/auth.json`; the adapter neither reads nor exposes that file. The bridge refreshes rejected sessions internally. It forwards to the Codex Responses service and forces SSE plus `store: false`.

Canonical wire IDs found in source are `gpt-5.6-luna`, `gpt-5.6-terra`, and `gpt-5.6-sol`. Therefore Luna, Terra, and Sol are model-name shorthand, not accepted route aliases in this adapter. Note: the inspected bridge's `/v1/models` enumeration currently lists Terra and Sol but protocol code explicitly supports Luna's distinct Responses Lite contract. Luna moves tools/developer instructions into input, disables parallel tool calls, adds a cache key and reasoning context.

The bridge normalizes effort aliases to `none`, `low`, `medium`, `high`, `xhigh`, and `max`, defaulting unknown values to `medium`. It normalizes input/history, function calls, encrypted reasoning history and streamed output. Tool calls arrive as Responses function-call item/argument events. Usage is reported on `response.completed` as input/output/total token counts.

Pre-stream errors use HTTP JSON `{error:{code,message}}`, including invalid request (422), busy/rate-limit (429), auth unavailable (503), timeout (504), and upstream failures (502). Midstream failures use `response.failed`. Truncated, malformed, idle, model-mismatch, and transport streams are converted to explicit failure events. The adapter contract tests use local fake HTTP servers and do not exercise subscription-backed live generation.
