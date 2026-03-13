---
name: tokenstunt-setup
description: Project diagnostics with index health, languages, and embeddings status
---

Show project diagnostics using the `ts_setup` MCP tool.

## Presenting results

- Show index health: root, database path, file count, code block count, dependency stats.
- Show language breakdown.
- Show embeddings status (configured or not, coverage percentage).

## Diagnosing issues

After reviewing the results, proactively flag problems:

- **0 files indexed**: The index is empty. Tell the user to restart the MCP server or run `tokenstunt index --root <path>`.
- **Low dependency resolution**: Many unresolved dependencies may indicate missing files in the index scope. Suggest expanding the root path.
- **Embeddings not configured**: Explain that semantic search is disabled. Suggest `/tokenstunt-configure` to set up embeddings.
- **Low embedding coverage**: Some code blocks are missing embeddings. The server will catch up automatically, but the user can re-index to speed it up.

## Follow-up suggestions

- `/tokenstunt-configure` to set up or reconfigure embeddings
- `/tokenstunt-overview` to explore the indexed project structure
