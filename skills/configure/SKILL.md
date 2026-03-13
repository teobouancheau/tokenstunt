---
name: tokenstunt-configure
description: Configure Token Stunt embeddings with auto-detection of provider, models, and dimensions
---

Configure Token Stunt embeddings automatically.

## Before starting

Check if `~/.cache/tokenstunt/` contains a project directory with a `config.toml`. If it does, read it and show the current config, then ask if the user wants to reconfigure.

## Step 1: Pick a provider

Ask the user which provider they want to use:
- **Ollama** (local, free, recommended)
- **LM Studio** (local, free)
- **OpenAI** (cloud, paid)
- **Other** (any OpenAI-compatible endpoint)

## Step 2: Auto-detect models

Based on the provider, query the API to list available models. Do NOT ask the user to type a model name manually.

**Ollama:**
```bash
curl -s http://localhost:11434/api/tags
```
Parse the JSON `.models` array. Show model names and let the user pick. If curl fails, tell the user to start Ollama first.

**LM Studio:**
```bash
curl -s http://localhost:1234/v1/models
```
Parse the JSON `.data` array. Filter to embedding models (names containing "embed"). If none match, show all loaded models. If curl fails, tell the user to start the LM Studio server first.

**OpenAI:**
Show these options directly (no API call needed):
- text-embedding-3-small (1536 dimensions, cost-effective)
- text-embedding-3-large (3072 dimensions, highest quality)
- text-embedding-ada-002 (1536 dimensions, legacy)

**Other:** Ask for the endpoint, then query `<endpoint>/models` to list available models.

## Step 3: Auto-detect dimensions

Send a test embedding request to determine the vector dimensions:

**Ollama:**
```bash
curl -s http://localhost:11434/api/embed -d '{"model":"MODEL_NAME","input":"test"}'
```
Count the length of the first embedding vector.

**LM Studio / OpenAI / Other:**
```bash
curl -s ENDPOINT -H "Content-Type: application/json" -H "Authorization: Bearer API_KEY" -d '{"model":"MODEL_NAME","input":["test"]}'
```
Count the length of `.data[0].embedding`.

If auto-detect fails, use known defaults:
- nomic-embed-text: 768
- text-embedding-3-small: 1536
- text-embedding-3-large: 3072

## Step 4: API key

- **Ollama / LM Studio**: Skip, no key needed.
- **OpenAI**: Ask for the API key.
- **Other**: Ask if an API key is needed.

## Step 5: Write config

Map the provider:
- Ollama: `provider = "ollama"`
- LM Studio / OpenAI / Other: `provider = "openai-compat"`

Set the endpoint:
- Ollama: `http://localhost:11434`
- LM Studio: `http://localhost:1234/v1/embeddings`
- OpenAI: `https://api.openai.com/v1/embeddings`

Write `~/.cache/tokenstunt/<project>/config.toml` with the detected values. Show the final config to the user for confirmation before writing.

## Step 6: Verify

Run `ts_setup` to confirm the embeddings are configured and healthy. If coverage is 0%, tell the user the server will start embedding code blocks automatically.
