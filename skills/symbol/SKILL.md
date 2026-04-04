---
name: tokenstunt-symbol
description: Exact symbol lookup by name with full definition
argument-hint: <name>
---

Look up the exact definition of "$ARGUMENTS" using the `lookup_symbol` MCP tool.

## How to use

- Pass the exact symbol name. This is a precise lookup, not a fuzzy search.
- If the user specifies a kind filter (function, class, interface, etc.), pass it.

## Presenting results

- Show the full symbol body with file path and line numbers.
- If multiple definitions exist (e.g., overloads, re-exports), show all of them.
- Do NOT summarize the code. Show the complete definition.

## Handling edge cases

- **Index not ready**: If the response says "Index is empty", indexing is still in progress. Wait a moment and retry, or run `/tokenstunt-setup` to check status. Do NOT suggest alternative queries.
- **Not found**: Tell the user clearly. Suggest `/tokenstunt-search <name>` for a broader search instead.
- **Multiple matches**: Show all matches. Let the user identify which one they need.

## Follow-up suggestions

After showing the definition:
- `/tokenstunt-context <name>` to see dependencies and dependents
- `/tokenstunt-impact <name>` to understand blast radius before modifying
