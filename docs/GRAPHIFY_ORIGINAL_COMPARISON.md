# graphify-rs vs safishamsi/graphify comparison notes

Reference inspected: `safishamsi/graphify` default branch `v6` at the time this fork work started.

## What graphify-rs already had

- Rust workspace with split crates for detect, extract, build, cluster, analyze, export, serve, watch, hooks, and benchmark.
- `graphify-rs serve` stdio MCP server with 15 graph tools.
- Codex install path that writes `AGENTS.md` plus `.codex/hooks.json`.
- Native tree-sitter extraction and NetworkX-compatible `graph.json` output.

## What original graphify had that matters for Codex

- Codex hook hardening moved to a `graphify hook-check` command that exits silently.
- Codex hook install merges with existing hooks instead of overwriting the whole file.
- Codex hook avoids unsupported `permissionDecision` and avoids emitting unsupported `additionalContext`.
- Skill docs are explicitly platform-aware and describe MCP usage.

## Changes adopted in this fork

- Added `graphify-rs serve --transport http` while preserving stdio as the default.
- Added local HTTP endpoints:
  - `POST /mcp` for JSON-RPC MCP requests.
  - `GET /health` for liveness and graph stats.
  - `GET /graphifyq/stats` for short-lived stats calls.
  - `POST /graphifyq/query` for short-lived graph queries.
  - `POST /graphifyq/tool` for raw MCP tool calls.
- Added `--http-bind`, `--http-path`, and `--registry-path` for local sidecar wiring.
- Added `graphifyq`, a short-lived helper inspired by `fffq`:
  - builds a missing AST-only graph,
  - starts/reuses a per-project HTTP sidecar,
  - stores registry under `.graphify/.graphifyq-server.json`,
  - provides `ensure`, `doctor`, `query`, `stats`, `summary`, and raw `tool`.
- Hardened Codex install:
  - hook command is `graphify-rs hook-check`,
  - hook-check is a silent no-op,
  - existing `.codex/hooks.json` is preserved,
  - stale graphify-rs hook entries are replaced,
  - unsupported `permissionDecision` is not emitted.

## SQL extractor port

The original graphify optional SQL extractor used `tree-sitter-sql` to find tables, views, functions, foreign keys, and `FROM`/`JOIN` relationships. The Rust fork now ports that capability with the pure-Rust `sqlparser` crate instead:

- `.sql` is collected as code.
- `graphify-extract` exposes an optional default-enabled `sql` feature.
- Dialect parsing tries `ClickHouseDialect`, then `PostgreSqlDialect`, then `GenericDialect`.
- Extracted SQL nodes include tables, views/materialized views, functions/procedures, columns, external referenced tables, and relationship edges (`defines`, `has_column`, `references`, `reads_from`, `writes_to`, `alters`).
- ClickHouse DDL/topology extraction handles common parser gaps around table-tail clauses without falling back to regex-only extraction.

## Deliberately not ported yet

- Python original's multimodal video/audio/office pipeline.
- Python original's large platform-specific skill variants.
- Hook behavior that injects context into Codex. Codex Desktop rejected previous extra context forms, so graph guidance is kept in `AGENTS.md`/skill and queried explicitly through `graphifyq`.
