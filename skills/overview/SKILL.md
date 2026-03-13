---
name: tokenstunt-overview
description: Project structure with module tree, languages, public API, and entry points
---

Show the project structure using the `ts_overview` MCP tool.

## How to use

- Call with no scope to see the full project.
- If the user asks about a specific directory (e.g., "show me the src/ structure"), pass it as the scope parameter.

## Presenting results

- Present the overview as a structural map of the codebase.
- Highlight key information: total files, code blocks, dominant languages, main modules, and entry points.
- The public API section shows exported symbols, which is useful for understanding the project's surface area.

## Follow-up suggestions

After showing the overview, guide the user to dive deeper:
- `/tokenstunt-search <concept>` to find specific functionality
- `/tokenstunt-overview` with a scope to zoom into a specific module
- `/tokenstunt-setup` to check index health and embeddings status
