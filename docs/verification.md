# Verification, artifacts, and evidence

Aster treats verification as recorded evidence, not model confidence. `DeterministicCheck` describes an allowlisted process, scrubbed environment, working directory, deadline, and expected artifacts. `run_check` always submits the process through `EffectBroker`; normal callers cannot bypass capability, path, authorization, or audit enforcement.

Each check records its distinct terminal status (`passed`, `failed`, `inconclusive`, `cancelled`, or `timed out`), process exit code, complete stdout/stderr, SHA-256 digests of both streams, and SHA-256/size/media type metadata for produced artifacts. A process launch/policy/broker error is inconclusive rather than a product failure. A nonzero exit is failed. A deadline is timed out. Missing required artifacts turn an otherwise passing check inconclusive.

## Proportional workflows

`VerificationPolicy::proportional` supplies minimum templates:

| Risk | deterministic checks | independent checkers | maximum fixer rounds |
|---|---:|---:|---:|
| Low | 1 | 0 | 0 |
| Medium | 1 | 1 | 2 |
| High | 2 | 2 | 3 |

Policies may strengthen these floors, but cannot weaken them; fixer rounds have an absolute ceiling of ten. `MakerCheckerFixerDag::template` creates a maker, parallel deterministic and independent checker nodes, bounded fixer nodes, and a finalizer. Checker actor IDs must be unique and cannot equal the maker ID. Final assembly refuses missing evidence, repeated checker identities, or excess fixer rounds and carries every check, review, and artifact into `FinalEvidence`.

## Limitations

The template is a control-plane DAG description; scheduling these nodes through the durable runtime remains integration work. Cancellation is represented by adapters that can report it, while `SystemAdapter` currently only observes completion or timeout and does not terminate a spawned child when the timeout future is dropped. Evidence is serializable but is not yet persisted in dedicated store tables or rendered in the TUI. Full output capture is intentionally lossless and currently has no size cap or redaction layer.
