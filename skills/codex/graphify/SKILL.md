---
name: graphify
description: Build, update, and query graphify-rs knowledge graphs from Codex; prefer graphifyq for short-lived HTTP MCP graph access.
---

# graphify for Codex

Use this skill when the user asks to build or query a project knowledge graph,
understand repository architecture through graphify-rs, or use `graphifyq`.

## Install / availability check

```bash
command -v graphify-rs >/dev/null || echo "Install graphify-rs first"
command -v graphifyq >/dev/null || echo "Install graphifyq first"
command -v graphify-llm-codex >/dev/null || echo "Install graphify-llm-codex first if you need Codex-backed LLM extraction"
```

## Build or update a graph

From the repository root:

```bash
graphify-rs build --path . --output .graphify --no-llm --format json,report
```

This default is intentionally AST/local-document only for new calls. It does
not delete existing cached LLM extraction data in `.graphify/llm-cache.json`.

After code changes, prefer the short-lived helper. It auto-refreshes a stale
per-repo graph every 300s with a safe incremental build and restarts the local
HTTP sidecar if the graph changed:

```bash
graphifyq ensure
```

To force the update immediately:

```bash
graphify-rs build --path . --output .graphify --no-llm --update --format json,report
```

## Optional LLM enrichment via installed Codex CLI

Do not ask the user for Anthropic/OpenAI API keys just to enrich graphify.
graphify-rs can call any installed local CLI that reads a prompt from stdin and
prints compact JSON with `entities` and `relationships`. The bundled adapter is
for Codex CLI:

```bash
graphify-llm-codex --model gpt-5.4-mini --reasoning-effort low < prompt.txt
```

Use LLM enrichment only when the user explicitly asks for it, or when indexing
large docs/prose where AST extraction is insufficient:

```bash
graphify-rs build --path . --output .graphify --update --embed \
  --llm-command "graphify-llm-codex --model gpt-5.4-mini --reasoning-effort low" \
  --llm-provider codex-cli

graphifyq ensure --with-llm \
  --llm-command "graphify-llm-codex --model gpt-5.4-mini --reasoning-effort low" \
  --llm-provider codex-cli
```

LLM cache rules:

- `--no-llm` means "make no new LLM calls"; it preserves existing LLM output.
- `--llm-command` reuses previous extraction as context when a file changed, so
  rebuilds are incremental/reiterative instead of starting from scratch.
- If the command/provider/prompt contract changes, graphify marks old LLM output
  as stale-preserved instead of silently treating it as fresh.
- Pass the repository root to `--path`; use `.graphifyignore` include/exclude
  rules when you need to narrow to a subdirectory such as `src` or `bin`.

## Query from Codex

Prefer `graphifyq`; it behaves like `fffq`: it starts or reuses a per-project
local HTTP MCP sidecar, auto-refreshes stale graphs, writes its registry to
`.graphify/.graphifyq-server.json`, prints the answer, and exits.

```bash
graphifyq ensure
graphifyq query "where is authentication wired?"
graphifyq summary architecture --budget 3000
graphifyq stats
graphifyq tool graph_stats '{}'
```

Use graphify for architecture/codebase questions after FFF/grepai source lookup,
not as a replacement for exact file search. Good graphify questions:

- "what are the main communities in this repo?"
- "which modules bridge the data ingestion and API layers?"
- "what depends on this table/service/function?"
- "where are cycles or surprising cross-community edges?"

## Project-level Codex setup

```bash
graphify-rs codex install
```

This adds graphify guidance to `AGENTS.md` and writes `.codex/hooks.json` with a
silent `graphify-rs hook-check` entry. It intentionally does not inject unsupported
Codex hook output; use `graphifyq` explicitly for graph context.

## Rules for agents

- If `.graphify/GRAPH_REPORT.md` exists, consult it before broad architecture answers.
- Prefer `graphifyq query` or `graphifyq summary architecture` for concise context.
- Keep `.graphify/` current after meaningful code edits with `graphifyq ensure`; use `--no-llm --update` only to force an immediate rebuild without new LLM calls.
- Use `graphifyq ensure --with-llm --llm-command "graphify-llm-codex ..."` only for explicit LLM refresh/enrichment.
- Do not paste entire reports; summarize god nodes, communities, cycles, and next questions.
