# Recovery

## Principles

Do not infer success from process disappearance. Persist intent before effects, then start and outcome. After a crash, an operation with no durable outcome is `outcome-unknown`/reconciliation-required and must not be automatically repeated unless its adapter proves retry safety and idempotency.

Lifecycle states remain distinct: queued, running, pausing, paused, cancelling, cancelled, succeeded, failed, timed out, and outcome unknown. Pausing prevents new steps; an in-flight operation may finish unless its adapter supports safe cancellation.

## Procedure

1. Stop Aster and preserve `.aster/state.db` plus SQLite sidecars, maintaining file permissions.
2. Record the binary version/commit and failure time without copying secrets into the incident record.
3. Restart normally; inspect recovered task/event state and every reconciliation-required operation.
4. Verify external state (filesystem, process, provider, or service) through a read-only, authorized check.
5. Record one decision: accept observed completion, compensate, fail, or retry only with demonstrated retry safety.
6. Run deterministic verification and retain safe evidence.
7. Resume dependent work only after reconciliation.

## Checkpoints and artifacts

Each operation persists normalized checkpoints and artifacts under an immutable task/attempt/operation owner tuple. Payload integrity is verified with a `sha256:` digest before insertion. Provider output records include media type and route/model decision provenance; dependent DAG tasks resolve successful parents' artifact digests at their operation-intent checkpoint. Restart recovery changes only in-flight task/operation state: existing checkpoint and artifact rows remain inspectable in the TUI Artifact screen and are never inferred to prove an unknown external outcome.

## Database restore

Restore only from a consistent stopped-process backup. Keep the damaged database read-only until investigation completes. A backup can lose later effects; reconcile external state before replay. Never edit append-only events to make state appear healthy and never delete the database as a recovery shortcut.

## Current limitations

Single-node restart handling and persistence tests exist. Complete UI-driven pause/resume/cancel/retry controls, broad failure injection, automated backup tooling, and live external adapter reconciliation evidence remain incomplete and are release blockers in the acceptance matrix.
