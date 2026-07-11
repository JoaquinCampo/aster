# Configuration, context, and memory

## Architecture

Aster keeps these control-plane services independent of providers, runtime, and TUI. `config` loads ordered TOML layers into a typed, validated schema while retaining unknown top-level values. A loaded document records a content digest; save validates, detects concurrent edits, writes and syncs a sibling temporary file, then atomically renames it.

## Context and compatibility

Discovery walks from project root to the working directory. At each level it recognizes `.claude/{CLAUDE.md,AGENTS.md}`, `.agents/{CLAUDE.md,AGENTS.md}`, and recursive `skills/**/SKILL.md`. Deeper assets override ancestors by relative key and `.claude` overrides equivalent `.agents` assets. Other immediate files are inventoried as unsupported rather than silently executed. Symlinks and ecosystem-specific frontmatter, hooks, commands, plugins, and remote skills are not interpreted in v0.1.

A context manifest records every included item, source path, ecosystem, trust classification, category, estimated tokens, and aggregate budget. Project instructions are trusted only as instructions selected by core discovery; arbitrary retrieved/model/tool content must be represented as `UntrustedContent`. The estimate is deterministic bytes/4 and admission fails closed when over budget.

## Memory

Memory scopes are turn, task, session, user preference, project knowledge, and architecture decision; audit remains a separate append-only store. Entries have explicit provenance. Normalized content digests deduplicate within scope. Same-key differing values are surfaced as contradictions, not silently resolved. Amend creates a replacement after deleting the old payload. Delete atomically nulls content and writes a metadata-only tombstone (identifier, scope, digest, time); deleted payloads are neither active nor recoverable from the memory database.

## Limitations

Layer merging is typed and currently replaces the context block as a unit. Unknown values are semantically preserved, but comments and original TOML formatting are not. Atomic rename and fsync cover the file, but parent-directory fsync is not yet performed. Trust labels communicate policy; capability enforcement remains the responsibility of the existing broker/runtime boundary.
