---
name: tokenstunt-context
description: Symbol dependency graph showing what it calls and what calls it
argument-hint: <symbol>
---

Show the dependency graph for "$ARGUMENTS" using the `show_context` MCP tool.

## How to use

- Pass the exact symbol name.
- Default direction is "both" (dependencies + dependents). Only filter if the user asks for one direction.

## Presenting results

- Show the symbol definition first so the user has context.
- Show dependencies (what this symbol calls) and dependents (what calls this symbol) as separate sections.
- For each dependency/dependent, include the kind, name, file path, and relationship type.

## Handling edge cases

- **Index not ready**: If the response says "Index is empty", indexing is still in progress. Wait a moment and retry, or run `/tokenstunt-setup` to check status. Do NOT suggest alternative queries.
- **Not found**: Tell the user clearly. Suggest `/tokenstunt-search <name>` to find the correct symbol name.
- **No dependencies or dependents**: This is useful information. Tell the user the symbol is standalone (no coupling).

## Follow-up suggestions

- `/tokenstunt-impact <name>` for full transitive blast radius (context shows only direct; impact walks the full graph)
- `Read <file_path>` to examine a specific dependency or dependent in detail
