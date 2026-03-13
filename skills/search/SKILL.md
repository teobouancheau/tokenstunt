---
name: tokenstunt-search
description: Semantic code search by concept or keyword
argument-hint: <query>
---

Search the codebase using the `ts_search` MCP tool with the query "$ARGUMENTS".

## How to use

- Pass the user's query as-is. The search engine handles natural language and keywords.
- If the user specifies a language, directory scope, or symbol kind, pass those as filters.
- Default limit is 10. Only increase if the user asks for more.

## Presenting results

- Show each result with its kind, name, file path, and line range.
- Include the code block so the user can read the actual implementation.
- Do NOT summarize or paraphrase the code. Let the user read it directly.
- If there are many results, highlight the most relevant ones first.

## Handling edge cases

- **No results**: Tell the user clearly. Suggest alternative queries (synonyms, broader terms). Suggest `ts_symbol` if the query looks like an exact symbol name.
- **Too many results**: Suggest narrowing with a language filter, scope, or symbol kind.

## Follow-up suggestions

After showing results, suggest next steps when relevant:
- `/tokenstunt-symbol <name>` for the exact definition of a specific result
- `/tokenstunt-context <name>` to see what a result depends on and what depends on it
- `/tokenstunt-impact <name>` before refactoring a result
