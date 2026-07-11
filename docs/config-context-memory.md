# Configuration, context, and memory

## Architecture

Aster keeps these control-plane services independent of providers, runtime, and TUI. `config` loads ordered TOML layers into a typed, validated schema while retaining unknown top-level values. A loaded document records a content digest; save validates, detects concurrent edits, writes and syncs a sibling temporary file, then atomically renames it.

## Context and compatibility

Discovery walks from project root to the working directory. At each level it recognizes `.claude/{CLAUDE.md,AGENTS.md}`, `.agents/{CLAUDE.md,AGENTS.md}`, and recursive `skills/**/SKILL.md`. Deeper assets override ancestors by relative key and `.claude` overrides equivalent `.agents` assets. Other immediate files are inventoried as unsupported rather than silently executed. Symlinks and ecosystem-specific frontmatter, hooks, commands, plugins, and remote skills are not interpreted in v0.1.

A context manifest records every included item, source path, ecosystem, trust classification, category, estimated tokens, and aggregate budget. Project instructions are trusted only as instructions selected by core discovery; arbitrary retrieved/model/tool content must be represented as `UntrustedContent`. The estimate is deterministic bytes/4 and admission fails closed when over budget.

## Memory

Memory scopes are turn, task, session, user preference, project knowledge, architecture decision, and audit history; runtime audit remains a separate append-only store. Entries have explicit provenance. A store-keyed digest deduplicates active content within scope. Same-key differing values are surfaced as contradictions, not silently resolved. Amend creates a replacement after deleting the old payload. Delete removes the complete payload row—including key, value, provenance, and digest—then writes a metadata-only tombstone containing only identifier, scope, and deletion time. It truncates WAL state and vacuums free pages so deleted text cannot remain in the database files. Startup migrates legacy digest-bearing tombstones to the non-reconstructable shape.

The Memory TUI is a complete command surface. Commands are entered in the normal input line with `|` separators: `inspect`, `search|query`, `add|scope|key|value|provenance`, `amend|id|value|provenance`, `merge|id,id|key|value|provenance`, `contradictions|scope|key|value`, `expire`, `delete|id`, and `export`. Add performs deterministic deduplication. Every result or validation failure is visible in TUI status.

## Limitations

Layer merging is typed and currently replaces the context block as a unit. Unknown values are semantically preserved, but comments and original TOML formatting are not. Atomic rename and fsync cover the file, but parent-directory fsync is not yet performed. Trust labels communicate policy; capability enforcement remains the responsibility of the existing broker/runtime boundary.
