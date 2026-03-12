---
name: tokenstunt-configure
description: Configure TokenStunt embeddings — auto-detect provider, models, and dimensions
---

Configure TokenStunt embeddings automatically.

First, check if `~/.cache/tokenstunt/<project>/config.toml` already exists. If it does, read it and show the current config, then ask if the user wants to reconfigure.

## Step 1: Pick a provider

Ask the user which provider (Ollama, LM Studio, OpenAI, Other).

## Step 2: Auto-detect models

Based on the provider, query the API to list available models. Do NOT ask the user to type a model name manually.

**Ollama:**
```bash
curl -s http://localhost:11434/api/tags
```
Parse the JSON `.models` array. Show model names to the user and let them pick one. If curl fails, tell the user to start Ollama first.

**LM Studio:**
```bash
curl -s http://localhost:1234/v1/models
```
Parse the JSON `.data` array. Filter to only show embedding models (names containing "embed" or "embedding"). If none match, show all loaded models. Let the user pick one. If curl fails, tell the user to start the LM Studio server first.

**OpenAI:**
Show these options directly (no API call needed):
- text-embedding-3-small (1536 dims, cheap)
- text-embedding-3-large (3072 dims, best quality)
- text-embedding-ada-002 (1536 dims, legacy)

**Other:** Ask the user for endpoint, then query `<endpoint>/models` to list available models.

## Step 3: Auto-detect dimensions

Once the model is selected, auto-detect dimensions by sending a test embedding request:

**Ollama:**
```bash
curl -s http://localhost:11434/api/embed -d '{"model":"MODEL_NAME","input":"test"}'
```
Count the length of the first embedding vector in the response.

**LM Studio / OpenAI / Other:**
```bash
curl -s ENDPOINT -H "Content-Type: application/json" -H "Authorization: Bearer API_KEY" -d '{"model":"MODEL_NAME","input":["test"]}'
```
Count the length of `.data[0].embedding` in the response.

If auto-detect fails, fall back to common defaults:
- nomic-embed-text: 768
- text-embedding-3-small: 1536
- text-embedding-3-large: 3072

## Step 4: API key (only if needed)

- Ollama / LM Studio: skip, no key needed
- OpenAI: ask for the API key
- Other: ask if an API key is needed

## Step 5: Write config

Map the provider:
- Ollama -> provider = "ollama"
- LM Studio / OpenAI / Other -> provider = "openai-compat"

Set the endpoint:
- Ollama: "http://localhost:11434"
- LM Studio: "http://localhost:1234/v1/embeddings"
- OpenAI: "https://api.openai.com/v1/embeddings"

Write `~/.cache/tokenstunt/<project>/config.toml` with the detected values. Show the config to the user for confirmation.

## Step 6: Verify

Run `ts_setup` to confirm everything is healthy.
