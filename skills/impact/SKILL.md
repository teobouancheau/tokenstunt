---
name: tokenstunt-impact
description: Blast radius analysis showing all dependents and affected files
argument-hint: <symbol>
---

Analyze the blast radius of "$ARGUMENTS" using the `ts_impact` MCP tool.

## How to use

- Pass the exact symbol name.
- Default traversal depth is 3. Only increase (max 5) if the user asks to see deeper transitive dependents.

## Presenting results

- Show the total dependent count and affected file count upfront.
- Group dependents by depth: direct (depth 1) are the most important, deeper levels show transitive impact.
- Show affected files as a summary of where changes would ripple.

## Handling edge cases

- **Not found**: Tell the user clearly. Suggest `/tokenstunt-search <name>` to find the correct symbol name.
- **No dependents**: This is good news. Tell the user the symbol can be safely modified without affecting other code.
- **Large blast radius**: Warn the user. Suggest breaking the change into smaller steps or adding an abstraction layer.

## Follow-up suggestions

- `/tokenstunt-context <name>` to see the direct dependency graph in detail
- `Read <file_path>` to examine specific affected files before making changes
