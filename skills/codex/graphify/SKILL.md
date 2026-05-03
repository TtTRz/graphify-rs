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
```

## Build or update a graph

From the repository root:

```bash
graphify-rs build --path . --output .graphify --no-llm --format json,report
```

After code changes, update instead of rebuilding everything:

```bash
graphify-rs build --path . --output .graphify --no-llm --update --format json,report
```

## Query from Codex

Prefer `graphifyq`; it behaves like `fffq`: it starts or reuses a per-project
local HTTP MCP sidecar, writes its registry to
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
- Keep `.graphify/` current after meaningful code edits with `--no-llm --update`.
- Do not paste entire reports; summarize god nodes, communities, cycles, and next questions.
