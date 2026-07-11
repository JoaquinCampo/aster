# Project Brief: A State-of-the-Art Agent Harness Built on Pi

## Status

This document is the executable project brief for a new, independent open-source repository under `joaquincampo/`. The project name is intentionally provisional. The implementation agent should begin by selecting a temporary codename that can be replaced without architectural impact.

The project is ambitious by design. It is not intended to clone Grok Build, Claude Code, or Pi feature for feature. Those systems are evidence and prior art. The goal is to design and build the best agent harness we can: capable, efficient, observable, safe, extensible, and pleasant to use.

## Mission

Build a state-of-the-art interactive agent harness on top of [`badlogic/pi-mono`](https://github.com/badlogic/pi-mono), with:

- a custom Rust control plane;
- a custom terminal user interface;
- Pi used as the underlying agent runtime;
- dynamic, per-execution selection of role, model, reasoning effort, context, tools, permissions, isolation, and lifecycle;
- durable multi-agent orchestration;
- excellent observability and user control;
- first-class context engineering, memory, verification, and extensibility.

The harness should push the state of the art rather than merely wrap existing functionality.

## Product thesis

A strong agent harness must jointly optimize:

1. result quality;
2. cost and quota usage;
3. latency;
4. context efficiency;
5. safety and user control;
6. observability and debuggability;
7. extensibility;
8. long-running reliability.

Optimizing only model capability is insufficient. A harness that silently assigns an expensive model to a trivial task, delegates work without a clear benefit, hides why it made decisions, or cannot recover durable state is not a good harness.

The system must treat the following as independent dimensions:

| Dimension | Question |
|---|---|
| Role | What responsibility does this agent have? |
| Model | What level of capability does this execution need? |
| Reasoning effort | How much reasoning should be allocated? |
| Context | What information is necessary and relevant? |
| Tools | What operations does the task require? |
| Permissions | What may this execution do without approval? |
| Isolation | Where and how should it execute? |
| Lifecycle | Foreground, background, durable, retryable, resumable? |
| Verification | What evidence is required before accepting the result? |

A role must never be synonymous with a model. The same `reviewer`, `explorer`, or `learning-capture` role may run on different models and efforts depending on the execution.

## Non-goals

- Do not make a pixel-perfect clone of an existing harness.
- Do not preserve limitations merely for compatibility.
- Do not build a thin wrapper that delegates every meaningful decision to Pi.
- Do not use role proliferation as a substitute for dynamic routing (for example, avoid `reviewer-terra` and `reviewer-sol`).
- Do not make the architecture depend on the provisional project name.
- Do not add product telemetry in v0.1.
- Do not sacrifice a coherent design to reproduce every legacy configuration format exactly.

## Foundation and repository strategy

- Base runtime: `badlogic/pi-mono`.
- Repository: a new, isolated repository under the `joaquincampo/` GitHub namespace.
- Distribution: open source under a permissive license. Choose MIT or Apache-2.0 and document the rationale.
- Primary platform for v0.1: macOS on Apple Silicon.
- Implementation language for the control plane and custom TUI: Rust.
- Architecture: a custom control plane and TUI that use Pi as the runtime.
- Pi strategy: maintain a fork of Pi when necessary. Keep the fork disciplined, document every divergence, and preserve a practical path for syncing upstream.

The agent should inspect Pi before fixing integration boundaries. It may introduce adapters, protocol layers, or targeted fork changes as needed. High-level product decisions in this brief are fixed; low-level APIs and code structure are not prescribed.

## Core design principles

### 1. Delegate only when delegation has a benefit

Before spawning a subagent, the harness should identify the benefit:

- protect the parent context;
- parallelize independent work;
- obtain independent review;
- apply different tools or permissions;
- isolate changes;
- use a more appropriate model;
- preserve a long-running task independently.

If the parent can perform a trivial operation immediately and safely, it should not spawn a subagent. A one-line user preference should not automatically cause a costly agent execution.

### 2. Start with the cheapest reliable route

Routing should begin with the least expensive model and effort likely to succeed, then escalate only when evidence justifies it. Evidence may include:

- failed deterministic checks;
- repeated unsuccessful attempts;
- contradictory evidence;
- low confidence grounded in observable conditions;
- high task risk;
- checker rejection;
- scope growth;
- unmet quality thresholds.

Escalation should preserve useful work and pass a compact, structured handoff rather than restart from the full transcript.

### 3. Routing must be dynamic and auditable

The router must choose per execution:

- role;
- model;
- reasoning effort;
- context budget;
- tool set;
- capability set;
- isolation;
- execution budget;
- verification policy.

The user must be able to see and override the choice. Every routing and escalation event must record the selected route, reason, relevant signals, budget, and outcome.

Before implementing the Codex adapter, locate and inspect the existing local Codex bridge. Record its source or installation location, startup and discovery mechanism, transport protocol, authentication flow, model enumeration, streaming and tool-call behavior, reasoning-effort mapping, usage reporting, and error semantics in a provider contract document backed by an integration test. Determine from the bridge itself whether Luna, Terra, and Sol are route aliases or wire-level model identifiers. If the bridge cannot be located or exercised, report that as a genuine blocker rather than inventing a compatible protocol. Search workspace repositories, local development configuration, environment variables, running services, and project documentation as necessary.

The initial routing approach should be hybrid and auditable:

- explicit declarative policies;
- observable task features;
- historical outcomes;
- no silent self-modification of routing policies;
- learned recommendations may be proposed, but policy changes require explicit review.

### 4. Context is a first-class resource

The harness should send each execution the minimum sufficient context, with provenance. It should distinguish:

- user intent;
- system and project rules;
- retrieved evidence;
- prior decisions;
- task state;
- memory;
- untrusted content;
- excluded or stale context.

Each execution should have an inspectable context manifest. Handoffs should be structured, compact, and loss-aware. The system should measure context size, duplication, relevance, and rework caused by missing information.

### 5. Verification should be proportional to risk

Prefer deterministic evidence over model confidence:

1. tests and executable checks;
2. type checking and static analysis;
3. artifact validation;
4. diff inspection;
5. independent model review;
6. self-reported confidence.

Low-risk mechanical work may need only deterministic checks. Substantive work should support maker/checker/fixer loops with independent agents. High-risk or external actions should use explicit policy gates.

### 6. Capabilities and risk determine permissions

Permissions should be risk-based:

- local, reversible reads, edits, and tests may proceed automatically;
- destructive operations, shared-state changes, secrets, publication, production actions, and other hard-to-reverse effects require appropriate safeguards;
- capabilities should be granted per execution and follow least privilege;
- actions should be classified by effect, not only by tool name;
- every approval and capability grant should be auditable.

The user has authorized the implementation agent to operate autonomously for this project, including repository setup, implementation, commits, pushes, and releases. This is one-time authorization for the implementation project and must not establish permissive defaults for the harness being built. The agent must still preserve user data, avoid destructive shortcuts, protect secrets, and clearly record consequential actions.

Define the v0.1 trust and enforcement model before implementation. Treat model output and externally supplied MCP servers, hooks, skills, and plugins as untrusted inputs. All effectful operations must either pass through a capability-enforcing broker or execute inside an isolation boundary that enforces the granted filesystem, process, network, secret, and external-service capabilities. A manifest declaration is not enforcement. Document trusted core components, bypass paths, platform limitations, and behavior when an operation cannot be safely mediated.

Treat isolation as a set of explicit dimensions rather than a single label: workspace or worktree isolation, process isolation, filesystem restrictions, network restrictions, credential isolation, and external-service isolation. Every execution must state which dimensions are active and which are not.

### 7. Durable state is fundamental

v0.1 must include all of the following persistence categories:

- user preferences, editable and with provenance;
- project knowledge and discovered conventions;
- durable task state, checkpoints, attempts, artifacts, and dependencies;
- an append-only audit log for routing, tools, permissions, changes, and lifecycle events.

Pause, cancellation, retry, and recovery semantics must be explicit at step boundaries. Pausing prevents new work from starting; an already-running operation may complete or be cancelled only when its adapter supports safe cancellation. Every effectful operation must have a durable operation identity and recorded intent, start, and outcome. After a crash, operations with an unknown outcome must enter a reconciliation-required state and must not be automatically repeated unless the adapter proves retry safety. Recovery must preserve explicit distinction among queued, running, pausing, paused, cancelling, cancelled, succeeded, failed, timed out, and outcome-unknown states.

Append-only audit history and user-directed deletion must coexist without retaining deleted payloads. Deletable memory content must not be embedded in immutable audit events. Audit history may retain non-sensitive metadata, tombstones, and digests needed to explain that an operation occurred, but must not retain deleted content, secrets, or enough derived material to reconstruct them.

Memory must support inspection, correction, deletion, deduplication, contradiction detection, provenance, and scope. Do not conflate turn context, task state, session state, project knowledge, user preferences, and audit history.

### 8. Observability is a product feature

The user should always be able to answer:

- What is running?
- Why was it launched?
- Which role, model, and reasoning effort are active?
- What context and permissions were provided?
- What has it done?
- What files or external systems did it affect?
- How much time, quota, tokens, or estimated cost has it consumed?
- Why did routing escalate or de-escalate?
- What is blocked?
- How can it be paused, resumed, inspected, retried, or cancelled?

Observability must include, from v0.1:

- task pane;
- usage and budget views;
- routing trace;
- audit trail;
- inspectable transcripts and artifacts;
- task graph and critical-path visibility.

### 9. Files and TUI are equal configuration surfaces

Roles, providers, policies, skills, hooks, and plugins must have declarative, versionable configuration and a complete TUI for inspection and editing. The two representations must remain consistent and round-trip safely.

### 10. No product telemetry in v0.1

Do not build product analytics or transmit usage telemetry. Local operational logs, audit records, benchmarks, and usage accounting are required, but remain local unless the user explicitly exports them.

Product telemetry is distinct from task data intentionally sent to configured model, MCP, or external tool providers. The harness must show those destinations through permission and audit surfaces, disclose which context is transmitted, and apply the configured capability policy. Disabling telemetry must not disable explicitly requested provider communication.

## Model and provider support

v0.1 must support:

1. GPT-5.6 Luna, Terra, and Sol through the existing local Codex bridge;
2. xAI/Grok models;
3. generic OpenAI-compatible providers.

Authentication in v0.1 must support:

- the local Codex bridge;
- generic API keys and environment-variable references.

Never persist secrets in plaintext configuration or logs. If secure OS-backed storage is added, treat it as an enhancement rather than a substitute for environment-based configuration.

Reasoning effort must be selectable independently for every execution when the provider supports it. Provider capability differences must be explicit and normalized through a well-defined adapter contract rather than hidden.

## Roles

The role system must be extensible. The initial built-in role set should cover at least:

- orchestrator;
- explorer;
- planner;
- implementer;
- reviewer;
- verifier;
- fixer;
- advisor;
- learning-capture.

Roles define responsibility, behavioral contract, default capabilities, expected inputs, and expected outputs. They may define safe defaults, but all execution properties must remain dynamically overridable.

A role definition should be able to declare:

- purpose and boundaries;
- input/output contracts;
- default context policy;
- default capabilities;
- allowed tools;
- verification expectations;
- fallback routing preferences;
- isolation preference;
- completion criteria.

The implementation format is intentionally unspecified.

## Orchestration runtime

v0.1 must provide a durable runtime rather than a fire-and-forget spawn wrapper.

Required capabilities:

- foreground and background agents;
- task DAGs and explicit dependencies;
- bounded concurrency;
- retries with policy and backoff;
- cancellation;
- pause and resume;
- timeouts and budgets;
- durable checkpoints;
- crash recovery;
- idempotent recovery where practical;
- compact handoffs;
- artifact passing;
- maker/checker/fixer workflows;
- detection of stalled or looping agents;
- graceful escalation and de-escalation;
- explicit terminal states and failure reasons.

Only bounded, policy-controlled delegation depth should be allowed. The system must prevent accidental runaway fan-out.

## Custom TUI

The primary v0.1 interface is a custom Rust TUI, not Pi's existing TUI.

It should provide a coherent workspace for:

- primary conversation;
- active and completed tasks;
- task DAG;
- subagent transcript inspection;
- routing decisions;
- token/quota/time budgets;
- audit events;
- permissions and approvals;
- context manifests;
- artifacts and diffs;
- configuration;
- memory and project knowledge;
- provider status;
- pause, resume, cancel, retry, and override operations.

The TUI should prioritize clarity and keyboard-driven operation. It must remain responsive while agents and tools stream events. Accessibility, terminal compatibility on macOS, graceful degradation, and recoverability matter more than ornamental complexity.

The agent should define interaction design and keybindings after studying relevant prior art. Do not copy another product blindly.

## Context engine

The context engine must support:

- hierarchical instruction discovery;
- relevance-based retrieval;
- explicit context manifests;
- source provenance;
- token budgets by category;
- freshness and invalidation;
- structured summaries;
- compact handoffs;
- protection of critical constraints;
- separation of trusted instructions and untrusted content;
- measurement of duplication and omitted-context rework.

Instruction compatibility requirements:

- accept `.claude/` and `.agents/` conventions;
- when equivalent instructions conflict, prefer `.claude/` over `.agents/`;
- support hierarchical rules and skills from those ecosystems;
- the new harness may define richer native capabilities and formats beyond this compatibility floor.

Compatibility should not prevent a clean internal canonical representation.

Compatibility must be defined through an inventory and executable fixtures rather than vague equivalence. Inventory the `.claude/` and `.agents/` assets supported in v0.1; document discovery, hierarchy, precedence, merge behavior, scoping, unknown fields, and unsupported features; and add fixtures for nested projects, conflicts, and partial compatibility.

## Memory and knowledge

Memory must be explicit rather than an opaque transcript dump.

Required stores or scopes:

- current turn;
- current task;
- session;
- user preferences;
- project knowledge;
- architectural decisions;
- durable audit history.

Required operations:

- inspect;
- search;
- add;
- amend;
- merge;
- deduplicate;
- identify contradictions;
- expire or invalidate;
- delete/forget;
- export;
- trace provenance.

Simple preference capture should use deterministic storage when possible. It must not require an expensive model invocation merely to persist a short structured fact.

## Extensibility

v0.1 must include:

- MCP client and server integration where appropriate;
- skills compatible with existing `SKILL.md` conventions;
- hierarchical project rules;
- configurable lifecycle hooks;
- an installable plugin system.

Plugins and hooks must declare capabilities and be subject to permissions. Their failures must not corrupt core state. Versioning, compatibility, discovery, enable/disable controls, and diagnostics are required.

The harness should accept existing `.claude/` and `.agents/` assets, preferring `.claude/` on conflicts, while allowing richer native extensions.

## Configuration

Configuration must be:

- declarative;
- versionable;
- schema-validated;
- inspectable;
- editable through files and the TUI;
- safe to round-trip;
- layered by sensible scopes;
- explicit about precedence;
- migratable across schema versions.

File/TUI round-trip means semantic preservation, including unknown fields, atomic writes, and conflict detection; preservation of comments and whitespace is not required unless the chosen format explicitly supports it. Secret values are never part of the round-trip contract: the TUI and files persist secret references, not resolved values, and must never reveal or rewrite secret material accidentally.

At minimum, configuration must cover:

- providers and models;
- roles;
- routing policies;
- budgets;
- permissions;
- tools and MCP;
- skills and rules;
- hooks and plugins;
- persistence paths;
- TUI preferences;
- verification policy;
- concurrency and lifecycle policy.

## Evaluation and benchmarks

The harness must evaluate itself systematically. Success is multidimensional.

Required evaluation dimensions:

### Quality

- task success;
- deterministic verification rate;
- independent reviewer verdicts;
- regression benchmark scores;
- retry and escalation effectiveness.

### Cost and quota

- tokens and provider usage;
- estimated cost where meaningful;
- quality per unit of usage;
- unnecessary high-capability routing;
- escalation overhead.

### Latency

- wall-clock completion time;
- time to first useful action;
- critical-path duration;
- queue and tool wait time;
- parallelism effectiveness.

### UX and control

- unnecessary approval prompts;
- user overrides;
- cancelled or misunderstood tasks;
- visibility of current state;
- recovery experience;
- routing explanation quality.

### Context efficiency

- context size and relevance;
- duplicated information;
- handoff compression;
- omissions that cause rework;
- cache/retrieval effectiveness.

Benchmarks should be versioned and compared against clear baselines, including a strong fixed-model baseline. Before claiming routing improvements, define fixed representative scenarios, reproducible provider and reasoning settings, acceptance thresholds, and the statistical or qualitative method used to compare quality, cost, latency, UX, and context efficiency. Evaluations against real providers may be manual or explicitly invoked in v0.1; routine tests must not unpredictably consume paid quota.

## Requirements traceability

Maintain a versioned acceptance matrix that maps every normative `must` and every v0.1 acceptance criterion to:

- an owning milestone;
- implementation status;
- automated test or explicit manual acceptance procedure;
- evidence artifact or report;
- known limitations.

Stubs, placeholders, disabled tests, or UI elements without enforced behavior do not count as support. The matrix is part of the release evidence and must remain synchronized with the brief as requirements evolve.

## Quality bar

v0.1 requires:

- unit tests for core logic;
- integration tests across runtime components;
- end-to-end tests for the TUI and representative workflows;
- deterministic fake or recorded providers for routine tests;
- failure-injection tests for persistence and recovery;
- security and permission-boundary tests;
- format, lint, and static-analysis gates;
- pre-commit checks;
- GitHub Actions with a real macOS Apple Silicon release-gate run;
- preserved CI logs and evidence artifacts;
- documentation for architecture, operation, configuration, extension development, and recovery.

If hosted macOS arm64 runners are unavailable, document and operate an alternative arm64 runner; do not silently waive platform verification. Before selecting MIT or Apache-2.0, inspect Pi's license and all relevant fork/distribution obligations, then record the project license, required notices, and compatibility rationale in an ADR.

The repository should remain runnable and tested throughout development. Although v0.1 has a broad required scope, implement it through internally coherent vertical slices rather than an unverifiable big bang.

## Required first architectural slice

The first architectural slice must validate the complete process and persistence boundary without implementing the full release breadth:

1. A task is submitted through the Rust TUI.
2. The Rust control plane invokes one Pi execution through an adapter.
3. A simple explicit route and its rationale are recorded and can be overridden before execution.
4. A durable single-node task and event history survive restart.
5. Output and deterministic verification evidence are inspectable.
6. The workflow has an integration test using a deterministic fake provider.

Subsequent vertical slices add DAG scheduling, lifecycle controls, isolation, provider breadth, permissions, memory, extensibility, and full observability. All items in the integrated acceptance workflow below remain mandatory for v0.1.

## Required v0.1 integrated acceptance workflow

The complete v0.1 workflow must demonstrate the architectural core end to end:

1. A user submits a real repository task through the custom TUI.
2. The orchestrator decides whether delegation provides a benefit.
3. The router chooses a role, model, reasoning effort, context, tools, capabilities, budget, and verification policy.
4. The choice and rationale are visible and overridable.
5. One or more agents execute through Pi.
6. Tasks can run in the background and appear in a durable DAG.
7. The user can inspect, pause, resume, cancel, retry, or override them.
8. An implementer can work in isolation.
9. Deterministic checks execute.
10. An independent reviewer or verifier evaluates the result when policy requires it.
11. Failures can trigger bounded retries or evidence-based escalation.
12. A crash or restart can recover durable state.
13. The final result includes artifacts, verification evidence, routing trace, audit trail, context accounting, and usage.

A complementary acceptance case must show that a trivial request is handled directly without spawning a subagent.

## v0.1 acceptance criteria

v0.1 is complete only when all major systems described in this brief are implemented to a coherent, usable baseline. Specifically:

- Custom Rust TUI is the primary interface.
- Pi executes agents beneath a custom control plane.
- The same role can run with different models and reasoning efforts per execution.
- Codex bridge, xAI/Grok, and generic OpenAI-compatible providers are supported.
- Routing is hybrid, auditable, explainable, and user-overridable.
- Role, model, effort, context, capabilities, isolation, and lifecycle are independent.
- Durable DAG execution supports retries, persistence, recovery, pause/resume, cancellation, and budgets.
- Task pane, usage, routing trace, audit trail, context manifests, and artifacts are inspectable.
- Risk-based permissions and least-privilege capabilities are enforced.
- User preferences, project knowledge, task state, and audit history persist with provenance.
- Configuration round-trips between declarative files and the TUI.
- `.claude/` and `.agents/` are accepted, with `.claude/` preferred on conflicts.
- MCP, skills, project rules, hooks, and plugins are usable.
- No product telemetry is transmitted.
- Test pyramid and CI gates pass.
- Representative benchmark results for quality, cost, latency, UX/control, and context efficiency are recorded against baselines.
- The repository includes operational, architecture, contributor, extension, and recovery documentation.

## Autonomous implementation contract

The implementation agent is expected to complete the project without routine user supervision. It must treat this brief, discovered repository evidence, executable tests, benchmarks, and ADRs as its primary decision inputs. It should ask the user only when progress depends on a genuinely unavailable external fact or authority that cannot be discovered, safely defaulted, simulated, or isolated.

### Provisioned environment and assumptions

The agent may assume:

- the local repository should be created under `~/Documents/Personal/<provisional-name>`;
- GitHub CLI authentication for the `joaquincampo/` namespace is available or discoverable through the existing environment;
- it is authorized to create a public GitHub repository under `joaquincampo/`;
- it may push branches, manage pull requests, and publish development releases autonomously;
- the local Codex bridge is available and authenticated, but its endpoint and contract must be discovered and verified rather than guessed;
- Rust, Git, GitHub CLI, and ordinary development tools may be installed or bootstrapped locally when absent;
- there is no fixed implementation deadline or quota ceiling; quality and completion of the full v0.1 scope take priority, while all model usage must still be budgeted and observable;
- external provider credentials other than the Codex bridge must not be assumed. Generic and xAI provider support may use deterministic contract fixtures until real credentials are discoverable.

The agent must never print, persist, commit, or transmit discovered credentials except to their explicitly configured destination.

### Autonomous preflight

Before implementation, run and record a preflight that checks:

- the chosen local path is safe and available;
- Git and `gh` identity/authentication;
- public repository creation capability;
- Rust toolchain and platform information;
- Pi source, license, build, tests, and integration points;
- Codex bridge discovery, startup behavior, supported models, reasoning efforts, streaming, tool calls, usage reporting, and errors;
- availability of required local skills and tools, especially `tui-use`;
- CI feasibility for a real macOS arm64 release gate;
- secret-handling and local persistence paths.

Preflight findings belong in durable project documentation. A missing optional integration must not block unrelated architectural work.

### Blocker and fallback policy

Classify blockers rather than stopping the whole project:

1. **Core blocker:** prevents a required architectural boundary from being validated. Investigate exhaustively, preserve evidence, and continue independent work. Use a deterministic adapter or fake only when it validates the same boundary without pretending the external integration works.
2. **Integration blocker:** affects one provider, plugin, tool, or compatibility surface. Implement the contract and fixtures, record the live-validation gap in the acceptance matrix, and continue.
3. **Environmental blocker:** missing local dependency, runner, credential, or service. Bootstrap safely when possible; otherwise use a documented reproducible substitute and preserve a clear final validation requirement.
4. **Product decision:** resolve through the principles in this brief, evidence, prior art, prototypes, benchmarks, and an ADR. Do not ask the user to choose routine libraries, schemas, keybindings, or internal APIs.

Never invent an external contract, silently weaken a `must`, mark a fixture-only integration as live-validated, or hide a blocker by disabling tests. When a requirement truly cannot be completed without unavailable external authority, isolate it precisely and finish all unaffected work before reporting it.

### TUI validation with `tui-use`

The agent has access to the `tui-use` skill at:

`~/.claude-pento/plugins/cache/my-plugins/tools/0.1.0/skills/tui-use/SKILL.md`

Read and follow that skill whenever operating or validating the custom TUI. TUI correctness must be tested through the rendered terminal interface, not inferred solely from component tests or internal state.

For every release-critical interactive workflow:

1. build the actual binary;
2. launch it in a PTY with `tui-use start`, using explicit terminal dimensions;
3. wait for semantic screen signals with `tui-use wait --text` where possible;
4. interact using real typing and key presses;
5. capture screen and JSON snapshots at meaningful states;
6. exercise success, failure, cancellation, timeout, permission, and recovery paths;
7. restart the process and verify durable state through the UI;
8. verify task DAG, routing rationale, usage, audit, context, artifacts, and override controls through the UI;
9. test multiple terminal dimensions and degraded terminal capabilities;
10. kill every PTY session after validation;
11. preserve reproducible scripts, fixtures, and evidence artifacts.

Automated TUI E2E tests should use deterministic providers and stable semantic assertions. Visual snapshots alone are insufficient; tests must assert state transitions and persisted outcomes.

### Autonomous definition of done

The agent must not declare v0.1 complete until:

- every acceptance criterion maps to implementation and evidence in the requirements traceability matrix;
- all required unit, integration, failure-injection, security, recovery, and TUI E2E tests pass;
- release-critical workflows have been operated through `tui-use` as a real user;
- pause, resume, cancellation, retry, crash recovery, and outcome reconciliation have been exercised;
- permission boundaries have positive and negative tests demonstrating enforcement rather than declarations;
- provider adapters have contract tests and accurately report which live integrations were exercised;
- `.claude/` and `.agents/` compatibility fixtures cover hierarchy and conflicts;
- routing benchmarks compare against fixed baselines across quality, cost, latency, UX/control, and context efficiency;
- a clean checkout can build, test, configure, and run using only documented steps;
- the public repository contains no secrets, local-only data, generated credentials, or private transcripts;
- macOS arm64 release artifacts have been built and exercised;
- architecture, operation, configuration, security, recovery, plugin development, and contribution documentation are complete;
- known limitations are explicit and do not contradict a claimed acceptance criterion.

## Expected implementation approach

The implementation agent has autonomy over low-level design and code. It should not ask the user to make routine technical decisions that can be resolved through investigation, prototypes, tests, or ADRs.

It should begin by:

1. creating the independent repository;
2. inspecting `badlogic/pi-mono` and identifying stable runtime integration points;
3. mapping the requirements in this brief to native Pi support, adapter work, fork changes, and new components;
4. documenting key architectural choices as concise ADRs;
5. defining an executable milestone graph that covers the full v0.1 scope;
6. building the first vertical slice early;
7. expanding it iteratively while preserving testability and durable migrations;
8. benchmarking routing and orchestration decisions throughout development.

The agent may revise implementation details when evidence demands it, but it must preserve the product principles and acceptance criteria. If a requirement proves incompatible with Pi, it should document the evidence and implement the necessary controlled fork or replacement boundary rather than silently weakening the product.

## Autonomy and decision policy

The implementation agent is authorized to proceed autonomously, including:

- creating and configuring the repository;
- selecting the provisional name;
- choosing internal libraries and schemas;
- implementing the system;
- writing tests and documentation;
- creating commits;
- pushing branches;
- opening and merging project changes;
- publishing development releases when appropriate.

It must:

- protect credentials and private data;
- avoid destructive shortcuts;
- preserve recoverability;
- keep consequential decisions in ADRs;
- maintain an audit-friendly history;
- report genuine blockers rather than lowering requirements silently;
- prefer evidence from working software, tests, and benchmarks over unsupported claims.

## Guiding product test

For every feature and architectural decision, ask:

> Does this make the harness more capable, efficient, understandable, controllable, extensible, or reliable without hiding important trade-offs from the user?

If the answer is no, the feature does not belong merely because another harness has it.

## Final directive

Build an ambitious, production-quality v0.1 that establishes a durable foundation for continued research and product development. Use Pi as leverage, not as a constraint. Preserve the best ideas from existing agent harnesses, reject accidental limitations, and make model routing, context, permissions, verification, durability, and observability first-class parts of the system.

