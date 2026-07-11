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

Policies may strengthen these floors, but cannot weaken them; fixer rounds have an absolute ceiling of ten. `MakerCheckerFixerDag::template` creates a maker, parallel deterministic and independent checker nodes, and a finalizer. Provider checker output must decode as a typed JSON `CheckerVerdict` with a non-empty rationale. The orchestration runtime executes maker/checker phases first, adds a fixer only when a typed verdict is non-passing, and then schedules the finalizer against either checker or fixer dependencies. Checker actor IDs must be unique and cannot equal the maker ID. Final assembly refuses missing evidence, repeated checker identities, or excess fixer rounds and carries every check, review, and artifact into `FinalEvidence`.

## Durable evidence

Schema migration 4 adds normalized `verification_runs` and `verification_evidence` tables. A run binds task, attempt, checker identity and maker/checker/fixer/finalizer ownership to the serialized policy, command/tool identity, start/completion timestamps, environment and isolation profiles, terminal outcome, and exit status. Evidence rows contain only typed artifact/output references plus SHA-256 digest, media type, size, and timestamp; payload bytes remain in their owning task/artifact store. Queries are ordered by attempt and timestamp and survive restart. The TUI Artifacts pane renders these query results, including ownership, policy, execution profile, outcome, and evidence digests.

Task payload deletion retains the non-reconstructive run metadata and digest while atomically nulling evidence payload references. Foreign keys reject orphan evidence; malformed digests are rejected before insertion. Migration, restart, ownership, failure, and deletion behavior is covered by `verification_persistence`.

## Limitations

Provider-specific reviewer schemas beyond the canonical verdict remain future work. Cancellation is represented by adapters that can report it, while `SystemAdapter` currently only observes completion or timeout and does not terminate a spawned child when the timeout future is dropped. Full deterministic-check output capture is intentionally lossless and currently has no size cap or redaction layer.
