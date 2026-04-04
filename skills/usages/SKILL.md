---
name: tokenstunt-usages
description: Find all call sites and usages of a symbol
argument-hint: <symbol>
---

Find all usages of "$ARGUMENTS" using the `find_usages` MCP tool.

## How to use

- Pass the exact symbol name.
- Optionally filter by kind or limit results.

## Presenting results

- Show each usage with the caller's name, file path, line range, and relationship type.
- Include the full code block at each call site so the user can see the context.

## Handling edge cases

- **Index not ready**: If the response says "Index is empty", indexing is still in progress. Wait a moment and retry, or run `/tokenstunt-setup` to check status.
- **Symbol not found**: Suggest `/tokenstunt-search <name>` for a broader search.
- **No usages**: The symbol is not called anywhere. It can be safely modified or removed.

## Follow-up suggestions

- `/tokenstunt-impact <symbol>` to see the full transitive blast radius beyond direct callers
- `/tokenstunt-context <symbol>` to see what the symbol itself depends on
