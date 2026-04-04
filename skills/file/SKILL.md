---
name: tokenstunt-file
description: All symbols in a file with signatures and line numbers
argument-hint: <path>
---

Show all symbols in "$ARGUMENTS" using the `list_file_symbols` MCP tool.

## How to use

- Pass the file path relative to the project root.
- Optionally filter by symbol kind (function, class, interface, etc.).

## Presenting results

- Show each symbol with its kind, name, line range, and signature.
- Include the full code block for each symbol.
- Do NOT summarize or paraphrase the code. Let the user read it directly.

## Handling edge cases

- **Index not ready**: If the response says "Index is empty", indexing is still in progress. Wait a moment and retry, or run `/tokenstunt-setup` to check status.
- **No symbols found**: The file may not contain parseable symbols, or the language may not be supported. Suggest `/tokenstunt-setup` to check supported languages.

## Follow-up suggestions

- `/tokenstunt-search <concept>` to find specific functionality across all files
- `/tokenstunt-context <symbol>` to see dependencies of a specific symbol from the file
