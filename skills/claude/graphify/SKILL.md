---
name: graphify
description: Build, update, and query graphify-rs knowledge graphs from Claude Code; supports graphifyq and MCP graph tools.
trigger: /graphify
---

# graphify for Claude Code

Use this skill when the user invokes `/graphify`, asks to build/query a graph,
or asks architecture questions that benefit from graphify-rs.

## Build or update a graph

Default full build from the current repository:

```bash
graphify-rs build --path . --output .graphify
```

After code edits, prefer graphifyq's per-repo auto-refresh path:

```bash
graphifyq ensure
```

It refreshes stale graphs every 300s using the safe incremental build path and
restarts the local HTTP sidecar if the graph changed. To force the update
immediately:

```bash
graphify-rs build --path . --output .graphify --no-llm --update --format json,report
```

## Query options

Direct CLI query:

```bash
graphify-rs query "QUESTION" --graph .graphify/graph.json
```

Short-lived HTTP MCP helper, useful in Claude Code and other terminal agents:

```bash
graphifyq ensure
graphifyq ensure --no-auto-refresh
graphifyq query "QUESTION"
graphifyq summary architecture --budget 3000
graphifyq stats
```

Long-lived stdio MCP server:

```bash
graphify-rs serve --graph .graphify/graph.json
```

HTTP MCP sidecar:

```bash
graphify-rs serve --transport http --http-bind 127.0.0.1:0 \
  --registry-path .graphify/.graphifyq-server.json \
  --graph .graphify/graph.json
```

## Project-level Claude setup

```bash
graphify-rs claude install
```

This updates `CLAUDE.md` and `.claude/settings.json` so Claude Code is reminded
that a graph exists before broad file search.

## Response workflow

When invoked for a build:

1. Use `.` if no path was provided.
2. Run `graphify-rs build` with user-provided flags.
3. Read `.graphify/GRAPH_REPORT.md`.
4. Summarize only the useful parts: god nodes, communities, surprising edges,
   cycles, and suggested questions.
5. Offer one concrete follow-up query.

When answering architecture questions:

- Prefer existing `.graphify/GRAPH_REPORT.md` and `graphifyq summary architecture` first.
- Use `graphifyq query "..."` for focused questions.
- Rebuild with `--no-llm --update` after meaningful code changes.
- Do not paste full graph JSON or full reports unless explicitly requested.
